//! missionsEvents/*.txt 行式脚本解析与序列化（无损往返）。
//!
//! 脚本由三部分拼接而成：
//! 1. 必填/可填项目：`key=value` 行；
//! 2. 逻辑块：`trigger_and|trigger_or|trigger_and_not` + 块首
//!    `next_and|next_or|next_and_not` + 条件行 + `trigger_*_end`；
//! 3. 收益块：`option_btn` + `name=` + 效果行 + `option_end`。
//!
//! 设计目标：
//! - 对任意输入**绝不报错**，未知键、拼写错误的键、无 `=` 的行一律保留；
//! - `parse(text) → to_text()` 与原文本逐字节一致（含空行与末尾换行）。

use super::event_schema;

/// 一行 `key=value`（或仅有 key 的结构行）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryLine {
	/// 此行之前紧邻的空行数量（保留排版用）。
	pub blank_before: usize,
	pub key: String,
	/// `None` 表示行内没有 `=`（如结构标记、注释）。
	pub value: Option<String>,
}

impl EntryLine {
	pub fn new(key: &str, value: &str) -> Self {
		Self {
			blank_before: 0,
			key: key.to_string(),
			value: Some(value.to_string()),
		}
	}

	pub fn value_or_default(&self) -> &str {
		self.value.as_deref().unwrap_or("")
	}

	pub fn set_value(&mut self, value: &str) {
		self.value = Some(value.to_string());
	}
}

/// 触发块类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TriggerKind {
	#[default]
	And,
	Or,
	AndNot,
}

impl TriggerKind {
	pub fn open_token(self) -> &'static str {
		match self {
			Self::And => "trigger_and",
			Self::Or => "trigger_or",
			Self::AndNot => "trigger_and_not",
		}
	}

	pub fn close_token(self) -> &'static str {
		match self {
			Self::And => "trigger_and_end",
			Self::Or => "trigger_or_end",
			Self::AndNot => "trigger_and_not_end",
		}
	}

	pub fn from_open_token(token: &str) -> Self {
		match token {
			"trigger_or" => Self::Or,
			"trigger_and_not" => Self::AndNot,
			_ => Self::And,
		}
	}
}

/// 块首连接操作符。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NextOp {
	#[default]
	And,
	Or,
	AndNot,
}

impl NextOp {
	pub fn token(self) -> &'static str {
		match self {
			Self::And => "next_and",
			Self::Or => "next_or",
			Self::AndNot => "next_and_not",
		}
	}

	pub fn from_token(token: &str) -> Option<Self> {
		match token {
			"next_and" => Some(Self::And),
			"next_or" => Some(Self::Or),
			"next_and_not" | "next_not" => Some(Self::AndNot),
			_ => None,
		}
	}
}

/// 一个触发逻辑块。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TriggerBlock {
	/// 块首之前的空行数。
	pub blank_before: usize,
	pub kind: TriggerKind,
	/// `None` 表示源脚本没有写 next_ 行，序列化时保持不写。
	pub join: Option<NextOp>,
	pub conditions: Vec<EntryLine>,
}

/// 一个收益选项块。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OptionBlock {
	/// 块首之前的空行数。
	pub blank_before: usize,
	/// `None` 表示源脚本没有 name 行。
	pub name: Option<String>,
	pub effects: Vec<EntryLine>,
}

/// 解析后的国策事件脚本。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MissionEvent {
	/// 头部的必填/可填项目（保序，含未知键）。
	pub header: Vec<EntryLine>,
	pub triggers: Vec<TriggerBlock>,
	pub options: Vec<OptionBlock>,
	/// 文件末尾的空行数（用于保留末尾换行）。
	pub trailing_blank_lines: usize,
}

/// 便捷 API 目前主要由测试与后续功能（模板生成、字段复制等）使用。
#[allow(dead_code)]
impl MissionEvent {
	pub fn parse(text: &str) -> Self {
		parse(text)
	}

	pub fn to_text(&self) -> String {
		to_text(self)
	}

	/// 解析 → 序列化是否与原文本逐字节一致。
	pub fn round_trip(text: &str) -> bool {
		text == Self::parse(text).to_text()
	}

	/// 读取头部字段值（首个匹配）。
	pub fn get_field(&self, key: &str) -> Option<&str> {
		self.header
			.iter()
			.find(|entry| entry.key == key)
			.and_then(|entry| entry.value.as_deref())
	}

	/// 更新头部字段值；不存在则追加到末尾。
	pub fn set_field(&mut self, key: &str, value: &str) {
		if let Some(entry) = self.header.iter_mut().find(|entry| entry.key == key) {
			entry.value = Some(value.to_string());
		} else {
			self.header.push(EntryLine::new(key, value));
		}
	}

	/// 删除头部字段（首个匹配），返回被删除的行。
	pub fn remove_field(&mut self, key: &str) -> Option<EntryLine> {
		let index = self.header.iter().position(|entry| entry.key == key)?;
		Some(self.header.remove(index))
	}
}

/// 容错解析：未知结构一律原样保留，绝不报错。
pub fn parse(text: &str) -> MissionEvent {
	// 空输入没有行，直接返回，避免 split('\n') 产生的伪空行。
	if text.is_empty() {
		return MissionEvent::default();
	}

	let mut event = MissionEvent::default();
	let mut pending_blanks = 0_usize;
	let mut current_trigger: Option<TriggerBlock> = None;
	let mut current_option: Option<OptionBlock> = None;

	let flush_trigger = |event: &mut MissionEvent, block: Option<TriggerBlock>| {
		if let Some(block) = block {
			event.triggers.push(block);
		}
	};
	let flush_option = |event: &mut MissionEvent, block: Option<OptionBlock>| {
		if let Some(block) = block {
			event.options.push(block);
		}
	};

	for raw in text.split('\n') {
		let line = raw.strip_suffix('\r').unwrap_or(raw);

		// 空行只计数，不落盘，等下一个内容行决定归属。
		if line.trim().is_empty() {
			pending_blanks += 1;
			continue;
		}
		let blank_before = pending_blanks;
		pending_blanks = 0;

		if let Some(block) = current_trigger.as_mut() {
			if is_trigger_close(line) {
				flush_trigger(&mut event, current_trigger.take());
				continue;
			}
			if let Some(op) = NextOp::from_token(line) {
				if block.join.is_none() {
					block.join = Some(op);
				} else {
					// 罕见的重复 next_ 行也保留为条件行，避免丢内容。
					block.conditions.push(EntryLine {
						blank_before,
						key: line.to_string(),
						value: None,
					});
				}
				continue;
			}
			block.conditions.push(parse_entry_line(line, blank_before));
			continue;
		}

		if let Some(block) = current_option.as_mut() {
			if line == "option_end" {
				flush_option(&mut event, current_option.take());
				continue;
			}
			// 首个 name= 作为选项名，之后出现的 name= 保留为效果行。
			if block.name.is_none() {
				if let Some((key, value)) = line.split_once('=') {
					if key == "name" {
						block.name = Some(value.to_string());
						continue;
					}
				}
			}
			block.effects.push(parse_entry_line(line, blank_before));
			continue;
		}

		match trigger_kind_from_open(line) {
			Some(kind) => {
				current_trigger = Some(TriggerBlock {
					blank_before,
					kind,
					join: None,
					conditions: Vec::new(),
				});
			}
			None if line == "option_btn" => {
				current_option = Some(OptionBlock {
					blank_before,
					name: None,
					effects: Vec::new(),
				});
			}
			None => {
				event.header.push(parse_entry_line(line, blank_before));
			}
		}
	}

	// 输入在未闭合块处结束：保留已收集的内容，避免丢数据。
	flush_trigger(&mut event, current_trigger.take());
	flush_option(&mut event, current_option.take());
	event.trailing_blank_lines = pending_blanks;
	event
}

fn parse_entry_line(line: &str, blank_before: usize) -> EntryLine {
	match line.split_once('=') {
		Some((key, value)) => EntryLine {
			blank_before,
			key: key.to_string(),
			value: Some(value.to_string()),
		},
		None => EntryLine {
			blank_before,
			key: line.to_string(),
			value: None,
		},
	}
}

fn trigger_kind_from_open(line: &str) -> Option<TriggerKind> {
	match line {
		"trigger_and" => Some(TriggerKind::And),
		"trigger_or" => Some(TriggerKind::Or),
		"trigger_and_not" => Some(TriggerKind::AndNot),
		_ => None,
	}
}

fn is_trigger_close(line: &str) -> bool {
	matches!(line, "trigger_and_end" | "trigger_or_end" | "trigger_and_not_end")
}

/// 按游戏模板样式序列化；未改动的内容会与原文件逐字节一致。
pub fn to_text(event: &MissionEvent) -> String {
	let mut lines: Vec<String> = Vec::new();

	for entry in &event.header {
		push_entry_line(&mut lines, entry);
	}
	for block in &event.triggers {
		for _ in 0..block.blank_before {
			lines.push(String::new());
		}
		lines.push(block.kind.open_token().to_string());
		if let Some(join) = block.join {
			lines.push(join.token().to_string());
		}
		for entry in &block.conditions {
			push_entry_line(&mut lines, entry);
		}
		lines.push(block.kind.close_token().to_string());
	}
	for block in &event.options {
		for _ in 0..block.blank_before {
			lines.push(String::new());
		}
		lines.push("option_btn".to_string());
		if let Some(name) = &block.name {
			lines.push(format!("name={name}"));
		}
		for entry in &block.effects {
			push_entry_line(&mut lines, entry);
		}
		lines.push("option_end".to_string());
	}

	let mut output = lines.join("\n");
	for _ in 0..event.trailing_blank_lines {
		output.push('\n');
	}
	output
}

fn push_entry_line(lines: &mut Vec<String>, entry: &EntryLine) {
	for _ in 0..entry.blank_before {
		lines.push(String::new());
	}
	match &entry.value {
		Some(value) => lines.push(format!("{}={value}", entry.key)),
		None => lines.push(entry.key.clone()),
	}
}

/// 诊断当前事件脚本：返回 (类型不符数, 未识别键数, 疑似拼写错误数)。
pub fn diagnostics(event: &MissionEvent) -> (usize, usize, usize) {
	let mut invalid = 0_usize;
	let mut unknown = 0_usize;
	let mut typos = 0_usize;
	let mut check = |key: &str, value: &str| {
		match event_schema::lookup(key) {
			Some(spec) => {
				if !event_schema::is_valid_value(spec.spec, value) {
					invalid += 1;
				}
			}
			None => {
				if event_schema::KNOWN_TYPOS.iter().any(|(typo, _)| *typo == key) {
					typos += 1;
				} else {
					unknown += 1;
				}
			}
		}
	};
	for entry in &event.header {
		check(&entry.key, entry.value_or_default());
	}
	for block in &event.triggers {
		for entry in &block.conditions {
			check(&entry.key, entry.value_or_default());
		}
	}
	for block in &event.options {
		for entry in &block.effects {
			check(&entry.key, entry.value_or_default());
		}
	}
	(invalid, unknown, typos)
}

/// 生成新国策的默认事件脚本，排版与游戏内置模板一致。
/// 新建卡片时用 `default_event(标题).to_text()` 写入 `<标题>.txt`。
pub fn default_event(title: &str) -> MissionEvent {
	let mut event = MissionEvent::default();
	event.header.push(EntryLine::new("id", title));
	event.header.push(EntryLine::new("title", title));
	event.header.push(EntryLine::new("desc", ""));
	event.header.push(EntryLine {
		blank_before: 1,
		key: "image".to_string(),
		value: Some("国策通用.png".to_string()),
	});
	event.header.push(EntryLine {
		blank_before: 1,
		key: "show_in_missions".to_string(),
		value: Some("false".to_string()),
	});
	event.header.push(EntryLine::new("mission_image", "tww.png"));
	event.header.push(EntryLine::new("popUp", "true"));
	event.header.push(EntryLine::new("pobssible_to_run", "true"));
	event.header.push(EntryLine::new("no_background", "true"));
	event.header.push(EntryLine::new("no_text", "true"));
	event.header.push(EntryLine::new("focus_dura", "35"));
	event.header.push(EntryLine {
		blank_before: 1,
		key: "only_once".to_string(),
		value: Some("true".to_string()),
	});
	event.triggers.push(TriggerBlock {
		blank_before: 1,
		kind: TriggerKind::And,
		join: Some(NextOp::And),
		conditions: vec![EntryLine::new("civ_capital_unrest_over", "-1")],
	});
	event.options.push(OptionBlock {
		blank_before: 1,
		name: Some(String::new()),
		effects: vec![
			EntryLine::new("legacy", "100"),
			EntryLine::new("province_unrest_all", "-99"),
			EntryLine::new("ai", "50"),
		],
	});
	event
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::app::components::event_schema as schema;

	/// 与真实文件同构的样例（重视工业.txt 简化版）。
	const SAMPLE: &str = "id=重视工业\n\
	title=重视工业\n\
	desc=国策所需时长:70日左右,前置国策:无,收益效果:100政治点\n\
	\n\
	image=国策通用.png\n\
	\n\
	show_in_missions=false\n\
	mission_image=tww.png\n\
	popUp=true\n\
	pobssible_to_run=true\n\
	no_background=true\n\
	focus_dura=70\n\
	no_text=true\n\
	\n\
	only_once=true\n\
	\n\
	trigger_and\n\
	next_and\n\
	civ_capital_unrest_over=-1\n\
	trigger_and_end\n\
	\n\
	option_btn\n\
	name=\n\
	legacy=100\n\
	province_unrest_all=-99\n\
	ai=50\n\
	option_end";

	#[test]
	fn round_trip_sample() {
		let event = MissionEvent::parse(SAMPLE);
		assert_eq!(event.to_text(), SAMPLE);
	}

	#[test]
	fn parses_structure() {
		let event = MissionEvent::parse(SAMPLE);
		assert_eq!(event.get_field("id"), Some("重视工业"));
		assert_eq!(event.get_field("desc").unwrap().contains("70日"), true);
		assert_eq!(event.header.len(), 12);
		assert_eq!(event.triggers.len(), 1);
		assert_eq!(event.triggers[0].kind, TriggerKind::And);
		assert_eq!(event.triggers[0].join, Some(NextOp::And));
		assert_eq!(event.triggers[0].conditions.len(), 1);
		assert_eq!(event.triggers[0].conditions[0].key, "civ_capital_unrest_over");
		assert_eq!(event.options.len(), 1);
		assert_eq!(event.options[0].name.as_deref(), Some(""));
		assert_eq!(event.options[0].effects.len(), 3);
		assert_eq!(event.options[0].effects[2].key, "ai");
	}

	#[test]
	fn preserves_unknown_keys_and_typos() {
		let text = "id=测试\n\
		leagcy=20\n\
		province_is_capital=6881\n\
		\n\
		trigger_and\n\
		next_and\n\
		has_variable_not=推崇罗马文化\n\
		trigger_and_end\n\
		\n\
		option_btn\n\
		name=\n\
		leagcy=30\n\
		option_end";
		let event = MissionEvent::parse(text);
		assert_eq!(event.to_text(), text);
		assert_eq!(event.get_field("leagcy"), Some("20"));
	}

	#[test]
	fn composite_values_keep_equals() {
		let text = "id=宣战\n\
		\n\
		trigger_and\n\
		next_and\n\
		civs_are_at_war=fra=ger\n\
		trigger_and_end\n\
		\n\
		option_btn\n\
		name=\n\
		declare_war2=bra=ger\n\
		set_counter=$现代化点数=$现代化点数+35\n\
		option_end";
		let event = MissionEvent::parse(text);
		assert_eq!(event.to_text(), text);
		assert_eq!(
			event.triggers[0].conditions[0].value_or_default(),
			"fra=ger"
		);
	}

	#[test]
	fn preserves_blank_lines_and_trailing_newlines() {
		let text = "\n\na=1\n\n\nb=2\n\n";
		assert!(MissionEvent::round_trip(text));
		assert!(MissionEvent::round_trip("a=1\n"));
		assert!(MissionEvent::round_trip("a=1\n\n"));
		assert!(MissionEvent::round_trip(""));
	}

	#[test]
	fn crlf_is_normalized_to_lf() {
		assert_eq!(
			MissionEvent::parse("a=1\r\nb=2\r\n").to_text(),
			"a=1\nb=2\n"
		);
	}

	#[test]
	fn multiple_blocks_with_join_variants() {
		let text = "id=多块\n\
		\n\
		trigger_and\n\
		next_and\n\
		has_variable=a\n\
		trigger_and_end\n\
		\n\
		trigger_or\n\
		next_or\n\
		has_variable=b\n\
		trigger_or_end\n\
		\n\
		option_btn\n\
		name=选项A\n\
		legacy=10\n\
		option_end\n\
		\n\
		option_btn\n\
		name=选项B\n\
		gold=20\n\
		option_end";
		let event = MissionEvent::parse(text);
		assert_eq!(event.to_text(), text);
		assert_eq!(event.triggers.len(), 2);
		assert_eq!(event.triggers[1].kind, TriggerKind::Or);
		assert_eq!(event.triggers[1].join, Some(NextOp::Or));
		assert_eq!(event.options.len(), 2);
		assert_eq!(event.options[1].name.as_deref(), Some("选项B"));
	}

	#[test]
	fn missing_join_or_name_stays_missing() {
		let text = "id=极简\n\
		\n\
		trigger_and\n\
		has_variable=a\n\
		trigger_and_end\n\
		\n\
		option_btn\n\
		legacy=1\n\
		option_end";
		let event = MissionEvent::parse(text);
		assert_eq!(event.triggers[0].join, None);
		assert_eq!(event.options[0].name, None);
		assert_eq!(event.to_text(), text);
	}

	#[test]
	fn set_and_remove_field() {
		let mut event = MissionEvent::parse("id=a\ntitle=b\n");
		event.set_field("title", "新标题");
		event.set_field("desc", "新增描述");
		assert_eq!(event.to_text(), "id=a\ntitle=新标题\ndesc=新增描述\n");
		assert!(event.remove_field("title").is_some());
		assert_eq!(event.get_field("title"), None);
	}

	#[test]
	fn schema_validates_sample_values() {
		let event = MissionEvent::parse(SAMPLE);
		assert!(schema::is_valid_value(
			schema::lookup("focus_dura").unwrap().spec,
			event.get_field("focus_dura").unwrap()
		));
		let condition = &event.triggers[0].conditions[0];
		assert!(schema::is_valid_value(
			schema::lookup(&condition.key).unwrap().spec,
			condition.value_or_default()
		));
	}

	#[test]
	fn default_event_matches_template_and_round_trips() {
		let text = default_event("新卡片").to_text();
		assert!(text.starts_with("id=新卡片\ntitle=新卡片\ndesc=\n\nimage=国策通用.png\n"));
		assert!(text.contains(
			"\nonly_once=true\n\ntrigger_and\nnext_and\nciv_capital_unrest_over=-1\ntrigger_and_end\n"
		));
		assert!(text.ends_with(
			"\noption_btn\nname=\nlegacy=100\nprovince_unrest_all=-99\nai=50\noption_end"
		));
		assert!(MissionEvent::round_trip(&text));
	}

	#[test]
	fn diagnostics_counts_issues() {
		let text = "id=测试\nleagcy=20\nfocus_dura=abc\n\noption_btn\nname=\nlegacy=1\noption_end";
		let event = MissionEvent::parse(text);
		let (invalid, unknown, typos) = diagnostics(&event);
		assert_eq!(invalid, 1); // focus_dura=abc
		assert_eq!(unknown, 0);
		assert_eq!(typos, 1); // leagcy
		assert_eq!(diagnostics(&MissionEvent::default()), (0, 0, 0));
	}

	/// 用真实的 GameCivs 工作目录做逐字节往返校验。
	/// 仅在显式执行时运行：`cargo test -p age-civ-mod-tool-ui -- --ignored`
	#[test]
	#[ignore = "需要真实数据目录 A:\\android\\GameCivs\\missions\\missionsEvents"]
	fn round_trip_real_files() {
		let directory = r"A:\android\GameCivs\missions\missionsEvents";
		let paths: Vec<_> = std::fs::read_dir(directory)
			.expect("找不到真实数据目录，请先准备 GameCivs 工作区")
			.filter_map(Result::ok)
			.map(|entry| entry.path())
			.filter(|path| {
				path.extension()
					.is_some_and(|extension| extension.eq_ignore_ascii_case("txt"))
			})
			.collect();
		assert!(!paths.is_empty(), "数据目录中没有 txt 文件");

		let mut failures = Vec::new();
		let mut total = 0_usize;
		for path in paths {
			total += 1;
			let text = std::fs::read_to_string(&path).expect("读取文件失败");
			let event = MissionEvent::parse(&text);
			let output = event.to_text();
			if output != text {
				failures.push(format!(
					"{}：往返不一致（原 {} 字节，输出 {} 字节）",
					path.display(),
					text.len(),
					output.len()
				));
			}
		}
		assert!(
			failures.is_empty(),
			"{total} 个文件中有 {} 个往返不一致：\n{}",
			failures.len(),
			failures.join("\n")
		);
	}

	/// 报告 Schema 注册表对真实数据的覆盖度（仅打印，不判失败）。
	/// 仅在显式执行时运行：`cargo test -p age-civ-mod-tool-ui -- --ignored`
	#[test]
	#[ignore = "需要真实数据目录 A:\\android\\GameCivs\\missions\\missionsEvents"]
	fn schema_coverage_report() {
		let directory = r"A:\android\GameCivs\missions\missionsEvents";
		let paths: Vec<_> = std::fs::read_dir(directory)
			.expect("找不到真实数据目录，请先准备 GameCivs 工作区")
			.filter_map(Result::ok)
			.map(|entry| entry.path())
			.filter(|path| {
				path.extension()
					.is_some_and(|extension| extension.eq_ignore_ascii_case("txt"))
			})
			.collect();

		let mut unknown: std::collections::BTreeMap<String, usize> =
			std::collections::BTreeMap::new();
		let mut invalid: Vec<String> = Vec::new();
		for path in paths {
			let text = std::fs::read_to_string(&path).expect("读取文件失败");
			let event = MissionEvent::parse(&text);
			let name = path.file_name().unwrap_or_default().to_string_lossy();
			let mut all_entries: Vec<(String, String)> = event
				.header
				.iter()
				.map(|entry| (entry.key.clone(), entry.value_or_default().to_string()))
				.collect();
			for block in &event.triggers {
				all_entries.extend(
					block
						.conditions
						.iter()
						.map(|entry| (entry.key.clone(), entry.value_or_default().to_string())),
				);
			}
			for block in &event.options {
				all_entries.extend(
					block
						.effects
						.iter()
						.map(|entry| (entry.key.clone(), entry.value_or_default().to_string())),
				);
			}
			for (key, value) in all_entries {
				match schema::lookup(&key) {
					Some(spec) => {
						if !schema::is_valid_value(spec.spec, &value) {
							invalid.push(format!("{name}: {key}={value}"));
						}
					}
					None => {
						*unknown.entry(key).or_insert(0) += 1;
					}
				}
			}
		}

		println!("== Schema 覆盖度报告 ==");
		if unknown.is_empty() {
			println!("未知键：无");
		} else {
			println!("未知键（解析器会原样保留）：");
			for (key, count) in unknown {
				println!("  {key} × {count}");
			}
		}
		if invalid.is_empty() {
			println!("类型校验失败：无");
		} else {
			println!("类型校验失败（{} 处）：", invalid.len());
			for line in invalid {
				println!("  {line}");
			}
		}
	}
}
