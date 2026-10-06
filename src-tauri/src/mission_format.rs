//! 国策文件的解析、宽松语法自动纠正与序列化。
//!
//! 国策文件使用 JSON5 方言，且游戏本身接受的写法比 JSON5 更宽松
//! （数组/对象成员之间可省略逗号、值可为裸文本）。本模块负责在解析前
//! 把这类文本纠正为规范格式，并把纠正结果写回原文件。

use serde::{Deserialize, Serialize};

use crate::models::MissionRecord;

/// 国策文件根对象（`{ Mission: [...], Age_of_History: Mission }`）。
#[derive(Deserialize, Serialize)]
pub struct MissionFile {
    #[serde(rename = "Mission")]
    pub mission: Vec<MissionRecord>,
}

/// 解析国策配置文本。文件为合法 JSON5 方言时直接返回；若是游戏常见的宽松写法
/// （数组元素/对象成员之间缺逗号、值为裸标识符文本如 `Age_of_History: Mission`），
/// 先经 [`normalize_loose_json`] 纠正语法再解析，并附带规范化后的完整文件内容，
/// 由调用方写回原文件（打开文件时自动纠正并保存）。
pub fn parse_mission_file_with_correction(
    content: &str,
) -> Result<(MissionFile, Option<String>), String> {
    // 工具自身写出的规范格式（`Age_of_History: Mission` 裸标识符值）先按原样解析，避免无谓重写。
    let canonical_attempt =
        content.replace("Age_of_History: Mission", "Age_of_History: \"Mission\"");
    match json5::from_str::<MissionFile>(&canonical_attempt) {
        Ok(file) => Ok((file, None)),
        Err(original_error) => {
            let repaired = normalize_loose_json(content);
            match json5::from_str::<MissionFile>(&repaired) {
                Ok(file) => {
                    let corrected = serialize_mission_file(&file.mission)?;
                    Ok((file, Some(corrected)))
                }
                Err(repair_error) => Err(format!(
                    "国策文件语法错误，自动纠正后仍无法解析：{repair_error}（原始错误：{original_error}）"
                )),
            }
        }
    }
}

/// [`parse_mission_file_with_correction`] 的简化入口：只关心解析结果（仅供测试使用）。
#[cfg(test)]
fn parse_mission_file(content: &str) -> Result<MissionFile, String> {
    parse_mission_file_with_correction(content).map(|(file, _)| file)
}

/// 把游戏使用的宽松 JSON 方言规范化为合法 JSON5 文本：
/// - 数组元素、对象成员之间缺少的逗号自动补上（宽松文件常以换行分隔它们）；
/// - 值位置的裸标识符（文本类型，如 `Age_of_History: Mission`、`Name: 奥地利`）
///   自动加引号；`true`/`false`/`null`/`Infinity`/`NaN` 保持原样；
/// - 数字形态但不是合法数字的记号（如 `1.2.3`）按文本加引号；
/// - 注释（`//`、`/* */`）与其它内容原样保留。
///
/// 本函数只作为解析前的修复步骤，其输出继续交给 JSON5 解析器；
/// 真正写回文件的规范内容由 [`serialize_mission_file`] 生成。
pub fn normalize_loose_json(content: &str) -> String {
    let content = content.trim_start_matches('\u{feff}');
    let chars: Vec<char> = content.chars().collect();
    let mut out = String::with_capacity(content.len() + content.len() / 8);
    let mut stack: Vec<char> = Vec::new();
    let mut last_was_value = false;
    let mut index = 0usize;

    while index < chars.len() {
        let current = chars[index];
        if current.is_whitespace() {
            out.push(current);
            index += 1;
            continue;
        }
        // 注释原样透传（JSON5 解析器本身支持注释）。
        if current == '/' && matches!(chars.get(index + 1), Some('/')) {
            while index < chars.len() && chars[index] != '\n' {
                out.push(chars[index]);
                index += 1;
            }
            continue;
        }
        if current == '/' && matches!(chars.get(index + 1), Some('*')) {
            out.push('/');
            out.push('*');
            index += 2;
            while index < chars.len() {
                if chars[index] == '*' && matches!(chars.get(index + 1), Some('/')) {
                    out.push('*');
                    out.push('/');
                    index += 2;
                    break;
                }
                out.push(chars[index]);
                index += 1;
            }
            continue;
        }

        match current {
            '{' | '[' => {
                if last_was_value {
                    out.push(',');
                }
                stack.push(current);
                last_was_value = false;
                out.push(current);
                index += 1;
            }
            '}' | ']' => {
                stack.pop();
                last_was_value = true;
                out.push(current);
                index += 1;
            }
            ',' => {
                out.push(',');
                last_was_value = false;
                index += 1;
            }
            ':' => {
                out.push(':');
                last_was_value = false;
                index += 1;
            }
            '"' | '\'' => {
                if last_was_value {
                    out.push(',');
                }
                let quote = current;
                out.push(quote);
                index += 1;
                while index < chars.len() {
                    let ch = chars[index];
                    out.push(ch);
                    index += 1;
                    if ch == '\\' {
                        if index < chars.len() {
                            out.push(chars[index]);
                            index += 1;
                        }
                    } else if ch == quote {
                        break;
                    }
                }
                // 对象里的字符串若不是紧跟着冒号，则是普通值而非键。
                let is_key = matches!(stack.last(), Some('{'))
                    && next_significant_char(&chars, index) == Some(':');
                last_was_value = !is_key;
            }
            _ if current.is_ascii_digit()
                || current == '-'
                || current == '+'
                || current == '.' =>
            {
                if last_was_value {
                    out.push(',');
                }
                let mut token = String::new();
                if current == '-' || current == '+' {
                    token.push(current);
                    index += 1;
                }
                while index < chars.len() {
                    let ch = chars[index];
                    if ch.is_ascii_alphanumeric() || ch == '.' {
                        token.push(ch);
                        index += 1;
                    } else if (ch == '+' || ch == '-')
                        && index > 0
                        && matches!(chars.get(index - 1), Some('e' | 'E'))
                    {
                        token.push(ch);
                        index += 1;
                    } else {
                        break;
                    }
                }
                if is_number_token(&token) {
                    out.push_str(&token);
                } else {
                    // 数字形态但不是合法数字（如 `1.2.3`、`2ndDivision`）：按文本处理。
                    out.push('"');
                    out.push_str(&token);
                    out.push('"');
                }
                last_was_value = true;
            }
            _ if current.is_alphabetic() || current == '_' || current == '$' => {
                let start = index;
                while index < chars.len()
                    && (chars[index].is_alphanumeric()
                        || chars[index] == '_'
                        || chars[index] == '$'
                        || chars[index] == '.')
                {
                    index += 1;
                }
                let ident: String = chars[start..index].iter().collect();
                let is_key = matches!(stack.last(), Some('{'))
                    && next_significant_char(&chars, index) == Some(':');
                if last_was_value {
                    out.push(',');
                }
                if is_key && is_valid_unquoted_key(&ident) {
                    out.push_str(&ident);
                    last_was_value = false;
                } else {
                    match ident.to_ascii_lowercase().as_str() {
                        "true" | "false" | "null" => {
                            out.push_str(&ident.to_ascii_lowercase());
                        }
                        "infinity" => out.push_str("Infinity"),
                        "nan" => out.push_str("NaN"),
                        _ => {
                            // 无引号的文本类型值（或含点号的非标识符键）→ 按字符串处理。
                            out.push('"');
                            out.push_str(&ident);
                            out.push('"');
                            last_was_value = !is_key;
                            continue;
                        }
                    }
                    last_was_value = true;
                }
            }
            _ => {
                out.push(current);
                index += 1;
            }
        }
    }

    out
}

/// 判断宽松语法的数字记号是否是 JSON5 合法数字
/// （支持正负号、小数、指数、`Infinity`/`NaN`、十六进制）。
fn is_number_token(token: &str) -> bool {
    if token.parse::<f64>().is_ok() {
        return true;
    }
    let digits = token
        .strip_prefix('+')
        .or_else(|| token.strip_prefix('-'))
        .unwrap_or(token);
    (digits.starts_with("0x") || digits.starts_with("0X"))
        && digits.len() > 2
        && digits[2..].chars().all(|ch| ch.is_ascii_hexdigit())
}

/// 无引号键在 JSON5 中必须形如 ES 标识符；含点号的裸键（罕见）需加引号输出。
fn is_valid_unquoted_key(ident: &str) -> bool {
    let mut chars = ident.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_alphabetic() || first == '_' || first == '$')
        && chars.all(|ch| ch.is_alphanumeric() || ch == '_' || ch == '$')
}

/// 从 `index` 起跳过空白与注释，返回下一个有效字符（用于判断标识符/字符串是键还是值）。
fn next_significant_char(chars: &[char], mut index: usize) -> Option<char> {
    while index < chars.len() {
        let current = chars[index];
        if current.is_whitespace() {
            index += 1;
            continue;
        }
        if current == '/' {
            match chars.get(index + 1) {
                Some('/') => {
                    index += 2;
                    while index < chars.len() && chars[index] != '\n' {
                        index += 1;
                    }
                }
                Some('*') => {
                    index += 2;
                    while index < chars.len() {
                        if chars[index] == '*' && matches!(chars.get(index + 1), Some('/')) {
                            index += 2;
                            break;
                        }
                        index += 1;
                    }
                }
                _ => return Some('/'),
            }
            continue;
        }
        return Some(current);
    }
    None
}

/// 把国策记录序列化为工具自己的规范格式（游戏可直接读回）。
pub fn serialize_mission_file(missions: &[MissionRecord]) -> Result<String, String> {
    let mut content = String::from("{\n\tMission:\n\t[\n");
    for mission in missions {
        let name = serde_json::to_string(&mission.name).map_err(|error| error.to_string())?;
        let image_name =
            serde_json::to_string(&mission.image_name).map_err(|error| error.to_string())?;
        let mission_event =
            serde_json::to_string(&mission.mission_event).map_err(|error| error.to_string())?;
        content.push_str(&format!(
            "\t\t{{\n\t\t\tID: {},\n\t\t\tName: {},\n\t\t\tImageName: {},\n\t\t\tMissionEvent: {},\n\t\t\tTreeColumn: {},\n\t\t\tTreeRow: {},\n\t\t\tRequiredMission: {},\n\t\t\tRequiredMission2: {},\n\t\t\tAI: {},\n\t\t}},\n",
            mission.id,
            name,
            image_name,
            mission_event,
            mission.tree_column,
            mission.tree_row,
            mission.required_mission,
            mission.required_mission2,
            mission.ai,
        ));
    }
    content.push_str("\t],\n\tAge_of_History: Mission\n}\n");
    Ok(content)
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_loose_json, parse_mission_file, parse_mission_file_with_correction,
        serialize_mission_file,
    };

    #[test]
    fn mission_file_round_trips_json5_syntax() {
        let source = r#"{
                Mission: [
                    {
                        ID: 7,
                        Name: "测试国策",
                        ImageName: "测试国策.png",
                        MissionEvent: "测试国策.txt",
                        TreeColumn: 0,
                        TreeRow: 2,
                        RequiredMission: -1,
                        RequiredMission2: -1,
                        AI: 100,
                    },
                ],
                Age_of_History: Mission
            }"#;

        let parsed = parse_mission_file(source).expect("valid mission data");
        assert_eq!(parsed.mission.len(), 1);
        assert_eq!(parsed.mission[0].id, 7);
        assert_eq!(parsed.mission[0].tree_column, 0);

        let serialized = serialize_mission_file(&parsed.mission).expect("serialize mission data");
        assert!(serialized.contains("Age_of_History: Mission"));
        let round_tripped = parse_mission_file(&serialized).expect("valid serialized mission data");
        assert_eq!(round_tripped.mission[0].name, "测试国策");
    }

    /// 复现 atr.json 的宽松写法：数组元素之间缺逗号、`Age_of_History: Mission` 为裸文本值。
    const LOOSE_SOURCE: &str = r#"{
	Mission:
	[
		{
			ID: 0,
			Name: "维持奥匈二元协定",
			ImageName: "维持奥匈二元协定.png",
			
			MissionEvent: "维持奥匈二元协定.txt",
			
			TreeColumn: 9,
			TreeRow: 0,
			
			RequiredMission: -1,
			RequiredMission2: -1,
			
			AI: 100,
        }
        {
			ID: 1,
			Name: "内莱塔尼亚政治",
			ImageName: "内莱塔尼亚政治.png",
			MissionEvent: "内莱塔尼亚政治.txt",
			TreeColumn: 15,
			TreeRow: 1,
			RequiredMission: 0,
			RequiredMission2: -1,
			AI: 100,
        }
	],
	Age_of_History: Mission
}"#;

    #[test]
    fn loose_syntax_is_repaired_with_correction() {
        // 原始宽松文本无法直接交给 JSON5 解析器（元素间缺逗号、裸文本值）。
        assert!(json5::from_str::<super::MissionFile>(LOOSE_SOURCE).is_err());

        let (file, corrected) = parse_mission_file_with_correction(LOOSE_SOURCE)
            .expect("宽松语法应能被自动纠正后解析");
        assert_eq!(file.mission.len(), 2);
        assert_eq!(file.mission[1].name, "内莱塔尼亚政治");

        let corrected = corrected.expect("发生纠正时应附带规范内容");
        assert!(corrected.contains("Age_of_History: Mission"));
        // 规范内容可以直接解析，且不再需要纠正。
        let (again, second_correction) =
            parse_mission_file_with_correction(&corrected).expect("规范内容应可直接解析");
        assert_eq!(again.mission.len(), 2);
        assert!(second_correction.is_none());
    }

    #[test]
    fn loose_json_normalization_handles_commas_bare_text_and_numbers() {
        let source =
            "{\n a: 1\n b: 2\n c: Mission\n d: 1.2.3\n e: True\n // 注释\n f: [1 2 3]\n}";
        let normalized = normalize_loose_json(source);

        #[derive(serde::Deserialize)]
        struct Probe {
            a: i64,
            b: i64,
            c: String,
            d: String,
            e: bool,
            f: Vec<i64>,
        }

        let probe: Probe = json5::from_str(&normalized).expect("规范化结果应可解析");
        assert_eq!(probe.a, 1);
        assert_eq!(probe.b, 2);
        assert_eq!(probe.c, "Mission");
        assert_eq!(probe.d, "1.2.3");
        assert!(probe.e);
        assert_eq!(probe.f, vec![1, 2, 3]);
    }

    /// 本机验证：用真实的宽松国策文件（atr.json）跑一遍自动纠正与再解析。
    /// 默认忽略：`cargo test -p age_civ_mod_tool --lib -- --ignored`
    #[test]
    #[ignore = "依赖本机 A:\\android\\GameCivs\\missions\\atr.json"]
    fn real_loose_mission_file_is_repaired() {
        let path = r"A:\android\GameCivs\missions\atr.json";
        let Ok(content) = std::fs::read_to_string(path) else {
            println!("真实文件不存在，跳过");
            return;
        };
        let (file, corrected) =
            parse_mission_file_with_correction(&content).expect("真实宽松文件应能自动纠正后解析");
        assert!(!file.mission.is_empty());
        let corrected = corrected.expect("宽松文件应产生纠正内容");
        let (again, second_correction) =
            parse_mission_file_with_correction(&corrected).expect("纠正后的内容应可直接解析");
        assert_eq!(again.mission.len(), file.mission.len());
        assert!(second_correction.is_none());
    }
}
