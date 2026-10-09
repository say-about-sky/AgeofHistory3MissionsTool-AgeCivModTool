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

/// 法令组条目（`laws/Laws.json` 数组顺序即组号；`options` 顺序即组内选项号）。
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct LawItem {
	pub title: String,
	#[serde(default)]
	pub options: Vec<String>,
}

/// 决议条目（`rainfall/rfEvent_decision.json`；`id` 为脚本取值，`events` 供复合键补全）。
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct DecisionItem {
	pub id: String,
	pub name: String,
	#[serde(default)]
	pub events: Vec<String>,
}

/// 兵种条目（`units/Units.json` 的 `ID` + 型号名称表；`add_new_army`）。
/// `armies` 的数组顺序即型号 ID（值的奇数段）。
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct UnitItem {
	pub id: u32,
	pub name: String,
	#[serde(default)]
	pub armies: Vec<String>,
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
	/// 资源清单（price_change 系列）。
	#[serde(default)]
	pub resources: Vec<IdNameItem>,
	/// 法令清单（`laws/Laws.json` 数组顺序即组号；`change_law`）。
	#[serde(default)]
	pub laws: Vec<LawItem>,
	/// 大洲清单（`<地图>/Continents.json` 数组顺序即编号；`civ_capital_continent_is`）。
	#[serde(default)]
	pub continents: Vec<IdNameItem>,
	/// 决议清单（`rainfall/rfEvent_decision.json`；`add_decision` / `taking_decision` 等）。
	#[serde(default)]
	pub decisions: Vec<DecisionItem>,
	/// 兵种清单（`units/Units.json`；`add_new_army` 的兵种与型号两段）。
	#[serde(default)]
	pub units: Vec<UnitItem>,
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
			&& self.laws.is_empty()
			&& self.continents.is_empty()
			&& self.decisions.is_empty()
			&& self.units.is_empty()
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

	/// 按编号查大洲名称。
	pub fn continent_name(&self, index: u32) -> Option<&str> {
		name_of(&self.continents, index)
	}

	/// 按 id 查决议名称（ASCII 大小写不敏感）。
	pub fn decision_name(&self, id: &str) -> Option<&str> {
		self.decisions
			.iter()
			.find(|item| item.id.eq_ignore_ascii_case(id))
			.map(|item| item.name.as_str())
	}

	/// 按 ID 查兵种名称（名称为空时返回 None）。
	pub fn unit_name(&self, id: u32) -> Option<&str> {
		self.units
			.iter()
			.find(|item| item.id == id)
			.map(|item| item.name.as_str())
			.filter(|name| !name.is_empty())
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
	/// 省份 ID 分号列表段（token 化补全；文本原样显示/编辑，选中补全时自动续写 `;`）。
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
	/// 资源整数 ID（`Resources.json`；`price_change` 系列）。
	Resource,
	/// 事件文件名（`run_event` / `run_event_instantly`，值 = 文件名去 `.txt`）。
	EventId,
	/// 音乐名（`musicName` / `play_music`；音频文件名去扩展名或 `list*.txt` 条目）。
	Music,
	/// 图片资源文件名（`image` / `mission_image` / `decision_image`，含 `.png` 扩展名）。
	Image,
	/// 法令组编号（`change_law` 首段；`laws/Laws.json` 数组下标）。
	Law,
	/// 法令组内选项编号（`change_law` 次段；候选项取决于首段组）。
	LawStatus,
	/// 决议 id（`add_decision` / `remove_decision`；可含中文，如 `巴西南美扩张`）。
	Decision,
	/// 决议运行实例复合键 `决策id:事件id`（`taking_decision` / `start_decision`）。
	DecisionRun,
	/// 大洲编号（`civ_capital_continent_is` 等；`Continents.json` 数组下标）。
	Continent,
	/// 统治者头像（`add_ruler` 第三段；值 = 图片文件名去 `.png` 或数字编号）。
	RulerImage,
	/// 特殊联盟编号（`join_alliance_special_id_*` / `leave_alliance_special_id`）。
	AllianceSpecial,
	/// 兵种 ID（`add_new_army` 的偶数段；`units/Units.json` 的 `ID`）。
	Army,
	/// 兵种型号号（`add_new_army` 的奇数段；该兵种文件内 `Army` 数组下标）。
	ArmyLevel,
	/// 顾问类型（`add_advisor`；0 行政 1 经济 2 创新 3 军事）。
	AdvisorType,
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
pub const LAW_DATALIST_ID: &str = "evdl-law";
pub const CONTINENT_DATALIST_ID: &str = "evdl-cont";
pub const DECISION_DATALIST_ID: &str = "evdl-decision";
pub const DECISION_RUN_DATALIST_ID: &str = "evdl-decisionrun";
pub const RULER_IMAGE_DATALIST_ID: &str = "evdl-rulerimg";
pub const ALLIANCE_SPECIAL_DATALIST_ID: &str = "evdl-alliance";
pub const ARMY_DATALIST_ID: &str = "evdl-army";

/// 顾问类型候选（`add_advisor`；FAQ `Events_Outcomes.txt`：0 行政 / 1 经济 / 2 创新 / 3 军事）。
pub const ADVISOR_TYPE_OPTIONS: &[(&str, &str)] = &[
	("0", "行政顾问"),
	("1", "经济顾问"),
	("2", "创新顾问"),
	("3", "军事顾问"),
];

/// 顾问类型编号 → 名称（去首尾空白后精确匹配）。
pub fn advisor_type_name(value: &str) -> Option<&'static str> {
	ADVISOR_TYPE_OPTIONS
		.iter()
		.find(|(number, _)| *number == value.trim())
		.map(|(_, name)| *name)
}

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
		ValuePart::Law => LAW_DATALIST_ID,
		ValuePart::Army => ARMY_DATALIST_ID,
		// 组内选项 / 型号段为上下文候选（自绘下拉按行生成）；顾问类型为固定枚举。
		ValuePart::LawStatus | ValuePart::ArmyLevel | ValuePart::AdvisorType => "",
		ValuePart::Decision => DECISION_DATALIST_ID,
		ValuePart::DecisionRun => DECISION_RUN_DATALIST_ID,
		ValuePart::Continent => CONTINENT_DATALIST_ID,
		ValuePart::RulerImage => RULER_IMAGE_DATALIST_ID,
		ValuePart::AllianceSpecial => ALLIANCE_SPECIAL_DATALIST_ID,
		ValuePart::Plain => "",
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
		"civ_is_vassal_of_civ" | "civs_are_at_war" | "civ_has_more_provinces_than_civ"
		| "civ_has_more_regiments_than_civ" | "civs_are_neighbors"
		| "civs_are_not_at_war" | "civs_are_not_neighbors" | "civs_are_rivals" | "civs_have_alliance"
		| "civs_have_defensive_pact" | "civs_have_non_aggression" | "civs_have_truce"
		| "civ_has_higher_ranking_than_civ" | "civ_has_larger_economy_than_civ"
		| "civ_has_larger_population_than_civ" | "civ_has_larger_regiments_limit_than_civ"
		| "civ_has_more_technologies_than_civ" | "civ_has_rivalry" | "civ_has_rivalry_not"
		| "civ_have_guarantee" | "civ_have_military_access" => &[Civ, Civ],
		// 实测 `ukr=rus=-50.0`（两个文明 + 数值）。
		"civs_opinion_below" | "civs_opinion_over" => &[Civ, Civ, Plain],
		// 傀儡判定：实测模组也直接写文明标签（如 `fin2`），空值为主。
		"is_puppet" | "is_not_puppet" | "exists"
		| "alliance_special_is_member_id" => &[Civ],
		// 取值可为空 / true / false / 省份 ID（实测 151、170）。
		"province_is_occupied" | "province_is_under_siege" | "province_is_capital" => &[Province],
		"province_not_controlled_by" | "province_civ_has_core" => &[Province, Civ],
		"civ_religion_is" | "civ_capital_religion_is" => &[Religion],
		"civ_tag_religion_is" | "civ_tag_religion_is_not" => &[Civ, Religion],
		"civ_government_is" => &[Gov],
		"civ_tag_government_is" | "civ_tag_government_is_not" => &[Civ, Gov],
		// `建筑ID` 单值或 `建筑ID=等级`（第二段自动回退 Plain）。
		"civ_capital_has_building" => &[Building],
		// APK 全局事件（assets/game/events）实测：资源 ID / 资源ID=数值。
		"civ_has_resource" => &[Resource],
		"civ_has_resource_over" | "largest_producer_production_over" => &[Resource, Plain],
		// AoH3 FAQ：`文明某资源产量高于`（资源ID=产量，实测 `civ_production_over=0=50`）。
		"civ_production_over" => &[Resource, Plain],
		"has_variable_civ" => &[Civ, Plain],
		"province_has_building" => &[Province, Building],
		// ===== 收益效果 =====
		"change_ideology" => &[Gov],
		// 实测 `0=LRF`、`23=HEN`（45 处：首段 44/45 为政体序号、次段 40/45 为文明标签），
		// 与 schema 注解「意识形态序号=文明标签」一致——首段政体、次段文明。
		"change_ideology_civ" => &[Gov, Civ],
		"set_civ_tag" | "set_civ_tag_reset" | "player_set_civ" | "annex_civ" | "annexed_by_civ"
		| "declare_war" | "alliance" | "non_aggression_pact" | "military_access" | "vassalize" => {
			&[Civ]
		}
		// 「文明A=文明B」两段（实测 missionsEvents：declare_war2 3181 处、white_peace 3017 处
		// 写作 `==` 即两段皆空，其余为 `tagA=tagB`；空段表示按上下文取默认方）。
		"set_civ_tag2" | "make_puppet" | "declare_war2" | "white_peace" | "white_peace2"
		| "add_defensive_pact" | "add_guarantee" | "add_truce" => &[Civ, Civ],
		"annex_by_civ_from_civ" => &[Civ, Civ, Civ],
		// 实测 `ANF=JAP=15` / `pol=ukr=40`（文明A=文明B=数值）。
		"relations_set" | "relations_change" => &[Civ, Civ, Plain],
		"add_variable2" | "remove_variable2" => &[Civ, Plain],
		// 实测 `rus=rom`、`ming=tume`（双方文明）。
		"add_non_aggression" => &[Civ, Civ],
		// 实测 `CXL_chuan=1567;9497;4369;`（文明=省份列表，右侧 token 化补全）。
		"annex_from_civ" => &[Civ, ProvinceSemi],
		// 实测 `1=CaoRulin`（顾问类型=人物名；首段 0-3 固定枚举）。
		"add_advisor2" => &[AdvisorType, Character],
		// 省份（第二段为数值/未确认语义时按 Plain 处理）
		"move_capital" => &[Province],
		"province_economy_id" | "province_manpower_id" | "province_growth_rate_id"
		| "province_tax_efficiency_id" | "province_devastation_id" | "province_id_pop_set" => {
			&[Province, Plain]
		}
		// AoH3 FAQ（Events_Triggers/Outcomes）数值型省份键：`省份=阈值`（实测 `province_economy_below=464=3.2`）。
		"province_buildings_below" | "province_buildings_over" | "province_buildings_limit_below"
		| "province_buildings_limit_over" | "province_defense_lvl_below" | "province_defense_lvl_over"
		| "province_economy_below" | "province_economy_over" | "province_growth_rate_below"
		| "province_growth_rate_over" | "province_income_below" | "province_income_over"
		| "province_infrastructure_below" | "province_infrastructure_over" | "province_infrastructure_id"
		| "province_manpower_below" | "province_population_below" | "province_tax_efficiency_below"
		| "province_tax_efficiency_over" | "province_unrest_id" => &[Province, Plain],
		"province_religion_is" | "province_religion_is_not" | "province_religion_id" => &[Province, Religion],
		"province_is_not_occupied" => &[Province],
		"province_id_build_add" | "province_id_build_remove" => &[Province, Building],
		"province_id_spread_disease" => &[Province, Disease],
		// 值段以 `;` 结尾的 Lambda 格式：`省份ID;=建筑ID=整数`
		"province_add_building" => &[ProvinceSemi, Building, Plain],
		// 核心省份 / 核打击：`省份列表;=文明标签`（GameCivs 实测 add/remove/nuke
		// 共 256 处全部是「分号列表 + `=` 文明标签」写法，没有裸列表写法）。
		"province_add_core_civ" | "province_remove_core_civ" | "province_nuke" => {
			&[ProvinceSemi, Civ]
		}
		// 吞并省份：值即省份 ID 列表（实测分号列表 90 处如 `404;409;410;`、单值 13 处
		// 如 `217;`/`85`；约 3376 个文件为空占位）。无文明段（对比 `annex_from_civ`）。
		"annex" => &[ProvinceSemi],
		// 人物名称（add_general 旧版也接受 true，候选列表只是建议）
		"add_general" | "add_general2" => &[Character],
		"add_general3" => &[Character, Plain, Plain],
		// 图片资源（值=含扩展名的文件名，如 `国策通用.png`）：候选来自
		// `<missions root>/missionsImages/H` 与 `<game>/events/images/H`（见后端 list_event_assets）。
		"image" | "mission_image" | "decision_image" => &[Image],
		// 国家精神（值 = `NationalSpirit.json` 的字符串 id，如 `fra1`）。
		"add_ns" | "remove_ns" => &[NationalSpirit],
		// 科技 / 宗教 / 资源（值 = 对应数据表的整数 ID）。
		"unlock_tech" => &[Technology],
		// 省份宗教效果（单值 = 宗教编号；FAQ `province_religion=3` / `province_religion_capital=2`）。
		"change_religion" | "province_religion" | "province_religion_capital"
		| "province_religion_all" => &[Religion],
		// 实测 `5=RUS2`、`4=ITA`（8 处全部为「宗教编号=文明标签」），
		// 与 schema 注解一致——首段宗教、次段文明。
		"change_religion_civ" => &[Religion, Civ],
		// APK 全局事件变量体：`价格变化=资源ID=价格…`，首段为资源 ID。
		"price_change" | "price_change_up" | "price_change_down" => &[Resource],
		// 事件（值 = 事件文件名去 `.txt`，如 `1919年巴西大选补选`）。
		"run_event" | "run_event_instantly" => &[EventId],
		// 音乐（值 = 音频文件名去扩展名或 `list*.txt` 清单条目）。
		"musicName" | "play_music" => &[Music],
		// ===== 引擎逆向批次（2026-10-09）：法令 / 决议 / 大洲 / 联盟 / 统治者 =====
		// 引擎实证：lawID=Laws.json 组下标、lawStatus=组内 Law[] 下标。
		"change_law" => &[Law, LawStatus],
		// 决议 id（DecisionSchema；可中文，如 `巴西南美扩张`）。
		"add_decision" | "remove_decision" => &[Decision],
		// 运行实例复合键 `决策id:事件id`（dura 键；实测 `sov左翼社会革命党叛乱:左翼社会革命党企图叛乱`）。
		"taking_decision" | "start_decision" => &[DecisionRun],
		"start_decision2" => &[Civ, DecisionRun],
		// 大洲编号（`Continents.json` 数组顺序；实测 `2`=欧洲、`6`=非洲）。
		"civ_capital_continent_is" | "civ_capital_continent_is_not" => &[Continent],
		// `名字=姓氏=头像=日=月=年`（FAQ Events_Outcomes 实证；头像可写数字或图片名）。
		"add_ruler" => &[Plain, Plain, RulerImage, Plain, Plain, Plain],
		// `文明标签=该文明 rulers/<tag>.json 的 Rulers 数组下标`（0 基）。
		"add_ruler_custom" => &[Civ, Plain],
		// 0 行政 1 经济 2 创新 3 军事（FAQ：Civilization will get Random X Advisor）。
		"add_advisor" => &[AdvisorType],
		// 特殊联盟编号（剧情 `AlliancesSpecial.json` 数组下标）。
		"join_alliance_special_id_first_tier" | "join_alliance_special_id_second_tier"
		| "leave_alliance_special_id" => &[AllianceSpecial],
		// 精确日期 `日=月=年`（三段数值；如 `15=8=1917`）。
		"exact_day" => &[Plain, Plain, Plain],
		_ => return None,
	};
	Some(parts)
}

/// `add_new_army` 的分段模式：`兵种=型号` 成对重复。段数按实际值长从本表截取
/// （短值不会渲染成一长排空输入框）；缓冲区 96 段（五模组实测最长 86 段，超出按普通文本）。
static ADD_ARMY_PARTS: [ValuePart; 96] = {
	let mut parts = [ValuePart::Army; 96];
	let mut index = 0;
	while index < 96 {
		parts[index] = if index % 2 == 0 {
			ValuePart::Army
		} else {
			ValuePart::ArmyLevel
		};
		index += 1;
	}
	parts
};

/// 静态 [`value_parts`] 无法表达的两种分段形态（同一键「单值 / 成对序列」共用）：
/// - `unlock_tech`：含 `=` 时按 `文明=科技编号`（FAQ `Events_Outcomes`：`JAP=95`）；单值形态走静态表 `[Technology]`；
/// - `add_new_army`：`兵种=型号` 成对重复，段数随值长（`0=1=3=2` → 四段；空值/单段 → 一段兵种）。
///
/// 当前只被表格行的分段渲染使用（`event_grid::GridRow`）：动态结果优先，`None` 落回静态表。
pub fn value_parts_dynamic(key: &str, value: &str) -> Option<&'static [ValuePart]> {
	match key {
		"unlock_tech" if value.contains('=') => Some(&[ValuePart::Civ, ValuePart::Technology]),
		"add_new_army" => {
			let count = value.split('=').count().min(ADD_ARMY_PARTS.len());
			Some(&ADD_ARMY_PARTS[..count])
		}
		_ => None,
	}
}

/// 替换 `value` 中以 `=` 分隔的第 `index` 段（不足处补空段），返回拼接结果。
/// 段文本原样写入：省份列表的尾随 `;` 由文本自身携带（不再有隐藏后缀机制）。
pub fn replace_value_part(value: &str, index: usize, new_part: &str) -> String {
	let mut parts: Vec<String> = value.split('=').map(str::to_string).collect();
	while parts.len() <= index {
		parts.push(String::new());
	}
	parts[index] = new_part.to_string();
	parts.join("=")
}

/// `add_new_army` 的段序列（空值 → 空表；否则按 `=` 切分，尾随空段原样保留）。
/// 成对行编辑器（`event_grid::ArmyValueInput`）以它为编辑状态。
pub fn army_segments(value: &str) -> Vec<String> {
	if value.is_empty() {
		Vec::new()
	} else {
		value.split('=').map(str::to_string).collect()
	}
}

/// 设置 `add_new_army` 值中第 `index` 段（不足处补空段，其余段原样保留）。
pub fn set_army_segment(value: &str, index: usize, new_part: &str) -> String {
	let mut segments = army_segments(value);
	while segments.len() <= index {
		segments.push(String::new());
	}
	segments[index] = new_part.to_string();
	segments.join("=")
}

/// 删除 `add_new_army` 值的第 `row` 对（两段；末尾孤立段按一段删除；越界不动）。
pub fn remove_army_pair(value: &str, row: usize) -> String {
	let mut segments = army_segments(value);
	let start = row * 2;
	if start < segments.len() {
		let end = (start + 2).min(segments.len());
		segments.drain(start..end);
	}
	segments.join("=")
}

/// 追加一对：空值先归一为单空段再补一段（→ `=`）；非空追加一段（→ `…=`，
/// 行编辑器里即新增一行空的「兵种 / 型号」）。
pub fn append_army_pair(value: &str) -> String {
	let mut segments = army_segments(value);
	if segments.is_empty() {
		segments.push(String::new());
	}
	segments.push(String::new());
	segments.join("=")
}

/// 段内最后一个 token（按 `;` 切分）——省份列表段的补全查询目标。
/// 文本以 `;` 结尾时返回空串（表示「准备输入下一个 token」）。
pub fn last_token(text: &str) -> &str {
	match text.rsplit_once(';') {
		Some((_, token)) => token,
		None => text,
	}
}

/// 把段内最后一个 token 替换为 `token`（保留前缀与分隔符）。
pub fn replace_last_token(text: &str, token: &str) -> String {
	match text.rsplit_once(';') {
		Some((prefix, _)) => format!("{prefix};{token}"),
		None => token.to_string(),
	}
}

/// `add_ruler` 复合值的单段替换：保持 `名字=姓氏=头像=日=月=年` 顺序，不足处补空段；
/// 写回时裁去尾部空段（至少保留一段，与旧分段编辑行为一致）。
/// 供专用复合编辑器 [`super::event_grid`] 回写使用。
pub fn replace_ruler_segment(value: &str, index: usize, new_part: &str) -> String {
	let mut parts: Vec<String> = value.split('=').map(str::to_string).collect();
	while parts.len() <= index {
		parts.push(String::new());
	}
	parts[index] = new_part.to_string();
	while parts.len() > 1 && parts.last().is_some_and(|part| part.is_empty()) {
		parts.pop();
	}
	parts.join("=")
}

/// 下拉选中省份列表段的一个候选：替换最后一个 token；段已是列表风格（含 `;`）时
/// 自动补尾随 `;`，便于连续选中下一个省份。单值（无 `;`）不补，保持 `省份=数值` 写法。
pub fn select_last_token(text: &str, selected: &str) -> String {
	let mut result = replace_last_token(text, selected);
	if text.contains(';') && !result.ends_with(';') {
		result.push(';');
	}
	result
}

/// 段级最小语法检查（整值校验之外的补充，用于把一个段的红框定位到列表 token）：
/// 省份列表段的每个 token 必须是整数（空 token 容忍，尾随 `;` 属正常写法）；
/// 其余段返回 true（整值校验由 `event_schema::is_valid_value` 负责）。
pub fn part_syntax_ok(kind: ValuePart, text: &str) -> bool {
	match kind {
		ValuePart::Province | ValuePart::ProvinceSemi => text
			.split(';')
			.all(|token| token.trim().is_empty() || token.trim().parse::<i64>().is_ok()),
		_ => true,
	}
}

/// 拼接「值段 → 名称」对照提示（只显示能查到的段，拼写中间状态的段不打扰）。
/// 省份列表段（`Province` / `ProvinceSemi`）在段内按 `;` 分 token 逐个对照；
/// 提示最多显示 12 条，超出以 `…` 收尾（实测列表可达 20+ 省，防止提示条纵贯面板）。
pub fn name_hint(lookup: &EventLookup, parts: &[ValuePart], value: &str) -> String {
	const HINT_MAX_PIECES: usize = 12;
	let mut pieces: Vec<String> = Vec::new();
	// 次段对照用上下文：首段组号（法令）/ 首段兵种 ID（add_new_army 的型号段）。
	let first_segment = value.split('=').next().unwrap_or_default().trim();
	let law_group = first_segment
		.parse::<usize>()
		.ok()
		.and_then(|index| lookup.laws.get(index));
	let army_group = first_segment
		.parse::<u32>()
		.ok()
		.and_then(|id| lookup.units.iter().find(|item| item.id == id));
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
				for token in part.split(';') {
					let token = token.trim();
					if token.is_empty() {
						continue;
					}
					if let Ok(id) = token.parse::<u32>() {
						if let Some(name) = lookup.province_name(id) {
							pieces.push(format!("{token}={name}"));
						}
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
			ValuePart::Law => {
				if let Ok(index) = part.parse::<usize>() {
					if let Some(law) = lookup.laws.get(index) {
						if !law.title.is_empty() {
							pieces.push(format!("{part}={}", law.title));
						}
					}
				}
			}
			ValuePart::LawStatus => {
				if let (Ok(index), Some(group)) = (part.parse::<usize>(), law_group) {
					if let Some(name) = group.options.get(index).filter(|name| !name.is_empty()) {
						pieces.push(format!("{part}={name}"));
					}
				}
			}
			ValuePart::Continent => {
				if let Ok(index) = part.parse::<u32>() {
					if let Some(name) = lookup.continent_name(index) {
						pieces.push(format!("{part}={name}"));
					}
				}
			}
			ValuePart::Decision => {
				if let Some(name) = lookup.decision_name(part) {
					if !name.is_empty() {
						pieces.push(format!("{part}={name}"));
					}
				}
			}
			ValuePart::DecisionRun => {
				if let Some((id, _event)) = part.split_once(':') {
					if let Some(name) = lookup.decision_name(id) {
						if !name.is_empty() {
							pieces.push(format!("{part}={name}"));
						}
					}
				}
			}
			ValuePart::AdvisorType => {
				if let Some(name) = advisor_type_name(part) {
					pieces.push(format!("{part}={name}"));
				}
			}
			ValuePart::Army => {
				if let Ok(id) = part.parse::<u32>() {
					if let Some(name) = lookup.unit_name(id) {
						pieces.push(format!("{part}={name}"));
					}
				}
			}
			ValuePart::ArmyLevel => {
				if let (Ok(level), Some(unit)) = (part.parse::<usize>(), army_group) {
					if let Some(name) = unit.armies.get(level).filter(|name| !name.is_empty()) {
						pieces.push(format!("{part}={name}"));
					}
				}
			}
			// 统治者头像 / 特殊联盟为资源 / 剧本级数据（不经对照表），无名称提示可对照。
			ValuePart::RulerImage | ValuePart::AllianceSpecial => {}
			// 人物 / 事件 / 音乐的值本身就是名称，无需再对照；图片文件名同理（无「值→名称」映射）。
			ValuePart::Character
			| ValuePart::EventId
			| ValuePart::Music
			| ValuePart::Image
			| ValuePart::Plain => {}
		}
	}
	if pieces.len() > HINT_MAX_PIECES {
		pieces.truncate(HINT_MAX_PIECES);
		pieces.push("…".to_string());
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
		// 法令组：值 = 组号，说明 = 组标题（组内选项名命中查询也计入）。
		ValuePart::Law => law_rank_filter(lookup, query, limit),
		// 组内选项 / 型号段 / 顾问类型 / 统治者头像 / 特殊联盟的候选由网格按行上下文构造
		// （`SuggestOptions::Items` / `Names`），不经对照表。
		ValuePart::LawStatus
		| ValuePart::ArmyLevel
		| ValuePart::RulerImage
		| ValuePart::AllianceSpecial
		| ValuePart::AdvisorType => Vec::new(),
		// 兵种：值 = 兵种 ID，说明 = 兵种名（型号名命中查询也计入）。
		ValuePart::Army => army_rank_filter(lookup, query, limit),
		ValuePart::Continent => numbered_rank_filter(
			lookup
				.continents
				.iter()
				.map(|item| (item.id, item.name.as_str())),
			query,
			limit,
		),
		ValuePart::Decision => str_rank_filter(
			lookup
				.decisions
				.iter()
				.map(|item| (item.id.as_str(), item.name.as_str())),
			query,
			limit,
		),
		ValuePart::DecisionRun => decision_run_filter(lookup, query, limit),
		// 事件 / 音乐 / 图片候选不走对照表（由 `list_event_assets` 单独加载，见 `SuggestOptions::Names`）。
		ValuePart::EventId | ValuePart::Music | ValuePart::Image => Vec::new(),
		ValuePart::Plain => Vec::new(),
	}
}

/// 法令组候选：值 = 组号，说明 = 组标题；组内选项名命中查询也计入
/// （排在其标题命中之后，便于「按选项名找组」）。
fn law_rank_filter(lookup: &EventLookup, query: &str, limit: usize) -> Vec<(String, String)> {
	use std::fmt::Write as _;

	if query.is_empty() {
		return lookup
			.laws
			.iter()
			.enumerate()
			.take(limit)
			.map(|(index, law)| (index.to_string(), law.title.clone()))
			.collect();
	}
	let mut scored: Vec<(u8, String, String)> = Vec::new();
	for (index, law) in lookup.laws.iter().enumerate() {
		let mut buffer = String::new();
		let _ = write!(&mut buffer, "{index}");
		let rank = rank_of(&buffer, &law.title, query).or_else(|| {
			law.options
				.iter()
				.find_map(|option| rank_of(&buffer, option, query))
				.map(|rank| rank.saturating_add(2))
		});
		if let Some(rank) = rank {
			scored.push((rank, buffer, law.title.clone()));
		}
	}
	scored.sort_by_key(|(rank, _, _)| *rank);
	scored.truncate(limit);
	scored
		.into_iter()
		.map(|(_, value, label)| (value, label))
		.collect()
}

/// 决议运行实例候选：值 = `决策id:事件id`（dura 键），说明 = `决策名（事件名）`；
/// 查询同时匹配 id / 名称 / 事件名。
fn decision_run_filter(lookup: &EventLookup, query: &str, limit: usize) -> Vec<(String, String)> {
	if query.is_empty() {
		let mut items = Vec::new();
		'outer: for decision in &lookup.decisions {
			for event in &decision.events {
				items.push((
					format!("{}:{}", decision.id, event),
					format!("{}（{}）", decision.name, event),
				));
				if items.len() >= limit {
					break 'outer;
				}
			}
		}
		return items;
	}
	let mut scored: Vec<(u8, String, String)> = Vec::new();
	for decision in &lookup.decisions {
		for event in &decision.events {
			let value = format!("{}:{}", decision.id, event);
			let label = format!("{}（{}）", decision.name, event);
			if let Some(rank) = rank_of(&value, &label, query) {
				scored.push((rank, value, label));
			}
		}
	}
	scored.sort_by_key(|(rank, _, _)| *rank);
	scored.truncate(limit);
	scored
		.into_iter()
		.map(|(_, value, label)| (value, label))
		.collect()
}

/// 兵种候选：值 = 兵种 ID，说明 = 兵种名（文件名）；型号名命中查询也计入
/// （排在其兵种名命中之后，便于「按型号名找兵种」，如输入「弓箭手」命中 Archer 家族）。
fn army_rank_filter(lookup: &EventLookup, query: &str, limit: usize) -> Vec<(String, String)> {
	use std::fmt::Write as _;

	if query.is_empty() {
		return lookup
			.units
			.iter()
			.take(limit)
			.map(|unit| (unit.id.to_string(), unit.name.clone()))
			.collect();
	}
	let mut scored: Vec<(u8, String, String)> = Vec::new();
	for unit in &lookup.units {
		let mut buffer = String::new();
		let _ = write!(&mut buffer, "{}", unit.id);
		let rank = rank_of(&buffer, &unit.name, query).or_else(|| {
			unit.armies
				.iter()
				.find_map(|army| rank_of(&buffer, army, query))
				.map(|rank| rank.saturating_add(2))
		});
		if let Some(rank) = rank {
			scored.push((rank, buffer, unit.name.clone()));
		}
	}
	scored.sort_by_key(|(rank, _, _)| *rank);
	scored.truncate(limit);
	scored
		.into_iter()
		.map(|(_, value, label)| (value, label))
		.collect()
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
	/// 统治者头像：文件名去 `.png`（`add_ruler` 第三段；含数字编号与文本名）。
	#[serde(default)]
	pub ruler_images: Vec<String>,
	/// 剧情特殊联盟名称（`AlliancesSpecial.json` 顺序即编号；`join/leave_alliance_special_id_*`）。
	#[serde(default)]
	pub alliance_specials: Vec<String>,
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
			laws: vec![
				LawItem {
					title: "征兵法案".into(),
					options: vec!["志愿兵制".into(), "有限征兵".into(), "广泛征兵".into()],
				},
				LawItem {
					title: "领导人的生命".into(),
					options: vec!["我们都是普通人".into(), "领导人永远不死".into()],
				},
			],
			continents: vec![
				IdNameItem { id: 0, name: "Ocean".into() },
				IdNameItem { id: 2, name: "Europe".into() },
			],
			decisions: vec![DecisionItem {
				id: "sov左翼社会革命党叛乱".into(),
				name: "左翼社会革命党叛乱".into(),
				events: vec!["左翼社会革命党企图叛乱".into(), "苏维埃政权存亡".into()],
			}],
			units: vec![
				UnitItem {
					id: 0,
					name: "Warior".into(),
					armies: vec!["新手战士".into(), "战士".into()],
				},
				UnitItem {
					id: 3,
					name: "弓箭手".into(),
					armies: vec!["投石手".into(), "弓箭手".into(), "长弓手".into()],
				},
			],
		}
	}

	#[test]
	fn value_parts_covers_civ_and_gov_fields() {
		let civ: &[ValuePart] = &[ValuePart::Civ];
		let civ_pair: &[ValuePart] = &[ValuePart::Civ, ValuePart::Civ];
		let civ_triple: &[ValuePart] = &[ValuePart::Civ, ValuePart::Civ, ValuePart::Civ];
		let gov: &[ValuePart] = &[ValuePart::Gov];
		let gov_civ: &[ValuePart] = &[ValuePart::Gov, ValuePart::Civ];

		assert_eq!(value_parts("is_civ"), Some(civ));
		assert_eq!(value_parts("annex_by_civ_from_civ"), Some(civ_triple));
		assert_eq!(value_parts("civ_is_vassal_of_civ"), Some(civ_pair));
		assert_eq!(value_parts("change_ideology"), Some(gov));
		// 实测 `0=LRF`（政体序号=文明标签）：首段政体、次段文明。
		assert_eq!(value_parts("change_ideology_civ"), Some(gov_civ));
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
		// 后缀（已取消隐藏机制）与 datalist 归属。
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
		// 政体序号=文明标签（实测 `0=LRF`）。
		let parts = value_parts("change_ideology_civ").unwrap();
		assert_eq!(name_hint(&lookup, parts, "0=ger"), "0=临时政府 · ger=德意志国");
		// 省份 / 建筑 / 疾病（含 `;` 后缀与空名称）。
		let parts = value_parts("move_capital").unwrap();
		assert_eq!(name_hint(&lookup, parts, "3475"), "3475=北平");
		assert_eq!(name_hint(&lookup, parts, "12049"), "");
		let parts = value_parts("province_has_building").unwrap();
		assert_eq!(name_hint(&lookup, parts, "11=6"), "11=里斯本 · 6=要塞");
		let parts = value_parts("province_add_building").unwrap();
		assert_eq!(name_hint(&lookup, parts, "3475;=6=0"), "3475=北平 · 6=要塞");
		// 吞并省份：裸省份列表（含尾随 `;`）。
		let parts = value_parts("annex").unwrap();
		assert_eq!(name_hint(&lookup, parts, "3475;11;"), "3475=北平 · 11=里斯本");
		let parts = value_parts("province_id_spread_disease").unwrap();
		assert_eq!(name_hint(&lookup, parts, "11=1"), "11=里斯本 · 1=天花");
		// 人物名称本身就是值，不产生对照提示。
		let parts = value_parts("add_general2").unwrap();
		assert_eq!(name_hint(&lookup, parts, "约瑟夫·霞飞"), "");
	}

	/// 引擎逆向批次（2026-10-09）：法令 / 决议 / 大洲 / 统治者 / 联盟 / 顾问。
	#[test]
	fn value_parts_covers_reverse_batch_fields() {
		let civ_plain: &[ValuePart] = &[ValuePart::Civ, ValuePart::Plain];
		let plain_triple: &[ValuePart] = &[ValuePart::Plain, ValuePart::Plain, ValuePart::Plain];
		let law: &[ValuePart] = &[ValuePart::Law, ValuePart::LawStatus];
		let decision: &[ValuePart] = &[ValuePart::Decision];
		let decision_run: &[ValuePart] = &[ValuePart::DecisionRun];
		let civ_run: &[ValuePart] = &[ValuePart::Civ, ValuePart::DecisionRun];
		let continent: &[ValuePart] = &[ValuePart::Continent];
		let ruler: &[ValuePart] = &[
			ValuePart::Plain,
			ValuePart::Plain,
			ValuePart::RulerImage,
			ValuePart::Plain,
			ValuePart::Plain,
			ValuePart::Plain,
		];
		let advisor: &[ValuePart] = &[ValuePart::AdvisorType];
		let alliance: &[ValuePart] = &[ValuePart::AllianceSpecial];

		assert_eq!(value_parts("change_law"), Some(law));
		assert_eq!(value_parts("add_decision"), Some(decision));
		assert_eq!(value_parts("remove_decision"), Some(decision));
		assert_eq!(value_parts("taking_decision"), Some(decision_run));
		assert_eq!(value_parts("start_decision"), Some(decision_run));
		assert_eq!(value_parts("start_decision2"), Some(civ_run));
		assert_eq!(value_parts("civ_capital_continent_is"), Some(continent));
		assert_eq!(value_parts("civ_capital_continent_is_not"), Some(continent));
		assert_eq!(value_parts("add_ruler"), Some(ruler));
		assert_eq!(value_parts("add_ruler_custom"), Some(civ_plain));
		assert_eq!(value_parts("add_advisor"), Some(advisor));
		assert_eq!(
			value_parts("join_alliance_special_id_first_tier"),
			Some(alliance)
		);
		assert_eq!(
			value_parts("join_alliance_special_id_second_tier"),
			Some(alliance)
		);
		assert_eq!(value_parts("leave_alliance_special_id"), Some(alliance));
		assert_eq!(value_parts("exact_day"), Some(plain_triple));
		// 省份宗教效果（单值宗教编号）与 add_advisor2 首段顾问类型。
		let religion: &[ValuePart] = &[ValuePart::Religion];
		assert_eq!(value_parts("province_religion"), Some(religion));
		assert_eq!(value_parts("province_religion_capital"), Some(religion));
		let advisor_pair: &[ValuePart] = &[ValuePart::AdvisorType, ValuePart::Character];
		assert_eq!(value_parts("add_advisor2"), Some(advisor_pair));
	}

	/// 动态分段（静态表无法表达的两形态键：`unlock_tech` 双写法 / `add_new_army` 变长对）。
	#[test]
	fn value_parts_dynamic_covers_army_and_two_form_tech() {
		let technology: &[ValuePart] = &[ValuePart::Technology];
		let civ_tech: &[ValuePart] = &[ValuePart::Civ, ValuePart::Technology];
		assert_eq!(value_parts("unlock_tech"), Some(technology));
		assert_eq!(value_parts_dynamic("unlock_tech", "132"), None);
		assert_eq!(value_parts_dynamic("unlock_tech", "JAP=95"), Some(civ_tech));
		let army: &[ValuePart] = &[ValuePart::Army];
		let pair: &[ValuePart] = &[ValuePart::Army, ValuePart::ArmyLevel];
		let triple: &[ValuePart] = &[ValuePart::Army, ValuePart::ArmyLevel, ValuePart::Army];
		assert_eq!(value_parts_dynamic("add_new_army", ""), Some(army));
		assert_eq!(value_parts_dynamic("add_new_army", "0"), Some(army));
		assert_eq!(value_parts_dynamic("add_new_army", "0=1"), Some(pair));
		assert_eq!(value_parts_dynamic("add_new_army", "0=1=3"), Some(triple));
		// 超长值按缓冲区截断（超出段落回普通文本）。
		let long = value_parts_dynamic("add_new_army", &vec!["1"; 130].join("=")).unwrap();
		assert_eq!(long.len(), 96);
		// 其它键返回 None（落回静态表 / 普通输入框）。
		assert_eq!(value_parts_dynamic("gold", "1"), None);
	}

	#[test]
	fn datalist_ids_cover_reverse_batch() {
		assert_eq!(datalist_id(ValuePart::Law), LAW_DATALIST_ID);
		assert_eq!(datalist_id(ValuePart::Decision), DECISION_DATALIST_ID);
		assert_eq!(datalist_id(ValuePart::DecisionRun), DECISION_RUN_DATALIST_ID);
		assert_eq!(datalist_id(ValuePart::Continent), CONTINENT_DATALIST_ID);
		assert_eq!(datalist_id(ValuePart::RulerImage), RULER_IMAGE_DATALIST_ID);
		assert_eq!(
			datalist_id(ValuePart::AllianceSpecial),
			ALLIANCE_SPECIAL_DATALIST_ID
		);
		// 行上下文候选（组内选项 / 型号段 / 顾问类型）无原生 datalist；兵种有。
		assert_eq!(datalist_id(ValuePart::Army), ARMY_DATALIST_ID);
		assert_eq!(datalist_id(ValuePart::LawStatus), "");
		assert_eq!(datalist_id(ValuePart::ArmyLevel), "");
		assert_eq!(datalist_id(ValuePart::AdvisorType), "");
	}

	#[test]
	fn matching_lookup_covers_army_table() {
		let lookup = sample_lookup();
		assert_eq!(
			matching_lookup(&lookup, ValuePart::Army, "", 10),
			vec![
				("0".to_string(), "Warior".to_string()),
				("3".to_string(), "弓箭手".to_string()),
			]
		);
		// 按型号名搜索命中兵种家族（排在其兵种名命中之后）。
		assert_eq!(
			matching_lookup(&lookup, ValuePart::Army, "投石手", 10),
			vec![("3".to_string(), "弓箭手".to_string())]
		);
		assert!(matching_lookup(&lookup, ValuePart::ArmyLevel, "0", 10).is_empty());
	}

	#[test]
	fn name_hint_covers_army_segments() {
		let lookup = sample_lookup();
		let army: &[ValuePart] = &[ValuePart::Army, ValuePart::ArmyLevel];
		assert_eq!(name_hint(&lookup, army, "3=0"), "3=弓箭手 · 0=投石手");
		assert_eq!(name_hint(&lookup, army, "3=2"), "3=弓箭手 · 2=长弓手");
		// 未知兵种 / 越界型号段无提示。
		assert_eq!(name_hint(&lookup, army, "9=0"), "");
		assert_eq!(name_hint(&lookup, army, "3=9"), "3=弓箭手");
		// add_advisor2 首段 = 顾问类型（固定枚举提示）。
		let advisor2: &[ValuePart] = &[ValuePart::AdvisorType, ValuePart::Character];
		assert_eq!(name_hint(&lookup, advisor2, "1=约瑟夫·霞飞"), "1=经济顾问");
	}

	#[test]
	fn matching_lookup_covers_reverse_batch() {
		let lookup = sample_lookup();
		// 法令组：值 = 组号；按组标题或组内选项名搜索。
		assert_eq!(
			matching_lookup(&lookup, ValuePart::Law, "", 10),
			vec![
				("0".to_string(), "征兵法案".to_string()),
				("1".to_string(), "领导人的生命".to_string()),
			]
		);
		assert_eq!(
			matching_lookup(&lookup, ValuePart::Law, "广泛", 10),
			vec![("0".to_string(), "征兵法案".to_string())]
		);
		assert_eq!(
			matching_lookup(&lookup, ValuePart::Law, "领导人", 10),
			vec![("1".to_string(), "领导人的生命".to_string())]
		);
		// 大洲：值 = 编号。
		assert_eq!(
			matching_lookup(&lookup, ValuePart::Continent, "Euro", 10),
			vec![("2".to_string(), "Europe".to_string())]
		);
		// 决议：值 = id。
		assert_eq!(
			matching_lookup(&lookup, ValuePart::Decision, "左翼", 10),
			vec![(
				"sov左翼社会革命党叛乱".to_string(),
				"左翼社会革命党叛乱".to_string()
			)]
		);
		// 决议运行实例：值 = `决策id:事件id`。
		assert_eq!(
			matching_lookup(&lookup, ValuePart::DecisionRun, "", 10),
			vec![
				(
					"sov左翼社会革命党叛乱:左翼社会革命党企图叛乱".to_string(),
					"左翼社会革命党叛乱（左翼社会革命党企图叛乱）".to_string()
				),
				(
					"sov左翼社会革命党叛乱:苏维埃政权存亡".to_string(),
					"左翼社会革命党叛乱（苏维埃政权存亡）".to_string()
				),
			]
		);
		assert_eq!(
			matching_lookup(&lookup, ValuePart::DecisionRun, "政权存亡", 10).len(),
			1
		);
		// 行上下文候选不经对照表。
		assert!(matching_lookup(&lookup, ValuePart::LawStatus, "志愿", 10).is_empty());
	}

	#[test]
	fn name_hint_covers_reverse_batch() {
		let lookup = sample_lookup();
		let law_parts: &[ValuePart] = &[ValuePart::Law, ValuePart::LawStatus];
		assert_eq!(
			name_hint(&lookup, law_parts, "1=1"),
			"1=领导人的生命 · 1=领导人永远不死"
		);
		// 首段缺失 / 越界时次段无提示。
		assert_eq!(name_hint(&lookup, law_parts, "9=0"), "");
		let continent: &[ValuePart] = &[ValuePart::Continent];
		assert_eq!(name_hint(&lookup, continent, "2"), "2=Europe");
		let decision: &[ValuePart] = &[ValuePart::Decision];
		assert_eq!(
			name_hint(&lookup, decision, "sov左翼社会革命党叛乱"),
			"sov左翼社会革命党叛乱=左翼社会革命党叛乱"
		);
		let decision_run: &[ValuePart] = &[ValuePart::DecisionRun];
		assert_eq!(
			name_hint(
				&lookup,
				decision_run,
				"sov左翼社会革命党叛乱:左翼社会革命党企图叛乱"
			),
			"sov左翼社会革命党叛乱:左翼社会革命党企图叛乱=左翼社会革命党叛乱"
		);
		let advisor: &[ValuePart] = &[ValuePart::AdvisorType];
		assert_eq!(name_hint(&lookup, advisor, "3"), "3=军事顾问");
		assert_eq!(name_hint(&lookup, advisor, "9"), "");
	}

	#[test]
	fn replace_ruler_segment_pads_and_trims() {
		// 空值填第 1 段：只留一段。
		assert_eq!(replace_ruler_segment("", 0, "陶尔斐斯"), "陶尔斐斯");
		// 越过现有段数时补空段（保持段序）。
		assert_eq!(replace_ruler_segment("a= b", 2, "c"), "a= b=c");
		// 中间段空值保留（`a==c` 是合法写法）。
		assert_eq!(replace_ruler_segment("a==c", 1, "b"), "a=b=c");
		assert_eq!(replace_ruler_segment("a=b=c", 1, ""), "a==c");
		// 六段全填时原样替换；尾部空段裁除。
		assert_eq!(
			replace_ruler_segment("陶尔斐斯= =陶尔斐斯=4=10=1892", 5, "1890"),
			"陶尔斐斯= =陶尔斐斯=4=10=1890"
		);
		assert_eq!(replace_ruler_segment("=", 0, "x"), "x");
	}

	#[test]
	fn replace_value_part_rebuilds_value() {
		assert_eq!(replace_value_part("fra=ger", 1, "atr"), "fra=atr");
		assert_eq!(replace_value_part("fra", 1, "ger"), "fra=ger");
		assert_eq!(replace_value_part("", 0, "atr"), "atr");
		assert_eq!(replace_value_part("a=b=c", 0, "x"), "x=b=c");
		// 段文本原样写入（省份列表的尾随 `;` 由文本自身携带，不再有隐藏后缀）。
		assert_eq!(replace_value_part("3994;=6=0", 0, "3475;"), "3475;=6=0");
	}

	/// `add_new_army` 成对行编辑器的纯函数：设置 / 删除 / 追加（保留尾随空段）。
	#[test]
	fn army_pair_helpers_edit_rows() {
		assert_eq!(army_segments("0=1=3=2"), vec!["0", "1", "3", "2"]);
		assert!(army_segments("").is_empty());
		// 设置段：不足处补空段。
		assert_eq!(set_army_segment("0=1", 1, "5"), "0=5");
		assert_eq!(set_army_segment("0=1", 3, "7"), "0=1==7");
		assert_eq!(set_army_segment("", 0, "3"), "3");
		// 删除行：整对两段；末尾孤立段按一段删除；越界不动。
		assert_eq!(remove_army_pair("0=1=3=2", 0), "3=2");
		assert_eq!(remove_army_pair("0=1=3=2", 1), "0=1");
		assert_eq!(remove_army_pair("0=1=3", 1), "0=1");
		assert_eq!(remove_army_pair("0=1", 5), "0=1");
		assert_eq!(remove_army_pair("0=1", 0), "");
		// 追一加对：空值 → `=`；非空 → `…=`（新增空行）。
		assert_eq!(append_army_pair("0=1"), "0=1=");
		assert_eq!(append_army_pair(""), "=");
		assert_eq!(append_army_pair("="), "==");
		// 删后追加（行编辑器常见操作序列）。
		assert_eq!(append_army_pair(&remove_army_pair("0=1=3=2", 1)), "0=1=");
		// 尾随空段保留（不因编辑而丢失原文件的填充段）。
		assert_eq!(set_army_segment("0=1=", 0, "7"), "7=1=");
	}

	#[test]
	fn value_parts_covers_province_list_fields() {
		let province_plain: &[ValuePart] = &[ValuePart::Province, ValuePart::Plain];
		let semi: &[ValuePart] = &[ValuePart::ProvinceSemi];
		let semi_civ: &[ValuePart] = &[ValuePart::ProvinceSemi, ValuePart::Civ];
		assert_eq!(value_parts("province_growth_rate_id"), Some(province_plain));
		assert_eq!(value_parts("province_tax_efficiency_id"), Some(province_plain));
		assert_eq!(value_parts("province_add_core_civ"), Some(semi_civ));
		assert_eq!(value_parts("province_remove_core_civ"), Some(semi_civ));
		assert_eq!(value_parts("province_nuke"), Some(semi_civ));
		// 吞并省份：裸省份列表（实测 `404;409;410;`）。
		assert_eq!(value_parts("annex"), Some(semi));
	}

	/// 补全审计第二批：关系三段、文明对、省份、资源/价格、建筑等键的段位注册。
	#[test]
	fn value_parts_covers_second_audit_batch() {
		let civ: &[ValuePart] = &[ValuePart::Civ];
		let civ_pair: &[ValuePart] = &[ValuePart::Civ, ValuePart::Civ];
		let civ_civ_plain: &[ValuePart] = &[ValuePart::Civ, ValuePart::Civ, ValuePart::Plain];
		let civ_plain: &[ValuePart] = &[ValuePart::Civ, ValuePart::Plain];
		let civ_semi: &[ValuePart] = &[ValuePart::Civ, ValuePart::ProvinceSemi];
		let province: &[ValuePart] = &[ValuePart::Province];
		let province_civ: &[ValuePart] = &[ValuePart::Province, ValuePart::Civ];
		let building: &[ValuePart] = &[ValuePart::Building];
		let religion: &[ValuePart] = &[ValuePart::Religion];
		let gov: &[ValuePart] = &[ValuePart::Gov];
		let resource: &[ValuePart] = &[ValuePart::Resource];
		let resource_plain: &[ValuePart] = &[ValuePart::Resource, ValuePart::Plain];
		let advisor_pair: &[ValuePart] = &[ValuePart::AdvisorType, ValuePart::Character];

		assert_eq!(value_parts("relations_set"), Some(civ_civ_plain));
		assert_eq!(value_parts("relations_change"), Some(civ_civ_plain));
		assert_eq!(value_parts("civs_opinion_below"), Some(civ_civ_plain));
		assert_eq!(value_parts("civ_has_more_regiments_than_civ"), Some(civ_pair));
		assert_eq!(value_parts("civs_are_neighbors"), Some(civ_pair));
		assert_eq!(value_parts("add_non_aggression"), Some(civ_pair));
		assert_eq!(value_parts("is_puppet"), Some(civ));
		assert_eq!(value_parts("is_not_puppet"), Some(civ));
		assert_eq!(value_parts("exists"), Some(civ));
		assert_eq!(value_parts("alliance_special_is_member_id"), Some(civ));
		assert_eq!(value_parts("province_is_occupied"), Some(province));
		assert_eq!(value_parts("province_is_under_siege"), Some(province));
		assert_eq!(value_parts("province_is_capital"), Some(province));
		assert_eq!(value_parts("province_not_controlled_by"), Some(province_civ));
		assert_eq!(value_parts("province_civ_has_core"), Some(province_civ));
		assert_eq!(value_parts("civ_religion_is"), Some(religion));
		assert_eq!(value_parts("civ_government_is"), Some(gov));
		assert_eq!(value_parts("civ_capital_has_building"), Some(building));
		assert_eq!(value_parts("civ_has_resource"), Some(resource));
		assert_eq!(value_parts("civ_has_resource_over"), Some(resource_plain));
		assert_eq!(
			value_parts("largest_producer_production_over"),
			Some(resource_plain)
		);
		assert_eq!(value_parts("price_change"), Some(resource));
		assert_eq!(value_parts("price_change_up"), Some(resource));
		assert_eq!(value_parts("price_change_down"), Some(resource));
		assert_eq!(value_parts("annex_from_civ"), Some(civ_semi));
		assert_eq!(value_parts("add_variable2"), Some(civ_plain));
		assert_eq!(value_parts("add_advisor2"), Some(advisor_pair));

		// 三段关系值的名称对照（`文明A=文明B=数值`）。
		let lookup = sample_lookup();
		let parts = value_parts("relations_set").unwrap();
		assert_eq!(
			name_hint(&lookup, parts, "ger=atr=-15"),
			"ger=德意志国 · atr=奥地利"
		);
	}

	/// 省份列表段的 token 拆分 / 最后一个 token 的替换与选中续写。
	#[test]
	fn province_token_helpers_split_and_replace_last_token() {
		assert_eq!(last_token("3475"), "3475");
		assert_eq!(last_token("3475;2278"), "2278");
		// 尾随 `;` = 空尾项（准备输入下一个 token）。
		assert_eq!(last_token("3475;2278;"), "");
		assert_eq!(replace_last_token("3475;2278", "6293"), "3475;6293");
		assert_eq!(replace_last_token("3475;", "6293"), "3475;6293");
		assert_eq!(replace_last_token("3475", "6293"), "6293");
		// 选中：已在列表风格（含 `;`）时自动补 `;` 便于连续追加；单值不补。
		assert_eq!(select_last_token("3475", "6293"), "6293");
		assert_eq!(select_last_token("3475;2278", "6293"), "3475;6293;");
		assert_eq!(select_last_token("3475;2278;", "6293"), "3475;2278;6293;");
		// 刚输入 `;`（尾随空尾项 = 准备下一个 token）时，选中候选应追加新 token。
		assert_eq!(select_last_token("0;", "10"), "0;10;");
		assert_eq!(select_last_token("", "12"), "12");
		// 段级语法：省份列表每个 token 必须是整数（空 token / 尾随 `;` 容忍）。
		assert!(part_syntax_ok(ValuePart::Province, "3475; 2278;"));
		assert!(!part_syntax_ok(ValuePart::Province, "3475;abc"));
		assert!(part_syntax_ok(ValuePart::ProvinceSemi, ""));
		assert!(part_syntax_ok(ValuePart::Civ, "任意文本"));
	}

	/// 省份列表段逐 token 对照名称（尾随 `;` 不干扰；超长列表限 12 条 + `…`）。
	#[test]
	fn name_hint_tokenizes_province_lists() {
		let lookup = sample_lookup();
		let parts = value_parts("province_add_core_civ").unwrap();
		assert_eq!(
			name_hint(&lookup, parts, "3475;11;=atr"),
			"3475=北平 · 11=里斯本 · atr=奥地利"
		);
		let many = format!("{};=atr", vec!["3475"; 20].join(";"));
		let hint = name_hint(&lookup, parts, &many);
		assert_eq!(hint.matches("北平").count(), 12);
		assert!(hint.ends_with('…'));
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
		let religion_civ: &[ValuePart] = &[ValuePart::Religion, ValuePart::Civ];
		let event: &[ValuePart] = &[ValuePart::EventId];
		let music: &[ValuePart] = &[ValuePart::Music];
		let province_civ: &[ValuePart] = &[ValuePart::Province, ValuePart::Civ];

		assert_eq!(value_parts("add_ns"), Some(spirit));
		assert_eq!(value_parts("remove_ns"), Some(spirit));
		assert_eq!(value_parts("unlock_tech"), Some(technology));
		assert_eq!(value_parts("change_religion"), Some(religion));
		assert_eq!(value_parts("province_religion_all"), Some(religion));
		// 实测 `5=RUS2`（宗教编号=文明标签）：首段宗教、次段文明。
		assert_eq!(value_parts("change_religion_civ"), Some(religion_civ));
		let price: &[ValuePart] = &[ValuePart::Resource];
		assert_eq!(value_parts("price_change"), Some(price));
		assert_eq!(value_parts("run_event"), Some(event));
		assert_eq!(value_parts("run_event_instantly"), Some(event));
		assert_eq!(value_parts("musicName"), Some(music));
		assert_eq!(value_parts("play_music"), Some(music));
		// 引擎实测：核心相关触发器为 `province_civ_has_core`（省份=文明）。
		assert_eq!(value_parts("province_civ_has_core"), Some(province_civ));
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
		assert_eq!(name_hint(&lookup, parts, "0=fra"), "0=Pagan · fra=法兰西");
		let parts = value_parts("price_change").unwrap();
		assert_eq!(name_hint(&lookup, parts, "1=5"), "1=Rice");
		let parts = value_parts("province_civ_has_core").unwrap();
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
