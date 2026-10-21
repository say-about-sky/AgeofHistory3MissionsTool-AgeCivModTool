//! 国策树配置（Missions.json 及多国策树文件）的读写命令。

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::SystemTime;

use rayon::prelude::*;
use serde::Serialize;

use super::apk::ProgressEmitter;
use super::events::parse_event_id;

use crate::decision_format::parse_decision_file_with_correction;
use crate::mission_format::{parse_mission_file_with_correction, serialize_mission_file};
use crate::models::MissionRecord;
use crate::paths::{
    missions_directory, missions_root_path, validate_work_directory_name, workspace_abs_path,
};

/// `parse_missions` 的返回：解析出的国策记录；语法被自动纠正时附带规范化后的完整内容，
/// 由前端（Android scoped 通道）负责写回原文件。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedMissions {
    missions: Vec<MissionRecord>,
    corrected_contents: Option<String>,
}

/// 解析国策配置文本（宽松语法会自动纠正，并返回规范化内容供调用方写回）。
#[tauri::command]
pub fn parse_missions(contents: String) -> Result<ParsedMissions, String> {
    let (file, corrected) = parse_mission_file_with_correction(&contents)?;
    Ok(ParsedMissions {
        missions: file.mission,
        corrected_contents: corrected,
    })
}

/// 规范化并序列化国策列表（按顺序重写 ID；`MissionEvent` 等其余字段原样保留）。
///
/// 注意：不再按标题补 `MissionEvent`。标题（`Name`）与事件脚本名（`MissionEvent`）
/// 是两个独立字段，工具不得在保存时把它们隐式绑定起来。
#[tauri::command]
pub fn serialize_missions(mut missions: Vec<MissionRecord>) -> Result<String, String> {
    for (index, mission) in missions.iter_mut().enumerate() {
        mission.id = index as i64;
    }
    serialize_mission_file(&missions)
}

/// 读取并解析国策配置文件；若是宽松语法，自动纠正并把规范内容写回原文件。
fn read_missions_file_with_autofix(path: &Path) -> Result<Vec<MissionRecord>, String> {
    let content = fs::read_to_string(path)
        .map_err(|error| format!("读取国策配置失败 {}：{error}", path.display()))?;
    let (file, corrected) = parse_mission_file_with_correction(&content)?;
    if let Some(corrected) = corrected {
        fs::write(path, corrected).map_err(|error| {
            format!(
                "国策文件语法已自动纠正，但写回失败 {}：{error}",
                path.display()
            )
        })?;
    }
    Ok(file.mission)
}

/// 加载工作目录下的 `missions/Missions.json`。
#[tauri::command]
pub fn load_missions(work_directory: String) -> Result<Vec<MissionRecord>, String> {
    let path = missions_directory(&work_directory)?.join("Missions.json");
    read_missions_file_with_autofix(&path)
}

/// 保存国策列表到工作目录下的 `missions/Missions.json`。
#[tauri::command]
pub fn save_missions(work_directory: String, missions: Vec<MissionRecord>) -> Result<(), String> {
    let content = serialize_missions(missions)?;
    fs::write(missions_directory(&work_directory)?.join("Missions.json"), content)
        .map_err(|error| error.to_string())
}

/// 按文件名读取指定国策资源目录（`missions_root`）下的单个国策树配置（多国策树工作区）。
/// `missions_root` 为工作区相对路径，如 `missions`、`assets/game/missions` 或
/// `assets/map/<地图>/scenarios/<剧本>/missions`。
/// 若文件使用宽松语法（缺逗号/裸文本值等），打开时自动纠正为规范格式并写回原文件。
#[tauri::command]
pub fn load_missions_file(
    work_directory: String,
    missions_root: String,
    file_name: String,
) -> Result<Vec<MissionRecord>, String> {
    validate_work_directory_name(&file_name)?;
    let path = missions_root_path(&work_directory, &missions_root)?.join(&file_name);
    read_missions_file_with_autofix(&path)
}

/// 按文件名保存单个国策树配置到指定的国策资源目录下。
#[tauri::command]
pub fn save_missions_file(
    work_directory: String,
    missions_root: String,
    file_name: String,
    missions: Vec<MissionRecord>,
) -> Result<(), String> {
    validate_work_directory_name(&file_name)?;
    let content = serialize_missions(missions)?;
    fs::write(
        missions_root_path(&work_directory, &missions_root)?.join(&file_name),
        content,
    )
    .map_err(|error| error.to_string())
}

/// 打包 / 导出前检查：列出 `missionsEvents` 目录内未被任何国策 / 决议引用的脚本副本。
///
/// `scope` 为工作区相对目录（空串 = 整个工作区；打包时传打包源目录）。
/// 「被引用」= 所在国策根的 `*.json` 国策树里出现 `MissionEvent`（缺省时**保守地**把
/// 「标题.txt」也算作引用，避免误报导致误删；编辑器本身不采用该回退），或同根事件脚本内被 `run_event` / `run_event_instantly` 引用，
/// 或决议定义文件（`rainfall/rfEvent_decision.json`）的 `events` 项引用该脚本
/// （按文件名主干或脚本内部 `id=` 行匹配；去尾部 `.txt`、大小写不敏感；
/// 工作区内相关决议文件全部合并计入）。
/// 保守判断：宁可多算引用，避免误报导致误删。
/// 只读检查：宽松语法的文件不会在此被写回纠正。
/// 没有任何可解析国策树的根不做判定（可能尚未导入完整，避免误导用户全删）。
/// 返回顺序：按文件的「创建/修改时间」中较晚者倒序（最新的排最前；同一时间按路径升序；
/// 创建时间在部分平台不支持时自动回退修改时间）。
/// 性能：命令在 `spawn_blocking` 内执行；先遍历出全部国策根，再并行处理各根
/// （根内国策树解析与脚本读取同样并行，rayon）——数千脚本也能快速完成。
/// 扫描期间通过 `apk-progress` 事件上报文件级进度（前端加载面板显示进度条）。
#[tauri::command]
pub async fn find_unused_event_scripts<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    work_directory: String,
    scope: String,
) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let emitter = ProgressEmitter::new(app, "unused-scripts");
        find_unused_event_scripts_blocking(work_directory, scope, &|completed: u64, total: u64| {
            emitter.emit("正在检查未使用脚本...", "files", completed, total);
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

/// 扫描进度回调：(已完成文件数, 总文件数)；在并行线程中调用，须 `Sync`。
type ScanProgress<'a> = &'a (dyn Fn(u64, u64) + Sync);

/// `find_unused_event_scripts` 的同步实现（在 `spawn_blocking` 线程中运行）。
/// `progress` 每扫描一个脚本回调一次（节流由调用方处理）。
/// 检查期间同时收集决议文件（`rainfall/rfEvent_decision.json`）的 `events` 引用：
/// scope 内遍历发现的 + 由各国策根向上逐级补查的（合并去重，见
/// [`collect_decision_references`]）。
fn find_unused_event_scripts_blocking(
    work_directory: String,
    scope: String,
    progress: ScanProgress<'_>,
) -> Result<Vec<String>, String> {
    let workspace_root = PathBuf::from(&work_directory);
    if !workspace_root.is_dir() {
        return Err(format!("工作目录不存在：{work_directory}"));
    }
    let scope_directory = if scope.trim().is_empty() {
        workspace_root.clone()
    } else {
        workspace_abs_path(&work_directory, scope.trim())?
    };
    // 先遍历出全部国策根（missions 与其 missionsEvents），随后并行处理；
    // 同时收集 scope 内的决议定义文件（`rainfall/rfEvent_decision.json`）。
    let mut roots: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut decision_files: Vec<PathBuf> = Vec::new();
    // scope 本身就是 missions 根时直接检查（常规打包传入的是上级目录）。
    if scope_directory
        .file_name()
        .is_some_and(|name| name == "missions")
    {
        let events = scope_directory.join("missionsEvents");
        if events.is_dir() {
            roots.push((scope_directory, events));
        }
    } else {
        let mut pending = vec![scope_directory];
        while let Some(directory) = pending.pop() {
            let Ok(entries) = fs::read_dir(&directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(metadata) = fs::symlink_metadata(&path) else {
                    continue;
                };
                if !metadata.file_type().is_dir() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                if name == "rainfall" {
                    // 决议定义文件（模组资源根的 `rainfall/` 版块；不存在则跳过）。
                    let decision_file = path.join("rfEvent_decision.json");
                    if decision_file.is_file() {
                        decision_files.push(decision_file);
                    }
                }
                if name == "missions" {
                    // `missions` 下直接查找 missionsEvents（其内部不会再有国策根）。
                    let events = path.join("missionsEvents");
                    if events.is_dir() {
                        roots.push((path, events));
                    }
                    continue;
                }
                // 跳过事件/图标目录，避免无谓遍历。
                if name == "missionsEvents" || name == "missionsImages" {
                    continue;
                }
                pending.push(path);
            }
        }
    }
    // 决议文件也可能位于国策根的上层（如解包布局的 `<模组>/assets/rainfall/`，
    // 或工作区根布局——scope 限定为子目录时遍历收集不到）：对每个根向上逐级
    // 补查直到工作区根。全部去重后解析为引用集合（保守：相关决议全部合并计入）。
    for (missions_root, _) in &roots {
        let mut current = missions_root.parent();
        while let Some(directory) = current {
            if !directory.starts_with(&workspace_root) {
                break;
            }
            let decision_file = directory.join("rainfall").join("rfEvent_decision.json");
            if decision_file.is_file() {
                decision_files.push(decision_file);
            }
            if directory == workspace_root {
                break;
            }
            current = directory.parent();
        }
    }
    decision_files.sort();
    decision_files.dedup();
    let decision_references = collect_decision_references(&decision_files);
    // 预列表各根脚本（并行）：得到进度条总数，列表随后直接用于扫描。
    let root_scans: Vec<(PathBuf, Vec<(String, PathBuf)>)> = roots
        .par_iter()
        .map(|(missions_root, events_directory)| {
            (missions_root.clone(), list_scripts(events_directory))
        })
        .collect();
    let total = root_scans
        .iter()
        .map(|(_, scripts)| scripts.len())
        .sum::<usize>();
    progress(0, total as u64);
    // 各根并行处理；根内再各自并行读文件（rayon 嵌套并行自动调度）；
    // 共享已完成计数驱动进度回调（发送端按时间节流）。
    let completed = AtomicUsize::new(0);
    let unused: Vec<(SystemTime, String)> = root_scans
        .par_iter()
        .flat_map(|(missions_root, scripts)| {
            collect_unused_scripts(
                &workspace_root,
                missions_root,
                scripts,
                &decision_references,
                &completed,
                total,
                progress,
            )
        })
        .collect();
    // 强制收尾（个别根因无可解析国策树被跳过时，完成数可能不足总数）。
    progress(total as u64, total as u64);
    Ok(unused_paths_sorted_by_latest_time(unused))
}

/// 列出 `missionsEvents` 内的 `.txt` 脚本（文件名是否小写在扫描阶段比对）。
fn list_scripts(events_directory: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = fs::read_dir(events_directory) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file() {
                return None;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            name.to_ascii_lowercase()
                .ends_with(".txt")
                .then_some((name, path))
        })
        .collect()
}

/// 单个国策根：收集被引用的脚本名并判定 `missionsEvents` 内未使用的 `.txt`（并行读取）。
/// `scripts` 为预列表结果；每扫描一个脚本递增 `completed` 并回调 `progress`。
/// `decision_references` 为决议 `events` 引用的归一化键（工作区级合并，见
/// [`collect_decision_references`]）：命中脚本文件名主干或内部 `id` 时视为已使用。
fn collect_unused_scripts(
    workspace_root: &Path,
    missions_root: &Path,
    scripts: &[(String, PathBuf)],
    decision_references: &HashSet<String>,
    completed: &AtomicUsize,
    total: usize,
    progress: ScanProgress<'_>,
) -> Vec<(SystemTime, String)> {
    // 1) 并行解析根内全部国策树，合并引用（MissionEvent；缺省时保守地把「标题.txt」
    //    也算作引用——仅用于「未使用脚本」检查，编辑器不采用该回退）。
    let mut tree_references = Vec::new();
    let mut tree_parsed = false;
    if let Ok(entries) = fs::read_dir(missions_root) {
        let tree_files: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_file()
                    && path
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
            })
            .collect();
        let parsed: Vec<Option<Vec<String>>> = tree_files
            .par_iter()
            .map(|path| parse_tree_references(path))
            .collect();
        for references in parsed.into_iter().flatten() {
            tree_parsed = true;
            tree_references.extend(references);
        }
    }
    if !tree_parsed {
        // 该根没有任何可解析的国策树：不判定（可能尚未导入完整）。
        return Vec::new();
    }
    // 2) 并行读取脚本内容收集 run_event 引用并记录时间戳（逐文件上报进度）。
    let scanned: Vec<ScannedScript> = scripts
        .par_iter()
        .map(|(name, path)| {
            let scanned = scan_script(workspace_root, name, path);
            let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
            progress(done as u64, total as u64);
            scanned
        })
        .collect();
    // 脚本之间可能互相引用（run_event）：先合并全部脚本内引用，再判定未使用。
    let mut referenced = HashSet::<String>::new();
    referenced.extend(tree_references);
    for script in &scanned {
        referenced.extend(script.references.iter().cloned());
    }
    scanned
        .into_iter()
        .filter(|script| !referenced.contains(&script.name))
        .filter(|script| !referenced_by_decision(script, decision_references))
        .filter_map(|script| {
            script
                .relative
                .map(|relative| (script.latest_time, relative))
        })
        .collect()
}

/// 单个脚本文件的扫描结果（在并行 map 中构建，避免重复读写）。
struct ScannedScript {
    /// 文件名（小写，用于与引用集合比对）。
    name: String,
    /// 脚本内部 `id=` 行值（小写；无 id / 读取失败为 None——决议 `events`
    /// 索引的是脚本内部 id）。
    id: Option<String>,
    /// 「创建/修改时间」中较晚者（读取失败按最早处理）。
    latest_time: SystemTime,
    /// 工作区相对路径（`/` 分隔）；无法推导时为 None。
    relative: Option<String>,
    /// 脚本内 `run_event` / `run_event_instantly` 引用（小写、含 `.txt`）。
    references: Vec<String>,
}

/// 读取单个脚本：收集 run_event 引用 / 内部 id + 时间戳 + 相对路径（在并行 map 中调用）。
fn scan_script(workspace_root: &Path, name: &str, path: &Path) -> ScannedScript {
    let content = fs::read_to_string(path).ok();
    // 决议 `events` 索引的是脚本内部 id（见 `commands::events` 模块头注释）。
    let id = content
        .as_deref()
        .and_then(parse_event_id)
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty());
    let references = content
        .as_deref()
        .map(collect_run_event_references)
        .unwrap_or_default();
    // 「创建/修改时间」取两者中较晚者；读取失败按最早处理（排序时放最后）。
    // 创建时间在部分平台（Linux/Android）不支持，自动回退修改时间。
    let metadata = fs::metadata(path).ok();
    let modified = metadata
        .as_ref()
        .and_then(|metadata| metadata.modified().ok())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let created = metadata
        .as_ref()
        .and_then(|metadata| metadata.created().ok())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    ScannedScript {
        name: name.to_ascii_lowercase(),
        id,
        latest_time: modified.max(created),
        relative: path
            .strip_prefix(workspace_root)
            .ok()
            .map(|relative| relative.to_string_lossy().replace('\\', "/")),
        references,
    }
}

/// 解析单个国策树文件，返回其声明的脚本引用；解析失败返回 None（不计入 tree_parsed）。
fn parse_tree_references(path: &Path) -> Option<Vec<String>> {
    let content = fs::read_to_string(path).ok()?;
    let (file, _) = parse_mission_file_with_correction(&content).ok()?;
    let mut references = Vec::new();
    for record in file.mission {
        let event = record.mission_event.trim();
        if !event.is_empty() {
            references.push(event.to_ascii_lowercase());
        } else if !record.name.trim().is_empty() {
            // 保守口径（仅用于「未使用脚本」检查）：MissionEvent 缺省时仍把「标题.txt」
            // 算作引用，宁可多算，避免误报导致误删脚本。
            // 编辑器不采用该回退：标题（Name）与 MissionEvent 是两个独立字段。
            references.push(format!("{}.txt", record.name.trim()).to_ascii_lowercase());
        }
    }
    Some(references)
}

/// 解析决议文件集合，合并其声明的脚本引用（归一化键，见
/// [`normalize_decision_reference`]）。文件读取或解析失败时忽略该文件
/// （与国策树 / 脚本扫描的容错一致，单个坏文件不中断检查）。
fn collect_decision_references(decision_files: &[PathBuf]) -> HashSet<String> {
    let mut references = HashSet::new();
    for path in decision_files {
        let Ok(content) = fs::read_to_string(path) else {
            continue;
        };
        let Ok((file, _)) = parse_decision_file_with_correction(&content) else {
            continue;
        };
        for group in &file.decisions {
            for event in &group.events {
                let key = normalize_decision_reference(event);
                if !key.is_empty() {
                    references.insert(key);
                }
            }
        }
    }
    references
}

/// 决议 `events` 项归一化：去首尾空白 → 去尾部 `.txt`（大小写不敏感）→ 小写。
/// 与脚本侧（文件名主干 / 内部 id 的小写形式）比对；宽松口径（宁多算引用）。
fn normalize_decision_reference(value: &str) -> String {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    let core = if lower.ends_with(".txt") {
        &trimmed[..trimmed.len() - 4]
    } else {
        trimmed
    };
    core.trim().to_lowercase()
}

/// 脚本副本是否被决议引用：文件名主干或内部 `id` 命中决议引用集合。
fn referenced_by_decision(script: &ScannedScript, decision_references: &HashSet<String>) -> bool {
    if decision_references.is_empty() {
        return false;
    }
    let stem = script.name.strip_suffix(".txt").unwrap_or(&script.name);
    decision_references.contains(stem)
        || script
            .id
            .as_deref()
            .is_some_and(|id| decision_references.contains(id))
}

/// 未使用脚本排序：按「创建/修改时间」中较晚者倒序（最新的排最前）；时间相同按路径升序保证稳定。
fn unused_paths_sorted_by_latest_time(mut unused: Vec<(SystemTime, String)>) -> Vec<String> {
    unused.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    unused.into_iter().map(|(_, path)| path).collect()
}

/// 收集事件脚本内的 `run_event` / `run_event_instantly` 引用（值 = 脚本文件名去扩展名）。
/// 保守扫描：行内以这两个键开头且含 `=` 时，整段值记入引用（宁可多算不误报）。
fn collect_run_event_references(content: &str) -> Vec<String> {
    let mut referenced = Vec::new();
    for line in content.lines() {
        let line = line.trim_start();
        let rest = if let Some(rest) = line.strip_prefix("run_event_instantly") {
            rest
        } else if let Some(rest) = line.strip_prefix("run_event") {
            rest
        } else {
            continue;
        };
        let Some(value) = rest.split_once('=').map(|(_, value)| value.trim()) else {
            continue;
        };
        let name = value.trim_matches(|character| character == '"' || character == '\'');
        if !name.is_empty() && !name.contains('/') && !name.contains('\\') {
            referenced.push(format!("{}.txt", name.to_ascii_lowercase()));
        }
    }
    referenced
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{Duration, SystemTime};

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ageciv-unused-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 显式设置文件时间戳（测试排序用；不依赖文件创建/写入的先后）。
    fn set_times(path: &Path, times: fs::FileTimes) {
        let file = fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_times(times).unwrap();
    }

    /// 经典根与解包前缀下的未使用脚本：树引用 / 名字回退 / run_event 引用都算「已使用」；
    /// 按创建/修改较晚者倒序返回；scope 限定子树；没有可解析国策树的根不判定。
    #[test]
    fn finds_unused_event_scripts_and_respects_scope() {
        let dir = temp_dir("events");
        fs::create_dir_all(dir.join("missions/missionsEvents")).unwrap();
        fs::write(
            dir.join("missions/Missions.json"),
            r#"{"Mission":[{"ID":0,"Name":"甲","MissionEvent":"used.txt"},{"ID":1,"Name":"乙"}]}"#,
        )
        .unwrap();
        fs::write(
            dir.join("missions/missionsEvents/used.txt"),
            "id=used\ntitle=x\nrun_event=chain\n",
        )
        .unwrap();
        fs::write(dir.join("missions/missionsEvents/chain.txt"), "id=chain\n").unwrap();
        fs::write(dir.join("missions/missionsEvents/乙.txt"), "id=yi\n").unwrap();
        fs::write(dir.join("missions/missionsEvents/orphan0.txt"), "id=o0\n").unwrap();
        fs::write(dir.join("missions/missionsEvents/orphan1.txt"), "id=o1\n").unwrap();
        fs::create_dir_all(dir.join("OtherAPK/assets/game/missions/missionsEvents")).unwrap();
        fs::write(
            dir.join("OtherAPK/assets/game/missions/Missions.json"),
            r#"{"Mission":[{"ID":0,"Name":"丙","MissionEvent":"g_used.txt"}]}"#,
        )
        .unwrap();
        fs::write(
            dir.join("OtherAPK/assets/game/missions/missionsEvents/g_used.txt"),
            "id=g\n",
        )
        .unwrap();
        fs::write(
            dir.join("OtherAPK/assets/game/missions/missionsEvents/g_orphan.txt"),
            "id=go\n",
        )
        .unwrap();

        // 时间设计（取创建/修改中较晚者；Windows 上创建时间可用）：
        // orphan0 修改时间设为未来(base+60) → 排第一；
        // g_orphan 修改时间设为过去(−7200s)，由「创建时间」(晚于 orphan1 写入) 决定 → 排第二
        //   （若实现错误地只看修改时间，g_orphan 会掉到最后）；
        // orphan1 不设置 → 创建/修改均为自然写入时刻 → 排最后。
        let base = SystemTime::now();
        set_times(
            &dir.join("missions/missionsEvents/orphan0.txt"),
            fs::FileTimes::new().set_modified(base + Duration::from_secs(60)),
        );
        set_times(
            &dir.join("OtherAPK/assets/game/missions/missionsEvents/g_orphan.txt"),
            fs::FileTimes::new().set_modified(base - Duration::from_secs(7200)),
        );

        let work = dir.to_string_lossy().into_owned();
        assert_eq!(
            find_unused_event_scripts_blocking(work.clone(), String::new(), &|_, _| {}).unwrap(),
            vec![
                "missions/missionsEvents/orphan0.txt",
                "OtherAPK/assets/game/missions/missionsEvents/g_orphan.txt",
                "missions/missionsEvents/orphan1.txt",
            ]
        );
        // scope 限定子树（打包某个 APK 内容目录时只检查该目录）。
        assert_eq!(
            find_unused_event_scripts_blocking(work.clone(), "OtherAPK".to_string(), &|_, _| {})
                .unwrap(),
            vec!["OtherAPK/assets/game/missions/missionsEvents/g_orphan.txt"]
        );
        // 没有可解析国策树的根不判定（安全阀）：移除树文件后该根不再上报。
        fs::remove_file(dir.join("missions/Missions.json")).unwrap();
        assert_eq!(
            find_unused_event_scripts_blocking(work, String::new(), &|_, _| {}).unwrap(),
            vec!["OtherAPK/assets/game/missions/missionsEvents/g_orphan.txt"]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// 决议引用（`rainfall/rfEvent_decision.json` 的 `events`）：脚本文件名主干 /
    /// 内部 id / 尾部 `.txt` / 大小写不敏感都算「已使用」；决议文件位于工作区根
    /// （打包 scope 外）时由国策根向上逐级补查找到；宽松语法决议文件可解析。
    #[test]
    fn decision_references_mark_scripts_as_used() {
        let dir = temp_dir("decision");
        let events = dir.join("modX/assets/game/missions/missionsEvents");
        fs::create_dir_all(&events).unwrap();
        fs::create_dir_all(dir.join("modX/assets/rainfall")).unwrap();
        fs::create_dir_all(dir.join("rainfall")).unwrap();
        fs::write(
            dir.join("modX/assets/game/missions/Missions.json"),
            r#"{"Mission":[{"ID":0,"Name":"甲","MissionEvent":"base_used.txt"}]}"#,
        )
        .unwrap();
        // 模组级决议（宽松语法：对象间缺逗号 + 尾随逗号）：按带后缀 / 主干匹配。
        fs::write(
            dir.join("modX/assets/rainfall/rfEvent_decision.json"),
            "{\n\t\"decisions\":[\n\t{\n\t\t\"id\":\"A\",\n\t\t\"name\":\"甲\",\n\t\t\"events\":[\"  MODNAME.txt \"],\n\t}\n\t{\n\t\t\"id\":\"B\",\n\t\t\"name\":\"乙\",\n\t\t\"events\":[\"脚本三\"],\n\t}\n\t]\n}",
        )
        .unwrap();
        // 工作区根级决议（打包 scope = modX 时不在遍历范围内 → 由根上溯补查）。
        fs::write(
            dir.join("rainfall/rfEvent_decision.json"),
            r#"{"decisions":[{"id":"R","name":"根","events":["root_ref","根id引用"]}]}"#,
        )
        .unwrap();
        fs::write(events.join("base_used.txt"), "id=base\n").unwrap();
        fs::write(events.join("root_ref.txt"), "id=r1\n").unwrap();
        fs::write(events.join("根id脚本.txt"), "id=根id引用\n").unwrap();
        fs::write(events.join("ModName.txt"), "id=m1\n").unwrap();
        fs::write(events.join("脚本三.txt"), "id=m3\n").unwrap();
        fs::write(events.join("未使用.txt"), "id=o\n").unwrap();

        let work = dir.to_string_lossy().into_owned();
        assert_eq!(
            find_unused_event_scripts_blocking(work, "modX".to_string(), &|_, _| {}).unwrap(),
            vec!["modX/assets/game/missions/missionsEvents/未使用.txt"]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// 实测扫描速度（忽略）：对本机真实工作区运行；
    /// `cargo test -p age_civ_mod_tool --lib benchmark_unused_scripts_scan -- --ignored --nocapture`
    #[test]
    #[ignore = "需要真实工作区 A:\\android\\GameCivs"]
    fn benchmark_unused_scripts_scan() {
        let work_directory = r"A:\android\GameCivs".to_string();
        let start = std::time::Instant::now();
        let unused =
            find_unused_event_scripts_blocking(work_directory.clone(), String::new(), &|_, _| {})
                .unwrap();
        eprintln!(
            "BENCH all: unused {} in {:?}",
            unused.len(),
            start.elapsed()
        );
        for scope in ["暮色黄昏_世界大战0.25.1", "1566AuroraPrever2"] {
            let start = std::time::Instant::now();
            let unused =
                find_unused_event_scripts_blocking(
                    work_directory.clone(),
                    scope.to_string(),
                    &|_, _| {},
                )
                .unwrap();
            eprintln!("BENCH {scope}: unused {} in {:?}", unused.len(), start.elapsed());
        }
    }

    /// 真实工作区回归（忽略）：决议引用的脚本副本不得出现在未使用列表——
    /// 暮色黄昏 TheGreatWar 剧本的 3 个决议事件脚本（对照数据实测：不被国策树
    /// 引用、不被 run_event 引用，仅被 `rainfall/rfEvent_decision.json` 的
    /// `events` 按文件名引用），去掉决议检查后必被误报。
    /// `cargo test -p age_civ_mod_tool --lib verify_decision_references_against_real_workspace -- --ignored --nocapture`
    #[test]
    #[ignore = "需要真实工作区 A:\\android\\GameCivs"]
    fn verify_decision_references_against_real_workspace() {
        let work_directory = r"A:\android\GameCivs".to_string();
        let unused = find_unused_event_scripts_blocking(
            work_directory,
            "暮色黄昏_世界大战0.25.1".to_string(),
            &|_, _| {},
        )
        .unwrap();
        let decision_used = [
            "暮色黄昏_世界大战0.25.1/assets/map/Earth3/scenarios/TheGreatWar/missions/missionsEvents/罗马尼亚解放保加利亚.txt",
            "暮色黄昏_世界大战0.25.1/assets/map/Earth3/scenarios/TheGreatWar/missions/missionsEvents/罗马尼亚解放希腊.txt",
            "暮色黄昏_世界大战0.25.1/assets/map/Earth3/scenarios/TheGreatWar/missions/missionsEvents/罗马尼亚解放阿尔巴尼亚.txt",
        ];
        for path in decision_used {
            assert!(
                !unused.iter().any(|item| item == path),
                "决议引用的脚本副本被误报为未使用：{path}"
            );
        }
        eprintln!("REAL: unused {}（决议引用的 3 个脚本副本均未被误报）", unused.len());
    }
}
