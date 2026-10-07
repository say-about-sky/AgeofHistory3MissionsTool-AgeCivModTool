//! 文明 / 政体数据解析与「事件编辑器对照表」。
//!
//! 解析核心移植自 `ScvGen` 库（`a:\android\missions_db\ScvGen\src\lib.rs`）：
//! - `load_translations` / `generate_civ`：`Civilizations.txt` + `civilizations/*.json` + 翻译
//!   `.properties` → `ID,Name,Tag` 记录（ID 即 tag 在清单中的顺序）；
//! - `generate_gov`：逐行扫描 `Governments.json` 的 `Name:` / `Extra_Tag:`，
//!   生成 `Index,Name,Tag` 记录（Index 从 0 递增，即游戏内使用的「政体整数」）。
//!
//! 在移植基础上新增 [`EventLookup`]（事件编辑器对照表），除「文明ID / tag / civ」与
//! 「政体整数」外，还提供以下补全数据（含 Name 提示）：
//! - 省份 ID ↔ 地名：地图由 `assets/map/Maps.json` 发现（可有多个地图文件夹），
//!   省份取自 `<地图>/data/ProvinceDetails.json`，名称取 `<地图>/cities/cities.json`
//!   （首个地名优先）；剧本根（`assets/map/<地图>/scenarios/<剧本>/missions`）
//!   按其所属地图加载；剧本清单由 `<地图>/Scenarios.txt` 校验/反查；
//! - 建筑 ID ↔ 名称：`buildings/Buildings.json` 数组顺序即 ID；
//! - 疾病 ID ↔ 名称：`diseases/Diseases.json` 数组顺序即 ID，名称经根语言表翻译；
//! - 人物名称：`characters/**.json` 的 `Name` 与文件名（去除 `.json`）都会收录。
//!
//! 相对上游的差异（为编辑器可用性）：
//! - 名称解析优先级相同（翻译[文件名 tag] → 翻译[JSON Tag] → JSON Name），
//!   仅将末位兜底从「空字符串」改为 tag 本身；
//! - 额外收录只出现在翻译表中的 tag（如 `afri_c` 这类运行时标签/其它地图文明），
//!   排在清单条目之后（按字典序）；
//! - 只为缺失翻译的 tag 读取对应 json（上游全量读取）。

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde::Serialize;

use crate::android_fs_bridge as bridge;
use crate::apk_extract::{list_apk_entries, read_apk_entry_bytes, ScanEntry};
use crate::apk_pack::SOURCE_APK_MARKER;

/// tag 顺序清单（分号分隔）。
const CIV_TAGS_FILE: &str = "Civilizations.txt";
/// 政体定义文件（JSON 风格，含 `Name:` / `Extra_Tag:` 行）。
const GOV_FILE: &str = "Governments.json";
/// 各文明详细定义目录（`{tag}.json`）。
const CIV_DIR: &str = "civilizations";
/// 文明翻译（默认语言，英文名兜底）。
const CIV_BUNDLE: &str = "languages/civilizations/Bundle.properties";
/// 文明翻译（简体中文，优先）。
const CIV_BUNDLE_CN: &str = "languages/civilizations/Bundle_cn_sp.properties";
/// 地图索引（位于 assets 根下；条目行为 `Folder:`）。
const MAPS_FILE: &str = "map/Maps.json";
/// 每张地图的剧本清单（分号分隔的剧本目录名）。
const SCENARIOS_FILE: &str = "Scenarios.txt";
/// 地图省份详情（单行 JSON，逐条含 `pid:`）。
const PROVINCE_DETAILS_FILE: &str = "data/ProvinceDetails.json";
/// 地图城市表（含 `Name:` / `p:` 行，`p` 为省份 ID）。
const CITIES_FILE: &str = "cities/cities.json";
/// 建筑定义（数组顺序即建筑 ID）。
const BUILDINGS_FILE: &str = "buildings/Buildings.json";
/// 疾病定义（数组顺序即疾病 ID）。
const DISEASES_FILE: &str = "diseases/Diseases.json";
/// 人物定义目录（`*.json` 含 `Name:`）。
const CHARACTERS_DIR: &str = "characters";
/// 根语言表（疾病等名称翻译，简体中文优先）。
const ROOT_BUNDLE_CN: &str = "languages/Bundle_cn_sp.properties";
/// 根语言表（默认语言兜底）。
const ROOT_BUNDLE: &str = "languages/Bundle.properties";

// ===== 移植：ScvGen lib.rs 基础解析 =====

/// 宽松 JSON 解析出的文明记录（字段与 ScvGen `Civ` 保持一致；
/// 当前消费方只使用 `tag` / `name`，其余字段完整保留以便后续扩展）。
#[derive(Debug, Default, Clone)]
#[allow(dead_code)]
pub struct Civ {
    pub group_id: i32,
    pub name: String,
    pub religion_id: i32,
    pub tag: String,
    pub wiki: Option<String>,
    pub ib: i32,
    pub ig: i32,
    pub ir: i32,
}

/// 读取并解析 `.properties` 翻译文件（移植自 `load_translations`；由 [`generate_civ`] 使用）。
///
/// - 忽略空行与 `#` / `!` 开头的注释；
/// - 以第一个 `=` 分隔键值（键值两侧空白会被去除）；
/// - 同一 key 重复出现时，后者覆盖前者。
#[allow(dead_code)]
pub fn load_translations(path: &Path) -> Result<HashMap<String, String>, String> {
    let content = fs::read_to_string(path)
        .map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;
    Ok(parse_translations(&content))
}

/// [`load_translations`] 的纯字符串版本（SAF 模式与测试使用）。
pub fn parse_translations(content: &str) -> HashMap<String, String> {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let mut map: HashMap<String, String> = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let key = k.trim();
        if key.is_empty() {
            continue;
        }
        map.insert(key.to_string(), v.trim().to_string());
    }
    map
}

/// 宽松 JSON 解析器（移植自 `parse_loose_json`）：
/// - key / value 允许不带引号；
/// - 允许字段缺失；
/// - 允许值里出现冒号（只在第一个 `:` 分割）；
/// - 允许字符串里出现逗号（跟踪引号状态）。
pub fn parse_loose_json(text: &str) -> Result<Civ, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let inner = text
        .trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .trim();

    // ---------- 分割顶层字段 ----------
    let mut fields: HashMap<String, String> = HashMap::new();
    let mut cur = String::new();
    let mut in_string = false;
    let mut parts: Vec<String> = Vec::new();

    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                if in_string && chars.peek() == Some(&'"') {
                    cur.push('"');
                    cur.push('"');
                    chars.next();
                } else {
                    in_string = !in_string;
                    cur.push(c);
                }
            }
            ',' if !in_string => {
                parts.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        parts.push(cur.trim().to_string());
    }

    // ---------- 逐个字段拆 key / value ----------
    for part in parts {
        if part.is_empty() {
            continue;
        }
        let (k, v) = part
            .split_once(':')
            .ok_or_else(|| format!("字段格式不合法: `{part}`"))?;
        let key = k.trim().trim_matches('"').trim().to_string();
        let value = v.trim().to_string();
        if !key.is_empty() {
            fields.insert(key, value);
        }
    }

    // ---------- 提取工具 ----------
    let get_str = |name: &str| -> Option<String> {
        fields.get(name).and_then(|v| {
            let v = v.trim();
            let unquoted = if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
                &v[1..v.len() - 1]
            } else {
                v
            };
            if unquoted.eq_ignore_ascii_case("null") {
                None
            } else {
                Some(unquoted.to_string())
            }
        })
    };

    let get_i32 = |name: &str| -> i32 {
        fields
            .get(name)
            .map(|v| v.trim().trim_matches('"'))
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(0)
    };

    // ---------- 组装 ----------
    Ok(Civ {
        group_id: get_i32("GroupID"),
        name: get_str("Name").unwrap_or_default(),
        religion_id: get_i32("ReligionID"),
        tag: get_str("Tag").unwrap_or_default(),
        wiki: get_str("Wiki"),
        ib: get_i32("iB"),
        ig: get_i32("iG"),
        ir: get_i32("iR"),
    })
}

/// 提取形如 `: "xxx",` / `: xxx,` 后面的值（移植自 `parse_value`，用于政体行扫描）。
fn parse_field_value(rest: &str) -> String {
    let mut value = rest.trim();

    // 去掉行尾逗号
    if value.ends_with(',') {
        value = value[..value.len() - 1].trim();
    }

    // 去掉首尾引号，并简单处理转义
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        let first = bytes[0] as char;
        let last = bytes[value.len() - 1] as char;

        if (first == '"' && last == '"') || (first == '\'' && last == '\'') {
            let inner = &value[1..value.len() - 1];
            return inner.replace("\\\"", "\"").replace("\\\\", "\\");
        }
    }

    value.to_string()
}

/// CSV 字段转义（移植自 `csv_escape`；由 [`generate_civ`] / [`generate_gov`] 使用）。
#[allow(dead_code)]
fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// 分号分隔清单（`Civilizations.txt` 与 `Scenarios.txt` 同构，保留顺序、忽略空段）。
fn split_semicolon_list(raw: &str) -> Vec<String> {
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    raw.split(';')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .collect()
}

/// 解析 `Civilizations.txt`：分号分隔的 tag 清单（保留顺序，忽略空段）。
pub fn parse_civilization_tags(raw: &str) -> Vec<String> {
    split_semicolon_list(raw)
}

/// 解析 `<地图>/Scenarios.txt`：分号分隔的剧本目录名清单。
pub fn parse_scenario_list(raw: &str) -> Vec<String> {
    split_semicolon_list(raw)
}

/// 政体条目：`index` 即游戏内使用的「政体整数」，`tag` 为 `Extra_Tag`。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GovernmentEntry {
    pub index: usize,
    pub name: String,
    pub tag: String,
}

/// 解析 `Governments.json`（移植自 `generate_gov_scv_bytes` 的逐行扫描）：
/// 每遇到一个 `Extra_Tag:` 生成一条记录，`index` 从 0 开始递增；
/// 只有 `Name:` 而没有后续 `Extra_Tag:` 的条目会被下一条 `Name:` 覆盖（与上游一致）。
pub fn parse_governments(text: &str) -> Vec<GovernmentEntry> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut entries: Vec<GovernmentEntry> = Vec::new();
    let mut current_name: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Name:") {
            current_name = Some(parse_field_value(rest));
        } else if let Some(rest) = line.strip_prefix("Extra_Tag:") {
            if let Some(name) = current_name.take() {
                entries.push(GovernmentEntry {
                    index: entries.len(),
                    name,
                    tag: parse_field_value(rest),
                });
            }
        }
    }
    entries
}

/// 政体 CSV 文本（移植 `generate_gov_scv_bytes` 的表头与列序：`Index,Name,Tag`）。
#[allow(dead_code)]
pub fn governments_csv(entries: &[GovernmentEntry]) -> String {
    let mut output = String::from("Index,Name,Tag\n");
    for entry in entries {
        output.push_str(&format!(
            "{},{},{}\n",
            entry.index,
            csv_escape(&entry.name),
            csv_escape(&entry.tag)
        ));
    }
    output
}

/// 生成政体 CSV 文件（移植 `generate_gov`），返回控制台消息列表。
///
/// 当前应用内暂无调用方（对照表加载复用 [`parse_governments`]），保留移植 API 供后续
/// 「导出 Governments.csv」类功能使用。
#[allow(dead_code)]
pub fn generate_gov(input_path: &Path, output_path: &Path) -> Result<Vec<String>, String> {
    let text = fs::read_to_string(input_path)
        .map_err(|e| format!("读取 {} 失败: {e}", input_path.display()))?;
    let entries = parse_governments(&text);
    fs::write(output_path, governments_csv(&entries))
        .map_err(|e| format!("写入 {} 失败: {e}", output_path.display()))?;
    Ok(vec![format!("共 {} 条记录", entries.len())])
}

/// 生成文明 CSV 文件（移植 `generate_civ`）：
/// - `civ_txt_path`：`Civilizations.txt`（分号 tag 清单）；
/// - `civ_dir`：`civilizations/` 目录（`{tag}.json`）；
/// - `lang_path`：翻译 `.properties`；
/// - 输出列与上游一致：`ID,Name,Tag`，ID 为 tag 在清单中的顺序（缺失 json 的 tag 会留空号）。
///
/// 当前应用内暂无调用方（对照表加载复用 [`build_civ_entries`]），保留移植 API 供后续
/// 「导出 Civilizations.csv」类功能使用。
#[allow(dead_code)]
pub fn generate_civ(
    civ_txt_path: &Path,
    civ_dir: &Path,
    out_path: &Path,
    lang_path: &Path,
) -> Result<Vec<String>, String> {
    let mut print_list: Vec<String> = Vec::new();
    let translations = match load_translations(lang_path) {
        Ok(map) => {
            print_list.push(format!(
                "已加载 {} 条翻译({})",
                map.len(),
                lang_path.display()
            ));
            map
        }
        Err(e) => {
            print_list.push(format!("警告：加载翻译文件失败（将不翻译）：{e}"));
            HashMap::new()
        }
    };

    let raw = fs::read_to_string(civ_txt_path)
        .map_err(|e| format!("读取 {} 失败: {e}", civ_txt_path.display()))?;
    let tags = parse_civilization_tags(&raw);
    print_list.push(format!("Civilizations.txt 中共 {} 个 Tag", tags.len()));

    let mut csv_output = String::from("ID,Name,Tag\n");
    let mut written: usize = 0;
    let mut translated: usize = 0;
    let mut untranslated: usize = 0;
    let mut missing: usize = 0;
    let mut failed: usize = 0;

    for (order, tag) in tags.iter().enumerate() {
        let json_path: PathBuf = civ_dir.join(format!("{tag}.json"));
        let content = match fs::read_to_string(&json_path) {
            Ok(content) => content,
            Err(_) => {
                missing += 1;
                continue;
            }
        };
        let civ = match parse_loose_json(&content) {
            Ok(civ) => civ,
            Err(_) => {
                failed += 1;
                continue;
            }
        };

        // Tag 缺失时用文件名兜底
        let final_tag = if civ.tag.is_empty() {
            (*tag).clone()
        } else {
            civ.tag.clone()
        };

        // 先按文件名 tag 查，其次用 JSON 里的 Tag 查，都没有就回退 JSON Name / tag
        let is_translated =
            translations.contains_key(tag) || translations.contains_key(&final_tag);
        let display_name = civ_display_name(tag, Some(&civ), &translations);
        if is_translated {
            translated += 1;
        } else {
            untranslated += 1;
        }

        csv_output.push_str(&format!(
            "{},{},{}\n",
            order,
            csv_escape(&display_name),
            csv_escape(&final_tag)
        ));
        written += 1;
    }

    print_list.push(format!(
        "共写入 {} 条记录，其中 {} 条使用了翻译，{} 条未找到翻译",
        written, translated, untranslated
    ));
    if missing > 0 {
        print_list.push(format!(
            "警告：{missing} 个 Tag 在 civilizations/ 下找不到对应 json（已跳过）"
        ));
    }
    if failed > 0 {
        print_list.push(format!(
            "警告：{failed} 个 json 解析失败（已跳过）"
        ));
    }

    fs::write(out_path, csv_output)
        .map_err(|e| format!("写入 {} 失败: {e}", out_path.display()))?;
    Ok(print_list)
}

// ===== 扩展对照数据：地图 / 省份 / 建筑 / 疾病 / 人物 =====

/// 省份条目（`id` 为游戏省份 ID；`name` 取地图城市表首个地名，可能为空）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProvinceEntry {
    pub id: u32,
    pub name: String,
}

/// 建筑条目（`id` 为 `Buildings.json` 数组顺序；名称取各级拼接）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BuildingEntry {
    pub id: u32,
    pub name: String,
}

/// 疾病条目（`id` 为 `Diseases.json` 数组顺序；名称为中译优先、原名兜底）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DiseaseEntry {
    pub id: u32,
    pub name: String,
}

/// 解析 `map/Maps.json`：逐行扫描 `Folder:`（与政体扫描同为移植风格的宽松解析）。
pub fn parse_map_folders(text: &str) -> Vec<String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut folders = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Folder:") {
            let folder = parse_field_value(rest);
            if !folder.is_empty() {
                folders.push(folder);
            }
        }
    }
    folders
}

/// 宽松扫描 `key:` 之后的全部「字符串或字符串数组」值（保持出现顺序）。
fn scan_key_values(text: &str, key: &str) -> Vec<Vec<String>> {
    let mut values: Vec<Vec<String>> = Vec::new();
    let mut cursor = 0;
    while let Some(position) = text[cursor..].find(key) {
        let start = cursor + position + key.len();
        cursor = start;
        let rest = text[start..].trim_start();
        let Some(rest) = rest.strip_prefix(':') else {
            continue;
        };
        let parsed = parse_string_or_array(rest.trim_start());
        if !parsed.is_empty() {
            values.push(parsed);
        }
    }
    values
}

/// 宽松扫描 `key:` 后的首个字符串值。
fn find_key_string(text: &str, key: &str) -> Option<String> {
    scan_key_values(text, key)
        .into_iter()
        .next()
        .and_then(|values| values.into_iter().next())
}

/// 解析 `key:` 后的值：`"字符串"` 或 `["A", "B"]`，返回其中的字符串列表。
fn parse_string_or_array(rest: &str) -> Vec<String> {
    let rest = rest.trim_start();
    if let Some(body) = rest.strip_prefix('[') {
        let end = body.find(']').unwrap_or(body.len());
        parse_quoted_strings(&body[..end])
    } else {
        parse_quoted_strings(rest).into_iter().take(1).collect()
    }
}

/// 提取文本中全部双引号字符串（支持 `\"` 与 `\\` 转义）。
fn parse_quoted_strings(text: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '"' {
            continue;
        }
        let mut value = String::new();
        while let Some(c) = chars.next() {
            match c {
                '\\' => {
                    if let Some(escaped) = chars.next() {
                        value.push(escaped);
                    }
                }
                '"' => break,
                _ => value.push(c),
            }
        }
        values.push(value);
    }
    values
}

/// 扫描 `pid:` 后的数字（省份详情为单行 JSON：关键词 + 数字直接扫描；不存在则不会匹配）。
fn scan_key_numbers(text: &str, key: &str) -> Vec<u32> {
    let bytes = text.as_bytes();
    let mut numbers = Vec::new();
    let mut cursor = 0;
    while let Some(position) = text[cursor..].find(key) {
        let start = cursor + position + key.len();
        cursor = start;
        let mut index = start;
        // 跳过 `"` / 空白 / `:`（兼容 `pid:0` 与 `"pid": 0`）
        while index < bytes.len() && matches!(bytes[index], b'"' | b' ' | b'\t' | b':') {
            index += 1;
        }
        let digits_start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if index > digits_start {
            if let Ok(number) = text[digits_start..index].parse::<u32>() {
                numbers.push(number);
            }
            cursor = index;
        }
    }
    numbers
}

/// 解析 `<地图>/data/ProvinceDetails.json` 的全部省份 ID（升序去重）。
pub fn parse_province_ids(text: &str) -> Vec<u32> {
    let mut ids = scan_key_numbers(text, "pid");
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// 解析 `<地图>/cities/cities.json`：逐行扫描 `Name:` / `p:` 对（`//` 注释行自然跳过）。
/// 返回（省份 ID，地名）列表（同一省份可出现多次，调用方按首个地名优先）。
pub fn parse_city_names(text: &str) -> Vec<(u32, String)> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut entries = Vec::new();
    let mut current_name: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Name:") {
            let name = parse_field_value(rest);
            current_name = if name.is_empty() { None } else { Some(name) };
        } else if let Some(rest) = line.strip_prefix("p:") {
            if let Some(name) = current_name.take() {
                if let Ok(id) = parse_field_value(rest).parse::<u32>() {
                    entries.push((id, name));
                }
            }
        }
    }
    entries
}

/// 解析 `buildings/Buildings.json`：`Name:` 数组按出现顺序即建筑 ID
/// （数组内为各等级名称，用 `/` 拼接展示）。
pub fn parse_buildings(text: &str) -> Vec<BuildingEntry> {
    scan_key_values(text, "Name")
        .into_iter()
        .enumerate()
        .filter(|(_, names)| !names.is_empty())
        .map(|(index, names)| BuildingEntry {
            id: index as u32,
            name: names.join("/"),
        })
        .collect()
}

/// 解析 `diseases/Diseases.json`：`Name:` 按出现顺序即疾病 ID；显示名优先中译。
pub fn parse_diseases(text: &str, translations: &HashMap<String, String>) -> Vec<DiseaseEntry> {
    scan_key_values(text, "Name")
        .into_iter()
        .enumerate()
        .filter_map(|(index, names)| {
            let raw = names.into_iter().next()?;
            let name = translations.get(&raw).cloned().unwrap_or(raw);
            Some(DiseaseEntry {
                id: index as u32,
                name,
            })
        })
        .collect()
}

/// 提取人物 json 中的首个 `Name:` 值（人物文件为小型数组包装）。
fn character_name_of(content: &str) -> Option<String> {
    find_key_string(content, "Name").filter(|name| !name.is_empty())
}

/// 由（文件名, 内容）列表构建人物名单：内部 `Name` 与文件名（去除 `.json`）都会收录
/// （去重、保持文件顺序）。少数文件两者不同（标签前缀/改名），两者都作为候选。
fn build_character_names(files: &[(String, String)]) -> Vec<String> {
    let mut names = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for (file_name, content) in files {
        let stem = file_name
            .strip_suffix(".json")
            .or_else(|| file_name.strip_suffix(".JSON"))
            .unwrap_or(file_name);
        let inner = character_name_of(content);
        for candidate in [inner.as_deref(), Some(stem)] {
            let Some(candidate) = candidate else { continue };
            if !candidate.is_empty() && seen.insert(candidate.to_string()) {
                names.push(candidate.to_string());
            }
        }
    }
    names
}

// ===== 对照表：文明 tag ↔ 名称 / 政体整数 ↔ 名称 =====

/// 对照表中的文明条目。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CivNameEntry {
    pub tag: String,
    pub name: String,
}

/// 事件编辑器对照表（`load_event_lookup` 命令的返回结构）。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct EventLookup {
    /// 文明清单：`Civilizations.txt` 顺序在前，翻译表额外条目在后。
    pub civs: Vec<CivNameEntry>,
    /// 政体清单：`index` 为游戏内「政体整数」。
    pub governments: Vec<GovernmentEntry>,
    /// 人物名称（`characters/**.json` 的 `Name` 与文件名，供 add_general 系列补全）。
    pub characters: Vec<String>,
    /// 省份清单（`id` 升序，名称可能为空）。
    pub provinces: Vec<ProvinceEntry>,
    /// 建筑清单（数组顺序即 ID）。
    pub buildings: Vec<BuildingEntry>,
    /// 疾病清单（数组顺序即 ID）。
    pub diseases: Vec<DiseaseEntry>,
}

/// 名称解析核心（与 `generate_civ` 的查找优先级一致）：
/// 1. 翻译[文件名 tag]；2. 翻译[JSON 内的 Tag]；3. JSON 的 Name；4. tag 本身兜底。
pub fn civ_display_name(
    tag: &str,
    civ: Option<&Civ>,
    translations: &HashMap<String, String>,
) -> String {
    if let Some(name) = translations.get(tag) {
        return name.clone();
    }
    if let Some(civ) = civ {
        if let Some(name) = translations.get(&civ.tag) {
            return name.clone();
        }
        if !civ.name.is_empty() {
            return civ.name.clone();
        }
    }
    tag.to_string()
}

/// 构建对照表文明清单：
/// - 按 `tags` 顺序输出，`load_json` 仅在缺失翻译时按需调用；
/// - 追加只存在于翻译表中的额外 tag（排除重复），按字典序排在最后。
pub fn build_civ_entries(
    tags: &[String],
    translations: &HashMap<String, String>,
    mut load_json: impl FnMut(&str) -> Option<Civ>,
) -> Vec<CivNameEntry> {
    let mut entries: Vec<CivNameEntry> = Vec::with_capacity(tags.len());
    for tag in tags {
        let civ = if translations.contains_key(tag) {
            None
        } else {
            load_json(tag)
        };
        entries.push(CivNameEntry {
            tag: tag.clone(),
            name: civ_display_name(tag, civ.as_ref(), translations),
        });
    }

    let listed: HashSet<&str> = tags.iter().map(String::as_str).collect();
    let mut extras: Vec<&String> = translations
        .keys()
        .filter(|key| !listed.contains(key.as_str()))
        .collect();
    extras.sort();
    for tag in extras {
        entries.push(CivNameEntry {
            tag: tag.clone(),
            name: translations[tag].clone(),
        });
    }
    entries
}

/// 缺失翻译的 tag（这些 tag 需要读取 `civilizations/{tag}.json` 补充名称）。
fn untranslated_tags<'a>(
    tags: &'a [String],
    translations: &HashMap<String, String>,
) -> Vec<&'a str> {
    tags.iter()
        .filter(|tag| !translations.contains_key(tag.as_str()))
        .map(String::as_str)
        .collect()
}

/// 由 missions 资源根推导候选游戏数据目录（工作区相对路径，按优先级）：
/// - `…/assets/game/missions` → `…/assets/game`（全局国策）；
/// - `…/assets/map/<地图>/scenarios/<剧本>/missions` → `<前缀>/assets/game`（剧本国策共用全局数据）；
/// - 经典 `missions` → 其上级目录。
pub fn game_dir_candidates(missions_root: &str) -> Vec<String> {
    let root = missions_root.trim().trim_matches('/');
    let segments: Vec<&str> = root.split('/').filter(|s| !s.is_empty()).collect();
    let mut candidates: Vec<String> = Vec::new();
    let mut push = |candidate: String| {
        if !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    };

    for (index, segment) in segments.iter().enumerate() {
        if *segment != "assets" {
            continue;
        }
        if segments.get(index + 1) == Some(&"game") {
            push(segments[..index + 2].join("/"));
        } else if segments.get(index + 2).is_some()
            && segments.get(index + 3) == Some(&"scenarios")
            && segments.get(index + 4).is_some()
            && segments.get(index + 5) == Some(&"missions")
        {
            let prefix = segments[..index].join("/");
            push(if prefix.is_empty() {
                "assets/game".to_string()
            } else {
                format!("{prefix}/assets/game")
            });
        }
    }

    // 经典布局：`missions` 的上级目录（工作区根用空串表示）。
    if !segments.is_empty() {
        push(segments[..segments.len() - 1].join("/"));
    }
    candidates
}

/// 地图信息：`Maps.json` 中的地图目录及其 `Scenarios.txt` 剧本清单。
#[derive(Debug, Clone, PartialEq)]
struct MapInfo {
    folder: String,
    scenarios: Vec<String>,
}

/// 由 missions 资源根解析所属地图目录（用于省份数据选择）：
/// 形如 `…/assets/map/<地图>/scenarios/<剧本>/missions` 时返回 `<地图>`；
/// 地图目录名不在 `Maps.json` 时，用 `<地图>/Scenarios.txt` 的剧本清单反查所属地图；
/// 全局资源根（`…/assets/game/missions`）与经典根返回 `None`（按全部地图合并）。
fn scenario_map_of(maps: &[MapInfo], missions_root: &str) -> Option<String> {
    let segments: Vec<&str> = missions_root
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    if segments.last() != Some(&"missions") {
        return None;
    }
    for index in 0..segments.len() {
        if segments[index] != "map" {
            continue;
        }
        if segments.get(index + 2).copied() != Some("scenarios") {
            continue;
        }
        let (Some(map), Some(scenario)) = (segments.get(index + 1), segments.get(index + 3))
        else {
            continue;
        };
        if maps.iter().any(|info| info.folder == *map) {
            return Some((*map).to_string());
        }
        // 目录名不在 Maps.json：尝试用剧本清单反查（同一剧本目录只属于一张地图）。
        if let Some(found) = maps
            .iter()
            .find(|info| info.scenarios.iter().any(|name| name == scenario))
        {
            return Some(found.folder.clone());
        }
    }
    None
}

/// 游戏数据目录 → assets 根（`<前缀>/assets/game` → `<前缀>/assets`；`game` → 空串）。
fn assets_prefix_of_candidate(candidate: &str) -> Option<String> {
    if candidate == "game" {
        return Some(String::new());
    }
    candidate.strip_suffix("/game").map(str::to_string)
}

/// 候选目录 → APK 内对应条目前缀（取从 `assets` 段起的前缀；无 `assets` 段返回 None）。
fn apk_base_of_candidate(candidate: &str) -> Option<String> {
    let mut parts = Vec::new();
    let mut found = false;
    for segment in candidate.split('/').filter(|segment| !segment.is_empty()) {
        if segment == "assets" {
            found = true;
        }
        if found {
            parts.push(segment);
        }
    }
    found.then(|| parts.join("/"))
}

// ===== 数据读取源：工作区目录 / APK 条目 =====

/// 对照数据读取源：把相对「游戏数据目录 / assets 目录」的名称映射到实际读取。
/// 工作区目录直接读文件；「从 apk 中导入」后的工作区只含 missions / scenarios 版块，
/// 回退到直接读取源 APK 内的条目。
trait DataSource {
    /// 读取文本文件（缺失/失败返回 `None`）。
    fn read_text(&self, relative: &str) -> Option<String>;
    /// 批量读取文本文件（默认逐个读取；目录源可并行）。
    fn read_texts(&self, relatives: &[String]) -> Vec<Option<String>> {
        relatives
            .iter()
            .map(|relative| self.read_text(relative))
            .collect()
    }
    /// 递归读取目录下全部 `.json`：返回（文件名, 内容）。
    fn read_json_files(&self, relative_dir: &str) -> Vec<(String, String)>;
    /// 探测文件是否存在。
    fn has_file(&self, relative: &str) -> bool;
}

/// 工作区目录数据源（`root` 为游戏数据目录或 assets 目录的绝对路径）。
struct DirSource {
    root: PathBuf,
}

impl DirSource {
    fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl DataSource for DirSource {
    fn read_text(&self, relative: &str) -> Option<String> {
        fs::read_to_string(self.root.join(relative)).ok()
    }

    fn read_texts(&self, relatives: &[String]) -> Vec<Option<String>> {
        relatives
            .par_iter()
            .map(|relative| self.read_text(relative))
            .collect()
    }

    fn read_json_files(&self, relative_dir: &str) -> Vec<(String, String)> {
        let mut files = Vec::new();
        collect_json_files(&self.root.join(relative_dir), &mut files);
        files.sort();
        files
            .iter()
            .filter_map(|path| {
                let name = path.file_name()?.to_string_lossy().into_owned();
                let content = fs::read_to_string(path).ok()?;
                Some((name, content))
            })
            .collect()
    }

    fn has_file(&self, relative: &str) -> bool {
        self.root.join(relative).is_file()
    }
}

/// 单个 APK 条目的最大解压尺寸（8MB；对照数据文件远小于此）。
const ENTRY_READ_LIMIT: u64 = 8 * 1024 * 1024;

/// 已打开的源 APK：文件句柄 + 中央目录条目。
struct ApkArchive {
    file: File,
    entries: Vec<ScanEntry>,
    /// 条目名 → 下标（精确读取用）。
    index: HashMap<String, usize>,
}

impl ApkArchive {
    fn open(file: File) -> Result<Self, String> {
        let entries = list_apk_entries(&file)?;
        let index = entries
            .iter()
            .enumerate()
            .map(|(position, entry)| (entry.name.clone(), position))
            .collect();
        Ok(Self {
            file,
            entries,
            index,
        })
    }

    fn read_entry(&self, entry_name: &str) -> Option<String> {
        let position = *self.index.get(entry_name)?;
        let bytes =
            read_apk_entry_bytes(&self.file, &self.entries[position], ENTRY_READ_LIMIT).ok()?;
        Some(String::from_utf8_lossy(&bytes).into_owned())
    }
}

/// APK 条目数据源：`base` 为条目前缀（如 `assets/game` / `assets`）。
struct ApkSource<'a> {
    archive: &'a ApkArchive,
    base: String,
}

impl<'a> ApkSource<'a> {
    fn new(archive: &'a ApkArchive, base: &str) -> Self {
        Self {
            archive,
            base: base.trim_matches('/').to_string(),
        }
    }

    fn entry_name(&self, relative: &str) -> String {
        join_rel(&self.base, relative)
    }
}

impl DataSource for ApkSource<'_> {
    fn read_text(&self, relative: &str) -> Option<String> {
        self.archive.read_entry(&self.entry_name(relative))
    }

    fn read_json_files(&self, relative_dir: &str) -> Vec<(String, String)> {
        let prefix = format!("{}/", self.entry_name(relative_dir));
        let mut files: Vec<(String, String)> = self
            .archive
            .entries
            .iter()
            .filter(|entry| {
                entry.name.starts_with(&prefix)
                    && entry.name.to_ascii_lowercase().ends_with(".json")
            })
            .filter_map(|entry| {
                let name = entry.name.rsplit('/').next()?.to_string();
                let content = self.archive.read_entry(&entry.name)?;
                Some((name, content))
            })
            .collect();
        files.sort();
        files
    }

    fn has_file(&self, relative: &str) -> bool {
        self.archive.index.contains_key(&self.entry_name(relative))
    }
}

/// 合并多个地图的省份数据：`ids` 为省份 ID 列表，`city_names` 为（省份 ID，地名）；
/// 跨地图按 ID 去重（先出现者优先），最终按 ID 升序。
fn assemble_provinces(map_data: Vec<(Vec<u32>, Vec<(u32, String)>)>) -> Vec<ProvinceEntry> {
    let mut provinces = Vec::new();
    let mut seen: HashSet<u32> = HashSet::new();
    for (ids, city_names) in map_data {
        let mut names: HashMap<u32, String> = HashMap::new();
        for (id, name) in city_names {
            names.entry(id).or_insert(name);
        }
        for id in ids {
            if seen.insert(id) {
                provinces.push(ProvinceEntry {
                    id,
                    name: names.get(&id).cloned().unwrap_or_default(),
                });
            }
        }
    }
    provinces.sort_by_key(|entry| entry.id);
    provinces
}

/// 从数据源加载地图清单（`Maps.json`，跳过没有数据的目录）。
fn load_maps_from(source: &dyn DataSource) -> Vec<MapInfo> {
    let Some(text) = source.read_text(MAPS_FILE) else {
        return Vec::new();
    };
    parse_map_folders(&text)
        .into_iter()
        .filter_map(|folder| {
            let map_dir = format!("map/{folder}");
            let scenarios_file = format!("{map_dir}/{SCENARIOS_FILE}");
            if !source.has_file(&scenarios_file)
                && !source.has_file(&format!("{map_dir}/{PROVINCE_DETAILS_FILE}"))
            {
                return None;
            }
            let scenarios = source
                .read_text(&scenarios_file)
                .map(|text| parse_scenario_list(&text))
                .unwrap_or_default();
            Some(MapInfo { folder, scenarios })
        })
        .collect()
}

/// 从数据源加载省份数据（`scenario_map` 为 Some 时只取该地图）。
fn load_provinces_from(
    source: &dyn DataSource,
    maps: &[MapInfo],
    scenario_map: Option<&str>,
) -> Vec<ProvinceEntry> {
    let mut map_data = Vec::new();
    for map in maps {
        if scenario_map.is_some_and(|want| want != map.folder) {
            continue;
        }
        let map_dir = format!("map/{}", map.folder);
        let ids = source
            .read_text(&format!("{map_dir}/{PROVINCE_DETAILS_FILE}"))
            .map(|text| parse_province_ids(&text))
            .unwrap_or_default();
        let city_names = source
            .read_text(&format!("{map_dir}/{CITIES_FILE}"))
            .map(|text| parse_city_names(&text))
            .unwrap_or_default();
        map_data.push((ids, city_names));
    }
    assemble_provinces(map_data)
}

/// 真实路径：递归收集目录下全部 `.json` 文件。
fn collect_json_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_json_files(&path, out);
        } else if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
        {
            out.push(path);
        }
    }
}

/// 读取一组 `.properties` 翻译（后者覆盖前者）。
fn load_translations_from(source: &dyn DataSource, relatives: &[&str]) -> HashMap<String, String> {
    let mut translations = HashMap::new();
    for relative in relatives {
        if let Some(text) = source.read_text(relative) {
            translations.extend(parse_translations(&text));
        }
    }
    translations
}

/// 组装对照表（真实路径 / SAF / 测试共用）。
#[allow(clippy::too_many_arguments)]
fn assemble_lookup(
    tags: &[String],
    translations: &HashMap<String, String>,
    civ_jsons: &HashMap<String, Civ>,
    governments_text: &str,
    characters: Vec<String>,
    provinces: Vec<ProvinceEntry>,
    buildings: Vec<BuildingEntry>,
    diseases: Vec<DiseaseEntry>,
) -> EventLookup {
    EventLookup {
        civs: build_civ_entries(tags, translations, |tag| civ_jsons.get(tag).cloned()),
        governments: parse_governments(governments_text),
        characters,
        provinces,
        buildings,
        diseases,
    }
}

/// 真实路径模式：从候选游戏数据目录加载对照表（无数据时返回空表）。
/// （Android 上命令统一走 [`load_event_lookup_blocking_with`]，不需要本包装。）
#[cfg(not(target_os = "android"))]
pub fn load_event_lookup_blocking(
    work_directory: &str,
    missions_root: &str,
) -> Result<EventLookup, String> {
    load_event_lookup_blocking_with(work_directory, missions_root, &|location| {
        File::open(location).ok()
    })
}

/// [`load_event_lookup_blocking`] 的通用版本：`open_apk` 负责把标记 / 搜索结果
/// 打开为可随机读取的文件句柄（Android 真实路径模式下 `content://` URI 走插件打开）。
pub fn load_event_lookup_blocking_with(
    work_directory: &str,
    missions_root: &str,
    open_apk: &dyn Fn(&str) -> Option<File>,
) -> Result<EventLookup, String> {
    let root = PathBuf::from(work_directory);
    if !root.is_dir() {
        return Err(format!("工作目录不存在：{}", root.display()));
    }
    let candidates = game_dir_candidates(missions_root);

    // 1) 工作区目录（完整解压 / 手工准备的游戏数据）。
    for candidate in &candidates {
        let game_dir = if candidate.is_empty() {
            root.clone()
        } else {
            root.join(candidate)
        };
        if !game_dir.join(CIV_TAGS_FILE).is_file() && !game_dir.join(GOV_FILE).is_file() {
            continue;
        }
        let assets_dir = assets_prefix_of_candidate(candidate).map(|relative| {
            if relative.is_empty() {
                root.clone()
            } else {
                root.join(&relative)
            }
        });
        let game_source = DirSource::new(game_dir);
        let assets_source = assets_dir.map(DirSource::new);
        let assets_ref: Option<&dyn DataSource> = assets_source
            .as_ref()
            .map(|source| source as &dyn DataSource);
        return Ok(load_lookup_from_sources(
            &game_source,
            assets_ref,
            missions_root,
        ));
    }

    // 2) APK 兜底：「从 apk 中导入」后的工作区只剩 missions / scenarios 版块，
    //    其余游戏数据直接从源 APK 按需读取。
    if let Some(archive) = open_source_apk(&root, missions_root, open_apk) {
        for candidate in &candidates {
            let Some(base) = apk_base_of_candidate(candidate) else {
                continue;
            };
            let game_source = ApkSource::new(&archive, &base);
            if !game_source.has_file(CIV_TAGS_FILE) && !game_source.has_file(GOV_FILE) {
                continue;
            }
            // 候选形如 `…/assets/game` → APK 内 assets 数据根为 `…/assets`。
            let assets_base = base.strip_suffix("/game").unwrap_or(&base);
            let assets_source = ApkSource::new(&archive, assets_base);
            return Ok(load_lookup_from_sources(
                &game_source,
                Some(&assets_source),
                missions_root,
            ));
        }
    }

    Ok(EventLookup::default())
}

/// 打开「源 APK」：优先进口时写入的标记文件（内容为 APK 路径或 URI），
/// 其次搜索工作区顶层的 `*.apk`（优先与解包目录同名的）。
/// 打开或解析失败返回 `None`（编辑器保持无补全状态，不报错）。
fn open_source_apk(
    work_directory: &Path,
    missions_root: &str,
    open_apk: &dyn Fn(&str) -> Option<File>,
) -> Option<ApkArchive> {
    let try_open = |location: &str| -> Option<ApkArchive> {
        let file = open_apk(location)?;
        ApkArchive::open(file).ok()
    };

    let top = missions_root
        .split('/')
        .find(|segment| !segment.is_empty());

    // 1) 标记文件：`<工作区>/<顶层目录>/.ageciv-source` 与 `<工作区>/.ageciv-source`。
    let mut markers = Vec::new();
    if let Some(top) = top {
        markers.push(work_directory.join(top).join(SOURCE_APK_MARKER));
    }
    markers.push(work_directory.join(SOURCE_APK_MARKER));
    for marker in markers {
        let Ok(text) = fs::read_to_string(&marker) else {
            continue;
        };
        let location = text.trim();
        if !location.is_empty() {
            if let Some(archive) = try_open(location) {
                return Some(archive);
            }
        }
    }

    // 2) 工作区顶层搜索 `*.apk`。
    let mut apks: Vec<PathBuf> = fs::read_dir(work_directory)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("apk"))
        })
        .collect();
    apks.sort();
    if let Some(top) = top {
        if let Some(position) = apks.iter().position(|path| {
            path.file_stem()
                .is_some_and(|stem| stem.to_string_lossy().eq_ignore_ascii_case(top))
        }) {
            let preferred = apks.remove(position);
            apks.insert(0, preferred);
        }
    }
    for apk in apks {
        if let Some(archive) = try_open(&apk.to_string_lossy()) {
            return Some(archive);
        }
    }
    None
}

/// 读取数据源下的全部对照原料（工作区目录 / APK / 测试共用）。
fn load_lookup_from_sources(
    game: &dyn DataSource,
    assets: Option<&dyn DataSource>,
    missions_root: &str,
) -> EventLookup {
    // 翻译：默认 Bundle 打底，简体中文覆盖（两者都可能缺失）。
    let translations = load_translations_from(game, &[CIV_BUNDLE, CIV_BUNDLE_CN]);

    let tags = game
        .read_text(CIV_TAGS_FILE)
        .map(|raw| parse_civilization_tags(&raw))
        .unwrap_or_default();

    // 只为缺失翻译的 tag 读取 json（目录源并行读取；失败项按无 json 处理）。
    let needed: Vec<String> = untranslated_tags(&tags, &translations)
        .iter()
        .map(|tag| format!("{CIV_DIR}/{tag}.json"))
        .collect();
    let civ_prefix = format!("{CIV_DIR}/");
    let civ_jsons: HashMap<String, Civ> = game
        .read_texts(&needed)
        .into_iter()
        .zip(needed.iter())
        .filter_map(|(content, relative)| {
            let content = content?;
            let civ = parse_loose_json(&content).ok()?;
            let tag = relative
                .strip_prefix(&civ_prefix)?
                .strip_suffix(".json")?
                .to_string();
            Some((tag, civ))
        })
        .collect();

    let governments_text = game.read_text(GOV_FILE).unwrap_or_default();

    // 人物 / 建筑 / 疾病。
    let characters = build_character_names(&game.read_json_files(CHARACTERS_DIR));
    let buildings = game
        .read_text(BUILDINGS_FILE)
        .map(|text| parse_buildings(&text))
        .unwrap_or_default();
    let disease_translations = load_translations_from(game, &[ROOT_BUNDLE, ROOT_BUNDLE_CN]);
    let diseases = game
        .read_text(DISEASES_FILE)
        .map(|text| parse_diseases(&text, &disease_translations))
        .unwrap_or_default();

    // 省份：由 assets 源发现地图（`Maps.json`），剧本根按其所属地图过滤。
    let provinces = match assets {
        Some(source) => {
            let maps = load_maps_from(source);
            let scenario_map = scenario_map_of(&maps, missions_root);
            load_provinces_from(source, &maps, scenario_map.as_deref())
        }
        None => Vec::new(),
    };

    assemble_lookup(
        &tags,
        &translations,
        &civ_jsons,
        &governments_text,
        characters,
        provinces,
        buildings,
        diseases,
    )
}

/// SAF/scoped 相对路径拼接（`dir` 为空串表示工作区根）。
fn join_rel(dir: &str, name: &str) -> String {
    let dir = dir.trim_matches('/');
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

/// SAF：读取目录下全部人物 json 的名称（内部 `Name` 与文件名都会收录）。
async fn load_characters_scoped<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    folder_id: &str,
    dir_path: &str,
) -> Vec<String> {
    let Ok(entries) = bridge::list_dir(app, folder_id, Some(dir_path.to_string())).await else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .iter()
        .filter(|entry| !entry.is_dir && entry.name.to_ascii_lowercase().ends_with(".json"))
        .map(|entry| entry.name.clone())
        .collect();
    names.sort();
    if names.is_empty() {
        return Vec::new();
    }
    let contents: HashMap<String, String> =
        bridge::read_text_files_in_dir(app, folder_id, dir_path, names.clone())
            .await
            .unwrap_or_default()
            .into_iter()
            .collect();
    let pairs: Vec<(String, String)> = names
        .into_iter()
        .map(|name| {
            let content = contents.get(&name).cloned().unwrap_or_default();
            (name, content)
        })
        .collect();
    build_character_names(&pairs)
}

/// SAF：从 assets 前缀加载地图清单（`Maps.json`；剧本清单读取失败按空处理）。
async fn load_maps_scoped<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    folder_id: &str,
    assets_prefix: &str,
) -> Vec<MapInfo> {
    let map_root = join_rel(assets_prefix, "map");
    let Ok(text) = bridge::read_text_file(app, folder_id, &join_rel(assets_prefix, MAPS_FILE)).await
    else {
        return Vec::new();
    };
    let mut maps = Vec::new();
    for folder in parse_map_folders(&text) {
        let scenarios = bridge::read_text_file(
            app,
            folder_id,
            &join_rel(&join_rel(&map_root, &folder), SCENARIOS_FILE),
        )
        .await
        .map(|text| parse_scenario_list(&text))
        .unwrap_or_default();
        maps.push(MapInfo { folder, scenarios });
    }
    maps
}

/// SAF：加载省份数据（`scenario_map` 为 Some 时只取该地图）。
async fn load_provinces_scoped<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    folder_id: &str,
    assets_prefix: &str,
    maps: &[MapInfo],
    scenario_map: Option<&str>,
) -> Vec<ProvinceEntry> {
    let map_root = join_rel(assets_prefix, "map");
    let mut map_data = Vec::new();
    for map in maps {
        if scenario_map.is_some_and(|want| want != map.folder) {
            continue;
        }
        let map_dir = join_rel(&map_root, &map.folder);
        let ids = bridge::read_text_file(
            app,
            folder_id,
            &join_rel(&map_dir, PROVINCE_DETAILS_FILE),
        )
        .await
        .map(|text| parse_province_ids(&text))
        .unwrap_or_default();
        let city_names = bridge::read_text_file(app, folder_id, &join_rel(&map_dir, CITIES_FILE))
            .await
            .map(|text| parse_city_names(&text))
            .unwrap_or_default();
        map_data.push((ids, city_names));
    }
    assemble_provinces(map_data)
}

/// 加载事件编辑器「文明 / 政体 / 省份 / 建筑 / 疾病 / 人物对照表」（真实路径模式）。
///
/// `missions_root` 为工作区内的国策资源根（如 `assets/game/missions`）；
/// 找不到游戏数据文件时返回空表（编辑器不显示补全但不报错）。
/// 「从 apk 中导入」的工作区只有 missions / scenarios 版块：优先按导入时写入的
/// 标记文件、其次搜索工作区内的 `.apk`，直接从源 APK 读取对照数据
/// （Android 上 `content://` 形式的 APK 位置经 android-fs 插件打开）。
#[tauri::command]
pub async fn load_event_lookup<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    work_directory: String,
    missions_root: String,
) -> Result<EventLookup, String> {
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(target_os = "android")]
        {
            use tauri_plugin_android_fs::{AndroidFsExt, FsUri};
            let open_apk = |location: &str| {
                let uri = if location.contains("://") {
                    FsUri::from_uri(location.to_string())
                } else {
                    FsUri::from_path(Path::new(location))
                };
                app.android_fs().open_file_readable(&uri).ok()
            };
            load_event_lookup_blocking_with(&work_directory, &missions_root, &open_apk)
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = app;
            load_event_lookup_blocking(&work_directory, &missions_root)
        }
    })
    .await
    .map_err(|error| format!("加载文明对照表失败：{error}"))?
}

/// 加载事件编辑器「文明 / 政体对照表」（Android SAF/scoped 模式）。
///
/// 与真实路径模式的区别：文明 json 与人物 json 均为按目录批量读取（缺失项跳过），
/// 其余文本文件读取失败同样按缺失处理；省份等地图数据按候选 assets 前缀定位。
#[tauri::command]
pub async fn load_event_lookup_scoped<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    folder_id: String,
    missions_root: String,
) -> Result<EventLookup, String> {
    if folder_id.trim().is_empty() {
        return Err("缺少 scoped 目录授权".to_string());
    }

    for candidate in game_dir_candidates(&missions_root) {
        let civ_text = bridge::read_text_file(&app, &folder_id, &join_rel(&candidate, CIV_TAGS_FILE))
            .await
            .ok();
        let governments_text =
            bridge::read_text_file(&app, &folder_id, &join_rel(&candidate, GOV_FILE))
                .await
                .ok();
        if civ_text.is_none() && governments_text.is_none() {
            continue;
        }

        let mut translations: HashMap<String, String> = HashMap::new();
        for relative in [CIV_BUNDLE, CIV_BUNDLE_CN] {
            if let Ok(text) =
                bridge::read_text_file(&app, &folder_id, &join_rel(&candidate, relative)).await
            {
                translations.extend(parse_translations(&text));
            }
        }

        let tags = civ_text
            .as_deref()
            .map(parse_civilization_tags)
            .unwrap_or_default();

        // 批量读取缺失翻译的文明 json（目录不存在时全部按缺失处理）。
        let needed: Vec<String> = untranslated_tags(&tags, &translations)
            .iter()
            .map(|tag| format!("{tag}.json"))
            .collect();
        let mut civ_jsons: HashMap<String, Civ> = HashMap::new();
        if !needed.is_empty() {
            let files = bridge::read_text_files_in_dir(
                &app,
                &folder_id,
                &join_rel(&candidate, CIV_DIR),
                needed,
            )
            .await
            .unwrap_or_default();
            for (name, content) in files {
                let Some(tag) = name.strip_suffix(".json") else {
                    continue;
                };
                if let Ok(civ) = parse_loose_json(&content) {
                    civ_jsons.insert(tag.to_string(), civ);
                }
            }
        }

        // 人物 / 建筑 / 疾病。
        let characters = load_characters_scoped(
            &app,
            &folder_id,
            &join_rel(&candidate, CHARACTERS_DIR),
        )
        .await;
        let buildings =
            bridge::read_text_file(&app, &folder_id, &join_rel(&candidate, BUILDINGS_FILE))
                .await
                .map(|text| parse_buildings(&text))
                .unwrap_or_default();
        let mut disease_translations: HashMap<String, String> = HashMap::new();
        for relative in [ROOT_BUNDLE, ROOT_BUNDLE_CN] {
            if let Ok(text) =
                bridge::read_text_file(&app, &folder_id, &join_rel(&candidate, relative)).await
            {
                disease_translations.extend(parse_translations(&text));
            }
        }
        let diseases =
            bridge::read_text_file(&app, &folder_id, &join_rel(&candidate, DISEASES_FILE))
                .await
                .map(|text| parse_diseases(&text, &disease_translations))
                .unwrap_or_default();

        // 省份：按候选 assets 前缀发现地图（`Maps.json`），剧本根只取所属地图。
        let provinces = match assets_prefix_of_candidate(&candidate) {
            Some(prefix) => {
                let maps = load_maps_scoped(&app, &folder_id, &prefix).await;
                let scenario_map = scenario_map_of(&maps, &missions_root);
                load_provinces_scoped(&app, &folder_id, &prefix, &maps, scenario_map.as_deref())
                    .await
            }
            None => Vec::new(),
        };

        return Ok(assemble_lookup(
            &tags,
            &translations,
            &civ_jsons,
            governments_text.as_deref().unwrap_or_default(),
            characters,
            provinces,
            buildings,
            diseases,
        ));
    }

    Ok(EventLookup::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ageciv-missions-db-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parse_translations_ignores_comments_and_spaces() {
        let map = parse_translations(
            "# 注释\n! 另一种注释\n\natr = 奥地利\nabba=阿拔斯王朝\natr = 奥地利帝国\n",
        );
        assert_eq!(map.get("atr").unwrap(), "奥地利帝国");
        assert_eq!(map.get("abba").unwrap(), "阿拔斯王朝");
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn parse_loose_json_reads_quoted_unquoted_and_missing_fields() {
        let civ = parse_loose_json(
            "{\n\"Tag\": \"atr\",\n\"Name\": \"\",\nGroupID: 7,\n\"Wiki\": \"Austria\",\n}",
        )
        .unwrap();
        assert_eq!(civ.tag, "atr");
        assert_eq!(civ.name, "");
        assert_eq!(civ.group_id, 7);
        assert_eq!(civ.wiki.as_deref(), Some("Austria"));
        assert_eq!(civ.religion_id, 0);

        // 值里允许出现冒号（只在第一个冒号分割）；字符串里的逗号不拆分字段。
        let civ = parse_loose_json("{Tag: x, Name: \"A:B, C\"}").unwrap();
        assert_eq!(civ.tag, "x");
        assert_eq!(civ.name, "A:B, C");
    }

    #[test]
    fn parse_civilization_tags_splits_semicolons() {
        let tags = parse_civilization_tags("\u{feff}abcde;adlulr;\n bhu ;;; sot ;\n");
        assert_eq!(tags, vec!["abcde", "adlulr", "bhu", "sot"]);
    }

    #[test]
    fn parse_scenario_list_splits_semicolons() {
        let scenarios = parse_scenario_list("TheGreatWar;\n 和平之路 ;");
        assert_eq!(scenarios, vec!["TheGreatWar", "和平之路"]);
    }

    #[test]
    fn parse_map_folders_reads_folder_entries() {
        // 与真实 `Maps.json` 一致：每个键独占一行。
        let text = "{\n Map: [\n {\n Folder: \"EarthM\",\n },\n {\n Folder: \"Earth3\"\n },\n ]\n}";
        assert_eq!(parse_map_folders(text), vec!["EarthM", "Earth3"]);
    }

    #[test]
    fn parse_province_ids_scans_single_line_json() {
        // 省份详情为单行 JSON：同时容忍带引号的键与重复/乱序。
        let text = "[{bd:22,pid:0,tr:5},{bd:24,pid:1,tr:5},{\"pid\": 123,tr:1},{pid:1,re:0}]";
        assert_eq!(parse_province_ids(text), vec![0, 1, 123]);
    }

    #[test]
    fn parse_city_names_pairs_name_and_province() {
        let text = "//@注释行\n[\n{\n Name: \"里斯本\",\n x: 7898,\n y: 2879,\n p: 11\n},\n{\n Name: \"马德里\",\n p: 6561\n},\n{\n p: 5\n},\n]";
        assert_eq!(
            parse_city_names(text),
            vec![(11, "里斯本".to_string()), (6561, "马德里".to_string())]
        );
    }

    #[test]
    fn parse_buildings_joins_level_names() {
        let text = "{\nBuildings:[\n{ Name: [\"基础设施\"], AI: [5], },\n{ Name: [\"铁路\", \"铁路Ⅱ\"], },\n]}";
        let buildings = parse_buildings(text);
        assert_eq!(
            buildings,
            vec![
                BuildingEntry { id: 0, name: "基础设施".into() },
                BuildingEntry { id: 1, name: "铁路/铁路Ⅱ".into() },
            ]
        );
    }

    #[test]
    fn parse_diseases_translates_names() {
        let text = "{\nDisease:[\n{ Name: \"BlackDeath\", Outbreak: 1, },\n{ Name: \"Smallpox\", },\n]}";
        let mut translations = HashMap::new();
        translations.insert("BlackDeath".to_string(), "黑死病".to_string());
        let diseases = parse_diseases(text, &translations);
        assert_eq!(
            diseases,
            vec![
                DiseaseEntry { id: 0, name: "黑死病".into() },
                DiseaseEntry { id: 1, name: "Smallpox".into() },
            ]
        );
    }

    #[test]
    fn build_character_names_keeps_inner_name_and_stem() {
        let files = vec![
            (
                "约瑟夫·霞飞.json".to_string(),
                "[{ Name: \"约瑟夫·霞飞\", Attack: 2, }]".to_string(),
            ),
            ("bra某人.json".to_string(), "[{ Name: \"阿尔贝托\", }]".to_string()),
            ("noName.json".to_string(), "[{ Attack: 1, }]".to_string()),
        ];
        assert_eq!(
            build_character_names(&files),
            vec!["约瑟夫·霞飞", "阿尔贝托", "bra某人", "noName"]
        );
    }

    #[test]
    fn scenario_map_of_resolves_and_falls_back() {
        let maps = vec![
            MapInfo { folder: "EarthM".into(), scenarios: Vec::new() },
            MapInfo { folder: "Earth3".into(), scenarios: vec!["TheGreatWar".into()] },
        ];
        assert_eq!(
            scenario_map_of(&maps, "包名/assets/map/Earth3/scenarios/TheGreatWar/missions"),
            Some("Earth3".to_string())
        );
        // 全局资源根：无剧本上下文（按全部地图合并）。
        assert_eq!(scenario_map_of(&maps, "包名/assets/game/missions"), None);
        // 地图目录名不在清单：用剧本清单反查所属地图。
        assert_eq!(
            scenario_map_of(&maps, "x/assets/map/Earth9/scenarios/TheGreatWar/missions"),
            Some("Earth3".to_string())
        );
        // 地图在清单、剧本未列出：宽容接受该地图。
        assert_eq!(
            scenario_map_of(&maps, "x/assets/map/Earth3/scenarios/Other/missions"),
            Some("Earth3".to_string())
        );
        // 经典根：无地图上下文。
        assert_eq!(scenario_map_of(&maps, "missions"), None);
    }

    #[test]
    fn assets_prefix_strips_game_suffix() {
        assert_eq!(
            assets_prefix_of_candidate("暮色/assets/game"),
            Some("暮色/assets".to_string())
        );
        assert_eq!(assets_prefix_of_candidate("assets/game"), Some("assets".to_string()));
        assert_eq!(assets_prefix_of_candidate("game"), Some(String::new()));
        assert_eq!(assets_prefix_of_candidate("foo"), None);
        assert_eq!(assets_prefix_of_candidate(""), None);
    }

    #[test]
    fn assemble_provinces_merges_maps_and_keeps_first_name() {
        let provinces = assemble_provinces(vec![
            (
                vec![11, 260],
                vec![
                    (11, "里斯本".to_string()),
                    (260, "旧名".to_string()),
                    (260, "巴黎".to_string()),
                ],
            ),
            (vec![260, 1000], vec![(260, "重叠".to_string())]),
        ]);
        assert_eq!(
            provinces,
            vec![
                ProvinceEntry { id: 11, name: "里斯本".into() },
                ProvinceEntry { id: 260, name: "旧名".into() },
                ProvinceEntry { id: 1000, name: String::new() },
            ]
        );
    }

    #[test]
    fn build_civ_entries_prefers_translation_then_json_then_tag() {
        let tags = vec![
            "atr".to_string(),
            "xyz".to_string(),
            "abc".to_string(),
            "abc2".to_string(),
        ];
        let mut translations = HashMap::new();
        translations.insert("atr".to_string(), "奥地利".to_string());
        translations.insert("abc2".to_string(), "别名国".to_string());
        translations.insert("afri_c".to_string(), "非洲人民共和国".to_string());

        let mut loaded: Vec<String> = Vec::new();
        let entries = build_civ_entries(&tags, &translations, |tag| {
            loaded.push(tag.to_string());
            match tag {
                "xyz" => Some(Civ {
                    tag: "xyz".to_string(),
                    name: "埃克斯国".to_string(),
                    ..Civ::default()
                }),
                "abc" => Some(Civ {
                    tag: "abc".to_string(),
                    name: String::new(),
                    ..Civ::default()
                }),
                "abc2" => Some(Civ {
                    tag: "abc2".to_string(),
                    name: "JSON 名".to_string(),
                    ..Civ::default()
                }),
                _ => None,
            }
        });

        // 有翻译的 tag 不读 json。
        assert!(!loaded.contains(&"atr".to_string()));
        assert_eq!(entries[0], CivNameEntry { tag: "atr".into(), name: "奥地利".into() });
        assert_eq!(entries[1], CivNameEntry { tag: "xyz".into(), name: "埃克斯国".into() });
        // JSON Name 为空 → tag 兜底。
        assert_eq!(entries[2], CivNameEntry { tag: "abc".into(), name: "abc".into() });
        // JSON Tag 命中翻译 → 用翻译，不用 JSON Name。
        assert_eq!(entries[3], CivNameEntry { tag: "abc2".into(), name: "别名国".into() });
        // 只存在于翻译表的额外 tag 追加在最后。
        assert_eq!(entries[4], CivNameEntry { tag: "afri_c".into(), name: "非洲人民共和国".into() });
        assert_eq!(entries.len(), 5);
    }

    #[test]
    fn parse_governments_maps_index_and_name() {
        let text = r#"{
    Government: [
        {
            Name: "临时政府",
            Extra_Tag: "",
            GOV_GROUP_ID: 0,
        },
        {
            Name: "绝对君主制",
            Extra_Tag: "m",
        },
        {
            Name: "只有名字没有 Extra_Tag",
        },
        {
            Name: "带,逗号的政府",
            Extra_Tag: "c",
        },
    ]
}"#;
        let entries = parse_governments(text);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].index, 0);
        assert_eq!(entries[0].name, "临时政府");
        assert_eq!(entries[0].tag, "");
        assert_eq!(entries[1].index, 1);
        assert_eq!(entries[1].name, "绝对君主制");
        assert_eq!(entries[1].tag, "m");
        assert_eq!(entries[2].index, 2);
        assert_eq!(entries[2].name, "带,逗号的政府");
    }

    #[test]
    fn governments_csv_escapes_names() {
        let csv = governments_csv(&[
            GovernmentEntry { index: 0, name: "临时政府".into(), tag: String::new() },
            GovernmentEntry { index: 1, name: "带,逗号".into(), tag: "m".into() },
        ]);
        assert_eq!(csv, "Index,Name,Tag\n0,临时政府,\n1,\"带,逗号\",m\n");
    }

    #[test]
    fn game_dir_candidates_cover_layouts() {
        assert_eq!(
            game_dir_candidates("暮色黄昏_世界大战0.25.1/assets/game/missions")[0],
            "暮色黄昏_世界大战0.25.1/assets/game"
        );
        assert_eq!(
            game_dir_candidates("包名/assets/map/Earth3/scenarios/TheGreatWar/missions")[0],
            "包名/assets/game"
        );
        assert_eq!(game_dir_candidates("assets/game/missions")[0], "assets/game");
        // 经典布局：missions 的上级（工作区根用空串表示）。
        assert_eq!(game_dir_candidates("missions"), vec![String::new()]);
        assert_eq!(game_dir_candidates("foo/missions")[0], "foo");
    }

    #[test]
    fn generate_gov_writes_csv_file() {
        let dir = temp_dir("gov");
        let input = dir.join("Governments.json");
        fs::write(
            &input,
            "{\nGovernment:[\n{\nName: \"甲\",\nExtra_Tag: \"a\",\n},\n{\nName: \"乙\",\nExtra_Tag: \"b\",\n},\n]}\n",
        )
        .unwrap();
        let output = dir.join("Governments.csv");
        let messages = generate_gov(&input, &output).unwrap();
        assert_eq!(messages, vec!["共 2 条记录".to_string()]);
        assert_eq!(
            fs::read_to_string(&output).unwrap(),
            "Index,Name,Tag\n0,甲,a\n1,乙,b\n"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn generate_civ_writes_csv_file_with_order_and_translations() {
        let dir = temp_dir("civ");
        fs::create_dir_all(dir.join("civilizations")).unwrap();
        fs::write(dir.join("Civilizations.txt"), "atr;xyz;bhu;").unwrap();
        fs::write(
            dir.join("civilizations/atr.json"),
            "{\n\"Tag\": \"atr\",\n\"Name\": \"\",\n}",
        )
        .unwrap();
        fs::write(
            dir.join("civilizations/xyz.json"),
            "{\n\"Tag\": \"xyz\",\n\"Name\": \"埃克斯国\",\n}",
        )
        .unwrap();
        // bhu.json 缺失 → 跳过该行，ID 留空号（与上游一致）。
        fs::write(dir.join("lang.properties"), "atr = 奥地利\n").unwrap();

        let output = dir.join("Civilizations.csv");
        let messages = generate_civ(
            &dir.join("Civilizations.txt"),
            &dir.join("civilizations"),
            &output,
            &dir.join("lang.properties"),
        )
        .unwrap();
        assert!(messages.iter().any(|message| message.contains("共写入 2 条记录")));
        assert!(messages.iter().any(|message| message.contains("1 个 Tag")));
        assert_eq!(
            fs::read_to_string(&output).unwrap(),
            "ID,Name,Tag\n0,奥地利,atr\n1,埃克斯国,xyz\n"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// 用真实数据做整体校验（工作区为「从 apk 中导入」后的版块：
    /// `暮色黄昏_世界大战0.25.1/assets/{game/missions,map/...}`，其余游戏数据从
    /// 工作区内的源 APK 读取）。
    /// 仅在显式执行时运行：`cargo test -p age_civ_mod_tool --lib -- --ignored`
    #[test]
    #[ignore = "需要真实数据 A:\\android\\GameCivs\\暮色黄昏_世界大战0.25.1(.apk)"]
    fn real_game_data_lookup() {
        let work_directory = r"A:\android\GameCivs";
        let missions_root = "暮色黄昏_世界大战0.25.1/assets/game/missions";
        let lookup =
            load_event_lookup_blocking(work_directory, missions_root).expect("读取真实数据失败");

        assert!(lookup.civs.len() >= 558, "文明条目过少：{}", lookup.civs.len());
        assert!(lookup.governments.len() >= 100);

        let atr = lookup.civs.iter().find(|entry| entry.tag == "atr").unwrap();
        assert_eq!(atr.name, "奥地利");
        // 无翻译时回退到 `civilizations/{tag}.json` 的 Name 字段。
        let abcde = lookup.civs.iter().find(|entry| entry.tag == "abcde").unwrap();
        assert_eq!(abcde.name, "事件国");
        // 只存在于翻译表 / JSON Name 的条目也应收录。
        assert!(lookup
            .civs
            .iter()
            .any(|entry| entry.tag == "afri_c" && entry.name == "非洲人民共和国"));

        let gov10 = lookup
            .governments
            .iter()
            .find(|entry| entry.index == 10)
            .unwrap();
        assert_eq!(gov10.name, "专制主义");

        // 扩展对照数据：省份 / 人物 / 建筑 / 疾病。
        assert!(lookup.provinces.len() >= 13000, "省份条目过少：{}", lookup.provinces.len());
        let shanghai = lookup.provinces.iter().find(|entry| entry.id == 2855).unwrap();
        assert_eq!(shanghai.name, "上海");
        let munich = lookup.provinces.iter().find(|entry| entry.id == 716).unwrap();
        assert_eq!(munich.name, "慕尼黑");
        assert!(lookup.characters.iter().any(|name| name == "约瑟夫·霞飞"));
        assert!(lookup
            .buildings
            .iter()
            .any(|entry| entry.id == 6 && entry.name == "要塞"));
        assert!(lookup
            .diseases
            .iter()
            .any(|entry| entry.id == 0 && entry.name == "黑死病"));

        // 剧本根（`assets/map/<地图>/scenarios/<剧本>/missions`）应同样加载到地图省份。
        let scenario_root = "暮色黄昏_世界大战0.25.1/assets/map/Earth3/scenarios/TheGreatWar/missions";
        let scenario_lookup =
            load_event_lookup_blocking(work_directory, scenario_root).expect("读取剧本数据失败");
        assert_eq!(scenario_lookup.provinces.len(), lookup.provinces.len());
        assert!(scenario_lookup
            .provinces
            .iter()
            .any(|entry| entry.id == 12049 && entry.name == "费罗尔"));

        // 标记文件方式：工作区只有导入版块，标记指向工作区之外的源 APK
        // （避免拷贝 GB 级文件，与 Android 上 `content://` URI 用法一致）。
        let import_dir = temp_dir("apk-import-real");
        let package_dir = import_dir.join("暮色黄昏_世界大战0.25.1");
        fs::create_dir_all(package_dir.join("assets/game/missions/missionsEvents")).unwrap();
        fs::write(
            package_dir.join(SOURCE_APK_MARKER),
            "A:\\android\\GameCivs\\暮色黄昏_世界大战0.25.1.apk\n",
        )
        .unwrap();
        let marked = load_event_lookup_blocking(
            import_dir.to_str().unwrap(),
            "暮色黄昏_世界大战0.25.1/assets/game/missions",
        )
        .expect("标记文件方式读取失败");
        assert_eq!(marked.civs.len(), lookup.civs.len());
        assert_eq!(marked.provinces.len(), lookup.provinces.len());
        assert!(marked.characters.iter().any(|name| name == "约瑟夫·霞飞"));
        let _ = fs::remove_dir_all(&import_dir);
    }

    /// 写入合成源 APK（「从 apk 中导入」兜底测试用）。
    fn write_synthetic_source_apk(path: &Path) {
        use std::io::Write;

        let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
        let mut add = |name: &str, content: &str| {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        };
        add("assets/game/Civilizations.txt", "atr;sok;");
        add(
            "assets/game/languages/civilizations/Bundle.properties",
            "atr = Austria",
        );
        add(
            "assets/game/languages/civilizations/Bundle_cn_sp.properties",
            "atr = 奥地利",
        );
        add(
            "assets/game/civilizations/sok.json",
            "{\n\"Tag\": \"sok\",\n\"Name\": \"索克国\",\n}",
        );
        add(
            "assets/game/Governments.json",
            "{\nGovernment:[\n{\nName: \"临时政府\",\nExtra_Tag: \"\",\n},\n{\nName: \"绝对君主制\",\nExtra_Tag: \"m\",\n},\n]}",
        );
        add(
            "assets/game/buildings/Buildings.json",
            "{\nBuildings:[\n{ Name: [\"基础设施\"], },\n{ Name: [\"要塞\", \"要塞Ⅱ\"], },\n]}",
        );
        add(
            "assets/game/diseases/Diseases.json",
            "{\nDisease:[\n{ Name: \"Plague\", },\n{ Name: \"Smallpox\", },\n]}",
        );
        add("assets/game/languages/Bundle_cn_sp.properties", "Plague = 瘟疫");
        add("assets/game/characters/某人.json", "[{ Name: \"某人甲\", }]");
        add(
            "assets/map/Maps.json",
            "{\n Map: [\n {\n Folder: \"TestMap\",\n },\n ]\n}",
        );
        add("assets/map/TestMap/Scenarios.txt", "TheGreatWar;");
        add(
            "assets/map/TestMap/data/ProvinceDetails.json",
            "[{bd:22,pid:7,tr:5},{bd:24,pid:9,tr:5}]",
        );
        add(
            "assets/map/TestMap/cities/cities.json",
            "[\n{\n Name: \"示例城\",\n p: 7\n},\n{\n Name: \"无名城\",\n p: 42\n},\n]",
        );
        drop(add);
        zip.finish().unwrap();
    }

    /// 校验合成 APK 的完整对照表（两个兜底测试共用）。
    fn assert_synthetic_lookup(lookup: &EventLookup) {
        assert_eq!(
            lookup.civs,
            vec![
                CivNameEntry { tag: "atr".into(), name: "奥地利".into() },
                CivNameEntry { tag: "sok".into(), name: "索克国".into() },
            ]
        );
        assert_eq!(lookup.governments.len(), 2);
        assert_eq!(lookup.governments[0].index, 0);
        assert_eq!(lookup.governments[0].name, "临时政府");
        assert_eq!(lookup.governments[0].tag, "");
        assert_eq!(lookup.governments[1].index, 1);
        assert_eq!(lookup.governments[1].name, "绝对君主制");
        assert_eq!(lookup.governments[1].tag, "m");
        assert_eq!(lookup.characters, vec!["某人甲".to_string(), "某人".to_string()]);
        assert_eq!(
            lookup.buildings,
            vec![
                BuildingEntry { id: 0, name: "基础设施".into() },
                BuildingEntry { id: 1, name: "要塞/要塞Ⅱ".into() },
            ]
        );
        assert_eq!(
            lookup.diseases,
            vec![
                DiseaseEntry { id: 0, name: "瘟疫".into() },
                DiseaseEntry { id: 1, name: "Smallpox".into() },
            ]
        );
        assert_eq!(
            lookup.provinces,
            vec![
                ProvinceEntry { id: 7, name: "示例城".into() },
                ProvinceEntry { id: 9, name: String::new() },
            ]
        );
    }

    /// 「从 apk 中导入」后的工作区（只有 missions / scenarios 版块）：
    /// 其余数据从工作区内的同名 `*.apk` 读取；剧本根按其所属地图过滤省份。
    #[test]
    fn imported_workspace_reads_lookup_from_workspace_apk() {
        let workspace = temp_dir("apk-workspace");
        let package_dir = workspace.join("测试包");
        fs::create_dir_all(package_dir.join("assets/game/missions/missionsEvents")).unwrap();
        fs::write(
            package_dir.join("assets/game/missions/missionsEvents/事件.txt"),
            "{}\n",
        )
        .unwrap();
        fs::create_dir_all(package_dir.join("assets/map/TestMap/scenarios/TheGreatWar/missions"))
            .unwrap();
        write_synthetic_source_apk(&workspace.join("测试包.apk"));

        let lookup = load_event_lookup_blocking(
            workspace.to_str().unwrap(),
            "测试包/assets/map/TestMap/scenarios/TheGreatWar/missions",
        )
        .expect("从工作区内 APK 读取失败");
        assert_synthetic_lookup(&lookup);

        let _ = fs::remove_dir_all(&workspace);
    }

    /// 导入时写入的标记文件指向工作区之外的 APK 时，同样能加载对照表
    /// （Android「从 apk 中导入」后标记内为 `content://` URI，由命令用插件打开）。
    #[test]
    fn imported_workspace_reads_lookup_from_marker_apk() {
        let import_dir = temp_dir("apk-marker");
        let apk = std::env::temp_dir().join(format!(
            "ageciv-missions-db-marker-{}.apk",
            std::process::id()
        ));
        write_synthetic_source_apk(&apk);
        let package_dir = import_dir.join("测试包");
        fs::create_dir_all(package_dir.join("assets/game/missions/missionsEvents")).unwrap();
        // 标记内容允许带尾部空白（路径 + 换行）。
        fs::write(
            package_dir.join(SOURCE_APK_MARKER),
            format!("{}\n", apk.display()),
        )
        .unwrap();

        let lookup = load_event_lookup_blocking(
            import_dir.to_str().unwrap(),
            "测试包/assets/game/missions",
        )
        .expect("从标记指向的 APK 读取失败");
        assert_synthetic_lookup(&lookup);

        let _ = fs::remove_dir_all(&import_dir);
        let _ = fs::remove_file(&apk);
    }
}
