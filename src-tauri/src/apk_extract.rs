//! 宽容的 APK/zip 解压实现。
//!
//! 部分真实游戏 APK 的 extra field 不符合 zip 规范（例如被截断、Unicode 扩展字段
//! CRC 不符），常规 zip 库会直接拒绝打开整个文件；本实现只使用中央目录中的固定
//! 字段与本地文件头来定位、解压数据，完全忽略 extra 数据，从而尽可能把内容解压出来。
//!
//! 支持 stored / deflate 两种压缩方式（APK 中全部使用这两种），以及 ZIP64 扩展。
//!
//! # 性能设计（安卓 5 万文件级游戏 APK 实测：359s → 24~30s）
//!
//! 安卓共享存储（FUSE/MediaProvider）的瓶颈是**每次文件系统调用的毫秒级延迟**。
//! 实测 16 线程并行效率≈99%（墙钟时间 ≈ 各操作累计耗时 ÷ 线程数），因此优化
//! 两条主线：减少系统调用次数、用并发掩盖延迟。以下措施均已实测验证：
//!
//! - **扫描**：中央目录（≤64MB）一次性读入内存解析，避免每条目多次 4~60 字节小读；
//!   全程 `read_at` 定位读取，多线程共享同一个文件句柄（安卓 `content://` 句柄无法按路径重开）。
//! - **解压**：每条目一次 64KB 预读同时覆盖「本地头 + 小条目数据」，小文件全生命周期
//!   仅 1 次源读；大条目用 `Cursor(缓冲).chain(Take<PosReader>)` 链式补读。
//! - **写盘**：线程本地复用缓冲（256KB）替代每条目新建 BufReader/BufWriter（避免 mmap 抖动）；
//!   条目按输出路径排序 + `openat` 缓存父目录句柄（绕开 FUSE 逐级路径解析）；
//!   deflate 由 flate2 的 zlib-rs 后端解压（纯 Rust zlib-ng 移植，远快于默认 miniz_oxide）。
//! - **媒体库**：安卓端在解压根目录放置 `.nomedia`（见 `commands::apk`），让媒体库
//!   跳过整树逐文件索引登记（新文件创建开销的主要来源）。
//! - **并发**：显式自建局部 rayon 线程池（不依赖全局池默认值），线程数上限 16——
//!   实测 32 线程会让安卓存储层退化为串行队列（建文件 230ms/次、整体反而慢 10 倍）。

use std::cell::RefCell;
#[cfg(unix)]
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{self, Read, Write};
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rayon::prelude::*;

const EOCD_SIG: u32 = 0x0605_4b50;
const CDFH_SIG: u32 = 0x0201_4b50;
const LFH_SIG: u32 = 0x0403_4b50;
const ZIP64_EOCD_SIG: u32 = 0x0606_4b50;
const ZIP64_LOCATOR_SIG: u32 = 0x0706_4b50;
const EOCD_MIN_SIZE: u64 = 22;
const CDFH_FIXED_SIZE: u64 = 46;
/// 中央目录大于该值时不再一次性读入内存（理论极端场景的回退阈值）。
const CD_IN_MEMORY_LIMIT: u64 = 64 * 1024 * 1024;

/// 解压结果统计。
pub struct ExtractOutcome {
    /// 成功解压出的文件数（不含目录）。
    pub entries: u64,
    /// 无法处理的条目数（压缩方式不支持、文件名非法等）。
    pub skipped: u64,
}

/// 解压进度回调：`completed` / `total` 为已处理 / 总文件数。
pub type ExtractProgress<'a> = &'a (dyn Fn(u64, u64) + Sync);

/// 扫描阶段收集的条目元数据（仅中央目录固定字段）。
pub(crate) struct ScanEntry {
    pub(crate) name: String,
    pub(crate) method: u16,
    pub(crate) comp_size: u64,
    pub(crate) lfh_offset: u64,
}

/// 扫描 APK 中央目录，返回全部条目元数据（「从 APK 中导入」后的补全数据兜底等
/// 只需读取少量条目的场景使用；与大解压共用同一套宽容扫描）。
pub(crate) fn list_apk_entries(file: &File) -> Result<Vec<ScanEntry>, String> {
    let metadata = file
        .metadata()
        .map_err(|error| format!("读取 APK 信息失败：{error}"))?;
    if !metadata.is_file() {
        return Err(
            "该 APK 无法随机读取（可能来自云存储等虚拟目录），请先把 APK 保存到设备本地后再解压"
                .to_string(),
        );
    }
    let file_len = metadata.len();

    let eocd_pos = find_eocd(file, file_len)?;
    let (cd_offset, entry_count) = read_eocd_metadata(file, eocd_pos)?;
    let cd_start = locate_cd_start(file, cd_offset, eocd_pos)?;

    let mut scan_entries: Vec<ScanEntry> = Vec::new();
    // 中央目录一次性读入内存解析：游戏 APK 常有一万以上条目，逐条目做
    // 「4 字节签名 + 46 字节头 + 文件名」多次小读在 Android FUSE 上非常昂贵。
    let cd_len = eocd_pos.saturating_sub(cd_start);
    if cd_len <= CD_IN_MEMORY_LIMIT {
        let mut data = vec![0u8; cd_len as usize];
        if cd_len > 0 {
            read_exact_at(file, cd_start, &mut data)?;
        }
        parse_central_directory(&data, &mut scan_entries);
    } else {
        // 极端巨大的中央目录：回退为逐条目定位读取，避免一次性占用过多内存。
        let mut pos = cd_start;
        while pos + CDFH_FIXED_SIZE <= eocd_pos {
            if read_u32_at(file, pos)? != CDFH_SIG {
                break; // 中央目录结束（条目数可能不准，以签名缺失为准）。
            }
            let (method, comp_size, name, lfh_offset, next_pos) = read_cdfh(file, pos)?;
            pos = next_pos;
            scan_entries.push(ScanEntry {
                name,
                method,
                comp_size,
                lfh_offset,
            });
        }
    }
    let _ = entry_count; // 仅作参考，实际以中央目录签名为准。
    Ok(scan_entries)
}

/// 记录前若干条失败原因（最多 5 条），用于最终错误汇总。
fn push_first_error(errors: &mut Vec<String>, name: &str, message: &str) {
    if errors.len() < 5 {
        errors.push(format!("{name}：{message}"));
    }
}

/// 把 APK/zip 解压到目标目录（覆盖同名文件）。
/// 简版接口（无进度回调），测试与内部简单场景使用。
#[allow(dead_code)]
pub fn extract_apk_contents(apk: &Path, destination: &Path) -> Result<ExtractOutcome, String> {
    extract_apk_contents_with_progress(apk, destination, &|_, _| {})
}

/// 带进度回调的宽容解压：先单线程扫描中央目录，再用 rayon 多线程解压。
pub fn extract_apk_contents_with_progress(
    apk: &Path,
    destination: &Path,
    on_progress: ExtractProgress<'_>,
) -> Result<ExtractOutcome, String> {
    let file =
        File::open(apk).map_err(|error| format!("打开 APK 失败 {}：{error}", apk.display()))?;
    extract_from_file(&file, destination, on_progress)
}

/// 解压并行度：上限 16——实测 32 线程会让安卓存储层退化为串行队列（整体反而慢约 10 倍）。
/// 不依赖 rayon 全局池默认值（个别设备默认并发数过低）；小任务按量收缩线程数。
fn extraction_thread_count() -> usize {
    let cores = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(4)
        .max(2);
    (cores * 2).clamp(6, 16)
}

/// 基于已打开文件句柄的宽容解压（全部条目）：见 [`extract_selected_from_file`]。
pub fn extract_from_file(
    file: &File,
    destination: &Path,
    on_progress: ExtractProgress<'_>,
) -> Result<ExtractOutcome, String> {
    extract_selected_from_file(file, destination, &[], on_progress)
}

/// 从路径打开并按前缀过滤解压（「从 APK 中导入」版块的测试辅助；正式命令统一走
/// [`extract_selected_from_file`] 的已打开文件句柄路径，桌面 / 安卓共用）。
#[cfg(test)]
pub fn extract_apk_sections_with_progress(
    apk: &Path,
    destination: &Path,
    prefixes: &[&str],
    on_progress: ExtractProgress<'_>,
) -> Result<ExtractOutcome, String> {
    let file =
        File::open(apk).map_err(|error| format!("打开 APK 失败 {}：{error}", apk.display()))?;
    extract_selected_from_file(&file, destination, prefixes, on_progress)
}

/// 动态推导「从 APK 中导入」的版块前缀（适配各模组自定义的地图 / 剧本目录名）。
///
/// 国策相关目录的命名是各模组自定义、由游戏运行时加载的：
/// `assets/map/<地图目录名：由 map/Maps.json 的 Folder: 指定>/scenarios/<剧本目录名：
/// 由该地图的 Scenarios.txt 清单指定>/`——不能写死具体名字（例如「白日升」用
/// `Begonia/RWS`、「暮色黄昏」用 `Earth3/TheGreatWar`）。这里直接按中央目录里
/// 实际存在的 `assets/map/*/scenarios/*/` 组合识别（清单缺失 / 自定义的模组同样覆盖），
/// 并固定包含 `assets/game/missions/`（全局国策）。
pub fn discover_section_prefixes(file: &File) -> Result<Vec<String>, String> {
    let entries = list_apk_entries(file)?;
    let mut prefixes: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    prefixes.insert("assets/game/missions/".to_string());
    for entry in &entries {
        if let Some(prefix) = section_prefix_of_entry(&entry.name) {
            prefixes.insert(prefix);
        }
    }
    Ok(prefixes.into_iter().collect())
}

/// 条目名属于剧本版块（`assets/map/<地图>/scenarios/<剧本>/…`）时返回其版块前缀
/// （末尾带 `/`）；非该结构（如 `assets/map/Maps.json`）返回 `None`。
fn section_prefix_of_entry(name: &str) -> Option<String> {
    let rest = name.strip_prefix("assets/map/")?;
    let mut segments = rest.split('/');
    let map = segments.next()?;
    if segments.next()? != "scenarios" {
        return None;
    }
    let scenario = segments.next()?;
    if map.is_empty() || scenario.is_empty() {
        return None;
    }
    Some(format!("assets/map/{map}/scenarios/{scenario}/"))
}

/// 基于已打开文件句柄的宽容解压：全程使用定位读取（`read_at`），
/// 多线程共享同一个句柄而互不干扰偏移。
///
/// `prefixes` 非空时只解压条目名以任一前缀开头的条目（其余静默跳过、不计入失败），
/// 用于「从 APK 中导入」指定版块（保留完整路径）。
///
/// Android 端 `content://` 句柄无法按路径重新打开，只能共享句柄读取；
/// 桌面端同样受益于省去的每线程反复打开开销。
pub fn extract_selected_from_file(
    file: &File,
    destination: &Path,
    prefixes: &[&str],
    on_progress: ExtractProgress<'_>,
) -> Result<ExtractOutcome, String> {
    // ---- 1. 扫描中央目录（只读元数据）----
    let scan_entries = list_apk_entries(file)?;

    // ---- 2. 规划：规范文件名、预建目录、筛出待解压文件 ----
    let mut files: Vec<(ScanEntry, PathBuf)> = Vec::with_capacity(scan_entries.len());
    let mut skipped = 0_u64;
    let mut first_errors: Vec<String> = Vec::new();
    let mut parents: std::collections::BTreeSet<PathBuf> = std::collections::BTreeSet::new();
    for entry in scan_entries {
        if !prefixes.is_empty() && !prefixes.iter().any(|prefix| entry.name.starts_with(prefix)) {
            continue; // 版块过滤：不在目标前缀内的条目静默跳过。
        }
        let Some(relative) = sanitize_entry_name(&entry.name) else {
            skipped += 1;
            push_first_error(&mut first_errors, &entry.name, "文件名无效或越出目标目录");
            continue;
        };
        let output_path = destination.join(&relative);
        if entry.name.ends_with('/') {
            if let Err(error) = fs::create_dir_all(&output_path) {
                skipped += 1;
                push_first_error(
                    &mut first_errors,
                    &entry.name,
                    &format!("创建目录失败：{error}"),
                );
            }
            continue;
        }
        if let Some(parent) = output_path.parent() {
            parents.insert(parent.to_path_buf());
        }
        files.push((entry, output_path));
    }
    // BTreeSet 已按目录去重（同一目录只出现一次）；目录创建也并行化——
    // FUSE 上每次 mkdir 都不便宜，条目数多时建目录是纯串行浪费。
    let parents: Vec<PathBuf> = parents.into_iter().collect();
    parents.par_iter().try_for_each(|parent| {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建目录失败 {}：{error}", parent.display()))
    })?;

    // ---- 3. 并行解压（定位读取：所有线程共享同一句柄，互不干扰偏移）----
    // 按输出路径排序：同目录文件连续处理 → openat 的父目录句柄缓存命中率最高，
    // 且 rayon 的连续区间划分让不同线程处理不同的目录子集（减少共享存储上的干扰）。
    // 源读取不再严格顺序，但预读合并后源读仅占约 1%，可忽略。
    files.sort_by(|a, b| a.1.cmp(&b.1));
    let total = files.len() as u64;
    on_progress(0, total);
    let step = (total / 2000).max(1); // 细粒度进度：约 2000 档，进度条更平滑
    let completed = AtomicU64::new(0);
    // 显式建局部线程池：不依赖 rayon 全局默认值（个别安卓设备并发数过低导致单线程）。
    // 小档案无需开满线程，避免建线程开销（每 16 个任务才用 1 个线程，至少 2 个）。
    let threads = extraction_thread_count().min(((total as usize) / 16).max(2));
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .thread_name(|index| format!("apk-extract-{index}"))
        .build()
        .map_err(|error| format!("创建解压线程池失败：{error}"))?;
    let results: Vec<Result<(), String>> = pool.install(|| {
        files
            .par_iter()
            .map(|(entry, output_path)| {
                let result = extract_entry(file, entry, output_path);
                let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
                if done % step == 0 || done == total {
                    on_progress(done, total);
                }
                result
            })
            .collect()
    });

    let mut entries = 0_u64;
    for (result, (entry, _)) in results.into_iter().zip(files.iter()) {
        match result {
            Ok(()) => entries += 1,
            Err(error) => {
                skipped += 1;
                push_first_error(&mut first_errors, &entry.name, &error);
            }
        }
    }

    if entries == 0 && skipped > 0 {
        return Err(format!("解压失败：{}", first_errors.join("；")));
    }
    Ok(ExtractOutcome { entries, skipped })
}

/// 从尾部向前扫描 EOCD 签名（注释最长 65535 字节，不校验注释长度以保持宽容）。
fn find_eocd(file: &File, file_len: u64) -> Result<u64, String> {
    if file_len < EOCD_MIN_SIZE {
        return Err("文件过小，不是有效的 APK/zip".to_string());
    }
    let lower_bound = file_len.saturating_sub(EOCD_MIN_SIZE + u16::MAX as u64);
    let max_pos = file_len - EOCD_MIN_SIZE;
    let window = 64 * 1024;
    let mut block_end = max_pos + 1; // 当前块的候选起点上界（不含）
    let mut buffer = vec![0u8; window + 4]; // 额外 3 字节：签名可能跨过块尾
    while block_end > lower_bound {
        let block_start = block_end.saturating_sub(window as u64).max(lower_bound);
        let byte_end = (block_end + 3).min(file_len);
        let take = (byte_end - block_start) as usize;
        read_exact_at(file, block_start, &mut buffer[..take])?;
        let last_start = (block_end - block_start) as usize;
        for index in (0..last_start).rev() {
            if index + 4 <= take
                && u32::from_le_bytes(buffer[index..index + 4].try_into().unwrap()) == EOCD_SIG
            {
                return Ok(block_start + index as u64);
            }
        }
        if block_start == lower_bound {
            break;
        }
        block_end = block_start + 3; // 保留 3 字节重叠避免漏检跨块签名
    }
    Err("未找到 zip 中央目录结尾，文件可能已损坏".to_string())
}

/// 读取 EOCD 中的中央目录偏移与条目数；必要时转读 ZIP64 记录。
fn read_eocd_metadata(file: &File, eocd_pos: u64) -> Result<(u64, u64), String> {
    let entry_count_16 = read_u16_at(file, eocd_pos + 10)? as u64;
    let cd_offset_32 = read_u32_at(file, eocd_pos + 16)? as u64;

    // 常规 zip32 字段未溢出时直接返回。
    if cd_offset_32 != 0xFFFF_FFFF && entry_count_16 != 0xFFFF {
        return Ok((cd_offset_32, entry_count_16));
    }

    // ZIP64：EOCD 前 20 字节处是定位器。
    if eocd_pos < 20 {
        return Ok((cd_offset_32, entry_count_16));
    }
    let locator_pos = eocd_pos - 20;
    if read_u32_at(file, locator_pos)? != ZIP64_LOCATOR_SIG {
        return Ok((cd_offset_32, entry_count_16));
    }
    let zip64_eocd_pos = read_u64_at(file, locator_pos + 8)?;
    if read_u32_at(file, zip64_eocd_pos)? != ZIP64_EOCD_SIG {
        return Ok((cd_offset_32, entry_count_16));
    }
    let entry_count_64 = read_u64_at(file, zip64_eocd_pos + 32)?;
    let cd_offset_64 = read_u64_at(file, zip64_eocd_pos + 48)?;
    Ok((cd_offset_64, entry_count_64))
}

/// 确认中央目录起点：优先使用记录值，签名不符时向前扫描（部分工具改包后偏移会有偏差）。
fn locate_cd_start(file: &File, cd_offset: u64, eocd_pos: u64) -> Result<u64, String> {
    if cd_offset + 4 <= eocd_pos && read_u32_at(file, cd_offset)? == CDFH_SIG {
        return Ok(cd_offset);
    }
    let mut pos = cd_offset;
    let mut buffer = vec![0u8; 64 * 1024];
    while pos + 4 <= eocd_pos {
        let take = ((eocd_pos - pos) as usize).min(buffer.len());
        read_exact_at(file, pos, &mut buffer[..take])?;
        let mut index = 0usize;
        while index + 4 <= take {
            if u32::from_le_bytes(buffer[index..index + 4].try_into().unwrap()) == CDFH_SIG {
                return Ok(pos + index as u64);
            }
            index += 1;
        }
        pos += take.saturating_sub(3).max(1) as u64; // 保留 3 字节重叠避免漏检跨块签名
    }
    Err("未找到 zip 中央目录".to_string())
}

/// 原始中央目录条目（导出到 APK 时用于原样保留未改动条目的目录记录）。
pub(crate) struct RawCdEntry {
    pub name: String,
    /// 完整原始记录字节（固定头 + 名称 + 扩展字段 + 注释）。
    pub raw: Vec<u8>,
}

/// 原始中央目录（仅供 `apk_update` 使用）。
pub(crate) struct RawCentralDirectory {
    pub entries: Vec<RawCdEntry>,
    /// 中央目录在文件中的起始偏移（新数据/新目录的写入起点）。
    pub cd_start: u64,
}

/// 读取完整原始中央目录：条目按目录顺序、保留每条记录的原始字节，
/// 供「导出到 APK」以「追加新数据 + 重建目录」的方式做更新替换。
pub(crate) fn read_central_directory(file: &File) -> Result<RawCentralDirectory, String> {
    let file_len = file
        .metadata()
        .map_err(|error| format!("读取 APK 信息失败：{error}"))?
        .len();
    let eocd_pos = find_eocd(file, file_len)?;
    let (cd_offset, _entry_count) = read_eocd_metadata(file, eocd_pos)?;
    let cd_start = locate_cd_start(file, cd_offset, eocd_pos)?;

    let mut entries: Vec<RawCdEntry> = Vec::new();
    let mut pos = cd_start;
    while pos + CDFH_FIXED_SIZE <= eocd_pos {
        let mut fixed = [0u8; CDFH_FIXED_SIZE as usize];
        if read_exact_at(file, pos, &mut fixed).is_err()
            || u32::from_le_bytes(fixed[0..4].try_into().unwrap()) != CDFH_SIG
        {
            break; // 中央目录结束（条目数可能不准，以签名缺失为准）。
        }
        let name_len = u16::from_le_bytes([fixed[28], fixed[29]]) as u64;
        let extra_len = u16::from_le_bytes([fixed[30], fixed[31]]) as u64;
        let comment_len = u16::from_le_bytes([fixed[32], fixed[33]]) as u64;
        let total = CDFH_FIXED_SIZE + name_len + extra_len + comment_len;
        let mut raw = vec![0u8; total as usize];
        read_exact_at(file, pos, &mut raw)
            .map_err(|error| format!("读取中央目录失败：{error}"))?;
        let name = String::from_utf8_lossy(&raw[46..46 + name_len as usize]).into_owned();
        entries.push(RawCdEntry { name, raw });
        pos += total;
    }
    Ok(RawCentralDirectory { entries, cd_start })
}

/// 读取一个中央目录条目：返回（压缩方式、压缩尺寸、文件名、本地头偏移、下一项位置）。
#[allow(clippy::type_complexity)]
fn read_cdfh(file: &File, pos: u64) -> Result<(u16, u64, String, u64, u64), String> {
    let mut fixed = [0u8; CDFH_FIXED_SIZE as usize];
    read_exact_at(file, pos, &mut fixed).map_err(|error| format!("读取中央目录条目失败：{error}"))?;
    let method = u16::from_le_bytes([fixed[10], fixed[11]]);
    let comp_size_32 = u32::from_le_bytes(fixed[20..24].try_into().unwrap()) as u64;
    let uncomp_size_32 = u32::from_le_bytes(fixed[24..28].try_into().unwrap()) as u64;
    let name_len = u16::from_le_bytes([fixed[28], fixed[29]]) as u64;
    let extra_len = u16::from_le_bytes([fixed[30], fixed[31]]) as u64;
    let comment_len = u16::from_le_bytes([fixed[32], fixed[33]]) as u64;
    let lfh_offset_32 = u32::from_le_bytes(fixed[42..46].try_into().unwrap()) as u64;

    let mut name_bytes = vec![0u8; name_len as usize];
    read_exact_at(file, pos + CDFH_FIXED_SIZE, &mut name_bytes)
        .map_err(|error| format!("读取文件名失败：{error}"))?;
    let name = String::from_utf8_lossy(&name_bytes).into_owned();

    // 仅在 32 位字段溢出时解析 ZIP64 扩展字段。
    let mut comp_size = comp_size_32;
    let mut lfh_offset = lfh_offset_32;
    if comp_size_32 == 0xFFFF_FFFF || uncomp_size_32 == 0xFFFF_FFFF || lfh_offset_32 == 0xFFFF_FFFF
    {
        let mut extra = vec![0u8; extra_len as usize];
        read_exact_at(file, pos + CDFH_FIXED_SIZE + name_len, &mut extra)
            .map_err(|error| format!("读取 ZIP64 扩展字段失败：{error}"))?;
        let (zip64_comp, zip64_lfh) =
            parse_zip64_sizes(&extra, comp_size_32, uncomp_size_32, lfh_offset_32)?;
        comp_size = zip64_comp;
        lfh_offset = zip64_lfh;
    }

    let next_pos = pos + CDFH_FIXED_SIZE + name_len + extra_len + comment_len;
    Ok((method, comp_size, name, lfh_offset, next_pos))
}

/// 从内存中的中央目录数据解析全部条目（宽松：签名不符或尾部截断即停止）。
fn parse_central_directory(data: &[u8], entries: &mut Vec<ScanEntry>) {
    let mut offset = 0usize;
    while offset + CDFH_FIXED_SIZE as usize <= data.len() {
        let fixed = &data[offset..offset + CDFH_FIXED_SIZE as usize];
        if u32::from_le_bytes(fixed[0..4].try_into().unwrap()) != CDFH_SIG {
            break; // 中央目录结束（条目数可能不准，以签名缺失为准）。
        }
        let method = u16::from_le_bytes([fixed[10], fixed[11]]);
        let comp_size_32 = u32::from_le_bytes(fixed[20..24].try_into().unwrap()) as u64;
        let uncomp_size_32 = u32::from_le_bytes(fixed[24..28].try_into().unwrap()) as u64;
        let name_len = u16::from_le_bytes([fixed[28], fixed[29]]) as usize;
        let extra_len = u16::from_le_bytes([fixed[30], fixed[31]]) as usize;
        let comment_len = u16::from_le_bytes([fixed[32], fixed[33]]) as usize;
        let lfh_offset_32 = u32::from_le_bytes(fixed[42..46].try_into().unwrap()) as u64;
        let total = CDFH_FIXED_SIZE as usize + name_len + extra_len + comment_len;
        if offset + total > data.len() {
            break; // 尾部条目被截断：按宽容策略忽略剩余部分。
        }
        let name_start = offset + CDFH_FIXED_SIZE as usize;
        let name = String::from_utf8_lossy(&data[name_start..name_start + name_len]).into_owned();

        // 仅在 32 位字段溢出时解析 ZIP64 扩展字段。
        let mut comp_size = comp_size_32;
        let mut lfh_offset = lfh_offset_32;
        if comp_size_32 == 0xFFFF_FFFF
            || uncomp_size_32 == 0xFFFF_FFFF
            || lfh_offset_32 == 0xFFFF_FFFF
        {
            let extra = &data[name_start + name_len..name_start + name_len + extra_len];
            if let Ok((zip64_comp, zip64_lfh)) =
                parse_zip64_sizes(extra, comp_size_32, uncomp_size_32, lfh_offset_32)
            {
                comp_size = zip64_comp;
                lfh_offset = zip64_lfh;
            }
        }

        entries.push(ScanEntry {
            name,
            method,
            comp_size,
            lfh_offset,
        });
        offset += total;
    }
}

/// 从 ZIP64 扩展字段中取出（溢出的）压缩尺寸与本地头偏移；
/// 未溢出的字段保持原 32 位值。
fn parse_zip64_sizes(
    extra: &[u8],
    comp_size_32: u64,
    uncomp_size_32: u64,
    lfh_offset_32: u64,
) -> Result<(u64, u64), String> {
    let mut i = 0usize;
    while i + 4 <= extra.len() {
        let id = u16::from_le_bytes([extra[i], extra[i + 1]]);
        let size = u16::from_le_bytes([extra[i + 2], extra[i + 3]]) as usize;
        i += 4;
        if i + size > extra.len() {
            break; // 扩展字段被截断：按宽容策略忽略。
        }
        if id == 0x0001 {
            let mut p = &extra[i..i + size];
            let mut comp = comp_size_32;
            let mut lfh = lfh_offset_32;
            if uncomp_size_32 == 0xFFFF_FFFF {
                p = take8(p, "未压缩尺寸")?;
            }
            if comp_size_32 == 0xFFFF_FFFF {
                if p.len() < 8 {
                    return Err("ZIP64 压缩尺寸缺失".to_string());
                }
                comp = u64::from_le_bytes(p[..8].try_into().unwrap());
                p = &p[8..];
            }
            if lfh_offset_32 == 0xFFFF_FFFF {
                if p.len() < 8 {
                    return Err("ZIP64 本地头偏移缺失".to_string());
                }
                lfh = u64::from_le_bytes(p[..8].try_into().unwrap());
            }
            return Ok((comp, lfh));
        }
        i += size;
    }
    Ok((comp_size_32, lfh_offset_32))
}

fn take8<'a>(slice: &'a [u8], name: &str) -> Result<&'a [u8], String> {
    if slice.len() < 8 {
        Err(format!("ZIP64 {name} 缺失"))
    } else {
        Ok(&slice[8..])
    }
}

/// 条目的预读缓冲大小：一次定位读取最多覆盖「本地头 + 小条目数据」。
const ENTRY_PREFETCH: usize = 64 * 1024;

thread_local! {
    static PREFETCH_BUFFER: RefCell<Vec<u8>> = RefCell::new(vec![0u8; ENTRY_PREFETCH]);
}

/// 条目数据读取核心：一次预读本地头（小条目数据一并读入），把解压后的数据流
/// 交给 `consumer`（支持 stored / deflate）。供解压写盘与按需内存读取共用。
fn with_entry_data(
    file: &File,
    entry: &ScanEntry,
    consumer: impl FnOnce(&mut dyn Read) -> Result<(), String>,
) -> Result<(), String> {
    PREFETCH_BUFFER.with(|cell| {
        let mut buffer = cell.borrow_mut();
        // 一次读取同时覆盖：本地头（30B）+ 名称/扩展字段；小条目的数据也在此次读取内。
        // FUSE/云盘句柄上每次系统调用都很贵，合并后小文件全生命周期只剩 1 次源读。
        let read = read_at(file, &mut buffer[..], entry.lfh_offset)
            .map_err(|error| format!("读取本地文件头失败：{error}"))?;
        if read < 30 {
            return Err("读取本地文件头失败：文件被截断".to_string());
        }
        if u32::from_le_bytes(buffer[0..4].try_into().unwrap()) != LFH_SIG {
            return Err("本地文件头签名无效".to_string());
        }
        let lfh_name_len = u16::from_le_bytes([buffer[26], buffer[27]]) as usize;
        let lfh_extra_len = u16::from_le_bytes([buffer[28], buffer[29]]) as usize;
        let data_off = 30 + lfh_name_len + lfh_extra_len;

        // 数据跨越预读边界（大条目）时：缓冲内部分与剩余句柄数据链式读取；
        // 名称/扩展字段异常大而超出预读缓冲时，缓冲部分视为空（全部走句柄）。
        let fits = data_off as u64 + entry.comp_size <= read as u64;
        let buffered = ((read as u64).saturating_sub(data_off as u64))
            .min(entry.comp_size) as usize;
        let data: &[u8] = if data_off <= read && buffered > 0 {
            &buffer[data_off..data_off + buffered]
        } else {
            &[]
        };
        let mut source: Box<dyn Read + '_> = if fits {
            Box::new(std::io::Cursor::new(data))
        } else {
            let data_start = entry.lfh_offset + data_off as u64;
            let rest = PosReader::new(file, data_start + buffered as u64)
                .take(entry.comp_size - buffered as u64);
            Box::new(std::io::Cursor::new(data).chain(rest))
        };
        match entry.method {
            0 => consumer(&mut source),
            8 => {
                let mut decoder = flate2::read::DeflateDecoder::new(source);
                consumer(&mut decoder)
            }
            other => Err(format!("不支持的压缩方式 {other}")),
        }
    })
}

/// 解压单个文件条目（一次预读覆盖本地头与小条目数据；大条目再链式补读）。
fn extract_entry(file: &File, entry: &ScanEntry, output_path: &Path) -> Result<(), String> {
    // 0 字节的 stored 条目：直接建空文件，省去一次本地头读取。
    if entry.comp_size == 0 && entry.method == 0 {
        create_file(output_path)?;
        return Ok(());
    }
    let context = if entry.method == 8 { "解压数据失败" } else { "复制数据失败" };
    with_entry_data(file, entry, |reader| {
        let mut output = create_file(output_path)?;
        copy_stream(reader, &mut output).map_err(|error| format!("{context}：{error}"))?;
        Ok(())
    })
}

/// 读取单个条目到内存（「从 APK 中导入」后的工作区补全等按需取数场景使用）。
/// `limit` 为允许的最大解压尺寸（防止意外读取超大条目）。
pub(crate) fn read_apk_entry_bytes(
    file: &File,
    entry: &ScanEntry,
    limit: u64,
) -> Result<Vec<u8>, String> {
    if entry.comp_size > limit {
        return Err(format!(
            "条目过大（{} 字节），已跳过：{}",
            entry.comp_size, entry.name
        ));
    }
    let mut data: Vec<u8> = Vec::with_capacity(entry.comp_size.min(limit) as usize);
    with_entry_data(file, entry, |reader| {
        let mut limited = reader.take(limit + 1);
        limited
            .read_to_end(&mut data)
            .map_err(|error| format!("读取条目失败：{error}"))?;
        Ok(())
    })?;
    Ok(data)
}

/// 建文件（覆盖语义；unix 走 openat + 父目录句柄缓存）。
fn create_file(output_path: &Path) -> Result<File, String> {
    create_file_impl(output_path)
        .map_err(|error| format!("写入失败 {}：{error}", output_path.display()))
}

#[cfg(unix)]
thread_local! {
    /// 每线程缓存最近使用的父目录句柄。
    /// 安卓共享存储（FUSE）上路径逐级解析代价高昂（每级都可能是一次内核往返），
    /// 改用 openat 后每个文件只需解析文件名一段；同目录文件连续处理时命中率最高。
    static PARENT_DIR: RefCell<Option<(PathBuf, File)>> = RefCell::new(None);
}

/// 建文件（unix）：缓存父目录句柄 + `openat`，避免每个文件重新逐级解析路径。
#[cfg(unix)]
fn create_file_impl(output_path: &Path) -> io::Result<File> {
    let file_name = output_path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "无效的文件路径"))?;
    let parent = output_path.parent().unwrap_or_else(|| Path::new("."));
    PARENT_DIR.with(|cell| {
        let mut slot = cell.borrow_mut();
        let stale = match slot.as_ref() {
            Some((cached, _)) => cached.as_path() != parent,
            None => true,
        };
        if stale {
            let dir = File::open(parent)?;
            *slot = Some((parent.to_path_buf(), dir));
        }
        let dir_fd = slot
            .as_ref()
            .map(|(_, dir)| dir.as_raw_fd())
            .unwrap_or(-1);
        openat_create(dir_fd, file_name)
    })
}

/// `openat(dir_fd, name, O_WRONLY|O_CREAT|O_TRUNC)`（语义与 `File::create` 一致）。
#[cfg(unix)]
fn openat_create(dir_fd: RawFd, file_name: &OsStr) -> io::Result<File> {
    let mut name = Vec::with_capacity(file_name.len() + 1);
    name.extend_from_slice(file_name.as_bytes());
    name.push(0);
    let fd = unsafe {
        libc::openat(
            dir_fd,
            name.as_ptr() as *const libc::c_char,
            libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC | libc::O_CLOEXEC,
            0o666 as libc::c_uint,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

/// 建文件（非 unix）：直接按路径创建。
#[cfg(not(unix))]
fn create_file_impl(output_path: &Path) -> io::Result<File> {
    File::create(output_path)
}

/// 用线程本地大缓冲区拷贝数据流。
///
/// 替代「每条目新建 BufReader/BufWriter（各 128KB）」与 `io::copy`（默认 8KB 小块）：
/// 上万条目时反复申请/释放大块内存会触发 mmap 抖动，小分块则在 FUSE 上产生大量小读写。
fn copy_stream(reader: &mut dyn Read, writer: &mut File) -> io::Result<u64> {
    thread_local! {
        static COPY_BUFFER: RefCell<Vec<u8>> = RefCell::new(vec![0u8; 256 * 1024]);
    }
    COPY_BUFFER.with(|cell| {
        let mut buffer = cell.borrow_mut();
        let mut total = 0_u64;
        loop {
            let read = reader.read(&mut buffer[..])?;
            if read == 0 {
                return Ok(total);
            }
            writer.write_all(&buffer[..read])?;
            total += read as u64;
        }
    })
}

/// 校验并规范化条目路径（防 zip-slip；Windows 非法字符替换为 `_`）。
fn sanitize_entry_name(raw: &str) -> Option<PathBuf> {
    let name = raw.trim_end_matches('/');
    if name.is_empty() {
        return None;
    }
    let mut path = PathBuf::new();
    for part in name.split(['/', '\\']) {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." || part.contains(':') {
            return None;
        }
        path.push(sanitize_component(part));
    }
    if path.as_os_str().is_empty() {
        None
    } else {
        Some(path)
    }
}

/// 替换 Windows 不允许的字符，并去掉结尾的空格/点。
fn sanitize_component(part: &str) -> String {
    let mut out: String = part
        .chars()
        .map(|ch| {
            if matches!(ch, '<' | '>' | '"' | '|' | '?' | '*') || (ch as u32) < 32 {
                '_'
            } else {
                ch
            }
        })
        .collect();
    while out.ends_with(' ') || out.ends_with('.') {
        out.pop();
    }
    if out.is_empty() {
        out.push('_');
    }
    out
}

/// 定位读取：不改变句柄当前偏移，可多线程共享同一句柄
/// （Android `content://` 句柄无法按路径重开，必须共享）。
fn read_at(file: &File, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
    #[cfg(unix)]
    {
        std::os::unix::fs::FileExt::read_at(file, buffer, offset)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::FileExt::seek_read(file, buffer, offset)
    }
}

/// 从指定偏移读满整个缓冲区（循环处理短读）。
fn read_exact_at(file: &File, offset: u64, buffer: &mut [u8]) -> Result<(), String> {
    let mut done = 0usize;
    while done < buffer.len() {
        let read = read_at(file, &mut buffer[done..], offset + done as u64)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("读取超出文件末尾".to_string());
        }
        done += read;
    }
    Ok(())
}

/// 定位读取游标：实现 `Read`，供流式解压器（deflate 解码器）使用。
struct PosReader<'a> {
    file: &'a File,
    pos: u64,
}

impl<'a> PosReader<'a> {
    fn new(file: &'a File, pos: u64) -> Self {
        Self { file, pos }
    }
}

impl Read for PosReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let read = read_at(self.file, buffer, self.pos)?;
        self.pos += read as u64;
        Ok(read)
    }
}

fn read_u16_at(file: &File, offset: u64) -> Result<u16, String> {
    let mut buffer = [0u8; 2];
    read_exact_at(file, offset, &mut buffer)?;
    Ok(u16::from_le_bytes(buffer))
}

fn read_u32_at(file: &File, offset: u64) -> Result<u32, String> {
    let mut buffer = [0u8; 4];
    read_exact_at(file, offset, &mut buffer)?;
    Ok(u32::from_le_bytes(buffer))
}

fn read_u64_at(file: &File, offset: u64) -> Result<u64, String> {
    let mut buffer = [0u8; 8];
    read_exact_at(file, offset, &mut buffer)?;
    Ok(u64::from_le_bytes(buffer))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ageciv-extract-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn extracts_zip_written_by_zip_crate() {
        let dir = temp_dir("normal");
        let apk = dir.join("test.apk");
        {
            let mut zip = zip::ZipWriter::new(File::create(&apk).unwrap());
            zip.start_file(
                "assets/data.bin",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(b"hello-apk").unwrap();
            zip.start_file(
                "root.txt",
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
            zip.write_all(b"stored").unwrap();
            zip.finish().unwrap();
        }
        let dest = dir.join("out");
        fs::create_dir_all(&dest).unwrap();
        let outcome = extract_apk_contents(&apk, &dest).unwrap();
        assert_eq!(outcome.entries, 2);
        assert_eq!(outcome.skipped, 0);
        assert_eq!(
            fs::read_to_string(dest.join("assets/data.bin")).unwrap(),
            "hello-apk"
        );
        assert_eq!(fs::read_to_string(dest.join("root.txt")).unwrap(), "stored");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn lists_and_reads_entries_in_memory() {
        let dir = temp_dir("read-entry");
        let apk = dir.join("test.apk");
        {
            let mut zip = zip::ZipWriter::new(File::create(&apk).unwrap());
            zip.start_file(
                "assets/game/Civilizations.txt",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(b"atr;").unwrap();
            zip.start_file(
                "assets/game/data.json",
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
            zip.write_all(b"{\"a\":1}").unwrap();
            zip.finish().unwrap();
        }
        let file = File::open(&apk).unwrap();
        let entries = list_apk_entries(&file).unwrap();
        assert_eq!(entries.len(), 2);
        let civ = entries
            .iter()
            .find(|entry| entry.name == "assets/game/Civilizations.txt")
            .unwrap();
        assert_eq!(
            read_apk_entry_bytes(&file, civ, 1024 * 1024).unwrap(),
            b"atr;"
        );
        // 尺寸上限保护：超过 limit 的条目拒绝读取。
        assert!(read_apk_entry_bytes(&file, civ, 2).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn extract_progress_reaches_total() {
        let dir = temp_dir("progress");
        let apk = dir.join("test.apk");
        {
            let mut zip = zip::ZipWriter::new(File::create(&apk).unwrap());
            for index in 0..3 {
                zip.start_file(
                    format!("file-{index}.txt"),
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
                zip.write_all(format!("content-{index}").as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }
        let dest = dir.join("out");
        fs::create_dir_all(&dest).unwrap();
        let emissions = std::sync::Mutex::new(Vec::new());
        let outcome = extract_apk_contents_with_progress(&apk, &dest, &|done, total| {
            emissions.lock().unwrap().push((done, total));
        })
        .unwrap();
        assert_eq!(outcome.entries, 3);
        let emissions = emissions.into_inner().unwrap();
        assert_eq!(emissions.first(), Some(&(0, 3)));
        assert_eq!(emissions.last(), Some(&(3, 3)));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn tolerates_malformed_extra_fields() {
        // 手工构造一个「中央目录 extra 被截断」的 zip：常规 zip 库会拒绝，宽容解压器应能解出。
        let dir = temp_dir("malformed");
        let apk = dir.join("malformed.apk");
        let data = b"payload-123";
        let name = b"mod.txt";
        let mut bytes = Vec::new();

        // 本地文件头（无 extra）。
        bytes.extend_from_slice(&LFH_SIG.to_le_bytes());
        bytes.extend_from_slice(&20u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes()); // stored
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes()); // crc（不校验）
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(name);
        bytes.extend_from_slice(data);

        // 中央目录条目：extra 声明内容 16 字节但只有 5 字节（截断）。
        let cd_start = bytes.len() as u32;
        bytes.extend_from_slice(&CDFH_SIG.to_le_bytes());
        bytes.extend_from_slice(&20u16.to_le_bytes());
        bytes.extend_from_slice(&20u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&5u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes()); // 本地头偏移 = 0
        bytes.extend_from_slice(name);
        bytes.extend_from_slice(&[0x75, 0x70, 0x10, 0x00, 0xAA]); // Unicode 扩展：声称 16 字节
        let cd_size = bytes.len() as u32 - cd_start;

        // EOCD。
        bytes.extend_from_slice(&EOCD_SIG.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&cd_size.to_le_bytes());
        bytes.extend_from_slice(&cd_start.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        fs::write(&apk, &bytes).unwrap();

        // 宽容解压成功。
        let dest = dir.join("out");
        fs::create_dir_all(&dest).unwrap();
        let outcome = extract_apk_contents(&apk, &dest).unwrap();
        assert_eq!(outcome.entries, 1);
        assert_eq!(
            fs::read_to_string(dest.join("mod.txt")).unwrap(),
            "payload-123"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn extracts_large_entries_across_prefetch_boundary() {
        // 覆盖「数据跨越 64KB 预读边界」的链式读取路径（deflate 与 stored 各一）。
        let dir = temp_dir("large");
        let apk = dir.join("large.apk");
        let mut state: u32 = 0x1234_5678;
        let mut make_bytes = |len: usize| -> Vec<u8> {
            (0..len)
                .map(|_| {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    (state >> 24) as u8
                })
                .collect()
        };
        let big = make_bytes(300 * 1024); // 伪随机：deflate 压缩率接近 1，压缩后仍 > 64KB
        let stored = make_bytes(200 * 1024);
        {
            let mut zip = zip::ZipWriter::new(File::create(&apk).unwrap());
            zip.start_file("assets/big.bin", zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(&big).unwrap();
            zip.start_file(
                "stored.bin",
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
            zip.write_all(&stored).unwrap();
            zip.finish().unwrap();
        }
        let dest = dir.join("out");
        fs::create_dir_all(&dest).unwrap();
        let outcome = extract_apk_contents(&apk, &dest).unwrap();
        assert_eq!(outcome.entries, 2);
        assert_eq!(outcome.skipped, 0);
        assert_eq!(fs::read(dest.join("assets/big.bin")).unwrap(), big);
        assert_eq!(fs::read(dest.join("stored.bin")).unwrap(), stored);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn extracts_only_selected_prefixes() {
        // 版块过滤：只解压给定前缀（全局 missions 与指定剧本目录），其余条目静默跳过。
        let dir = temp_dir("sections");
        let apk = dir.join("sections.apk");
        {
            let mut zip = zip::ZipWriter::new(File::create(&apk).unwrap());
            for (name, contents) in [
                ("assets/game/missions/a.txt", "a"),
                ("assets/map/Earth3/scenarios/s.json", "s"),
                ("assets/other/x.txt", "x"),
                ("root.txt", "r"),
            ] {
                zip.start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(contents.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }
        let dest = dir.join("out");
        fs::create_dir_all(&dest).unwrap();
        let outcome = extract_apk_sections_with_progress(
            &apk,
            &dest,
            &["assets/game/missions/", "assets/map/Earth3/scenarios/"],
            &|_, _| {},
        )
        .unwrap();
        assert_eq!(outcome.entries, 2);
        assert_eq!(outcome.skipped, 0);
        assert!(dest.join("assets/game/missions/a.txt").is_file());
        assert!(dest.join("assets/map/Earth3/scenarios/s.json").is_file());
        assert!(!dest.join("assets/other").exists());
        assert!(!dest.join("root.txt").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn discovers_section_prefixes_for_custom_maps() {
        // 版块前缀按中央目录实际结构识别：地图 / 剧本目录名各模组自定义
        // （如白日升的 Begonia/RWS、暮色黄昏的 Earth3/TheGreatWar）。
        let dir = temp_dir("discover");
        let apk = dir.join("mod.apk");
        {
            let mut zip = zip::ZipWriter::new(File::create(&apk).unwrap());
            for name in [
                "assets/game/missions/missionsEvents/a.txt",
                "assets/game/missions/missionsImages/H/x.png",
                "assets/map/Begonia/scenarios/RWS/missions/tree.json",
                "assets/map/Begonia/scenarios/RWS/missionsEvents/e.txt",
                "assets/map/Earth3/scenarios/TheGreatWar/missions/tree.json",
                "assets/map/Maps.json",
                "assets/other/x.txt",
                "AndroidManifest.xml",
            ] {
                zip.start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(b"x").unwrap();
            }
            zip.finish().unwrap();
        }
        let file = File::open(&apk).unwrap();
        let prefixes = discover_section_prefixes(&file).unwrap();
        assert_eq!(
            prefixes,
            vec![
                "assets/game/missions/".to_string(),
                "assets/map/Begonia/scenarios/RWS/".to_string(),
                "assets/map/Earth3/scenarios/TheGreatWar/".to_string(),
            ]
        );
        // 用识别出的前缀解压：只落盘版块内容（资源文件 / 其他目录不进工作区）。
        let dest = dir.join("out");
        fs::create_dir_all(&dest).unwrap();
        let refs: Vec<&str> = prefixes.iter().map(String::as_str).collect();
        let outcome = extract_selected_from_file(&file, &dest, &refs, &|_, _| {}).unwrap();
        assert_eq!(outcome.entries, 5);
        assert!(dest
            .join("assets/map/Begonia/scenarios/RWS/missions/tree.json")
            .is_file());
        assert!(!dest.join("assets/other").exists());
        assert!(!dest.join("assets/map/Maps.json").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    /// 真实模组 APK 的版块识别（各模组自定义地图 / 剧本目录名）：
    /// `cargo test -p age_civ_mod_tool --lib discover_real_apk_section_prefixes -- --ignored`
    #[test]
    #[ignore = "需要真实模组 APK（A:\\android\\GameCivs）"]
    fn discover_real_apk_section_prefixes() {
        let cases: [(&str, &[&str]); 5] = [
            (
                r"A:\android\GameCivs\暮色黄昏_世界大战0.25.1.apk",
                &["assets/map/Earth3/scenarios/TheGreatWar/"],
            ),
            (r"A:\android\GameCivs\白日升.apk", &["assets/map/Begonia/scenarios/RWS/"]),
            (r"A:\android\GameCivs\road_to_56.apk", &["assets/map/Earth3/scenarios/WW2/"]),
            (
                r"A:\android\GameCivs\europe.apk",
                &[
                    "assets/map/ES/scenarios/RusUkrWar/",
                    "assets/map/ES/scenarios/Ukr2014/",
                ],
            ),
            (
                r"A:\android\GameCivs\1566AuroraPrever2.apk",
                &[
                    "assets/map/Earth3/scenarios/ming/",
                    "assets/map/Earth3/scenarios/province/",
                    "assets/map/Earth3/scenarios/zhu/",
                ],
            ),
        ];
        for (path, expected) in cases {
            let file = File::open(path).unwrap_or_else(|error| panic!("打开 {path} 失败：{error}"));
            let prefixes = discover_section_prefixes(&file).unwrap();
            assert!(
                prefixes.iter().any(|prefix| prefix == "assets/game/missions/"),
                "{path} 缺全局 missions 版块"
            );
            for prefix in expected {
                assert!(
                    prefixes.iter().any(|item| item == prefix),
                    "{path} 缺 {prefix}（实际：{prefixes:?}）"
                );
            }
        }
    }
}
