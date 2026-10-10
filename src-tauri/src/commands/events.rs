//! 国策 / 模组事件脚本的读写命令。
//!
//! 事件目录为工作区内相对路径，支持两类布局：
//! - 国策事件：`<国策资源根>/missionsEvents`（如 `assets/game/missions/missionsEvents`）；
//! - 全局 / 剧本事件：`<…>/events/<子目录>`（如 `assets/game/events/common`，
//!   决议事件脚本的常用位置）。

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use crate::android_fs_bridge as bridge;
use crate::paths::{validate_relative_path, validate_work_directory_name};

// ===== 事件脚本 id 索引 =====
//
// 决议 `events` 字段索引的是脚本**内部 `id`**（而非 txt 文件名）——很多模组两者不同
// （如 `chi改任改革派1.txt` 的 id 为 `改任维新派1`，实现 959 个脚本中 299 个如此）。
// 双击事件在文件名主干未命中时，改用本索引按 id 解析路径：
// 候选路径由前端传入（已按模组前缀过滤），后端「按需扫描 + 会话级缓存」——
// 同一模组首次按 id 解析会读一遍事件脚本（桌面毫秒级；SAF 每文件一次桥调用，大模组较慢），
// 之后（含其他 id）直接命中缓存。写 / 改名 / 删除事件脚本时失效对应条目。

/// 单个工作区的事件脚本 id 索引。
#[derive(Default)]
struct EventIdIndex {
    /// 已解析文件 → 其 id（`None` = 无 id / 读取失败）。
    scanned: HashMap<String, Option<String>>,
    /// id（小写）→ 工作区相对路径。
    by_id: HashMap<String, String>,
}

/// 索引缓存：键 = 工作区（真实路径或 scoped folder_id）。
static EVENT_ID_INDEX: OnceLock<Mutex<HashMap<String, EventIdIndex>>> = OnceLock::new();

fn event_id_index() -> &'static Mutex<HashMap<String, EventIdIndex>> {
    EVENT_ID_INDEX.get_or_init(|| Mutex::new(HashMap::new()))
}

/// id 比较键（去首尾空白 + 小写）。
fn event_key(value: &str) -> String {
    value.trim().to_lowercase()
}

/// 解析脚本内部 `id=` 行（容忍 `id = x` 空格、引号、UTF-8 BOM；取首个非空值）。
pub(crate) fn parse_event_id(contents: &str) -> Option<String> {
    for line in contents.lines() {
        let trimmed = line.trim().trim_start_matches('\u{feff}');
        let Some(rest) = trimmed.strip_prefix("id") else {
            continue;
        };
        let Some(value) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

/// 文件名主干（去 `.txt`，大小写不敏感）。
fn script_stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    if name.len() > 4 && name[name.len() - 4..].eq_ignore_ascii_case(".txt") {
        &name[..name.len() - 4]
    } else {
        name
    }
}

/// 文件名主干与事件名一致的候选（可能多个目录同名；需读内容验证 id）。
fn stem_candidates<'a>(paths: &'a [String], event_name: &str) -> Vec<&'a String> {
    let wanted = event_name.trim();
    paths
        .iter()
        .filter(|path| script_stem(path).eq_ignore_ascii_case(wanted))
        .collect()
}

/// 缓存中查 id → 路径（校验路径仍在当前候选列表内，防已删除残留）。
fn cached_lookup(index: &EventIdIndex, wanted: &str, paths: &[String]) -> Option<String> {
    let found = index.by_id.get(wanted)?;
    paths.iter().find(|path| path == &found).cloned()
}

/// 写入索引（解析结果 `(path, id)` 列表）。
/// 入库键以**内部 id 为准**；文件无 id 时退回文件名主干（引擎对无 id 脚本按名索引的兜底）。
fn index_insert(index: &mut EventIdIndex, entries: Vec<(String, Option<String>)>) {
    for (path, id) in entries {
        let key = id
            .as_deref()
            .map(event_key)
            .unwrap_or_else(|| event_key(script_stem(&path)));
        index.by_id.insert(key, path.clone());
        index.scanned.insert(path, id);
    }
}

/// 验证「文件名命中」候选：`id` 未记录则读入。返回是否采用（`id` 与查询一致或文件无 id）。
fn stem_candidate_ok(index: &EventIdIndex, path: &str, wanted: &str) -> Option<bool> {
    let scanned = index.scanned.get(path)?;
    Some(
        scanned
            .as_deref()
            .map_or(true, |id| event_key(id) == wanted),
    )
}

/// 失效某个事件脚本路径的缓存（保存 / 改名 / 删除后调用；相对路径含 `/`）。
pub(crate) fn invalidate_event_script_cache(relative_path: &str) {
    if let Ok(mut guard) = event_id_index().lock() {
        for index in guard.values_mut() {
            if index.scanned.remove(relative_path).is_some() {
                index.by_id.retain(|_, path| path != relative_path);
            }
        }
    }
}

/// 按事件 id 解析事件脚本路径（真实路径模式）——见本模块头部「事件脚本 id 索引」。
/// `paths` 为候选脚本工作区相对路径（调用方已按模组前缀过滤）。
/// 解析次序（**内部 id 语义优先于文件名**）：
/// 1) 文件名命中候选：读内容验证（id 与查询名一致 / 文件无 id 才采用——如 `16.txt`
///    文件名是 `16` 但内部 id 不同时不得命中）；
/// 2) 缓存 / 按需扫描剩余候选，按内部 id（无 id 则文件名主干）匹配。
/// `Ok(None)` = 候选范围内没有该 id 的脚本。
#[tauri::command]
pub fn resolve_event_script_by_id(
    work_directory: String,
    paths: Vec<String>,
    event_name: String,
) -> Result<Option<String>, String> {
    if event_name.trim().is_empty() {
        return Ok(None);
    }
    let root = PathBuf::from(&work_directory);
    let wanted = event_key(&event_name);
    let mut guard = event_id_index()
        .lock()
        .map_err(|_| "事件索引锁异常".to_string())?;
    let index = guard.entry(work_directory.clone()).or_default();
    // 1) 文件名命中候选：读内容验证 id（id 语义优先）。
    for path in stem_candidates(&paths, &event_name) {
        if !index.scanned.contains_key(path) {
            let contents = fs::read_to_string(root.join(path)).ok();
            let id = contents.as_deref().and_then(parse_event_id);
            index_insert(index, vec![(path.clone(), id)]);
        }
        if stem_candidate_ok(index, path, &wanted) == Some(true) {
            return Ok(Some(path.clone()));
        }
    }
    if let Some(found) = cached_lookup(index, &wanted, &paths) {
        return Ok(Some(found));
    }
    // 2) 扫描尚未解析过的候选（按内部 id / 文件名主干入库）。
    let mut entries = Vec::new();
    for path in &paths {
        if index.scanned.contains_key(path) {
            continue;
        }
        let id = fs::read_to_string(root.join(path))
            .ok()
            .as_deref()
            .and_then(parse_event_id);
        entries.push((path.clone(), id));
    }
    index_insert(index, entries);
    Ok(cached_lookup(index, &wanted, &paths))
}

/// [`resolve_event_script_by_id`] 的 SAF/scoped 版：候选文件经 bridge 读取。
#[tauri::command]
pub async fn resolve_event_script_by_id_scoped<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    folder_id: String,
    paths: Vec<String>,
    event_name: String,
) -> Result<Option<String>, String> {
    if folder_id.trim().is_empty() {
        return Err("缺少 scoped 目录授权".to_string());
    }
    if event_name.trim().is_empty() {
        return Ok(None);
    }
    let wanted = event_key(&event_name);
    // 1) 文件名命中候选：读内容验证 id（id 语义优先；锁不跨 await）。
    for path in stem_candidates(&paths, &event_name) {
        let cached = {
            let guard = event_id_index()
                .lock()
                .map_err(|_| "事件索引锁异常".to_string())?;
            guard
                .get(&folder_id)
                .and_then(|index| index.scanned.get(path))
                .cloned()
        };
        let id = match cached {
            Some(id) => id,
            None => {
                let contents = bridge::read_text_file(&app, &folder_id, path).await.ok();
                let id = contents.as_deref().and_then(parse_event_id);
                let mut guard = event_id_index()
                    .lock()
                    .map_err(|_| "事件索引锁异常".to_string())?;
                let index = guard.entry(folder_id.clone()).or_default();
                index_insert(index, vec![(path.clone(), id.clone())]);
                id
            }
        };
        if id.as_deref().map_or(true, |id| event_key(id) == wanted) {
            return Ok(Some(path.clone()));
        }
    }
    // 2) 缓存 / 扫描剩余候选。
    let to_scan: Vec<String> = {
        let guard = event_id_index()
            .lock()
            .map_err(|_| "事件索引锁异常".to_string())?;
        match guard.get(&folder_id) {
            Some(index) => {
                if let Some(found) = cached_lookup(index, &wanted, &paths) {
                    return Ok(Some(found));
                }
                paths
                    .iter()
                    .filter(|path| !index.scanned.contains_key(*path))
                    .cloned()
                    .collect()
            }
            None => paths.clone(),
        }
    };
    let mut entries = Vec::new();
    for path in &to_scan {
        let id = bridge::read_text_file(&app, &folder_id, path)
            .await
            .ok()
            .as_deref()
            .and_then(parse_event_id);
        entries.push((path.clone(), id));
    }
    let mut guard = event_id_index()
        .lock()
        .map_err(|_| "事件索引锁异常".to_string())?;
    let index = guard.entry(folder_id.clone()).or_default();
    index_insert(index, entries);
    Ok(cached_lookup(index, &wanted, &paths))
}

/// 解析并校验事件目录（工作区相对路径）：
/// - 必须是合法相对路径（拒绝 `..` 等逃逸写法）；
/// - 路径中须含 `events` 或 `missionsEvents` 目录段。
///
/// 返回目录**不要求已存在**——保存（含一键创建脚本）时自动建目录；
/// 读取 / 删除时由文件操作自行报告「文件不存在」。
fn events_directory(work_directory: &str, events_dir: &str) -> Result<PathBuf, String> {
    validate_relative_path(events_dir)?;
    let root = PathBuf::from(work_directory);
    if !root.is_dir() {
        return Err(format!("工作目录不存在：{work_directory}"));
    }
    let is_events_layout = events_dir
        .split('/')
        .any(|segment| segment == "events" || segment == "missionsEvents");
    if !is_events_layout {
        return Err(format!("不是有效的事件目录：{events_dir}"));
    }
    Ok(root.join(events_dir))
}

/// 读取事件脚本内容。`events_dir` 为工作区内的事件目录相对路径
/// （如 `assets/game/missions/missionsEvents`、`assets/game/events/common`）。
#[tauri::command]
pub fn load_mission_event(
    work_directory: String,
    events_dir: String,
    file_name: String,
) -> Result<String, String> {
    validate_work_directory_name(&file_name)?;
    let path = events_directory(&work_directory, &events_dir)?.join(&file_name);
    fs::read_to_string(&path)
        .map_err(|error| format!("读取事件脚本失败 {}：{error}", path.display()))
}

/// 保存事件脚本内容（事件目录不存在时自动创建——支持「一键创建决议脚本」）。
#[tauri::command]
pub fn save_mission_event(
    work_directory: String,
    events_dir: String,
    file_name: String,
    contents: String,
) -> Result<(), String> {
    validate_work_directory_name(&file_name)?;
    let directory = events_directory(&work_directory, &events_dir)?;
    fs::create_dir_all(&directory)
        .map_err(|error| format!("创建事件目录失败 {}：{error}", directory.display()))?;
    let path = directory.join(&file_name);
    fs::write(&path, contents)
        .map_err(|error| format!("保存事件脚本失败 {}：{error}", path.display()))?;
    // 内容可能修改了 id：失效该脚本的索引缓存。
    invalidate_event_script_cache(&format!("{}/{}", events_dir.trim_matches('/'), file_name));
    Ok(())
}

/// 删除事件脚本。
#[tauri::command]
pub fn delete_mission_event(
    work_directory: String,
    events_dir: String,
    file_name: String,
) -> Result<(), String> {
    validate_work_directory_name(&file_name)?;
    let path = events_directory(&work_directory, &events_dir)?.join(&file_name);
    fs::remove_file(&path)
        .map_err(|error| format!("删除事件脚本失败 {}：{error}", path.display()))?;
    invalidate_event_script_cache(&format!("{}/{}", events_dir.trim_matches('/'), file_name));
    Ok(())
}

/// 重命名事件脚本（同目录内改名）。
#[tauri::command]
pub fn rename_mission_event(
    work_directory: String,
    events_dir: String,
    old_file_name: String,
    new_file_name: String,
) -> Result<(), String> {
    validate_work_directory_name(&old_file_name)?;
    validate_work_directory_name(&new_file_name)?;
    let directory = events_directory(&work_directory, &events_dir)?;
    let from = directory.join(&old_file_name);
    let to = directory.join(&new_file_name);
    fs::rename(&from, &to)
        .map_err(|error| format!("重命名事件脚本失败 {}：{error}", from.display()))?;
    let dir = events_dir.trim_matches('/');
    invalidate_event_script_cache(&format!("{dir}/{old_file_name}"));
    invalidate_event_script_cache(&format!("{dir}/{new_file_name}"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_directory_accepts_events_layouts_and_rejects_others() {
        let dir = std::env::temp_dir().join(format!("ageciv-events-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("assets/game/missions/missionsEvents")).unwrap();
        fs::create_dir_all(dir.join("assets/game/events/common")).unwrap();
        let work = dir.to_string_lossy().into_owned();

        assert!(events_directory(&work, "assets/game/missions/missionsEvents").is_ok());
        assert!(events_directory(&work, "assets/game/events/common").is_ok());
        // 目录尚不存在也允许（保存时自动创建）。
        assert!(events_directory(&work, "assets/game/events/headless").is_ok());
        // 拒绝逃逸与非事件目录。
        assert!(events_directory(&work, "../outside/events/x").is_err());
        assert!(events_directory(&work, "assets/game/missions").is_err());
        assert!(events_directory(&work, "C:/absolute/events").is_err());

        let _ = fs::remove_dir_all(&dir);
    }

    /// 解析 `id=` 行（容忍空格 / 引号 / BOM；取首个非空值；拒绝 `identifier=` 之类）。
    #[test]
    fn parses_event_id_lines() {
        assert_eq!(
            parse_event_id("id=改任维新派1\ntitle=x\n"),
            Some("改任维新派1".to_string())
        );
        assert_eq!(parse_event_id("id = 测试 值 \n"), Some("测试 值".to_string()));
        assert_eq!(parse_event_id("\u{feff}id=\"带引号\"\n"), Some("带引号".to_string()));
        assert_eq!(parse_event_id("title=x\nid=后面出现\n"), Some("后面出现".to_string()));
        assert_eq!(parse_event_id("identifier=x\n"), None);
        assert_eq!(parse_event_id("id=\n"), None);
        assert_eq!(parse_event_id(""), None);
    }

    /// 按 id 解析事件脚本（决议 `events` 索引的是脚本内部 id）：
    /// id 命中（文件名不同）→ 文件名与 id 一致 → 无 id 文件按名兜底；
    /// 文件名与 id 不一致的文件**不得**按名命中（id 语义优先）；保存 / 删除后缓存失效。
    #[test]
    fn resolves_event_script_by_internal_id() {
        let dir = std::env::temp_dir().join(format!("ageciv-events-id-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let events_dir = "assets/map/Earth3/scenarios/tno/events/common";
        fs::create_dir_all(dir.join(events_dir)).unwrap();
        fs::write(
            dir.join(events_dir).join("chi改任改革派1.txt"),
            "id=改任维新派1\ntitle=x\n",
        )
        .unwrap();
        fs::write(dir.join(events_dir).join("普通事件.txt"), "id=普通事件\n").unwrap();
        fs::write(dir.join(events_dir).join("无id事件.txt"), "title=无id\n").unwrap();
        let work = dir.to_string_lossy().into_owned();
        let mut paths = vec![
            format!("{events_dir}/chi改任改革派1.txt"),
            format!("{events_dir}/普通事件.txt"),
            format!("{events_dir}/无id事件.txt"),
        ];
        paths.sort();

        // id 命中：文件名与 id 不同（用户实测场景）。
        assert_eq!(
            resolve_event_script_by_id(work.clone(), paths.clone(), "改任维新派1".to_string())
                .unwrap()
                .as_deref(),
            Some("assets/map/Earth3/scenarios/tno/events/common/chi改任改革派1.txt")
        );
        // 文件名与 id 一致：正常命中。
        assert_eq!(
            resolve_event_script_by_id(work.clone(), paths.clone(), "普通事件".to_string())
                .unwrap()
                .as_deref(),
            Some("assets/map/Earth3/scenarios/tno/events/common/普通事件.txt")
        );
        // 文件无 id：按文件名兜底命中。
        assert_eq!(
            resolve_event_script_by_id(work.clone(), paths.clone(), "无id事件".to_string())
                .unwrap()
                .as_deref(),
            Some("assets/map/Earth3/scenarios/tno/events/common/无id事件.txt")
        );
        // 文件名与 id 不一致时**不按文件名命中**（如把文件名写进 events 的场景解析不到）。
        assert_eq!(
            resolve_event_script_by_id(work.clone(), paths.clone(), "chi改任改革派1".to_string())
                .unwrap(),
            None
        );
        // 前后空白容忍。
        assert!(resolve_event_script_by_id(work.clone(), paths.clone(), "  普通事件  ".to_string())
            .unwrap()
            .is_some());
        // 不存在 → None。
        assert_eq!(
            resolve_event_script_by_id(work.clone(), paths.clone(), "不存在".to_string()).unwrap(),
            None
        );

        // 保存修改内容（id 变化）后缓存失效：旧 id 不再命中、新 id 命中。
        save_mission_event(
            work.clone(),
            events_dir.to_string(),
            "chi改任改革派1.txt".to_string(),
            "id=改名后\n".to_string(),
        )
        .unwrap();
        assert_eq!(
            resolve_event_script_by_id(work.clone(), paths.clone(), "改任维新派1".to_string())
                .unwrap(),
            None
        );
        assert!(resolve_event_script_by_id(work.clone(), paths.clone(), "改名后".to_string())
            .unwrap()
            .is_some());

        // 删除后缓存失效：不再命中。
        delete_mission_event(
            work.clone(),
            events_dir.to_string(),
            "chi改任改革派1.txt".to_string(),
        )
        .unwrap();
        assert_eq!(
            resolve_event_script_by_id(work.clone(), paths, "改名后".to_string()).unwrap(),
            None,
            "文件已删除，缓存应失效"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    /// 真实模组回归（ignored）：决议 `events` 索引内部 id 的解析覆盖面——
    /// 「历史时代 三」959 个事件脚本中约 299 个文件名与 id 不同，逐个按 id 解析应全部
    /// 返回「内容 id 匹配」的脚本。运行：
    /// `cargo test -p age_civ_mod_tool --lib resolve_real_mod_event_ids -- --ignored --nocapture`
    #[test]
    #[ignore = "需要真实模组目录（A:\\android\\GameCivs）"]
    fn resolve_real_mod_event_ids() {
        let mod_root = std::path::Path::new(
            r"A:\android\GameCivs\历史时代 三 - 虚 革 - 3.4.2 [昔日的暮色]_3.4.2",
        );
        if !mod_root.is_dir() {
            println!("跳过：模组目录不存在");
            return;
        }
        // 收集候选事件脚本（与前端 `event_script_candidates` 同规则）。
        let mut paths: Vec<String> = Vec::new();
        let mut stack = vec![mod_root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let Ok(relative) = path.strip_prefix(mod_root) else {
                    continue;
                };
                let relative = relative.to_string_lossy().replace('\\', "/");
                let lower = relative.to_ascii_lowercase();
                if lower.ends_with(".txt")
                    && (lower.contains("/events/") || lower.contains("missionsevents/"))
                {
                    paths.push(relative);
                }
            }
        }
        paths.sort();
        println!("事件脚本候选：{}", paths.len());

        // 文件名与 id 不同的样本（决议 events 按 id 索引——本次修复的主场景）。
        let mut samples: Vec<(String, String)> = Vec::new();
        for path in &paths {
            let Ok(contents) = fs::read_to_string(mod_root.join(path)) else {
                continue;
            };
            let Some(id) = parse_event_id(&contents) else {
                continue;
            };
            if id != script_stem(path) {
                samples.push((id, path.clone()));
            }
        }
        println!("id 与文件名不同：{}/{}", samples.len(), paths.len());
        assert!(samples.len() > 100, "错位样本过少：{}", samples.len());

        let work = mod_root.to_string_lossy().into_owned();
        let started = std::time::Instant::now();
        let mut misses = 0;
        let mut relocate = 0;
        for (id, expected) in &samples {
            match resolve_event_script_by_id(work.clone(), paths.clone(), id.clone()) {
                Ok(Some(found)) => {
                    let found_id = fs::read_to_string(mod_root.join(&found))
                        .ok()
                        .as_deref()
                        .and_then(parse_event_id);
                    if found_id.as_deref() != Some(id.as_str()) {
                        misses += 1;
                        if misses <= 5 {
                            println!("内容不匹配：{id} → {found}");
                        }
                    } else if &found != expected {
                        relocate += 1; // 同名 id 在其他目录先命中（引擎本就要求 id 唯一）。
                    }
                }
                other => {
                    misses += 1;
                    if misses <= 5 {
                        println!("未命中：{id} → {other:?}（应为 {expected}）");
                    }
                }
            }
        }
        println!(
            "按 id 解析：{}/{} 命中（其中 {} 个同名 id 命中其他目录），耗时 {:?}",
            samples.len() - misses,
            samples.len(),
            relocate,
            started.elapsed()
        );
        assert_eq!(misses, 0, "存在未按 id 解析到脚本的样本");
    }
}
