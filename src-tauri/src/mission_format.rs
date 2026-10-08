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
    // 每个对象层级是否已出现过 `ID` 键（与 stack 平行）：
    // 同一对象里再次出现 `ID` = 作者漏写了条目分隔（`},` + `{`），需拆分为两个条目。
    let mut id_seen: Vec<bool> = Vec::new();
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
                id_seen.push(false);
                last_was_value = false;
                out.push(current);
                index += 1;
            }
            '}' | ']' => {
                stack.pop();
                id_seen.pop();
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
                // 先把整段字符串读入缓冲区（确定是键后再按顺序写出，
                // 以便在「重复 ID 键」处插入条目分隔）。
                let quote = current;
                let mut text = String::new();
                index += 1;
                while index < chars.len() {
                    let ch = chars[index];
                    index += 1;
                    if ch == '\\' {
                        text.push(ch);
                        if index < chars.len() {
                            text.push(chars[index]);
                            index += 1;
                        }
                    } else if ch == quote {
                        break;
                    } else {
                        text.push(ch);
                    }
                }
                // 对象里的字符串若不是紧跟着冒号，则是普通值而非键。
                let is_key = matches!(stack.last(), Some('{'))
                    && next_significant_char(&chars, index) == Some(':');
                if last_was_value {
                    out.push(',');
                }
                if is_key && text.eq_ignore_ascii_case("ID") {
                    push_split_if_duplicate_id(&mut out, &mut id_seen);
                }
                out.push(quote);
                out.push_str(&text);
                out.push(quote);
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
                    if ident.eq_ignore_ascii_case("ID") {
                        push_split_if_duplicate_id(&mut out, &mut id_seen);
                    }
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

/// 拆分「漏写条目分隔」的对象：同一对象里出现第二个 `ID` 键时，在键前插入
/// `},\n{` 关闭旧对象并开启新对象（工具随后按两个独立条目解析）。
/// 仅当字符串处于对象键位置时调用；写入后把当前层标记为「新对象已有 ID」。
fn push_split_if_duplicate_id(out: &mut String, id_seen: &mut Vec<bool>) {
    if *id_seen.last().unwrap_or(&false) {
        out.push_str("},\n{");
    }
    if let Some(seen) = id_seen.last_mut() {
        *seen = true;
    }
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
///
/// 前置国策字段按原样保留：标量写法（`RequiredMission` / `RequiredMission2`）只为
/// 原本就有该字段的记录写出；列表写法（`RequiredMissions` / `RequiredMissionsOR` /
/// `RequiredMissionsOR2` / `RequiredMissionsOR3` / `MutuallyExclusiveMissions`）
/// 只在存在时写出——列表写法的条目不会被额外补上 `-1` 标量，避免改变引擎对
/// 两种写法的解读（白日升等模组的文件里两种写法混用）。
///
/// 未知字段（[`MissionRecord::extra`]，未来游戏 / 模组新增的键）逐条原样写出，
/// 保证「打开一次再保存」不会把工具还不认识的字段抹掉。
pub fn serialize_mission_file(missions: &[MissionRecord]) -> Result<String, String> {
    let mut content = String::from("{\n\tMission:\n\t[\n");
    for mission in missions {
        let name = serde_json::to_string(&mission.name).map_err(|error| error.to_string())?;
        let image_name =
            serde_json::to_string(&mission.image_name).map_err(|error| error.to_string())?;
        let mission_event =
            serde_json::to_string(&mission.mission_event).map_err(|error| error.to_string())?;
        content.push_str(&format!(
            "\t\t{{\n\t\t\tID: {},\n\t\t\tName: {},\n\t\t\tImageName: {},\n\t\t\tMissionEvent: {},\n\t\t\tTreeColumn: {},\n\t\t\tTreeRow: {},\n",
            mission.id, name, image_name, mission_event, mission.tree_column, mission.tree_row,
        ));
        if let Some(value) = mission.required_mission {
            content.push_str(&format!("\t\t\tRequiredMission: {value},\n"));
        }
        if let Some(value) = mission.required_mission2 {
            content.push_str(&format!("\t\t\tRequiredMission2: {value},\n"));
        }
        push_id_list(&mut content, "RequiredMissions", &mission.required_missions);
        push_id_list(
            &mut content,
            "RequiredMissionsOR",
            &mission.required_missions_or,
        );
        push_id_list(
            &mut content,
            "RequiredMissionsOR2",
            &mission.required_missions_or2,
        );
        push_id_list(
            &mut content,
            "RequiredMissionsOR3",
            &mission.required_missions_or3,
        );
        push_id_list(
            &mut content,
            "MutuallyExclusiveMissions",
            &mission.mutually_exclusive_missions,
        );
        // 未知字段原样写出（工具未注册的键：未来游戏 / 模组扩展）。
        for (key, value) in &mission.extra {
            content.push_str(&format!(
                "\t\t\t{}: {},\n",
                render_extra_key(key),
                render_extra_value(value)
            ));
        }
        content.push_str(&format!("\t\t\tAI: {},\n\t\t}},\n", mission.ai));
    }
    content.push_str("\t],\n\tAge_of_History: Mission\n}\n");
    Ok(content)
}

/// 未知字段键名：常规标识符原样写出，含特殊字符时按 JSON 字符串转义。
fn render_extra_key(key: &str) -> String {
    let is_identifier = !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_');
    if is_identifier {
        key.to_string()
    } else {
        serde_json::to_string(key).unwrap_or_else(|_| format!("\"{key}\""))
    }
}

/// 未知字段值按 JSON 渲染（数组内逗号后补空格，与规范格式其余部分保持一致）。
fn render_extra_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Array(items) => {
            let parts: Vec<String> = items.iter().map(render_extra_value).collect();
            format!("[{}]", parts.join(", "))
        }
        other => serde_json::to_string(other).unwrap_or_else(|_| "null".to_string()),
    }
}

/// 向规范格式追加一行 `键: [ID, ID, …],`（值为 `None` 或空列表时跳过）。
fn push_id_list(content: &mut String, key: &str, values: &Option<Vec<i64>>) {
    let Some(values) = values else {
        return;
    };
    if values.is_empty() {
        return;
    }
    let list = values
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    content.push_str(&format!("\t\t\t{key}: [{list}],\n"));
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

    /// 工具未注册的未知字段（未来游戏 / 模组新增的键）解析后保留、序列化时原样写出，
    /// 「打开一次再保存」不得把新字段抹掉。
    #[test]
    fn mission_file_preserves_unknown_fields() {
        let source = r#"{
	Mission:
	[
		{
			ID: 0,
			Name: "甲",
			ImageName: "甲.png",
			MissionEvent: "甲.txt",
			TreeColumn: 0,
			TreeRow: 0,
			RequiredMission: -1,
			AI: 80,
			NewRequirement: [1, 2, 3],
			FutureFlag: true,
			ExtraNote: "自定义文本",
		},
	],
	Age_of_History: Mission
}"#;
        let (file, _correction) = parse_mission_file_with_correction(source).expect("应能解析");
        let record = &file.mission[0];
        assert_eq!(
            record.extra.get("FutureFlag"),
            Some(&serde_json::Value::Bool(true))
        );
        assert_eq!(
            record.extra.get("NewRequirement"),
            Some(&serde_json::json!([1, 2, 3]))
        );

        let rendered = serialize_mission_file(&file.mission).expect("应能序列化");
        assert!(rendered.contains("\t\t\tNewRequirement: [1, 2, 3],\n"));
        assert!(rendered.contains("\t\t\tFutureFlag: true,\n"));
        assert!(rendered.contains("\t\t\tExtraNote: \"自定义文本\",\n"));

        // 再次解析：未知字段内容一致，且规范格式无需二次纠正。
        let (again, second_correction) =
            parse_mission_file_with_correction(&rendered).expect("规范格式应可直接解析");
        assert!(second_correction.is_none());
        assert_eq!(again.mission[0].extra, record.extra);
        assert_eq!(again.mission[0].ai, 80);
    }

    /// 白日升等模组的「列表写法」前置国策：`RequiredMissions` / `RequiredMissionsOR`
    /// （含 OR2 / OR3 两组）/ `MutuallyExclusiveMissions`——缺少标量字段时不应报
    /// missing field，且两种写法在规范化写回时都按原样保留。
    #[test]
    fn mission_file_supports_required_mission_lists() {
        let source = r#"{
	Mission:
	[
		{
			ID: 0,
			Name: "抓住机会",
			ImageName: "抓住机会.png",
			MissionEvent: "抓住机会.txt",
			TreeColumn: 3,
			TreeRow: 0,
			RequiredMission: -1,
			RequiredMission2: -1,
			AI: 5,
		}
		{
			ID: 1,
			Name: "发布讨逆檄文",
			ImageName: "发布讨逆檄文.png",
			MissionEvent: "发布讨逆檄文.txt",
			TreeColumn: 3,
			TreeRow: 3,
			RequiredMissions: [2, 3, 4, 5],
			RequiredMissionsOR: [99, 101],
			RequiredMissionsOR2: [108, 110],
			RequiredMissionsOR3: [115, 116],
			MutuallyExclusiveMissions: [6, 10],
			AI: 5,
		}
	],
	Age_of_History: Mission
}"#;

        let (file, corrected) = parse_mission_file_with_correction(source)
            .expect("列表写法应能解析（缺标量字段时按未写处理）");
        assert_eq!(file.mission.len(), 2);
        // 标量字段缺失 → None（而不是 missing field 报错）。
        assert_eq!(file.mission[1].required_mission, None);
        assert_eq!(file.mission[1].required_mission2, None);
        assert_eq!(
            file.mission[1].required_missions.as_deref(),
            Some(&[2, 3, 4, 5][..])
        );
        assert_eq!(
            file.mission[1].required_missions_or.as_deref(),
            Some(&[99, 101][..])
        );
        assert_eq!(
            file.mission[1].required_missions_or2.as_deref(),
            Some(&[108, 110][..])
        );
        assert_eq!(
            file.mission[1].required_missions_or3.as_deref(),
            Some(&[115, 116][..])
        );
        assert_eq!(
            file.mission[1].mutually_exclusive_missions.as_deref(),
            Some(&[6, 10][..])
        );

        // 原始文本元素间缺逗号（宽松写法）→ 产生规范内容：
        // 标量写法条目保留标量字段；列表写法条目保持原样，不被补上 -1 标量。
        let corrected = corrected.expect("宽松写法应产生纠正内容");
        assert_eq!(corrected.matches("RequiredMission:").count(), 1);
        assert_eq!(corrected.matches("RequiredMission2:").count(), 1);
        assert!(corrected.contains("RequiredMissions: [2, 3, 4, 5],"));
        assert!(corrected.contains("RequiredMissionsOR: [99, 101],"));
        assert!(corrected.contains("RequiredMissionsOR2: [108, 110],"));
        assert!(corrected.contains("RequiredMissionsOR3: [115, 116],"));
        assert!(corrected.contains("MutuallyExclusiveMissions: [6, 10],"));

        // 规范内容可直接解析（无第二次纠正），列表字段等值。
        let (again, second_correction) =
            parse_mission_file_with_correction(&corrected).expect("规范内容应可直接解析");
        assert!(second_correction.is_none());
        assert_eq!(again.mission[0].required_mission, Some(-1));
        assert_eq!(
            again.mission[1].required_missions,
            file.mission[1].required_missions
        );
        assert_eq!(
            again.mission[1].mutually_exclusive_missions,
            file.mission[1].mutually_exclusive_missions
        );
    }

    /// 复现 road_to_56/ger.json 的写法：作者漏写条目分隔（`},` + `{`），两条国策
    /// 合并进同一个对象（对象内出现重复 `ID`），且其中一条缺 `AI`。
    /// 自动纠正应把对象拆回两条，缺省字段按默认值读取。
    #[test]
    fn mission_file_splits_merged_entries_and_defaults_missing_fields() {
        let source = r#"{
	Mission:
	[
		{
			ID: 15,
			Name: "创新战争",
			ImageName: "创新战争.png",
			MissionEvent: "创新战争.txt",
			TreeColumn: 4,
			TreeRow: 9,
			RequiredMission: 10,
			RequiredMission2: -1,
			AI: 5,
			
			
			ID: 18,
			Name: "对法作战",
			ImageName: "对法作战.png",
			MissionEvent: "对法作战.txt",
			TreeColumn: 4,
			TreeRow: 10,
			RequiredMission: 8,
			RequiredMission2: -1,
		},
	],
	Age_of_History: Mission
}"#;

        let (file, corrected) =
            parse_mission_file_with_correction(source).expect("合并条目应能自动拆分后解析");
        assert_eq!(file.mission.len(), 2);
        assert_eq!(file.mission[0].id, 15);
        assert_eq!(file.mission[0].name, "创新战争");
        assert_eq!(file.mission[0].ai, 5);
        assert_eq!(file.mission[1].id, 18);
        assert_eq!(file.mission[1].name, "对法作战");
        assert_eq!(file.mission[1].required_mission, Some(8));
        // 缺 `AI` 字段的条目按默认权重 100 读取。
        assert_eq!(file.mission[1].ai, 100);

        let corrected = corrected.expect("合并写法应产生纠正内容");
        assert!(corrected.contains("创新战争"));
        assert!(corrected.contains("对法作战"));
        // 规范内容里两个条目各自独立。
        assert_eq!(corrected.matches("ID: 15").count(), 1);
        assert_eq!(corrected.matches("ID: 18").count(), 1);

        let (again, second_correction) =
            parse_mission_file_with_correction(&corrected).expect("规范内容应可直接解析");
        assert!(second_correction.is_none());
        assert_eq!(again.mission.len(), 2);
        assert_eq!(again.mission[1].ai, 100);

        // 带引号键的合并写法（另有模组整体使用引号键）同样能拆分；
        // 缺省的基础字段按空串 / 0 / 100 读取。
        let quoted = r#"{ "Mission": [ { "ID": 0, "Name": "甲", "AI": 5, "ID": 1, "Name": "乙" } ], "Age_of_History": "Mission" }"#;
        let (file, _) = parse_mission_file_with_correction(quoted).expect("引号键合并写法应能拆分");
        assert_eq!(file.mission.len(), 2);
        assert_eq!(file.mission[1].id, 1);
        assert_eq!(file.mission[1].name, "乙");
        assert_eq!(file.mission[1].tree_column, 0);
        assert_eq!(file.mission[1].ai, 100);
    }

    /// 本机验证：解析三个模组（白日升 / road_to_56 / 暮色黄昏）的全部国策目录，
    /// 覆盖列表写法、合并条目、缺省字段等真实写法，并验证规范格式往返稳定。
    /// 默认忽略：`cargo test -p age_civ_mod_tool --lib -- --ignored`
    #[test]
    #[ignore = "依赖本机 A:\\android\\GameCivs 下的模组目录"]
    fn real_modded_mission_trees_parse_and_round_trip() {
        let dirs = [
            r"A:\android\GameCivs\1566AuroraPrever2\assets\map\Earth3\scenarios\ming\missions",
            r"A:\android\GameCivs\europe\assets\map\ES\scenarios\RusUkrWar\missions",
            r"A:\android\GameCivs\europe\assets\map\ES\scenarios\Ukr2014\missions",
            r"A:\android\GameCivs\白日升\assets\map\Begonia\scenarios\RWS\missions",
            r"A:\android\GameCivs\road_to_56\assets\map\Earth3\scenarios\WW2\missions",
            r"A:\android\GameCivs\暮色黄昏_世界大战0.25.1\assets\map\Earth3\scenarios\TheGreatWar\missions",
            r"A:\android\GameCivs\1566AuroraPrever2\assets\game\missions",
            r"A:\android\GameCivs\europe\assets\game\missions",
            r"A:\android\GameCivs\白日升\assets\game\missions",
            r"A:\android\GameCivs\road_to_56\assets\game\missions",
            r"A:\android\GameCivs\暮色黄昏_世界大战0.25.1\assets\game\missions",
        ];
        let mut files = 0_usize;
        let mut missions_total = 0_usize;
        for dir in dirs {
            let Ok(entries) = std::fs::read_dir(dir) else {
                println!("目录不存在，跳过：{dir}");
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let is_json = path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("json"));
                if !is_json {
                    continue;
                }
                let content = std::fs::read_to_string(&path).unwrap();
                let (file, _) = parse_mission_file_with_correction(&content)
                    .unwrap_or_else(|error| panic!("{}：{error}", path.display()));
                // 空树（`Mission: []`）是合法内容，不要求条目数 > 0。
                let canonical = serialize_mission_file(&file.mission).unwrap();
                let (again, second_correction) = parse_mission_file_with_correction(&canonical)
                    .unwrap_or_else(|error| panic!("{}（规范化后）：{error}", path.display()));
                assert!(second_correction.is_none(), "{}", path.display());
                assert_eq!(again.mission.len(), file.mission.len(), "{}", path.display());
                files += 1;
                missions_total += file.mission.len();
            }
        }
        assert!(files >= 70, "解析文件过少：{files}");
        assert!(missions_total >= 1200, "国策条目过少：{missions_total}");

        // road_to_56/ger.json：漏写条目分隔的合并写法应拆回两条，缺 AI 的按 100 读取。
        let ger = std::fs::read_to_string(
            r"A:\android\GameCivs\road_to_56\assets\map\Earth3\scenarios\WW2\missions\ger.json",
        )
        .unwrap();
        let (file, _) = parse_mission_file_with_correction(&ger).unwrap();
        assert_eq!(file.mission.len(), 20, "ger.json 拆分后应为 20 条");
        assert!(file
            .mission
            .iter()
            .any(|mission| mission.id == 15 && mission.name == "创新战争"));
        let war = file
            .mission
            .iter()
            .find(|mission| mission.name == "对法作战")
            .unwrap();
        assert_eq!(war.id, 18);
        assert_eq!(war.ai, 100);
        assert_eq!(war.required_mission, Some(8));

        // 白日升 CFT.json：OR 组写法（含 OR2 / OR3）字段在解析后保持原值。
        let cft = std::fs::read_to_string(
            r"A:\android\GameCivs\白日升\assets\map\Begonia\scenarios\RWS\missions\CFT.json",
        )
        .unwrap();
        let (file, _) = parse_mission_file_with_correction(&cft).unwrap();
        let entry = file
            .mission
            .iter()
            .find(|mission| mission.required_missions_or2.is_some())
            .expect("CFT.json 应含 RequiredMissionsOR2 写法");
        assert_eq!(entry.required_missions_or.as_deref(), Some(&[99, 101][..]));
        assert_eq!(entry.required_missions_or2.as_deref(), Some(&[108, 110][..]));
        assert_eq!(entry.required_missions_or3.as_deref(), Some(&[115, 116][..]));
    }
}
