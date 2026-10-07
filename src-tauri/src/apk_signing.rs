//! APK 签名（APK Signature Scheme v1 + v2 + v3 的最小实现）。
//!
//! 打包出的 APK 需要签名才能在 Android 上安装；本模块直接读写 APK 的
//! ZIP 结构，生成 v1（JAR 签名：META-INF/MANIFEST.MF + CERT.SF + CERT.RSA，
//! 最小 PKCS#7 分离签名）并在中央目录前插入 v2/v3 签名块，全程纯 Rust
//! （rsa + sha2），桌面与 Android 端共用同一套实现。
//!
//! 性能：写入阶段单遍完成（读源 → 写临时文件 → 回填签名块），
//! v1 条目摘要用 rayon 并行计算，避免多轮全文件 IO。
//!
//! 默认使用内置的自签名测试密钥（`assets/mod-signing.pem`），
//! 用户可在工作区放置 `signing.pem` 覆盖（见 `commands::apk`）。

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use rayon::prelude::*;
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs8::{DecodePrivateKey, EncodePublicKey};
use rsa::{Pkcs1v15Sign, RsaPrivateKey, RsaPublicKey};
use sha2::{Digest, Sha256};

/// 工具内置的默认签名密钥（自签名 RSA 2048：证书 + PKCS8 私钥）。
const BUILTIN_SIGNING_PEM: &str = include_str!("../assets/mod-signing.pem");

const APK_SIGNING_BLOCK_MAGIC: &[u8; 16] = b"APK Sig Block 42";
const APK_SIGNING_BLOCK_V2_ID: u32 = 0x7109_871a;
const APK_SIGNING_BLOCK_V3_ID: u32 = 0xf053_68c0;
const RSA_PKCS1V15_SHA2_256: u32 = 0x0103;
const CHUNK_SIZE: usize = 1024 * 1024;
const EOCD_SIGNATURE: [u8; 4] = *b"PK\x05\x06";
const EOCD_MIN_SIZE: u64 = 22;
const LFH_SIGNATURE: u32 = 0x0403_4b50;
const CDFH_SIGNATURE: u32 = 0x0201_4b50;
/// v3 签名声明的 SDK 范围（min=24 覆盖 Android 7+，max 不限）。
const V3_MIN_SDK: u32 = 24;
const V3_MAX_SDK: u32 = 0x7fff_ffff;
/// v1（JAR 签名）生成的三个条目名。
const V1_MANIFEST_NAME: &str = "META-INF/MANIFEST.MF";
const V1_SF_NAME: &str = "META-INF/CERT.SF";
const V1_RSA_NAME: &str = "META-INF/CERT.RSA";

/// 签名密钥：RSA 私钥 + 证书与公钥（保留原始 DER 编码，直接嵌入签名块）。
pub struct ApkSigningKey {
    key: RsaPrivateKey,
    pubkey_der: Vec<u8>,
    cert_der: Vec<u8>,
}

impl ApkSigningKey {
    /// 从 PEM 文本加载密钥（需同时包含证书与 PKCS8/PKCS1 私钥块）。
    pub fn from_pem(pem: &str) -> Result<Self, String> {
        let key = if let Some(der) = pem_decode(pem, "PRIVATE KEY") {
            RsaPrivateKey::from_pkcs8_der(&der)
                .map_err(|error| format!("解析签名私钥失败：{error}"))?
        } else if let Some(der) = pem_decode(pem, "RSA PRIVATE KEY") {
            RsaPrivateKey::from_pkcs1_der(&der)
                .map_err(|error| format!("解析签名私钥失败：{error}"))?
        } else {
            return Err("签名文件缺少私钥块（PRIVATE KEY / RSA PRIVATE KEY）".to_string());
        };
        let cert_der = pem_decode(pem, "CERTIFICATE")
            .ok_or_else(|| "签名文件缺少证书块（CERTIFICATE）".to_string())?;
        let pubkey_der = RsaPublicKey::from(&key)
            .to_public_key_der()
            .map_err(|error| format!("导出签名公钥失败：{error}"))?
            .as_bytes()
            .to_vec();
        Ok(Self {
            key,
            pubkey_der,
            cert_der,
        })
    }

    /// 加载工具内置的默认测试密钥。
    pub fn builtin() -> Result<Self, String> {
        Self::from_pem(BUILTIN_SIGNING_PEM)
    }

    /// 公钥 DER 字节（校验与测试用）。
    #[allow(dead_code)]
    pub fn pubkey_der_bytes(&self) -> &[u8] {
        &self.pubkey_der
    }

    /// 证书 DER 字节（校验与测试用）。
    #[allow(dead_code)]
    pub fn cert_der_bytes(&self) -> &[u8] {
        &self.cert_der
    }

    /// 对指定 APK 就地写入 v1/v2/v3 签名（已有旧签名块会被替换）。
    /// 简版接口（无进度回调），测试与内部简单场景使用。
    #[allow(dead_code)]
    pub fn sign_apk(&self, path: &Path) -> Result<(), String> {
        self.sign_apk_with_progress(path, &|_, _, _, _| {})
    }

    /// 带进度回调的签名：`stage` 标识当前阶段，`unit` 为 `bytes`/`files`，
    /// `completed` / `total` 为对应单位的进度。
    ///
    /// 流程：v1（当 APK 尚无 MANIFEST.MF 时）→ 单遍读源并写临时文件（同时计算
    /// v2/v3 内容摘要）→ 回填 v2/v3 签名块 → 替换原文件。
    pub fn sign_apk_with_progress(
        &self,
        path: &Path,
        on_progress: &(dyn Fn(&'static str, &'static str, u64, u64) + Sync),
    ) -> Result<(), String> {
        const STAGE_WRITE: &str = "正在签名并写入 APK";
        let mut src =
            File::open(path).map_err(|error| format!("打开 APK 失败 {}：{error}", path.display()))?;
        let file_len = src
            .seek(SeekFrom::End(0))
            .map_err(|error| error.to_string())?;
        let layout = locate_zip_layout(&mut src)?;
        let entries = read_central_directory(&mut src, &layout)?;

        // v1：仅在 APK 尚无 MANIFEST.MF 时生成（已有 v1 签名则保留旧签名，只追加 v2/v3）。
        let v1 = if entries
            .iter()
            .any(|entry| entry.name.eq_ignore_ascii_case(V1_MANIFEST_NAME))
        {
            None
        } else {
            Some(self.build_v1_entries(path, &entries, layout.sb_start, on_progress)?)
        };
        let v1_bytes_len = v1.as_ref().map_or(0, |v1| v1.entries_bytes.len() as u64);
        let v1_records_len = v1.as_ref().map_or(0, |v1| v1.central_records.len() as u64);
        let v1_count = if v1.is_some() { 3_u16 } else { 0_u16 };

        // 预构建一次块（占位摘要）确定块长度，签名后回填真实块。
        let (v2_len, v3_len) = {
            let probe_v2 = self.build_v2_pair([0_u8; 32])?;
            let probe_v3 = self.build_v3_pair([0_u8; 32])?;
            (
                probe_v2.len() as u64 + 12,
                probe_v3.len() as u64 + 12,
            )
        };
        let block_len = v2_len + v3_len + 32;
        let new_sb_start = layout.sb_start + v1_bytes_len;
        let cd_offset_in_file = new_sb_start + block_len;
        let total_out = file_len + v1_bytes_len + block_len + v1_records_len;
        on_progress(STAGE_WRITE, "bytes", 0, total_out);

        let temp_path = path.with_file_name(format!(
            "{}.signing.tmp",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("repacked.apk")
        ));
        let mut hasher = ChunkedHasher::new();
        let mut written = 0_u64;
        let result = (|| -> Result<(), String> {
            let mut dst = File::create(&temp_path)
                .map_err(|error| format!("创建临时文件失败 {}：{error}", temp_path.display()))?;

            // 1) 原内容（签名块之前）。
            stream_and_hash(
                &mut src,
                &mut dst,
                0,
                layout.sb_start,
                &mut hasher,
                on_progress,
                STAGE_WRITE,
                &mut written,
                total_out,
            )?;

            // 2) v1 条目（MANIFEST.MF / CERT.SF / CERT.RSA，位于签名块之前，参与摘要）。
            if let Some(v1) = &v1 {
                dst.write_all(&v1.entries_bytes)
                    .map_err(|error| error.to_string())?;
                hasher.update(&v1.entries_bytes);
                written += v1_bytes_len;
                on_progress(STAGE_WRITE, "bytes", written, total_out);
            }

            // 3) 签名块占位（不属于摘要范围，签名后回填）。
            let block_offset = written;
            dst.write_all(&vec![0u8; block_len as usize])
                .map_err(|error| error.to_string())?;
            written += block_len;
            on_progress(STAGE_WRITE, "bytes", written, total_out);

            // 4) 原中央目录（内容区与中央目录区分块边界）。
            hasher.flush_boundary();
            stream_and_hash(
                &mut src,
                &mut dst,
                layout.cd_start,
                layout.cde_start,
                &mut hasher,
                on_progress,
                STAGE_WRITE,
                &mut written,
                total_out,
            )?;

            // 5) v1 条目的中央目录记录。
            if let Some(v1) = &v1 {
                dst.write_all(&v1.central_records)
                    .map_err(|error| error.to_string())?;
                hasher.update(&v1.central_records);
                written += v1_records_len;
                on_progress(STAGE_WRITE, "bytes", written, total_out);
            }
            hasher.flush_boundary();

            // 6) 重建 EOCD：条目数/目录长度/真实目录偏移。
            let mut eocd = Vec::new();
            src.seek(SeekFrom::Start(layout.cde_start))
                .map_err(|error| error.to_string())?;
            src.read_to_end(&mut eocd).map_err(|error| error.to_string())?;
            if (eocd.len() as u64) < EOCD_MIN_SIZE {
                return Err("zip 结尾记录不完整".to_string());
            }
            let entry_count = u16::from_le_bytes([eocd[10], eocd[11]])
                .checked_add(v1_count)
                .ok_or_else(|| "条目数超出限制".to_string())?;
            eocd[8..10].copy_from_slice(&entry_count.to_le_bytes());
            eocd[10..12].copy_from_slice(&entry_count.to_le_bytes());
            let cd_size =
                u32::from_le_bytes([eocd[12], eocd[13], eocd[14], eocd[15]]) as u64 + v1_records_len;
            eocd[12..16].copy_from_slice(&(cd_size as u32).to_le_bytes());
            eocd[16..20].copy_from_slice(&(cd_offset_in_file as u32).to_le_bytes());
            dst.write_all(&eocd).map_err(|error| error.to_string())?;
            written += eocd.len() as u64;
            on_progress(STAGE_WRITE, "bytes", written.min(total_out), total_out);

            // 摘要输入中的 EOCD：中央目录偏移改写为签名块起点。
            let mut eocd_for_digest = eocd.clone();
            eocd_for_digest[16..20].copy_from_slice(&(new_sb_start as u32).to_le_bytes());
            let digest = hasher.finalize(&eocd_for_digest);

            // 7) 用真实摘要重建 v2/v3 pair 并回填单个签名块。
            let v2 = self.build_v2_pair(digest)?;
            let v3 = self.build_v3_pair(digest)?;
            if v2.len() as u64 + 12 != v2_len || v3.len() as u64 + 12 != v3_len {
                return Err("签名块长度在回填时发生变化".to_string());
            }
            let block = build_signature_block(&[
                (APK_SIGNING_BLOCK_V2_ID, &v2),
                (APK_SIGNING_BLOCK_V3_ID, &v3),
            ]);
            if block.len() as u64 != block_len {
                return Err("签名块容器长度在回填时发生变化".to_string());
            }
            dst.seek(SeekFrom::Start(block_offset))
                .map_err(|error| error.to_string())?;
            dst.write_all(&block).map_err(|error| error.to_string())?;
            dst.sync_all().map_err(|error| error.to_string())?;
            Ok(())
        })();

        drop(src);
        match result {
            Ok(()) => fs::rename(&temp_path, path).map_err(|error| {
                let _ = fs::remove_file(&temp_path);
                format!("替换签名 APK 失败：{error}")
            }),
            Err(error) => {
                let _ = fs::remove_file(&temp_path);
                Err(error)
            }
        }
    }

    /// 对任意字节做 SHA-256 + RSA PKCS#1 v1.5 签名。
    fn sign_bytes(&self, data: &[u8]) -> Result<Vec<u8>, String> {
        let hashed = Sha256::digest(data);
        self.key
            .sign(Pkcs1v15Sign::new::<Sha256>(), &hashed)
            .map_err(|error| format!("RSA 签名失败：{error}"))
    }

    /// 构造 v2 pair 数据（signers 区）。
    fn build_v2_pair(&self, digest: [u8; 32]) -> Result<Vec<u8>, String> {
        self.build_v2_like_signer(digest, None)
    }

    /// 构造 v3 pair 数据（signers 区，带 min/max SDK）。
    fn build_v3_pair(&self, digest: [u8; 32]) -> Result<Vec<u8>, String> {
        self.build_v2_like_signer(digest, Some((V3_MIN_SDK, V3_MAX_SDK)))
    }

    /// 构造 v2/v3 共用的 signer 区；`sdk_range` 有值时按 v3 布局附加 min/max SDK。
    fn build_v2_like_signer(
        &self,
        digest: [u8; 32],
        sdk_range: Option<(u32, u32)>,
    ) -> Result<Vec<u8>, String> {
        // 1. signed data（真正被签名的内容）：摘要 + 证书 [+ min/max SDK] + 附加属性（无）。
        let mut signed_data = Vec::new();
        push_u32(&mut signed_data, 12 + 32); // 摘要区总长（含单条 12 字节头部）
        push_u32(&mut signed_data, 8 + 32); // 单条摘要长度（不含本字段）
        push_u32(&mut signed_data, RSA_PKCS1V15_SHA2_256);
        push_u32(&mut signed_data, 32);
        signed_data.extend_from_slice(&digest);
        push_u32(&mut signed_data, 4 + self.cert_der.len() as u32); // 证书区总长
        push_u32(&mut signed_data, self.cert_der.len() as u32);
        signed_data.extend_from_slice(&self.cert_der);
        if let Some((min_sdk, max_sdk)) = sdk_range {
            push_u32(&mut signed_data, min_sdk);
            push_u32(&mut signed_data, max_sdk);
        }
        push_u32(&mut signed_data, 0); // 附加属性：无

        let signature = self.sign_bytes(&signed_data)?;

        // 2. signer：signed data [+ min/max SDK] + 签名 + 公钥。
        let mut signer = Vec::new();
        push_u32(&mut signer, signed_data.len() as u32);
        signer.extend_from_slice(&signed_data);
        if let Some((min_sdk, max_sdk)) = sdk_range {
            push_u32(&mut signer, min_sdk);
            push_u32(&mut signer, max_sdk);
        }
        push_u32(&mut signer, signature.len() as u32 + 12); // 签名区总长（含单条 4 字节长度前缀）
        push_u32(&mut signer, signature.len() as u32 + 8); // 单条签名条目长度
        push_u32(&mut signer, RSA_PKCS1V15_SHA2_256);
        push_u32(&mut signer, signature.len() as u32);
        signer.extend_from_slice(&signature);
        push_u32(&mut signer, self.pubkey_der.len() as u32);
        signer.extend_from_slice(&self.pubkey_der);

        // 3. signers 区（长度前缀 + 单条 signer）。
        let mut data = Vec::new();
        push_u32(&mut data, signer.len() as u32 + 4);
        push_u32(&mut data, signer.len() as u32);
        data.extend_from_slice(&signer);
        Ok(data)
    }
}

/// 组装 APK 签名块容器（单个块内可含 v2/v3 多个 pair）：
/// `[size][pair…][size][magic]`。
fn build_signature_block(pairs: &[(u32, &[u8])]) -> Vec<u8> {
    let pairs_len: u64 = pairs.iter().map(|(_, data)| data.len() as u64 + 12).sum();
    let container_size = pairs_len + 24;
    let mut block = Vec::with_capacity(pairs_len as usize + 32);
    push_u64(&mut block, container_size);
    for (id, data) in pairs {
        push_u64(&mut block, data.len() as u64 + 4); // 该 pair 的值长度（id + 数据）
        push_u32(&mut block, *id);
        block.extend_from_slice(data);
    }
    push_u64(&mut block, container_size);
    block.extend_from_slice(APK_SIGNING_BLOCK_MAGIC);
    block
}

/// zip/apk 的关键偏移：签名块插入位置（sb_start）、中央目录与 EOCD 的位置。
struct ZipLayout {
    sb_start: u64,
    cd_start: u64,
    cde_start: u64,
}

/// 定位签名块插入点：找到 EOCD 与中央目录，并跳过已存在的旧签名块。
fn locate_zip_layout(src: &mut File) -> Result<ZipLayout, String> {
    let file_len = src
        .seek(SeekFrom::End(0))
        .map_err(|error| error.to_string())?;
    if file_len < EOCD_MIN_SIZE {
        return Err("文件过小，不是有效的 APK/zip".to_string());
    }

    // 从尾部向前扫描 EOCD 签名（注释最长 65535 字节）。
    let lower_bound = file_len.saturating_sub(EOCD_MIN_SIZE + u16::MAX as u64);
    let mut pos = file_len - EOCD_MIN_SIZE;
    let cde_start = loop {
        src.seek(SeekFrom::Start(pos))
            .map_err(|error| error.to_string())?;
        let mut signature = [0u8; 4];
        src.read_exact(&mut signature)
            .map_err(|error| format!("读取 EOCD 签名失败（pos={pos}）：{error}"))?;
        if signature == EOCD_SIGNATURE {
            break pos;
        }
        if pos <= lower_bound {
            return Err("未找到 zip 中央目录结尾，文件可能已损坏".to_string());
        }
        pos -= 1;
    };

    src.seek(SeekFrom::Start(cde_start + 16))
        .map_err(|error| error.to_string())?;
    let cd_start = read_u32_le(src)? as u64;
    let mut layout = ZipLayout {
        sb_start: cd_start,
        cd_start,
        cde_start,
    };

    // 中央目录前若已存在签名块（[size][pairs][size][magic]），
    // 其起点位于尾部 size 字段之前 `size + 8` 字节处。
    if cd_start >= 24 {
        src.seek(SeekFrom::Start(cd_start - 24))
            .map_err(|error| error.to_string())?;
        let content_size = read_u64_le(src)?;
        let mut magic = [0u8; 16];
        src.read_exact(&mut magic)
            .map_err(|error| format!("读取签名块 magic 失败（cd_start={cd_start}）：{error}"))?;
        if &magic == APK_SIGNING_BLOCK_MAGIC {
            if let Some(sb_start) = cd_start.checked_sub(content_size + 8) {
                layout.sb_start = sb_start;
            }
        }
    }
    Ok(layout)
}

/// v2/v3 内容摘要器：按 1MB 分块（0xa5 + 长度 + 数据），以汇总块（0x5a + 块数 + 块摘要）收官。
///
/// 内容区与中央目录区是两条独立的分块序列，需在边界处调用 `flush_boundary`；
/// EOCD（中央目录偏移已改写为签名块起点）作为最后一块由 `finalize` 处理。
struct ChunkedHasher {
    chunk_digests: Vec<[u8; 32]>,
    buffer: Vec<u8>,
}

impl ChunkedHasher {
    fn new() -> Self {
        Self {
            chunk_digests: Vec::new(),
            buffer: Vec::with_capacity(CHUNK_SIZE),
        }
    }

    fn update(&mut self, mut data: &[u8]) {
        while !data.is_empty() {
            let room = CHUNK_SIZE - self.buffer.len();
            let take = room.min(data.len());
            self.buffer.extend_from_slice(&data[..take]);
            data = &data[take..];
            if self.buffer.len() == CHUNK_SIZE {
                self.flush_chunk();
            }
        }
    }

    /// 结束当前区段（把残留数据作为一块收尾）。
    fn flush_boundary(&mut self) {
        if !self.buffer.is_empty() {
            self.flush_chunk();
        }
    }

    fn flush_chunk(&mut self) {
        let mut hasher = Sha256::new();
        hasher.update([0xa5]);
        hasher.update((self.buffer.len() as u32).to_le_bytes());
        hasher.update(&self.buffer);
        self.chunk_digests.push(hasher.finalize().into());
        self.buffer.clear();
    }

    /// 把 EOCD 作为最后一块并输出根摘要。
    fn finalize(mut self, eocd: &[u8]) -> [u8; 32] {
        self.flush_boundary();
        let mut hasher = Sha256::new();
        hasher.update([0xa5]);
        hasher.update((eocd.len() as u32).to_le_bytes());
        hasher.update(eocd);
        self.chunk_digests.push(hasher.finalize().into());

        let mut hasher = Sha256::new();
        hasher.update([0x5a]);
        hasher.update((self.chunk_digests.len() as u32).to_le_bytes());
        for chunk in &self.chunk_digests {
            hasher.update(chunk);
        }
        hasher.finalize().into()
    }
}

/// 从源读取 `[from, to)`：写入目标并喂给摘要器，同时上报字节进度。
#[allow(clippy::too_many_arguments)]
fn stream_and_hash(
    src: &mut File,
    dst: &mut File,
    from: u64,
    to: u64,
    hasher: &mut ChunkedHasher,
    on_progress: &(dyn Fn(&'static str, &'static str, u64, u64) + Sync),
    stage: &'static str,
    written: &mut u64,
    total: u64,
) -> Result<(), String> {
    src.seek(SeekFrom::Start(from))
        .map_err(|error| error.to_string())?;
    let mut remaining = to.saturating_sub(from);
    let mut buffer = vec![0u8; 256 * 1024];
    while remaining > 0 {
        let take = remaining.min(buffer.len() as u64) as usize;
        src.read_exact(&mut buffer[..take])
            .map_err(|error| format!("读取失败（from={from}, to={to}）：{error}"))?;
        dst.write_all(&buffer[..take])
            .map_err(|error| error.to_string())?;
        hasher.update(&buffer[..take]);
        remaining -= take as u64;
        *written += take as u64;
        on_progress(stage, "bytes", *written, total);
    }
    Ok(())
}

/// 中央目录条目（仅签名所需字段）。
struct CentralEntry {
    name: String,
    method: u16,
    comp_size: u64,
    lfh_offset: u64,
}

/// 读取中央目录全部条目。
fn read_central_directory(src: &mut File, layout: &ZipLayout) -> Result<Vec<CentralEntry>, String> {
    let mut entries = Vec::new();
    let mut pos = layout.cd_start;
    while pos + 46 <= layout.cde_start {
        src.seek(SeekFrom::Start(pos))
            .map_err(|error| error.to_string())?;
        let mut head = [0u8; 46];
        src.read_exact(&mut head)
            .map_err(|error| error.to_string())?;
        if u32::from_le_bytes([head[0], head[1], head[2], head[3]]) != CDFH_SIGNATURE {
            break;
        }
        let name_len = u16::from_le_bytes([head[28], head[29]]) as usize;
        let extra_len = u16::from_le_bytes([head[30], head[31]]) as usize;
        let comment_len = u16::from_le_bytes([head[32], head[33]]) as usize;
        let mut name_bytes = vec![0u8; name_len];
        src.read_exact(&mut name_bytes)
            .map_err(|error| error.to_string())?;
        entries.push(CentralEntry {
            name: String::from_utf8_lossy(&name_bytes).into_owned(),
            method: u16::from_le_bytes([head[10], head[11]]),
            comp_size: u32::from_le_bytes([head[20], head[21], head[22], head[23]]) as u64,
            lfh_offset: u32::from_le_bytes([head[42], head[43], head[44], head[45]]) as u64,
        });
        pos += (46 + name_len + extra_len + comment_len) as u64;
    }
    Ok(entries)
}

/// v1 签名产物：3 个 zip 条目（LFH + 数据）与其中央目录记录。
struct V1Entries {
    entries_bytes: Vec<u8>,
    central_records: Vec<u8>,
}

impl ApkSigningKey {
    /// 生成 v1（JAR）签名条目：MANIFEST.MF + CERT.SF + CERT.RSA。
    fn build_v1_entries(
        &self,
        path: &Path,
        entries: &[CentralEntry],
        base_offset: u64,
        on_progress: &(dyn Fn(&'static str, &'static str, u64, u64) + Sync),
    ) -> Result<V1Entries, String> {
        const STAGE: &str = "正在计算 v1 条目摘要";
        let signable: Vec<&CentralEntry> = entries
            .iter()
            .filter(|entry| is_v1_signable(&entry.name))
            .collect();
        let total = signable.len() as u64;
        on_progress(STAGE, "files", 0, total);
        let completed = AtomicU64::new(0);
        let results: Vec<Result<(String, [u8; 32]), String>> = signable
            .par_iter()
            .map_init(
                || File::open(path),
                |handle, entry| {
                    let result = match handle {
                        Ok(handle) => compute_entry_digest(handle, entry),
                        Err(error) => Err(format!("打开 APK 失败：{error}")),
                    };
                    let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
                    if done % 64 == 0 || done == total {
                        on_progress(STAGE, "files", done, total);
                    }
                    result
                },
            )
            .collect();
        let mut items = Vec::with_capacity(results.len());
        for result in results {
            items.push(result?);
        }
        items.sort_by(|left, right| left.0.cmp(&right.0));

        // MANIFEST.MF：主属性 + 每条目 SHA-256 摘要；同时记录每个条目的区段范围，
        // 供 CERT.SF 生成逐条目区段摘要（与 jarsigner 产物对齐，区段含尾随空行）。
        let mut manifest = Vec::new();
        manifest.extend_from_slice(b"Manifest-Version: 1.0\r\nCreated-By: AgeCivModTool\r\n\r\n");
        let main_attrs_len = manifest.len();
        let mut sections: Vec<(&str, usize, usize)> = Vec::with_capacity(items.len());
        for (name, digest) in &items {
            let start = manifest.len();
            push_manifest_line(&mut manifest, &format!("Name: {name}"));
            push_manifest_line(
                &mut manifest,
                &format!("SHA-256-Digest: {}", STANDARD.encode(digest)),
            );
            manifest.extend_from_slice(b"\r\n");
            sections.push((name.as_str(), start, manifest.len()));
        }

        // CERT.SF：整体清单摘要 + 主属性区摘要 + 逐条目区段摘要。
        // X-Android-APK-Signed 声明同时使用 v2/v3，防止仅校验 v1 的平台被剥离签名攻击。
        let manifest_digest = Sha256::digest(&manifest);
        let main_attrs_digest = Sha256::digest(&manifest[..main_attrs_len]);
        let mut sf = Vec::new();
        sf.extend_from_slice(b"Signature-Version: 1.0\r\n");
        sf.extend_from_slice(b"Created-By: AgeCivModTool\r\n");
        sf.extend_from_slice(b"X-Android-APK-Signed: 2, 3\r\n");
        push_manifest_line(
            &mut sf,
            &format!(
                "SHA-256-Digest-Manifest: {}",
                STANDARD.encode(manifest_digest)
            ),
        );
        push_manifest_line(
            &mut sf,
            &format!(
                "SHA-256-Digest-Manifest-Main-Attributes: {}",
                STANDARD.encode(main_attrs_digest)
            ),
        );
        sf.extend_from_slice(b"\r\n");
        for (name, start, end) in &sections {
            let section_digest = Sha256::digest(&manifest[*start..*end]);
            push_manifest_line(&mut sf, &format!("Name: {name}"));
            push_manifest_line(
                &mut sf,
                &format!("SHA-256-Digest: {}", STANDARD.encode(section_digest)),
            );
            sf.extend_from_slice(b"\r\n");
        }

        // CERT.RSA：对 CERT.SF 的最小 PKCS#7 分离签名。
        let sf_signature = self.sign_bytes(&sf)?;
        let rsa = build_pkcs7_signed_data(&self.cert_der, &sf_signature)?;

        // 组装存储型 zip 条目（LFH + 数据）与中央目录记录。
        let mut entries_bytes = Vec::new();
        let mut central_records = Vec::new();
        let mut offset = base_offset;
        for (name, data) in [
            (V1_MANIFEST_NAME, &manifest),
            (V1_SF_NAME, &sf),
            (V1_RSA_NAME, &rsa),
        ] {
            let crc = crc32fast::hash(data);
            let name_bytes = name.as_bytes();
            entries_bytes.extend_from_slice(&LFH_SIGNATURE.to_le_bytes());
            entries_bytes.extend_from_slice(&20u16.to_le_bytes()); // version needed
            entries_bytes.extend_from_slice(&0u16.to_le_bytes()); // flags
            entries_bytes.extend_from_slice(&0u16.to_le_bytes()); // method: stored
            entries_bytes.extend_from_slice(&0u16.to_le_bytes()); // time
            entries_bytes.extend_from_slice(&0x21u16.to_le_bytes()); // date: 1980-01-01
            entries_bytes.extend_from_slice(&crc.to_le_bytes());
            entries_bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
            entries_bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
            entries_bytes.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
            entries_bytes.extend_from_slice(&0u16.to_le_bytes()); // extra
            entries_bytes.extend_from_slice(name_bytes);
            entries_bytes.extend_from_slice(data);

            central_records.extend_from_slice(&CDFH_SIGNATURE.to_le_bytes());
            central_records.extend_from_slice(&20u16.to_le_bytes()); // made by
            central_records.extend_from_slice(&20u16.to_le_bytes()); // needed
            central_records.extend_from_slice(&0u16.to_le_bytes()); // flags
            central_records.extend_from_slice(&0u16.to_le_bytes()); // method
            central_records.extend_from_slice(&0u16.to_le_bytes()); // time
            central_records.extend_from_slice(&0x21u16.to_le_bytes()); // date
            central_records.extend_from_slice(&crc.to_le_bytes());
            central_records.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central_records.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central_records.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
            central_records.extend_from_slice(&0u16.to_le_bytes()); // extra
            central_records.extend_from_slice(&0u16.to_le_bytes()); // comment
            central_records.extend_from_slice(&0u16.to_le_bytes()); // disk
            central_records.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
            central_records.extend_from_slice(&0u32.to_le_bytes()); // external attrs
            central_records.extend_from_slice(&(offset as u32).to_le_bytes());
            central_records.extend_from_slice(name_bytes);

            offset += (30 + name_bytes.len() + data.len()) as u64;
        }

        Ok(V1Entries {
            entries_bytes,
            central_records,
        })
    }
}

/// v1 需要签名（列入 MANIFEST）的条目：跳过目录与 META-INF 下的签名相关文件。
fn is_v1_signable(name: &str) -> bool {
    if name.ends_with('/') {
        return false;
    }
    let Some(rest) = name.strip_prefix("META-INF/") else {
        return true;
    };
    let upper = rest.to_ascii_uppercase();
    !(rest.eq_ignore_ascii_case("MANIFEST.MF")
        || upper.ends_with(".SF")
        || upper.ends_with(".RSA")
        || upper.ends_with(".DSA")
        || upper.ends_with(".EC")
        || upper.starts_with("SIG-"))
}

/// 计算单个条目未压缩内容的 SHA-256（与 JAR 校验口径一致）。
fn compute_entry_digest(
    handle: &mut File,
    entry: &CentralEntry,
) -> Result<(String, [u8; 32]), String> {
    handle
        .seek(SeekFrom::Start(entry.lfh_offset))
        .map_err(|error| error.to_string())?;
    let mut head = [0u8; 30];
    handle
        .read_exact(&mut head)
        .map_err(|error| format!("读取本地文件头失败：{error}"))?;
    if u32::from_le_bytes([head[0], head[1], head[2], head[3]]) != LFH_SIGNATURE {
        return Err("本地文件头签名无效".to_string());
    }
    let name_len = u16::from_le_bytes([head[26], head[27]]) as u64;
    let extra_len = u16::from_le_bytes([head[28], head[29]]) as u64;
    handle
        .seek(SeekFrom::Start(entry.lfh_offset + 30 + name_len + extra_len))
        .map_err(|error| error.to_string())?;
    let mut limited = Read::take(handle, entry.comp_size);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 256 * 1024];
    match entry.method {
        0 => hash_reader(&mut limited, &mut hasher, &mut buffer)?,
        8 => {
            let mut decoder = flate2::read::DeflateDecoder::new(limited);
            hash_reader(&mut decoder, &mut hasher, &mut buffer)?;
        }
        other => return Err(format!("不支持的压缩方式 {other}")),
    }
    Ok((entry.name.clone(), hasher.finalize().into()))
}

fn hash_reader(
    reader: &mut impl Read,
    hasher: &mut Sha256,
    buffer: &mut [u8],
) -> Result<(), String> {
    loop {
        let read = reader.read(buffer).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(())
}

/// 按 JAR Manifest 规范写入一行：首行最多 72 个字符，续行以单个空格开头且内容最多 71 个字符。
/// 与 Java 行为一致地按“字符”而非字节折行，避免切断 UTF-8 多字节字符（每行仍逐行 UTF-8 解码）。
fn push_manifest_line(out: &mut Vec<u8>, line: &str) {
    if line.is_empty() {
        out.extend_from_slice(b"\r\n");
        return;
    }
    let mut chars = line.chars().peekable();
    let mut first = true;
    while chars.peek().is_some() {
        if !first {
            out.push(b' ');
        }
        let limit = if first { 72 } else { 71 };
        for _ in 0..limit {
            match chars.next() {
                Some(character) => {
                    let mut buffer = [0u8; 4];
                    out.extend_from_slice(character.encode_utf8(&mut buffer).as_bytes());
                }
                None => break,
            }
        }
        out.extend_from_slice(b"\r\n");
        first = false;
    }
}

// ============================== DER / PKCS#7 ==============================

/// DER 编码的 OID：1.2.840.113549.1.7.2（signedData）。
const OID_SIGNED_DATA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x07, 0x02];
/// 1.2.840.113549.1.7.1（data）。
const OID_DATA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x07, 0x01];
/// 2.16.840.1.101.3.4.2.1（SHA-256）。
const OID_SHA256: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01];
/// 1.2.840.113549.1.1.11（sha256WithRSAEncryption）。
const OID_SHA256_RSA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0b];

/// 构造 v1 的 CERT.RSA 内容：最小 PKCS#7（CMS）分离签名。
fn build_pkcs7_signed_data(cert_der: &[u8], signature: &[u8]) -> Result<Vec<u8>, String> {
    let (issuer_der, serial) = parse_certificate_issuer_serial(cert_der)?;

    let mut issuer_and_serial = Vec::new();
    issuer_and_serial.extend_from_slice(&issuer_der);
    issuer_and_serial.extend_from_slice(&der_int(&serial));
    let issuer_and_serial = der_wrap(0x30, &issuer_and_serial);

    let mut signer_info_body = Vec::new();
    signer_info_body.extend_from_slice(&der_int_u64(1)); // version
    signer_info_body.extend_from_slice(&issuer_and_serial);
    signer_info_body.extend_from_slice(&der_alg(OID_SHA256)); // digestAlgorithm
    signer_info_body.extend_from_slice(&der_alg(OID_SHA256_RSA)); // signatureAlgorithm
    signer_info_body.extend_from_slice(&der_wrap(0x04, signature)); // signature
    let signer_info = der_wrap(0x30, &signer_info_body);

    let mut signed_data_body = Vec::new();
    signed_data_body.extend_from_slice(&der_int_u64(1)); // version
    signed_data_body.extend_from_slice(&der_wrap(0x31, &der_alg(OID_SHA256))); // digestAlgorithms SET
    signed_data_body.extend_from_slice(&der_wrap(0x30, &der_oid(OID_DATA))); // encapContentInfo
    signed_data_body.extend_from_slice(&der_wrap(0xA0, cert_der)); // certificates
    signed_data_body.extend_from_slice(&der_wrap(0x31, &signer_info)); // signerInfos SET
    let signed_data = der_wrap(0x30, &signed_data_body);

    let mut content_info = Vec::new();
    content_info.extend_from_slice(&der_oid(OID_SIGNED_DATA));
    content_info.extend_from_slice(&der_wrap(0xA0, &signed_data));
    Ok(der_wrap(0x30, &content_info))
}

/// 从 X.509 证书 DER 中取出签发者名称的完整 DER 与序列号（大端值字节）。
fn parse_certificate_issuer_serial(cert_der: &[u8]) -> Result<(Vec<u8>, Vec<u8>), String> {
    let (tag, body_start, _) = read_der_tlv(cert_der, 0)?;
    if tag != 0x30 {
        return Err("证书不是 SEQUENCE".to_string());
    }
    let (tag, tbs_start, _) = read_der_tlv(cert_der, body_start)?;
    if tag != 0x30 {
        return Err("证书缺少 tbsCertificate".to_string());
    }
    let mut pos = tbs_start;
    let (tag, _, end) = read_der_tlv(cert_der, pos)?;
    if tag == 0xA0 {
        pos = end; // 跳过 [0] version
    }
    let (tag, serial_start, serial_end) = read_der_tlv(cert_der, pos)?;
    if tag != 0x02 {
        return Err("证书缺少序列号".to_string());
    }
    let serial = cert_der[serial_start..serial_end].to_vec();
    pos = serial_end;
    let (_, _, end) = read_der_tlv(cert_der, pos)?; // 跳过 signature AlgorithmIdentifier
    pos = end;
    let (tag, _, end) = read_der_tlv(cert_der, pos)?;
    if tag != 0x30 {
        return Err("证书缺少签发者名称".to_string());
    }
    let issuer = cert_der[pos..end].to_vec();
    Ok((issuer, serial))
}

/// 读取一个 DER TLV：返回（tag，内容起点，TLV 终点）。
fn read_der_tlv(data: &[u8], pos: usize) -> Result<(u8, usize, usize), String> {
    if pos >= data.len() {
        return Err("DER 越界".to_string());
    }
    let tag = data[pos];
    let mut cursor = pos + 1;
    if cursor >= data.len() {
        return Err("DER 长度缺失".to_string());
    }
    let first = data[cursor];
    cursor += 1;
    let len = if first < 0x80 {
        first as usize
    } else {
        let count = (first & 0x7f) as usize;
        if count == 0 || count > 4 || cursor + count > data.len() {
            return Err("DER 长度无效".to_string());
        }
        let mut len = 0usize;
        for byte in &data[cursor..cursor + count] {
            len = (len << 8) | *byte as usize;
        }
        cursor += count;
        len
    };
    let body_start = cursor;
    let body_end = body_start + len;
    if body_end > data.len() {
        return Err("DER 内容越界".to_string());
    }
    Ok((tag, body_start, body_end))
}

fn der_wrap(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 6);
    out.push(tag);
    push_der_len(&mut out, body.len());
    out.extend_from_slice(body);
    out
}

fn push_der_len(out: &mut Vec<u8>, len: usize) {
    if len < 0x80 {
        out.push(len as u8);
    } else if len <= 0xFFFF {
        out.push(0x82);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        out.push(0x84);
        out.extend_from_slice(&(len as u32).to_be_bytes());
    }
}

fn der_oid(bytes: &[u8]) -> Vec<u8> {
    der_wrap(0x06, bytes)
}

fn der_alg(oid: &[u8]) -> Vec<u8> {
    der_wrap(0x30, &der_oid(oid))
}

/// 正数 INTEGER（值字节，自动补符号位）。
fn der_int(value: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(value.len() + 1);
    if value.first().is_some_and(|byte| byte & 0x80 != 0) {
        body.push(0);
    }
    body.extend_from_slice(value);
    der_wrap(0x02, &body)
}

/// 正数 INTEGER（u64，自动去掉多余前导零）。
fn der_int_u64(value: u64) -> Vec<u8> {
    let bytes = value.to_be_bytes();
    let start = bytes
        .iter()
        .position(|byte| *byte != 0)
        .unwrap_or(bytes.len() - 1);
    der_int(&bytes[start..])
}

/// 从 PEM 文本中提取指定标签的 DER 内容（简单解析，支持多块混排）。
fn pem_decode(pem: &str, tag: &str) -> Option<Vec<u8>> {
    let begin = format!("-----BEGIN {tag}-----");
    let end = format!("-----END {tag}-----");
    let start = pem.find(&begin)? + begin.len();
    let tail = &pem[start..];
    let stop = tail.find(&end)?;
    let body: String = tail[..stop]
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect();
    STANDARD.decode(body).ok()
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn read_u32_le(reader: &mut impl Read) -> Result<u32, String> {
    let mut buffer = [0u8; 4];
    reader
        .read_exact(&mut buffer)
        .map_err(|error| error.to_string())?;
    Ok(u32::from_le_bytes(buffer))
}

fn read_u64_le(reader: &mut impl Read) -> Result<u64, String> {
    let mut buffer = [0u8; 8];
    reader
        .read_exact(&mut buffer)
        .map_err(|error| error.to_string())?;
    Ok(u64::from_le_bytes(buffer))
}

/// 在常见位置查找本机 apksigner（找不到返回 None，仅供测试使用）。
#[cfg(test)]
pub(crate) fn find_apksigner() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    let mut roots = Vec::new();
    for variable in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Ok(value) = std::env::var(variable) {
            roots.push(PathBuf::from(value));
        }
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        roots.push(PathBuf::from(local).join("Android").join("Sdk"));
    }
    roots.push(PathBuf::from(r"A:\AndroidSdk"));
    for root in roots {
        let Ok(entries) = fs::read_dir(root.join("build-tools")) else {
            continue;
        };
        let mut versions: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect();
        versions.sort();
        for version in versions.into_iter().rev() {
            for name in ["apksigner.bat", "apksigner"] {
                let candidate = version.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use rsa::pkcs8::DecodePublicKey;
    use std::io::Cursor;
    use std::path::PathBuf;
    use zip::write::SimpleFileOptions;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ageciv-apk-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 构造一个最小可签名的测试 APK（普通 zip）。
    fn build_test_apk(path: &Path) {
        let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
        let options = SimpleFileOptions::default();
        zip.start_file("AndroidManifest.xml", options).unwrap();
        zip.write_all(b"<manifest/>").unwrap();
        zip.start_file("classes.dex", options).unwrap();
        zip.write_all(b"not-a-real-dex").unwrap();
        zip.start_file("resources.arsc", options).unwrap();
        zip.write_all(&[0u8; 64]).unwrap();
        zip.finish().unwrap();
    }

    /// 测试用：解析 v2 签名块，校验 RSA 签名与内容摘要。
    fn verify_v2_block(path: &Path) -> Result<(), String> {
        let mut src = File::open(path).map_err(|error| error.to_string())?;
        let layout = locate_zip_layout(&mut src)?;
        if layout.sb_start >= layout.cd_start {
            return Err("未找到签名块".to_string());
        }

        // 扫描顶层 pair，取出 v2 数据。
        let mut pos = layout.sb_start + 8;
        let mut v2_value = None;
        while pos + 12 <= layout.cd_start {
            src.seek(SeekFrom::Start(pos))
                .map_err(|error| error.to_string())?;
            let length = read_u64_le(&mut src)?;
            let id = read_u32_le(&mut src)?;
            if length < 4 {
                break;
            }
            if id == APK_SIGNING_BLOCK_V2_ID {
                let mut value = vec![0u8; (length - 4) as usize];
                src.read_exact(&mut value)
                    .map_err(|error| error.to_string())?;
                v2_value = Some(value);
                break;
            }
            pos += 8 + length;
        }
        let value = v2_value.ok_or_else(|| "缺少 v2 签名数据".to_string())?;

        // 解析 signer：摘要 + 签名 + 公钥。
        let mut cursor = Cursor::new(value.as_slice());
        let _signers_total = read_u32_le(&mut cursor)?;
        let _signer_len = read_u32_le(&mut cursor)?;
        let signed_data_len = read_u32_le(&mut cursor)?;
        let mut signed_data = vec![0u8; signed_data_len as usize];
        cursor
            .read_exact(&mut signed_data)
            .map_err(|error| error.to_string())?;
        let signatures_total = read_u32_le(&mut cursor)?;
        let signature_entry_len = read_u32_le(&mut cursor)?;
        let algorithm = read_u32_le(&mut cursor)?;
        let signature_len = read_u32_le(&mut cursor)?;
        let mut signature = vec![0u8; signature_len as usize];
        cursor
            .read_exact(&mut signature)
            .map_err(|error| error.to_string())?;
        let public_key_len = read_u32_le(&mut cursor)?;
        let mut public_key = vec![0u8; public_key_len as usize];
        cursor
            .read_exact(&mut public_key)
            .map_err(|error| error.to_string())?;

        assert_eq!(algorithm, RSA_PKCS1V15_SHA2_256, "签名算法应为 RSA PKCS#1 v1.5 SHA-256");
        assert_eq!(signature_entry_len, signature.len() as u32 + 8);
        assert_eq!(signatures_total, signature.len() as u32 + 12);

        // RSA 验签。
        let pubkey = RsaPublicKey::from_public_key_der(&public_key)
            .map_err(|error| format!("解析公钥失败：{error}"))?;
        let hashed = Sha256::digest(&signed_data);
        pubkey
            .verify(Pkcs1v15Sign::new::<Sha256>(), &hashed, &signature)
            .map_err(|error| format!("签名校验失败：{error}"))?;

        // 摘要对比：文件当前内容重新计算后应等于签名中记录的摘要。
        let mut sd = Cursor::new(signed_data.as_slice());
        let _digests_total = read_u32_le(&mut sd)?;
        let _digest_entry_len = read_u32_le(&mut sd)?;
        let digest_alg = read_u32_le(&mut sd)?;
        let digest_len = read_u32_le(&mut sd)?;
        let mut digest = vec![0u8; digest_len as usize];
        sd.read_exact(&mut digest).map_err(|error| error.to_string())?;
        assert_eq!(digest_alg, RSA_PKCS1V15_SHA2_256);
        let recomputed = recompute_digest(&mut src, &layout)?;
        assert_eq!(digest, recomputed.to_vec(), "摘要应与重新计算的一致");
        Ok(())
    }

    /// 测试用：按 v2/v3 规范重算内容摘要（内容区/中央目录区独立分块 + EOCD 偏移改写）。
    fn recompute_digest(src: &mut File, layout: &ZipLayout) -> Result<[u8; 32], String> {
        let mut hasher = ChunkedHasher::new();
        let mut buffer = vec![0u8; 256 * 1024];
        for (from, to) in [
            (0_u64, layout.sb_start),
            (layout.cd_start, layout.cde_start),
        ] {
            src.seek(SeekFrom::Start(from))
                .map_err(|error| error.to_string())?;
            let mut remaining = to.saturating_sub(from);
            while remaining > 0 {
                let take = remaining.min(buffer.len() as u64) as usize;
                src.read_exact(&mut buffer[..take])
                    .map_err(|error| error.to_string())?;
                hasher.update(&buffer[..take]);
                remaining -= take as u64;
            }
            hasher.flush_boundary();
        }
        let file_len = src
            .seek(SeekFrom::End(0))
            .map_err(|error| error.to_string())?;
        src.seek(SeekFrom::Start(layout.cde_start))
            .map_err(|error| error.to_string())?;
        let mut eocd = vec![0u8; (file_len - layout.cde_start) as usize];
        src.read_exact(&mut eocd)
            .map_err(|error| error.to_string())?;
        eocd[16..20].copy_from_slice(&(layout.sb_start as u32).to_le_bytes());
        Ok(hasher.finalize(&eocd))
    }

    #[test]
    fn builtin_key_loads() {
        let key = ApkSigningKey::builtin().expect("内置签名密钥应能加载");
        assert!(!key.pubkey_der.is_empty());
        assert!(!key.cert_der.is_empty());
    }

    #[test]
    fn sign_apk_writes_verifiable_v1_v2_v3() {
        let dir = temp_dir("sign");
        let apk = dir.join("test.apk");
        build_test_apk(&apk);
        let key = ApkSigningKey::builtin().unwrap();

        key.sign_apk(&apk).unwrap();
        verify_v1_entries(&apk).unwrap();
        verify_v2_block(&apk).unwrap();
        verify_v3_block(&apk).unwrap();

        // 重复签名（已有旧签名块与 v1 签名）应能正常替换并保持可校验。
        key.sign_apk(&apk).unwrap();
        verify_v1_entries(&apk).unwrap();
        verify_v2_block(&apk).unwrap();
        verify_v3_block(&apk).unwrap();

        let _ = fs::remove_dir_all(&dir);
    }

    /// 测试用：校验 v1 签名条目与 CERT.SF 三类摘要（主属性区、整体、逐条目区段）。
    fn verify_v1_entries(path: &Path) -> Result<(), String> {
        let mut archive =
            zip::ZipArchive::new(File::open(path).map_err(|error| error.to_string())?)
                .map_err(|error| error.to_string())?;
        let mut manifest = Vec::new();
        archive
            .by_name(V1_MANIFEST_NAME)
            .map_err(|error| error.to_string())?
            .read_to_end(&mut manifest)
            .map_err(|error| error.to_string())?;
        let manifest_text =
            String::from_utf8(manifest.clone()).map_err(|error| error.to_string())?;
        assert!(manifest_text.contains("Name: classes.dex"), "MANIFEST 缺少条目");
        assert!(manifest_text.contains("SHA-256-Digest:"), "MANIFEST 缺少条目摘要");
        let mut sf = String::new();
        archive
            .by_name(V1_SF_NAME)
            .map_err(|error| error.to_string())?
            .read_to_string(&mut sf)
            .map_err(|error| error.to_string())?;
        // 合并折行（续行以单个空格开头），便于直接匹配属性值。
        let sf = sf.replace("\r\n ", "");
        let mut rsa = Vec::new();
        archive
            .by_name(V1_RSA_NAME)
            .map_err(|error| error.to_string())?
            .read_to_end(&mut rsa)
            .map_err(|error| error.to_string())?;
        assert_eq!(rsa.first(), Some(&0x30), "CERT.RSA 应为 DER SEQUENCE");

        // 主属性区摘要：第一个条目区段之前（含尾随空行）。
        let first_entry = find_bytes(&manifest, b"Name: ").ok_or("MANIFEST 无条目区段")?;
        let main_digest = STANDARD.encode(Sha256::digest(&manifest[..first_entry]));
        assert!(
            sf.contains(&format!("SHA-256-Digest-Manifest-Main-Attributes: {main_digest}")),
            "CERT.SF 主属性摘要不匹配：{sf}"
        );
        // 整体清单摘要。
        let manifest_digest = STANDARD.encode(Sha256::digest(&manifest));
        assert!(
            sf.contains(&format!("SHA-256-Digest-Manifest: {manifest_digest}")),
            "CERT.SF 清单摘要不匹配"
        );
        // 逐条目区段摘要（含尾随空行）。
        for name in ["classes.dex", "resources.arsc", "AndroidManifest.xml"] {
            let start = find_bytes(&manifest, format!("Name: {name}\r\n").as_bytes())
                .ok_or_else(|| format!("MANIFEST 缺少 {name}"))?;
            let next = find_bytes(&manifest[start + 6..], b"Name: ")
                .map(|offset| start + 6 + offset)
                .unwrap_or(manifest.len());
            let section_digest = STANDARD.encode(Sha256::digest(&manifest[start..next]));
            assert!(
                sf.contains(&format!("Name: {name}\r\nSHA-256-Digest: {section_digest}")),
                "CERT.SF 条目区段摘要不匹配：{name}"
            );
        }
        Ok(())
    }

    /// 在字节串中查找子串首次出现的位置。
    fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
    }

    /// 测试用：解析 v3 签名块，校验 min/max SDK、RSA 签名与内容摘要。
    fn verify_v3_block(path: &Path) -> Result<(), String> {
        let mut src = File::open(path).map_err(|error| error.to_string())?;
        let layout = locate_zip_layout(&mut src)?;
        let mut value = None;
        let mut pos = layout.sb_start + 8;
        while pos + 12 <= layout.cd_start {
            src.seek(SeekFrom::Start(pos))
                .map_err(|error| error.to_string())?;
            let length = read_u64_le(&mut src)?;
            let id = read_u32_le(&mut src)?;
            if length < 4 {
                break;
            }
            if id == APK_SIGNING_BLOCK_V3_ID {
                let mut data = vec![0u8; (length - 4) as usize];
                src.read_exact(&mut data)
                    .map_err(|error| error.to_string())?;
                value = Some(data);
                break;
            }
            pos += 8 + length;
        }
        let value = value.ok_or_else(|| "缺少 v3 签名数据".to_string())?;
        let mut cursor = Cursor::new(value.as_slice());
        let _signers_total = read_u32_le(&mut cursor)?;
        let _signer_len = read_u32_le(&mut cursor)?;
        let signed_data_len = read_u32_le(&mut cursor)?;
        let mut signed_data = vec![0u8; signed_data_len as usize];
        cursor
            .read_exact(&mut signed_data)
            .map_err(|error| error.to_string())?;
        let min_sdk = read_u32_le(&mut cursor)?;
        let max_sdk = read_u32_le(&mut cursor)?;
        let _signatures_total = read_u32_le(&mut cursor)?;
        let _signature_entry_len = read_u32_le(&mut cursor)?;
        let algorithm = read_u32_le(&mut cursor)?;
        let signature_len = read_u32_le(&mut cursor)?;
        let mut signature = vec![0u8; signature_len as usize];
        cursor
            .read_exact(&mut signature)
            .map_err(|error| error.to_string())?;
        let public_key_len = read_u32_le(&mut cursor)?;
        let mut public_key = vec![0u8; public_key_len as usize];
        cursor
            .read_exact(&mut public_key)
            .map_err(|error| error.to_string())?;
        assert_eq!(min_sdk, V3_MIN_SDK);
        assert_eq!(max_sdk, V3_MAX_SDK);
        assert_eq!(algorithm, RSA_PKCS1V15_SHA2_256);

        let pubkey = RsaPublicKey::from_public_key_der(&public_key)
            .map_err(|error| format!("解析公钥失败：{error}"))?;
        let hashed = Sha256::digest(&signed_data);
        pubkey
            .verify(Pkcs1v15Sign::new::<Sha256>(), &hashed, &signature)
            .map_err(|error| format!("v3 签名校验失败：{error}"))?;

        let mut sd = Cursor::new(signed_data.as_slice());
        let _digests_total = read_u32_le(&mut sd)?;
        let _digest_entry_len = read_u32_le(&mut sd)?;
        let digest_alg = read_u32_le(&mut sd)?;
        let digest_len = read_u32_le(&mut sd)?;
        let mut digest = vec![0u8; digest_len as usize];
        sd.read_exact(&mut digest)
            .map_err(|error| error.to_string())?;
        assert_eq!(digest_alg, RSA_PKCS1V15_SHA2_256);
        let recomputed = recompute_digest(&mut src, &layout)?;
        assert_eq!(digest, recomputed.to_vec(), "v3 摘要应与重新计算的一致");
        Ok(())
    }

    /// 本机验证：用真实 apksigner 校验签名产物（v1/v2/v3 三方案）。
    /// 默认忽略：`cargo test -p age_civ_mod_tool --lib -- --ignored`
    #[test]
    #[ignore = "依赖本机 Android SDK build-tools 中的 apksigner"]
    fn apksigner_accepts_signature() {
        let Some(apksigner) = find_apksigner() else {
            println!("未找到 apksigner，跳过");
            return;
        };
        let dir = temp_dir("apksigner");
        let apk = dir.join("test.apk");
        build_test_apk(&apk);
        ApkSigningKey::builtin().unwrap().sign_apk(&apk).unwrap();

        // 测试 APK 的清单是假文本，无法解析 minSdkVersion，这里直接指定：
        // 低版本强制校验 v1，高版本校验 v2/v3。
        let verify = |min_sdk: &str| -> (bool, String) {
            let output = std::process::Command::new("cmd")
                .arg("/C")
                .arg(&apksigner)
                .arg("verify")
                .arg("--verbose")
                .arg("--min-sdk-version")
                .arg(min_sdk)
                .arg(&apk)
                .output()
                .expect("运行 apksigner 失败");
            let mut log = String::from_utf8_lossy(&output.stdout).into_owned();
            log.push_str(&String::from_utf8_lossy(&output.stderr));
            (output.status.success(), log)
        };

        let (ok_low, log_low) = verify("18");
        println!("[min-sdk 18]\n{log_low}");
        assert!(ok_low, "apksigner（min-sdk 18）verify 未通过");
        assert!(
            log_low.contains("Verified using v1 scheme (JAR signing): true"),
            "apksigner 未确认 v1 签名：{log_low}"
        );
        assert!(
            log_low.contains("Verified using v2 scheme (APK Signature Scheme v2): true"),
            "apksigner 未确认 v2 签名：{log_low}"
        );
        assert!(
            log_low.contains("Verified using v3 scheme (APK Signature Scheme v3): true"),
            "apksigner 未确认 v3 签名：{log_low}"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
