//! 国策事件脚本（missionsEvents/*.txt）键位注册表。
//!
//! 键名、值类型与中文注释移植自：
//! - `a:\android\missions_db\ScvGen\src\sqlite.rs`
//! - 项目内《国策系统说明文档.md》
//!
//! 用途："特化 Excel 表格"可视化编辑器根据此表生成控件、
//! 提供中文提示，并对用户输入做类型诊断。

/// 基础值类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
	/// 任意字符串（含空串，用于允许留空的字段）
	Str,
	/// 整数
	Int,
	/// 仅 `true` / `false`
	Bool,
	/// 有限浮点数
	Float,
}

impl ValueType {
	/// 判断单个值是否符合基础类型。
	pub fn accepts(self, value: &str) -> bool {
		let value = value.trim();
		match self {
			Self::Str => true,
			Self::Int => value.parse::<i64>().is_ok(),
			Self::Bool => matches!(value, "true" | "false"),
			Self::Float => value.parse::<f64>().is_ok_and(f64::is_finite),
		}
	}
}

/// 复合值类型，对应 sqlite.rs 中的 `TypeSpec`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueSpec {
	/// 单值，如 `Int`、`Bool`
	Single(ValueType),
	/// 二选一，如 `bool/文明ID`
	OneOf(ValueType, ValueType),
	/// `a=b` 两段
	Pair(ValueType, ValueType),
	/// `a=b=c` 三段
	Triple(ValueType, ValueType, ValueType),
	/// 分号分隔列表，如 `1;2;3`
	List(ValueType),
	/// `a=b=a=b...` 成对序列，如 `add_new_army` 的 类型=数量=...
	Sequence(ValueType, ValueType),
	/// 特殊复合：`列表;=a=b`，如 `province_add_building` 的 `3994;=6=0`
	Lambda(ValueType, ValueType, ValueType),
}

impl ValueSpec {
	pub const fn single(value_type: ValueType) -> Self {
		Self::Single(value_type)
	}
	pub const fn one_of(first: ValueType, second: ValueType) -> Self {
		Self::OneOf(first, second)
	}
	pub const fn pair(first: ValueType, second: ValueType) -> Self {
		Self::Pair(first, second)
	}
	pub const fn triple(first: ValueType, second: ValueType, third: ValueType) -> Self {
		Self::Triple(first, second, third)
	}
	pub const fn list(value_type: ValueType) -> Self {
		Self::List(value_type)
	}
	pub const fn sequence(first: ValueType, second: ValueType) -> Self {
		Self::Sequence(first, second)
	}
	pub const fn lambda(first: ValueType, second: ValueType, third: ValueType) -> Self {
		Self::Lambda(first, second, third)
	}
}

/// 字段分组，编辑器按此分区展示。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldCategory {
	/// 必填项目：id / title / desc
	Required,
	/// 头部可填项目：mission_desc / image / musicName / only_once / show_in_missions
	Header,
	/// 可填项目（对应 sqlite.rs 的 MISSIONS_BUTTON）
	Optional,
	/// 触发条件（trigger 块内）
	Trigger,
	/// 收益效果（option 块内，含 ai）
	Effect,
}

/// 单个键位的注册信息。
pub struct FieldSpec {
	pub key: &'static str,
	pub spec: ValueSpec,
	pub annotation: &'static str,
	pub category: FieldCategory,
}

impl FieldSpec {
	pub const fn new(
		key: &'static str,
		spec: ValueSpec,
		annotation: &'static str,
		category: FieldCategory,
	) -> Self {
		Self {
			key,
			spec,
			annotation,
			category,
		}
	}
}

// ===== 必填项目 =====

pub const REQUIRED_FIELDS: &[FieldSpec] = &[
	FieldSpec::new("id", ValueSpec::single(ValueType::Str), "事件唯一标识（通常与国策名相同）", FieldCategory::Required),
	FieldSpec::new("title", ValueSpec::single(ValueType::Str), "事件弹窗标题（可留空）", FieldCategory::Required),
	FieldSpec::new("desc", ValueSpec::single(ValueType::Str), "事件描述文本（可留空）", FieldCategory::Required),
];

// ===== 头部可填项目 =====

pub const HEADER_FIELDS: &[FieldSpec] = &[
	FieldSpec::new("mission_desc", ValueSpec::single(ValueType::Str), "国策详细描述文本（可留空）", FieldCategory::Header),
	FieldSpec::new("image", ValueSpec::single(ValueType::Str), "国策显示照片，如 xxx.png（200x130）", FieldCategory::Header),
	FieldSpec::new("musicName", ValueSpec::single(ValueType::Str), "背景音乐名称（可留空）", FieldCategory::Header),
	FieldSpec::new("only_once", ValueSpec::single(ValueType::Bool), "是否只能执行一次", FieldCategory::Header),
	FieldSpec::new("show_in_missions", ValueSpec::single(ValueType::Bool), "是否在国策树界面显示", FieldCategory::Header),
];

// ===== 可填项目（sqlite.rs MISSIONS_BUTTON） =====

pub const OPTIONAL_FIELDS: &[FieldSpec] = &[
	FieldSpec::new("mission_image", ValueSpec::single(ValueType::Str), "国策完成照片（200x130）.png", FieldCategory::Optional),
	FieldSpec::new("popUp", ValueSpec::single(ValueType::Bool), "完成时是否弹窗", FieldCategory::Optional),
	FieldSpec::new("pobssible_to_run", ValueSpec::single(ValueType::Bool), "是否可执行（官方拼写，勿改）", FieldCategory::Optional),
	FieldSpec::new("no_background", ValueSpec::single(ValueType::Bool), "是否无背景", FieldCategory::Optional),
	FieldSpec::new("no_text", ValueSpec::single(ValueType::Bool), "是否不显示文本", FieldCategory::Optional),
	FieldSpec::new("run_in_background", ValueSpec::single(ValueType::Bool), "是否在后台运行", FieldCategory::Optional),
	FieldSpec::new("important", ValueSpec::single(ValueType::Bool), "是否重要", FieldCategory::Optional),
	FieldSpec::new("super_event", ValueSpec::single(ValueType::Bool), "是否为超级事件", FieldCategory::Optional),
	FieldSpec::new("focus_dura", ValueSpec::single(ValueType::Int), "国策执行所需时长/天", FieldCategory::Optional),
	FieldSpec::new("showCondition", ValueSpec::single(ValueType::Str), "显示条件表达式（可留空）", FieldCategory::Optional),
	FieldSpec::new("decisionCiv", ValueSpec::single(ValueType::Int), "决策文明ID", FieldCategory::Optional),
	FieldSpec::new("decisionDura", ValueSpec::single(ValueType::Int), "决策时长/天", FieldCategory::Optional),
	FieldSpec::new("execPosition", ValueSpec::single(ValueType::Int), "执行位置", FieldCategory::Optional),
	FieldSpec::new("layoutID", ValueSpec::single(ValueType::Int), "布局ID", FieldCategory::Optional),
	FieldSpec::new("preprocessorID", ValueSpec::single(ValueType::Int), "预处理器ID", FieldCategory::Optional),
	FieldSpec::new("runCivsID", ValueSpec::single(ValueType::Int), "运行文明ID", FieldCategory::Optional),
];

// ===== 触发条件（sqlite.rs MISSIONS_TRIGGER_BUTTON + 说明文档） =====

pub const TRIGGER_FIELDS: &[FieldSpec] = &[
	FieldSpec::new("is_civ", ValueSpec::single(ValueType::Str), "指定文明（如 atr、spa）", FieldCategory::Trigger),
	FieldSpec::new("is_player", ValueSpec::one_of(ValueType::Bool, ValueType::Str), "是否为玩家（可指定文明）", FieldCategory::Trigger),
	FieldSpec::new("is_not_player", ValueSpec::one_of(ValueType::Bool, ValueType::Str), "是否非玩家（可指定文明）", FieldCategory::Trigger),
	FieldSpec::new("is_puppet", ValueSpec::single(ValueType::Bool), "是否为傀儡国", FieldCategory::Trigger),
	FieldSpec::new("is_not_puppet", ValueSpec::single(ValueType::Bool), "是否非傀儡国", FieldCategory::Trigger),
	FieldSpec::new("civ_is_at_war", ValueSpec::single(ValueType::Bool), "是否处于战争状态", FieldCategory::Trigger),
	FieldSpec::new("civ_is_not_at_war", ValueSpec::single(ValueType::Bool), "是否不处于战争状态", FieldCategory::Trigger),
	FieldSpec::new("civ_is_at_war_days_over", ValueSpec::single(ValueType::Int), "战争持续天数超过此值", FieldCategory::Trigger),
	FieldSpec::new("civ_is_vassal_of_civ", ValueSpec::pair(ValueType::Str, ValueType::Str), "某文明是否为另一文明的附属国（如 bah=atr）", FieldCategory::Trigger),
	FieldSpec::new("exists_any", ValueSpec::single(ValueType::Str), "存在某文明", FieldCategory::Trigger),
	FieldSpec::new("exists_any_not", ValueSpec::single(ValueType::Str), "不存在某文明", FieldCategory::Trigger),
	FieldSpec::new("province_controlled_by", ValueSpec::single(ValueType::Str), "省份被某文明控制", FieldCategory::Trigger),
	FieldSpec::new("province_is_occupied", ValueSpec::single(ValueType::Bool), "省份是否被占领", FieldCategory::Trigger),
	FieldSpec::new("province_is_under_siege", ValueSpec::single(ValueType::Bool), "省份是否被围困", FieldCategory::Trigger),
	FieldSpec::new("has_variable", ValueSpec::single(ValueType::Str), "拥有某变量/标签", FieldCategory::Trigger),
	FieldSpec::new("has_variable_not", ValueSpec::single(ValueType::Str), "没有某变量/标签", FieldCategory::Trigger),
	FieldSpec::new("has_variable_civ", ValueSpec::pair(ValueType::Str, ValueType::Str), "某文明拥有某变量", FieldCategory::Trigger),
	FieldSpec::new("civ_capital_unrest_over", ValueSpec::single(ValueType::Float), "首都动荡度高于此值（如 0.7、5.2）", FieldCategory::Trigger),
	FieldSpec::new("civ_capital_unrest_below", ValueSpec::single(ValueType::Float), "首都动荡度低于此值（如 0.01）", FieldCategory::Trigger),
	FieldSpec::new("civ_prestige_over", ValueSpec::single(ValueType::Int), "威望高于此值", FieldCategory::Trigger),
	FieldSpec::new("civ_provinces_over", ValueSpec::single(ValueType::Int), "控制省份数高于此值", FieldCategory::Trigger),
	FieldSpec::new("civ_provinces_below", ValueSpec::single(ValueType::Int), "控制省份数低于此值", FieldCategory::Trigger),
	FieldSpec::new("civ_conquered_provinces_over", ValueSpec::single(ValueType::Int), "征服省份数高于此值（如 750、1000）", FieldCategory::Trigger),
	FieldSpec::new("civ_regiments_over", ValueSpec::single(ValueType::Int), "军团数高于此值", FieldCategory::Trigger),
	FieldSpec::new("civ_wars_total_over", ValueSpec::single(ValueType::Int), "战争总数超过此值", FieldCategory::Trigger),
	FieldSpec::new("civ_capital_continent_is", ValueSpec::single(ValueType::Int), "首都所在大洲（2=欧洲）", FieldCategory::Trigger),
	FieldSpec::new("civs_are_at_war", ValueSpec::pair(ValueType::Str, ValueType::Str), "两个文明是否处于战争状态（如 fra=ger）", FieldCategory::Trigger),
	FieldSpec::new("exact_day", ValueSpec::triple(ValueType::Int, ValueType::Int, ValueType::Int), "精确日期条件（如 15=8=1917 表示 1917 年 8 月 15 日）", FieldCategory::Trigger),
	FieldSpec::new("playing_time_over", ValueSpec::single(ValueType::Int), "游戏时间超过此值（天）", FieldCategory::Trigger),
	FieldSpec::new("civ_is_in_civil_war", ValueSpec::single(ValueType::Bool), "是否处于内战", FieldCategory::Trigger),
	FieldSpec::new("civ_has_truce_with", ValueSpec::single(ValueType::Str), "是否与某文明停战", FieldCategory::Trigger),
	FieldSpec::new("civ_num_of_provinces_over", ValueSpec::single(ValueType::Int), "省份数量超过", FieldCategory::Trigger),
	FieldSpec::new("civ_num_of_provinces_below", ValueSpec::single(ValueType::Int), "省份数量低于", FieldCategory::Trigger),
	FieldSpec::new("civ_num_of_cities_over", ValueSpec::single(ValueType::Int), "城市数量超过", FieldCategory::Trigger),
	FieldSpec::new("civ_num_of_cities_below", ValueSpec::single(ValueType::Int), "城市数量低于", FieldCategory::Trigger),
	FieldSpec::new("civ_total_population_over", ValueSpec::single(ValueType::Int), "总人口超过", FieldCategory::Trigger),
	FieldSpec::new("civ_total_population_below", ValueSpec::single(ValueType::Int), "总人口低于", FieldCategory::Trigger),
	FieldSpec::new("civ_manpower_over", ValueSpec::single(ValueType::Int), "人力超过", FieldCategory::Trigger),
	FieldSpec::new("civ_manpower_below", ValueSpec::single(ValueType::Int), "人力低于", FieldCategory::Trigger),
	FieldSpec::new("civ_gold_over", ValueSpec::single(ValueType::Int), "金币超过", FieldCategory::Trigger),
	FieldSpec::new("civ_gold_below", ValueSpec::single(ValueType::Int), "金币低于", FieldCategory::Trigger),
	FieldSpec::new("civ_legacy_over", ValueSpec::single(ValueType::Int), "威望超过此值", FieldCategory::Trigger),
	FieldSpec::new("civ_legacy_below", ValueSpec::single(ValueType::Int), "威望低于此值", FieldCategory::Trigger),
	FieldSpec::new("civ_stability_over", ValueSpec::single(ValueType::Int), "稳定度超过此值", FieldCategory::Trigger),
	FieldSpec::new("civ_stability_below", ValueSpec::single(ValueType::Int), "稳定度低于此值", FieldCategory::Trigger),
	FieldSpec::new("civ_war_support_over", ValueSpec::single(ValueType::Int), "战争支持度超过此值", FieldCategory::Trigger),
	FieldSpec::new("civ_war_support_below", ValueSpec::single(ValueType::Int), "战争支持度低于此值", FieldCategory::Trigger),
	FieldSpec::new("province_has_building", ValueSpec::pair(ValueType::Int, ValueType::Int), "省份是否有某建筑", FieldCategory::Trigger),
	FieldSpec::new("province_core_of", ValueSpec::pair(ValueType::Int, ValueType::Int), "省份是否为某文明核心", FieldCategory::Trigger),
	FieldSpec::new("province_is_coastal", ValueSpec::single(ValueType::Bool), "省份是否沿海", FieldCategory::Trigger),
	FieldSpec::new("province_is_in_capital_continent", ValueSpec::single(ValueType::Bool), "省份是否在首都大洲", FieldCategory::Trigger),
	FieldSpec::new("province_manpower_over", ValueSpec::single(ValueType::Int), "省份人力超过", FieldCategory::Trigger),
	FieldSpec::new("province_population_over", ValueSpec::single(ValueType::Int), "省份人口超过", FieldCategory::Trigger),
	FieldSpec::new("province_devastation_over", ValueSpec::single(ValueType::Float), "省份破坏度超过", FieldCategory::Trigger),
	FieldSpec::new("province_unrest_over", ValueSpec::single(ValueType::Float), "省份动荡度超过", FieldCategory::Trigger),
	FieldSpec::new("num_of_controlled_provinces_over", ValueSpec::single(ValueType::Int), "控制省份数超过", FieldCategory::Trigger),
	FieldSpec::new("num_of_controlled_provinces_below", ValueSpec::single(ValueType::Int), "控制省份数低于", FieldCategory::Trigger),
	FieldSpec::new("num_of_armies_over", ValueSpec::single(ValueType::Int), "军队数量超过", FieldCategory::Trigger),
	FieldSpec::new("num_of_armies_below", ValueSpec::single(ValueType::Int), "军队数量低于", FieldCategory::Trigger),
	FieldSpec::new("num_of_generals_over", ValueSpec::single(ValueType::Int), "将领数量超过", FieldCategory::Trigger),
	FieldSpec::new("num_of_generals_below", ValueSpec::single(ValueType::Int), "将领数量低于", FieldCategory::Trigger),
	FieldSpec::new("num_of_advisors_over", ValueSpec::single(ValueType::Int), "顾问数量超过", FieldCategory::Trigger),
	FieldSpec::new("num_of_advisors_below", ValueSpec::single(ValueType::Int), "顾问数量低于", FieldCategory::Trigger),
	FieldSpec::new("has_country_flag", ValueSpec::single(ValueType::Str), "拥有国家标志", FieldCategory::Trigger),
	FieldSpec::new("has_country_flag_not", ValueSpec::single(ValueType::Str), "没有国家标志", FieldCategory::Trigger),
	FieldSpec::new("has_global_flag", ValueSpec::single(ValueType::Str), "拥有全局标志", FieldCategory::Trigger),
	FieldSpec::new("has_global_flag_not", ValueSpec::single(ValueType::Str), "没有全局标志", FieldCategory::Trigger),
];

// ===== 收益效果（sqlite.rs MISSIONS_OPTIONS_BUTTON + 说明文档） =====

pub const EFFECT_FIELDS: &[FieldSpec] = &[
	FieldSpec::new("legacy", ValueSpec::single(ValueType::Int), "威望/遗产", FieldCategory::Effect),
	FieldSpec::new("gold", ValueSpec::single(ValueType::Int), "金币", FieldCategory::Effect),
	FieldSpec::new("manpower", ValueSpec::single(ValueType::Int), "人力", FieldCategory::Effect),
	FieldSpec::new("bonus_duration", ValueSpec::single(ValueType::Int), "后续加成的持续时间（99=永久）", FieldCategory::Effect),
	FieldSpec::new("bonus_monthly_legacy", ValueSpec::single(ValueType::Float), "每月威望加成（如 0.6）", FieldCategory::Effect),
	FieldSpec::new("bonus_monthly_income", ValueSpec::single(ValueType::Int), "每月收入加成", FieldCategory::Effect),
	FieldSpec::new("bonus_units_attack", ValueSpec::single(ValueType::Int), "部队攻击加成", FieldCategory::Effect),
	FieldSpec::new("bonus_units_defense", ValueSpec::single(ValueType::Int), "部队防御加成", FieldCategory::Effect),
	FieldSpec::new("bonus_army_movement_speed", ValueSpec::single(ValueType::Float), "军队移动速度加成（如 0.1）", FieldCategory::Effect),
	FieldSpec::new("bonus_army_morale_recovery", ValueSpec::single(ValueType::Int), "军队士气恢复速度加成", FieldCategory::Effect),
	FieldSpec::new("bonus_recruitment_time", ValueSpec::single(ValueType::Int), "征召时间加成", FieldCategory::Effect),
	FieldSpec::new("bonus_manpower_recovery_speed", ValueSpec::single(ValueType::Int), "人力恢复速度加成", FieldCategory::Effect),
	FieldSpec::new("bonus_max_manpower", ValueSpec::single(ValueType::Int), "最大人力加成", FieldCategory::Effect),
	FieldSpec::new("bonus_max_manpower_percentage", ValueSpec::single(ValueType::Int), "最大人力百分比加成", FieldCategory::Effect),
	FieldSpec::new("bonus_research", ValueSpec::single(ValueType::Int), "科研槽加成", FieldCategory::Effect),
	FieldSpec::new("bonus_research_points", ValueSpec::single(ValueType::Int), "科研点数加成", FieldCategory::Effect),
	FieldSpec::new("bonus_generals_attack", ValueSpec::single(ValueType::Int), "将领攻击加成", FieldCategory::Effect),
	FieldSpec::new("bonus_generals_defense", ValueSpec::single(ValueType::Int), "将领防御加成", FieldCategory::Effect),
	FieldSpec::new("bonus_income_production", ValueSpec::single(ValueType::Int), "收入产出加成", FieldCategory::Effect),
	FieldSpec::new("bonus_production_efficiency", ValueSpec::single(ValueType::Int), "生产效率加成", FieldCategory::Effect),
	FieldSpec::new("bonus_tax_efficiency", ValueSpec::single(ValueType::Int), "税收效率加成", FieldCategory::Effect),
	FieldSpec::new("bonus_construction_cost", ValueSpec::single(ValueType::Int), "建筑成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_construction_time", ValueSpec::single(ValueType::Int), "建筑时间加成（负数=减少）", FieldCategory::Effect),
	FieldSpec::new("bonus_discipline", ValueSpec::single(ValueType::Int), "纪律加成", FieldCategory::Effect),
	FieldSpec::new("bonus_loans_limit", ValueSpec::single(ValueType::Int), "贷款上限加成", FieldCategory::Effect),
	FieldSpec::new("bonus_administration_buildings_cost", ValueSpec::single(ValueType::Int), "行政建筑成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_military_buildings_cost", ValueSpec::single(ValueType::Int), "军事建筑成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_economy_buildings_cost", ValueSpec::single(ValueType::Int), "经济建筑成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_army_maintenance", ValueSpec::single(ValueType::Int), "军队维护成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_province_maintenance", ValueSpec::single(ValueType::Int), "省份维护成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_buildings_maintenance_cost", ValueSpec::single(ValueType::Int), "建筑维护成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_recruit_army_cost", ValueSpec::single(ValueType::Int), "征召军队成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_recruit_army_first_line_cost", ValueSpec::single(ValueType::Int), "征召一线部队成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_recruit_army_second_line_cost", ValueSpec::single(ValueType::Int), "征召二线部队成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_reinforcement_speed", ValueSpec::single(ValueType::Int), "增援速度加成", FieldCategory::Effect),
	FieldSpec::new("province_unrest_all", ValueSpec::single(ValueType::Int), "全境动荡变化（-99=消除动荡）", FieldCategory::Effect),
	FieldSpec::new("province_economy_capital_all", ValueSpec::single(ValueType::Int), "首都所有经济", FieldCategory::Effect),
	FieldSpec::new("province_economy_capital_bul", ValueSpec::single(ValueType::Int), "首都经济（保加利亚特化）", FieldCategory::Effect),
	FieldSpec::new("province_economy_all", ValueSpec::single(ValueType::Int), "所有省份经济", FieldCategory::Effect),
	FieldSpec::new("province_economy", ValueSpec::single(ValueType::Int), "省份经济", FieldCategory::Effect),
	FieldSpec::new("province_economy_id", ValueSpec::pair(ValueType::Int, ValueType::Int), "指定省份经济", FieldCategory::Effect),
	FieldSpec::new("province_infrastructure_all", ValueSpec::single(ValueType::Int), "所有省份基础设施", FieldCategory::Effect),
	FieldSpec::new("province_infrastructure", ValueSpec::single(ValueType::Int), "省份基础设施", FieldCategory::Effect),
	FieldSpec::new("province_growth_rate_all", ValueSpec::single(ValueType::Int), "所有省份增长率", FieldCategory::Effect),
	FieldSpec::new("province_growth_rate", ValueSpec::single(ValueType::Int), "省份增长率", FieldCategory::Effect),
	FieldSpec::new("province_population_all", ValueSpec::single(ValueType::Int), "所有省份人口", FieldCategory::Effect),
	FieldSpec::new("province_religion_all", ValueSpec::single(ValueType::Int), "所有省份宗教", FieldCategory::Effect),
	FieldSpec::new("province_manpower_id", ValueSpec::pair(ValueType::Int, ValueType::Int), "指定省份人力", FieldCategory::Effect),
	FieldSpec::new("province_add_building", ValueSpec::lambda(ValueType::Int, ValueType::Int, ValueType::Int), "添加建筑（如 3994;=6=0）", FieldCategory::Effect),
	FieldSpec::new("province_add_core_civ", ValueSpec::list(ValueType::Int), "添加核心省份（分号分隔）", FieldCategory::Effect),
	FieldSpec::new("change_ideology", ValueSpec::single(ValueType::Int), "改变意识形态倾向", FieldCategory::Effect),
	FieldSpec::new("change_ideology_civ", ValueSpec::pair(ValueType::Str, ValueType::Int), "改变某文明的意识形态倾向", FieldCategory::Effect),
	FieldSpec::new("set_civ_tag", ValueSpec::single(ValueType::Str), "设置文明标签/变身", FieldCategory::Effect),
	FieldSpec::new("set_civ_tag2", ValueSpec::pair(ValueType::Str, ValueType::Str), "设置文明标签2（如 ger=ger_c）", FieldCategory::Effect),
	FieldSpec::new("set_civ_tag_reset", ValueSpec::single(ValueType::Str), "重置文明标签", FieldCategory::Effect),
	FieldSpec::new("player_set_civ", ValueSpec::single(ValueType::Str), "将玩家设置为某文明", FieldCategory::Effect),
	FieldSpec::new("annex_civ", ValueSpec::single(ValueType::Str), "吞并某文明（可多次使用吞并多个）", FieldCategory::Effect),
	FieldSpec::new("annexed_by_civ", ValueSpec::single(ValueType::Str), "被某文明吞并", FieldCategory::Effect),
	FieldSpec::new("annex_by_civ_from_civ", ValueSpec::triple(ValueType::Str, ValueType::Str, ValueType::Str), "从某文明吞并给另一文明", FieldCategory::Effect),
	FieldSpec::new("make_puppet", ValueSpec::pair(ValueType::Str, ValueType::Str), "成为某文明傀儡", FieldCategory::Effect),
	FieldSpec::new("annex", ValueSpec::single(ValueType::Int), "吞并", FieldCategory::Effect),
	FieldSpec::new("add_ruler", ValueSpec::single(ValueType::Str), "添加统治者（如 \"鲍里斯=三世=鲍里斯三世=30=1=1894\"）", FieldCategory::Effect),
	FieldSpec::new("add_general2", ValueSpec::single(ValueType::Str), "添加将领", FieldCategory::Effect),
	FieldSpec::new("add_general", ValueSpec::single(ValueType::Str), "添加将领（旧版）", FieldCategory::Effect),
	FieldSpec::new("add_ruler_custom", ValueSpec::single(ValueType::Str), "自定义统治者", FieldCategory::Effect),
	FieldSpec::new("add_advisor", ValueSpec::single(ValueType::Int), "添加顾问（0-3 不同类型）", FieldCategory::Effect),
	FieldSpec::new("run_event", ValueSpec::single(ValueType::Str), "运行某事件", FieldCategory::Effect),
	FieldSpec::new("run_event_instantly", ValueSpec::single(ValueType::Str), "立即运行某事件", FieldCategory::Effect),
	FieldSpec::new("white_peace", ValueSpec::single(ValueType::Str), "与某文明白和平", FieldCategory::Effect),
	FieldSpec::new("declare_war2", ValueSpec::pair(ValueType::Str, ValueType::Str), "向某文明宣战（宣战国=被宣战国）", FieldCategory::Effect),
	FieldSpec::new("declare_war", ValueSpec::single(ValueType::Str), "向某文明宣战（旧版）", FieldCategory::Effect),
	FieldSpec::new("move_capital", ValueSpec::single(ValueType::Int), "迁都到某省份", FieldCategory::Effect),
	FieldSpec::new("add_ns", ValueSpec::single(ValueType::Str), "添加国家精神", FieldCategory::Effect),
	FieldSpec::new("remove_ns", ValueSpec::single(ValueType::Str), "移除国家精神", FieldCategory::Effect),
	FieldSpec::new("add_new_army", ValueSpec::sequence(ValueType::Int, ValueType::Int), "添加新军队（类型=数量=类型=数量...）", FieldCategory::Effect),
	FieldSpec::new("add_defensive_pact", ValueSpec::pair(ValueType::Str, ValueType::Str), "添加防御条约（如 fra=uni）", FieldCategory::Effect),
	FieldSpec::new("add_guarantee", ValueSpec::pair(ValueType::Str, ValueType::Str), "添加保证（如 fra=bel）", FieldCategory::Effect),
	FieldSpec::new("add_truce", ValueSpec::pair(ValueType::Str, ValueType::Str), "添加休战协议（如 atr=ser）", FieldCategory::Effect),
	FieldSpec::new("add_decision", ValueSpec::single(ValueType::Str), "添加决议", FieldCategory::Effect),
	FieldSpec::new("start_decision", ValueSpec::single(ValueType::Str), "启动决议", FieldCategory::Effect),
	FieldSpec::new("taking_decision", ValueSpec::single(ValueType::Str), "执行决议", FieldCategory::Effect),
	FieldSpec::new("join_alliance_special_id_first_tier", ValueSpec::single(ValueType::Int), "加入联盟第一层级", FieldCategory::Effect),
	FieldSpec::new("join_alliance_special_id_second_tier", ValueSpec::single(ValueType::Int), "加入联盟第二层级（0=同盟国）", FieldCategory::Effect),
	FieldSpec::new("leave_alliance_special_id", ValueSpec::single(ValueType::Int), "离开联盟（1=协约国）", FieldCategory::Effect),
	FieldSpec::new("unlock_tech", ValueSpec::single(ValueType::Int), "解锁科技", FieldCategory::Effect),
	FieldSpec::new("military_academy", ValueSpec::single(ValueType::Int), "军事学院", FieldCategory::Effect),
	FieldSpec::new("set_counter", ValueSpec::pair(ValueType::Str, ValueType::Str), "设置计数器 $变量名=$表达式", FieldCategory::Effect),
	FieldSpec::new("ae_set", ValueSpec::single(ValueType::Int), "设置侵略扩张值（如 -50、10、100）", FieldCategory::Effect),
	FieldSpec::new("ae_ste", ValueSpec::single(ValueType::Int), "设置侵略扩张值（步进）", FieldCategory::Effect),
	FieldSpec::new("se_set", ValueSpec::single(ValueType::Int), "设置超级事件值", FieldCategory::Effect),
	FieldSpec::new("supreme_court", ValueSpec::single(ValueType::Int), "最高法院", FieldCategory::Effect),
	FieldSpec::new("tooltip", ValueSpec::pair(ValueType::Bool, ValueType::Str), "提示信息（如 true=这将获得...）", FieldCategory::Effect),
	FieldSpec::new("ai", ValueSpec::single(ValueType::Int), "AI 选择概率", FieldCategory::Effect),
	FieldSpec::new("alliance", ValueSpec::single(ValueType::Str), "加入联盟", FieldCategory::Effect),
	FieldSpec::new("non_aggression_pact", ValueSpec::single(ValueType::Str), "互不侵犯条约", FieldCategory::Effect),
	FieldSpec::new("military_access", ValueSpec::single(ValueType::Str), "军事通行权", FieldCategory::Effect),
	FieldSpec::new("vassalize", ValueSpec::single(ValueType::Str), "使某文明成为附庸", FieldCategory::Effect),
	FieldSpec::new("relation_change", ValueSpec::pair(ValueType::Str, ValueType::Int), "关系变化", FieldCategory::Effect),
	FieldSpec::new("relation_set", ValueSpec::pair(ValueType::Str, ValueType::Int), "设置关系值", FieldCategory::Effect),
	FieldSpec::new("remove_alliance", ValueSpec::single(ValueType::Int), "移除联盟", FieldCategory::Effect),
	FieldSpec::new("change_law", ValueSpec::single(ValueType::Int), "改变法律", FieldCategory::Effect),
	FieldSpec::new("change_law2", ValueSpec::single(ValueType::Int), "改变法律2", FieldCategory::Effect),
	FieldSpec::new("change_religion", ValueSpec::single(ValueType::Int), "改变宗教", FieldCategory::Effect),
	FieldSpec::new("change_religion_civ", ValueSpec::pair(ValueType::Str, ValueType::Int), "改变某文明宗教", FieldCategory::Effect),
	FieldSpec::new("switch_ability", ValueSpec::single(ValueType::Str), "切换能力", FieldCategory::Effect),
	FieldSpec::new("add_variable", ValueSpec::single(ValueType::Str), "添加变量", FieldCategory::Effect),
	FieldSpec::new("add_variable_civ", ValueSpec::pair(ValueType::Str, ValueType::Str), "为某文明添加变量", FieldCategory::Effect),
	FieldSpec::new("remove_variable", ValueSpec::single(ValueType::Str), "移除变量", FieldCategory::Effect),
	FieldSpec::new("remove_variable_civ", ValueSpec::pair(ValueType::Str, ValueType::Str), "移除某文明变量", FieldCategory::Effect),
	FieldSpec::new("add_counter", ValueSpec::pair(ValueType::Str, ValueType::Int), "计数器加法", FieldCategory::Effect),
	FieldSpec::new("sub_counter", ValueSpec::pair(ValueType::Str, ValueType::Int), "计数器减法", FieldCategory::Effect),
	FieldSpec::new("mul_counter", ValueSpec::pair(ValueType::Str, ValueType::Int), "计数器乘法", FieldCategory::Effect),
	FieldSpec::new("div_counter", ValueSpec::pair(ValueType::Str, ValueType::Int), "计数器除法", FieldCategory::Effect),
	FieldSpec::new("add_advisor_character", ValueSpec::single(ValueType::Str), "添加顾问角色", FieldCategory::Effect),
	FieldSpec::new("add_general_character", ValueSpec::single(ValueType::Str), "添加将领角色", FieldCategory::Effect),
	FieldSpec::new("add_general_character_attack_defense", ValueSpec::triple(ValueType::Str, ValueType::Int, ValueType::Int), "添加将领并设置攻防", FieldCategory::Effect),
	FieldSpec::new("promote_advisor", ValueSpec::single(ValueType::Int), "晋升顾问", FieldCategory::Effect),
	FieldSpec::new("kill_advisor", ValueSpec::single(ValueType::Int), "杀死顾问", FieldCategory::Effect),
	FieldSpec::new("kill_ruler", ValueSpec::single(ValueType::Int), "杀死统治者", FieldCategory::Effect),
	FieldSpec::new("kill_ruler_chance", ValueSpec::single(ValueType::Float), "概率杀死统治者（0-1）", FieldCategory::Effect),
	FieldSpec::new("remove_decision", ValueSpec::single(ValueType::Str), "移除决议", FieldCategory::Effect),
	FieldSpec::new("start_decision2", ValueSpec::single(ValueType::Str), "启动决议2", FieldCategory::Effect),
	FieldSpec::new("decision_desc", ValueSpec::single(ValueType::Str), "决议描述", FieldCategory::Effect),
	FieldSpec::new("decision_image", ValueSpec::single(ValueType::Str), "决议图片", FieldCategory::Effect),
	FieldSpec::new("province_devastation", ValueSpec::single(ValueType::Int), "省份破坏", FieldCategory::Effect),
	FieldSpec::new("province_devastation_all", ValueSpec::single(ValueType::Int), "全境破坏", FieldCategory::Effect),
	FieldSpec::new("province_devastation_capital", ValueSpec::single(ValueType::Int), "首都破坏", FieldCategory::Effect),
	FieldSpec::new("province_devastation_id", ValueSpec::pair(ValueType::Int, ValueType::Int), "指定省份破坏", FieldCategory::Effect),
	FieldSpec::new("province_manpower", ValueSpec::single(ValueType::Int), "省份人力", FieldCategory::Effect),
	FieldSpec::new("province_manpower_all", ValueSpec::single(ValueType::Int), "全境人力", FieldCategory::Effect),
	FieldSpec::new("province_manpower_capital", ValueSpec::single(ValueType::Int), "首都人力", FieldCategory::Effect),
	FieldSpec::new("province_population", ValueSpec::single(ValueType::Int), "省份人口", FieldCategory::Effect),
	FieldSpec::new("province_population_capital", ValueSpec::single(ValueType::Int), "首都人口", FieldCategory::Effect),
	FieldSpec::new("province_tax_efficiency", ValueSpec::single(ValueType::Int), "省份税收效率", FieldCategory::Effect),
	FieldSpec::new("province_tax_efficiency_all", ValueSpec::single(ValueType::Int), "全境税收效率", FieldCategory::Effect),
	FieldSpec::new("province_id_build_add", ValueSpec::pair(ValueType::Int, ValueType::Int), "指定省份添加建筑", FieldCategory::Effect),
	FieldSpec::new("province_id_build_remove", ValueSpec::pair(ValueType::Int, ValueType::Int), "指定省份移除建筑", FieldCategory::Effect),
	FieldSpec::new("province_id_core_add", ValueSpec::pair(ValueType::Int, ValueType::Int), "指定省份添加核心", FieldCategory::Effect),
	FieldSpec::new("province_id_core_remove", ValueSpec::pair(ValueType::Int, ValueType::Int), "指定省份移除核心", FieldCategory::Effect),
	FieldSpec::new("province_id_nuke", ValueSpec::single(ValueType::Int), "核打击省份", FieldCategory::Effect),
	FieldSpec::new("province_id_pop_set", ValueSpec::pair(ValueType::Int, ValueType::Int), "设置省份人口", FieldCategory::Effect),
	FieldSpec::new("province_id_spread_disease", ValueSpec::pair(ValueType::Int, ValueType::Int), "省份传播疾病", FieldCategory::Effect),
	FieldSpec::new("resource_price_change", ValueSpec::pair(ValueType::Int, ValueType::Int), "资源价格变化", FieldCategory::Effect),
	FieldSpec::new("resource_price_change_up", ValueSpec::pair(ValueType::Int, ValueType::Int), "资源价格上涨", FieldCategory::Effect),
	FieldSpec::new("resource_price_change_down", ValueSpec::pair(ValueType::Int, ValueType::Int), "资源价格下跌", FieldCategory::Effect),
	FieldSpec::new("resource_price_change_random", ValueSpec::pair(ValueType::Int, ValueType::Int), "资源价格随机变化", FieldCategory::Effect),
	FieldSpec::new("resource_price_change_group", ValueSpec::pair(ValueType::Int, ValueType::Int), "资源组价格变化", FieldCategory::Effect),
	FieldSpec::new("ai_aggression", ValueSpec::single(ValueType::Float), "AI 侵略性", FieldCategory::Effect),
	FieldSpec::new("advantage_points", ValueSpec::single(ValueType::Int), "优势点数", FieldCategory::Effect),
	FieldSpec::new("annex_provinces", ValueSpec::single(ValueType::Int), "吞并省份", FieldCategory::Effect),
	FieldSpec::new("annex_provinces_from_civ", ValueSpec::pair(ValueType::Str, ValueType::Int), "从某文明吞并省份", FieldCategory::Effect),
	FieldSpec::new("capital_city_level", ValueSpec::single(ValueType::Int), "首都城市等级", FieldCategory::Effect),
	FieldSpec::new("explode", ValueSpec::single(ValueType::Int), "爆炸效果", FieldCategory::Effect),
	FieldSpec::new("inflation", ValueSpec::single(ValueType::Int), "通货膨胀", FieldCategory::Effect),
	FieldSpec::new("nuclear_reactor", ValueSpec::single(ValueType::Int), "核反应堆", FieldCategory::Effect),
	FieldSpec::new("play_music", ValueSpec::single(ValueType::Str), "播放音乐", FieldCategory::Effect),
	FieldSpec::new("run_script", ValueSpec::single(ValueType::Str), "运行脚本", FieldCategory::Effect),
	FieldSpec::new("skip_focus", ValueSpec::single(ValueType::Int), "跳过焦点", FieldCategory::Effect),
	FieldSpec::new("skip_goal", ValueSpec::single(ValueType::Int), "跳过目标", FieldCategory::Effect),
	FieldSpec::new("military_academy_for_generals", ValueSpec::single(ValueType::Int), "将领军事学院", FieldCategory::Effect),
	FieldSpec::new("white_peace2", ValueSpec::single(ValueType::Str), "白和平2", FieldCategory::Effect),
	FieldSpec::new("bonus_advisor_cost", ValueSpec::single(ValueType::Int), "顾问成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_advisor_max_level", ValueSpec::single(ValueType::Int), "顾问最大等级加成", FieldCategory::Effect),
	FieldSpec::new("bonus_aggressive_expansion", ValueSpec::single(ValueType::Int), "侵略扩张加成", FieldCategory::Effect),
	FieldSpec::new("bonus_all_characters_life_expectancy", ValueSpec::single(ValueType::Int), "所有角色寿命加成", FieldCategory::Effect),
	FieldSpec::new("bonus_battle_width", ValueSpec::single(ValueType::Int), "战斗宽度加成", FieldCategory::Effect),
	FieldSpec::new("bonus_core_cost", ValueSpec::single(ValueType::Int), "核心成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_corruption", ValueSpec::single(ValueType::Int), "腐败加成", FieldCategory::Effect),
	FieldSpec::new("bonus_develop_infrastructure_cost", ValueSpec::single(ValueType::Int), "发展基建成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_diplomacy_points", ValueSpec::single(ValueType::Int), "外交点数加成", FieldCategory::Effect),
	FieldSpec::new("bonus_disease_death_rate", ValueSpec::single(ValueType::Int), "疾病死亡率加成", FieldCategory::Effect),
	FieldSpec::new("bonus_growth_rate", ValueSpec::single(ValueType::Int), "增长率加成", FieldCategory::Effect),
	FieldSpec::new("bonus_improve_relations_modifier", ValueSpec::single(ValueType::Int), "改善关系修正加成", FieldCategory::Effect),
	FieldSpec::new("bonus_income_economy", ValueSpec::single(ValueType::Int), "经济收入加成", FieldCategory::Effect),
	FieldSpec::new("bonus_income_from_vassals", ValueSpec::single(ValueType::Int), "附庸收入加成", FieldCategory::Effect),
	FieldSpec::new("bonus_income_taxation", ValueSpec::single(ValueType::Int), "税收收入加成", FieldCategory::Effect),
	FieldSpec::new("bonus_increase_growth_rate_cost", ValueSpec::single(ValueType::Int), "增加增长率成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_increase_manpower_cost", ValueSpec::single(ValueType::Int), "增加人力成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_increase_tax_efficiency_cost", ValueSpec::single(ValueType::Int), "增加税收效率成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_inflation", ValueSpec::single(ValueType::Int), "通货膨胀加成", FieldCategory::Effect),
	FieldSpec::new("bonus_invest_in_economy_cost", ValueSpec::single(ValueType::Int), "投资经济成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_loan_interest", ValueSpec::single(ValueType::Int), "贷款利息加成", FieldCategory::Effect),
	FieldSpec::new("bonus_maintenance_cost", ValueSpec::single(ValueType::Int), "维护成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_manpower_recovery_from_a_disbanded_army", ValueSpec::single(ValueType::Int), "解散军队人力恢复加成", FieldCategory::Effect),
	FieldSpec::new("bonus_max_morale", ValueSpec::single(ValueType::Int), "最大士气加成", FieldCategory::Effect),
	FieldSpec::new("bonus_maximum_amount_of_gold", ValueSpec::single(ValueType::Int), "最大金币上限加成", FieldCategory::Effect),
	FieldSpec::new("bonus_monthly_legacy_perc", ValueSpec::single(ValueType::Float), "每月威望百分比加成", FieldCategory::Effect),
	FieldSpec::new("bonus_regiments_limit", ValueSpec::single(ValueType::Int), "军团上限加成", FieldCategory::Effect),
	FieldSpec::new("bonus_religion_cost", ValueSpec::single(ValueType::Int), "宗教成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_revolutionary_risk", ValueSpec::single(ValueType::Int), "革命风险加成", FieldCategory::Effect),
	FieldSpec::new("bonus_siege_effectiveness", ValueSpec::single(ValueType::Int), "围困效率加成", FieldCategory::Effect),
	FieldSpec::new("bonus_war_score_cost", ValueSpec::single(ValueType::Int), "战争分数成本加成", FieldCategory::Effect),
];

/// 常见的拼写错误键 → 正确键名提示（编辑器诊断用）。
pub const KNOWN_TYPOS: &[(&str, &str)] = &[
	("leagcy", "legacy"),
];

/// 遍历全部已注册的键位。
pub fn all_specs() -> impl Iterator<Item = &'static FieldSpec> {
	REQUIRED_FIELDS
		.iter()
		.chain(HEADER_FIELDS)
		.chain(OPTIONAL_FIELDS)
		.chain(TRIGGER_FIELDS)
		.chain(EFFECT_FIELDS)
}

/// 按键名查注册信息。
pub fn lookup(key: &str) -> Option<&'static FieldSpec> {
	all_specs().find(|spec| spec.key == key)
}

/// 类型语法诊断：判断 `value` 是否符合 `spec` 描述的语法。
/// 空值视为合法（允许留空）。
pub fn is_valid_value(spec: ValueSpec, value: &str) -> bool {
	let value = value.trim();
	if value.is_empty() {
		return true;
	}
	match spec {
		ValueSpec::Single(value_type) => value_type.accepts(value),
		ValueSpec::OneOf(first, second) => first.accepts(value) || second.accepts(value),
		ValueSpec::Pair(first, second) => {
			let parts: Vec<_> = value.split('=').collect();
			parts.len() == 2 && first.accepts(parts[0]) && second.accepts(parts[1])
		}
		ValueSpec::Triple(first, second, third) => {
			let parts: Vec<_> = value.split('=').collect();
			parts.len() == 3
				&& first.accepts(parts[0])
				&& second.accepts(parts[1])
				&& third.accepts(parts[2])
		}
		ValueSpec::List(value_type) => value
			.split(';')
			.filter(|part| !part.trim().is_empty())
			.all(|part| value_type.accepts(part)),
		ValueSpec::Sequence(first, second) => {
			let parts: Vec<_> = value.split('=').collect();
			if parts.len() < 2 || parts.len() % 2 != 0 {
				return false;
			}
			parts.iter().enumerate().all(|(index, part)| {
				if index % 2 == 0 {
					first.accepts(part)
				} else {
					second.accepts(part)
				}
			})
		}
		ValueSpec::Lambda(first, second, third) => {
			let Some((left, right)) = value.split_once('=') else {
				return false;
			};
			let list_ok = left
				.split(';')
				.filter(|part| !part.trim().is_empty())
				.all(|part| first.accepts(part));
			let pair_parts: Vec<_> = right.split('=').collect();
			list_ok
				&& pair_parts.len() == 2
				&& second.accepts(pair_parts[0])
				&& third.accepts(pair_parts[1])
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn lookup_finds_common_keys() {
		assert_eq!(lookup("id").unwrap().category, FieldCategory::Required);
		assert_eq!(lookup("focus_dura").unwrap().category, FieldCategory::Optional);
		assert_eq!(
			lookup("civ_capital_unrest_over").unwrap().category,
			FieldCategory::Trigger
		);
		assert_eq!(lookup("legacy").unwrap().category, FieldCategory::Effect);
		assert!(lookup("leagcy").is_none());
	}

	#[test]
	fn validates_basic_types() {
		assert!(is_valid_value(ValueSpec::single(ValueType::Int), "-99"));
		assert!(!is_valid_value(ValueSpec::single(ValueType::Int), "abc"));
		assert!(is_valid_value(ValueSpec::single(ValueType::Bool), "false"));
		assert!(!is_valid_value(ValueSpec::single(ValueType::Bool), "yes"));
		assert!(is_valid_value(ValueSpec::single(ValueType::Float), "0.6"));
		assert!(!is_valid_value(ValueSpec::single(ValueType::Float), "NaN"));
	}

	#[test]
	fn validates_composite_types() {
		assert!(is_valid_value(
			ValueSpec::pair(ValueType::Str, ValueType::Str),
			"fra=ger"
		));
		assert!(!is_valid_value(
			ValueSpec::pair(ValueType::Str, ValueType::Int),
			"fra=ger"
		));
		assert!(is_valid_value(
			ValueSpec::triple(ValueType::Int, ValueType::Int, ValueType::Int),
			"15=8=1917"
		));
		assert!(is_valid_value(ValueSpec::list(ValueType::Int), "1;2;3"));
		assert!(!is_valid_value(ValueSpec::list(ValueType::Int), "1;a;3"));
		assert!(is_valid_value(
			ValueSpec::sequence(ValueType::Int, ValueType::Int),
			"1=100=2=200"
		));
		assert!(!is_valid_value(
			ValueSpec::sequence(ValueType::Int, ValueType::Int),
			"1=100=2"
		));
		assert!(is_valid_value(
			ValueSpec::lambda(ValueType::Int, ValueType::Int, ValueType::Int),
			"3994;=6=0"
		));
	}

	#[test]
	fn empty_values_are_valid() {
		assert!(is_valid_value(ValueSpec::single(ValueType::Int), ""));
		assert!(is_valid_value(ValueSpec::pair(ValueType::Str, ValueType::Str), ""));
	}
}
