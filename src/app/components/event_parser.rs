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
	/// next_ 行位于「第几个条件之前」（原文件里常见先写若干条件再写 next_，
	/// 序列化时按原位置还原）。
	pub join_after: usize,
	/// next_ 行之前的空行数。
	pub join_blank_before: usize,
	/// next_ 行的原始写法（如 `next_not` 是 `next_and_not` 的别名；
	/// 与标准 token 不同的写法原样保留，用户改动后清空）。
	pub join_token: Option<String>,
	/// 块结束行之前的空行数。
	pub close_blank_before: usize,
	/// 块结束行：`None` = 与类型一致（按类型输出）；`Some("")` = 原文件未写结束行；
	/// `Some(token)` = 原文件写的是非本类型的结束行（个别模组手误，原样保留）。
	pub close: Option<String>,
	pub conditions: Vec<EntryLine>,
}

/// 一个收益选项块。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OptionBlock {
	/// 块首之前的空行数。
	pub blank_before: usize,
	/// `None` 表示源脚本没有 name 行。
	pub name: Option<String>,
	/// `name=` 行位于「第几个效果之前」（原文件里 name 可能写在若干效果之后）。
	pub name_after: usize,
	/// `name=` 行之前的空行数。
	pub name_blank_before: usize,
	/// 块结束行之前的空行数。
	pub close_blank_before: usize,
	/// 块结束行：`None` = 按规范写 `option_end`；`Some("")` = 原文件未写结束行。
	pub close: Option<String>,
	pub effects: Vec<EntryLine>,
}

/// 顶层内容项（保留 header / 触发块 / 选项块在原文件里的交错顺序；
/// 个别模组把内容写在块之后，序列化时按表还原）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopItem {
	Header(usize),
	Trigger(usize),
	Option(usize),
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
	/// 原文件使用 CRLF（Windows 风格）换行；序列化时保持相同换行符。
	pub crlf: bool,
	/// 顶层内容顺序；常规排版（header → trigger → option）为空，此时按类型输出。
	pub sequence: Vec<TopItem>,
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
	// 文件整体用 CRLF 时保持原样输出（Windows 风格的模组文件）。
	event.crlf = text.contains("\r\n");
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
		// 注意：只有「完全为空」的行算空行；仅含空格 / 制表符的行按内容原样保留
		//（个别模组用它做排版，丢了会破坏「未改动即逐字节一致」的往返）。
		if line.is_empty() {
			pending_blanks += 1;
			continue;
		}
		let blank_before = pending_blanks;
		pending_blanks = 0;

		if let Some(block) = current_trigger.as_mut() {
			if is_trigger_close(line) {
				// 手误写错结束行（如 trigger_and 块用 trigger_or_end 收尾）时原样保留。
				if line != block.kind.close_token() {
					block.close = Some(line.to_string());
				}
				block.close_blank_before = blank_before;
				flush_trigger(&mut event, current_trigger.take());
				continue;
			}
			if let Some(op) = NextOp::from_token(line) {
				if block.join.is_none() {
					block.join = Some(op);
					// 记录 next_ 行位置与原始写法：原文件里可能先写若干条件再写 next_，
					//且可能是别名写法（如 `next_not`），序列化时按原文还原。
					block.join_after = block.conditions.len();
					block.join_blank_before = blank_before;
					if line != op.token() {
						block.join_token = Some(line.to_string());
					}
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
				block.close_blank_before = blank_before;
				flush_option(&mut event, current_option.take());
				continue;
			}
			// 首个 name= 作为选项名，之后出现的 name= 保留为效果行。
			if block.name.is_none() {
				if let Some((key, value)) = line.split_once('=') {
					if key == "name" {
						block.name = Some(value.to_string());
						// 记录 name 行位置（原文件里可能先写效果再写 name=）。
						block.name_after = block.effects.len();
						block.name_blank_before = blank_before;
						continue;
					}
				}
			}
			block.effects.push(parse_entry_line(line, blank_before));
			continue;
		}

		match trigger_kind_from_open(line) {
			Some(kind) => {
				event.sequence.push(TopItem::Trigger(event.triggers.len()));
				current_trigger = Some(TriggerBlock {
					blank_before,
					kind,
					join: None,
					join_after: 0,
					join_blank_before: 0,
					join_token: None,
					close_blank_before: 0,
					close: None,
					conditions: Vec::new(),
				});
			}
			None if line == "option_btn" => {
				event.sequence.push(TopItem::Option(event.options.len()));
				current_option = Some(OptionBlock {
					blank_before,
					name: None,
					name_after: 0,
					name_blank_before: 0,
					close_blank_before: 0,
					close: None,
					effects: Vec::new(),
				});
			}
			None => {
				event.header.push(parse_entry_line(line, blank_before));
				event.sequence.push(TopItem::Header(event.header.len() - 1));
			}
		}
	}

	// 输入在未闭合块处结束：保留已收集的内容（也不补写结束行），避免丢数据。
	if let Some(block) = current_trigger.as_mut() {
		if block.close.is_none() {
			block.close = Some(String::new());
		}
	}
	if let Some(block) = current_option.as_mut() {
		if block.close.is_none() {
			block.close = Some(String::new());
		}
	}
	flush_trigger(&mut event, current_trigger.take());
	flush_option(&mut event, current_option.take());
	event.trailing_blank_lines = pending_blanks;
	// 常规排版（全部 header → trigger → option）无需顺序表；异常排版才按表还原。
	if sequence_is_canonical(&event) {
		event.sequence.clear();
	}
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
	let eol = if event.crlf { "\r\n" } else { "\n" };
	let mut lines: Vec<String> = Vec::new();

	if !event.sequence.is_empty() && sequence_is_valid(event) {
		// 异常排版（内容写在块之后等）：按原顺序还原。
		for item in &event.sequence {
			match item {
				TopItem::Header(index) => push_entry_line(&mut lines, &event.header[*index]),
				TopItem::Trigger(index) => push_trigger_block(&mut lines, &event.triggers[*index]),
				TopItem::Option(index) => push_option_block(&mut lines, &event.options[*index]),
			}
		}
	} else {
		for entry in &event.header {
			push_entry_line(&mut lines, entry);
		}
		for block in &event.triggers {
			push_trigger_block(&mut lines, block);
		}
		for block in &event.options {
			push_option_block(&mut lines, block);
		}
	}

	let mut output = lines.join(eol);
	for _ in 0..event.trailing_blank_lines {
		output.push_str(eol);
	}
	output
}

/// 顺序表是否覆盖全部内容且各类型下标严格升序（编辑增删后自动失效，
/// 此时回退到常规排版，避免用过期的下标还原）。
fn sequence_is_valid(event: &MissionEvent) -> bool {
	let mut header = 0_usize;
	let mut trigger = 0_usize;
	let mut option = 0_usize;
	for item in &event.sequence {
		match item {
			TopItem::Header(index) => {
				if *index != header {
					return false;
				}
				header += 1;
			}
			TopItem::Trigger(index) => {
				if *index != trigger {
					return false;
				}
				trigger += 1;
			}
			TopItem::Option(index) => {
				if *index != option {
					return false;
				}
				option += 1;
			}
		}
	}
	header == event.header.len() && trigger == event.triggers.len() && option == event.options.len()
}

/// 顺序表是否就是常规排版（全部 header → 全部 trigger → 全部 option）：
/// 解析结束时据此丢掉顺序表（常规文件不保留额外状态）。
fn sequence_is_canonical(event: &MissionEvent) -> bool {
	if !sequence_is_valid(event) {
		return false;
	}
	let mut stage = 0_u8; // 0=header，1=trigger，2=option
	for item in &event.sequence {
		match item {
			TopItem::Header(_) => {
				if stage != 0 {
					return false;
				}
			}
			TopItem::Trigger(_) => {
				if stage > 1 {
					return false;
				}
				stage = 1;
			}
			TopItem::Option(_) => stage = 2,
		}
	}
	true
}

/// 追加一个触发块（含 next_ 行位置与结束行的原样还原）。
fn push_trigger_block(lines: &mut Vec<String>, block: &TriggerBlock) {
	for _ in 0..block.blank_before {
		lines.push(String::new());
	}
	lines.push(block.kind.open_token().to_string());
	for (index, entry) in block.conditions.iter().enumerate() {
		// next_ 行按解析时记录的位置还原（可在若干条件之后）。
		if let Some(join) = block.join {
			if block.join_after == index {
				for _ in 0..block.join_blank_before {
					lines.push(String::new());
				}
				match &block.join_token {
					Some(token) => lines.push(token.clone()),
					None => lines.push(join.token().to_string()),
				}
			}
		}
		push_entry_line(lines, entry);
	}
	if let Some(join) = block.join {
		if block.join_after >= block.conditions.len() {
			for _ in 0..block.join_blank_before {
				lines.push(String::new());
			}
			match &block.join_token {
				Some(token) => lines.push(token.clone()),
				None => lines.push(join.token().to_string()),
			}
		}
	}
	for _ in 0..block.close_blank_before {
		lines.push(String::new());
	}
	match &block.close {
		// `Some("")` = 原文件未写结束行：保持不写。
		Some(close) if close.is_empty() => {}
		Some(close) => lines.push(close.clone()),
		None => lines.push(block.kind.close_token().to_string()),
	}
}

/// 追加一个选项块（含 name= 行位置与结束行的原样还原）。
fn push_option_block(lines: &mut Vec<String>, block: &OptionBlock) {
	for _ in 0..block.blank_before {
		lines.push(String::new());
	}
	lines.push("option_btn".to_string());
	for (index, entry) in block.effects.iter().enumerate() {
		// name= 行按解析时记录的位置还原（可在若干效果之后）。
		if let Some(name) = &block.name {
			if block.name_after == index {
				for _ in 0..block.name_blank_before {
					lines.push(String::new());
				}
				lines.push(format!("name={name}"));
			}
		}
		push_entry_line(lines, entry);
	}
	if let Some(name) = &block.name {
		if block.name_after >= block.effects.len() {
			for _ in 0..block.name_blank_before {
				lines.push(String::new());
			}
			lines.push(format!("name={name}"));
		}
	}
	if block.close.is_none() {
		// 结束行前的空行原样还原。
		for _ in 0..block.close_blank_before {
			lines.push(String::new());
		}
		lines.push("option_end".to_string());
	}
	// `close == Some(_)`（含空串）= 原文件未写 `option_end`：保持不写。
}

/// 追加单行条目（先补空行，再按 `键=值` 或原样输出）。
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
		join_after: 0,
		join_blank_before: 0,
		join_token: None,
		close_blank_before: 0,
		close: None,
		conditions: vec![EntryLine::new("civ_capital_unrest_over", "-1")],
	});
	event.options.push(OptionBlock {
		blank_before: 1,
		name: Some(String::new()),
		name_after: 0,
		name_blank_before: 0,
		close_blank_before: 0,
		close: None,
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
	fn round_trip_preserves_crlf_whitespace_and_positions() {
		// CRLF + next_ 写在条件之后 + name 写在效果之后（新版模组的真实写法）。
		let text = "id=x\r\ntitle=y\r\ntrigger_and\r\na=1\r\nnext_and\r\nb=2\r\ntrigger_and_end\r\n\r\noption_btn\r\nai=25\r\nname=好\r\ngold=1\r\noption_end\r\n";
		let event = MissionEvent::parse(text);
		assert!(event.crlf);
		assert_eq!(event.triggers[0].join_after, 1);
		assert_eq!(event.options[0].name_after, 1);
		assert_eq!(event.to_text(), text);

		// 仅空白行按内容原样保留（不能当成空行丢弃）。
		let text = "id=x\n \noption_btn\nname=好\noption_end\n";
		assert_eq!(MissionEvent::parse(text).to_text(), text);

		// LF 文件不受影响。
		let text = "id=x\ntrigger_and\nnext_and\na=1\ntrigger_and_end\n";
		assert_eq!(MissionEvent::parse(text).to_text(), text);
	}

	#[test]
	fn round_trip_preserves_interleaved_and_odd_tokens() {
		// 内容写在触发块之后（个别模组手误）：按原顺序还原。
		let text = "id=x\ntrigger_and\nnext_and\nis_civ=a\ntrigger_and_end\nexists_any=b\n\ntrigger_and\nnext_and\nis_civ=c\ntrigger_and_end\n";
		assert_eq!(MissionEvent::parse(text).to_text(), text);

		// 结束行与块类型不一致：原样保留。
		let text = "id=x\ntrigger_and\nnext_and\na=1\nnext_or\nb=2\ntrigger_or_end\n";
		assert_eq!(MissionEvent::parse(text).to_text(), text);

		// 多出来的 option_end 与写在效果之后的 name（实测模组写法）：位置原样保留。
		let text = "id=x\noption_btn\nname=接受\ngold=1\noption_end\n\noption_end\nname=拒绝\ngold=2\noption_end\n";
		assert_eq!(MissionEvent::parse(text).to_text(), text);

		// next_not 别名与结束行之前的空行（实测模组写法）：原样保留。
		let text = "id=x\ntrigger_and\nnext_not\na=1\nnext_or\nb=2\n\ntrigger_and_end\n\noption_btn\nname=好\nai=5\n\noption_end\n";
		assert_eq!(MissionEvent::parse(text).to_text(), text);
	}

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
	fn crlf_is_preserved() {
		// Windows 风格（CRLF）的模组文件：解析后按原换行符输出，保证未改动即逐字节一致。
		assert_eq!(
			MissionEvent::parse("a=1\r\nb=2\r\n").to_text(),
			"a=1\r\nb=2\r\n"
		);
		// LF 文件保持 LF。
		assert_eq!(MissionEvent::parse("a=1\nb=2\n").to_text(), "a=1\nb=2\n");
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

	/// 五个模组工作区的全部事件文件路径（剧本 `missionsEvents` / `events/common` + 全局目录）。
	fn real_event_file_paths() -> Vec<std::path::PathBuf> {
		let roots = [
			r"A:\android\GameCivs\1566AuroraPrever2",
			r"A:\android\GameCivs\europe",
			r"A:\android\GameCivs\road_to_56",
			r"A:\android\GameCivs\暮色黄昏_世界大战0.25.1",
			r"A:\android\GameCivs\白日升",
		];
		let mut dirs: Vec<std::path::PathBuf> = Vec::new();
		for root in roots {
			let root = std::path::Path::new(root);
			if let Ok(maps) = std::fs::read_dir(root.join("assets/map")) {
				for map in maps.flatten() {
					if let Ok(scenarios) = std::fs::read_dir(map.path().join("scenarios")) {
						for scenario in scenarios.flatten() {
							dirs.push(scenario.path().join("missions/missionsEvents"));
							dirs.push(scenario.path().join("events/common"));
						}
					}
				}
			}
			dirs.push(root.join("assets/game/missions/missionsEvents"));
			dirs.push(root.join("assets/game/events/common"));
		}
		let mut paths = Vec::new();
		for dir in dirs {
			let Ok(entries) = std::fs::read_dir(&dir) else {
				continue;
			};
			for entry in entries.flatten() {
				let path = entry.path();
				if path
					.extension()
					.is_some_and(|extension| extension.eq_ignore_ascii_case("txt"))
				{
					paths.push(path);
				}
			}
		}
		paths
	}

	/// 用真实模组数据（GameCivs 下五个模组）做事件文件逐字节往返校验。
	/// 仅在显式执行时运行：`cargo test -p age-civ-mod-tool-ui --lib -- --ignored`
	#[test]
	#[ignore = "需要真实数据目录 A:\\android\\GameCivs 下的模组"]
	fn round_trip_real_files() {
		let paths = real_event_file_paths();
		assert!(paths.len() >= 9000, "事件文件过少：{}", paths.len());

		let mut failures: Vec<String> = Vec::new();
		let mut total = 0_usize;
		let mut skipped_encoding = 0_usize;
		for path in paths {
			let Ok(text) = std::fs::read_to_string(&path) else {
				skipped_encoding += 1;
				continue;
			};
			total += 1;
			let event = MissionEvent::parse(&text);
			let output = event.to_text();
			if output != text {
				if failures.len() < 15 {
					failures.push(format!(
						"{}：往返不一致（原 {} 字节，输出 {} 字节）",
						path.display(),
						text.len(),
						output.len()
					));
				}
			}
		}
		println!("往返校验：{total} 个文件，非 UTF-8 跳过 {skipped_encoding} 个");
		assert!(
			failures.is_empty(),
			"{total} 个文件中有往返不一致（前 {} 个）：\n{}",
			failures.len(),
			failures.join("\n")
		);
	}

	/// 报告 Schema 注册表对真实数据的覆盖度（仅打印，不判失败）。
	/// 仅在显式执行时运行：`cargo test -p age-civ-mod-tool-ui --lib -- --ignored`
	#[test]
	#[ignore = "需要真实数据目录 A:\\android\\GameCivs 下的模组"]
	fn schema_coverage_report() {
		let paths = real_event_file_paths();

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
