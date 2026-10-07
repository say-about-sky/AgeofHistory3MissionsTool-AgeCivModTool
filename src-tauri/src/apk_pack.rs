//! APK/zip 打包：流式写入 zip32（Stored / Deflated）。
//!
//! - 压缩阶段使用 rayon 多线程、按批次（限制内存占用）并行压缩；
//! - `resources.arsc` 不压缩并对齐 4 字节；`lib/*.so` 不压缩并对齐 16 KiB
//!   （通过本地头扩展字段填充实现，兼容 Android 10+ 与 16KB 页设备）；
//! - 支持字节级进度回调（打包命令用于进度条）；
//! - 固定时间戳，保证多次打包输出可复现。
//!
//! 只生成 zip32（条目数 < 65535、偏移 < 4GB），APK 场景足够；
//! 产物由既有 `apk_signing` 模块完成 v2 签名。

use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rayon::prelude::*;

const LFH_SIG: u32 = 0x0403_4b50;
const CDFH_SIG: u32 = 0x0201_4b50;
const EOCD_SIG: u32 = 0x0605_4b50;
const VERSION_NEEDED: u16 = 20;
const VERSION_MADE_BY: u16 = 20;
const FLAG_UTF8: u16 = 0x0800;
/// zipalign 使用的对齐填充扩展字段 ID。
const ALIGN_EXTRA_ID: u16 = 0xD935;
/// 固定时间戳（1980-01-01 00:00:00），保证输出可复现。
const DOS_TIME: u16 = 0;
const DOS_DATE: u16 = 0x21;
/// 单个并行压缩批次的原始字节上限，控制内存峰值。
const BATCH_BYTES: u64 = 24 * 1024 * 1024;
/// resources.arsc 对齐字节数（targetSdk 30+ 要求不压缩且 4 字节对齐）。
const ARSC_ALIGN: u64 = 4;
/// 原生库对齐字节数（16 KiB，兼容 Android 15 的 16KB 页设备）。
const SO_ALIGN: u64 = 16 * 1024;
const ZIP32_LIMIT: u64 = 0xFFFF_FFFF;

/// 打包进度回调：`completed` / `total` 为字节数。
pub type PackProgress<'a> = &'a (dyn Fn(u64, u64) + Sync);

/// 待打包的文件条目。
struct PlannedEntry {
    relative: String,
    source: PathBuf,
    /// 0 = Stored，8 = Deflated。
    method: u16,
    /// 数据区需要的对齐字节数（1 表示不需要对齐）。
    align: u64,
    size: u64,
}

/// 并行压缩结果。
struct CompressedEntry {
    crc: u32,
    compressed: Vec<u8>,
    uncompressed_size: u64,
}

/// 中央目录记录。
struct CentralRecord {
    name: String,
    flags: u16,
    method: u16,
    crc: u32,
    compressed_size: u64,
    uncompressed_size: u64,
    lfh_offset: u64,
}

/// 带位置计数的输出流（顺序写入，无需 seek）。
struct ZipOutput {
    inner: BufWriter<File>,
    pos: u64,
}

impl ZipOutput {
    fn new(file: File) -> Self {
        Self {
            inner: BufWriter::with_capacity(1024 * 1024, file),
            pos: 0,
        }
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.inner
            .write_all(bytes)
            .map_err(|error| error.to_string())?;
        self.pos += bytes.len() as u64;
        Ok(())
    }

    fn finish(mut self) -> Result<(), String> {
        self.inner.flush().map_err(|error| error.to_string())
    }
}

/// 打包工作区为 APK（跳过旧签名残留与用户签名文件），返回打包的文件数。
pub fn package_workspace(
    work_directory: &Path,
    output: &Path,
    on_progress: PackProgress<'_>,
) -> Result<u64, String> {
    // 1. 收集文件、排序（保证输出稳定），并统计元数据。
    let mut relative_files = Vec::new();
    collect_files(work_directory, "", &mut relative_files)?;
    relative_files.sort();

    let mut planned: Vec<PlannedEntry> = Vec::with_capacity(relative_files.len());
    let mut total_bytes = 0_u64;
    for relative in relative_files {
        if should_skip_apk_entry(&relative) {
            continue;
        }
        let source = work_directory.join(&relative);
        let size = fs::metadata(&source)
            .map_err(|error| format!("读取文件失败 {}：{error}", source.display()))?
            .len();
        let (method, align) = classify_entry(&relative);
        total_bytes += size;
        planned.push(PlannedEntry {
            relative,
            source,
            method,
            align,
            size,
        });
    }
    if planned.len() > u16::MAX as usize {
        return Err(format!("条目数超出 zip32 限制：{}", planned.len()));
    }
    let total_entries = planned.len() as u64;

    // 2. 打开输出，准备顺序写入。
    let file = File::create(output)
        .map_err(|error| format!("创建 APK 失败 {}：{error}", output.display()))?;
    let mut out = ZipOutput::new(file);
    let mut central: Vec<CentralRecord> = Vec::with_capacity(planned.len());
    let completed = AtomicU64::new(0);
    on_progress(0, total_bytes);

    // 3. 分批并行压缩 → 顺序写入。
    let mut batch: Vec<PlannedEntry> = Vec::new();
    let mut batch_bytes = 0_u64;
    for entry in planned {
        if !batch.is_empty() && batch_bytes + entry.size > BATCH_BYTES {
            flush_batch(
                &mut batch,
                &mut out,
                &mut central,
                &completed,
                total_bytes,
                on_progress,
            )?;
            batch_bytes = 0;
        }
        batch_bytes += entry.size;
        batch.push(entry);
    }
    flush_batch(
        &mut batch,
        &mut out,
        &mut central,
        &completed,
        total_bytes,
        on_progress,
    )?;

    // 4. 中央目录与 EOCD。
    let cd_offset = out.pos;
    for record in &central {
        write_central_header(&mut out, record)?;
    }
    let cd_size = out.pos - cd_offset;
    if cd_size > ZIP32_LIMIT || cd_offset > ZIP32_LIMIT {
        return Err("APK 超出 zip32 限制（4GB）".to_string());
    }
    let mut eocd = Vec::with_capacity(22);
    eocd.extend_from_slice(&EOCD_SIG.to_le_bytes());
    eocd.extend_from_slice(&0u16.to_le_bytes()); // 磁盘号
    eocd.extend_from_slice(&0u16.to_le_bytes()); // 中央目录起始磁盘
    eocd.extend_from_slice(&(central.len() as u16).to_le_bytes());
    eocd.extend_from_slice(&(central.len() as u16).to_le_bytes());
    eocd.extend_from_slice(&(cd_size as u32).to_le_bytes());
    eocd.extend_from_slice(&(cd_offset as u32).to_le_bytes());
    eocd.extend_from_slice(&0u16.to_le_bytes()); // 注释长度
    out.write_bytes(&eocd)?;
    out.finish()?;
    Ok(total_entries)
}

/// 一个批次：批内 rayon 并行压缩，随后按原顺序写入输出流。
fn flush_batch(
    batch: &mut Vec<PlannedEntry>,
    out: &mut ZipOutput,
    central: &mut Vec<CentralRecord>,
    completed: &AtomicU64,
    total_bytes: u64,
    on_progress: PackProgress<'_>,
) -> Result<(), String> {
    if batch.is_empty() {
        return Ok(());
    }
    let compressed: Vec<Result<CompressedEntry, String>> = batch
        .par_iter()
        .map(compress_entry)
        .collect();
    for (entry, result) in batch.iter().zip(compressed) {
        let compressed = result?;
        let flags = if entry.relative.is_ascii() {
            0
        } else {
            FLAG_UTF8
        };
        let lfh_offset = out.pos;
        write_local_header(out, entry, &compressed, flags)?;
        out.write_bytes(&compressed.compressed)?;
        central.push(CentralRecord {
            name: entry.relative.clone(),
            flags,
            method: entry.method,
            crc: compressed.crc,
            compressed_size: compressed.compressed.len() as u64,
            uncompressed_size: compressed.uncompressed_size,
            lfh_offset,
        });
        let done = completed.fetch_add(entry.size, Ordering::Relaxed) + entry.size;
        on_progress(done.min(total_bytes), total_bytes);
    }
    batch.clear();
    Ok(())
}

/// 压缩单个条目（Stored 原样保留，Deflated 用默认级别）。
fn compress_entry(entry: &PlannedEntry) -> Result<CompressedEntry, String> {
    let mut source = File::open(&entry.source)
        .map_err(|error| format!("读取文件失败 {}：{error}", entry.source.display()))?;
    let mut raw = Vec::with_capacity(entry.size.min(64 * 1024 * 1024) as usize);
    source
        .read_to_end(&mut raw)
        .map_err(|error| format!("读取文件失败 {}：{error}", entry.source.display()))?;
    let crc = crc32fast::hash(&raw);
    let uncompressed_size = raw.len() as u64;
    let compressed = match entry.method {
        0 => raw,
        _ => {
            let mut encoder = flate2::write::DeflateEncoder::new(
                Vec::with_capacity(raw.len() / 2 + 64),
                flate2::Compression::default(),
            );
            encoder
                .write_all(&raw)
                .map_err(|error| format!("压缩失败 {}：{error}", entry.source.display()))?;
            encoder
                .finish()
                .map_err(|error| format!("压缩失败 {}：{error}", entry.source.display()))?
        }
    };
    Ok(CompressedEntry {
        crc,
        compressed,
        uncompressed_size,
    })
}

/// 写入本地文件头（含对齐填充扩展字段），随后调用方写入数据。
fn write_local_header(
    out: &mut ZipOutput,
    entry: &PlannedEntry,
    compressed: &CompressedEntry,
    flags: u16,
) -> Result<(), String> {
    let name = entry.relative.as_bytes();
    if compressed.compressed.len() as u64 > ZIP32_LIMIT
        || compressed.uncompressed_size > ZIP32_LIMIT
    {
        return Err(format!("文件过大，超出 zip32 限制：{}", entry.relative));
    }
    let extra_len = if entry.align > 1 {
        let base = out.pos + 30 + name.len() as u64 + 4;
        let pad = (entry.align - base % entry.align) % entry.align;
        (4 + pad) as u16
    } else {
        0
    };
    let mut header = Vec::with_capacity(30 + name.len() + extra_len as usize);
    header.extend_from_slice(&LFH_SIG.to_le_bytes());
    header.extend_from_slice(&VERSION_NEEDED.to_le_bytes());
    header.extend_from_slice(&flags.to_le_bytes());
    header.extend_from_slice(&entry.method.to_le_bytes());
    header.extend_from_slice(&DOS_TIME.to_le_bytes());
    header.extend_from_slice(&DOS_DATE.to_le_bytes());
    header.extend_from_slice(&compressed.crc.to_le_bytes());
    header.extend_from_slice(&(compressed.compressed.len() as u32).to_le_bytes());
    header.extend_from_slice(&(compressed.uncompressed_size as u32).to_le_bytes());
    header.extend_from_slice(&(name.len() as u16).to_le_bytes());
    header.extend_from_slice(&extra_len.to_le_bytes());
    header.extend_from_slice(name);
    if extra_len > 0 {
        header.extend_from_slice(&ALIGN_EXTRA_ID.to_le_bytes());
        header.extend_from_slice(&(extra_len - 4).to_le_bytes());
        header.resize(header.len() + (extra_len as usize - 4), 0);
    }
    out.write_bytes(&header)
}

/// 写入中央目录条目。
fn write_central_header(out: &mut ZipOutput, record: &CentralRecord) -> Result<(), String> {
    let name = record.name.as_bytes();
    let mut header = Vec::with_capacity(46 + name.len());
    header.extend_from_slice(&CDFH_SIG.to_le_bytes());
    header.extend_from_slice(&VERSION_MADE_BY.to_le_bytes());
    header.extend_from_slice(&VERSION_NEEDED.to_le_bytes());
    header.extend_from_slice(&record.flags.to_le_bytes());
    header.extend_from_slice(&record.method.to_le_bytes());
    header.extend_from_slice(&DOS_TIME.to_le_bytes());
    header.extend_from_slice(&DOS_DATE.to_le_bytes());
    header.extend_from_slice(&record.crc.to_le_bytes());
    header.extend_from_slice(&(record.compressed_size as u32).to_le_bytes());
    header.extend_from_slice(&(record.uncompressed_size as u32).to_le_bytes());
    header.extend_from_slice(&(name.len() as u16).to_le_bytes());
    header.extend_from_slice(&0u16.to_le_bytes()); // 扩展字段
    header.extend_from_slice(&0u16.to_le_bytes()); // 注释
    header.extend_from_slice(&0u16.to_le_bytes()); // 起始磁盘
    header.extend_from_slice(&0u16.to_le_bytes()); // 内部属性
    header.extend_from_slice(&0u32.to_le_bytes()); // 外部属性
    header.extend_from_slice(&(record.lfh_offset as u32).to_le_bytes());
    header.extend_from_slice(name);
    out.write_bytes(&header)
}

/// 条目分类：返回（压缩方式，对齐字节数）。
fn classify_entry(relative: &str) -> (u16, u64) {
    if relative == "resources.arsc" {
        (0, ARSC_ALIGN)
    } else if relative.starts_with("lib/") && relative.ends_with(".so") {
        (0, SO_ALIGN)
    } else {
        (8, 1)
    }
}

/// 递归收集工作区内的文件（相对路径，`/` 分隔；符号链接等特殊类型跳过）。
fn collect_files(root: &Path, relative: &str, files: &mut Vec<String>) -> Result<(), String> {
    let directory = if relative.is_empty() {
        root.to_path_buf()
    } else {
        root.join(relative)
    };
    let mut children: Vec<PathBuf> = fs::read_dir(&directory)
        .map_err(|error| format!("读取目录失败 {}：{error}", directory.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    children.sort();
    for child in children {
        let name = child
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        let child_relative = if relative.is_empty() {
            name
        } else {
            format!("{relative}/{name}")
        };
        let file_type = fs::symlink_metadata(&child)
            .map_err(|error| error.to_string())?
            .file_type();
        if file_type.is_dir() {
            collect_files(root, &child_relative, files)?;
        } else if file_type.is_file() {
            files.push(child_relative);
        }
    }
    Ok(())
}

/// 打包时跳过：旧签名残留（META-INF 下的签名文件）与用户签名密钥。
fn should_skip_apk_entry(relative: &str) -> bool {
    if relative == "signing.pem" {
        return true;
    }
    let Some(rest) = relative.strip_prefix("META-INF/") else {
        return false;
    };
    let upper = rest.to_ascii_uppercase();
    upper == "MANIFEST.MF"
        || upper.ends_with(".SF")
        || upper.ends_with(".RSA")
        || upper.ends_with(".DSA")
        || upper.ends_with(".EC")
}
