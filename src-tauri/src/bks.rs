//! BKS（Bouncy Castle KeyStore）密钥库解析：提取证书与 PKCS#8 私钥并转为 PEM。
//!
//! 支持 BKS v1 / v2，格式依 BouncyCastle 1.79 的 `BcKeyStoreSpi` / `PKCS12ParametersGenerator`
//! 实现（已与 Java 产物实测比对）：
//! - 头部：`i32 版本、i32 盐长度、盐、i32 迭代次数`
//! - 条目区：逐条 `[u8 类型][UTF 别名][i64 时间][i32 链长][(UTF 类型 + i32 长度 + DER)…]` + 载荷，
//!   以单字节 `0x00` 结束
//! - 尾部：条目区（含终止符）的 `HMAC-SHA1`（20 字节），密钥来自 PKCS#12 KDF
//!   （v2 派生 20 字节、v1 派生 2 字节，与 BC 的怪癖保持一致）
//! - 私钥条目（SEALED=4）：`i32 盐长、盐、i32 迭代、PBEWithSHAAnd3-KeyTripleDES-CBC 密文`；
//!   解密后的载荷为 `[u8 类型][UTF 格式][UTF 算法][i32 长度][PKCS#8 DER]`

use base64::{engine::general_purpose::STANDARD, Engine as _};
use cbc::cipher::{block_padding::Pkcs7, BlockModeDecrypt, KeyIvInit};
use des::TdesEde3;
use hmac::{Hmac, KeyInit, Mac};
use sha1::{Digest, Sha1};

type HmacSha1 = Hmac<Sha1>;
type Tdes3Decryptor = cbc::Decryptor<TdesEde3>;

const TYPE_CERTIFICATE: u8 = 1;
const TYPE_KEY: u8 = 2;
const TYPE_SECRET: u8 = 3;
const TYPE_SEALED: u8 = 4;

/// 从 BKS 密钥库提取首个私钥条目的证书与 PKCS#8 私钥，生成与工具一致的 PEM 文本。
pub fn bks_to_pem(data: &[u8], password: &str) -> Result<String, String> {
    let key = parse_bks(data, password)?;
    let mut pem = String::new();
    pem.push_str("-----BEGIN PRIVATE KEY-----\n");
    push_pem_base64(&mut pem, &key.pkcs8_der);
    pem.push_str("-----END PRIVATE KEY-----\n");
    pem.push_str("-----BEGIN CERTIFICATE-----\n");
    push_pem_base64(&mut pem, &key.cert_der);
    pem.push_str("-----END CERTIFICATE-----\n");
    Ok(pem)
}

/// 判断字节是否为 BKS 密钥库（魔数：大端版本号 1 或 2）。
pub fn is_bks(data: &[u8]) -> bool {
    data.len() >= 4 && matches!(&data[..4], [0, 0, 0, 1] | [0, 0, 0, 2])
}

struct BksKey {
    cert_der: Vec<u8>,
    pkcs8_der: Vec<u8>,
}

fn parse_bks(data: &[u8], password: &str) -> Result<BksKey, String> {
    let mut reader = Reader::new(data);
    let version = reader.i32()?;
    if version != 1 && version != 2 {
        return Err(format!("不支持的 BKS 版本：{version}（仅支持 1/2）"));
    }
    let salt_len = reader.i32()?;
    if salt_len <= 0 || salt_len > (1 << 20) {
        return Err("BKS 盐长度无效".to_string());
    }
    let salt = reader.bytes(salt_len as usize)?.to_vec();
    let iterations = reader.i32()?;
    if iterations <= 0 {
        return Err("BKS 迭代次数无效".to_string());
    }

    let entries_start = reader.pos;
    let mut sealed_key: Option<(Vec<u8>, Vec<u8>)> = None;
    let mut plain_key: Option<(Vec<u8>, Vec<u8>)> = None;
    loop {
        let entry_type = reader.u8()?;
        if entry_type == 0 {
            break;
        }
        let _alias = reader.utf()?;
        let _timestamp = reader.i64()?;
        let chain_len = reader.i32()?;
        if !(0..=1024).contains(&chain_len) {
            return Err("BKS 证书链长度无效".to_string());
        }
        let mut chain_head: Option<Vec<u8>> = None;
        for index in 0..chain_len {
            let _cert_type = reader.utf()?;
            let len = reader.i32()?;
            let cert = reader.bytes(non_negative(len, "证书")?)?.to_vec();
            if index == 0 {
                chain_head = Some(cert);
            }
        }
        match entry_type {
            TYPE_CERTIFICATE => {
                let _cert_type = reader.utf()?;
                let len = reader.i32()?;
                reader.bytes(non_negative(len, "证书")?)?;
            }
            TYPE_KEY => {
                let pkcs8 = read_encoded_key(&mut reader)?;
                if plain_key.is_none() {
                    if let Some(head) = chain_head {
                        plain_key = Some((head, pkcs8));
                    }
                }
            }
            TYPE_SECRET | TYPE_SEALED => {
                let len = reader.i32()?;
                let blob = reader.bytes(non_negative(len, "条目")?)?.to_vec();
                if entry_type == TYPE_SEALED && sealed_key.is_none() {
                    if let Some(head) = chain_head {
                        sealed_key = Some((head, blob));
                    }
                }
            }
            other => return Err(format!("BKS 条目类型未知：{other}")),
        }
    }
    let entries_end = reader.pos;
    let stored_mac = reader.bytes(20)?.to_vec();

    // 完整性校验：密码错误/文件损坏时给出明确提示。
    let mac_key_len = if version == 2 { 20 } else { 2 };
    let password_bytes = pkcs12_password_bytes(password);
    let mac_key = pkcs12_derived_key(
        &password_bytes,
        &salt,
        iterations as u32,
        3,
        mac_key_len,
    );
    let mut mac = HmacSha1::new_from_slice(&mac_key).map_err(|error| error.to_string())?;
    mac.update(&data[entries_start..entries_end]);
    mac.verify_slice(&stored_mac)
        .map_err(|_| "BKS 密码错误或密钥库文件已损坏".to_string())?;

    if let Some((cert, blob)) = sealed_key {
        let pkcs8_der = decrypt_sealed(&blob, password)?;
        return Ok(BksKey { cert_der: cert, pkcs8_der });
    }
    if let Some((cert, pkcs8_der)) = plain_key {
        return Ok(BksKey { cert_der: cert, pkcs8_der });
    }
    Err("BKS 密钥库中没有可用的私钥条目".to_string())
}

fn non_negative(value: i32, what: &str) -> Result<usize, String> {
    if value < 0 {
        Err(format!("BKS {what}长度无效"))
    } else {
        Ok(value as usize)
    }
}

/// 解密 SEALED 条目：PKCS#12 KDF 派生 3DES-CBC 密钥与 IV，解密后读取 PKCS#8 DER。
fn decrypt_sealed(blob: &[u8], password: &str) -> Result<Vec<u8>, String> {
    let mut reader = Reader::new(blob);
    let salt_len = reader.i32()?;
    let salt = reader.bytes(non_negative(salt_len, "私钥盐")?)?.to_vec();
    let iterations = reader.i32()?;
    if iterations <= 0 {
        return Err("BKS 私钥条目迭代次数无效".to_string());
    }
    let ciphertext = &blob[reader.pos..];
    let password_bytes = pkcs12_password_bytes(password);
    let key = pkcs12_derived_key(&password_bytes, &salt, iterations as u32, 1, 24);
    let iv = pkcs12_derived_key(&password_bytes, &salt, iterations as u32, 2, 8);
    let mut buffer = ciphertext.to_vec();
    let decryptor =
        Tdes3Decryptor::new_from_slices(&key, &iv).map_err(|error| error.to_string())?;
    let plain = decryptor
        .decrypt_padded::<Pkcs7>(&mut buffer)
        .map_err(|_| "解密私钥失败（密码错误？）".to_string())?;
    let mut plain_reader = Reader::new(plain);
    read_encoded_key(&mut plain_reader)
}

/// 解析 BC `encodeKey` 输出：`[u8 类型][UTF 格式][UTF 算法][i32 长度][编码数据]`，返回编码数据。
fn read_encoded_key(reader: &mut Reader<'_>) -> Result<Vec<u8>, String> {
    let _key_type = reader.u8()?;
    let _format = reader.utf()?;
    let _algorithm = reader.utf()?;
    let len = reader.i32()?;
    Ok(reader.bytes(non_negative(len, "私钥")?)?.to_vec())
}

/// Java `PKCS12PasswordToBytes`：UTF-16BE 编码密码并附加 2 字节零终止符。
fn pkcs12_password_bytes(password: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity((password.len() + 1) * 2);
    for unit in password.encode_utf16() {
        out.extend_from_slice(&unit.to_be_bytes());
    }
    out.extend_from_slice(&[0, 0]);
    out
}

/// PKCS#12 KDF（RFC 7292 附录 B，与 BC `PKCS12ParametersGenerator.generateDerivedKey` 一致）。
/// `id`：1=密钥、2=IV、3=MAC；输出 `n` 字节。
fn pkcs12_derived_key(password: &[u8], salt: &[u8], iterations: u32, id: u8, n: usize) -> Vec<u8> {
    const U: usize = 20; // SHA-1 输出长度
    const V: usize = 64; // SHA-1 分块长度

    let d = vec![id; V];
    let s: Vec<u8> = if salt.is_empty() {
        Vec::new()
    } else {
        let len = V * salt.len().div_ceil(V);
        (0..len).map(|index| salt[index % salt.len()]).collect()
    };
    let p: Vec<u8> = if password.is_empty() {
        Vec::new()
    } else {
        let len = V * password.len().div_ceil(V);
        (0..len)
            .map(|index| password[index % password.len()])
            .collect()
    };
    let mut i_buffer = Vec::with_capacity(s.len() + p.len());
    i_buffer.extend_from_slice(&s);
    i_buffer.extend_from_slice(&p);

    let blocks = n.div_ceil(U);
    let mut a = [0u8; U];
    let mut out = vec![0u8; n];
    for block in 1..=blocks {
        let mut hasher = Sha1::new();
        hasher.update(&d);
        hasher.update(&i_buffer);
        a.copy_from_slice(hasher.finalize().as_slice());
        for _ in 1..iterations {
            let hashed = Sha1::digest(a);
            a.copy_from_slice(hashed.as_slice());
        }
        let mut b = [0u8; V];
        for (index, byte) in b.iter_mut().enumerate() {
            *byte = a[index % U];
        }
        for chunk in i_buffer.chunks_mut(V) {
            adjust(chunk, &b);
        }
        if block == blocks {
            let start = (block - 1) * U;
            out[start..].copy_from_slice(&a[..n - start]);
        } else {
            out[(block - 1) * U..block * U].copy_from_slice(&a);
        }
    }
    out
}

/// 大端大整数加法：`a += b + 1`（结果对 2^(b.len()*8) 取模，与 BC `adjust` 一致）。
fn adjust(a: &mut [u8], b: &[u8]) {
    let mut carry = (b[b.len() - 1] as i32) + (a[b.len() - 1] as i32) + 1;
    a[b.len() - 1] = carry as u8;
    carry >>= 8;
    for index in (0..b.len() - 1).rev() {
        carry += (b[index] as i32) + (a[index] as i32);
        a[index] = carry as u8;
        carry >>= 8;
    }
}

/// PEM Base64 正文：每行 64 字符。
fn push_pem_base64(out: &mut String, der: &[u8]) {
    let encoded = STANDARD.encode(der);
    for chunk in encoded.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(chunk).expect("base64 恒为 ASCII"));
        out.push('\n');
    }
}

/// 大端定长读取器（BKS 使用 Java DataInputStream 的字节序）。
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn u8(&mut self) -> Result<u8, String> {
        let byte = *self
            .data
            .get(self.pos)
            .ok_or_else(|| "BKS 文件意外结束".to_string())?;
        self.pos += 1;
        Ok(byte)
    }

    fn i32(&mut self) -> Result<i32, String> {
        let bytes = self.bytes(4)?;
        Ok(i32::from_be_bytes(bytes.try_into().expect("长度已校验")))
    }

    fn i64(&mut self) -> Result<i64, String> {
        let bytes = self.bytes(8)?;
        Ok(i64::from_be_bytes(bytes.try_into().expect("长度已校验")))
    }

    fn bytes(&mut self, len: usize) -> Result<&'a [u8], String> {
        let end = self
            .pos
            .checked_add(len)
            .filter(|end| *end <= self.data.len())
            .ok_or_else(|| "BKS 文件意外结束".to_string())?;
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    /// Java `DataOutputStream.writeUTF`：2 字节大端长度 + modified UTF-8（名字均为 ASCII，宽松解码）。
    fn utf(&mut self) -> Result<String, String> {
        let len = {
            let bytes = self.bytes(2)?;
            u16::from_be_bytes([bytes[0], bytes[1]]) as usize
        };
        let raw = self.bytes(len)?;
        Ok(String::from_utf8_lossy(raw).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// 本机集成测试：解析由 BouncyCastle 生成的测试密钥库（密码 test1234）。
    /// 先用 Java 生成：`java -cp <bcprov.jar> tmp/GenBks.java src-tauri/assets/mod-signing.pem tmp/test-bks.bks test1234`
    #[test]
    fn parses_java_generated_bks() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("tmp")
            .join("test-bks.bks");
        if !path.is_file() {
            println!("未找到 {}，跳过（先用 GenBks.java 生成）", path.display());
            return;
        }
        let data = std::fs::read(&path).unwrap();
        assert!(is_bks(&data), "魔数应为 BKS 版本 1/2");

        // 错误密码应报错。
        let wrong = bks_to_pem(&data, "wrong-password");
        assert!(wrong.is_err(), "错误密码不应解析成功");

        let pem = bks_to_pem(&data, "test1234").unwrap();
        assert!(pem.contains("-----BEGIN PRIVATE KEY-----"));
        assert!(pem.contains("-----BEGIN CERTIFICATE-----"));
        // 解析出的私钥与证书应能被签名模块直接加载。
        let key = crate::apk_signing::ApkSigningKey::from_pem(&pem).unwrap();
        // 与内置密钥应为同一对密钥（GenBks 使用内置 PEM 生成）。
        let builtin = crate::apk_signing::ApkSigningKey::builtin().unwrap();
        assert_eq!(key.pubkey_der_bytes(), builtin.pubkey_der_bytes());
        assert_eq!(key.cert_der_bytes(), builtin.cert_der_bytes());
    }
}
