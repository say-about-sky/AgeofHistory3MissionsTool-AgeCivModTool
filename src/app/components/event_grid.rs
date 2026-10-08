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
//! 文明ID / tag / civ 与政体整数字段：值按 `=` 分段渲染，各段挂 datalist 候选，
//! 并在下方显示「值 → 名称」对照提示（数据来自 `event_lookup` 对照表）。

use dioxus::prelude::*;

use super::event_lookup::{self, datalist_id, name_hint, part_suffix, value_parts, EventLookup, ValuePart};
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
/// 分别挂对应的候选列表提供自动补全；`suffix` 为拼接时自动附加的固定后缀）。
/// 桌面端用原生 `datalist`（`kind` 决定候选表）；安卓端原生弹层定位不可靠
/// （页面滚动 / 软键盘弹出后上/下都会错位，旧 WebView 还不支持 datalist），
/// 改用页面内自绘下拉（见 [`SuggestInput`]）。
#[component]
fn ValuePartInput(
	text: String,
	kind: ValuePart,
	suffix: &'static str,
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
	on_value: EventHandler<String>,
) -> Element {
	let on_text = EventHandler::new(move |new_text: String| {
		on_value.call(event_lookup::replace_value_part(
			&full_value,
			segment_index,
			suffix,
			&new_text,
		));
	});
	let list_id = datalist_id(kind);
	rsx! {
        if native_autocomplete {
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
                    _ => SuggestOptions::Lookup(lookup, kind),
                },
                invalid,
                on_value: on_text,
                on_focus: move |_| {},
                on_blur: move |_| {},
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
//      `text` 为受控显示文本；`on_value` 回传所选/输入的文本，由调用方写回模型；
//      `on_focus` / `on_blur` 供调用方额外记账（如键行的冻结分组），无需求传 `move |_| {}`。
//   3. 桌面端保留原生控件时用 `native_autocomplete` 开关分支（见 `GridRow` 与触发块控件）。
//
// 【新增一种资源候选】以图片 / 事件 / 音乐为例：后端在 `list_event_assets`（`EventAssets`，
//   工作区目录 + 源 APK 条目，见 `missions_db.rs`）里增加一类收集函数与字段；前端在
//   `event_lookup.rs` 加 `ValuePart` 变体；在 `event.rs` 加载后经 `images` / `events` /
//   `music` 属性传到 `ValuePartInput`（`SuggestOptions::Names`）。
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
	/// 对照表候选——按字段类型实时过滤（值, 名称）。
	Lookup(Shared<EventLookup>, ValuePart),
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

/// 自绘候选下拉输入框（安卓端替代原生 `<datalist>` / `<select>` 弹层）。
///
/// 安卓 WebView 的原生弹层是浏览器原生窗口，按屏幕坐标 + 锚点定位，页面滚动 /
/// 软键盘弹出（WebView 尺寸变化）后会按错误偏移量放置（向上/向下都会偏）；
/// 改为页面内绝对定位的下拉后，位置始终跟随输入框。
///
/// # 行为契约（修改前必读，均为触屏实测后的选择）
///
/// 1. **松手提交**：`pointerdown` 只记录起点，`pointerup` 位移 ≤ `TAP_SLOP` 才选中；
///    拖动超阈值或 `pointercancel`（浏览器判定为滚动）不选中，列表保持打开可继续滚。
///    触摸的 `pointerdown` 不能 `preventDefault`（否则滚动失效）；鼠标 / 触控笔则相反，
///    要在按下时 `preventDefault`（它们按下瞬间就夺焦，键行会中途重新分组致下拉卸载）。
/// 2. **失焦不关闭**：滚动列表 / 点按候选都会让输入框失焦；关闭只由遮罩点击、选中、Esc 触发。
///    若改回「失焦即关」，触摸滚动点选将丢失（松手前列表已被卸载）。
/// 3. **遮罩模式**：展开时铺全屏 `.suggest-backdrop`（点按关闭）。展开的 wrap 提升层级
///    （`.suggest-wrap-open`），保证输入框（z25）与列表（z30）在遮罩（z24）之上。
///    因此列表是「模态候选」：先点空白关闭、再点其他控件（移动端点选器惯例）。
/// 4. **触摸点选抑制兼容鼠标事件**（item `touchend` preventDefault）：防止松手后列表
///    卸载时「幽灵点击」落到下层元素（如行尾删除按钮）；滚动手势走 pointercancel 不受影响。
/// 5. **展开方向自适应**：聚焦时 [`measure_drop_space`] 即刻实测 + 320ms 复测（等软键盘 /
///    视口稳定），`below < 214 && above > below + 24` 时向上展开（`.suggest-list-up`），
///    并按所向空间内联 `max-height`；`measure_generation` 纪元用于丢弃过期结果。
/// 6. **按需过滤**：候选项仅在展开时计算（同一时刻只有一个输入框展开，避免大对照表反复扫描）。
#[component]
fn SuggestInput(
	text: String,
	options: SuggestOptions,
	invalid: bool,
	on_value: EventHandler<String>,
	on_focus: EventHandler<()>,
	on_blur: EventHandler<()>,
) -> Element {
	let mut open = use_signal(|| false);
	// 按压起点（逻辑坐标）：用于区分「点选」（松手位移很小）与「拖动滚动」。
	let mut press_start = use_signal(|| None::<(f64, f64)>);
	// 展开方向与限高（实测值；测量失败时保持 208/向下）。
	let drop_up = use_signal(|| false);
	let drop_height = use_signal(|| 208.0_f64);
	// 测量纪元：重新聚焦时旧的后继复测结果作废。
	let mut measure_generation = use_signal(|| 0_u64);
	let suggestions: Vec<(String, String)> = if *open.read() {
		match &options {
			SuggestOptions::Fixed(items) => {
				let items: Vec<(String, String)> = items
					.iter()
					.map(|(value, label)| (value.to_string(), label.to_string()))
					.collect();
				event_lookup::filter_suggestions(&items, &text, 60)
			}
			SuggestOptions::Keys(list) => event_lookup::filter_suggestions(list, &text, 60),
			SuggestOptions::Names(names) => event_lookup::filter_names(names, &text, 60),
			SuggestOptions::Lookup(lookup, kind) => {
				event_lookup::matching_lookup(lookup, *kind, &text, 60)
			}
		}
	} else {
		Vec::new()
	};
	let drop_height_value = *drop_height.read();
	rsx! {
        div { class: if *open.read() { "suggest-wrap suggest-wrap-open" } else { "suggest-wrap" },
            input {
                class: if invalid { "event-cell-input invalid" } else { "event-cell-input" },
                value: "{text}",
                spellcheck: "false",
                autocomplete: "off",
                autocapitalize: "none",
                oninput: move |evt: FormEvent| on_value.call(evt.value()),
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
                    onpointerdown: move |_| open.set(false),
                }
            }
            if *open.read() && !suggestions.is_empty() {
                div {
                    class: if *drop_up.read() { "suggest-list suggest-list-up" } else { "suggest-list" },
                    style: "max-height: {drop_height_value:.0}px;",
                    for (item_value , item_label) in suggestions {
                        div {
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
                                if tapped {
                                    on_value.call(item_value.clone());
                                    open.set(false);
                                }
                            },
                            // 浏览器把触摸判定为滚动时触发：本次手势不算点选。
                            onpointercancel: move |_| press_start.set(None),
                            // 触摸点选不生成兼容鼠标事件（click 等）：避免松手后列表卸载时
                            //「幽灵点击」落到下层元素（如行尾的删除按钮），同时输入框不失焦、
                            // 键盘不收起；滚动手势走 pointercancel 路径，不受影响。
                            ontouchend: move |evt: Event<TouchData>| evt.prevent_default(),
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
	let parts = if input_kind == CellInput::Text {
		value_parts(&field_key)
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
	// 分段渲染数据：（段索引、显示文本、语义类型、固定后缀）。
	// 固定后缀（如 province_add_building 的 `;`）只在拼接时写出，显示时隐藏。
	let segment_render: Vec<(usize, String, ValuePart, &'static str)> = (0..segment_count)
		.map(|index| {
			let kind = parts
				.map(|parts| parts.get(index).copied().unwrap_or(ValuePart::Plain))
				.unwrap_or(ValuePart::Plain);
			let suffix = part_suffix(kind);
			let raw = segment_values.get(index).cloned().unwrap_or_default();
			let text = if suffix.is_empty() {
				raw
			} else {
				raw.strip_suffix(suffix).unwrap_or(&raw).to_string()
			};
			(index, text, kind, suffix)
		})
		.collect();
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
                        for (index , text , kind , suffix) in segment_render {
                            if index > 0 {
                                span { class: "event-value-sep", "=" }
                            }
                            ValuePartInput {
                                text,
                                kind,
                                suffix,
                                invalid,
                                segment_index: index,
                                full_value: value.clone(),
                                native_autocomplete,
                                lookup: lookup.clone(),
                                images: images.clone(),
                                events: events.clone(),
                                music: music.clone(),
                                on_value,
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
#[component]
pub fn EventGrid(
	event: Signal<Option<MissionEvent>>,
	lookup: Shared<EventLookup>,
	native_autocomplete: bool,
	images: Shared<Vec<String>>,
	events: Shared<Vec<String>>,
	music: Shared<Vec<String>>,
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
        div { class: if native_autocomplete { "event-grid" } else { "event-grid suggest-overlay" },
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
