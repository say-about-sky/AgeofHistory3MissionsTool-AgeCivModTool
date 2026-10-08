//! 事件编辑器「文明 / 政体 / 省份 / 建筑 / 疾病 / 人物 / 国家精神 / 科技 / 宗教 / 资源对照表」。
//!
//! 数据由后端命令提供（解析核心移植自 ScvGen 库，见
//! `src-tauri/src/commands/missions_db.rs`）：
//! - 文明：tag → 名称（`Civilizations.txt` + 翻译 `.properties` + `civilizations/*.json`）；
//! - 政体：整数序号 → 名称（`Governments.json` 的 `Name:` / `Extra_Tag:` 顺序扫描，
//!   序号即游戏内使用的「政体整数」）；
//! - 省份：地图（`assets/map/Maps.json` 发现）省份 ID → 地名（`cities/cities.json`）；
//! - 建筑 / 疾病：定义数组顺序即 ID（疾病名称经根语言表翻译）；
//! - 人物：`characters/**.json` 的 `Name` 与文件名（供 add_general 系列补全）；
//! - 国家精神：`NationalSpirit.json` 的字符串 id（如 `fra1`）→ 名称（add_ns / remove_ns）；
//! - 科技 / 资源：`Name:` + `ID:` 配对表（TypeNames / Resources.json）；
//! - 宗教：`Religions.json` 数组顺序即 ID（change_religion 等）。
//!
//! 供 `event_grid` 为对应值单元格提供候选补全与「值 → 名称」对照提示：
//! 桌面端走原生 `<datalist>`，安卓端走 `event_grid::SuggestInput` 页面内自绘下拉（同一套数据）。
//! 另载入「指向资源」的事件 / 音乐 / 图片候选（见 [`load_event_assets`]）。

use serde::{Deserialize, Serialize};
use wasm_bindgen_futures::JsFuture;

use super::WorkDirectory;
use crate::app::tauri_bridge::invoke;

/// 文明对照条目。
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct CivItem {
	pub tag: String,
	pub name: String,
}

/// 政体对照条目（`index` 为游戏内「政体整数」）。
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct GovItem {
	pub index: u32,
	pub name: String,
}

/// 数值 ID + 名称对照条目（省份 / 建筑 / 疾病 / 科技 / 宗教 / 资源共用）。
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct IdNameItem {
	pub id: u32,
	pub name: String,
}

/// 字符串 ID + 名称对照条目（国家精神：`id` 如 `fra1`）。
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct IdTextItem {
	pub id: String,
	pub name: String,
}

/// 事件编辑器对照表（后端 `load_event_lookup` 命令返回结构的镜像）。
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct EventLookup {
	#[serde(default)]
	pub civs: Vec<CivItem>,
	#[serde(default)]
	pub governments: Vec<GovItem>,
	/// 人物名称（add_general / add_general2 等）。
	#[serde(default)]
	pub characters: Vec<String>,
	/// 省份清单（`id` 升序，名称可能为空）。
	#[serde(default)]
	pub provinces: Vec<IdNameItem>,
	/// 建筑清单（数组顺序即 ID）。
	#[serde(default)]
	pub buildings: Vec<IdNameItem>,
	/// 疾病清单（数组顺序即 ID）。
	#[serde(default)]
	pub diseases: Vec<IdNameItem>,
	/// 国家精神清单（`id` 为字符串；add_ns / remove_ns）。
	#[serde(default)]
	pub national_spirits: Vec<IdTextItem>,
	/// 科技清单（unlock_tech）。
	#[serde(default)]
	pub technologies: Vec<IdNameItem>,
	/// 宗教清单（数组顺序即 ID；change_religion 等）。
	#[serde(default)]
	pub religions: Vec<IdNameItem>,
	/// 资源清单（resource_price_change 系列）。
	#[serde(default)]
	pub resources: Vec<IdNameItem>,
}

impl EventLookup {
	/// 是否没有任何对照数据（未加载或工作区没有游戏数据文件）。
	pub fn is_empty(&self) -> bool {
		self.civs.is_empty()
			&& self.governments.is_empty()
			&& self.characters.is_empty()
			&& self.provinces.is_empty()
			&& self.buildings.is_empty()
			&& self.diseases.is_empty()
			&& self.national_spirits.is_empty()
			&& self.technologies.is_empty()
			&& self.religions.is_empty()
			&& self.resources.is_empty()
	}

	/// 按 tag 查文明名称（ASCII 大小写不敏感，中文等非 ASCII 精确匹配）。
	pub fn civ_name(&self, tag: &str) -> Option<&str> {
		self.civs
			.iter()
			.find(|item| item.tag.eq_ignore_ascii_case(tag))
			.map(|item| item.name.as_str())
	}

	/// 按序号查政体名称。
	pub fn gov_name(&self, index: u32) -> Option<&str> {
		self.governments
			.binary_search_by_key(&index, |item| item.index)
			.ok()
			.map(|position| self.governments[position].name.as_str())
	}

	/// 按 ID 查省份名称（无地名时返回 None）。
	pub fn province_name(&self, id: u32) -> Option<&str> {
		name_of(&self.provinces, id)
	}

	/// 按 ID 查建筑名称。
	pub fn building_name(&self, id: u32) -> Option<&str> {
		name_of(&self.buildings, id)
	}

	/// 按 ID 查疾病名称（中译优先）。
	pub fn disease_name(&self, id: u32) -> Option<&str> {
		name_of(&self.diseases, id)
	}

	/// 按字符串 ID 查国家精神名称。
	pub fn national_spirit_name(&self, id: &str) -> Option<&str> {
		self.national_spirits
			.iter()
			.find(|item| item.id == id)
			.map(|item| item.name.as_str())
	}

	/// 按 ID 查科技名称。
	pub fn technology_name(&self, id: u32) -> Option<&str> {
		name_of(&self.technologies, id)
	}

	/// 按 ID 查宗教名称。
	pub fn religion_name(&self, id: u32) -> Option<&str> {
		name_of(&self.religions, id)
	}

	/// 按 ID 查资源名称。
	pub fn resource_name(&self, id: u32) -> Option<&str> {
		name_of(&self.resources, id)
	}
}

/// 按 ID 二分查找非空名称（后端输出按 ID 升序）。
fn name_of(items: &[IdNameItem], id: u32) -> Option<&str> {
	items
		.binary_search_by_key(&id, |item| item.id)
		.ok()
		.map(|position| items[position].name.as_str())
		.filter(|name| !name.is_empty())
}

/// 值单元格中每一段（以 `=` 分隔）的语义。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ValuePart {
	/// 文明 tag（如 `atr`、`fra`）。
	Civ,
	/// 政体整数（`Governments.json` 顺序号）。
	Gov,
	/// 省份 ID（如 `move_capital`）。
	Province,
	/// 省份 ID，且值段以 `;` 结尾（如 `province_add_building` 的 `3994;`）。
	ProvinceSemi,
	/// 建筑整数 ID（`Buildings.json` 顺序号）。
	Building,
	/// 疾病整数 ID（`Diseases.json` 顺序号）。
	Disease,
	/// 人物名称（`characters` 清单，值本身就是名称）。
	Character,
	/// 国家精神字符串 ID（`NationalSpirit.json`，如 `fra1`）。
	NationalSpirit,
	/// 科技整数 ID（`Technologies.json` 顺序号）。
	Technology,
	/// 宗教整数 ID（`Religions.json` 顺序号）。
	Religion,
	/// 资源整数 ID（`Resources.json`；`resource_price_change` 系列）。
	Resource,
	/// 事件文件名（`run_event` / `run_event_instantly`，值 = 文件名去 `.txt`）。
	EventId,
	/// 音乐名（`musicName` / `play_music`；音频文件名去扩展名或 `list*.txt` 条目）。
	Music,
	/// 图片资源文件名（`image` / `mission_image` / `decision_image`，含 `.png` 扩展名）。
	Image,
	/// 普通文本段（不提供补全与提示）。
	Plain,
}

pub const CIV_DATALIST_ID: &str = "evdl-civ";
pub const GOV_DATALIST_ID: &str = "evdl-gov";
pub const PROVINCE_DATALIST_ID: &str = "evdl-prov";
pub const BUILDING_DATALIST_ID: &str = "evdl-build";
pub const DISEASE_DATALIST_ID: &str = "evdl-disease";
pub const CHARACTER_DATALIST_ID: &str = "evdl-char";
pub const NATIONAL_SPIRIT_DATALIST_ID: &str = "evdl-ns";
pub const TECHNOLOGY_DATALIST_ID: &str = "evdl-tech";
pub const RELIGION_DATALIST_ID: &str = "evdl-religion";
pub const RESOURCE_DATALIST_ID: &str = "evdl-resource";
pub const EVENT_DATALIST_ID: &str = "evdl-event";
pub const MUSIC_DATALIST_ID: &str = "evdl-music";
pub const IMAGE_DATALIST_ID: &str = "evdl-image";

/// 分段语义对应的 datalist id（`Plain` 无候选列表）。
pub fn datalist_id(kind: ValuePart) -> &'static str {
	match kind {
		ValuePart::Civ => CIV_DATALIST_ID,
		ValuePart::Gov => GOV_DATALIST_ID,
		ValuePart::Province | ValuePart::ProvinceSemi => PROVINCE_DATALIST_ID,
		ValuePart::Building => BUILDING_DATALIST_ID,
		ValuePart::Disease => DISEASE_DATALIST_ID,
		ValuePart::Character => CHARACTER_DATALIST_ID,
		ValuePart::NationalSpirit => NATIONAL_SPIRIT_DATALIST_ID,
		ValuePart::Technology => TECHNOLOGY_DATALIST_ID,
		ValuePart::Religion => RELIGION_DATALIST_ID,
		ValuePart::Resource => RESOURCE_DATALIST_ID,
		ValuePart::EventId => EVENT_DATALIST_ID,
		ValuePart::Music => MUSIC_DATALIST_ID,
		ValuePart::Image => IMAGE_DATALIST_ID,
		ValuePart::Plain => "",
	}
}

/// 值段固定后缀（分段输入框在拼接时自动附加，显示时自动去除）。
pub fn part_suffix(kind: ValuePart) -> &'static str {
	match kind {
		ValuePart::ProvinceSemi => ";",
		_ => "",
	}
}

/// 按键名查询值分段语义；返回 `None` 表示该键与对照表无关。
///
/// 依据《国策系统说明文档》（docs/国策系统说明文档.md）与 `event_schema` 的注解整理：
/// 只有标注「文明ID / tag / civ」的段位返回 [`ValuePart::Civ`]，
/// 只有「政体」相关字段返回 [`ValuePart::Gov`]；省份 / 建筑 / 疾病 / 人物同理。
pub fn value_parts(key: &str) -> Option<&'static [ValuePart]> {
	use ValuePart::*;
	let parts: &'static [ValuePart] = match key {
		// ===== 触发条件 =====
		"is_civ" | "is_player" | "is_not_player" | "exists_any" | "exists_any_not"
		| "province_controlled_by" | "civ_has_truce_with" => &[Civ],
		"civ_is_vassal_of_civ" | "civs_are_at_war" => &[Civ, Civ],
		"has_variable_civ" => &[Civ, Plain],
		"province_has_building" => &[Province, Building],
		// ===== 收益效果 =====
		"change_ideology" => &[Gov],
		"change_ideology_civ" => &[Civ, Gov],
		"set_civ_tag" | "set_civ_tag_reset" | "player_set_civ" | "annex_civ" | "annexed_by_civ"
		| "declare_war" | "alliance" | "non_aggression_pact" | "military_access" | "vassalize" => {
			&[Civ]
		}
		// 「文明A=文明B」两段（实测 missionsEvents：declare_war2 3181 处、white_peace 3017 处
		// 写作 `==` 即两段皆空，其余为 `tagA=tagB`；空段表示按上下文取默认方）。
		"set_civ_tag2" | "make_puppet" | "declare_war2" | "white_peace" | "white_peace2"
		| "add_defensive_pact" | "add_guarantee" | "add_truce" => &[Civ, Civ],
		"annex_by_civ_from_civ" => &[Civ, Civ, Civ],
		"relation_change" | "relation_set" | "add_variable_civ" | "remove_variable_civ"
		| "annex_provinces_from_civ" => &[Civ, Plain],
		// 省份（第二段为数值/未确认语义时按 Plain 处理）
		"move_capital" | "province_id_nuke" => &[Province],
		"province_economy_id" | "province_manpower_id" | "province_devastation_id"
		| "province_id_pop_set" => &[Province, Plain],
		// 核心相关：文档为「省份ID=文明ID」（第二段是文明而非数字）。
		"province_core_of" | "province_id_core_add" | "province_id_core_remove" => {
			&[Province, Civ]
		}
		"province_id_build_add" | "province_id_build_remove" => &[Province, Building],
		"province_id_spread_disease" => &[Province, Disease],
		// 值段以 `;` 结尾的 Lambda 格式：`省份ID;=建筑ID=整数`
		"province_add_building" => &[ProvinceSemi, Building, Plain],
		// 人物名称（add_general 旧版也接受 true，候选列表只是建议）
		"add_general" | "add_general2" | "add_general_character" | "add_advisor_character" => {
			&[Character]
		}
		"add_general_character_attack_defense" => &[Character, Plain, Plain],
		// 图片资源（值=含扩展名的文件名，如 `国策通用.png`）：候选来自
		// `<missions root>/missionsImages/H` 与 `<game>/events/images/H`（见后端 list_event_assets）。
		"image" | "mission_image" | "decision_image" => &[Image],
		// 国家精神（值 = `NationalSpirit.json` 的字符串 id，如 `fra1`）。
		"add_ns" | "remove_ns" => &[NationalSpirit],
		// 科技 / 宗教 / 资源（值 = 对应数据表的整数 ID）。
		"unlock_tech" => &[Technology],
		"change_religion" | "province_religion_all" => &[Religion],
		"change_religion_civ" => &[Civ, Religion],
		"resource_price_change" | "resource_price_change_up" | "resource_price_change_down"
		| "resource_price_change_random" => &[Resource, Plain],
		// 事件（值 = 事件文件名去 `.txt`，如 `1919年巴西大选补选`）。
		"run_event" | "run_event_instantly" => &[EventId],
		// 音乐（值 = 音频文件名去扩展名或 `list*.txt` 清单条目）。
		"musicName" | "play_music" => &[Music],
		_ => return None,
	};
	Some(parts)
}

/// 替换 `value` 中以 `=` 分隔的第 `index` 段（不足处补空段），返回拼接结果。
/// `suffix` 为该段的固定后缀（如 `ProvinceSemi` 的 `;`），写入时自动附加。
pub fn replace_value_part(value: &str, index: usize, suffix: &str, new_part: &str) -> String {
	let mut parts: Vec<String> = value.split('=').map(str::to_string).collect();
	while parts.len() <= index {
		parts.push(String::new());
	}
	parts[index] = if suffix.is_empty() {
		new_part.to_string()
	} else {
		format!("{new_part}{suffix}")
	};
	parts.join("=")
}

/// 拼接「值段 → 名称」对照提示（只显示能查到的段，拼写中间状态的段不打扰）。
pub fn name_hint(lookup: &EventLookup, parts: &[ValuePart], value: &str) -> String {
	let mut pieces: Vec<String> = Vec::new();
	for (index, part) in value.split('=').enumerate() {
		let Some(kind) = parts.get(index) else {
			continue;
		};
		let part = part.trim();
		if part.is_empty() {
			continue;
		}
		match kind {
			ValuePart::Civ => {
				if let Some(name) = lookup.civ_name(part) {
					pieces.push(format!("{part}={name}"));
				}
			}
			ValuePart::Gov => {
				if let Ok(index) = part.parse::<u32>() {
					if let Some(name) = lookup.gov_name(index) {
						pieces.push(format!("{part}={name}"));
					}
				}
			}
			ValuePart::Province | ValuePart::ProvinceSemi => {
				let text = part.trim_end_matches(';').trim();
				if let Ok(id) = text.parse::<u32>() {
					if let Some(name) = lookup.province_name(id) {
						pieces.push(format!("{text}={name}"));
					}
				}
			}
			ValuePart::Building => {
				if let Ok(id) = part.parse::<u32>() {
					if let Some(name) = lookup.building_name(id) {
						pieces.push(format!("{part}={name}"));
					}
				}
			}
			ValuePart::Disease => {
				if let Ok(id) = part.parse::<u32>() {
					if let Some(name) = lookup.disease_name(id) {
						pieces.push(format!("{part}={name}"));
					}
				}
			}
			ValuePart::NationalSpirit => {
				if let Some(name) = lookup.national_spirit_name(part) {
					pieces.push(format!("{part}={name}"));
				}
			}
			ValuePart::Technology | ValuePart::Religion | ValuePart::Resource => {
				if let Ok(id) = part.parse::<u32>() {
					let name = match kind {
						ValuePart::Technology => lookup.technology_name(id),
						ValuePart::Religion => lookup.religion_name(id),
						_ => lookup.resource_name(id),
					};
					if let Some(name) = name {
						pieces.push(format!("{part}={name}"));
					}
				}
			}
			// 人物 / 事件 / 音乐的值本身就是名称，无需再对照；图片文件名同理（无「值→名称」映射）。
			ValuePart::Character
			| ValuePart::EventId
			| ValuePart::Music
			| ValuePart::Image
			| ValuePart::Plain => {}
		}
	}
	pieces.join(" · ")
}

// ===== 自绘下拉候选（安卓端替代原生列表弹层；见 event_grid::SuggestInput） =====

/// 候选项匹配排名：0=值前缀，1=值包含，2=说明包含；不匹配返回 `None`。
fn rank_of(value: &str, label: &str, query: &str) -> Option<u8> {
	if ci_starts_with(value, query) {
		return Some(0);
	}
	if ci_contains(value, query) {
		return Some(1);
	}
	if !label.is_empty() && ci_contains(label, query) {
		return Some(2);
	}
	None
}

/// ASCII 不区分大小写前缀判断（非 ASCII 退回 `to_lowercase` 比较）。
fn ci_starts_with(value: &str, query: &str) -> bool {
	if value.is_ascii() && query.is_ascii() {
		value.len() >= query.len()
			&& value.as_bytes()[..query.len()].eq_ignore_ascii_case(query.as_bytes())
	} else {
		value.to_lowercase().starts_with(&query.to_lowercase())
	}
}

/// ASCII 不区分大小写包含判断（非 ASCII 退回 `to_lowercase` 比较）。
fn ci_contains(value: &str, query: &str) -> bool {
	if value.is_ascii() && query.is_ascii() {
		query.len() <= value.len()
			&& value
				.as_bytes()
				.windows(query.len())
				.any(|window| window.eq_ignore_ascii_case(query.as_bytes()))
	} else {
		value.to_lowercase().contains(&query.to_lowercase())
	}
}

/// 字符串候选过滤（保持原顺序，按「前缀 > 包含 > 说明包含」稳定排序后截断）。
fn str_rank_filter<'a>(
	items: impl Iterator<Item = (&'a str, &'a str)>,
	query: &str,
	limit: usize,
) -> Vec<(String, String)> {
	if query.is_empty() {
		return items
			.take(limit)
			.map(|(value, label)| (value.to_string(), label.to_string()))
			.collect();
	}
	let mut scored: Vec<(u8, String, String)> = Vec::new();
	for (value, label) in items {
		if let Some(rank) = rank_of(value, label, query) {
			scored.push((rank, value.to_string(), label.to_string()));
		}
	}
	scored.sort_by_key(|(rank, _, _)| *rank);
	scored.truncate(limit);
	scored
		.into_iter()
		.map(|(_, value, label)| (value, label))
		.collect()
}

/// 数值 ID 候选过滤（ID 先转字符串再匹配，避免全表预先分配）。
fn numbered_rank_filter<'a>(
	items: impl Iterator<Item = (u32, &'a str)>,
	query: &str,
	limit: usize,
) -> Vec<(String, String)> {
	use std::fmt::Write as _;

	if query.is_empty() {
		return items
			.take(limit)
			.map(|(id, label)| (id.to_string(), label.to_string()))
			.collect();
	}
	let mut scored: Vec<(u8, String, String)> = Vec::new();
	let mut buffer = String::new();
	for (id, label) in items {
		buffer.clear();
		let _ = write!(&mut buffer, "{id}");
		if let Some(rank) = rank_of(&buffer, label, query) {
			scored.push((rank, buffer.clone(), label.to_string()));
		}
	}
	scored.sort_by_key(|(rank, _, _)| *rank);
	scored.truncate(limit);
	scored
		.into_iter()
		.map(|(_, value, label)| (value, label))
		.collect()
}

/// 键候选过滤（选项为「键, 说明」，与说明列同源）。
pub fn filter_suggestions(
	options: &[(String, String)],
	query: &str,
	limit: usize,
) -> Vec<(String, String)> {
	str_rank_filter(
		options.iter().map(|(value, label)| (value.as_str(), label.as_str())),
		query,
		limit,
	)
}

/// 资源文件名候选过滤（无说明标签；`filter_suggestions` 的纯名称版）。
pub fn filter_names(names: &[String], query: &str, limit: usize) -> Vec<(String, String)> {
	str_rank_filter(names.iter().map(|name| (name.as_str(), "")), query, limit)
}

/// 对照表候选：按字段类型生成「值, 说明」并过滤（支持按名称/地名搜索，
/// 如输入「奥」可命中 `atr=奥地利`，输入「上海」可命中对应省份 ID）。
pub fn matching_lookup(
	lookup: &EventLookup,
	kind: ValuePart,
	query: &str,
	limit: usize,
) -> Vec<(String, String)> {
	match kind {
		ValuePart::Civ => str_rank_filter(
			lookup
				.civs
				.iter()
				.map(|item| (item.tag.as_str(), item.name.as_str())),
			query,
			limit,
		),
		ValuePart::Gov => numbered_rank_filter(
			lookup
				.governments
				.iter()
				.map(|item| (item.index, item.name.as_str())),
			query,
			limit,
		),
		ValuePart::Province | ValuePart::ProvinceSemi => numbered_rank_filter(
			lookup
				.provinces
				.iter()
				.map(|item| (item.id, item.name.as_str())),
			query,
			limit,
		),
		ValuePart::Building => numbered_rank_filter(
			lookup
				.buildings
				.iter()
				.map(|item| (item.id, item.name.as_str())),
			query,
			limit,
		),
		ValuePart::Disease => numbered_rank_filter(
			lookup
				.diseases
				.iter()
				.map(|item| (item.id, item.name.as_str())),
			query,
			limit,
		),
		ValuePart::Character => str_rank_filter(
			lookup.characters.iter().map(|name| (name.as_str(), "")),
			query,
			limit,
		),
		ValuePart::NationalSpirit => str_rank_filter(
			lookup
				.national_spirits
				.iter()
				.map(|item| (item.id.as_str(), item.name.as_str())),
			query,
			limit,
		),
		ValuePart::Technology => numbered_rank_filter(
			lookup
				.technologies
				.iter()
				.map(|item| (item.id, item.name.as_str())),
			query,
			limit,
		),
		ValuePart::Religion => numbered_rank_filter(
			lookup.religions.iter().map(|item| (item.id, item.name.as_str())),
			query,
			limit,
		),
		ValuePart::Resource => numbered_rank_filter(
			lookup.resources.iter().map(|item| (item.id, item.name.as_str())),
			query,
			limit,
		),
		// 事件 / 音乐 / 图片候选不走对照表（由 `list_event_assets` 单独加载，见 `SuggestOptions::Names`）。
		ValuePart::EventId | ValuePart::Music | ValuePart::Image => Vec::new(),
		ValuePart::Plain => Vec::new(),
	}
}

/// 对照表缓存键：同一游戏数据目录（全局与各剧本资源根共享 `assets/game`）只需加载一次。
///
/// 与后端 `missions_db::game_dir_candidates` 的首选项推导规则保持一致：
/// `…/assets/game/missions` → `…/assets/game`；
/// `…/assets/map/<地图>/scenarios/<剧本>/missions` → `<前缀>/assets/game`；
/// 经典 `missions` → 其上级目录。
pub fn game_dir_key(missions_root: &str) -> String {
	let segments: Vec<&str> = missions_root
		.split('/')
		.filter(|segment| !segment.is_empty())
		.collect();
	for (index, segment) in segments.iter().enumerate() {
		if *segment != "assets" {
			continue;
		}
		if segments.get(index + 1) == Some(&"game") {
			return segments[..index + 2].join("/");
		}
		if segments.get(index + 2).is_some()
			&& segments.get(index + 3) == Some(&"scenarios")
			&& segments.get(index + 4).is_some()
			&& segments.get(index + 5) == Some(&"missions")
		{
			let prefix = segments[..index].join("/");
			return if prefix.is_empty() {
				"assets/game".to_string()
			} else {
				format!("{prefix}/assets/game")
			};
		}
	}
	if segments.len() > 1 {
		segments[..segments.len() - 1].join("/")
	} else {
		String::new()
	}
}

/// 剧本上下文键：`…/assets/map/<地图>/scenarios/<剧本>/…` → `<地图>`；其余（全局根）→ 空串。
///
/// 与后端 `missions_db::scenario_map_of` 的路径规则对应：剧本根按其所属地图
/// 加载省份数据（多地图工作区）；全局根按全部地图合并。
pub fn scenario_map_key(missions_root: &str) -> String {
	let segments: Vec<&str> = missions_root
		.split('/')
		.filter(|segment| !segment.is_empty())
		.collect();
	for (index, segment) in segments.iter().enumerate() {
		if *segment != "map" {
			continue;
		}
		if segments.get(index + 2).copied() != Some("scenarios") {
			continue;
		}
		if let Some(map) = segments.get(index + 1) {
			return (*map).to_string();
		}
	}
	String::new()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LookupArgs {
	work_directory: String,
	missions_root: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LookupScopedArgs {
	folder_id: String,
	missions_root: String,
}

/// 事件脚本资源候选（后端 `list_event_assets` 命令返回结构的镜像）。
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct EventAssets {
	/// 图片：`.png` 文件名（含扩展名，如 `国策通用.png`）。
	#[serde(default)]
	pub images: Vec<String>,
	/// 事件：文件名去 `.txt`（`run_event` / `run_event_instantly` 的值）。
	#[serde(default)]
	pub events: Vec<String>,
	/// 音乐：音频文件名去扩展名与 `list*.txt` 条目（`musicName` / `play_music` 的值）。
	#[serde(default)]
	pub music: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EventAssetsArgs {
	work_directory: String,
	missions_root: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EventAssetsScopedArgs {
	folder_id: String,
	missions_root: String,
}

/// 加载事件脚本资源候选（图片 / 事件 / 音乐，见 [`EventAssets`]）。
///
/// 真实路径走 `list_event_assets`，SAF 走 scoped 变体；两者都会在「从 apk 中导入」
/// 的版块工作区里回退读取源 APK 条目。失败由调用方兜底为空（编辑器仅无候选，不报错）。
pub async fn load_event_assets(
	directory: &WorkDirectory,
	missions_root: &str,
) -> Result<EventAssets, String> {
	let value = if let Some(folder_id) = &directory.folder_id {
		let args = serde_wasm_bindgen::to_value(&EventAssetsScopedArgs {
			folder_id: folder_id.clone(),
			missions_root: missions_root.to_string(),
		})
		.map_err(|error| error.to_string())?;
		JsFuture::from(invoke("list_event_assets_scoped", args))
			.await
			.map_err(|error| format!("加载资源候选失败：{error:?}"))?
	} else {
		let args = serde_wasm_bindgen::to_value(&EventAssetsArgs {
			work_directory: directory.root_path.clone(),
			missions_root: missions_root.to_string(),
		})
		.map_err(|error| error.to_string())?;
		JsFuture::from(invoke("list_event_assets", args))
			.await
			.map_err(|error| format!("加载资源候选失败：{error:?}"))?
	};
	serde_wasm_bindgen::from_value(value).map_err(|error| format!("资源候选格式错误：{error}"))
}

/// 加载对照表：真实路径模式走 `load_event_lookup`，SAF 模式走 `load_event_lookup_scoped`。
///
/// 工作区没有游戏数据文件时后端返回空表（编辑器不显示补全但不报错）。
pub async fn load_event_lookup(
	directory: &WorkDirectory,
	missions_root: &str,
) -> Result<EventLookup, String> {
	if let Some(folder_id) = &directory.folder_id {
		let args = serde_wasm_bindgen::to_value(&LookupScopedArgs {
			folder_id: folder_id.clone(),
			missions_root: missions_root.to_string(),
		})
		.map_err(|error| error.to_string())?;
		let value = JsFuture::from(invoke("load_event_lookup_scoped", args))
			.await
			.map_err(|error| format!("加载文明对照表失败：{error:?}"))?;
		serde_wasm_bindgen::from_value(value)
			.map_err(|error| format!("文明对照表格式错误：{error}"))
	} else {
		let args = serde_wasm_bindgen::to_value(&LookupArgs {
			work_directory: directory.root_path.clone(),
			missions_root: missions_root.to_string(),
		})
		.map_err(|error| error.to_string())?;
		let value = JsFuture::from(invoke("load_event_lookup", args))
			.await
			.map_err(|error| format!("加载文明对照表失败：{error:?}"))?;
		serde_wasm_bindgen::from_value(value)
			.map_err(|error| format!("文明对照表格式错误：{error}"))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn sample_lookup() -> EventLookup {
		EventLookup {
			civs: vec![
				CivItem { tag: "atr".into(), name: "奥地利".into() },
				CivItem { tag: "fra".into(), name: "法兰西".into() },
				CivItem { tag: "ger".into(), name: "德意志国".into() },
				CivItem { tag: "bah".into(), name: "巴哈马".into() },
			],
			governments: vec![
				GovItem { index: 0, name: "临时政府".into() },
				GovItem { index: 10, name: "专制主义".into() },
			],
			characters: vec!["约瑟夫·霞飞".into(), "斐迪南·福熙".into()],
			provinces: vec![
				IdNameItem { id: 11, name: "里斯本".into() },
				IdNameItem { id: 3475, name: "北平".into() },
				IdNameItem { id: 12049, name: String::new() },
			],
			buildings: vec![
				IdNameItem { id: 6, name: "要塞".into() },
				IdNameItem { id: 10, name: "补给中心".into() },
			],
			diseases: vec![
				IdNameItem { id: 0, name: "黑死病".into() },
				IdNameItem { id: 1, name: "天花".into() },
			],
			national_spirits: vec![
				IdTextItem { id: "fra1".into(), name: "法兰西万岁".into() },
				IdTextItem { id: "sov3".into(), name: "苏维埃精神".into() },
			],
			technologies: vec![
				IdNameItem { id: 0, name: "毒气科技".into() },
				IdNameItem { id: 15, name: "机械化".into() },
			],
			religions: vec![
				IdNameItem { id: 0, name: "Pagan".into() },
				IdNameItem { id: 1, name: "Catholic".into() },
			],
			resources: vec![
				IdNameItem { id: 0, name: "Grain".into() },
				IdNameItem { id: 1, name: "Rice".into() },
			],
		}
	}

	#[test]
	fn value_parts_covers_civ_and_gov_fields() {
		let civ: &[ValuePart] = &[ValuePart::Civ];
		let civ_pair: &[ValuePart] = &[ValuePart::Civ, ValuePart::Civ];
		let civ_triple: &[ValuePart] = &[ValuePart::Civ, ValuePart::Civ, ValuePart::Civ];
		let gov: &[ValuePart] = &[ValuePart::Gov];
		let civ_gov: &[ValuePart] = &[ValuePart::Civ, ValuePart::Gov];

		assert_eq!(value_parts("is_civ"), Some(civ));
		assert_eq!(value_parts("annex_by_civ_from_civ"), Some(civ_triple));
		assert_eq!(value_parts("civ_is_vassal_of_civ"), Some(civ_pair));
		assert_eq!(value_parts("change_ideology"), Some(gov));
		assert_eq!(value_parts("change_ideology_civ"), Some(civ_gov));
		// 白和平 / 宣战2：实测 missionsEvents 为「文明A=文明B」（如 `bul=tur_n`、`atr=ita`；
		// 多数写作 `==` 两空段），因此按两段文明渲染。
		assert_eq!(value_parts("white_peace"), Some(civ_pair));
		assert_eq!(value_parts("white_peace2"), Some(civ_pair));
		assert_eq!(value_parts("declare_war2"), Some(civ_pair));
		// 与文明/政体无关的键不参与。
		assert_eq!(value_parts("gold"), None);
		assert_eq!(value_parts("legacy"), None);
	}

	#[test]
	fn value_parts_covers_province_building_disease_character_fields() {
		let province: &[ValuePart] = &[ValuePart::Province];
		let province_plain: &[ValuePart] = &[ValuePart::Province, ValuePart::Plain];
		let province_building: &[ValuePart] = &[ValuePart::Province, ValuePart::Building];
		let province_disease: &[ValuePart] = &[ValuePart::Province, ValuePart::Disease];
		let semi_building: &[ValuePart] = &[
			ValuePart::ProvinceSemi,
			ValuePart::Building,
			ValuePart::Plain,
		];
		let character: &[ValuePart] = &[ValuePart::Character];

		assert_eq!(value_parts("move_capital"), Some(province));
		assert_eq!(value_parts("province_economy_id"), Some(province_plain));
		assert_eq!(value_parts("province_has_building"), Some(province_building));
		assert_eq!(value_parts("province_id_build_add"), Some(province_building));
		assert_eq!(value_parts("province_id_spread_disease"), Some(province_disease));
		assert_eq!(value_parts("province_add_building"), Some(semi_building));
		assert_eq!(value_parts("add_general2"), Some(character));
		assert_eq!(value_parts("add_general"), Some(character));
		// 后缀与 datalist 归属。
		assert_eq!(part_suffix(ValuePart::ProvinceSemi), ";");
		assert_eq!(part_suffix(ValuePart::Province), "");
		assert_eq!(datalist_id(ValuePart::ProvinceSemi), PROVINCE_DATALIST_ID);
		assert_eq!(datalist_id(ValuePart::Character), CHARACTER_DATALIST_ID);
	}

	#[test]
	fn civ_and_gov_name_lookup_work() {
		let lookup = sample_lookup();
		assert_eq!(lookup.civ_name("atr"), Some("奥地利"));
		// ASCII 大小写不敏感。
		assert_eq!(lookup.civ_name("ATR"), Some("奥地利"));
		assert_eq!(lookup.civ_name("xxx"), None);
		assert_eq!(lookup.gov_name(10), Some("专制主义"));
		assert_eq!(lookup.gov_name(9), None);
		// 省份 / 建筑 / 疾病名称（无名称的省份返回 None）。
		assert_eq!(lookup.province_name(3475), Some("北平"));
		assert_eq!(lookup.province_name(12049), None);
		assert_eq!(lookup.building_name(6), Some("要塞"));
		assert_eq!(lookup.disease_name(1), Some("天花"));
	}

	#[test]
	fn name_hint_maps_each_segment() {
		let lookup = sample_lookup();
		let parts = value_parts("declare_war2").unwrap();
		assert_eq!(name_hint(&lookup, parts, "fra=ger"), "fra=法兰西 · ger=德意志国");
		// 拼写中间状态不显示，不干扰输入。
		assert_eq!(name_hint(&lookup, parts, "fr"), "");
		assert_eq!(name_hint(&lookup, parts, "fra="), "fra=法兰西");
		// 政体整数。
		let parts = value_parts("change_ideology").unwrap();
		assert_eq!(name_hint(&lookup, parts, "10"), "10=专制主义");
		assert_eq!(name_hint(&lookup, parts, "3"), "");
		// 文明=政体混合。
		let parts = value_parts("change_ideology_civ").unwrap();
		assert_eq!(name_hint(&lookup, parts, "ger=0"), "ger=德意志国 · 0=临时政府");
		// 省份 / 建筑 / 疾病（含 `;` 后缀与空名称）。
		let parts = value_parts("move_capital").unwrap();
		assert_eq!(name_hint(&lookup, parts, "3475"), "3475=北平");
		assert_eq!(name_hint(&lookup, parts, "12049"), "");
		let parts = value_parts("province_has_building").unwrap();
		assert_eq!(name_hint(&lookup, parts, "11=6"), "11=里斯本 · 6=要塞");
		let parts = value_parts("province_add_building").unwrap();
		assert_eq!(name_hint(&lookup, parts, "3475;=6=0"), "3475=北平 · 6=要塞");
		let parts = value_parts("province_id_spread_disease").unwrap();
		assert_eq!(name_hint(&lookup, parts, "11=1"), "11=里斯本 · 1=天花");
		// 人物名称本身就是值，不产生对照提示。
		let parts = value_parts("add_general2").unwrap();
		assert_eq!(name_hint(&lookup, parts, "约瑟夫·霞飞"), "");
	}

	#[test]
	fn replace_value_part_rebuilds_value() {
		assert_eq!(replace_value_part("fra=ger", 1, "", "atr"), "fra=atr");
		assert_eq!(replace_value_part("fra", 1, "", "ger"), "fra=ger");
		assert_eq!(replace_value_part("", 0, "", "atr"), "atr");
		assert_eq!(replace_value_part("a=b=c", 0, "", "x"), "x=b=c");
		// 固定后缀：写入时自动附加（`province_add_building` 的 `省份ID;`）。
		assert_eq!(replace_value_part("", 0, ";", "3994"), "3994;");
		assert_eq!(replace_value_part("3994;=6=0", 0, ";", "3475"), "3475;=6=0");
	}

	#[test]
	fn game_dir_key_matches_backend_layouts() {
		assert_eq!(
			game_dir_key("暮色黄昏_世界大战0.25.1/assets/game/missions"),
			"暮色黄昏_世界大战0.25.1/assets/game"
		);
		assert_eq!(
			game_dir_key("包名/assets/map/Earth3/scenarios/TheGreatWar/missions"),
			"包名/assets/game"
		);
		assert_eq!(game_dir_key("assets/game/missions"), "assets/game");
		assert_eq!(game_dir_key("missions"), "");
		assert_eq!(game_dir_key("foo/missions"), "foo");
	}

	#[test]
	fn value_parts_covers_new_lookup_fields() {
		let spirit: &[ValuePart] = &[ValuePart::NationalSpirit];
		let technology: &[ValuePart] = &[ValuePart::Technology];
		let religion: &[ValuePart] = &[ValuePart::Religion];
		let resource: &[ValuePart] = &[ValuePart::Resource, ValuePart::Plain];
		let civ_religion: &[ValuePart] = &[ValuePart::Civ, ValuePart::Religion];
		let event: &[ValuePart] = &[ValuePart::EventId];
		let music: &[ValuePart] = &[ValuePart::Music];
		let province_civ: &[ValuePart] = &[ValuePart::Province, ValuePart::Civ];

		assert_eq!(value_parts("add_ns"), Some(spirit));
		assert_eq!(value_parts("remove_ns"), Some(spirit));
		assert_eq!(value_parts("unlock_tech"), Some(technology));
		assert_eq!(value_parts("change_religion"), Some(religion));
		assert_eq!(value_parts("province_religion_all"), Some(religion));
		assert_eq!(value_parts("change_religion_civ"), Some(civ_religion));
		assert_eq!(value_parts("resource_price_change"), Some(resource));
		assert_eq!(value_parts("resource_price_change_random"), Some(resource));
		assert_eq!(value_parts("run_event"), Some(event));
		assert_eq!(value_parts("run_event_instantly"), Some(event));
		assert_eq!(value_parts("musicName"), Some(music));
		assert_eq!(value_parts("play_music"), Some(music));
		// 文档更正：核心相关字段第二段是文明 ID（而非数字）。
		assert_eq!(value_parts("province_core_of"), Some(province_civ));
		assert_eq!(value_parts("province_id_core_add"), Some(province_civ));
		assert_eq!(value_parts("province_id_core_remove"), Some(province_civ));
		// datalist 归属。
		assert_eq!(
			datalist_id(ValuePart::NationalSpirit),
			NATIONAL_SPIRIT_DATALIST_ID
		);
		assert_eq!(datalist_id(ValuePart::Technology), TECHNOLOGY_DATALIST_ID);
		assert_eq!(datalist_id(ValuePart::Religion), RELIGION_DATALIST_ID);
		assert_eq!(datalist_id(ValuePart::Resource), RESOURCE_DATALIST_ID);
		assert_eq!(datalist_id(ValuePart::EventId), EVENT_DATALIST_ID);
		assert_eq!(datalist_id(ValuePart::Music), MUSIC_DATALIST_ID);
	}

	#[test]
	fn name_hint_maps_new_lookup_fields() {
		let lookup = sample_lookup();
		let parts = value_parts("add_ns").unwrap();
		assert_eq!(name_hint(&lookup, parts, "fra1"), "fra1=法兰西万岁");
		assert_eq!(name_hint(&lookup, parts, "fr"), "");
		let parts = value_parts("unlock_tech").unwrap();
		assert_eq!(name_hint(&lookup, parts, "15"), "15=机械化");
		assert_eq!(name_hint(&lookup, parts, "99"), "");
		let parts = value_parts("change_religion").unwrap();
		assert_eq!(name_hint(&lookup, parts, "1"), "1=Catholic");
		let parts = value_parts("change_religion_civ").unwrap();
		assert_eq!(name_hint(&lookup, parts, "fra=0"), "fra=法兰西 · 0=Pagan");
		let parts = value_parts("resource_price_change").unwrap();
		assert_eq!(name_hint(&lookup, parts, "1=5"), "1=Rice");
		let parts = value_parts("province_id_core_add").unwrap();
		assert_eq!(name_hint(&lookup, parts, "3475=atr"), "3475=北平 · atr=奥地利");
		// 事件 / 音乐的值本身就是名称，不产生对照提示。
		let parts = value_parts("run_event").unwrap();
		assert_eq!(name_hint(&lookup, parts, "1919年巴西大选补选"), "");
		let parts = value_parts("play_music").unwrap();
		assert_eq!(name_hint(&lookup, parts, "世界大战"), "");
	}

	#[test]
	fn lookup_filter_supports_new_tables() {
		let lookup = sample_lookup();
		let matched = matching_lookup(&lookup, ValuePart::NationalSpirit, "fra", 10);
		assert_eq!(matched[0], ("fra1".to_string(), "法兰西万岁".to_string()));
		let matched = matching_lookup(&lookup, ValuePart::NationalSpirit, "苏维埃", 10);
		assert_eq!(matched, vec![("sov3".to_string(), "苏维埃精神".to_string())]);
		let matched = matching_lookup(&lookup, ValuePart::Technology, "毒气", 10);
		assert_eq!(matched, vec![("0".to_string(), "毒气科技".to_string())]);
		let matched = matching_lookup(&lookup, ValuePart::Religion, "cath", 10);
		assert_eq!(matched, vec![("1".to_string(), "Catholic".to_string())]);
		let matched = matching_lookup(&lookup, ValuePart::Resource, "grain", 10);
		assert_eq!(matched, vec![("0".to_string(), "Grain".to_string())]);
		// 事件 / 音乐候选不走对照表（由 list_event_assets 加载）。
		assert!(matching_lookup(&lookup, ValuePart::EventId, "x", 10).is_empty());
		assert!(matching_lookup(&lookup, ValuePart::Music, "x", 10).is_empty());
	}

	#[test]
	fn value_parts_covers_image_fields() {
		let image: &[ValuePart] = &[ValuePart::Image];
		assert_eq!(value_parts("image"), Some(image));
		assert_eq!(value_parts("mission_image"), Some(image));
		assert_eq!(value_parts("decision_image"), Some(image));
		assert_eq!(datalist_id(ValuePart::Image), IMAGE_DATALIST_ID);
		assert_eq!(part_suffix(ValuePart::Image), "");
		// 图片值不参与「值 → 名称」对照。
		let lookup = sample_lookup();
		assert_eq!(name_hint(&lookup, image, "国策通用.png"), "");
	}

	#[test]
	fn name_filter_ranks_prefix_then_contains() {
		let names: Vec<String> = ["国策通用.png", "通用图标.png", "tww.png", "map.png"]
			.iter()
			.map(|name| name.to_string())
			.collect();
		let matched = filter_names(&names, "通用", 10);
		// 前缀优先：通用图标.png 排在 国策通用.png 之前。
		assert_eq!(matched[0].0, "通用图标.png");
		assert!(matched.iter().any(|(value, _)| value == "国策通用.png"));
		// 空查询取前 limit 条；无匹配为空；标签均为空。
		assert_eq!(filter_names(&names, "", 2).len(), 2);
		assert!(filter_names(&names, "zzz", 10).is_empty());
		assert!(matched.iter().all(|(_, label)| label.is_empty()));
	}

	#[test]
	fn scenario_map_key_extracts_map_folder() {
		assert_eq!(
			scenario_map_key("包名/assets/map/Earth3/scenarios/TheGreatWar/missions"),
			"Earth3"
		);
		// 全局根 / 经典根：无地图上下文。
		assert_eq!(scenario_map_key("包名/assets/game/missions"), "");
		assert_eq!(scenario_map_key("missions"), "");
	}

	#[test]
	fn lookup_filter_ranks_prefix_then_contains_then_label() {
		let lookup = sample_lookup();
		// 值前缀优先；「fra」前缀命中 fra，其余（含说明/名称包含）排后。
		let matched = matching_lookup(&lookup, ValuePart::Civ, "fra", 10);
		assert_eq!(matched[0], ("fra".to_string(), "法兰西".to_string()));
		// 名称包含：输入中文命中对应 tag。
		let matched = matching_lookup(&lookup, ValuePart::Civ, "奥", 10);
		assert_eq!(matched, vec![("atr".to_string(), "奥地利".to_string())]);
		// 大小写不敏感。
		let matched = matching_lookup(&lookup, ValuePart::Civ, "AT", 10);
		assert_eq!(matched[0], ("atr".to_string(), "奥地利".to_string()));
		// 无匹配时为空。
		assert!(matching_lookup(&lookup, ValuePart::Civ, "zzz", 10).is_empty());
	}

	#[test]
	fn lookup_filter_handles_ids_names_and_limits() {
		let lookup = sample_lookup();
		// 省份：数字前缀。
		let matched = matching_lookup(&lookup, ValuePart::Province, "347", 10);
		assert_eq!(matched[0], ("3475".to_string(), "北平".to_string()));
		// 省份：地名包含（空名称条目仍可命中 ID）。
		let matched = matching_lookup(&lookup, ValuePart::Province, "12049", 10);
		assert_eq!(matched, vec![("12049".to_string(), String::new())]);
		// 政体 / 建筑 / 疾病 / 人物。
		let matched = matching_lookup(&lookup, ValuePart::Gov, "专制", 10);
		assert_eq!(matched, vec![("10".to_string(), "专制主义".to_string())]);
		let matched = matching_lookup(&lookup, ValuePart::Building, "要塞", 10);
		assert_eq!(matched, vec![("6".to_string(), "要塞".to_string())]);
		let matched = matching_lookup(&lookup, ValuePart::Disease, "0", 10);
		assert_eq!(matched[0], ("0".to_string(), "黑死病".to_string()));
		let matched = matching_lookup(&lookup, ValuePart::Character, "霞", 10);
		assert_eq!(matched, vec![("约瑟夫·霞飞".to_string(), String::new())]);
		// 空查询取前 limit 条；Plain 无候选。
		let matched = matching_lookup(&lookup, ValuePart::Civ, "", 2);
		assert_eq!(matched.len(), 2);
		assert!(matching_lookup(&lookup, ValuePart::Plain, "atr", 10).is_empty());
	}

	#[test]
	fn key_suggestion_filter_matches_value_and_note() {
		let options = vec![
			("id".to_string(), "事件唯一标识".to_string()),
			("image".to_string(), "国策显示照片".to_string()),
			("mission_image".to_string(), "国策完成照片".to_string()),
		];
		// 值前缀优先于值包含。
		let matched = filter_suggestions(&options, "image", 10);
		assert_eq!(matched[0].0, "image");
		assert_eq!(matched[1].0, "mission_image");
		// 说明包含。
		let matched = filter_suggestions(&options, "完成", 10);
		assert_eq!(matched, vec![("mission_image".to_string(), "国策完成照片".to_string())]);
		// 空查询取前 limit 条（保持原顺序）。
		let matched = filter_suggestions(&options, "", 2);
		assert_eq!(matched.len(), 2);
		assert_eq!(matched[0].0, "id");
	}
}
