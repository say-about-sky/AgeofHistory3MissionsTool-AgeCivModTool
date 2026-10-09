//! APK 版块更新（「导出到 APK」）：把工作区中的 `assets/game/missions/` 与
//! 各剧本目录（`assets/map/<地图>/scenarios/<剧本>/`，目录名按工作区实际结构
//! 自动识别，适配各模组自定义命名）以「更新替换式覆盖」写回 APK。
//!
//! # 实现方式（追加新数据 + 重建中央目录）
//!
//! 不重写整个 APK：原文件 `[0, cd_start)` 的数据区原样保留，被替换条目的旧数据
//! 成为孤儿数据（zip 允许，读取端以中央目录为准）；新版本条目追加在原中央目录
//! 位置之后，随后重建中央目录与 EOCD 并截断文件。好处：
//! - 未改动条目（含 `lib/*.so` 等对齐敏感项）的偏移完全不变，天然保持 zipalign；
//! - 磁盘写入量 ≈ 版块数据量（几百 KB~几 MB），与 APK 总体积无关；
//! - 全部新数据先并行构建（rayon 多线程读取 + 压缩）完成，之后才顺序组装并写目标文件，
//!   尽量避免中途失败损坏原文件；写出走 1MB 缓冲成批写。
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
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use flate2::write::DeflateEncoder;
use flate2::Compression;
use rayon::prelude::*;

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

/// 从工作区目录推导「导出到 APK」的版块前缀（适配各模组自定义地图 / 剧本目录名）：
/// - `assets/game/missions/`（目录存在时）；
/// - 工作区里全部实际存在的 `assets/map/<地图>/scenarios/<剧本>/` 目录
///   （地图目录名由各模组的 `maps/Maps.json` 指定、剧本目录名由 `<地图>/Scenarios.txt`
///   指定；导出时按工作区里已有的目录识别，与导入时写入的内容一一对应）。
pub fn discover_workspace_section_prefixes(source_root: &Path) -> Vec<String> {
    let mut prefixes: Vec<String> = Vec::new();
    if source_root.join("assets/game/missions").is_dir() {
        prefixes.push("assets/game/missions/".to_string());
    }
    let mut scenarios: Vec<String> = Vec::new();
    if let Ok(maps) = fs::read_dir(source_root.join("assets/map")) {
        for map in maps.flatten() {
            let Some(map_name) = map.file_name().to_str().map(str::to_string) else {
                continue;
            };
            let scenarios_dir = map.path().join("scenarios");
            if !scenarios_dir.is_dir() {
                continue;
            }
            if let Ok(entries) = fs::read_dir(&scenarios_dir) {
                for scenario in entries.flatten() {
                    if !scenario.path().is_dir() {
                        continue;
                    }
                    let scenario_name = scenario.file_name();
                    let Some(scenario_name) = scenario_name.to_str() else {
                        continue;
                    };
                    scenarios.push(format!("assets/map/{map_name}/scenarios/{scenario_name}/"));
                }
            }
        }
    }
    scenarios.sort();
    prefixes.extend(scenarios);
    prefixes
}

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

/// 构建完成的单个新条目：本地头、压缩数据与中央目录记录（分开存放，写出时避免再拷贝）。
struct BuiltEntry {
    header: Vec<u8>,
    data: Vec<u8>,
    record: Vec<u8>,
}

impl BuiltEntry {
    /// 写入后的字节数（本地头 + 数据）。
    fn byte_len(&self) -> u64 {
        (self.header.len() + self.data.len()) as u64
    }
}

/// 以更新替换方式把 `source_root` 中的版块文件写回 `apk`（需同时具备读写权限）。
pub fn update_apk_sections(
    apk: &mut File,
    source_root: &Path,
    prefixes: &[&str],
    on_progress: ExtractProgress<'_>,
) -> Result<SectionUpdateOutcome, String> {
    let (source_files, missing_dirs) = collect_section_files(source_root, prefixes)?;
    if source_files.is_empty() && missing_dirs == prefixes.len() as u64 {
        return Err(
            "工作区中未找到国策版块（assets/game/missions、assets/map/*/scenarios/*）\
             —— 请先对目标 APK 执行「从 apk 中导入」"
                .to_string(),
        );
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

    // 1) 并行读取 + 压缩全部新条目（压缩是耗时步骤；此阶段不写目标文件）。
    let names_in_order: Vec<String> = replaced_names
        .iter()
        .chain(added_names.iter())
        .cloned()
        .collect();
    let completed = AtomicU64::new(0);
    let compressed: Vec<Result<CompressedData, String>> = names_in_order
        .par_iter()
        .map(|name| {
            let result = compress_source(&source_map[name.as_str()], name);
            let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
            on_progress(done, total_work);
            result
        })
        .collect();

    // 2) 顺序计算偏移并组装（仅构建头部/记录与引用数据，很轻量）。
    let mut built: HashMap<String, BuiltEntry> = HashMap::with_capacity(names_in_order.len());
    let mut cursor = central.cd_start; // 模拟写入偏移（用于本地头偏移字段）
    for (name, result) in names_in_order.iter().zip(compressed) {
        let compressed = result?;
        let (time, date) = if existing.contains(name.as_str()) {
            original_time_date(&central, name)
        } else {
            (DOS_TIME, DOS_DATE)
        };
        let entry = assemble_entry(compressed, name, cursor, time, date)?;
        cursor += entry.byte_len();
        built.insert(name.clone(), entry);
    }

    // 3) 写入：新数据追加在原中央目录起点处，随后重建中央目录与 EOCD，最后截断。
    apk.seek(SeekFrom::Start(central.cd_start))
        .map_err(|error| format!("定位 APK 写入位置失败：{error}"))?;
    let cd_offset = cursor;
    let mut cd_size = 0_u64;
    for entry in &central.entries {
        let bytes = match built.get(&entry.name) {
            Some(built_entry) => built_entry.record.as_slice(),
            None => entry.raw.as_slice(),
        };
        cd_size += bytes.len() as u64;
    }
    for name in &added_names {
        cd_size += built[name].record.len() as u64;
    }
    let end = cd_offset + cd_size + 22;
    if cd_offset > ZIP32_LIMIT || cd_size > ZIP32_LIMIT || end > ZIP32_LIMIT {
        return Err("更新后文件超出 zip32 限制（4GB）".to_string());
    }
    {
        // 缓冲写出：中央目录可能有数万条小记录，成批写减少系统调用。
        let mut writer = BufWriter::with_capacity(1024 * 1024, &mut *apk);
        for name in replaced_names.iter().chain(added_names.iter()) {
            let built_entry = &built[name];
            writer
                .write_all(&built_entry.header)
                .map_err(|error| format!("写入 APK 失败：{error}"))?;
            writer
                .write_all(&built_entry.data)
                .map_err(|error| format!("写入 APK 失败：{error}"))?;
        }
        for entry in &central.entries {
            let bytes = match built.get(&entry.name) {
                Some(built_entry) => built_entry.record.as_slice(),
                None => entry.raw.as_slice(),
            };
            writer
                .write_all(bytes)
                .map_err(|error| format!("写入中央目录失败：{error}"))?;
        }
        for name in &added_names {
            writer
                .write_all(&built[name].record)
                .map_err(|error| format!("写入中央目录失败：{error}"))?;
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
        writer
            .write_all(&eocd)
            .map_err(|error| format!("写入目录结尾失败：{error}"))?;
        writer
            .flush()
            .map_err(|error| format!("写入 APK 失败：{error}"))?;
    }
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
/// 目录遍历与元数据获取走并行收集（`apk_pack::collect_files_parallel`）。
fn collect_section_files(
    root: &Path,
    prefixes: &[&str],
) -> Result<(Vec<(String, PathBuf)>, u64), String> {
    let mut files = Vec::new();
    let mut missing = 0_u64;
    for prefix in prefixes {
        let dir = root.join(prefix.trim_end_matches('/'));
        if !dir.is_dir() {
            missing += 1;
            continue;
        }
        for (relative, path, _size) in crate::apk_pack::collect_files_parallel(&dir)? {
            files.push((format!("{prefix}{relative}"), path));
        }
    }
    Ok((files, missing))
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

/// 并行压缩阶段的产物：压缩方式 / CRC / 原始大小 / 数据（与偏移无关）。
struct CompressedData {
    method: u16,
    crc: u32,
    uncompressed_size: u64,
    data: Vec<u8>,
}

/// 读取并压缩单个文件（在并行 map 中调用）；自动在 Deflated / Stored 中取更小者。
fn compress_source(source: &Path, name: &str) -> Result<CompressedData, String> {
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
    Ok(CompressedData {
        method,
        crc,
        uncompressed_size,
        data,
    })
}

/// 顺序组装单个新条目：本地头（含偏移）+ 中央目录记录（数据直接引用，避免再拷贝）。
fn assemble_entry(
    compressed: CompressedData,
    name: &str,
    lfh_offset: u64,
    time: u16,
    date: u16,
) -> Result<BuiltEntry, String> {
    if lfh_offset > ZIP32_LIMIT {
        return Err("更新后偏移超出 zip32 限制（4GB）".to_string());
    }
    let CompressedData {
        method,
        crc,
        uncompressed_size,
        data,
    } = compressed;
    let name_bytes = name.as_bytes();
    let flags = if name.is_ascii() { 0 } else { FLAG_UTF8 };

    let mut header = Vec::with_capacity(30 + name_bytes.len());
    header.extend_from_slice(&LFH_SIG.to_le_bytes());
    header.extend_from_slice(&VERSION_NEEDED.to_le_bytes());
    header.extend_from_slice(&flags.to_le_bytes());
    header.extend_from_slice(&method.to_le_bytes());
    header.extend_from_slice(&time.to_le_bytes());
    header.extend_from_slice(&date.to_le_bytes());
    header.extend_from_slice(&crc.to_le_bytes());
    header.extend_from_slice(&(data.len() as u32).to_le_bytes());
    header.extend_from_slice(&(uncompressed_size as u32).to_le_bytes());
    header.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
    header.extend_from_slice(&0u16.to_le_bytes()); // 扩展字段长度
    header.extend_from_slice(name_bytes);

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

    Ok(BuiltEntry {
        header,
        data,
        record,
    })
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
        // 自定义地图 / 剧本目录名（如白日升的 Begonia/RWS）同样写回。
        write_file(
            &workspace.join("assets/map/Begonia/scenarios/RWS/missions/t.json"),
            "new-t",
        );

        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&apk)
            .unwrap();
        let prefixes = discover_workspace_section_prefixes(&workspace);
        let refs: Vec<&str> = prefixes.iter().map(String::as_str).collect();
        let outcome = update_apk_sections(&mut file, &workspace, &refs, &|_, _| {}).unwrap();
        assert_eq!(outcome.replaced, 1);
        assert_eq!(outcome.added, 2);
        assert_eq!(outcome.kept, 3);
        drop(file);

        let out = dir.join("out");
        fs::create_dir_all(&out).unwrap();
        let extracted = crate::apk_extract::extract_apk_contents(&apk, &out).unwrap();
        assert_eq!(extracted.entries, 6);
        assert_eq!(
            fs::read_to_string(out.join("assets/game/missions/a.txt")).unwrap(),
            "new-a"
        );
        assert_eq!(
            fs::read_to_string(out.join("assets/game/missions/sub/b.txt")).unwrap(),
            "added-b"
        );
        assert_eq!(
            fs::read_to_string(out.join("assets/map/Begonia/scenarios/RWS/missions/t.json"))
                .unwrap(),
            "new-t"
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
        let prefixes = discover_workspace_section_prefixes(&workspace);
        assert!(prefixes.is_empty());
        let refs: Vec<&str> = prefixes.iter().map(String::as_str).collect();
        let error = update_apk_sections(&mut file, &workspace, &refs, &|_, _| {}).unwrap_err();
        assert!(error.contains("从 apk 中导入"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn discovers_workspace_section_prefixes_for_custom_maps() {
        let dir = temp_dir("discover-ws");
        let workspace = dir.join("ws");
        fs::create_dir_all(&workspace).unwrap();
        // 空工作区：没有版块目录。
        assert!(discover_workspace_section_prefixes(&workspace).is_empty());
        write_file(&workspace.join("assets/game/missions/Missions.json"), "[]");
        write_file(
            &workspace.join("assets/map/Begonia/scenarios/RWS/missions/Missions.json"),
            "[]",
        );
        write_file(
            &workspace.join("assets/map/Earth3/scenarios/TheGreatWar/missions/Missions.json"),
            "[]",
        );
        // 非目录 / 无关目录不收录。
        fs::write(workspace.join("assets/map/Begonia/scenarios/readme.txt"), "x").unwrap();
        fs::create_dir_all(workspace.join("assets/map/Begonia/data")).unwrap();
        assert_eq!(
            discover_workspace_section_prefixes(&workspace),
            vec![
                "assets/game/missions/".to_string(),
                "assets/map/Begonia/scenarios/RWS/".to_string(),
                "assets/map/Earth3/scenarios/TheGreatWar/".to_string(),
            ]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// 实测「导出到 APK」速度（忽略）：复制真实 APK 后执行更新（不改原文件）。
    /// `cargo test --release -p age_civ_mod_tool --lib benchmark_update_real_apk -- --ignored --nocapture`
    #[test]
    #[ignore = "需要真实数据 A:\\android\\GameCivs\\暮色黄昏_世界大战0.25.1(.apk)"]
    fn benchmark_update_real_apk() {
        let apk_source = Path::new(r"A:\android\GameCivs\暮色黄昏_世界大战0.25.1.apk");
        let workspace = Path::new(r"A:\android\GameCivs\暮色黄昏_世界大战0.25.1");
        if !apk_source.is_file() || !workspace.is_dir() {
            println!("未找到真实 APK / 工作区，跳过");
            return;
        }
        let dir = temp_dir("bench-update");
        let apk = dir.join("bench.apk");
        let started = std::time::Instant::now();
        fs::copy(apk_source, &apk).unwrap();
        println!("BENCH copy: {:?}", started.elapsed());
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&apk)
            .unwrap();
        let prefixes = discover_workspace_section_prefixes(workspace);
        let refs: Vec<&str> = prefixes.iter().map(String::as_str).collect();
        let started = std::time::Instant::now();
        let outcome = update_apk_sections(&mut file, workspace, &refs, &|_, _| {}).unwrap();
        println!(
            "BENCH update: replaced={} added={} kept={} in {:?}",
            outcome.replaced,
            outcome.added,
            outcome.kept,
            started.elapsed()
        );
        drop(file);
        let _ = fs::remove_dir_all(&dir);
    }
}
