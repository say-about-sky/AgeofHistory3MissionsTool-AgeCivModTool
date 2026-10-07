//! APK 版块更新（「导出到 APK」）：把工作区中的 `assets/game/missions/` 与
//! `assets/map/Earth3/scenarios/` 目录以「更新替换式覆盖」写回 APK。
//!
//! # 实现方式（追加新数据 + 重建中央目录）
//!
//! 不重写整个 APK：原文件 `[0, cd_start)` 的数据区原样保留，被替换条目的旧数据
//! 成为孤儿数据（zip 允许，读取端以中央目录为准）；新版本条目追加在原中央目录
//! 位置之后，随后重建中央目录与 EOCD 并截断文件。好处：
//! - 未改动条目（含 `lib/*.so` 等对齐敏感项）的偏移完全不变，天然保持 zipalign；
//! - 磁盘写入量 ≈ 版块数据量（几百 KB~几 MB），与 APK 总体积无关；
//! - 全部新数据先在内存中构建完成，之后才动目标文件，尽量避免中途失败损坏原文件。
//!
//! # 语义
//! - 工作区存在的文件 → 替换 APK 中同名条目（记录保留原时间/日期）；
//! - 工作区存在、APK 中没有的文件 → 追加为新条目；
//! - APK 中有、工作区没有的条目 → 保留原样（不删除）。
//!
//! # 限制
//! 仅支持 zip32 输出：更新后总条目数 ≥ 65535 或文件 ≥ 4GiB 时返回错误（APK 场景足够）。

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use flate2::write::DeflateEncoder;
use flate2::Compression;

use crate::apk_extract::ExtractProgress;

const LFH_SIG: u32 = 0x0403_4b50;
const CDFH_SIG: u32 = 0x0201_4b50;
const EOCD_SIG: u32 = 0x0605_4b50;
const VERSION_NEEDED: u16 = 20;
const VERSION_MADE_BY: u16 = 20;
const FLAG_UTF8: u16 = 0x0800;
/// 固定时间戳（1980-01-01 00:00:00），用于新增条目。
const DOS_TIME: u16 = 0;
const DOS_DATE: u16 = 0x21;
const ZIP32_LIMIT: u64 = 0xFFFF_FFFF;

/// 「从 APK 中导入 / 导出到 APK」处理的版块前缀（保留完整路径）。
pub const APK_SECTION_PREFIXES: [&str; 2] = ["assets/game/missions/", "assets/map/Earth3/scenarios/"];

/// 更新结果统计。
#[derive(Debug)]
pub struct SectionUpdateOutcome {
    /// 被替换的文件数（按唯一名称计）。
    pub replaced: u64,
    /// 新增的文件数。
    pub added: u64,
    /// 原样保留的中央目录条目数（含目录条目与被跳过条目）。
    pub kept: u64,
}

/// 构建完成的单个新条目：数据块（本地头 + 压缩数据）与中央目录记录。
struct BuiltEntry {
    blob: Vec<u8>,
    record: Vec<u8>,
}

/// 以更新替换方式把 `source_root` 中的版块文件写回 `apk`（需同时具备读写权限）。
pub fn update_apk_sections(
    apk: &mut File,
    source_root: &Path,
    prefixes: &[&str],
    on_progress: ExtractProgress<'_>,
) -> Result<SectionUpdateOutcome, String> {
    let (source_files, missing_dirs) = collect_section_files(source_root, prefixes);
    if source_files.is_empty() && missing_dirs == prefixes.len() as u64 {
        return Err(format!(
            "工作区中未找到 {} —— 请先对目标 APK 执行「从 apk 中导入」",
            prefixes
                .iter()
                .map(|prefix| prefix.trim_end_matches('/'))
                .collect::<Vec<_>>()
                .join(" 与 ")
        ));
    }

    let central = crate::apk_extract::read_central_directory(apk)?;
    let existing: HashSet<&str> = central
        .entries
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();

    let mut source_map: HashMap<String, PathBuf> = HashMap::new();
    for (name, path) in source_files {
        source_map.insert(name, path);
    }
    let mut replaced_names: Vec<String> = Vec::new();
    let mut added_names: Vec<String> = Vec::new();
    for name in source_map.keys() {
        if existing.contains(name.as_str()) {
            replaced_names.push(name.clone());
        } else {
            added_names.push(name.clone());
        }
    }
    replaced_names.sort();
    added_names.sort();

    let total_entries = central.entries.len() as u64 + added_names.len() as u64;
    if total_entries > u16::MAX as u64 {
        return Err(format!(
            "更新后条目数超出 zip32 限制（{total_entries} ≥ 65535）"
        ));
    }

    let total_work = (replaced_names.len() + added_names.len()) as u64;
    on_progress(0, total_work);

    // 1) 先在内存中构建全部新条目（压缩是耗时步骤；此阶段不写目标文件）。
    let mut built: HashMap<String, BuiltEntry> = HashMap::new();
    let mut cursor = central.cd_start; // 模拟写入偏移（用于本地头偏移字段）
    let mut done = 0_u64;
    for name in replaced_names.iter().chain(added_names.iter()) {
        let (time, date) = if existing.contains(name.as_str()) {
            original_time_date(&central, name)
        } else {
            (DOS_TIME, DOS_DATE)
        };
        let entry = build_entry(&source_map[name], name, cursor, time, date)?;
        cursor += entry.blob.len() as u64;
        built.insert(name.clone(), entry);
        done += 1;
        on_progress(done, total_work);
    }

    // 2) 写入：新数据追加在原中央目录起点处，随后重建中央目录与 EOCD，最后截断。
    apk.seek(SeekFrom::Start(central.cd_start))
        .map_err(|error| format!("定位 APK 写入位置失败：{error}"))?;
    for name in replaced_names.iter().chain(added_names.iter()) {
        apk.write_all(&built[name].blob)
            .map_err(|error| format!("写入 APK 失败：{error}"))?;
    }

    let cd_offset = cursor;
    let mut cd_size = 0_u64;
    for entry in &central.entries {
        let bytes = match built.get(&entry.name) {
            Some(built_entry) => built_entry.record.as_slice(),
            None => entry.raw.as_slice(),
        };
        apk.write_all(bytes)
            .map_err(|error| format!("写入中央目录失败：{error}"))?;
        cd_size += bytes.len() as u64;
    }
    for name in &added_names {
        let bytes = built[name].record.as_slice();
        apk.write_all(bytes)
            .map_err(|error| format!("写入中央目录失败：{error}"))?;
        cd_size += bytes.len() as u64;
    }

    let end = cd_offset + cd_size + 22;
    if cd_offset > ZIP32_LIMIT || cd_size > ZIP32_LIMIT || end > ZIP32_LIMIT {
        return Err("更新后文件超出 zip32 限制（4GB）".to_string());
    }
    let mut eocd = Vec::with_capacity(22);
    eocd.extend_from_slice(&EOCD_SIG.to_le_bytes());
    eocd.extend_from_slice(&0u16.to_le_bytes()); // 磁盘号
    eocd.extend_from_slice(&0u16.to_le_bytes()); // 中央目录起始磁盘
    eocd.extend_from_slice(&(total_entries as u16).to_le_bytes());
    eocd.extend_from_slice(&(total_entries as u16).to_le_bytes());
    eocd.extend_from_slice(&(cd_size as u32).to_le_bytes());
    eocd.extend_from_slice(&(cd_offset as u32).to_le_bytes());
    eocd.extend_from_slice(&0u16.to_le_bytes()); // 注释长度
    apk.write_all(&eocd)
        .map_err(|error| format!("写入目录结尾失败：{error}"))?;
    apk.set_len(end)
        .map_err(|error| format!("截断 APK 失败：{error}"))?;

    let replaced_records = central
        .entries
        .iter()
        .filter(|entry| built.contains_key(&entry.name))
        .count() as u64;
    Ok(SectionUpdateOutcome {
        replaced: replaced_names.len() as u64,
        added: added_names.len() as u64,
        kept: central.entries.len() as u64 - replaced_records,
    })
}

/// 收集版块内的全部文件：返回（完整相对路径（`/` 分隔），绝对路径）与缺失的版块目录数。
fn collect_section_files(root: &Path, prefixes: &[&str]) -> (Vec<(String, PathBuf)>, u64) {
    let mut files = Vec::new();
    let mut missing = 0_u64;
    for prefix in prefixes {
        let dir = root.join(prefix.trim_end_matches('/'));
        if !dir.is_dir() {
            missing += 1;
            continue;
        }
        walk_files(&dir, &mut |relative, path| {
            files.push((format!("{prefix}{relative}"), path.to_path_buf()));
        });
    }
    (files, missing)
}

/// 迭代递归收集目录内文件（相对路径用 `/` 分隔）。
fn walk_files(dir: &Path, collect: &mut impl FnMut(&str, &Path)) {
    let mut stack: Vec<(PathBuf, String)> = vec![(dir.to_path_buf(), String::new())];
    while let Some((current, relative)) = stack.pop() {
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let child_relative = if relative.is_empty() {
                name
            } else {
                format!("{relative}/{name}")
            };
            let path = entry.path();
            if path.is_dir() {
                stack.push((path, child_relative));
            } else if path.is_file() {
                collect(&child_relative, &path);
            }
        }
    }
}

/// 从原始中央目录记录中取（时间, 日期）字段；找不到时回退固定时间戳。
fn original_time_date(central: &crate::apk_extract::RawCentralDirectory, name: &str) -> (u16, u16) {
    let Some(record) = central.entries.iter().find(|entry| entry.name == name) else {
        return (DOS_TIME, DOS_DATE);
    };
    let time = u16::from_le_bytes([record.raw[12], record.raw[13]]);
    let date = u16::from_le_bytes([record.raw[14], record.raw[15]]);
    (time, date)
}

/// 压缩并构建单个新条目（本地头 + 数据、中央目录记录）。
fn build_entry(
    source: &Path,
    name: &str,
    lfh_offset: u64,
    time: u16,
    date: u16,
) -> Result<BuiltEntry, String> {
    if lfh_offset > ZIP32_LIMIT {
        return Err("更新后偏移超出 zip32 限制（4GB）".to_string());
    }
    let mut raw = Vec::new();
    File::open(source)
        .and_then(|mut file| file.read_to_end(&mut raw))
        .map_err(|error| format!("读取文件失败 {}：{error}", source.display()))?;
    let crc = crc32fast::hash(&raw);
    let uncompressed_size = raw.len() as u64;
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(&raw)
        .map_err(|error| format!("压缩失败 {}：{error}", source.display()))?;
    let deflated = encoder
        .finish()
        .map_err(|error| format!("压缩失败 {}：{error}", source.display()))?;
    let (method, data) = if deflated.len() < raw.len() {
        (8_u16, deflated)
    } else {
        (0_u16, raw)
    };
    if data.len() as u64 > ZIP32_LIMIT || uncompressed_size > ZIP32_LIMIT {
        return Err(format!("文件过大，超出 zip32 限制：{name}"));
    }

    let name_bytes = name.as_bytes();
    let flags = if name.is_ascii() { 0 } else { FLAG_UTF8 };

    let mut blob = Vec::with_capacity(30 + name_bytes.len() + data.len());
    blob.extend_from_slice(&LFH_SIG.to_le_bytes());
    blob.extend_from_slice(&VERSION_NEEDED.to_le_bytes());
    blob.extend_from_slice(&flags.to_le_bytes());
    blob.extend_from_slice(&method.to_le_bytes());
    blob.extend_from_slice(&time.to_le_bytes());
    blob.extend_from_slice(&date.to_le_bytes());
    blob.extend_from_slice(&crc.to_le_bytes());
    blob.extend_from_slice(&(data.len() as u32).to_le_bytes());
    blob.extend_from_slice(&(uncompressed_size as u32).to_le_bytes());
    blob.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
    blob.extend_from_slice(&0u16.to_le_bytes()); // 扩展字段长度
    blob.extend_from_slice(name_bytes);
    blob.extend_from_slice(&data);

    let mut record = Vec::with_capacity(46 + name_bytes.len());
    record.extend_from_slice(&CDFH_SIG.to_le_bytes());
    record.extend_from_slice(&VERSION_MADE_BY.to_le_bytes());
    record.extend_from_slice(&VERSION_NEEDED.to_le_bytes());
    record.extend_from_slice(&flags.to_le_bytes());
    record.extend_from_slice(&method.to_le_bytes());
    record.extend_from_slice(&time.to_le_bytes());
    record.extend_from_slice(&date.to_le_bytes());
    record.extend_from_slice(&crc.to_le_bytes());
    record.extend_from_slice(&(data.len() as u32).to_le_bytes());
    record.extend_from_slice(&(uncompressed_size as u32).to_le_bytes());
    record.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
    record.extend_from_slice(&0u16.to_le_bytes()); // 扩展字段
    record.extend_from_slice(&0u16.to_le_bytes()); // 注释
    record.extend_from_slice(&0u16.to_le_bytes()); // 起始磁盘
    record.extend_from_slice(&0u16.to_le_bytes()); // 内部属性
    record.extend_from_slice(&0u32.to_le_bytes()); // 外部属性
    record.extend_from_slice(&(lfh_offset as u32).to_le_bytes());
    record.extend_from_slice(name_bytes);

    Ok(BuiltEntry { blob, record })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ageciv-apk-update-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn replaces_adds_and_keeps_section_entries() {
        let dir = temp_dir("update");
        let apk = dir.join("target.apk");
        {
            let mut zip = zip::ZipWriter::new(File::create(&apk).unwrap());
            for (name, contents) in [
                ("assets/game/missions/a.txt", "old-a"),
                ("assets/map/Earth3/scenarios/s.json", "old-s"),
                ("assets/other/keep.txt", "keep"),
                ("AndroidManifest.xml", "manifest"),
            ] {
                zip.start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(contents.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }

        let workspace = dir.join("ws");
        write_file(&workspace.join("assets/game/missions/a.txt"), "new-a");
        write_file(&workspace.join("assets/game/missions/sub/b.txt"), "added-b");

        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&apk)
            .unwrap();
        let outcome =
            update_apk_sections(&mut file, &workspace, &APK_SECTION_PREFIXES, &|_, _| {}).unwrap();
        assert_eq!(outcome.replaced, 1);
        assert_eq!(outcome.added, 1);
        assert_eq!(outcome.kept, 3);
        drop(file);

        let out = dir.join("out");
        fs::create_dir_all(&out).unwrap();
        let extracted = crate::apk_extract::extract_apk_contents(&apk, &out).unwrap();
        assert_eq!(extracted.entries, 5);
        assert_eq!(
            fs::read_to_string(out.join("assets/game/missions/a.txt")).unwrap(),
            "new-a"
        );
        assert_eq!(
            fs::read_to_string(out.join("assets/game/missions/sub/b.txt")).unwrap(),
            "added-b"
        );
        assert_eq!(
            fs::read_to_string(out.join("assets/map/Earth3/scenarios/s.json")).unwrap(),
            "old-s"
        );
        assert_eq!(
            fs::read_to_string(out.join("assets/other/keep.txt")).unwrap(),
            "keep"
        );
        assert_eq!(
            fs::read_to_string(out.join("AndroidManifest.xml")).unwrap(),
            "manifest"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_when_sections_missing() {
        let dir = temp_dir("missing");
        let apk = dir.join("target.apk");
        {
            let mut zip = zip::ZipWriter::new(File::create(&apk).unwrap());
            zip.start_file(
                "assets/other/keep.txt",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(b"keep").unwrap();
            zip.finish().unwrap();
        }
        let workspace = dir.join("empty-ws");
        fs::create_dir_all(&workspace).unwrap();
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&apk)
            .unwrap();
        let error =
            update_apk_sections(&mut file, &workspace, &APK_SECTION_PREFIXES, &|_, _| {})
                .unwrap_err();
        assert!(error.contains("从 apk 中导入"));
        let _ = fs::remove_dir_all(&dir);
    }
}
