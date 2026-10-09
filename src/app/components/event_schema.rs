//! 国策事件脚本（missionsEvents/*.txt）键位注册表。
//!
//! 键名、值类型与中文注释移植自：
//! - `a:\android\missions_db\ScvGen\src\sqlite.rs`
//! - 项目内《国策系统说明文档》（docs/国策系统说明文档.md）
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
	/// `a=b=a=b...` 成对序列，如 `add_new_army` 的 `兵种=型号=…`（对长可变）。
	Sequence(ValueType, ValueType),
	/// 省份列表 + 值段：`[省份;…;]=a[=b]`，如 `province_add_building` 的
	/// `6217;=6=0`、`province_add_core_civ` 的 `4381;4877;=ANF6_Anfu`。
	/// 解析规则（GameCivs 五模组 400+ 处实测）：在**首个** `=` 处切分——
	/// 左侧是省份 ID 列表（`;` 分隔；尾随 `;` 只是空尾项、可省略；
	/// 单省时惯用不写 `;`，即 `省份=值`），右侧是 1~2 段值
	/// （单值如数值/文明标签，或两段如 `建筑=等级`）。
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
	FieldSpec::new("pobssible_to_run", ValueSpec::single(ValueType::Bool), "是否可执行（历史拼写；引擎实测只读 possible_to_run，此键可能在当前引擎被忽略，但保留支持）", FieldCategory::Optional),
	FieldSpec::new("possible_to_run", ValueSpec::single(ValueType::Bool), "是否可执行（0.25.1 解析器实测读取的拼写，新写内容请用此项）", FieldCategory::Optional),
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
	// 实测模组常见键（GameCivs 五模组统计后注册）
	FieldSpec::new("ui_type", ValueSpec::single(ValueType::Int), "界面类型", FieldCategory::Optional),
	FieldSpec::new("layout", ValueSpec::single(ValueType::Str), "布局（可留空）", FieldCategory::Optional),
	FieldSpec::new("music_file", ValueSpec::single(ValueType::Str), "音乐文件（musicName 的模组写法）", FieldCategory::Optional),
	FieldSpec::new("events_desc", ValueSpec::single(ValueType::Str), "事件详细描述（可留空）", FieldCategory::Optional),
	FieldSpec::new("goal_dura", ValueSpec::single(ValueType::Int), "目标完成时长/天", FieldCategory::Optional),
	FieldSpec::new("decision_dura", ValueSpec::single(ValueType::Int), "决策时长/天", FieldCategory::Optional),
	FieldSpec::new("historic_choice", ValueSpec::single(ValueType::Bool), "历史性选择标记", FieldCategory::Optional),
	FieldSpec::new("popup", ValueSpec::single(ValueType::Bool), "是否弹窗（小写写法，同 popUp）", FieldCategory::Optional),
	FieldSpec::new("textBackground", ValueSpec::single(ValueType::Bool), "是否显示文本背景", FieldCategory::Optional),
	FieldSpec::new("Important", ValueSpec::single(ValueType::Bool), "是否重要（大写写法，同 important）", FieldCategory::Optional),
	FieldSpec::new("name", ValueSpec::single(ValueType::Str), "名称（选项名；选项块内由解析器单独识别）", FieldCategory::Optional),
];

// ===== 触发条件（sqlite.rs MISSIONS_TRIGGER_BUTTON + 说明文档） =====

pub const TRIGGER_FIELDS: &[FieldSpec] = &[
	FieldSpec::new("is_civ", ValueSpec::single(ValueType::Str), "指定文明（如 atr、spa）", FieldCategory::Trigger),
	FieldSpec::new("is_player", ValueSpec::one_of(ValueType::Bool, ValueType::Str), "是否为玩家（可指定文明）", FieldCategory::Trigger),
	FieldSpec::new("is_not_player", ValueSpec::one_of(ValueType::Bool, ValueType::Str), "是否非玩家（可指定文明）", FieldCategory::Trigger),
	FieldSpec::new("is_puppet", ValueSpec::one_of(ValueType::Bool, ValueType::Str), "是否为傀儡国（true/false；模组实测也写文明标签）", FieldCategory::Trigger),
	FieldSpec::new("is_not_puppet", ValueSpec::one_of(ValueType::Bool, ValueType::Str), "是否非傀儡国（true/false；模组实测也写文明标签）", FieldCategory::Trigger),
	FieldSpec::new("civ_is_at_war", ValueSpec::single(ValueType::Bool), "是否处于战争状态", FieldCategory::Trigger),
	FieldSpec::new("civ_is_not_at_war", ValueSpec::single(ValueType::Bool), "是否不处于战争状态", FieldCategory::Trigger),
	FieldSpec::new("civ_is_at_war_days_over", ValueSpec::single(ValueType::Int), "战争持续天数超过此值", FieldCategory::Trigger),
	FieldSpec::new("civ_is_vassal_of_civ", ValueSpec::pair(ValueType::Str, ValueType::Str), "某文明是否为另一文明的附属国（如 bah=atr）", FieldCategory::Trigger),
	FieldSpec::new("exists_any", ValueSpec::single(ValueType::Str), "存在某文明", FieldCategory::Trigger),
	FieldSpec::new("exists_any_not", ValueSpec::single(ValueType::Str), "不存在某文明", FieldCategory::Trigger),
	FieldSpec::new("province_controlled_by", ValueSpec::single(ValueType::Str), "省份被某文明控制", FieldCategory::Trigger),
	FieldSpec::new("province_is_occupied", ValueSpec::one_of(ValueType::Bool, ValueType::Int), "省份是否被占领（true/false；或省份 ID）", FieldCategory::Trigger),
	FieldSpec::new("province_is_under_siege", ValueSpec::one_of(ValueType::Bool, ValueType::Int), "省份是否被围困（true/false；或省份 ID）", FieldCategory::Trigger),
	FieldSpec::new("has_variable", ValueSpec::single(ValueType::Str), "拥有某变量/标签", FieldCategory::Trigger),
	FieldSpec::new("has_variable_not", ValueSpec::single(ValueType::Str), "没有某变量/标签", FieldCategory::Trigger),
	FieldSpec::new("has_variable_civ", ValueSpec::single(ValueType::Str), "某文明拥有某变量（通常 `文明=变量`；也可单写变量名）", FieldCategory::Trigger),
	FieldSpec::new("civ_capital_unrest_over", ValueSpec::single(ValueType::Float), "首都动荡度高于此值（如 0.7、5.2）", FieldCategory::Trigger),
	FieldSpec::new("civ_capital_unrest_below", ValueSpec::single(ValueType::Float), "首都动荡度低于此值（如 0.01）", FieldCategory::Trigger),
	FieldSpec::new("civ_prestige_over", ValueSpec::single(ValueType::Int), "威望高于此值", FieldCategory::Trigger),
	FieldSpec::new("civ_provinces_over", ValueSpec::single(ValueType::Int), "控制省份数高于此值", FieldCategory::Trigger),
	FieldSpec::new("civ_provinces_below", ValueSpec::single(ValueType::Int), "控制省份数低于此值", FieldCategory::Trigger),
	FieldSpec::new("civ_conquered_provinces_over", ValueSpec::single(ValueType::Int), "征服省份数高于此值（如 750、1000）", FieldCategory::Trigger),
	FieldSpec::new("civ_regiments_over", ValueSpec::single(ValueType::Int), "军团数高于此值", FieldCategory::Trigger),
	FieldSpec::new("civ_wars_total_over", ValueSpec::single(ValueType::Int), "战争总数超过此值", FieldCategory::Trigger),
	FieldSpec::new("civ_capital_continent_is", ValueSpec::single(ValueType::Int), "首都所在大洲（2=欧洲；详见 `Continents.json` 数组顺序）", FieldCategory::Trigger),
	FieldSpec::new("civs_are_at_war", ValueSpec::pair(ValueType::Str, ValueType::Str), "两个文明是否处于战争状态（如 fra=ger）", FieldCategory::Trigger),
	FieldSpec::new("exact_day", ValueSpec::triple(ValueType::Int, ValueType::Int, ValueType::Int), "精确日期条件（如 15=8=1917 表示 1917 年 8 月 15 日）", FieldCategory::Trigger),
	FieldSpec::new("playing_time_over", ValueSpec::single(ValueType::Int), "游戏时间超过此值（天）", FieldCategory::Trigger),
	FieldSpec::new("civ_is_in_civil_war", ValueSpec::single(ValueType::Bool), "是否处于内战", FieldCategory::Trigger),
	FieldSpec::new("civ_has_truce_with", ValueSpec::single(ValueType::Str), "是否与某文明停战", FieldCategory::Trigger),
	FieldSpec::new("civ_num_of_cities_over", ValueSpec::single(ValueType::Int), "城市数量超过", FieldCategory::Trigger),
	FieldSpec::new("civ_num_of_cities_below", ValueSpec::single(ValueType::Int), "城市数量低于", FieldCategory::Trigger),
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
	// 实测模组常见键（GameCivs 五模组统计后注册）
	FieldSpec::new("exists", ValueSpec::single(ValueType::Str), "存在某文明（旧版写法，同 exists_any）", FieldCategory::Trigger),
	FieldSpec::new("year_over", ValueSpec::single(ValueType::Int), "年份高于（如 2024）", FieldCategory::Trigger),
	FieldSpec::new("year_below", ValueSpec::single(ValueType::Int), "年份低于", FieldCategory::Trigger),
	FieldSpec::new("random_chance", ValueSpec::single(ValueType::Float), "随机概率（如 20.0，单位 %）", FieldCategory::Trigger),
	FieldSpec::new("civ_government_is", ValueSpec::single(ValueType::Int), "文明政体是否为（政体编号）", FieldCategory::Trigger),
	FieldSpec::new("civ_religion_is", ValueSpec::single(ValueType::Int), "文明宗教是否为（宗教编号）", FieldCategory::Trigger),
	FieldSpec::new("if_counter", ValueSpec::single(ValueType::Str), "计数器条件（`$变量>阈值` / `$变量<阈值` / `$变量=阈值`）", FieldCategory::Trigger),
	FieldSpec::new("is_player2", ValueSpec::single(ValueType::Bool), "是否操控第二位玩家", FieldCategory::Trigger),
	// 作为条件行出现的结构行（块内衔接/结束行出现在意外位置时原样保留，注册后提示更清楚）
	FieldSpec::new("next_and", ValueSpec::single(ValueType::Str), "触发器衔接行：与相邻条件 AND（结构行）", FieldCategory::Trigger),
	FieldSpec::new("next_or", ValueSpec::single(ValueType::Str), "触发器衔接行：与相邻条件 OR（结构行）", FieldCategory::Trigger),
	FieldSpec::new("next_and_not", ValueSpec::single(ValueType::Str), "触发器衔接行：与相邻条件 AND NOT（结构行）", FieldCategory::Trigger),
	FieldSpec::new("next_or_not", ValueSpec::single(ValueType::Str), "触发器衔接行：与相邻条件 OR NOT（结构行）", FieldCategory::Trigger),
	FieldSpec::new("trigger_and", ValueSpec::single(ValueType::Str), "触发器块开始行（出现在块外时原样保留）", FieldCategory::Trigger),
	FieldSpec::new("trigger_or", ValueSpec::single(ValueType::Str), "或触发器块开始行（出现在块外时原样保留）", FieldCategory::Trigger),
	FieldSpec::new("trigger_and_not", ValueSpec::single(ValueType::Str), "非触发器块开始行（出现在块外时原样保留）", FieldCategory::Trigger),
	FieldSpec::new("trigger_and_end", ValueSpec::single(ValueType::Str), "触发器块结束行（出现在块外时原样保留）", FieldCategory::Trigger),
	FieldSpec::new("trigger_or_end", ValueSpec::single(ValueType::Str), "或触发器块结束行（出现在块外时原样保留）", FieldCategory::Trigger),
	FieldSpec::new("trigger_and_not_end", ValueSpec::single(ValueType::Str), "非触发器块结束行（出现在块外时原样保留）", FieldCategory::Trigger),
	// 实测模组常见键（第二批）
	FieldSpec::new("government_type", ValueSpec::single(ValueType::Str), "政体类型是否为（政体名称）", FieldCategory::Trigger),
	FieldSpec::new("alliance_special_is_member_id", ValueSpec::single(ValueType::Str), "联盟特殊成员标识（如 ming）", FieldCategory::Trigger),
	FieldSpec::new("civ_population_over", ValueSpec::single(ValueType::Int), "文明人口高于", FieldCategory::Trigger),
	FieldSpec::new("civ_population_below", ValueSpec::single(ValueType::Int), "文明人口低于", FieldCategory::Trigger),
	FieldSpec::new("civ_battle_width_over", ValueSpec::single(ValueType::Int), "文明战斗宽度高于", FieldCategory::Trigger),
	FieldSpec::new("civ_administrative_advisor_skill_over", ValueSpec::single(ValueType::Int), "行政顾问技能高于", FieldCategory::Trigger),
	FieldSpec::new("civ_economic_advisor_skill_over", ValueSpec::single(ValueType::Int), "经济顾问技能高于", FieldCategory::Trigger),
	FieldSpec::new("civ_innovation_advisor_skill_over", ValueSpec::single(ValueType::Int), "创新顾问技能高于", FieldCategory::Trigger),
	FieldSpec::new("civ_military_advisor_skill_over", ValueSpec::single(ValueType::Int), "军事顾问技能高于", FieldCategory::Trigger),
	FieldSpec::new("civ_regiments_limit_over", ValueSpec::single(ValueType::Int), "军团上限高于", FieldCategory::Trigger),
	FieldSpec::new("civ_unlocked_advantages_over", ValueSpec::single(ValueType::Int), "已解锁优势数高于", FieldCategory::Trigger),
	FieldSpec::new("civ_unlocked_legacies_over", ValueSpec::single(ValueType::Int), "已解锁遗产数高于", FieldCategory::Trigger),
	FieldSpec::new("civ_vassals_over", ValueSpec::single(ValueType::Int), "附庸数量高于", FieldCategory::Trigger),
	FieldSpec::new("civ_allies_over", ValueSpec::single(ValueType::Int), "盟友数量高于", FieldCategory::Trigger),
	FieldSpec::new("civ_allies_below", ValueSpec::single(ValueType::Int), "盟友数量低于", FieldCategory::Trigger),
	FieldSpec::new("civ_defensive_pacts_below", ValueSpec::single(ValueType::Int), "防御条约数量低于", FieldCategory::Trigger),
	FieldSpec::new("civ_non_aggression_pacts_below", ValueSpec::single(ValueType::Int), "互不侵犯条约数量低于", FieldCategory::Trigger),
	FieldSpec::new("civ_is_at_war_days", ValueSpec::single(ValueType::Int), "战争持续天数（变体写法）", FieldCategory::Trigger),
	FieldSpec::new("civ_supreme_court_over", ValueSpec::single(ValueType::Int), "最高法院值高于", FieldCategory::Trigger),
	FieldSpec::new("civ_research_points_over", ValueSpec::single(ValueType::Int), "科研点数高于", FieldCategory::Trigger),
	FieldSpec::new("civ_research_per_month_over", ValueSpec::single(ValueType::Float), "每月科研高于", FieldCategory::Trigger),
	FieldSpec::new("civ_legacy_per_month_over", ValueSpec::single(ValueType::Float), "每月威望高于", FieldCategory::Trigger),
	FieldSpec::new("civ_manpower_perc_below", ValueSpec::single(ValueType::Float), "人力百分比低于", FieldCategory::Trigger),
	FieldSpec::new("civ_diplomacy_over", ValueSpec::single(ValueType::Int), "外交点高于", FieldCategory::Trigger),
	FieldSpec::new("civ_capital_has_building", ValueSpec::single(ValueType::Str), "首都拥有建筑（建筑 ID；或 `建筑ID=等级`）", FieldCategory::Trigger),
	FieldSpec::new("civ_capital_is_occupied", ValueSpec::single(ValueType::Bool), "首都是否被占领", FieldCategory::Trigger),
	FieldSpec::new("civ_has_more_provinces_than_civ", ValueSpec::pair(ValueType::Str, ValueType::Str), "省份数多于另一文明（如 unnn=pol）", FieldCategory::Trigger),
	FieldSpec::new("civ_has_more_regiments_than_civ", ValueSpec::pair(ValueType::Str, ValueType::Str), "军团数多于另一文明（如 ger=czsl）", FieldCategory::Trigger),
	FieldSpec::new("civs_are_neighbors", ValueSpec::pair(ValueType::Str, ValueType::Str), "两文明是否相邻（如 rus=ukr）", FieldCategory::Trigger),
	FieldSpec::new("province_civ_has_core", ValueSpec::pair(ValueType::Int, ValueType::Str), "省份拥有某文明核心（省份=文明）", FieldCategory::Trigger),
	FieldSpec::new("province_not_controlled_by", ValueSpec::pair(ValueType::Int, ValueType::Str), "省份未被某文明控制（省份=文明）", FieldCategory::Trigger),
	FieldSpec::new("province_is_capital", ValueSpec::single(ValueType::Int), "省份是否为首都（省份 ID）", FieldCategory::Trigger),
	FieldSpec::new("province_unrest_below", ValueSpec::single(ValueType::Float), "省份动荡低于", FieldCategory::Trigger),
	FieldSpec::new("province_stability", ValueSpec::single(ValueType::Float), "省份稳定度", FieldCategory::Trigger),
	FieldSpec::new("increased_manpower_over", ValueSpec::single(ValueType::Int), "人力增长值高于", FieldCategory::Trigger),
	FieldSpec::new("buildings_constructed_over", ValueSpec::single(ValueType::Int), "已建造建筑数高于", FieldCategory::Trigger),
	FieldSpec::new("administrative_buildings_constructed_over", ValueSpec::single(ValueType::Int), "已建造行政建筑数高于", FieldCategory::Trigger),
	FieldSpec::new("military_buildings_constructed_over", ValueSpec::single(ValueType::Int), "已建造军事建筑数高于", FieldCategory::Trigger),
	FieldSpec::new("economy_buildings_constructed_over", ValueSpec::single(ValueType::Int), "已建造经济建筑数高于", FieldCategory::Trigger),
	FieldSpec::new("developed_infrastructure_over", ValueSpec::single(ValueType::Int), "已开发基建数高于", FieldCategory::Trigger),
	FieldSpec::new("invested_in_economy_over", ValueSpec::single(ValueType::Int), "经济投资次数高于", FieldCategory::Trigger),
	FieldSpec::new("increased_growth_rate_over", ValueSpec::single(ValueType::Int), "增长率提升次数高于", FieldCategory::Trigger),
	FieldSpec::new("increased_tax_efficiency_over", ValueSpec::single(ValueType::Int), "税收效率提升次数高于", FieldCategory::Trigger),
	// 实测模组常见键（第三批：GameCivs 五模组 missionsEvents + APK 全局 events 统计）
	FieldSpec::new("civs_opinion_below", ValueSpec::triple(ValueType::Str, ValueType::Str, ValueType::Float), "两文明关系低于（`文明A=文明B=数值`，实测 ukr=rus=-50.0）", FieldCategory::Trigger),
	FieldSpec::new("largest_producer_production_over", ValueSpec::pair(ValueType::Int, ValueType::Int), "最大生产国产量高于（资源ID=产量；APK 全局事件实测）", FieldCategory::Trigger),
	FieldSpec::new("civ_has_resource", ValueSpec::single(ValueType::Int), "文明是否拥有某资源（资源 ID；APK 全局事件实测）", FieldCategory::Trigger),
	FieldSpec::new("civ_has_resource_over", ValueSpec::pair(ValueType::Int, ValueType::Int), "文明某资源数量高于（资源ID=数量；APK 全局事件实测）", FieldCategory::Trigger),
	// AoH3 官方 FAQ（game/_FAQ/Events_Triggers.txt）实测键——2026-10-09 批量入库（手机引擎 classes.dex 字符串表全部命中）。
	FieldSpec::new("administrative_buildings_constructed_below", ValueSpec::single(ValueType::Int), "已建造行政建筑数低于", FieldCategory::Trigger),
	FieldSpec::new("alliance_special_is_leader", ValueSpec::single(ValueType::Bool), "是否为特殊联盟（如神罗）领袖", FieldCategory::Trigger),
	FieldSpec::new("alliance_special_is_leader_id", ValueSpec::single(ValueType::Int), "特殊联盟领袖 ID 是", FieldCategory::Trigger),
	FieldSpec::new("alliance_special_is_not_member_id", ValueSpec::single(ValueType::Int), "不是特殊联盟成员 ID（联盟编号）", FieldCategory::Trigger),
	FieldSpec::new("buildings_constructed_below", ValueSpec::single(ValueType::Int), "已建造建筑数低于", FieldCategory::Trigger),
	FieldSpec::new("civ_administrative_advisor_skill_below", ValueSpec::single(ValueType::Int), "行政顾问技能低于", FieldCategory::Trigger),
	FieldSpec::new("civ_advisor_age_over", ValueSpec::pair(ValueType::Int, ValueType::Int), "某类型顾问年龄高于（类型=年龄）", FieldCategory::Trigger),
	FieldSpec::new("civ_advisor_construction_cost_over", ValueSpec::pair(ValueType::Int, ValueType::Int), "某类型顾问建造成本高于（类型=数值）", FieldCategory::Trigger),
	FieldSpec::new("civ_advisor_production_efficiency_over", ValueSpec::pair(ValueType::Int, ValueType::Float), "某类型顾问生产效率高于（类型=数值）", FieldCategory::Trigger),
	FieldSpec::new("civ_battle_width_below", ValueSpec::single(ValueType::Int), "战斗宽度低于", FieldCategory::Trigger),
	FieldSpec::new("civ_capital_buildings_below", ValueSpec::single(ValueType::Int), "首都建筑数低于", FieldCategory::Trigger),
	FieldSpec::new("civ_capital_buildings_over", ValueSpec::single(ValueType::Int), "首都建筑数高于", FieldCategory::Trigger),
	FieldSpec::new("civ_capital_city_below", ValueSpec::single(ValueType::Int), "首都城市等级低于", FieldCategory::Trigger),
	FieldSpec::new("civ_capital_city_over", ValueSpec::single(ValueType::Int), "首都城市等级高于", FieldCategory::Trigger),
	FieldSpec::new("civ_capital_continent_is_not", ValueSpec::single(ValueType::Int), "首都所在大洲不是（大洲编号）", FieldCategory::Trigger),
	FieldSpec::new("civ_capital_economy_below", ValueSpec::single(ValueType::Float), "首都经济低于", FieldCategory::Trigger),
	FieldSpec::new("civ_capital_economy_over", ValueSpec::single(ValueType::Float), "首都经济高于", FieldCategory::Trigger),
	FieldSpec::new("civ_capital_fort_level_below", ValueSpec::single(ValueType::Int), "首都堡垒等级低于", FieldCategory::Trigger),
	FieldSpec::new("civ_capital_fort_level_over", ValueSpec::single(ValueType::Int), "首都堡垒等级高于", FieldCategory::Trigger),
	FieldSpec::new("civ_capital_growth_rate_below", ValueSpec::single(ValueType::Float), "首都增长率低于", FieldCategory::Trigger),
	FieldSpec::new("civ_capital_growth_rate_over", ValueSpec::single(ValueType::Float), "首都增长率高于", FieldCategory::Trigger),
	FieldSpec::new("civ_capital_income_below", ValueSpec::single(ValueType::Float), "首都收入低于", FieldCategory::Trigger),
	FieldSpec::new("civ_capital_income_over", ValueSpec::single(ValueType::Float), "首都收入高于", FieldCategory::Trigger),
	FieldSpec::new("civ_capital_infrastructure_below", ValueSpec::single(ValueType::Int), "首都基建低于", FieldCategory::Trigger),
	FieldSpec::new("civ_capital_infrastructure_over", ValueSpec::single(ValueType::Int), "首都基建高于", FieldCategory::Trigger),
	FieldSpec::new("civ_capital_is_not_occupied", ValueSpec::single(ValueType::Bool), "首都未被占领", FieldCategory::Trigger),
	FieldSpec::new("civ_capital_is_under_siege", ValueSpec::single(ValueType::Bool), "首都被围困", FieldCategory::Trigger),
	FieldSpec::new("civ_capital_manpower_below", ValueSpec::single(ValueType::Float), "首都人力低于", FieldCategory::Trigger),
	FieldSpec::new("civ_capital_manpower_over", ValueSpec::single(ValueType::Float), "首都人力高于", FieldCategory::Trigger),
	FieldSpec::new("civ_capital_population_below", ValueSpec::single(ValueType::Int), "首都人口低于", FieldCategory::Trigger),
	FieldSpec::new("civ_capital_population_over", ValueSpec::single(ValueType::Int), "首都人口高于", FieldCategory::Trigger),
	FieldSpec::new("civ_capital_religion_is", ValueSpec::single(ValueType::Int), "首都宗教是（宗教编号）", FieldCategory::Trigger),
	FieldSpec::new("civ_capital_tax_efficiency_below", ValueSpec::single(ValueType::Float), "首都税收效率低于", FieldCategory::Trigger),
	FieldSpec::new("civ_capital_tax_efficiency_over", ValueSpec::single(ValueType::Float), "首都税收效率高于", FieldCategory::Trigger),
	FieldSpec::new("civ_conquered_provinces_below", ValueSpec::single(ValueType::Int), "已征服省份数低于", FieldCategory::Trigger),
	FieldSpec::new("civ_defensive_pacts_over", ValueSpec::single(ValueType::Int), "防御条约数高于", FieldCategory::Trigger),
	FieldSpec::new("civ_diplomacy_below", ValueSpec::single(ValueType::Float), "外交值低于", FieldCategory::Trigger),
	FieldSpec::new("civ_economic_advisor_skill_below", ValueSpec::single(ValueType::Int), "经济顾问技能低于", FieldCategory::Trigger),
	FieldSpec::new("civ_economy_below", ValueSpec::single(ValueType::Int), "经济总量低于", FieldCategory::Trigger),
	FieldSpec::new("civ_economy_over", ValueSpec::single(ValueType::Int), "经济总量高于", FieldCategory::Trigger),
	FieldSpec::new("civ_gold_over_max_amount_of_gold", ValueSpec::single(ValueType::Bool), "黄金超过上限（true=超过）", FieldCategory::Trigger),
	FieldSpec::new("civ_has_higher_ranking_than_civ", ValueSpec::pair(ValueType::Str, ValueType::Str), "排名高于另一文明（文明A=文明B）", FieldCategory::Trigger),
	FieldSpec::new("civ_has_larger_economy_than_civ", ValueSpec::pair(ValueType::Str, ValueType::Str), "经济大于另一文明", FieldCategory::Trigger),
	FieldSpec::new("civ_has_larger_population_than_civ", ValueSpec::pair(ValueType::Str, ValueType::Str), "人口大于另一文明", FieldCategory::Trigger),
	FieldSpec::new("civ_has_larger_regiments_limit_than_civ", ValueSpec::pair(ValueType::Str, ValueType::Str), "军团上限大于另一文明", FieldCategory::Trigger),
	FieldSpec::new("civ_has_more_technologies_than_civ", ValueSpec::pair(ValueType::Str, ValueType::Str), "科技数多于另一文明", FieldCategory::Trigger),
	FieldSpec::new("civ_has_rivalry", ValueSpec::pair(ValueType::Str, ValueType::Str), "与某文明为宿敌（文明A=文明B）", FieldCategory::Trigger),
	FieldSpec::new("civ_has_rivalry_not", ValueSpec::pair(ValueType::Str, ValueType::Str), "与某文明不是宿敌", FieldCategory::Trigger),
	FieldSpec::new("civ_have_guarantee", ValueSpec::pair(ValueType::Str, ValueType::Str), "与某文明有保证（给予方=获得方）", FieldCategory::Trigger),
	FieldSpec::new("civ_have_military_access", ValueSpec::pair(ValueType::Str, ValueType::Str), "与某文明有军事通行权", FieldCategory::Trigger),
	FieldSpec::new("civ_income_economy_below", ValueSpec::single(ValueType::Float), "经济收入低于", FieldCategory::Trigger),
	FieldSpec::new("civ_income_economy_over", ValueSpec::single(ValueType::Float), "经济收入高于", FieldCategory::Trigger),
	FieldSpec::new("civ_income_production_below", ValueSpec::single(ValueType::Float), "生产收入低于", FieldCategory::Trigger),
	FieldSpec::new("civ_income_production_over", ValueSpec::single(ValueType::Float), "生产收入高于", FieldCategory::Trigger),
	FieldSpec::new("civ_income_taxation_below", ValueSpec::single(ValueType::Float), "税收收入低于", FieldCategory::Trigger),
	FieldSpec::new("civ_income_taxation_over", ValueSpec::single(ValueType::Float), "税收收入高于", FieldCategory::Trigger),
	FieldSpec::new("civ_inflation_below", ValueSpec::single(ValueType::Float), "通胀低于", FieldCategory::Trigger),
	FieldSpec::new("civ_inflation_over", ValueSpec::single(ValueType::Float), "通胀高于", FieldCategory::Trigger),
	FieldSpec::new("civ_innovation_advisor_skill_below", ValueSpec::single(ValueType::Int), "创新顾问技能低于", FieldCategory::Trigger),
	FieldSpec::new("civ_is_largest_producer", ValueSpec::single(ValueType::Int), "是否为某资源最大生产国（资源ID）", FieldCategory::Trigger),
	FieldSpec::new("civ_largest_producer_over", ValueSpec::single(ValueType::Int), "最大生产国产量高于（资源ID）", FieldCategory::Trigger),
	FieldSpec::new("civ_legacy_per_month_below", ValueSpec::single(ValueType::Float), "每月威望低于", FieldCategory::Trigger),
	FieldSpec::new("civ_loans_below", ValueSpec::single(ValueType::Int), "贷款数低于", FieldCategory::Trigger),
	FieldSpec::new("civ_loans_over", ValueSpec::single(ValueType::Int), "贷款数高于", FieldCategory::Trigger),
	FieldSpec::new("civ_manpower_perc_over", ValueSpec::single(ValueType::Float), "人力百分比高于", FieldCategory::Trigger),
	FieldSpec::new("civ_max_manpower_below", ValueSpec::single(ValueType::Int), "最大人力低于", FieldCategory::Trigger),
	FieldSpec::new("civ_max_manpower_over", ValueSpec::single(ValueType::Int), "最大人力高于", FieldCategory::Trigger),
	FieldSpec::new("civ_military_academy_below", ValueSpec::single(ValueType::Int), "军事学院等级低于", FieldCategory::Trigger),
	FieldSpec::new("civ_military_academy_for_generals_below", ValueSpec::single(ValueType::Int), "将领军事学院等级低于", FieldCategory::Trigger),
	FieldSpec::new("civ_military_academy_for_generals_over", ValueSpec::single(ValueType::Int), "将领军事学院等级高于", FieldCategory::Trigger),
	FieldSpec::new("civ_military_academy_over", ValueSpec::single(ValueType::Int), "军事学院等级高于", FieldCategory::Trigger),
	FieldSpec::new("civ_military_advisor_skill_below", ValueSpec::single(ValueType::Int), "军事顾问技能低于", FieldCategory::Trigger),
	FieldSpec::new("civ_neighbors_below", ValueSpec::single(ValueType::Int), "邻国数低于", FieldCategory::Trigger),
	FieldSpec::new("civ_neighbors_over", ValueSpec::single(ValueType::Int), "邻国数高于", FieldCategory::Trigger),
	FieldSpec::new("civ_non_aggression_pacts_over", ValueSpec::single(ValueType::Int), "互不侵犯条约数高于", FieldCategory::Trigger),
	FieldSpec::new("civ_nuclear_reactor_below", ValueSpec::single(ValueType::Int), "核反应堆等级低于", FieldCategory::Trigger),
	FieldSpec::new("civ_nuclear_reactor_over", ValueSpec::single(ValueType::Int), "核反应堆等级高于", FieldCategory::Trigger),
	FieldSpec::new("civ_prestige_below", ValueSpec::single(ValueType::Int), "威望低于", FieldCategory::Trigger),
	FieldSpec::new("civ_production_over", ValueSpec::pair(ValueType::Int, ValueType::Int), "某资源产量高于（资源ID=产量）", FieldCategory::Trigger),
	FieldSpec::new("civ_provinces_equals", ValueSpec::single(ValueType::Int), "省份数等于", FieldCategory::Trigger),
	FieldSpec::new("civ_rank_position_below", ValueSpec::single(ValueType::Int), "排名低于（名次数值）", FieldCategory::Trigger),
	FieldSpec::new("civ_rank_position_over", ValueSpec::single(ValueType::Int), "排名高于", FieldCategory::Trigger),
	FieldSpec::new("civ_regiments_below", ValueSpec::single(ValueType::Int), "军团数低于", FieldCategory::Trigger),
	FieldSpec::new("civ_regiments_limit_below", ValueSpec::single(ValueType::Int), "军团上限低于", FieldCategory::Trigger),
	FieldSpec::new("civ_regiments_over_regiments_limit", ValueSpec::single(ValueType::Bool), "军团数超过军团上限", FieldCategory::Trigger),
	FieldSpec::new("civ_research_per_month_below", ValueSpec::single(ValueType::Float), "每月研究点低于", FieldCategory::Trigger),
	FieldSpec::new("civ_supreme_court_below", ValueSpec::single(ValueType::Int), "最高法院等级低于", FieldCategory::Trigger),
	FieldSpec::new("civ_tag_government_is", ValueSpec::pair(ValueType::Str, ValueType::Int), "某文明政体是（文明=政体编号）", FieldCategory::Trigger),
	FieldSpec::new("civ_tag_government_is_not", ValueSpec::pair(ValueType::Str, ValueType::Int), "某文明政体不是", FieldCategory::Trigger),
	FieldSpec::new("civ_tag_religion_is", ValueSpec::pair(ValueType::Str, ValueType::Int), "某文明宗教是（文明=宗教编号）", FieldCategory::Trigger),
	FieldSpec::new("civ_tag_religion_is_not", ValueSpec::pair(ValueType::Str, ValueType::Int), "某文明宗教不是", FieldCategory::Trigger),
	FieldSpec::new("civ_total_income_below", ValueSpec::single(ValueType::Float), "总收入低于", FieldCategory::Trigger),
	FieldSpec::new("civ_total_income_over", ValueSpec::single(ValueType::Float), "总收入高于", FieldCategory::Trigger),
	FieldSpec::new("civ_unlocked_advantages_below", ValueSpec::single(ValueType::Int), "已解锁优势低于", FieldCategory::Trigger),
	FieldSpec::new("civ_unlocked_legacies_below", ValueSpec::single(ValueType::Int), "已解锁遗产低于", FieldCategory::Trigger),
	FieldSpec::new("civ_unlocked_technologies_below", ValueSpec::single(ValueType::Int), "已解锁科技低于", FieldCategory::Trigger),
	FieldSpec::new("civ_unlocked_technologies_over", ValueSpec::single(ValueType::Int), "已解锁科技高于", FieldCategory::Trigger),
	FieldSpec::new("civ_vassals_below", ValueSpec::single(ValueType::Int), "附庸数低于", FieldCategory::Trigger),
	FieldSpec::new("civ_wars_total_below", ValueSpec::single(ValueType::Int), "战争总数低于", FieldCategory::Trigger),
	FieldSpec::new("civs_are_not_at_war", ValueSpec::pair(ValueType::Str, ValueType::Str), "两文明未处于战争", FieldCategory::Trigger),
	FieldSpec::new("civs_are_not_neighbors", ValueSpec::pair(ValueType::Str, ValueType::Str), "两文明不相邻", FieldCategory::Trigger),
	FieldSpec::new("civs_are_rivals", ValueSpec::pair(ValueType::Str, ValueType::Str), "两文明为宿敌", FieldCategory::Trigger),
	FieldSpec::new("civs_have_alliance", ValueSpec::pair(ValueType::Str, ValueType::Str), "两文明有联盟", FieldCategory::Trigger),
	FieldSpec::new("civs_have_defensive_pact", ValueSpec::pair(ValueType::Str, ValueType::Str), "两文明有防御条约", FieldCategory::Trigger),
	FieldSpec::new("civs_have_non_aggression", ValueSpec::pair(ValueType::Str, ValueType::Str), "两文明有互不侵犯", FieldCategory::Trigger),
	FieldSpec::new("civs_have_truce", ValueSpec::pair(ValueType::Str, ValueType::Str), "两文明有休战", FieldCategory::Trigger),
	FieldSpec::new("civs_opinion_over", ValueSpec::triple(ValueType::Str, ValueType::Str, ValueType::Float), "两文明关系高于（文明A=文明B=数值）", FieldCategory::Trigger),
	FieldSpec::new("developed_infrastructure_below", ValueSpec::single(ValueType::Int), "已开发基建数低于", FieldCategory::Trigger),
	FieldSpec::new("economy_buildings_constructed_below", ValueSpec::single(ValueType::Int), "已建造经济建筑数低于", FieldCategory::Trigger),
	FieldSpec::new("increased_growth_rate_below", ValueSpec::single(ValueType::Int), "增长率提升次数低于", FieldCategory::Trigger),
	FieldSpec::new("increased_manpower_below", ValueSpec::single(ValueType::Int), "人力提升次数低于", FieldCategory::Trigger),
	FieldSpec::new("increased_tax_efficiency_below", ValueSpec::single(ValueType::Int), "税收效率提升次数低于", FieldCategory::Trigger),
	FieldSpec::new("invested_in_economy_below", ValueSpec::single(ValueType::Int), "经济投资次数低于", FieldCategory::Trigger),
	FieldSpec::new("military_buildings_constructed_below", ValueSpec::single(ValueType::Int), "已建造军事建筑数低于", FieldCategory::Trigger),
	FieldSpec::new("not_exists", ValueSpec::single(ValueType::Str), "文明不存在（文明标签）", FieldCategory::Trigger),
	FieldSpec::new("playing_time_below", ValueSpec::single(ValueType::Int), "游戏时长低于（天数）", FieldCategory::Trigger),
	FieldSpec::new("province_buildings_below", ValueSpec::pair(ValueType::Int, ValueType::Int), "省份建筑数低于（省份=数量）", FieldCategory::Trigger),
	FieldSpec::new("province_buildings_limit_below", ValueSpec::pair(ValueType::Int, ValueType::Int), "省份建筑上限低于", FieldCategory::Trigger),
	FieldSpec::new("province_buildings_limit_over", ValueSpec::pair(ValueType::Int, ValueType::Int), "省份建筑上限高于", FieldCategory::Trigger),
	FieldSpec::new("province_buildings_over", ValueSpec::pair(ValueType::Int, ValueType::Int), "省份建筑数高于", FieldCategory::Trigger),
	FieldSpec::new("province_defense_lvl_below", ValueSpec::pair(ValueType::Int, ValueType::Int), "省份防御等级低于", FieldCategory::Trigger),
	FieldSpec::new("province_defense_lvl_over", ValueSpec::pair(ValueType::Int, ValueType::Int), "省份防御等级高于", FieldCategory::Trigger),
	FieldSpec::new("province_economy_below", ValueSpec::pair(ValueType::Int, ValueType::Float), "省份经济低于（省份=数值）", FieldCategory::Trigger),
	FieldSpec::new("province_economy_over", ValueSpec::pair(ValueType::Int, ValueType::Float), "省份经济高于", FieldCategory::Trigger),
	FieldSpec::new("province_growth_rate_below", ValueSpec::pair(ValueType::Int, ValueType::Float), "省份增长率低于", FieldCategory::Trigger),
	FieldSpec::new("province_growth_rate_over", ValueSpec::pair(ValueType::Int, ValueType::Float), "省份增长率高于", FieldCategory::Trigger),
	FieldSpec::new("province_income_below", ValueSpec::pair(ValueType::Int, ValueType::Float), "省份收入低于", FieldCategory::Trigger),
	FieldSpec::new("province_income_over", ValueSpec::pair(ValueType::Int, ValueType::Float), "省份收入高于", FieldCategory::Trigger),
	FieldSpec::new("province_infrastructure_below", ValueSpec::pair(ValueType::Int, ValueType::Int), "省份基建低于", FieldCategory::Trigger),
	FieldSpec::new("province_infrastructure_over", ValueSpec::pair(ValueType::Int, ValueType::Int), "省份基建高于", FieldCategory::Trigger),
	FieldSpec::new("province_is_not_occupied", ValueSpec::single(ValueType::Int), "省份未被占领（省份ID）", FieldCategory::Trigger),
	FieldSpec::new("province_manpower_below", ValueSpec::pair(ValueType::Int, ValueType::Float), "省份人力低于", FieldCategory::Trigger),
	FieldSpec::new("province_population_below", ValueSpec::pair(ValueType::Int, ValueType::Int), "省份人口低于（省份=数量）", FieldCategory::Trigger),
	FieldSpec::new("province_religion_is", ValueSpec::pair(ValueType::Int, ValueType::Int), "省份宗教是（省份=宗教编号）", FieldCategory::Trigger),
	FieldSpec::new("province_religion_is_not", ValueSpec::pair(ValueType::Int, ValueType::Int), "省份宗教不是", FieldCategory::Trigger),
	FieldSpec::new("province_tax_efficiency_below", ValueSpec::pair(ValueType::Int, ValueType::Float), "省份税收效率低于", FieldCategory::Trigger),
	FieldSpec::new("province_tax_efficiency_over", ValueSpec::pair(ValueType::Int, ValueType::Float), "省份税收效率高于", FieldCategory::Trigger),
	FieldSpec::new("recruited_advisors_below", ValueSpec::single(ValueType::Int), "已招募顾问数低于", FieldCategory::Trigger),
	FieldSpec::new("recruited_advisors_over", ValueSpec::single(ValueType::Int), "已招募顾问数高于", FieldCategory::Trigger),
	FieldSpec::new("unique_capital_buildings_constructed_below", ValueSpec::single(ValueType::Int), "首都特有建筑数低于", FieldCategory::Trigger),
	FieldSpec::new("unique_capital_buildings_constructed_over", ValueSpec::single(ValueType::Int), "首都特有建筑数高于", FieldCategory::Trigger),
];

// ===== 收益效果（sqlite.rs MISSIONS_OPTIONS_BUTTON + 说明文档） =====

pub const EFFECT_FIELDS: &[FieldSpec] = &[
	FieldSpec::new("legacy", ValueSpec::single(ValueType::Float), "威望/遗产（可小数）", FieldCategory::Effect),
	FieldSpec::new("gold", ValueSpec::single(ValueType::Int), "金币", FieldCategory::Effect),
	FieldSpec::new("manpower", ValueSpec::single(ValueType::Int), "人力", FieldCategory::Effect),
	FieldSpec::new("bonus_duration", ValueSpec::single(ValueType::Float), "后续加成的持续时间（99=永久；可为小数）", FieldCategory::Effect),
	FieldSpec::new("bonus_monthly_legacy", ValueSpec::single(ValueType::Float), "每月威望加成（如 0.6）", FieldCategory::Effect),
	FieldSpec::new("bonus_monthly_income", ValueSpec::single(ValueType::Float), "每月收入加成", FieldCategory::Effect),
	FieldSpec::new("bonus_units_attack", ValueSpec::single(ValueType::Float), "部队攻击加成", FieldCategory::Effect),
	FieldSpec::new("bonus_units_defense", ValueSpec::single(ValueType::Float), "部队防御加成", FieldCategory::Effect),
	FieldSpec::new("bonus_army_movement_speed", ValueSpec::single(ValueType::Float), "军队移动速度加成（如 0.1）", FieldCategory::Effect),
	FieldSpec::new("bonus_army_morale_recovery", ValueSpec::single(ValueType::Float), "军队士气恢复速度加成", FieldCategory::Effect),
	FieldSpec::new("bonus_recruitment_time", ValueSpec::single(ValueType::Float), "征召时间加成", FieldCategory::Effect),
	FieldSpec::new("bonus_manpower_recovery_speed", ValueSpec::single(ValueType::Float), "人力恢复速度加成", FieldCategory::Effect),
	FieldSpec::new("bonus_max_manpower", ValueSpec::single(ValueType::Float), "最大人力加成", FieldCategory::Effect),
	FieldSpec::new("bonus_max_manpower_percentage", ValueSpec::single(ValueType::Float), "最大人力百分比加成", FieldCategory::Effect),
	FieldSpec::new("bonus_research", ValueSpec::single(ValueType::Float), "科研槽加成", FieldCategory::Effect),
	FieldSpec::new("bonus_research_points", ValueSpec::single(ValueType::Float), "科研点数加成", FieldCategory::Effect),
	FieldSpec::new("bonus_generals_attack", ValueSpec::single(ValueType::Float), "将领攻击加成", FieldCategory::Effect),
	FieldSpec::new("bonus_generals_defense", ValueSpec::single(ValueType::Float), "将领防御加成", FieldCategory::Effect),
	FieldSpec::new("bonus_income_production", ValueSpec::single(ValueType::Float), "收入产出加成", FieldCategory::Effect),
	FieldSpec::new("bonus_production_efficiency", ValueSpec::single(ValueType::Float), "生产效率加成", FieldCategory::Effect),
	FieldSpec::new("bonus_tax_efficiency", ValueSpec::single(ValueType::Float), "税收效率加成", FieldCategory::Effect),
	FieldSpec::new("bonus_construction_cost", ValueSpec::single(ValueType::Float), "建筑成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_construction_time", ValueSpec::single(ValueType::Float), "建筑时间加成（负数=减少）", FieldCategory::Effect),
	FieldSpec::new("bonus_discipline", ValueSpec::single(ValueType::Float), "纪律加成", FieldCategory::Effect),
	FieldSpec::new("bonus_loans_limit", ValueSpec::single(ValueType::Float), "贷款上限加成", FieldCategory::Effect),
	FieldSpec::new("bonus_administration_buildings_cost", ValueSpec::single(ValueType::Float), "行政建筑成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_military_buildings_cost", ValueSpec::single(ValueType::Float), "军事建筑成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_economy_buildings_cost", ValueSpec::single(ValueType::Float), "经济建筑成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_army_maintenance", ValueSpec::single(ValueType::Float), "军队维护成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_province_maintenance", ValueSpec::single(ValueType::Float), "省份维护成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_buildings_maintenance_cost", ValueSpec::single(ValueType::Float), "建筑维护成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_recruit_army_cost", ValueSpec::single(ValueType::Float), "征召军队成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_recruit_army_first_line_cost", ValueSpec::single(ValueType::Float), "征召一线部队成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_recruit_army_second_line_cost", ValueSpec::single(ValueType::Float), "征召二线部队成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_reinforcement_speed", ValueSpec::single(ValueType::Float), "增援速度加成", FieldCategory::Effect),
	FieldSpec::new("province_unrest_all", ValueSpec::single(ValueType::Float), "全境动荡变化（-99=消除动荡）", FieldCategory::Effect),
	FieldSpec::new("province_economy_capital_all", ValueSpec::single(ValueType::Float), "首都所有经济", FieldCategory::Effect),
	FieldSpec::new("province_economy_capital_bul", ValueSpec::single(ValueType::Float), "首都经济（保加利亚特化）", FieldCategory::Effect),
	FieldSpec::new("province_economy_all", ValueSpec::single(ValueType::Float), "所有省份经济", FieldCategory::Effect),
	FieldSpec::new("province_economy", ValueSpec::single(ValueType::Float), "省份经济", FieldCategory::Effect),
	FieldSpec::new("province_economy_id", ValueSpec::lambda(ValueType::Int, ValueType::Float, ValueType::Float), "指定省份经济（`省份=数值` 或 `省份列表;=数值`）", FieldCategory::Effect),
	FieldSpec::new("province_infrastructure_all", ValueSpec::single(ValueType::Int), "所有省份基础设施", FieldCategory::Effect),
	FieldSpec::new("province_infrastructure", ValueSpec::single(ValueType::Int), "省份基础设施", FieldCategory::Effect),
	FieldSpec::new("province_growth_rate_all", ValueSpec::single(ValueType::Float), "所有省份增长率", FieldCategory::Effect),
	FieldSpec::new("province_growth_rate", ValueSpec::single(ValueType::Float), "省份增长率", FieldCategory::Effect),
	FieldSpec::new("province_population_all", ValueSpec::single(ValueType::Int), "所有省份人口", FieldCategory::Effect),
	FieldSpec::new("province_religion_all", ValueSpec::single(ValueType::Int), "所有省份宗教", FieldCategory::Effect),
	FieldSpec::new("province_manpower_id", ValueSpec::lambda(ValueType::Int, ValueType::Float, ValueType::Float), "指定省份人力（`省份=数值` 或 `省份列表;=数值`）", FieldCategory::Effect),
	FieldSpec::new("province_add_building", ValueSpec::lambda(ValueType::Int, ValueType::Int, ValueType::Int), "添加建筑（如 3994;=6=0）", FieldCategory::Effect),
	FieldSpec::new("province_add_core_civ", ValueSpec::lambda(ValueType::Int, ValueType::Str, ValueType::Str), "添加核心省份（`省份列表;=文明标签`）", FieldCategory::Effect),
	FieldSpec::new("change_ideology", ValueSpec::single(ValueType::Int), "改变意识形态倾向", FieldCategory::Effect),
	FieldSpec::new("change_ideology_civ", ValueSpec::pair(ValueType::Int, ValueType::Str), "改变某文明意识形态（意识形态序号=文明标签，如 `0=LRF`）", FieldCategory::Effect),
	FieldSpec::new("set_civ_tag", ValueSpec::single(ValueType::Str), "设置文明标签/变身", FieldCategory::Effect),
	FieldSpec::new("set_civ_tag2", ValueSpec::pair(ValueType::Str, ValueType::Str), "设置文明标签2（如 ger=ger_c）", FieldCategory::Effect),
	FieldSpec::new("set_civ_tag_reset", ValueSpec::single(ValueType::Str), "重置文明标签", FieldCategory::Effect),
	FieldSpec::new("player_set_civ", ValueSpec::single(ValueType::Str), "将玩家设置为某文明", FieldCategory::Effect),
	FieldSpec::new("annex_civ", ValueSpec::single(ValueType::Str), "吞并某文明（可多次使用吞并多个）", FieldCategory::Effect),
	FieldSpec::new("annexed_by_civ", ValueSpec::single(ValueType::Str), "被某文明吞并", FieldCategory::Effect),
	FieldSpec::new("annex_by_civ_from_civ", ValueSpec::triple(ValueType::Str, ValueType::Str, ValueType::Str), "从某文明吞并给另一文明", FieldCategory::Effect),
	FieldSpec::new("make_puppet", ValueSpec::pair(ValueType::Str, ValueType::Str), "成为某文明傀儡", FieldCategory::Effect),
	FieldSpec::new("annex", ValueSpec::list(ValueType::Int), "吞并（省份 ID，可分号分隔多个）", FieldCategory::Effect),
	FieldSpec::new("add_ruler", ValueSpec::single(ValueType::Str), "添加统治者（名字=姓氏=头像=日=月=年；头像可写数字编号或图片名，如 `陶尔斐斯= =陶尔斐斯=4=10=1892`）", FieldCategory::Effect),
	FieldSpec::new("add_general2", ValueSpec::single(ValueType::Str), "添加将领", FieldCategory::Effect),
	FieldSpec::new("add_general", ValueSpec::single(ValueType::Str), "添加将领（旧版）", FieldCategory::Effect),
	FieldSpec::new("add_ruler_custom", ValueSpec::single(ValueType::Str), "自定义统治者（文明标签=该文明统治者序号；取自 `rulers/<标签>.json` 的 Rulers 数组）", FieldCategory::Effect),
	FieldSpec::new("add_advisor", ValueSpec::single(ValueType::Int), "添加顾问（0 行政 / 1 经济 / 2 创新 / 3 军事；获得随机对应类型顾问）", FieldCategory::Effect),
	FieldSpec::new("run_event", ValueSpec::single(ValueType::Str), "运行某事件", FieldCategory::Effect),
	FieldSpec::new("run_event_instantly", ValueSpec::single(ValueType::Str), "立即运行某事件", FieldCategory::Effect),
	FieldSpec::new("white_peace", ValueSpec::pair(ValueType::Str, ValueType::Str), "与某文明白和平（文明A=文明B，可空段）", FieldCategory::Effect),
	FieldSpec::new("declare_war2", ValueSpec::pair(ValueType::Str, ValueType::Str), "向某文明宣战（宣战国=被宣战国）", FieldCategory::Effect),
	FieldSpec::new("declare_war", ValueSpec::single(ValueType::Str), "向某文明宣战（旧版）", FieldCategory::Effect),
	FieldSpec::new("move_capital", ValueSpec::single(ValueType::Int), "迁都到某省份", FieldCategory::Effect),
	FieldSpec::new("add_ns", ValueSpec::single(ValueType::Str), "添加国家精神", FieldCategory::Effect),
	FieldSpec::new("remove_ns", ValueSpec::single(ValueType::Str), "移除国家精神", FieldCategory::Effect),
	FieldSpec::new("add_new_army", ValueSpec::sequence(ValueType::Int, ValueType::Int), "添加新军队（兵种ID=型号ID 成对重复；兵种见 units/Units.json，型号为该兵种文件内 Army 数组下标）", FieldCategory::Effect),
	FieldSpec::new("add_defensive_pact", ValueSpec::pair(ValueType::Str, ValueType::Str), "添加防御条约（如 fra=uni）", FieldCategory::Effect),
	FieldSpec::new("add_guarantee", ValueSpec::pair(ValueType::Str, ValueType::Str), "添加保证（如 fra=bel）", FieldCategory::Effect),
	FieldSpec::new("add_truce", ValueSpec::pair(ValueType::Str, ValueType::Str), "添加休战协议（如 atr=ser）", FieldCategory::Effect),
	FieldSpec::new("add_decision", ValueSpec::single(ValueType::Str), "添加决议（决议 id，如 `巴西南美扩张`；定义于 `rainfall/rfEvent_decision.json`）", FieldCategory::Effect),
	FieldSpec::new("start_decision", ValueSpec::single(ValueType::Str), "启动决议（决策id:事件id，如 `sov左翼社会革命党叛乱:左翼社会革命党企图叛乱`）", FieldCategory::Effect),
	FieldSpec::new("taking_decision", ValueSpec::single(ValueType::Str), "执行决议（决策id:事件id，如 `sov左翼社会革命党叛乱:左翼社会革命党企图叛乱`）", FieldCategory::Effect),
	FieldSpec::new("join_alliance_special_id_first_tier", ValueSpec::single(ValueType::Str), "加入特殊联盟第一层级（特殊联盟编号；或 `文明=编号`；编号见剧情 `AlliancesSpecial.json`）", FieldCategory::Effect),
	FieldSpec::new("join_alliance_special_id_second_tier", ValueSpec::single(ValueType::Int), "加入特殊联盟第二层级（编号；如暮色黄昏：0=同盟国 1=协约国）", FieldCategory::Effect),
	FieldSpec::new("leave_alliance_special_id", ValueSpec::single(ValueType::Int), "离开特殊联盟（编号；如暮色黄昏：1=协约国）", FieldCategory::Effect),
	FieldSpec::new("unlock_tech", ValueSpec::single(ValueType::Str), "解锁科技（科技编号；或 `文明=科技编号`）", FieldCategory::Effect),
	FieldSpec::new("military_academy", ValueSpec::single(ValueType::Int), "军事学院", FieldCategory::Effect),
	FieldSpec::new("set_counter", ValueSpec::single(ValueType::Str), "设置计数器（`计数器=名称` 或 `计数器=名称=$表达式`）", FieldCategory::Effect),
	FieldSpec::new("ae_set", ValueSpec::single(ValueType::Int), "设置侵略扩张值（如 -50、10、100）", FieldCategory::Effect),
	FieldSpec::new("se_set", ValueSpec::single(ValueType::Int), "设置超级事件值", FieldCategory::Effect),
	FieldSpec::new("supreme_court", ValueSpec::single(ValueType::Int), "最高法院", FieldCategory::Effect),
	FieldSpec::new("tooltip", ValueSpec::pair(ValueType::Bool, ValueType::Str), "提示信息（如 true=这将获得...）", FieldCategory::Effect),
	FieldSpec::new("ai", ValueSpec::single(ValueType::Int), "AI 选择概率", FieldCategory::Effect),
	FieldSpec::new("alliance", ValueSpec::single(ValueType::Str), "加入联盟", FieldCategory::Effect),
	FieldSpec::new("non_aggression_pact", ValueSpec::single(ValueType::Str), "互不侵犯条约", FieldCategory::Effect),
	FieldSpec::new("military_access", ValueSpec::single(ValueType::Str), "军事通行权", FieldCategory::Effect),
	FieldSpec::new("vassalize", ValueSpec::single(ValueType::Str), "使某文明成为附庸", FieldCategory::Effect),
	FieldSpec::new("remove_alliance", ValueSpec::single(ValueType::Int), "移除联盟", FieldCategory::Effect),
	FieldSpec::new("change_law", ValueSpec::pair(ValueType::Str, ValueType::Str), "改变法令（法令组编号=选项编号；详见 `laws/Laws.json` 数组顺序）", FieldCategory::Effect),
	FieldSpec::new("change_law2", ValueSpec::single(ValueType::Int), "改变法律2", FieldCategory::Effect),
	FieldSpec::new("change_religion", ValueSpec::single(ValueType::Int), "改变宗教", FieldCategory::Effect),
	FieldSpec::new("change_religion_civ", ValueSpec::pair(ValueType::Int, ValueType::Str), "改变某文明宗教（宗教编号=文明标签，如 `5=RUS2`）", FieldCategory::Effect),
	FieldSpec::new("switch_ability", ValueSpec::single(ValueType::Str), "切换能力", FieldCategory::Effect),
	FieldSpec::new("add_variable", ValueSpec::single(ValueType::Str), "添加变量", FieldCategory::Effect),
	FieldSpec::new("remove_variable", ValueSpec::single(ValueType::Str), "移除变量", FieldCategory::Effect),
	FieldSpec::new("remove_variable2", ValueSpec::pair(ValueType::Str, ValueType::Str), "移除某文明变量（文明=变量名，如 ger=var）", FieldCategory::Effect),
	FieldSpec::new("add_counter", ValueSpec::pair(ValueType::Str, ValueType::Int), "计数器加法", FieldCategory::Effect),
	FieldSpec::new("sub_counter", ValueSpec::pair(ValueType::Str, ValueType::Int), "计数器减法", FieldCategory::Effect),
	FieldSpec::new("mul_counter", ValueSpec::pair(ValueType::Str, ValueType::Int), "计数器乘法", FieldCategory::Effect),
	FieldSpec::new("div_counter", ValueSpec::pair(ValueType::Str, ValueType::Int), "计数器除法", FieldCategory::Effect),
	FieldSpec::new("promote_advisor", ValueSpec::single(ValueType::Int), "晋升顾问", FieldCategory::Effect),
	FieldSpec::new("kill_advisor", ValueSpec::single(ValueType::Int), "杀死顾问", FieldCategory::Effect),
	FieldSpec::new("kill_ruler", ValueSpec::single(ValueType::Int), "杀死统治者", FieldCategory::Effect),
	FieldSpec::new("kill_ruler_chance", ValueSpec::single(ValueType::Float), "概率杀死统治者（0-1）", FieldCategory::Effect),
	FieldSpec::new("remove_decision", ValueSpec::single(ValueType::Str), "移除决议", FieldCategory::Effect),
	FieldSpec::new("start_decision2", ValueSpec::single(ValueType::Str), "启动决议2（文明标签=决策id:事件id）", FieldCategory::Effect),
	FieldSpec::new("decision_desc", ValueSpec::single(ValueType::Str), "决议描述", FieldCategory::Effect),
	FieldSpec::new("decision_image", ValueSpec::single(ValueType::Str), "决议图片", FieldCategory::Effect),
	FieldSpec::new("province_devastation", ValueSpec::single(ValueType::Int), "省份破坏", FieldCategory::Effect),
	FieldSpec::new("province_devastation_all", ValueSpec::single(ValueType::Int), "全境破坏", FieldCategory::Effect),
	FieldSpec::new("province_devastation_capital", ValueSpec::single(ValueType::Int), "首都破坏", FieldCategory::Effect),
	FieldSpec::new("province_devastation_id", ValueSpec::pair(ValueType::Int, ValueType::Int), "指定省份破坏", FieldCategory::Effect),
	FieldSpec::new("province_manpower", ValueSpec::single(ValueType::Float), "省份人力", FieldCategory::Effect),
	FieldSpec::new("province_manpower_all", ValueSpec::single(ValueType::Float), "全境人力", FieldCategory::Effect),
	FieldSpec::new("province_manpower_capital", ValueSpec::single(ValueType::Float), "首都人力", FieldCategory::Effect),
	FieldSpec::new("province_population", ValueSpec::single(ValueType::Int), "省份人口", FieldCategory::Effect),
	FieldSpec::new("province_population_capital", ValueSpec::single(ValueType::Int), "首都人口", FieldCategory::Effect),
	FieldSpec::new("province_tax_efficiency", ValueSpec::single(ValueType::Float), "省份税收效率", FieldCategory::Effect),
	FieldSpec::new("province_tax_efficiency_all", ValueSpec::single(ValueType::Float), "全境税收效率", FieldCategory::Effect),
	FieldSpec::new("province_id_build_add", ValueSpec::pair(ValueType::Int, ValueType::Int), "指定省份添加建筑", FieldCategory::Effect),
	FieldSpec::new("province_id_build_remove", ValueSpec::pair(ValueType::Int, ValueType::Int), "指定省份移除建筑", FieldCategory::Effect),
	FieldSpec::new("province_id_pop_set", ValueSpec::pair(ValueType::Int, ValueType::Int), "设置省份人口", FieldCategory::Effect),
	FieldSpec::new("province_id_spread_disease", ValueSpec::pair(ValueType::Int, ValueType::Int), "省份传播疾病", FieldCategory::Effect),
	FieldSpec::new("ai_aggression", ValueSpec::single(ValueType::Float), "AI 侵略性", FieldCategory::Effect),
	FieldSpec::new("advantage_points", ValueSpec::single(ValueType::Int), "优势点数", FieldCategory::Effect),
	FieldSpec::new("capital_city_level", ValueSpec::single(ValueType::Int), "首都城市等级（如 1、-2；引擎键名，旧文档的 capital_building_level 无效）", FieldCategory::Effect),
	FieldSpec::new("explode", ValueSpec::single(ValueType::Str), "解体（文明标签，如 ger、hun）", FieldCategory::Effect),
	FieldSpec::new("inflation", ValueSpec::single(ValueType::Float), "通货膨胀", FieldCategory::Effect),
	FieldSpec::new("nuclear_reactor", ValueSpec::single(ValueType::Int), "核反应堆", FieldCategory::Effect),
	FieldSpec::new("play_music", ValueSpec::single(ValueType::Str), "播放音乐", FieldCategory::Effect),
	FieldSpec::new("run_script", ValueSpec::single(ValueType::Str), "运行脚本", FieldCategory::Effect),
	FieldSpec::new("skip_focus", ValueSpec::single(ValueType::Int), "跳过焦点", FieldCategory::Effect),
	FieldSpec::new("skip_goal", ValueSpec::single(ValueType::Str), "跳过目标（目标 ID；可 `目标=序号`）", FieldCategory::Effect),
	FieldSpec::new("white_peace2", ValueSpec::single(ValueType::Str), "白和平2（文明标签；也支持 文明A=文明B）", FieldCategory::Effect),
	FieldSpec::new("bonus_advisor_cost", ValueSpec::single(ValueType::Float), "顾问成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_advisors_max_level", ValueSpec::single(ValueType::Float), "顾问最大等级加成（如 2）", FieldCategory::Effect),
	FieldSpec::new("bonus_aggressive_expansion", ValueSpec::single(ValueType::Float), "侵略扩张加成", FieldCategory::Effect),
	FieldSpec::new("bonus_all_characters_life_expectancy", ValueSpec::single(ValueType::Float), "所有角色寿命加成", FieldCategory::Effect),
	FieldSpec::new("bonus_battle_width", ValueSpec::single(ValueType::Float), "战斗宽度加成", FieldCategory::Effect),
	FieldSpec::new("bonus_core_cost", ValueSpec::single(ValueType::Float), "核心成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_corruption", ValueSpec::single(ValueType::Float), "腐败加成", FieldCategory::Effect),
	FieldSpec::new("bonus_develop_infrastructure_cost", ValueSpec::single(ValueType::Float), "发展基建成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_diplomacy_points", ValueSpec::single(ValueType::Float), "外交点数加成", FieldCategory::Effect),
	FieldSpec::new("bonus_disease_death_rate", ValueSpec::single(ValueType::Float), "疾病死亡率加成", FieldCategory::Effect),
	FieldSpec::new("bonus_growth_rate", ValueSpec::single(ValueType::Float), "增长率加成", FieldCategory::Effect),
	FieldSpec::new("bonus_improve_relations_modifier", ValueSpec::single(ValueType::Float), "改善关系修正加成", FieldCategory::Effect),
	FieldSpec::new("bonus_income_economy", ValueSpec::single(ValueType::Float), "经济收入加成", FieldCategory::Effect),
	FieldSpec::new("bonus_income_from_vassals", ValueSpec::single(ValueType::Float), "附庸收入加成", FieldCategory::Effect),
	FieldSpec::new("bonus_income_taxation", ValueSpec::single(ValueType::Float), "税收收入加成", FieldCategory::Effect),
	FieldSpec::new("bonus_increase_growth_rate_cost", ValueSpec::single(ValueType::Float), "增加增长率成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_increase_manpower_cost", ValueSpec::single(ValueType::Float), "增加人力成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_increase_tax_efficiency_cost", ValueSpec::single(ValueType::Float), "增加税收效率成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_inflation", ValueSpec::single(ValueType::Float), "通货膨胀加成", FieldCategory::Effect),
	FieldSpec::new("bonus_invest_in_economy_cost", ValueSpec::single(ValueType::Float), "投资经济成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_loan_interest", ValueSpec::single(ValueType::Float), "贷款利息加成", FieldCategory::Effect),
	FieldSpec::new("bonus_maintenance_cost", ValueSpec::single(ValueType::Float), "维护成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_manpower_recovery_from_disbanded_army", ValueSpec::single(ValueType::Float), "解散军队人力返还加成（如 25）", FieldCategory::Effect),
	FieldSpec::new("bonus_max_morale", ValueSpec::single(ValueType::Float), "最大士气加成", FieldCategory::Effect),
	FieldSpec::new("bonus_maximum_amount_of_gold", ValueSpec::single(ValueType::Float), "最大金币上限加成", FieldCategory::Effect),
	FieldSpec::new("bonus_regiments_limit", ValueSpec::single(ValueType::Float), "军团上限加成", FieldCategory::Effect),
	FieldSpec::new("bonus_religion_cost", ValueSpec::single(ValueType::Float), "宗教成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_revolutionary_risk", ValueSpec::single(ValueType::Float), "革命风险加成", FieldCategory::Effect),
	FieldSpec::new("bonus_siege_effectiveness", ValueSpec::single(ValueType::Float), "围困效率加成", FieldCategory::Effect),
	FieldSpec::new("bonus_war_score_cost", ValueSpec::single(ValueType::Float), "战争分数成本加成", FieldCategory::Effect),
	// 实测模组常见键（GameCivs 五模组统计后注册）
	FieldSpec::new("add_military_access", ValueSpec::pair(ValueType::Str, ValueType::Str), "军事通行权（给予方=获得方）", FieldCategory::Effect),
	FieldSpec::new("add_alliance", ValueSpec::pair(ValueType::Str, ValueType::Str), "加入联盟（如 roc=xib）", FieldCategory::Effect),
	FieldSpec::new("add_non_aggression", ValueSpec::single(ValueType::Str), "互不侵犯条约", FieldCategory::Effect),
	FieldSpec::new("add_variable2", ValueSpec::pair(ValueType::Str, ValueType::Str), "为某文明添加变量（文明=变量名，如 ger=sudetenland_accepted）", FieldCategory::Effect),
	FieldSpec::new("annex_civ2", ValueSpec::single(ValueType::Str), "吞并某文明（第二版，可多次使用）", FieldCategory::Effect),
	FieldSpec::new("annexed_by_civ2", ValueSpec::single(ValueType::Str), "被某文明吞并（第二版）", FieldCategory::Effect),
	FieldSpec::new("annex_by_civ_from_civ2", ValueSpec::triple(ValueType::Str, ValueType::Str, ValueType::Str), "从某文明吞并给另一文明（第二版）", FieldCategory::Effect),
	FieldSpec::new("province_remove_core_civ", ValueSpec::lambda(ValueType::Int, ValueType::Str, ValueType::Str), "移除核心省份（`省份列表;=文明标签`）", FieldCategory::Effect),
	FieldSpec::new("province_growth_rate_id", ValueSpec::lambda(ValueType::Int, ValueType::Float, ValueType::Float), "指定省份增长率（`省份=数值` 或 `省份列表;=数值`）", FieldCategory::Effect),
	FieldSpec::new("province_unrest_capital", ValueSpec::single(ValueType::Float), "首都动荡变化", FieldCategory::Effect),
	FieldSpec::new("province_production_efficiency_all", ValueSpec::single(ValueType::Float), "全境生产效率", FieldCategory::Effect),
	FieldSpec::new("gold_monthly_income", ValueSpec::single(ValueType::Float), "每月收入加成", FieldCategory::Effect),
	FieldSpec::new("legacy_monthly", ValueSpec::single(ValueType::Float), "每月威望加成", FieldCategory::Effect),
	FieldSpec::new("bonus_monthly_research", ValueSpec::single(ValueType::Float), "每月科研加成", FieldCategory::Effect),
	FieldSpec::new("military_academy_generals", ValueSpec::single(ValueType::Int), "将领军事学院", FieldCategory::Effect),
	FieldSpec::new("relations_set", ValueSpec::triple(ValueType::Str, ValueType::Str, ValueType::Int), "设置关系值（`文明A=文明B=数值`，模组复数写法）", FieldCategory::Effect),
	FieldSpec::new("relations_change", ValueSpec::triple(ValueType::Str, ValueType::Str, ValueType::Int), "关系变化（`文明A=文明B=数值`，模组复数写法）", FieldCategory::Effect),
	FieldSpec::new("set_civ_tag_reset2", ValueSpec::single(ValueType::Str), "重置文明标签（第二版）", FieldCategory::Effect),
	FieldSpec::new("add_advisor2", ValueSpec::pair(ValueType::Str, ValueType::Str), "添加顾问（类型=人物名，如 `1=Franklin`；0行政 1经济 2创新 3军事）", FieldCategory::Effect),
	FieldSpec::new("add_general3", ValueSpec::triple(ValueType::Str, ValueType::Int, ValueType::Int), "添加将领（人物=攻击=防御，-1=随机，如 `Galileo=6=4`）", FieldCategory::Effect),
	// 实测模组常见键（第二批）
	FieldSpec::new("annex_from_civ", ValueSpec::pair(ValueType::Str, ValueType::Str), "从某文明吞并省份（文明=省份列表）", FieldCategory::Effect),
	FieldSpec::new("research", ValueSpec::single(ValueType::Int), "研究点数（旧写法，同 bonus_research_points）", FieldCategory::Effect),
	FieldSpec::new("Research", ValueSpec::single(ValueType::Int), "研究点数（大写写法）", FieldCategory::Effect),
	FieldSpec::new("Discipline", ValueSpec::single(ValueType::Float), "纪律加成（大写写法，同 bonus_discipline）", FieldCategory::Effect),
	FieldSpec::new("bonus_monthly_legacy_percentage", ValueSpec::single(ValueType::Float), "每月威望百分比加成", FieldCategory::Effect),
	FieldSpec::new("bonus_general_cost", ValueSpec::single(ValueType::Float), "将领成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_infrastructure_efficiency", ValueSpec::single(ValueType::Float), "基建效率加成", FieldCategory::Effect),
	FieldSpec::new("bonus_research_cost", ValueSpec::single(ValueType::Float), "科研成本加成", FieldCategory::Effect),
	FieldSpec::new("bonus_research_efficiency", ValueSpec::single(ValueType::Float), "科研效率加成", FieldCategory::Effect),
	FieldSpec::new("bonus_unlocked_legacies", ValueSpec::single(ValueType::Float), "已解锁遗产数加成", FieldCategory::Effect),
	FieldSpec::new("province_growth_rate_capital", ValueSpec::single(ValueType::Float), "首都增长率", FieldCategory::Effect),
	FieldSpec::new("province_infrastructure_capital", ValueSpec::single(ValueType::Int), "首都基础设施", FieldCategory::Effect),
	FieldSpec::new("province_tax_efficiency_capital", ValueSpec::single(ValueType::Float), "首都税收效率", FieldCategory::Effect),
	FieldSpec::new("province_tax_efficiency_id", ValueSpec::lambda(ValueType::Int, ValueType::Float, ValueType::Float), "指定省份税收效率（`省份=数值` 或 `省份列表;=数值`）", FieldCategory::Effect),
	FieldSpec::new("province_nuke", ValueSpec::lambda(ValueType::Int, ValueType::Str, ValueType::Str), "核打击省份（`省份列表;=文明标签`）", FieldCategory::Effect),
	FieldSpec::new("option_end", ValueSpec::single(ValueType::Str), "选项块结束行（出现在块外时原样保留）", FieldCategory::Effect),
	// 实测模组常见键（第三批：APK 全局事件 `assets/game/events` 统计）
	FieldSpec::new("price_change", ValueSpec::sequence(ValueType::Int, ValueType::Int), "资源价格变化（资源ID=幅度=随机幅度=月数=随机月数，如 0=20=40=18=24）", FieldCategory::Effect),
	FieldSpec::new("price_change_up", ValueSpec::sequence(ValueType::Int, ValueType::Int), "资源价格上涨（如 0=20=40=18=24）", FieldCategory::Effect),
	FieldSpec::new("price_change_down", ValueSpec::sequence(ValueType::Int, ValueType::Int), "资源价格下跌（如 0=20=40=18=24）", FieldCategory::Effect),
	FieldSpec::new("price_change_random", ValueSpec::sequence(ValueType::Int, ValueType::Int), "随机资源价格变化（幅度=随机幅度=月数=随机月数，如 15=16=12=36）", FieldCategory::Effect),
	FieldSpec::new("price_change_random_up", ValueSpec::sequence(ValueType::Int, ValueType::Int), "随机资源价格上涨（如 6=24=21=0）", FieldCategory::Effect),
	FieldSpec::new("price_change_random_down", ValueSpec::sequence(ValueType::Int, ValueType::Int), "随机资源价格下跌（如 40=25=12=4）", FieldCategory::Effect),
	FieldSpec::new("price_change_group", ValueSpec::sequence(ValueType::Int, ValueType::Int), "资源组价格变化（组ID=幅度…；0食物 1商品 2奢侈商品 3奢侈品 4生产资料）", FieldCategory::Effect),
	FieldSpec::new("price_change_group_up", ValueSpec::sequence(ValueType::Int, ValueType::Int), "资源组价格上涨（组ID=幅度…）", FieldCategory::Effect),
	FieldSpec::new("price_change_group_down", ValueSpec::sequence(ValueType::Int, ValueType::Int), "资源组价格下跌（组ID=幅度…）", FieldCategory::Effect),
	// AoH3 官方 FAQ（game/_FAQ/Events_Outcomes.txt）实测键——2026-10-09 批量入库。
	FieldSpec::new("province_infrastructure_id", ValueSpec::pair(ValueType::Int, ValueType::Int), "指定省份基建（省份=数值）", FieldCategory::Effect),
	FieldSpec::new("province_religion", ValueSpec::single(ValueType::Int), "某省份宗教改为（宗教编号）", FieldCategory::Effect),
	FieldSpec::new("province_religion_capital", ValueSpec::single(ValueType::Int), "首都省份宗教改为（宗教编号）", FieldCategory::Effect),
	FieldSpec::new("province_religion_id", ValueSpec::pair(ValueType::Int, ValueType::Int), "指定省份宗教改为（省份=宗教编号）", FieldCategory::Effect),
	FieldSpec::new("province_unrest", ValueSpec::single(ValueType::Float), "某省份动荡度（数值）", FieldCategory::Effect),
	FieldSpec::new("province_unrest_id", ValueSpec::pair(ValueType::Int, ValueType::Float), "指定省份动荡度（省份=数值）", FieldCategory::Effect),
	FieldSpec::new("province_economy_capital", ValueSpec::single(ValueType::Float), "首都省份经济（APK 全局事件实测）", FieldCategory::Effect),
];

/// 常见的拼写错误键 → 正确键名提示（编辑器诊断用）。
/// 条目来自 GameCivs 五模组真实事件文件的统计（部分行是被截断/漏敲的键名）。
pub const KNOWN_TYPOS: &[(&str, &str)] = &[
	("leagcy", "legacy"),
	("legcy", "legacy"),
	("lagacy", "legacy"),
	("laegacy", "legacy"),
	("laegcy", "legacy"),
	("dsec", "desc"),
	("mission_dsec", "mission_desc"),
	("missions_desc", "mission_desc"),
	("ser_civ_tag2", "set_civ_tag2"),
	("has_bariable_not", "has_variable_not"),
	("has_varia", "has_variable"),
	("existis_any_not", "exists_any_not"),
	("annex_by_civ_fro", "annex_by_civ_from_civ"),
	("annex_ci", "annex_civ"),
	("annexed_", "annexed_by_civ"),
	("annexed_by", "annexed_by_civ"),
	("annexed_by_c", "annexed_by_civ"),
	("change_ideolog", "change_ideology"),
	("civ_capital_unrest_ove", "civ_capital_unrest_over"),
	("whit", "white_peace"),
	("white_pe", "white_peace"),
	("white_pea", "white_peace"),
	("white_peac", "white_peace"),
	("add_ru", "add_ruler"),
	("add_rule", "add_ruler"),
	("add_defencive_pact", "add_defensive_pact"),
	("is_palyer", "is_player"),
	// AoH3 官方 FAQ（game/_FAQ/Events_*.txt）与手机版引擎（classes.dex 字符串表）实测对照：
	// 下列旧文档 / ScvGen 键名在引擎中不存在，右侧为引擎实际键名（2026-10-09）。
	("annex_provinces_from_civ", "annex_from_civ"),
	("annex_provinces", "annex"),
	("capital_building_level", "capital_city_level"),
	("military_academy_for_generals", "military_academy_generals"),
	("bonus_advisor_max_level", "bonus_advisors_max_level"),
	("bonus_manpower_recovery_from_a_disbanded_army", "bonus_manpower_recovery_from_disbanded_army"),
	("bonus_monthly_legacy_perc", "bonus_monthly_legacy_percentage"),
	("relation_change", "relations_change"),
	("relation_set", "relations_set"),
	("resource_price_change", "price_change"),
	("resource_price_change_up", "price_change_up"),
	("resource_price_change_down", "price_change_down"),
	("resource_price_change_random", "price_change_random"),
	("resource_price_change_group", "price_change_group"),
	("ae_ste", "ae_set"),
	("add_variable_civ", "add_variable2"),
	("remove_variable_civ", "remove_variable2"),
	// 第二批引擎对照（2026-10-09）：AoH3 FAQ 无此名 / 手机引擎字符串表无命中 → 均有实测存在的对应键。
	("add_advisor_character", "add_advisor2"),
	("add_general_character", "add_general2"),
	("add_general_character_attack_defense", "add_general3"),
	("province_id_nuke", "province_nuke"),
	("exists_not", "not_exists"),
	("civ_num_of_provinces_below", "civ_provinces_below"),
	("civ_num_of_provinces_over", "civ_provinces_over"),
	("civ_total_population_below", "civ_population_below"),
	("civ_total_population_over", "civ_population_over"),
	("province_core_of", "province_civ_has_core"),
	("province_id_core_add", "province_add_core_civ"),
	("province_id_core_remove", "province_remove_core_civ"),
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

/// 按键名查注册信息（忽略键首尾空白：解析器为逐字节保真会保留行首空格/`key = value` 形式的空格）。
pub fn lookup(key: &str) -> Option<&'static FieldSpec> {
	all_specs().find(|spec| spec.key == key.trim())
}

/// 类型语法诊断：判断 `value` 是否符合 `spec` 描述的语法。
/// 空值视为合法（允许留空）；先按原值校验，失败后再尝试去掉尾随 `=`
/// （`2=`、`a=b=`、`3=0=…=0=` 是模组脚本里的常见写法，相当于省略尾段）。
pub fn is_valid_value(spec: ValueSpec, value: &str) -> bool {
	let value = value.trim();
	if value.is_empty() {
		return true;
	}
	if spec_matches(spec, value) {
		return true;
	}
	let stripped = value.trim_end_matches('=');
	stripped != value && (stripped.is_empty() || spec_matches(spec, stripped))
}

/// 不带容错的语法匹配（[`is_valid_value`] 的主体）。
fn spec_matches(spec: ValueSpec, value: &str) -> bool {
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
			// 允许奇数段（模组里常以孤立的类型段收尾，如 `0=0=…=0`）。
			let parts: Vec<_> = value.split('=').collect();
			parts.len() >= 2
				&& parts.iter().enumerate().all(|(index, part)| {
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
			if !list_ok {
				return false;
			}
			// 右段既可以是 `a=b` 两段（如 `3994;=6=0`），
			// 也可以是单段（如 `4381;…;=ANF6_Anfu`、`…;=15.0`）。
			let mut parts = right.split('=');
			let head = parts.next().unwrap_or_default();
			match parts.next() {
				None => second.accepts(head),
				Some(tail) => {
					parts.next().is_none()
						&& second.accepts(head)
						&& third.accepts(tail)
				}
			}
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
		// 键首尾空白（行首缩进 / `key = value` 写法）按去空白匹配。
		assert!(lookup(" legacy").is_some());
		assert!(lookup("annex_by_civ_from_civ ").is_some());
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
		// 奇数段（末尾只剩类型）在模组里同样出现，视为合法。
		assert!(is_valid_value(
			ValueSpec::sequence(ValueType::Int, ValueType::Int),
			"1=100=2"
		));
		assert!(!is_valid_value(
			ValueSpec::sequence(ValueType::Int, ValueType::Int),
			"1=100=abc"
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

	/// 尾随 `=`（省略尾段）与 `列表;=单值` 的容错（真实模组数据驱动）。
	#[test]
	fn tolerates_trailing_equals_and_single_tail_lambda() {
		assert!(is_valid_value(ValueSpec::single(ValueType::Float), "2="));
		assert!(is_valid_value(ValueSpec::single(ValueType::Int), "100="));
		assert!(is_valid_value(
			ValueSpec::pair(ValueType::Str, ValueType::Str),
			"nejd=bedo2="
		));
		assert!(is_valid_value(
			ValueSpec::sequence(ValueType::Int, ValueType::Int),
			"3=0=5=0="
		));
		assert!(is_valid_value(
			ValueSpec::triple(ValueType::Int, ValueType::Int, ValueType::Int),
			"=="
		));
		// `省份列表;=文明标签` / `省份=数值`（右段单值）
		assert!(is_valid_value(
			ValueSpec::lambda(ValueType::Int, ValueType::Str, ValueType::Str),
			"4381;4877;=ANF6_Anfu"
		));
		assert!(is_valid_value(
			ValueSpec::lambda(ValueType::Int, ValueType::Float, ValueType::Float),
			"3475=100"
		));
		assert!(is_valid_value(
			ValueSpec::lambda(ValueType::Int, ValueType::Float, ValueType::Float),
			"5220;2952;=15.0"
		));
	}

	/// 注册表按真实模组数据逐条核对（值均取自 GameCivs 五个模组实测）。
	#[test]
	fn registered_specs_accept_real_mod_values() {
		let cases = [
			("province_is_occupied", "1123"),
			("province_is_occupied", "true"),
			("province_is_under_siege", "352"),
			("white_peace2", "GUO_kleft"),
			("annex", "404;409;"),
			("annex", "217"),
			("province_add_core_civ", "4381;4877;=ANF6_Anfu"),
			("change_ideology_civ", "0=LRF"),
			("change_ideology_civ", "="),
			("change_law", "ecun=un1"),
			("change_law", "8=1"),
			("skip_goal", "FNG1917_zhangshi=0"),
			("skip_goal", "rrus89"),
			("bonus_duration", "0.3"),
			("bonus_monthly_income", "0.5"),
			("bonus_province_maintenance", "-0.8"),
			("bonus_max_morale", "2.5"),
			("province_growth_rate", "2.0"),
			("province_manpower", "3.0"),
			("province_growth_rate_all", "1.5"),
			("set_counter", "neu=安福国会蒙藏院=$安福国会蒙藏院+10"),
			("set_counter", "neu=天下"),
			("unlock_tech", "JAP=95"),
			("unlock_tech", "132"),
			("change_religion_civ", "5=RUS2"),
			("province_manpower_id", "5220;2952;=15.0"),
			("province_economy_id", "2855=45"),
			("explode", "hun"),
			("add_new_army", "3=0=5=0=3=0="),
			("declare_war2", "nejd=bedo2="),
			("bonus_manpower_recovery_speed", "12.5"),
			("legacy", "100="),
			("legacy", "7.5"),
			("tooltip", "false="),
			("annex_by_civ_from_civ", "bah=ger="),
			("annex_by_civ_from_civ", "JEH_Wanxi=ANF6_Anfu="),
			("declare_war2", "spa_b="),
			("set_civ_tag2", "alb="),
			("has_variable_civ", "反清复明运动"),
			("add_new_army", "0=0=0=0=0=0=0=0=0=0=0=0=0=0=0"),
			("relations_set", "ANF=CHI=-15"),
			("relations_change", "ANF5_Anfu=ZHL2_Baoding=-25"),
			("if_counter", "$东北现代化点数>19"),
			("if_counter", "$亲日立场<4"),
			("civ_capital_has_building", "9=0"),
			("civ_capital_has_building", "6"),
			("province_nuke", "1;2;3;4;=neu"),
			("annex_from_civ", "CXL_chuan=1567;9497;4369;"),
			("civs_have_alliance", "ger=fra"),
			("civs_opinion_over", "ger=pol=-25.0"),
			("province_economy_below", "464=3.2"),
			("not_exists", "pol_m"),
			("civ_tag_religion_is", "bel=3"),
			("province_religion_is", "1322=0"),
			("civ_advisor_age_over", "2=37"),
			("province_civ_has_core", "3994=CFT_zhangshi"),
			("random_chance", "20.0"),
			("ui_type", "6"),
			("music_file", "［俄罗斯电台］四年战争"),
			// 省份列表型键（GameCivs 第二轮扫描样本）：单省对 / 分号列表 / 尾随 `;`
			// 可省略 / 右侧单值与两段值。
			("province_add_core_civ", "970;3041;3133;=CTT_Herogui"),
			("province_remove_core_civ", "2862;1108;7155;9463;6436;=WGX_Wanxi"),
			("province_economy_id", "4716;1027;9920;9516;1292;1514;=2.5"),
			("province_growth_rate_id", "8365;3318;7427;2161;=15.0"),
			("province_manpower_id", "3475=100"),
			("province_tax_efficiency_id", "2278=50"),
			("province_add_building", "6217;=4=1"),
			("province_add_building", "2373;6428;8389=2=0"),
			// 补全审计第二批样本（GameCivs missionsEvents + APK 全局 events 实测）
			("relations_set", "ANF7_Anfu=JAP=20"),
			("relations_change", "ming=mtang=90"),
			("civs_opinion_below", "ukr=rus=-50.0"),
			("largest_producer_production_over", "3=350"),
			("civ_has_resource", "34"),
			("civ_has_resource_over", "44=20"),
			("price_change_up", "3=15=30=36=24"),
			("price_change_down", "4=20=25=36=24"),
			("province_economy_capital", "1.8"),
			("is_puppet", "fin2"),
			("is_not_puppet", "fin2"),
			// 引擎逆向批次（2026-10-09）样本：法令 / 决议 / 统治者 / 大洲 / 联盟 / 顾问。
			("add_ruler", "陶尔斐斯= =陶尔斐斯=4=10=1892"),
			("add_ruler", "阿道夫希特勒= =年轻的阿道夫希特勒=1=1=1890"),
			("add_ruler_custom", "="),
			("add_decision", "巴西南美扩张"),
			("taking_decision", "sov左翼社会革命党叛乱:左翼社会革命党企图叛乱"),
			("start_decision2", "="),
			("civ_capital_continent_is", "2"),
			("add_advisor", "3"),
			("join_alliance_special_id_second_tier", "1"),
			("leave_alliance_special_id", "0"),
			("exact_day", "15=8=1917"),
			// 兵种 / 顾问 / 省份宗教（2026-10-09 第三批）。
			("add_new_army", "0=1=3=2"),
			("add_new_army", "1=6=1=6=1=6=1=6=1=6=1=6=1=6=1=6=0=0=0=0=0=0"),
			("add_advisor2", "1=CaoRulin"),
			("province_religion", "3"),
			("province_religion_capital", "2"),
		];
		for (key, value) in cases {
			let spec = lookup(key).unwrap_or_else(|| panic!("缺少键 {key}"));
			assert!(
				is_valid_value(spec.spec, value),
				"{key}={value} 应视为合法"
			);
		}
		// 省份列表型键的误写（缺 `=` / 非数字省份 token / 值段类型错）判为不合法。
		let economy = lookup("province_economy_id").unwrap().spec;
		assert!(!is_valid_value(economy, "4381;4877"));
		assert!(!is_valid_value(economy, "abc=1"));
		assert!(!is_valid_value(economy, "2855=abc"));
	}
}
