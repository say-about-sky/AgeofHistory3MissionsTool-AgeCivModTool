//! 决议定义文件（`rainfall/rfEvent_decision.json`）的解析、宽松语法纠正与序列化。
//!
//! 决议文件使用游戏宽松 JSON 方言（允许尾随逗号、对象成员之间缺逗号等，
//! 实测由各模组作者手写产生）。本模块负责在解析前把这类文本纠正为规范格式，
//! 并按固定风格重新序列化（键带引号、制表符缩进、每行带尾随逗号——与国策
//! 序列化风格及模组手写格式一致）。未知字段（工具未注册的键）原样保留，
//! 保证「打开一次再保存」不会把未来新增字段抹掉。

use crate::models::{DecisionFile, DecisionGroup};

/// 解析决议定义文本。合法 JSON5 方言直接解析；宽松写法（尾随逗号、
/// 对象间缺逗号等）先经 [`crate::mission_format::normalize_loose_json`]
/// 纠正语法再解析，并附带规范化后的完整文件内容，由调用方写回原文件。
pub fn parse_decision_file_with_correction(
    content: &str,
) -> Result<(DecisionFile, Option<String>), String> {
    match json5::from_str::<DecisionFile>(content) {
        Ok(file) => Ok((file, None)),
        Err(original_error) => {
            let repaired = crate::mission_format::normalize_loose_json(content);
            match json5::from_str::<DecisionFile>(&repaired) {
                Ok(file) => {
                    let corrected = serialize_decision_file(&file.decisions)?;
                    Ok((file, Some(corrected)))
                }
                Err(repair_error) => Err(format!(
                    "决议文件语法错误，自动纠正后仍无法解析：{repair_error}（原始错误：{original_error}）"
                )),
            }
        }
    }
}

/// 序列化决议组列表为规范格式：
/// ```text
/// {
/// 	"decisions": [
/// 		{
/// 			"id": "RUS",
/// 			"name": "政治决策",
/// 			"desc": ["…"],
/// 			"events": ["决议:清洗"],
/// 		},
/// 	]
/// }
/// ```
/// - `id` / `name` 始终写出（空值写成空串）；
/// - `desc` / `images` / `events` 为空时跳过（不额外补空数组）；
/// - 未知字段在已知字段之后原样写出。
pub fn serialize_decision_file(groups: &[DecisionGroup]) -> Result<String, String> {
    let mut content = String::from("{\n\t\"decisions\": [");
    if groups.is_empty() {
        content.push_str("]\n}\n");
        return Ok(content);
    }
    content.push('\n');
    for group in groups {
        let id = serde_json::to_string(&group.id).map_err(|error| error.to_string())?;
        let name = serde_json::to_string(&group.name).map_err(|error| error.to_string())?;
        content.push_str("\t\t{\n");
        content.push_str(&format!("\t\t\t\"id\": {id},\n"));
        content.push_str(&format!("\t\t\t\"name\": {name},\n"));
        push_string_list(&mut content, "desc", &group.desc)?;
        push_string_list(&mut content, "images", &group.images)?;
        push_string_list(&mut content, "events", &group.events)?;
        // 未知字段原样写出（工具未注册的键：未来游戏 / 模组扩展）。
        for (key, value) in &group.extra {
            let key = serde_json::to_string(key).unwrap_or_else(|_| format!("\"{key}\""));
            content.push_str(&format!("\t\t\t{key}: {},\n", render_json_value(value)));
        }
        content.push_str("\t\t},\n");
    }
    content.push_str("\t]\n}\n");
    Ok(content)
}

/// 向规范格式追加一行 `"键": ["值", …],`（列表为空时跳过）。
fn push_string_list(content: &mut String, key: &str, values: &[String]) -> Result<(), String> {
    if values.is_empty() {
        return Ok(());
    }
    let rendered: Vec<String> = values
        .iter()
        .map(|value| serde_json::to_string(value).map_err(|error| error.to_string()))
        .collect::<Result<_, _>>()?;
    content.push_str(&format!("\t\t\t\"{key}\": [{}],\n", rendered.join(", ")));
    Ok(())
}

/// 未知字段值按 JSON 渲染（字符串含引号转义、数组 / 对象原样）。
fn render_json_value(value: &serde_json::Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 模组手写风格：尾随逗号 + 两个对象之间缺逗号（自动纠正后正常解析）。
    #[test]
    fn parses_loose_decision_json() {
        let text = r#"{
	"decisions":[
	{
		"id":"A",
		"name":"甲",
		"desc":["第一段","第二段"],
		"images":["none.png"],
		"events":["e1","e2"],
	}
	{
		"id":"B",
		"name":"乙"
	},
	]
}"#;
        let (file, corrected) = parse_decision_file_with_correction(text).unwrap();
        assert_eq!(file.decisions.len(), 2);
        assert_eq!(file.decisions[0].id, "A");
        assert_eq!(file.decisions[0].desc, vec!["第一段", "第二段"]);
        assert_eq!(file.decisions[0].events, vec!["e1", "e2"]);
        assert_eq!(file.decisions[1].id, "B");
        assert!(corrected.is_some(), "宽松语法应产生纠正内容");
    }

    /// 序列化输出为规范格式：再解析不需要纠正，字段与未知键完整往返。
    #[test]
    fn round_trips_and_preserves_unknown_fields() {
        let mut extra = std::collections::BTreeMap::new();
        extra.insert("customKey".to_string(), serde_json::json!(7));
        extra.insert("扩展键".to_string(), serde_json::json!(["a", "b"]));
        let groups = vec![DecisionGroup {
            id: "X".to_string(),
            name: "实验".to_string(),
            desc: vec!["描述§Y高亮§0".to_string()],
            images: Vec::new(),
            events: vec!["决议:测试".to_string()],
            extra,
        }];
        let text = serialize_decision_file(&groups).unwrap();
        let (file, corrected) = parse_decision_file_with_correction(&text).unwrap();
        assert!(corrected.is_none(), "规范输出不应再触发纠正：{text}");
        let group = &file.decisions[0];
        assert_eq!(group.id, "X");
        assert_eq!(group.name, "实验");
        assert_eq!(group.desc, vec!["描述§Y高亮§0"]);
        assert!(group.images.is_empty());
        assert_eq!(group.events, vec!["决议:测试"]);
        assert_eq!(group.extra.get("customKey"), Some(&serde_json::json!(7)));
        assert_eq!(group.extra.get("扩展键"), Some(&serde_json::json!(["a", "b"])));
    }

    /// 空决议列表（合法内容）序列化 / 解析往返。
    #[test]
    fn empty_decisions_round_trip() {
        let text = serialize_decision_file(&[]).unwrap();
        let (file, corrected) = parse_decision_file_with_correction(&text).unwrap();
        assert!(file.decisions.is_empty());
        assert!(corrected.is_none());
    }
}
