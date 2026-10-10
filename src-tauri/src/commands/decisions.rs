//! 决议定义文件（`rainfall/rfEvent_decision.json`）的读写命令。
//!
//! 决议文件由 rfEvent 插件在运行时从 `assets/rainfall/` 读取，全项目仅此一份，
//! 内容为决议组列表（`decisions` 数组）。本模块的命令与国策系列（`missions.rs`）
//! 对称：`parse_*` / `serialize_*` 为纯函数（Android SAF 通道由前端负责写回纠正
//! 内容），`load_*_file` / `save_*_file` 走工作区相对路径（真实路径模式）。
//!
//! 注意：本模块的 [`parse_decisions`] 是完整结构化解析（含 `desc` / `images` /
//! `events` / 未知字段），与 `missions_db` 中供补全用的行扫描版 `parse_decisions`
//! （仅提取 id / name / events）用途不同，勿混用。

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use rayon::prelude::*;
use serde::Serialize;

use crate::decision_format::{parse_decision_file_with_correction, serialize_decision_file};
use crate::models::{DecisionGroup, FocusIcon};
use crate::paths::workspace_abs_path;

/// 决议定义文件的工作区相对路径尾段（位于模组 `assets/` 级目录下）。
const DECISION_FILE_RELATIVE: &str = "rainfall/rfEvent_decision.json";

/// [`parse_decisions`] 的返回：解析出的决议组；语法被自动纠正时附带规范化后的
/// 完整内容，由前端（Android scoped 通道）负责写回原文件。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedDecisions {
    decisions: Vec<DecisionGroup>,
    corrected_contents: Option<String>,
}

/// 解析决议定义文本（宽松语法会自动纠正，并返回规范化内容供调用方写回）。
#[tauri::command]
pub fn parse_decisions(contents: String) -> Result<ParsedDecisions, String> {
    let (file, corrected) = parse_decision_file_with_correction(&contents)?;
    Ok(ParsedDecisions {
        decisions: file.decisions,
        corrected_contents: corrected,
    })
}

/// 序列化决议组列表（规范格式；未知字段原样保留）。
#[tauri::command]
pub fn serialize_decisions(decisions: Vec<DecisionGroup>) -> Result<String, String> {
    serialize_decision_file(&decisions)
}

/// 按工作区相对路径读取并解析决议文件；若是宽松语法，自动纠正并把规范内容写回原文件。
#[tauri::command]
pub fn load_decisions_file(
    work_directory: String,
    relative_path: String,
) -> Result<Vec<DecisionGroup>, String> {
    let path = workspace_abs_path(&work_directory, &relative_path)?;
    let content = fs::read_to_string(&path)
        .map_err(|error| format!("读取决议文件失败 {}：{error}", path.display()))?;
    let (file, corrected) = parse_decision_file_with_correction(&content)?;
    if let Some(corrected) = corrected {
        fs::write(&path, corrected).map_err(|error| {
            format!(
                "决议文件语法已自动纠正，但写回失败 {}：{error}",
                path.display()
            )
        })?;
    }
    Ok(file.decisions)
}

/// 保存决议组列表到工作区相对路径（目录不存在时自动创建——「按需创建」流程）。
#[tauri::command]
pub fn save_decisions_file(
    work_directory: String,
    relative_path: String,
    decisions: Vec<DecisionGroup>,
) -> Result<(), String> {
    let path = workspace_abs_path(&work_directory, &relative_path)?;
    let content = serialize_decision_file(&decisions)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建目录失败 {}：{error}", parent.display()))?;
    }
    fs::write(&path, content)
        .map_err(|error| format!("保存决议文件失败 {}：{error}", path.display()))
}

/// 探测工作区内决议定义文件的位置，返回工作区相对路径（找不到时返回 `None`，
/// 由前端提示「按需创建」）。
///
/// 候选（按优先级）：
/// 1. `rainfall/rfEvent_decision.json`（工作区根 = 模组 `assets/` 级目录）；
/// 2. `assets/rainfall/rfEvent_decision.json`（工作区根 = 模组 / 工程根）；
/// 3. `<一级子目录>/rainfall/…` 与 `<一级子目录>/assets/rainfall/…`
///    （解包到 `<工作区>/<APK名>/` 的布局；子目录按名称排序保证确定性）。
#[tauri::command]
pub fn resolve_decision_file(work_directory: String) -> Result<Option<String>, String> {
    let root = Path::new(&work_directory);
    if !root.is_dir() {
        return Err(format!("工作目录不存在：{work_directory}"));
    }
    let mut candidates: Vec<String> = vec![
        DECISION_FILE_RELATIVE.to_string(),
        format!("assets/{DECISION_FILE_RELATIVE}"),
    ];
    let mut subdirs: Vec<String> = Vec::new();
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    subdirs.push(name.to_string());
                }
            }
        }
    }
    subdirs.sort();
    for name in subdirs {
        candidates.push(format!("{name}/{DECISION_FILE_RELATIVE}"));
        candidates.push(format!("{name}/assets/{DECISION_FILE_RELATIVE}"));
    }
    let mut seen = HashSet::new();
    for candidate in candidates {
        if !seen.insert(candidate.clone()) {
            continue;
        }
        if root.join(&candidate).is_file() {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

/// 决议图片目录（工作区相对）：`<前缀>rainfall/rfEvent_decision.json` → `<前缀>gfx/decision`。
/// 前缀与决议文件一致，覆盖工作区根 / `assets/` / `<包名>/assets/` 三种布局。
fn decision_image_relative_dir(relative_path: &str) -> Option<String> {
    let lower = relative_path.to_ascii_lowercase();
    let index = if lower.starts_with("rainfall/") {
        0
    } else if let Some(found) = lower.rfind("/rainfall/") {
        found + 1
    } else {
        return None;
    };
    Some(format!("{}gfx/decision", &relative_path[..index]))
}

/// 决议图片尺寸探测项（[`probe_decision_images`] 返回；只读文件头部，不传输正文）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionImageSize {
    pub name: String,
    pub width: u32,
    pub height: u32,
}

/// 解析 PNG 宽高（仅需前 24 字节：签名 + IHDR 长度/类型 + 宽 + 高；非 PNG 返回 None）。
fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    if bytes.len() < 24 || bytes[..8] != SIGNATURE || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((width, height))
}

/// 决议图片文件定位（单段文件名；前端按名称主干传名 → 原名未命中且不含 `.`
/// 时回退 `<名称>.png`，与 SAF 通道按主干匹配 `<主干>.png` 的行为一致）。
fn resolve_decision_image_path(directory: &Path, name: &str) -> Option<std::path::PathBuf> {
    let single_segment =
        Path::new(name).file_name().and_then(|part| part.to_str()) == Some(name);
    if !single_segment {
        return None;
    }
    let direct = directory.join(name);
    if direct.is_file() {
        return Some(direct);
    }
    if !name.contains('.') {
        let fallback = directory.join(format!("{name}.png"));
        if fallback.is_file() {
            return Some(fallback);
        }
    }
    None
}

/// 决议图片目录解析（绝对路径；两命令共用）。
fn decision_image_directory(
    work_directory: &str,
    relative_path: &str,
) -> Result<std::path::PathBuf, String> {
    let relative_dir = decision_image_relative_dir(relative_path)
        .ok_or_else(|| "决议文件路径无效，无法推导图片目录".to_string())?;
    let directory = workspace_abs_path(work_directory, &relative_dir)?;
    if !directory.is_dir() {
        return Err(format!("未找到决议图片目录：{relative_dir}"));
    }
    Ok(directory)
}

/// 批量加载决议图片（`gfx/decision/` 下；名称可含扩展名、也可为主干——主干自动补
/// `.png`；缺失或非法项静默跳过，前端保留占位符）。与 `icons.rs::load_mission_icons` 同风格。
#[tauri::command]
pub fn load_decision_images(
    work_directory: String,
    relative_path: String,
    names: Vec<String>,
) -> Result<Vec<FocusIcon>, String> {
    let directory = decision_image_directory(&work_directory, &relative_path)?;
    let images = names
        .into_par_iter()
        .filter_map(|name| {
            let path = resolve_decision_image_path(&directory, &name)?;
            let bytes = fs::read(path).ok()?;
            Some(FocusIcon {
                name,
                data_url: format!("data:image/png;base64,{}", STANDARD.encode(bytes)),
            })
        })
        .collect();
    Ok(images)
}

/// 只读 PNG 头部探测决议图片尺寸（前端在批量加载前先探测一遍，用 `aspect-ratio`
/// 为图片/占位符预留高度——加载完成与滚动期间都不再改变行高，消除列表跳动）。
#[tauri::command]
pub fn probe_decision_images(
    work_directory: String,
    relative_path: String,
    names: Vec<String>,
) -> Result<Vec<DecisionImageSize>, String> {
    let directory = decision_image_directory(&work_directory, &relative_path)?;
    let sizes = names
        .into_par_iter()
        .filter_map(|name| {
            use std::io::Read as _;
            let path = resolve_decision_image_path(&directory, &name)?;
            let mut file = fs::File::open(path).ok()?;
            let mut header = [0_u8; 24];
            file.read_exact(&mut header).ok()?;
            let (width, height) = png_dimensions(&header)?;
            Some(DecisionImageSize { name, width, height })
        })
        .collect();
    Ok(sizes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ageciv-decisions-{tag}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_group(id: &str, name: &str) -> DecisionGroup {
        DecisionGroup {
            id: id.to_string(),
            name: name.to_string(),
            desc: vec![format!("{name}的描述")],
            images: vec!["none.png".to_string()],
            events: vec![format!("决议:{id}")],
            extra: std::collections::BTreeMap::new(),
        }
    }

    /// 保存时自动建目录；读取往返一致。
    #[test]
    fn saves_creates_directories_and_loads_round_trip() {
        let dir = temp_dir("round");
        let work = dir.to_string_lossy().into_owned();
        let decisions = vec![sample_group("A", "甲"), sample_group("B", "乙")];
        save_decisions_file(
            work.clone(),
            DECISION_FILE_RELATIVE.to_string(),
            decisions,
        )
        .unwrap();
        assert!(dir.join(DECISION_FILE_RELATIVE).is_file());
        let loaded =
            load_decisions_file(work, DECISION_FILE_RELATIVE.to_string()).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].id, "A");
        assert_eq!(loaded[1].events, vec!["决议:B".to_string()]);
        let _ = fs::remove_dir_all(&dir);
    }

    /// 定位器覆盖三种常见工作区布局，且无文件时返回 None。
    #[test]
    fn resolves_decision_file_in_common_layouts() {
        // 1) 工作区根 = 模组 assets 级：rainfall/…
        let assets_root = temp_dir("resolve-assets");
        let work = assets_root.to_string_lossy().into_owned();
        assert_eq!(resolve_decision_file(work.clone()).unwrap(), None);
        fs::create_dir_all(assets_root.join("rainfall")).unwrap();
        fs::write(assets_root.join(DECISION_FILE_RELATIVE), "{}").unwrap();
        assert_eq!(
            resolve_decision_file(work).unwrap().as_deref(),
            Some(DECISION_FILE_RELATIVE)
        );

        // 2) 工作区根 = 模组 / 工程根：assets/rainfall/…
        let project_root = temp_dir("resolve-project");
        let work = project_root.to_string_lossy().into_owned();
        fs::create_dir_all(project_root.join("assets/rainfall")).unwrap();
        fs::write(project_root.join("assets").join(DECISION_FILE_RELATIVE), "{}").unwrap();
        assert_eq!(
            resolve_decision_file(work).unwrap().as_deref(),
            Some("assets/rainfall/rfEvent_decision.json")
        );

        // 3) 解包 APK 布局：<包名>/assets/rainfall/…
        let unpacked_root = temp_dir("resolve-unpacked");
        let work = unpacked_root.to_string_lossy().into_owned();
        fs::create_dir_all(unpacked_root.join("MyMod/assets/rainfall")).unwrap();
        fs::write(
            unpacked_root.join("MyMod/assets").join(DECISION_FILE_RELATIVE),
            "{}",
        )
        .unwrap();
        assert_eq!(
            resolve_decision_file(work).unwrap().as_deref(),
            Some("MyMod/assets/rainfall/rfEvent_decision.json")
        );

        let _ = fs::remove_dir_all(&assets_root);
        let _ = fs::remove_dir_all(&project_root);
        let _ = fs::remove_dir_all(&unpacked_root);
    }

    /// 决议图片目录推导（三种布局）与批量读取（缺失/非法名静默跳过）。
    #[test]
    fn loads_decision_images_and_derives_directory() {
        assert_eq!(
            decision_image_relative_dir("rainfall/rfEvent_decision.json").as_deref(),
            Some("gfx/decision")
        );
        assert_eq!(
            decision_image_relative_dir("assets/rainfall/rfEvent_decision.json").as_deref(),
            Some("assets/gfx/decision")
        );
        assert_eq!(
            decision_image_relative_dir("MyMod/assets/Rainfall/RFEvent_Decision.JSON").as_deref(),
            Some("MyMod/assets/gfx/decision")
        );
        assert_eq!(decision_image_relative_dir("assets/game/missions/x.json"), None);

        let dir = temp_dir("images");
        let work = dir.to_string_lossy().into_owned();
        let gfx = dir.join("assets/gfx/decision");
        fs::create_dir_all(&gfx).unwrap();
        fs::write(gfx.join("SC.png"), b"\x89PNG-fake").unwrap();
        let images = load_decision_images(
            work.clone(),
            "assets/rainfall/rfEvent_decision.json".to_string(),
            vec![
                "SC.png".to_string(),
                "SC".to_string(),
                "缺失.png".to_string(),
                "..\\..\\evil.png".to_string(),
            ],
        )
        .unwrap();
        // 含扩展名与主干两种写法都能命中（主干回退 `<名称>.png`）。
        assert_eq!(images.len(), 2);
        assert!(images.iter().any(|icon| icon.name == "SC.png"));
        assert!(images.iter().any(|icon| icon.name == "SC"));
        assert!(
            images
                .iter()
                .all(|icon| icon.data_url.starts_with("data:image/png;base64,"))
        );

        // 目录不存在时报错（前端保留占位符并提示）。
        assert!(
            load_decision_images(work, "rainfall/rfEvent_decision.json".to_string(), vec![]).is_err()
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// 24 字节最小 PNG 头部（签名 + IHDR 长度/类型 + 宽高），供尺寸解析测试使用。
    fn png_header(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13];
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes
    }

    /// PNG 头部尺寸解析（签名/长度/类型校验；非 PNG 与短数据返回 None）。
    #[test]
    fn parses_png_dimensions_from_header() {
        let header = png_header(32, 24);
        assert_eq!(png_dimensions(&header), Some((32, 24)));
        assert_eq!(png_dimensions(b"\x89PNG-fake"), None);
        assert_eq!(png_dimensions(&header[..20]), None);
    }

    /// 尺寸探测：命中文件返回宽高；缺失/非法名静默跳过；非 PNG 头部同样跳过。
    #[test]
    fn probes_image_sizes_and_skips_missing() {
        let dir = temp_dir("probe-images");
        let work = dir.to_string_lossy().into_owned();
        let gfx = dir.join("assets/gfx/decision");
        fs::create_dir_all(&gfx).unwrap();
        fs::write(gfx.join("banner.png"), png_header(64, 48)).unwrap();
        fs::write(gfx.join("broken.png"), b"not-a-png").unwrap();
        let sizes = probe_decision_images(
            work,
            "assets/rainfall/rfEvent_decision.json".to_string(),
            vec![
                "banner".to_string(),
                "broken".to_string(),
                "missing".to_string(),
            ],
        )
        .unwrap();
        assert_eq!(sizes.len(), 1);
        assert_eq!(sizes[0].name, "banner");
        assert_eq!((sizes[0].width, sizes[0].height), (64, 48));
        let _ = fs::remove_dir_all(&dir);
    }

    /// 宽松语法打开时自动纠正并写回（再次解析不再触发纠正）。
    #[test]
    fn loads_and_autofixes_loose_file() {
        let dir = temp_dir("autofix");
        fs::create_dir_all(dir.join("rainfall")).unwrap();
        let loose = "{\n\t\"decisions\":[\n\t{\n\t\t\"id\":\"A\",\n\t\t\"name\":\"甲\"\n\t}\n\t{\n\t\t\"id\":\"B\",\n\t\t\"name\":\"乙\",\n\t}\n\t]\n}";
        fs::write(dir.join(DECISION_FILE_RELATIVE), loose).unwrap();
        let work = dir.to_string_lossy().into_owned();
        let loaded = load_decisions_file(work.clone(), DECISION_FILE_RELATIVE.to_string()).unwrap();
        assert_eq!(loaded.len(), 2);

        let written = fs::read_to_string(dir.join(DECISION_FILE_RELATIVE)).unwrap();
        let (_, corrected) = parse_decision_file_with_correction(&written).unwrap();
        assert!(corrected.is_none(), "纠正写回后不应再次触发纠正");
        let _ = fs::remove_dir_all(&dir);
    }

    /// 真实模组数据：GameCivs 下各模组的 `rainfall/rfEvent_decision.json` 全解析、
    /// 序列化往返稳定（暮色黄昏为版块导入工作区、无 rainfall，自动跳过）。
    /// `cargo test -p age_civ_mod_tool --lib real_gamecivs_decision_files -- --ignored --nocapture`
    #[test]
    #[ignore = "需要真实数据 A:\\android\\GameCivs"]
    fn real_gamecivs_decision_files_round_trip() {
        let base = std::path::Path::new(r"A:\android\GameCivs");
        let mods = [
            "europe",
            "1566AuroraPrever2",
            "白日升",
            "暮色黄昏_世界大战0.25.1",
            "历史时代 三 - 虚 革 - 3.4.2 [昔日的暮色]_3.4.2",
        ];
        let mut checked = 0;
        for name in mods {
            let path = base.join(name).join("assets").join(DECISION_FILE_RELATIVE);
            if !path.is_file() {
                println!("SKIP {name}（无 {DECISION_FILE_RELATIVE}）");
                continue;
            }
            let content = std::fs::read_to_string(&path).unwrap();
            let (file, _) = parse_decision_file_with_correction(&content).unwrap();
            assert!(!file.decisions.is_empty(), "{name} 决议组为空");
            let serialized = serialize_decision_file(&file.decisions).unwrap();
            let (again, corrected) = parse_decision_file_with_correction(&serialized).unwrap();
            assert!(corrected.is_none(), "{name} 规范输出不应触发纠正");
            assert_eq!(again.decisions.len(), file.decisions.len(), "{name} 往返组数不一致");
            println!(
                "OK {name} groups={} first={} events={}",
                file.decisions.len(),
                file.decisions[0].id,
                file.decisions[0].events.len()
            );
            checked += 1;
        }
        assert!(checked >= 3, "至少应检查到 3 个模组的决议文件（实际 {checked}）");

        // 样本断言：europe 的「政治决策」组（RUS）包含已知条目。
        let europe = std::fs::read_to_string(
            base.join("europe").join("assets").join(DECISION_FILE_RELATIVE),
        )
        .unwrap();
        let (file, _) = parse_decision_file_with_correction(&europe).unwrap();
        let rus = file
            .decisions
            .iter()
            .find(|group| group.id == "RUS")
            .expect("europe 应含 RUS 组");
        assert!(
            rus.events.iter().any(|event| event.contains("清洗")),
            "RUS 组应含「清洗」条目，实际：{:?}",
            rus.events
        );
    }

    /// 工具用例：把虚革模组的决议文件「纠正 + 规范化」导出到 `tmp\`，
    /// 供浏览器 E2E mock / 外部工具直接读取（原文件含缺失逗号等宽松写法，无法直接 JSON 解析）。
    /// `cargo test -p age_civ_mod_tool --lib dump_xuge_decision -- --ignored`
    #[test]
    #[ignore = "工具用例：需要真实数据并写入 A:\\android\\AgeCivModTool\\tmp"]
    fn dump_xuge_decision_corrected() {
        let path = std::path::Path::new(
            r"A:\android\GameCivs\历史时代 三 - 虚 革 - 3.4.2 [昔日的暮色]_3.4.2\assets\rainfall\rfEvent_decision.json",
        );
        let content = std::fs::read_to_string(path).unwrap();
        let (file, _) = parse_decision_file_with_correction(&content).unwrap();
        let serialized = serialize_decision_file(&file.decisions).unwrap();
        let out_dir = std::path::Path::new(r"A:\android\AgeCivModTool\tmp");
        let _ = std::fs::create_dir_all(out_dir);
        std::fs::write(out_dir.join("xuge-decision-corrected.json"), serialized).unwrap();
        println!("dumped groups={}", file.decisions.len());
    }
}
