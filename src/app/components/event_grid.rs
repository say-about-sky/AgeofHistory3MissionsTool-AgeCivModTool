//! 国策事件脚本的"特化 Excel 表格"可视化编辑器。
//!
//! 每个脚本按来源拆成表格分区：
//! - 必填项目 / 可填项目 / 其他（未识别键，原样保留）；
//! - 触发条件块（可增删块，块内可增删条件行）；
//! - 收益选项块（可增删选项，选项内可增删效果行）。
//!
//! 表格列为：键 | 值 | 说明 | 删除。
//! 键输入提供 Schema 自动补全（候选标签即「说明」，选择时可直接看到各键含义），
//! 值按类型渲染（bool 为下拉框，其余文本），
//! 与 Schema 类型不符的单元格红框提示，保存时仍会原样写入。
//!
//! 候选 UI 双轨：桌面端用原生 `datalist` / `select`（`native_autocomplete = true`）；
//! 安卓端统一改用页面内自绘下拉（原生弹层定位不可靠），见 [`SuggestInput`] 的行为契约注释。
//! 文明ID / tag / civ 与政体整数字段：值按 `=` 分段渲染，各段挂候选，
//! 并在下方显示「值 → 名称」对照提示（数据来自 `event_lookup` 对照表）。
//! 省份列表段（`province_*_id` / `province_*_core_civ` 等）在段内再按 `;` 拆 token：
//! 文本原样显示（含尾随 `;`），候选按**最后一个 token** 过滤，选中后自动续写 `;`；
//! 这类段两平台都走自绘下拉（原生 datalist 只能按整段文本过滤，无法补全列表 token）。

use dioxus::prelude::*;

use super::event_lookup::{self, datalist_id, name_hint, value_parts, EventLookup, ValuePart};
use super::event_parser::{EntryLine, MissionEvent, NextOp, TriggerBlock, TriggerKind};
use super::event_schema::{self, FieldCategory, ValueSpec, ValueType};
use super::game_text::{has_game_codes, GameTextPreview};
use super::mind::Shared;

/// 值单元格的输入方式。
#[derive(Clone, Copy, PartialEq, Eq)]
enum CellInput {
	Text,
	Bool,
}

fn cell_input_for(key: &str) -> CellInput {
	match event_schema::lookup(key).map(|spec| spec.spec) {
		Some(ValueSpec::Single(ValueType::Bool)) => CellInput::Bool,
		_ => CellInput::Text,
	}
}

fn note_for(key: &str) -> String {
	if let Some(spec) = event_schema::lookup(key) {
		return spec.annotation.to_string();
	}
	let key = key.trim();
	if let Some((_, fix)) = event_schema::KNOWN_TYPOS.iter().find(|(typo, _)| *typo == key) {
		return format!("疑似拼写错误，应为 {fix}（保存时原样保留）");
	}
	"未识别键（保存时原样保留）".to_string()
}

fn is_invalid(key: &str, value: &str) -> bool {
	event_schema::lookup(key).is_some_and(|spec| !event_schema::is_valid_value(spec.spec, value))
}

/// 头部字段分组（按 Schema 分类）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum HeaderGroup {
	Required,
	Optional,
	Unknown,
}

fn header_group_of(key: &str) -> HeaderGroup {
	match event_schema::lookup(key).map(|spec| spec.category) {
		Some(FieldCategory::Required | FieldCategory::Header) => HeaderGroup::Required,
		Some(FieldCategory::Optional) => HeaderGroup::Optional,
		_ => HeaderGroup::Unknown,
	}
}

fn push_used_keys<'a>(entries: impl Iterator<Item = &'a EntryLine>, keys: &mut Vec<String>) {
	for entry in entries {
		if !keys.iter().any(|key| key == &entry.key) {
			keys.push(entry.key.clone());
		}
	}
}

/// 给候选键附加「说明」标签（与表格说明列同一来源；未知键为「未识别键」提示）。
fn with_notes(keys: Vec<String>) -> Vec<(String, String)> {
	keys.into_iter()
		.map(|key| {
			let note = note_for(&key);
			(key, note)
		})
	.collect()
}

/// 头部三组的自动补全候选（必填+可填 Schema 键，加当前使用的未知键），带说明标签。
fn header_suggestions(event: &MissionEvent) -> Vec<(String, String)> {
	let mut keys: Vec<String> = event_schema::all_specs()
		.filter(|spec| {
			matches!(
				spec.category,
				FieldCategory::Required | FieldCategory::Header | FieldCategory::Optional
			)
		})
		.map(|spec| spec.key.to_string())
		.collect();
	push_used_keys(event.header.iter(), &mut keys);
	with_notes(keys)
}

fn trigger_suggestions(event: &MissionEvent) -> Vec<(String, String)> {
	let mut keys: Vec<String> = event_schema::all_specs()
		.filter(|spec| spec.category == FieldCategory::Trigger)
		.map(|spec| spec.key.to_string())
		.collect();
	for block in &event.triggers {
		push_used_keys(block.conditions.iter(), &mut keys);
	}
	with_notes(keys)
}

fn effect_suggestions(event: &MissionEvent) -> Vec<(String, String)> {
	let mut keys: Vec<String> = event_schema::all_specs()
		.filter(|spec| spec.category == FieldCategory::Effect)
		.map(|spec| spec.key.to_string())
		.collect();
	for block in &event.options {
		push_used_keys(block.effects.iter(), &mut keys);
	}
	with_notes(keys)
}

/// 值单元格中的一个分段输入框（文明/政体/省份等字段按 `=` 拆分后各占一段，
/// 分别挂对应的候选列表提供自动补全）。
/// 桌面端普通段用原生 `datalist`（`kind` 决定候选表）；安卓端原生弹层定位不可靠
/// （页面滚动 / 软键盘弹出后上/下都会错位，旧 WebView 还不支持 datalist），
/// 改用页面内自绘下拉（见 [`SuggestInput`]）。
///
/// **省份列表段**（`Province` / `ProvinceSemi`）做 token 化补全：段文本原样显示
/// （含尾随 `;`——隐藏它会让「刚输入 `;` 时仍按旧 token 过滤」，2026-10 实测修复），
/// 候选按段内**最后一个 token** 过滤（如 `4716;1027;2` 时按 `2` 过滤，可连续选省），
/// 选中候选用 [`event_lookup::select_last_token`] 替换/追加 token 并续写 `;`
/// （`ProvinceSemi` 恒以 `;` 结尾）；这类段两种平台都走自绘下拉
/// （原生 datalist 只能按整段文本过滤，无法补全列表中间/末尾的单个省份）。
#[component]
fn ValuePartInput(
	text: String,
	kind: ValuePart,
	invalid: bool,
	segment_index: usize,
	full_value: String,
	native_autocomplete: bool,
	lookup: Shared<EventLookup>,
	/// 图片资源候选（`image` / `mission_image` 等），安卓自绘模式使用。
	images: Shared<Vec<String>>,
	/// 事件候选（`run_event` / `run_event_instantly` 值 = 文件名去 `.txt`）。
	events: Shared<Vec<String>>,
	/// 音乐候选（`musicName` / `play_music` 值）。
	music: Shared<Vec<String>>,
	/// 统治者头像候选（`add_ruler` 第三段；文件名去 `.png`，含数字编号与文本名）。
	ruler_images: Shared<Vec<String>>,
	/// 特殊联盟名称（剧情 `AlliancesSpecial.json` 顺序即编号；`join/leave_alliance_special_id_*`）。
	alliance_specials: Shared<Vec<String>>,
	on_value: EventHandler<String>,
) -> Element {
	let tokenized = matches!(kind, ValuePart::Province | ValuePart::ProvinceSemi);
	// 行上下文候选：法令组内选项取决于首段组号；特殊联盟由资源侧提供名称列表。
	let law_status_options: Vec<(String, String)> = if kind == ValuePart::LawStatus {
		law_status_items(&lookup, &full_value)
	} else {
		Vec::new()
	};
	// `add_new_army` 型号段：候选项取决于首段兵种 ID。
	let army_level_options: Vec<(String, String)> = if kind == ValuePart::ArmyLevel {
		army_level_items(&lookup, &full_value)
	} else {
		Vec::new()
	};
	let alliance_options: Vec<(String, String)> = if kind == ValuePart::AllianceSpecial {
		alliance_specials
			.iter()
			.enumerate()
			.map(|(index, name)| (index.to_string(), name.clone()))
			.collect()
	} else {
		Vec::new()
	};
	let on_text = EventHandler::new(move |new_text: String| {
		on_value.call(event_lookup::replace_value_part(
			&full_value,
			segment_index,
			&new_text,
		));
	});
	// 选中候选（自绘下拉）：token 化段做「替换/追加最后一个 token」并续写 `;`，
	// 其余段直接回传候选值（旧行为）。打字（oninput）始终整段回写、不做任何加工。
	let on_select = if tokenized {
		let select_text = text.clone();
		Some(EventHandler::new(move |selected: String| {
			let mut new_text = event_lookup::select_last_token(&select_text, &selected);
			// `;` 后缀段（province_add_core_civ / province_add_building 等）保持 `;=`
			// 写法：单省选中也补尾随 `;`（多省列表由 select_last_token 自动续写）。
			if kind == ValuePart::ProvinceSemi && !new_text.ends_with(';') {
				new_text.push(';');
			}
			on_text.call(new_text);
		}))
	} else {
		None
	};
	let filter_query = if tokenized {
		// 取最后一个 token 作为过滤文本（容忍用户手输的分隔空格，如 `a; 29`）。
		Some(event_lookup::last_token(&text).trim().to_string())
	} else {
		None
	};
	let list_id = datalist_id(kind);
	rsx! {
        if native_autocomplete && !tokenized {
            input {
                class: if invalid { "event-cell-input invalid" } else { "event-cell-input" },
                value: "{text}",
                list: "{list_id}",
                spellcheck: "false",
                oninput: move |evt: FormEvent| on_text.call(evt.value()),
            }
        } else {
            SuggestInput {
                text,
                // 资源类字段用文件名 / 名称候选；其余字段用对照表候选。
                options: match kind {
                    ValuePart::Image => SuggestOptions::Names(images),
                    ValuePart::EventId => SuggestOptions::Names(events),
                    ValuePart::Music => SuggestOptions::Names(music),
                    // 统治者头像：文件名去扩展名（数字编号与文本名两种写法均可）。
                    ValuePart::RulerImage => SuggestOptions::Names(ruler_images),
                    // 特殊联盟：值 = 编号，说明 = 名称。
                    ValuePart::AllianceSpecial => SuggestOptions::Items(alliance_options),
                    // 法令组内选项：候选项取决于首段组号（实时构造）。
                    ValuePart::LawStatus => SuggestOptions::Items(law_status_options),
                    // 兵种型号：候选项取决于首段兵种 ID（实时构造）。
                    ValuePart::ArmyLevel => SuggestOptions::Items(army_level_options),
                    // 顾问类型：固定枚举（0 行政 / 1 经济 / 2 创新 / 3 军事）。
                    ValuePart::AdvisorType => {
                        SuggestOptions::Fixed(event_lookup::ADVISOR_TYPE_OPTIONS)
                    }
                    _ => SuggestOptions::Lookup(lookup, kind),
                },
                filter_query,
                on_select,
                invalid,
                on_value: on_text,
                on_focus: move |_| {},
                on_blur: move |_| {},
            }
        }
    }
}

/// `add_ruler` 六段复合值专用编辑器（`名字=姓氏=头像=日=月=年`）。
///
/// 裸值里的五个 `=` 不利于直接编辑：这里渲染为带标签的字段
/// （名字 / 姓氏 / 头像 / 出生[日·月·年]），界面上不出现分隔符；
/// 任一字段改动都经 [`event_lookup::replace_ruler_segment`] 拼回整值
/// （保持段序、裁去尾部空段）。头像支持候选（数字编号或图片名两种写法）。
#[component]
fn RulerValueInput(
	value: String,
	invalid: bool,
	native_autocomplete: bool,
	ruler_images: Shared<Vec<String>>,
	on_value: EventHandler<String>,
) -> Element {
	let parts: Vec<String> = {
		let mut parts: Vec<String> = value.split('=').map(str::to_string).collect();
		while parts.len() < 6 {
			parts.push(String::new());
		}
		parts.truncate(6);
		parts
	};
	let name = parts[0].clone();
	let surname = parts[1].clone();
	let image = parts[2].clone();
	let day = parts[3].clone();
	let month = parts[4].clone();
	let year = parts[5].clone();
	// 每个字段一个回写处理器：均以「当前整值」为基准替换该段（避免各闭包互相覆盖）。
	let setter = move |index: usize| {
		let base = value.clone();
		EventHandler::new(move |text: String| {
			on_value.call(event_lookup::replace_ruler_segment(&base, index, &text));
		})
	};
	let set_name = setter(0);
	let set_surname = setter(1);
	let set_image = setter(2);
	let set_day = setter(3);
	let set_month = setter(4);
	let set_year = setter(5);
	rsx! {
        div { class: if invalid { "event-ruler-input invalid" } else { "event-ruler-input" },
            span { class: "event-ruler-field",
                input {
                    class: "event-cell-input",
                    value: "{name}",
                    placeholder: "名字",
                    title: "名字",
                    spellcheck: "false",
                    oninput: move |evt: FormEvent| set_name.call(evt.value()),
                }
            }
            span { class: "event-ruler-field",
                input {
                    class: "event-cell-input",
                    value: "{surname}",
                    placeholder: "姓氏",
                    title: "姓氏（可留空写成 `= `）",
                    spellcheck: "false",
                    oninput: move |evt: FormEvent| set_surname.call(evt.value()),
                }
            }
            span { class: "event-ruler-field",
                if native_autocomplete {
                    input {
                        class: "event-cell-input",
                        list: "{event_lookup::RULER_IMAGE_DATALIST_ID}",
                        value: "{image}",
                        placeholder: "头像",
                        title: "头像（数字编号或图片名）",
                        spellcheck: "false",
                        oninput: move |evt: FormEvent| set_image.call(evt.value()),
                    }
                } else {
                    SuggestInput {
                        text: image.clone(),
                        options: SuggestOptions::Names(ruler_images),
                        invalid: false,
                        placeholder: Some("头像".to_string()),
                        on_value: set_image,
                        on_focus: move |_| {},
                        on_blur: move |_| {},
                    }
                }
            }
            span { class: "event-ruler-field event-ruler-field-birth",
                input {
                    class: "event-ruler-num",
                    value: "{day}",
                    placeholder: "日",
                    title: "出生日（数字）",
                    inputmode: "numeric",
                    spellcheck: "false",
                    oninput: move |evt: FormEvent| set_day.call(evt.value()),
                }
                input {
                    class: "event-ruler-num",
                    value: "{month}",
                    placeholder: "月",
                    title: "出生月（数字）",
                    inputmode: "numeric",
                    spellcheck: "false",
                    oninput: move |evt: FormEvent| set_month.call(evt.value()),
                }
                input {
                    class: "event-ruler-num",
                    value: "{year}",
                    placeholder: "年",
                    title: "出生年（数字）",
                    inputmode: "numeric",
                    spellcheck: "false",
                    oninput: move |evt: FormEvent| set_year.call(evt.value()),
                }
            }
        }
    }
}

/// `add_new_army` 成对值（`兵种=型号` 重复）专用编辑器：每对一行，可逐行删除 / 追加。
///
/// 编辑状态就是值的段序列（[`event_lookup::army_segments`]）：每行两个输入框
/// （兵种候选 + 随兵种联动的型号候选），行尾 `×` 删除该对（末尾孤立段按一段删除），
/// 底部「＋ 添加一对」追加；写回只动被编辑 / 删除 / 追加的段，尾随空段原样保留。
#[component]
fn ArmyValueInput(
	value: String,
	invalid: bool,
	native_autocomplete: bool,
	lookup: Shared<EventLookup>,
	on_value: EventHandler<String>,
) -> Element {
	let segments = event_lookup::army_segments(&value);
	let rows: Vec<(String, String)> = (0..segments.len().div_ceil(2))
		.map(|index| {
			(
				segments.get(index * 2).cloned().unwrap_or_default(),
				segments.get(index * 2 + 1).cloned().unwrap_or_default(),
			)
		})
		.collect();
	// 每个字段一个回写处理器：均以「当前整值」为基准替换/删除该段（避免各闭包互相覆盖）。
	let value_for_setter = value.clone();
	let setter = move |index: usize| {
		let base = value_for_setter.clone();
		EventHandler::new(move |text: String| {
			on_value.call(event_lookup::set_army_segment(&base, index, &text));
		})
	};
	let value_for_remove = value.clone();
	let remover = move |row: usize| {
		let base = value_for_remove.clone();
		EventHandler::new(move |_: ()| {
			on_value.call(event_lookup::remove_army_pair(&base, row));
		})
	};
	let value_for_add = value.clone();
	let rows_render: Vec<(
		String,
		String,
		EventHandler<String>,
		EventHandler<String>,
		EventHandler<()>,
	)> = rows
		.into_iter()
		.enumerate()
		.map(|(index, (unit, level))| {
			(unit, level, setter(index * 2), setter(index * 2 + 1), remover(index))
		})
		.collect();
	rsx! {
        div { class: if invalid { "event-army-input invalid" } else { "event-army-input" },
            for (unit , level , on_unit , on_level , on_remove) in rows_render {
                div { class: "event-army-row",
                    if native_autocomplete {
                        input {
                            class: "event-cell-input",
                            list: "{event_lookup::ARMY_DATALIST_ID}",
                            value: "{unit}",
                            placeholder: "兵种",
                            title: "兵种（units/Units.json 的 ID）",
                            spellcheck: "false",
                            oninput: move |evt: FormEvent| on_unit.call(evt.value()),
                        }
                    } else {
                        SuggestInput {
                            text: unit.clone(),
                            options: SuggestOptions::Lookup(lookup.clone(), ValuePart::Army),
                            invalid: false,
                            placeholder: Some("兵种".to_string()),
                            on_value: on_unit,
                            on_focus: move |_| {},
                            on_blur: move |_| {},
                        }
                    }
                    if native_autocomplete {
                        input {
                            class: "event-cell-input",
                            value: "{level}",
                            placeholder: "型号",
                            title: "型号（该兵种文件内 Army 数组下标）",
                            inputmode: "numeric",
                            spellcheck: "false",
                            oninput: move |evt: FormEvent| on_level.call(evt.value()),
                        }
                    } else {
                        SuggestInput {
                            text: level.clone(),
                            options: SuggestOptions::Items(army_level_options(&lookup, &unit)),
                            invalid: false,
                            placeholder: Some("型号".to_string()),
                            on_value: on_level,
                            on_focus: move |_| {},
                            on_blur: move |_| {},
                        }
                    }
                    button {
                        class: "event-del event-army-del",
                        r#type: "button",
                        title: "删除这一对（兵种+型号）",
                        aria_label: "删除这一对",
                        onclick: move |_| on_remove.call(()),
                        "×"
                    }
                }
            }
            button {
                class: "event-add-row event-army-add",
                r#type: "button",
                title: "追加一对（兵种=型号）",
                aria_label: "添加一对",
                onclick: move |_| on_value.call(event_lookup::append_army_pair(&value_for_add)),
                "＋ 添加一对"
            }
        }
    }
}

// ===== 自绘候选下拉（安卓端替代原生列表弹层） =====
//
// 为什么：安卓 WebView 的原生列表弹层（`<datalist>` 候选、`<select>` 选项）是浏览器 /
// 系统原生窗口，按屏幕坐标 + 锚点定位，页面滚动或软键盘弹出（WebView 尺寸变化）后
// 上/下都会错位；旧版 WebView 还不支持 `<datalist>`。自绘下拉是页面内 DOM，
// 位置始终跟随输入框，且触屏交互可控。
//
// 【新增一处自绘下拉】
//   1. 选候选来源：静态枚举用 [`SuggestOptions::Fixed`]；表格键候选用 [`SuggestOptions::Keys`]；
//      对照表（文明/省份等）候选用 [`SuggestOptions::Lookup`]。需要新来源时在
//      `SuggestOptions` 加变体，并在 `SuggestInput` 内 `suggestions` 计算处加分支
//      （输出「值, 说明」二元组；过滤复用 `event_lookup::filter_suggestions` /
//      `matching_lookup`，空值项会以「仅标签」形式展示，用于「（留空）」这类选项）。
//   2. 渲染 `SuggestInput { text, options, invalid, on_value, on_focus, on_blur }`：
//      `text` 为受控显示文本；`on_value` 回传所输入/选的文本，由调用方写回模型；
//      `on_focus` / `on_blur` 供调用方额外记账（如键行的冻结分组），无需求传 `move |_| {}`。
//      可选：`filter_query`（候选过滤文本，供省份列表段传「最后一个 token」）、
//      `on_select`（仅承载下拉选中；缺省时选中回落到 `on_value`）。
//   3. 桌面端保留原生控件时用 `native_autocomplete` 开关分支（见 `GridRow` 与触发块控件；
//      省份列表段例外：两平台均走自绘，因原生 datalist 只能按整段文本过滤）。
//
// 【新增一种资源候选】以图片 / 事件 / 音乐为例：后端在 `list_event_assets`（`EventAssets`，
//   工作区目录 + 源 APK 条目，见 `missions_db.rs`）里增加一类收集函数与字段；前端在
//   `event_lookup.rs` 加 `ValuePart` 变体；在 `event.rs` 加载后经 `images` / `events` /
//   `music`（现另有 `ruler_images` / `alliance_specials`）属性传到 `ValuePartInput`。
//   候选为纯名称用 `SuggestOptions::Names`；「编号↔名称」配对（特殊联盟 / 法令组内选项）
//   用 `SuggestOptions::Items(Vec<(值, 说明)>)`（行上下文候选可实时构造，见 `LawStatus` 分支）。
//
// 【层级与裁剪】`.suggest-wrap`（展开时 z26）内含：遮罩 z24 / 输入框 z25 / 列表 z30；
//   分区的 `overflow` 裁剪切由 `.event-grid.suggest-overlay` 解除；展开方向与限高由
//   实测空间控制（`.suggest-list-up` 向上展开）。
//
// 【交互契约】修改前先读 [`SuggestInput`] 的文档注释，避免破坏触屏行为。

/// 自绘下拉的候选来源。
#[derive(Clone, PartialEq)]
enum SuggestOptions {
	/// 固定候选（静态枚举，如 true/false、触发块类型、连接方式）。
	Fixed(&'static [(&'static str, &'static str)]),
	/// 键候选（键, 说明）——由表格分区提供。
	Keys(Shared<Vec<(String, String)>>),
	/// 资源名称候选（图片 `.png` / 事件名 / 音乐名，无说明标签）——由事件面板按资源根加载。
	Names(Shared<Vec<String>>),
	/// 「值, 说明」对候选（按行上下文构造，如法令组内选项 / 特殊联盟编号）。
	Items(Vec<(String, String)>),
	/// 对照表候选——按字段类型实时过滤（值, 名称）。
	Lookup(Shared<EventLookup>, ValuePart),
}

/// `change_law` 次段候选：按首段组号从对照表取该组选项（值 = 选项号，说明 = 选项名）。
fn law_status_items(lookup: &EventLookup, full_value: &str) -> Vec<(String, String)> {
	let group = full_value
		.split('=')
		.next()
		.and_then(|head| head.trim().parse::<usize>().ok())
		.and_then(|index| lookup.laws.get(index));
	match group {
		Some(law) => law
			.options
			.iter()
			.enumerate()
			.map(|(index, name)| (index.to_string(), name.clone()))
			.collect(),
		None => Vec::new(),
	}
}

/// `add_new_army` 型号段候选：按首段兵种 ID 取该兵种的型号表（`ValuePartInput` 用）。
fn army_level_items(lookup: &EventLookup, full_value: &str) -> Vec<(String, String)> {
	let unit = full_value.split('=').next().unwrap_or_default();
	army_level_options(lookup, unit)
}

/// 兵种型号候选（值 = 型号号，说明 = 型号名）；`unit` 为兵种 ID 文本。
fn army_level_options(lookup: &EventLookup, unit: &str) -> Vec<(String, String)> {
	let unit = unit
		.trim()
		.parse::<u32>()
		.ok()
		.and_then(|id| lookup.units.iter().find(|item| item.id == id));
	match unit {
		Some(unit) => unit
			.armies
			.iter()
			.enumerate()
			.map(|(index, name)| (index.to_string(), name.clone()))
			.collect(),
		None => Vec::new(),
	}
}

/// 布尔值候选（空值项的「（留空）」选项以仅标签形式展示，选择后清空输入框）。
const BOOL_OPTIONS: &[(&str, &str)] = &[("", "（留空）"), ("true", ""), ("false", "")];

/// 触发块类型候选（自绘模式显示短名；写回文件时仍用 `trigger_*` 全名）。
const TRIGGER_KIND_OPTIONS: &[(&str, &str)] = &[
	("and", "与条件块"),
	("or", "或条件块"),
	("and_not", "与非条件块"),
];

/// 块首连接操作符候选（空值项 = 不写连接行）。
const TRIGGER_JOIN_OPTIONS: &[(&str, &str)] = &[
	("next_and", "与（默认）"),
	("next_or", "或"),
	("next_and_not", "与非"),
	("", "（不写）"),
];

/// 触发块类型短名（自绘下拉的显示文本；与旧原生 select 的显示标签一致）。
fn trigger_kind_short(kind: TriggerKind) -> &'static str {
	match kind {
		TriggerKind::And => "and",
		TriggerKind::Or => "or",
		TriggerKind::AndNot => "and_not",
	}
}

/// 短名或全名 → 触发块类型（未知按 `And` 兜底，与 `TriggerKind::from_open_token` 一致；
/// 比较不区分大小写，容手输）。
fn trigger_kind_from_text(text: &str) -> TriggerKind {
	match text.trim().to_ascii_lowercase().as_str() {
		"trigger_or" | "or" => TriggerKind::Or,
		"trigger_and_not" | "and_not" => TriggerKind::AndNot,
		_ => TriggerKind::And,
	}
}

/// 测量展开方向的空间脚本（在 dioxus 包装的 async 函数内执行，支持顶层 await）：
/// 先即时测一次；`__DELAY__` 毫秒后复测（等软键盘弹出引起的视口变化稳定）。
/// 返回 `[下方可用高度, 上方可用高度]`（相对 `.event-grid` 滚动视口；测量失败返回充足值）。
const DROP_SPACE_SCRIPT: &str = r#"
const el = document.activeElement;
if (!el || el.tagName !== 'INPUT') return [9999, 0];
await new Promise((resolve) => setTimeout(resolve, __DELAY__));
if (document.activeElement !== el) return [9999, 0];
const rect = el.getBoundingClientRect();
const pane = el.closest('.event-grid') || document.documentElement;
const bounds = pane.getBoundingClientRect();
return [bounds.bottom - rect.bottom, rect.top - bounds.top];
"#;

/// 返回（下方、上方）可用高度；脚本执行失败返回 `None`（保持当前方向）。
async fn measure_drop_space(delay_ms: u32) -> Option<(f64, f64)> {
    let script = DROP_SPACE_SCRIPT.replace("__DELAY__", &delay_ms.to_string());
    dioxus::document::eval(&script).join::<(f64, f64)>().await.ok()
}

/// 点选判定阈值（px）：松手位移超过即视为拖动 / 滚动，不提交选中。
const TAP_SLOP: f64 = 12.0;

/// 自绘下拉候选行固定高度（与 styles.css 的 `.suggest-item` 一致；虚拟滚动按行高换算窗口）。
const SUGGEST_ROW_HEIGHT: f64 = 34.0;
/// 虚拟滚动：可视区上下各保留的缓冲行数（限高最大 208px ≈ 7 行；节流滞后 / 快速滑动不露白）。
const SUGGEST_BUFFER_ROWS: usize = 24;
/// 自绘下拉候选上限（`usize::MAX` = 不截断）。此前固定传 60，导致省份（1.3 万+）/
/// 文明（4 千+）等大候选表最多只能看到前 60 条；全量结果交由虚拟滚动列表承载。
const SUGGEST_MATCH_LIMIT: usize = usize::MAX;

/// 虚拟滚动窗口计算：返回（窗口起始行、窗口结束行、顶占位高度、底占位高度）。
/// 行高固定 [`SUGGEST_ROW_HEIGHT`]，窗口在可视区外上/下各留 [`SUGGEST_BUFFER_ROWS`] 行缓冲。
fn suggest_window(
	scroll_top: f64,
	total: usize,
	viewport_rows: usize,
) -> (usize, usize, f64, f64) {
	if total == 0 {
		return (0, 0, 0.0, 0.0);
	}
	let max_start = total - 1;
	let start_offset = ((scroll_top / SUGGEST_ROW_HEIGHT).floor().max(0.0) as usize).min(max_start);
	let window_start = start_offset.saturating_sub(SUGGEST_BUFFER_ROWS);
	let window_end = (window_start + viewport_rows + SUGGEST_BUFFER_ROWS * 2).min(total);
	(
		window_start,
		window_end,
		window_start as f64 * SUGGEST_ROW_HEIGHT,
		(total - window_end) as f64 * SUGGEST_ROW_HEIGHT,
	)
}

/// 自绘候选下拉输入框（安卓端替代原生 `<datalist>` / `<select>` 弹层）。
///
/// 安卓 WebView 的原生弹层是浏览器原生窗口，按屏幕坐标 + 锚点定位，页面滚动 /
/// 软键盘弹出（WebView 尺寸变化）后会按错误偏移量放置（向上/向下都会偏）；
/// 改为页面内绝对定位的下拉后，位置始终跟随输入框。
///
/// # 行为契约（修改前必读，均为触屏实测后的选择）
///
/// 1. **松手提交**：`pointerdown` 只记录起点；鼠标 / 触控笔在 `pointerup` 位移 ≤ `TAP_SLOP`
///    时选中；**触摸在 `pointerup` 只记录待选值、由 `touchend` 提交**（见 4——在 `pointerup`
///    提交会先卸载列表，合成事件将穿透到下层控件）。
///    拖动超阈值或 `pointercancel`（浏览器判定为滚动）不选中，列表保持打开可继续滚。
///    触摸的 `pointerdown` 不能 `preventDefault`（否则滚动失效）；鼠标 / 触控笔则相反，
///    要在按下时 `preventDefault`（它们按下瞬间就夺焦，键行会中途重新分组致下拉卸载）。
/// 2. **失焦不关闭**：滚动列表 / 点按候选都会让输入框失焦；关闭只由遮罩点击、选中、Esc 触发。
///    若改回「失焦即关」，触摸滚动点选将丢失（松手前列表已被卸载）。
/// 3. **遮罩模式**：展开时铺全屏 `.suggest-backdrop`；关闭走 `onclick`（鼠标 / 触控笔）
///    与 `ontouchend`（触摸，**先 `preventDefault` 再关闭**——否则合成 click 会穿透到
///    遮罩下的控件；不能放 `pointerdown`：遮罩先卸载，touchend 无处抑制）。
///    展开的 wrap 提升层级（`.suggest-wrap-open`），保证输入框（z25）与列表（z30）在遮罩（z24）之上。
///    因此列表是「模态候选」：先点空白关闭、再点其他控件（移动端点选器惯例）。
/// 4. **触摸点选「touchend 提交」**：触摸的 `pointerup` 只把待选值存入 `pending_select`，
///    真正提交与关闭在 `touchend` 完成——同一句柄内先 `preventDefault()` 抑制本次触摸的
///    全部兼容鼠标 / `click` 合成事件，再提交（item 此刻仍在挂载，事件原子）。
///    此前在 `pointerup` 提交的写法有竞态：提交即卸载列表，`touchend` / 合成 `click`
///    落到弹窗遮挡的下层控件（误触下层选项 / 行尾删除按钮）。滚动手势走 pointercancel 不受影响。
/// 5. **展开方向自适应**：聚焦时 [`measure_drop_space`] 即刻实测 + 320ms 复测（等软键盘 /
///    视口稳定），`below < 214 && above > below + 24` 时向上展开（`.suggest-list-up`），
///    并按所向空间内联 `max-height`；`measure_generation` 纪元用于丢弃过期结果。
/// 6. **全量候选 + memo 缓存**：候选 = 全部匹配项（**不再截断为 60 条**；省份 1.3 万+ /
///    文明 4 千+ 都完整进入列表滚动浏览）。过滤结果用 `use_memo` 缓存，只在
///    （展开状态 / 查询文本 / 候选来源）变化时重算；滚动只重算虚拟滚动窗口，不重跑过滤。
/// 7. **选中与输入分离**：下拉候选项的选中走 `on_select`（可选；省份列表段用它做
///    「替换/追加最后一个 token + 续写 `;`」），打字仍走 `on_value`（整段回写）；
///    `filter_query`（可选）覆写候选过滤文本，默认用整段 `text`。
/// 8. **关闭后可重开**：选中候选 / 遮罩点击会关闭下拉，但输入框可能仍是焦点
///    （不会再有 focus 事件）；点按输入框（触摸走 `onclick`、鼠标 / 触控笔走
///    `onpointerdown`）或继续打字（`oninput`）会重新展开，避免「必须先点到别处、
///    再点回来才能继续补全」。**触摸的滑动安全**：展开不能放在触摸的 `pointerdown`
///    ——手指以输入框起势滑动页面时 `pointerdown` 先行触发，会造成「滑动中误弹补全」
///    （2026-10 修复）；滑动 / 拖动不产生 `click`，故触摸改由 `onclick` 兜底。
/// 9. **虚拟滚动**：列表只渲染可视窗口 ± [`SUGGEST_BUFFER_ROWS`] 行，上下用占位块撑起
///    滚动条；滚动信号按 2 行节流，`.suggest-list` 需保持 `overflow-anchor: none`、
///    行内文本单行省略。行高固定（[`SUGGEST_ROW_HEIGHT`]，与 CSS 同步）——**不要改回
///    多行换行**（虚拟滚动要求固定行高）。过滤条件变化时列表容器换 `key` 重挂载
///    （DOM 滚动归零）并在同一渲染内复位滚动信号，二者必须同步，否则窗口按旧偏移
///    计算、视口一片空白。
#[component]
fn SuggestInput(
	text: String,
	options: SuggestOptions,
	invalid: bool,
	on_value: EventHandler<String>,
	on_focus: EventHandler<()>,
	on_blur: EventHandler<()>,
	/// 候选过滤使用的查询文本（缺省=整段 `text`）。省份列表段传入「最后一个 token」，
	/// 使 `4716;1027;29` 能继续弹出 29xx 开头的省份候选。
	filter_query: Option<String>,
	/// 下拉选中回调（缺省=选中直接走 `on_value`）。与 `on_value` 分开是为了让
	/// 省份列表段把「选中」处理为 token 替换（打字则整段回写）。
	on_select: Option<EventHandler<String>>,
	/// 输入框占位提示（缺省无；空值时以浅色提示字段语义，如 add_ruler 的「头像」）。
	placeholder: Option<String>,
) -> Element {
	let mut open = use_signal(|| false);
	// 按压起点（逻辑坐标）：用于区分「点选」（松手位移很小）与「拖动滚动」。
	let mut press_start = use_signal(|| None::<(f64, f64)>);
	// 触摸点选的「待提交」值：pointerup 只记录，touchend 才提交并关闭（契约 4）。
	let mut pending_select = use_signal(|| None::<String>);
	// 展开方向与限高（实测值；测量失败时保持 208/向下）。
	let drop_up = use_signal(|| false);
	let drop_height = use_signal(|| 208.0_f64);
	// 测量纪元：重新聚焦时旧的后继复测结果作废。
	let mut measure_generation = use_signal(|| 0_u64);
	// 虚拟滚动：列表滚动位置 +「上次过滤条件」快照（变化时复位滚动，见下）。
	let mut list_scroll_top = use_signal(|| 0.0_f64);
	let mut last_filter = use_signal(String::new);
	// 候选列表 = 全量匹配项（不再截断为 60 条）：memo 缓存过滤结果——
	// 滚动只重算虚拟滚动窗口，不重跑过滤；关闭态重算返回空表，顺带释放大候选内存。
	let suggestions = use_memo(use_reactive(
		(&text, &filter_query, &options),
		move |(text, filter_query, options)| {
			if !*open.read() {
				return Vec::new();
			}
			let query = filter_query.as_deref().unwrap_or(&text);
			match &options {
				SuggestOptions::Fixed(items) => {
					let items: Vec<(String, String)> = items
						.iter()
						.map(|(value, label)| (value.to_string(), label.to_string()))
						.collect();
					event_lookup::filter_suggestions(&items, query, SUGGEST_MATCH_LIMIT)
				}
				SuggestOptions::Keys(list) => {
					event_lookup::filter_suggestions(list, query, SUGGEST_MATCH_LIMIT)
				}
				SuggestOptions::Names(names) => {
					event_lookup::filter_names(names, query, SUGGEST_MATCH_LIMIT)
				}
				SuggestOptions::Items(items) => {
					event_lookup::filter_suggestions(items, query, SUGGEST_MATCH_LIMIT)
				}
				SuggestOptions::Lookup(lookup, kind) => {
					event_lookup::matching_lookup(lookup, *kind, query, SUGGEST_MATCH_LIMIT)
				}
			}
		},
	));
	let open_now = *open.read();
	let drop_height_value = *drop_height.read();
	// 过滤条件（展开状态 / 查询文本）变化 → 滚动复位：DOM 侧由列表容器 `key` 变化
	// 重挂载归零，信号侧在同一渲染内同步复位（否则窗口按旧偏移计算、视口一片空白）。
	let query_now = filter_query.clone().unwrap_or_else(|| text.clone());
	let list_key = format!("{}|{}", u8::from(open_now), query_now);
	if *last_filter.peek() != list_key {
		last_filter.set(list_key.clone());
		if *list_scroll_top.peek() != 0.0 {
			list_scroll_top.set(0.0);
		}
	}
	// 虚拟滚动窗口（固定行高）：只渲染可视行 ± 缓冲行，上下占位块撑起滚动条。
	// 每渲染都读取 memo：关闭态重算返回空表，及时释放上一次的大候选。
	let (list_total, window_start, window_rows, top_spacer, bottom_spacer) = {
		let candidates = suggestions.read();
		let list_total = if open_now { candidates.len() } else { 0 };
		if list_total == 0 {
			(0, 0, Vec::new(), 0.0, 0.0)
		} else {
			let scroll_top = *list_scroll_top.read();
			let viewport_rows = (drop_height_value / SUGGEST_ROW_HEIGHT).ceil() as usize + 1;
			let (window_start, window_end, top_spacer, bottom_spacer) =
				suggest_window(scroll_top, list_total, viewport_rows);
			// 过滤变短后滚动信号可能超出内容高度：回夹防窗口空白。
			let max_scroll = list_total as f64 * SUGGEST_ROW_HEIGHT;
			if scroll_top > max_scroll {
				list_scroll_top.set(max_scroll);
			}
			(
				list_total,
				window_start,
				candidates[window_start..window_end].to_vec(),
				top_spacer,
				bottom_spacer,
			)
		}
	};
	rsx! {
        div { class: if *open.read() { "suggest-wrap suggest-wrap-open" } else { "suggest-wrap" },
            input {
                class: if invalid { "event-cell-input invalid" } else { "event-cell-input" },
                value: "{text}",
                placeholder: placeholder.unwrap_or_default(),
                spellcheck: "false",
                autocomplete: "off",
                autocapitalize: "none",
                // 点按输入框重新展开下拉（选中/遮罩关闭后输入框仍是焦点，不再触发 focus）。
                // 触摸不用 pointerdown 展开：手指以输入框起势滑动页面时会先触发 pointerdown，
                // 造成「滑动中误弹补全」（2026-10 修复）；触摸改由 onclick 兜底（滑动不产生 click）。
                onpointerdown: move |evt: Event<PointerData>| {
                    let pointer = evt.data().pointer_type();
                    if (pointer == "mouse" || pointer == "pen") && !*open.peek() {
                        open.set(true);
                    }
                },
                // 触摸 tap 在 touchend 后产生 click，鼠标点按同样到达这里（重复展开为无操作）。
                onclick: move |_| {
                    if !*open.peek() {
                        open.set(true);
                    }
                },
                // 继续打字同样重新展开（延续上一条：关闭 ≠ 失焦）。
                oninput: move |evt: FormEvent| {
                    if !*open.peek() {
                        open.set(true);
                    }
                    on_value.call(evt.value());
                },
                onfocus: move |_| {
                    open.set(true);
                    on_focus.call(());
                    // 即时测一次 + 延迟复测（等软键盘弹出后的视口稳定）。
                    let generation = measure_generation
                        .with_mut(|value| {
                            *value = value.wrapping_add(1);
                            *value
                        });
                    let open = open;
                    let mut drop_up = drop_up; // 已关闭或已重新聚焦（新一轮测量）：丢弃本次结果。
                    let mut drop_height = drop_height;
                    spawn(async move {
                        for delay in [0_u32, 320] {
                            let Some((below, above)) = measure_drop_space(delay).await else {
                                continue;
                            };
                            if *measure_generation.peek() != generation || !*open.peek() {
                                return;
                            }
                            let flip = below < 214.0 && above > below + 24.0;
                            drop_up.set(flip);
                            let room = if flip { above } else { below };
                            drop_height.set((room - 6.0).clamp(96.0, 208.0));
                        }
                    });
                },
                // 失焦不关闭：滚动或点按列表都会使输入框失焦，关闭交给遮罩 / 选择 / Esc。
                onblur: move |_| on_blur.call(()),
                onkeydown: move |evt: Event<KeyboardData>| {
                    if evt.key() == Key::Escape {
                        open.set(false);
                    }
                },
            }
            if *open.read() {
                // 全屏遮罩：点按空白处关闭下拉（移动端通用模式），并避免手势误触下层界面。
                div {
                    class: "suggest-backdrop",
                    // 鼠标 / 触控笔：点击关闭。
                    onclick: move |_| open.set(false),
                    // 触摸：先抑制兼容鼠标 / click 合成事件再关闭——否则「幽灵点击」会穿透到
                    // 遮罩下的控件；不能放 pointerdown（遮罩先卸载，touchend 无处抑制）。
                    ontouchend: move |evt: Event<TouchData>| {
                        evt.prevent_default();
                        open.set(false);
                    },
                }
            }
            if open_now && list_total > 0 {
                div {
                    // 过滤条件变化时换 key：列表 DOM 重挂载、滚动位置归零（与信号侧复位同步）。
                    key: "{list_key}",
                    class: if *drop_up.read() { "suggest-list suggest-list-up" } else { "suggest-list" },
                    style: "max-height: {drop_height_value:.0}px;",
                    onscroll: move |evt: Event<ScrollData>| {
                        let top = evt.scroll_top();
                        let mut list_scroll_top = list_scroll_top;
                        // 按 2 行节流：窗口带 24 行上下缓冲，无需跟随每个像素重渲染。
                        if (top - list_scroll_top.cloned()).abs() >= SUGGEST_ROW_HEIGHT * 2.0 {
                            list_scroll_top.set(top);
                        }
                    },
                    if top_spacer > 0.0 {
                        div {
                            style: "height: {top_spacer:.0}px;",
                            aria_hidden: "true",
                        }
                    }
                    for (row_offset , (item_value , item_label)) in window_rows.into_iter().enumerate() {
                        div {
                            key: "{window_start + row_offset}",
                            class: "suggest-item",
                            // 记录按压起点；不在 pointerdown 选中：轻触即选会让滚动无从下手。
                            onpointerdown: move |evt: Event<PointerData>| {
                                // 鼠标 / 触控笔的焦点变化发生在按下瞬间 → 阻止默认以免中途
                                // 失焦重排（如键行重新分组导致下拉被卸载）；触摸不阻止，
                                // 保证列表可以滚动（触摸焦点变化在松手之后，不影响点选）。
                                let pointer = evt.data().pointer_type();
                                if pointer == "mouse" || pointer == "pen" {
                                    evt.prevent_default();
                                }
                                // 新手势开始：丢弃上一次未提交的触摸点选。
                                pending_select.set(None);
                                let point = evt.client_coordinates();
                                press_start.set(Some((point.x, point.y)));
                            },
                            onpointerup: move |evt: Event<PointerData>| {
                                let start = *press_start.peek();
                                press_start.set(None);
                                let point = evt.client_coordinates();
                                let tapped = start
                                    .is_some_and(|(x, y)| {
                                        ((point.x - x).powi(2) + (point.y - y).powi(2)).sqrt() <= TAP_SLOP
                                    });
                                if !tapped {
                                    return;
                                }
                                if evt.data().pointer_type() == "touch" {
                                    // 触摸点选不在 pointerup 提交：此刻提交会先卸载列表（open=false），
                                    // 随后浏览器为本触摸合成的兼容 click 将落到弹窗遮挡的下层控件
                                    //（「幽灵点击」误触下层选项）。改由 touchend（晚于 pointerup）
                                    // 提交——届时 item 仍在挂载，可一并 preventDefault 抑制全部合成事件。
                                    pending_select.set(Some(item_value.clone()));
                                    return;
                                }
                                // 鼠标 / 触控笔：直接提交。
                                // 选中：token 化段有独立回调（替换最后一个 token），其余直接回传候选值。
                                match on_select {
                                    Some(handler) => handler.call(item_value.clone()),
                                    None => on_value.call(item_value.clone()),
                                }
                                open.set(false);
                            },
                            // 浏览器把触摸判定为滚动时触发：本次手势不算点选。
                            onpointercancel: move |_| {
                                press_start.set(None);
                                pending_select.set(None);
                            },
                            // 触摸松手：先抑制本次触摸的全部兼容鼠标 / click 合成事件
                            //（避免「幽灵点击」穿透到弹窗遮挡的下层控件），再提交
                            // pointerup 记录的待选值并关闭（item 此刻仍在，事件原子）。
                            // 同时输入框不失焦、键盘不收起；滚动手势走 pointercancel，不受影响。
                            ontouchend: move |evt: Event<TouchData>| {
                                evt.prevent_default();
                                press_start.set(None);
                                let pending = (*pending_select.peek()).clone();
                                pending_select.set(None);
                                if let Some(value) = pending {
                                    match on_select {
                                        Some(handler) => handler.call(value),
                                        None => on_value.call(value),
                                    }
                                    open.set(false);
                                }
                            },
                            // 空值项（如「（留空）」「（不写）」）只显示标签，选中即清空。
                            if item_value.is_empty() {
                                span { class: "suggest-item-value", "{item_label}" }
                            } else {
                                span { class: "suggest-item-value", "{item_value}" }
                                if !item_label.is_empty() {
                                    span { class: "suggest-item-label", "{item_label}" }
                                }
                            }
                        }
                    }
                    if bottom_spacer > 0.0 {
                        div {
                            style: "height: {bottom_spacer:.0}px;",
                            aria_hidden: "true",
                        }
                    }
                }
            }
        }
    }
}

/// 文明 / 政体候选列表（桌面端原生 `datalist`；独立组件：对照表不变时按 `Shared`
/// 指针判定跳过重渲染，避免数千条候选在每次输入时反复 diff）。
#[component]
fn LookupDatalists(lookup: Shared<EventLookup>) -> Element {
	rsx! {
        datalist { id: "{event_lookup::CIV_DATALIST_ID}",
            for item in lookup.civs.iter() {
                option { value: "{item.tag}", "{item.name}" }
            }
        }
        datalist { id: "{event_lookup::GOV_DATALIST_ID}",
            for item in lookup.governments.iter() {
                option { value: "{item.index}", "{item.name}" }
            }
        }
        datalist { id: "{event_lookup::PROVINCE_DATALIST_ID}",
            for item in lookup.provinces.iter() {
                if item.name.is_empty() {
                    option { value: "{item.id}" }
                } else {
                    option { value: "{item.id}", "{item.name}" }
                }
            }
        }
        datalist { id: "{event_lookup::BUILDING_DATALIST_ID}",
            for item in lookup.buildings.iter() {
                option { value: "{item.id}", "{item.name}" }
            }
        }
        datalist { id: "{event_lookup::DISEASE_DATALIST_ID}",
            for item in lookup.diseases.iter() {
                option { value: "{item.id}", "{item.name}" }
            }
        }
        datalist { id: "{event_lookup::CHARACTER_DATALIST_ID}",
            for name in lookup.characters.iter() {
                option { value: "{name}" }
            }
        }
        datalist { id: "{event_lookup::NATIONAL_SPIRIT_DATALIST_ID}",
            for item in lookup.national_spirits.iter() {
                if item.name.is_empty() {
                    option { value: "{item.id}" }
                } else {
                    option { value: "{item.id}", "{item.name}" }
                }
            }
        }
        datalist { id: "{event_lookup::TECHNOLOGY_DATALIST_ID}",
            for item in lookup.technologies.iter() {
                option { value: "{item.id}", "{item.name}" }
            }
        }
        datalist { id: "{event_lookup::RELIGION_DATALIST_ID}",
            for item in lookup.religions.iter() {
                option { value: "{item.id}", "{item.name}" }
            }
        }
        datalist { id: "{event_lookup::RESOURCE_DATALIST_ID}",
            for item in lookup.resources.iter() {
                option { value: "{item.id}", "{item.name}" }
            }
        }
        datalist { id: "{event_lookup::LAW_DATALIST_ID}",
            for (index , law) in lookup.laws.iter().enumerate() {
                option { value: "{index}", "{law.title}" }
            }
        }
        datalist { id: "{event_lookup::ARMY_DATALIST_ID}",
            for unit in lookup.units.iter() {
                option { value: "{unit.id}", "{unit.name}" }
            }
        }
        datalist { id: "{event_lookup::CONTINENT_DATALIST_ID}",
            for item in lookup.continents.iter() {
                option { value: "{item.id}", "{item.name}" }
            }
        }
        datalist { id: "{event_lookup::DECISION_DATALIST_ID}",
            for item in lookup.decisions.iter() {
                option { value: "{item.id}", "{item.name}" }
            }
        }
        datalist { id: "{event_lookup::DECISION_RUN_DATALIST_ID}",
            for item in lookup.decisions.iter() {
                for event in item.events.iter() {
                    option { value: "{item.id}:{event}", "{item.name}（{event}）" }
                }
            }
        }
    }
}

/// 表格行（键 | 值 | 说明 | 删除）。`show_delete` 为 false 时删除格留空。
#[component]
fn GridRow(
	field_key: String,
	value: String,
	input_kind: CellInput,
	invalid: bool,
	note: String,
	datalist: &'static str,
	options: Shared<Vec<(String, String)>>,
	native_autocomplete: bool,
	lookup: Shared<EventLookup>,
	images: Shared<Vec<String>>,
	events: Shared<Vec<String>>,
	music: Shared<Vec<String>>,
	ruler_images: Shared<Vec<String>>,
	alliance_specials: Shared<Vec<String>>,
	show_delete: bool,
	key_readonly: bool,
	on_key: EventHandler<String>,
	on_key_focus: EventHandler<()>,
	on_key_blur: EventHandler<()>,
	on_value: EventHandler<String>,
	on_remove: EventHandler<()>,
) -> Element {
	let show_preview = input_kind == CellInput::Text && has_game_codes(&value);
	let key_title = if key_readonly { "必填项目的键固定不可修改" } else { "" };
	// 文明 / 政体相关键：值按 `=` 分段渲染，各段挂对应的 datalist 并显示「值 → 名称」对照。
	// 少数键的分段随值形态 / 长度变化（如 `unlock_tech` 的 `文明=科技` 双写法、
	// `add_new_army` 的变长 `兵种=型号` 对）：优先用动态分段，`None` 落回静态表。
	let parts = if input_kind == CellInput::Text {
		event_lookup::value_parts_dynamic(&field_key, &value).or_else(|| value_parts(&field_key))
	} else {
		None
	};
	let segment_values: Vec<String> = value.split('=').map(str::to_string).collect();
	let segment_count = parts
		.map(|parts| parts.len().max(segment_values.len()))
		.unwrap_or(0);
	let hint = parts
		.map(|parts| name_hint(&lookup, parts, &value))
		.unwrap_or_default();
	// 分段渲染数据：（段索引、段文本、语义类型、段级语法是否合法）。
	// 段文本**原样显示**（包含省份列表的尾随 `;`——它是语法的一部分，隐藏会导致
	// 「刚输入 `;` 后候选仍按旧 token 过滤」、「选中后整体被替换」等错乱）；
	// 段级合法性用于把省份列表中的非法 token 定位到该段（整值红框仍作用于全部段）。
	let segment_render: Vec<(usize, String, ValuePart, bool)> = (0..segment_count)
		.map(|index| {
			let kind = parts
				.map(|parts| parts.get(index).copied().unwrap_or(ValuePart::Plain))
				.unwrap_or(ValuePart::Plain);
			let text = segment_values.get(index).cloned().unwrap_or_default();
			let part_ok = event_lookup::part_syntax_ok(kind, &text);
			(index, text, kind, part_ok)
		})
		.collect();
	// `add_ruler` 六段值改用专用编辑器（标签式字段，界面不显示 `=` 分隔符）；
	// `add_new_army` 成对值改用「每对一行」编辑器（可逐行删除 / 追加）。
	let ruler_widget = field_key == "add_ruler";
	let army_widget = field_key == "add_new_army";
	rsx! {
        div { class: "event-grid-row",
            div { class: "event-cell event-cell-key",
                if native_autocomplete {
                    input {
                        class: "event-cell-input",
                        list: "{datalist}",
                        value: "{field_key}",
                        spellcheck: "false",
                        readonly: key_readonly,
                        title: "{key_title}",
                        oninput: move |evt: FormEvent| on_key.call(evt.value()),
                        onfocus: move |_| on_key_focus.call(()),
                        onblur: move |_| on_key_blur.call(()),
                    }
                } else if key_readonly {
                    input {
                        class: "event-cell-input",
                        value: "{field_key}",
                        spellcheck: "false",
                        readonly: true,
                        title: "{key_title}",
                    }
                } else {
                    SuggestInput {
                        text: field_key.clone(),
                        options: SuggestOptions::Keys(options.clone()),
                        invalid: false,
                        on_value: move |new_key: String| on_key.call(new_key),
                        on_focus: move |_| on_key_focus.call(()),
                        on_blur: move |_| on_key_blur.call(()),
                    }
                }
            }
            div { class: "event-cell event-cell-value",
                // Bool 值：桌面端用原生 select；安卓端也走自绘（统一内联体验，
                // 并保留「（留空）」空值项——空值项以仅标签形式展示，选中即清空）。
                if input_kind == CellInput::Bool {
                    if native_autocomplete {
                        select {
                            class: if invalid { "event-cell-input invalid" } else { "event-cell-input" },
                            value: "{value}",
                            oninput: move |evt: FormEvent| on_value.call(evt.value()),
                            option { value: "", "（留空）" }
                            option { value: "true", "true" }
                            option { value: "false", "false" }
                        }
                    } else {
                        SuggestInput {
                            text: value.clone(),
                            options: SuggestOptions::Fixed(BOOL_OPTIONS),
                            invalid,
                            on_value,
                            on_focus: move |_| {},
                            on_blur: move |_| {},
                        }
                    }
                } else if parts.is_some() {
                    div { class: "event-value-parts",
                        if ruler_widget {
                            // 六段复合值（名字=姓氏=头像=日=月=年）改用带标签的专用编辑器：
                            // 界面上不再出现 `=` 分隔符，但写回文件时仍拼回原格式。
                            RulerValueInput {
                                value: value.clone(),
                                invalid,
                                native_autocomplete,
                                ruler_images: ruler_images.clone(),
                                on_value,
                            }
                        } else if army_widget {
                            // `兵种=型号` 成对重复：每对一行（行尾 × 删除该对、底部追加）。
                            ArmyValueInput {
                                value: value.clone(),
                                invalid,
                                native_autocomplete,
                                lookup: lookup.clone(),
                                on_value,
                            }
                        } else {
                            for (index , text , kind , part_ok) in segment_render {
                                if index > 0 {
                                    span { class: "event-value-sep", "=" }
                                }
                                ValuePartInput {
                                    text,
                                    kind,
                                    invalid: invalid || !part_ok,
                                    segment_index: index,
                                    full_value: value.clone(),
                                    native_autocomplete,
                                    lookup: lookup.clone(),
                                    images: images.clone(),
                                    events: events.clone(),
                                    music: music.clone(),
                                    ruler_images: ruler_images.clone(),
                                    alliance_specials: alliance_specials.clone(),
                                    on_value,
                                }
                            }
                        }
                    }
                    if !hint.is_empty() {
                        div { class: "event-value-hint", "{hint}" }
                    }
                    if show_preview {
                        GameTextPreview { text: value.clone() }
                    }
                } else {
                    input {
                        class: if invalid { "event-cell-input invalid" } else { "event-cell-input" },
                        value: "{value}",
                        spellcheck: "false",
                        oninput: move |evt: FormEvent| on_value.call(evt.value()),
                    }
                    if show_preview {
                        GameTextPreview { text: value.clone() }
                    }
                }
            }
            div { class: "event-cell event-cell-note",
                span { class: "event-note-text", title: "{note}", "{note}" }
            }
            if show_delete {
                div { class: "event-cell event-cell-del",
                    button {
                        class: "event-del",
                        r#type: "button",
                        title: "删除此行",
                        aria_label: "删除此行",
                        onclick: move |_| on_remove.call(()),
                        "×"
                    }
                }
            } else {
                div { class: "event-cell event-cell-del" }
            }
        }
    }
}

/// 表格表头（键 | 值 | 说明 | 空）。
#[component]
fn GridHead() -> Element {
	rsx! {
        div { class: "event-grid-head",
            span { "键" }
            span { "值" }
            span { "说明" }
            span {}
        }
    }
}

/// 一个表格分区（标题 + 表头 + 行 + 添加按钮）。
/// `can_add`/`can_remove` 控制该分区是否允许增删行（如必填项目固定不可增删）。
#[component]
fn GridSection(
	title: String,
	rows: Vec<(usize, String, String)>,
	datalist: &'static str,
	options: Shared<Vec<(String, String)>>,
	native_autocomplete: bool,
	lookup: Shared<EventLookup>,
	images: Shared<Vec<String>>,
	events: Shared<Vec<String>>,
	music: Shared<Vec<String>>,
	ruler_images: Shared<Vec<String>>,
	alliance_specials: Shared<Vec<String>>,
	can_add: bool,
	can_remove: bool,
	editable_key: bool,
	on_key: EventHandler<(usize, String)>,
	on_key_focus: EventHandler<usize>,
	on_key_blur: EventHandler<()>,
	on_value: EventHandler<(usize, String)>,
	on_remove: EventHandler<usize>,
	on_add: EventHandler<()>,
	add_label: String,
) -> Element {
	rsx! {
        section { class: "event-section",
            div { class: "event-section-title",
                span { "{title}" }
                if can_add {
                    button {
                        class: "event-title-add",
                        r#type: "button",
                        title: "{add_label}",
                        aria_label: "{add_label}",
                        onclick: move |_| on_add.call(()),
                        "＋"
                    }
                }
            }
            GridHead {}
            for (index , key , value) in rows {
                GridRow {
                    field_key: key.clone(),
                    value: value.clone(),
                    input_kind: cell_input_for(&key),
                    invalid: is_invalid(&key, &value),
                    note: note_for(&key),
                    datalist,
                    options: options.clone(),
                    native_autocomplete,
                    lookup: lookup.clone(),
                    images: images.clone(),
                    events: events.clone(),
                    music: music.clone(),
                    ruler_images: ruler_images.clone(),
                    alliance_specials: alliance_specials.clone(),
                    show_delete: can_remove,
                    key_readonly: !editable_key,
                    on_key: move |new_key: String| on_key.call((index, new_key)),
                    on_key_focus: move |_| on_key_focus.call(index),
                    on_key_blur: move |_| on_key_blur.call(()),
                    on_value: move |new_value: String| on_value.call((index, new_value)),
                    on_remove: move |_| on_remove.call(index),
                }
            }
            if can_add {
                button {
                    class: "event-add-row",
                    r#type: "button",
                    onclick: move |_| on_add.call(()),
                    "{add_label}"
                }
            }
        }
    }
}

/// 以可变方式访问当前事件；未载入时忽略。
/// Signal 按值复制后重新绑定为可变，使调用方闭包保持 Fn 语义、可被多个分区复用。
fn mutate_event(event: Signal<Option<MissionEvent>>, f: impl FnOnce(&mut MissionEvent)) {
	let mut event = event;
	event.with_mut(|current| {
		if let Some(data) = current.as_mut() {
			f(data);
		}
	});
}

/// 特化 Excel 表格编辑器。直接编辑 `MissionEvent` 信号。
/// `lookup` 为文明/政体对照表（事件面板加载后传入，供值单元格补全与名称提示）。
/// `native_autocomplete` 为 true 时（桌面端）用原生 `datalist`；安卓端改用页面内
/// 自绘下拉（原生弹层在滚动/软键盘后上/下都会错位）。
/// `images` 为事件图片候选（`image` / `mission_image` 等字段的 `.png` 文件名），
/// `events` 为事件候选（`run_event` 值 = 文件名去 `.txt`），`music` 为音乐候选
/// （`musicName` / `play_music` 值）——均由事件面板按资源根加载，含从 apk 导入工作区的源 APK 兜底。
/// `ruler_images` 为统治者头像候选（`add_ruler` 第三段），`alliance_specials` 为剧情特殊联盟名称。
#[component]
pub fn EventGrid(
	event: Signal<Option<MissionEvent>>,
	lookup: Shared<EventLookup>,
	native_autocomplete: bool,
	images: Shared<Vec<String>>,
	events: Shared<Vec<String>>,
	music: Shared<Vec<String>>,
	ruler_images: Shared<Vec<String>>,
	alliance_specials: Shared<Vec<String>>,
) -> Element {
	let mut editing_key: Signal<Option<(usize, HeaderGroup)>> = use_signal(|| None);
	let snapshot = event.read().clone();
	let Some(data) = snapshot else {
		return rsx! {
            div { class: "event-grid-empty", "脚本尚未载入" }
        };
	};

	// ===== 头部字段分组 =====
	// 键正在编辑的行冻结在编辑开始时的分区里（输入过程中不重新归类，
	// 避免行从当前分区"消失"）；失焦后再按新键重新分组。
	let frozen_edit = *editing_key.read();
	let mut required_rows: Vec<(usize, String, String)> = Vec::new();
	let mut optional_rows: Vec<(usize, String, String)> = Vec::new();
	let mut unknown_rows: Vec<(usize, String, String)> = Vec::new();
	for (index, entry) in data.header.iter().enumerate() {
		let row = (index, entry.key.clone(), entry.value_or_default().to_string());
		let group = match frozen_edit {
			Some((frozen_index, frozen)) if frozen_index == index => frozen,
			_ => header_group_of(&entry.key),
		};
		match group {
			HeaderGroup::Required => required_rows.push(row),
			HeaderGroup::Optional => optional_rows.push(row),
			HeaderGroup::Unknown => unknown_rows.push(row),
		}
	}

	let header_datalist = header_suggestions(&data);
	let trigger_datalist = trigger_suggestions(&data);
	let effect_datalist = effect_suggestions(&data);
	// 安卓自绘下拉的键候选（Shared 指针供各行/各输入框共享，避免逐行深拷贝）。
	let header_options = Shared::new(header_datalist.clone());
	let trigger_options = Shared::new(trigger_datalist.clone());
	let effect_options = Shared::new(effect_datalist.clone());

	// ===== 头部字段编辑 =====
	// 通过 mutate_event 共享访问，保证闭包是 Fn（可被多个表格分区复用）。
	let set_header_key = move |(index, key): (usize, String)| {
		mutate_event(event, |data| {
			if let Some(entry) = data.header.get_mut(index) {
				entry.key = key;
			}
		});
	};
	let set_header_value = move |(index, value): (usize, String)| {
		mutate_event(event, |data| {
			if let Some(entry) = data.header.get_mut(index) {
				entry.set_value(&value);
			}
		});
	};
	let remove_header = move |index: usize| {
		mutate_event(event, |data| {
			data.header.remove(index);
		});
	};
	let add_header_field = move |default_key: String| {
		mutate_event(event, |data| {
			data.header.push(EntryLine::new(&default_key, ""));
		});
	};

	// 记录"键编辑会话"：冻结该行的分区，失焦后恢复实时归类。
	let begin_key_edit = move |index: usize| {
		let group = {
			let guard = event.read();
			match &*guard {
				Some(data) => data
					.header
					.get(index)
					.map_or(HeaderGroup::Unknown, |entry| header_group_of(&entry.key)),
				None => HeaderGroup::Unknown,
			}
		};
		editing_key.set(Some((index, group)));
	};
	let end_key_edit = move |_: ()| {
		editing_key.set(None);
	};

	// ===== 触发块编辑 =====
	let set_trigger_kind = move |(block_index, kind): (usize, TriggerKind)| {
		mutate_event(event, |data| {
			if let Some(block) = data.triggers.get_mut(block_index) {
				block.kind = kind;
				// 改成新类型后，结束行按新类型写（不再保留原文件的手误结束行）。
				block.close = None;
			}
		});
	};
	let set_trigger_join = move |(block_index, join): (usize, Option<NextOp>)| {
		mutate_event(event, |data| {
			if let Some(block) = data.triggers.get_mut(block_index) {
				block.join = join;
				// 用户改动连接方式后按标准 token 写（不再保留原文件的别名写法）。
				block.join_token = None;
			}
		});
	};
	let set_condition_key = move |(block_index, row_index, key): (usize, usize, String)| {
		mutate_event(event, |data| {
			if let Some(entry) = data
				.triggers
				.get_mut(block_index)
				.and_then(|block| block.conditions.get_mut(row_index))
			{
				entry.key = key;
			}
		});
	};
	let set_condition_value = move |(block_index, row_index, value): (usize, usize, String)| {
		mutate_event(event, |data| {
			if let Some(entry) = data
				.triggers
				.get_mut(block_index)
				.and_then(|block| block.conditions.get_mut(row_index))
			{
				entry.set_value(&value);
			}
		});
	};
	let remove_condition = move |(block_index, row_index): (usize, usize)| {
		mutate_event(event, |data| {
			if let Some(block) = data.triggers.get_mut(block_index) {
				block.conditions.remove(row_index);
			}
		});
	};
	let add_condition = move |block_index: usize| {
		mutate_event(event, |data| {
			if let Some(block) = data.triggers.get_mut(block_index) {
				block.conditions.push(EntryLine::new("is_civ", ""));
			}
		});
	};
	let remove_trigger_block = move |block_index: usize| {
		mutate_event(event, |data| {
			data.triggers.remove(block_index);
		});
	};
	let add_trigger_block = move |_| {
		mutate_event(event, |data| {
			data.triggers.push(TriggerBlock {
				blank_before: 1,
				kind: TriggerKind::And,
				join: Some(NextOp::And),
				join_after: 0,
				join_blank_before: 0,
				join_token: None,
				close_blank_before: 0,
				close: None,
				conditions: vec![EntryLine::new("is_civ", "")],
			});
		});
	};

	// ===== 选项块编辑 =====
	let set_option_name = move |(option_index, name): (usize, String)| {
		mutate_event(event, |data| {
			if let Some(block) = data.options.get_mut(option_index) {
				block.name = Some(name);
			}
		});
	};
	let set_effect_key = move |(option_index, row_index, key): (usize, usize, String)| {
		mutate_event(event, |data| {
			if let Some(entry) = data
				.options
				.get_mut(option_index)
				.and_then(|block| block.effects.get_mut(row_index))
			{
				entry.key = key;
			}
		});
	};
	let set_effect_value = move |(option_index, row_index, value): (usize, usize, String)| {
		mutate_event(event, |data| {
			if let Some(entry) = data
				.options
				.get_mut(option_index)
				.and_then(|block| block.effects.get_mut(row_index))
			{
				entry.set_value(&value);
			}
		});
	};
	let remove_effect = move |(option_index, row_index): (usize, usize)| {
		mutate_event(event, |data| {
			if let Some(block) = data.options.get_mut(option_index) {
				block.effects.remove(row_index);
			}
		});
	};
	let add_effect = move |option_index: usize| {
		mutate_event(event, |data| {
			if let Some(block) = data.options.get_mut(option_index) {
				block.effects.push(EntryLine::new("legacy", ""));
			}
		});
	};
	let remove_option_block = move |option_index: usize| {
		mutate_event(event, |data| {
			data.options.remove(option_index);
		});
	};
	let add_option_block = move |_| {
		mutate_event(event, |data| {
			data.options.push(super::event_parser::OptionBlock {
				blank_before: 1,
				name: Some(String::new()),
				name_after: 0,
				name_blank_before: 0,
				close_blank_before: 0,
				close: None,
				effects: Vec::new(),
			});
		});
	};

	let trigger_blocks = data.triggers.clone();
	let option_blocks = data.options.clone();

	rsx! {
        // 自绘下拉需要越过分区圆角裁剪（`suggest-overlay` 解除 `.event-section` 的
        // `overflow: hidden`；省份列表段在桌面端也走自绘，故常开）。
        div { class: "event-grid suggest-overlay",
            // 桌面端：原生 datalist 候选（选项标签为「说明」提示）；安卓端改用自绘下拉。
            if native_autocomplete {
                datalist { id: "evdl-header",
                    for (key , note) in header_datalist {
                        option { value: "{key}", "{note}" }
                    }
                }
                datalist { id: "evdl-trigger",
                    for (key , note) in trigger_datalist {
                        option { value: "{key}", "{note}" }
                    }
                }
                datalist { id: "evdl-effect",
                    for (key , note) in effect_datalist {
                        option { value: "{key}", "{note}" }
                    }
                }
                // 文明 / 政体值候选（供值单元格分段输入框引用；无数据时不渲染）
                if !lookup.is_empty() {
                    LookupDatalists { lookup: lookup.clone() }
                }
                // 图片资源候选（image / mission_image 等字段引用的 .png 文件名）
                if !images.is_empty() {
                    datalist { id: "{event_lookup::IMAGE_DATALIST_ID}",
                        for name in images.iter() {
                            option { value: "{name}" }
                        }
                    }
                }
                // 统治者头像候选（add_ruler 第三段；文件名去 .png）
                if !ruler_images.is_empty() {
                    datalist { id: "{event_lookup::RULER_IMAGE_DATALIST_ID}",
                        for name in ruler_images.iter() {
                            option { value: "{name}" }
                        }
                    }
                }
                // 特殊联盟候选（join/leave_alliance_special_id_*；值 = 编号，标签 = 名称）
                if !alliance_specials.is_empty() {
                    datalist { id: "{event_lookup::ALLIANCE_SPECIAL_DATALIST_ID}",
                        for (index , name) in alliance_specials.iter().enumerate() {
                            option { value: "{index}", "{name}" }
                        }
                    }
                }
                // 事件 / 音乐候选（run_event 与 musicName / play_music 字段的值）
                if !events.is_empty() {
                    datalist { id: "{event_lookup::EVENT_DATALIST_ID}",
                        for name in events.iter() {
                            option { value: "{name}" }
                        }
                    }
                }
                if !music.is_empty() {
                    datalist { id: "{event_lookup::MUSIC_DATALIST_ID}",
                        for name in music.iter() {
                            option { value: "{name}" }
                        }
                    }
                }
            }

            GridSection {
                title: "必填项目".to_string(),
                rows: required_rows,
                datalist: "evdl-header",
                options: header_options.clone(),
                native_autocomplete,
                lookup: lookup.clone(),
                images: images.clone(),
                events: events.clone(),
                music: music.clone(),
                ruler_images: ruler_images.clone(),
                alliance_specials: alliance_specials.clone(),
                can_add: false,
                can_remove: false,
                editable_key: false,
                on_key: set_header_key,
                on_key_focus: begin_key_edit,
                on_key_blur: end_key_edit,
                on_value: set_header_value,
                on_remove: remove_header,
                on_add: move |_| add_header_field("id".to_string()),
                add_label: "＋ 添加必填项".to_string(),
            }
            GridSection {
                title: "可填项目".to_string(),
                rows: optional_rows,
                datalist: "evdl-header",
                options: header_options.clone(),
                native_autocomplete,
                lookup: lookup.clone(),
                images: images.clone(),
                events: events.clone(),
                music: music.clone(),
                ruler_images: ruler_images.clone(),
                alliance_specials: alliance_specials.clone(),
                can_add: true,
                can_remove: true,
                editable_key: true,
                on_key: set_header_key,
                on_key_focus: begin_key_edit,
                on_key_blur: end_key_edit,
                on_value: set_header_value,
                on_remove: remove_header,
                on_add: move |_| add_header_field("mission_image".to_string()),
                add_label: "＋ 添加可选项".to_string(),
            }
            GridSection {
                title: "其他（未识别字段）".to_string(),
                rows: unknown_rows,
                datalist: "evdl-header",
                options: header_options.clone(),
                native_autocomplete,
                lookup: lookup.clone(),
                images: images.clone(),
                events: events.clone(),
                music: music.clone(),
                ruler_images: ruler_images.clone(),
                alliance_specials: alliance_specials.clone(),
                can_add: true,
                can_remove: true,
                editable_key: true,
                on_key: set_header_key,
                on_key_focus: begin_key_edit,
                on_key_blur: end_key_edit,
                on_value: set_header_value,
                on_remove: remove_header,
                on_add: move |_| add_header_field(String::new()),
                add_label: "＋ 添加字段".to_string(),
            }

            for (block_index , block) in trigger_blocks.iter().enumerate() {
                section { class: "event-section",
                    div { class: "event-section-title event-section-title-block",
                        span { "触发条件 #{block_index + 1}" }
                        div { class: "event-block-controls",
                            // 标签 + 控件成对成组：窄面板换行时不会把标签与控件拆到两行。
                            span { class: "event-block-pair",
                                label { "块类型" }
                                if native_autocomplete {
                                    select {
                                        class: "event-block-select",
                                        value: "{block.kind.open_token()}",
                                        oninput: move |evt: FormEvent| {
                                            set_trigger_kind((block_index, TriggerKind::from_open_token(&evt.value())));
                                        },
                                        option { value: "trigger_and", "and" }
                                        option { value: "trigger_or", "or" }
                                        option { value: "trigger_and_not", "and_not" }
                                    }
                                } else {
                                    // 安卓自绘：显示短名（与原生 select 标签一致），写回时映射回类型。
                                    SuggestInput {
                                        text: trigger_kind_short(block.kind).to_string(),
                                        options: SuggestOptions::Fixed(TRIGGER_KIND_OPTIONS),
                                        invalid: false,
                                        on_value: move |text: String| {
                                            set_trigger_kind((block_index, trigger_kind_from_text(&text)));
                                        },
                                        on_focus: move |_| {},
                                        on_blur: move |_| {},
                                    }
                                }
                            }
                            span { class: "event-block-pair",
                                label { "连接" }
                                if native_autocomplete {
                                    select {
                                        class: "event-block-select",
                                        value: "{block.join.map_or(String::new(), |join| join.token().to_string())}",
                                        oninput: move |evt: FormEvent| {
                                            let join = NextOp::from_token(&evt.value());
                                            set_trigger_join((block_index, join));
                                        },
                                        option { value: "", "（不写）" }
                                        option { value: "next_and", "next_and" }
                                        option { value: "next_or", "next_or" }
                                        option { value: "next_and_not", "next_and_not" }
                                    }
                                } else {
                                    // 安卓自绘：候选为操作符 token，空文本 = 不写连接行。
                                    SuggestInput {
                                        text: block.join.map_or(String::new(), |join| join.token().to_string()),
                                        options: SuggestOptions::Fixed(TRIGGER_JOIN_OPTIONS),
                                        invalid: false,
                                        on_value: move |text: String| {
                                            set_trigger_join((block_index, NextOp::from_token(&text)));
                                        },
                                        on_focus: move |_| {},
                                        on_blur: move |_| {},
                                    }
                                }
                            }
                        }
                        // 删除按钮绝对定位在标题行右上角：不参与控件换行，
                        // 避免窄面板下「×」被单独挤到一行（见 styles.css）。
                        button {
                            class: "event-del event-block-del",
                            r#type: "button",
                            title: "删除此块",
                            aria_label: "删除此块",
                            onclick: move |_| remove_trigger_block(block_index),
                            "×"
                        }
                    }
                    GridHead {}
                    for (row_index , entry) in block.conditions.iter().enumerate() {
                        GridRow {
                            field_key: entry.key.clone(),
                            value: entry.value_or_default().to_string(),
                            input_kind: cell_input_for(&entry.key),
                            invalid: is_invalid(&entry.key, entry.value_or_default()),
                            note: note_for(&entry.key),
                            datalist: "evdl-trigger",
                            options: trigger_options.clone(),
                            native_autocomplete,
                            lookup: lookup.clone(),
                            images: images.clone(),
                            events: events.clone(),
                            music: music.clone(),
                            ruler_images: ruler_images.clone(),
                            alliance_specials: alliance_specials.clone(),
                            show_delete: true,
                            key_readonly: false,
                            on_key: move |new_key: String| set_condition_key((block_index, row_index, new_key)),
                            on_key_focus: move |_| {},
                            on_key_blur: move |_| {},
                            on_value: move |new_value: String| set_condition_value((block_index, row_index, new_value)),
                            on_remove: move |_| remove_condition((block_index, row_index)),
                        }
                    }
                    button {
                        class: "event-add-row",
                        r#type: "button",
                        onclick: move |_| add_condition(block_index),
                        "＋ 添加条件"
                    }
                }
            }

            for (option_index , block) in option_blocks.iter().enumerate() {
                section { class: "event-section",
                    div { class: "event-section-title",
                        span { "收益选项 #{option_index + 1}" }
                        button {
                            class: "event-del event-block-del",
                            r#type: "button",
                            title: "删除此选项",
                            aria_label: "删除此选项",
                            onclick: move |_| remove_option_block(option_index),
                            "×"
                        }
                    }
                    div { class: "event-option-name",
                        label { "name" }
                        input {
                            class: "event-cell-input",
                            value: "{block.name.clone().unwrap_or_default()}",
                            spellcheck: "false",
                            oninput: move |evt: FormEvent| set_option_name((option_index, evt.value())),
                        }
                        if has_game_codes(block.name.as_deref().unwrap_or_default()) {
                            GameTextPreview { text: block.name.clone().unwrap_or_default() }
                        }
                    }
                    GridHead {}
                    for (row_index , entry) in block.effects.iter().enumerate() {
                        GridRow {
                            field_key: entry.key.clone(),
                            value: entry.value_or_default().to_string(),
                            input_kind: cell_input_for(&entry.key),
                            invalid: is_invalid(&entry.key, entry.value_or_default()),
                            note: note_for(&entry.key),
                            datalist: "evdl-effect",
                            options: effect_options.clone(),
                            native_autocomplete,
                            lookup: lookup.clone(),
                            images: images.clone(),
                            events: events.clone(),
                            music: music.clone(),
                            ruler_images: ruler_images.clone(),
                            alliance_specials: alliance_specials.clone(),
                            show_delete: true,
                            key_readonly: false,
                            on_key: move |new_key: String| set_effect_key((option_index, row_index, new_key)),
                            on_key_focus: move |_| {},
                            on_key_blur: move |_| {},
                            on_value: move |new_value: String| set_effect_value((option_index, row_index, new_value)),
                            on_remove: move |_| remove_effect((option_index, row_index)),
                        }
                    }
                    button {
                        class: "event-add-row",
                        r#type: "button",
                        onclick: move |_| add_effect(option_index),
                        "＋ 添加效果"
                    }
                }
            }

            section { class: "event-section event-section-actions",
                button {
                    class: "event-add-row",
                    r#type: "button",
                    onclick: add_trigger_block,
                    "＋ 添加触发条件块"
                }
                button {
                    class: "event-add-row",
                    r#type: "button",
                    onclick: add_option_block,
                    "＋ 添加收益选项"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn suggest_window_covers_viewport_with_buffer() {
		// 顶部：窗口从第 0 行开始，覆盖可视行 + 下缓冲；顶占位为 0。
		let (start, end, top, bottom) = suggest_window(0.0, 1000, 7);
		assert_eq!(start, 0);
		assert_eq!(end, 7 + SUGGEST_BUFFER_ROWS * 2);
		assert_eq!(top, 0.0);
		assert_eq!(bottom, (1000 - end) as f64 * SUGGEST_ROW_HEIGHT);
	}

	#[test]
	fn suggest_window_follows_scroll_and_clamps() {
		// 中部：窗口起始 = 可视首行 - 上缓冲（顶占位与起始行一致）。
		let (start, end, top, _) = suggest_window(SUGGEST_ROW_HEIGHT * 500.0, 1000, 7);
		assert_eq!(start, 500 - SUGGEST_BUFFER_ROWS);
		assert_eq!(top, start as f64 * SUGGEST_ROW_HEIGHT);
		assert!(end <= 1000);
		// 超出内容高度：回夹到末尾，窗口不越界、底占位为 0。
		let (start, end, _, bottom) = suggest_window(f64::MAX, 50, 7);
		assert_eq!(end, 50);
		assert_eq!(bottom, 0.0);
		assert!(start < 50);
		// 空列表：空窗口。
		assert_eq!(suggest_window(0.0, 0, 7), (0, 0, 0.0, 0.0));
	}
}
