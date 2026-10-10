//! 决议编辑器（Rainfall Event / `rainfall/rfEvent_decision.json` 的可视化编辑）。
//!
//! 与 `mind.rs`（国策树画布）的分工与边界：
//! - 独立标签页体系：不并入 `open_tabs`，由 `Work` 另持 `decision_tabs` 信号；两类标签
//!   通过统一的标签条渲染、共享 `active_tab_id`（决议标签 id 带 `decision:` 前缀）；
//! - 独立撤销分区：[`crate::app::components::undo::UndoZone::Decisions`]；
//! - 数据模型：决议组列表（`id` / `name` / `desc[]` / `images[]` / `events[]` + 未知字段保留）；
//! - 打开入口：资源管理器双击 `**/rainfall/rfEvent_decision.json`；
//! - 「打开脚本」联动事件编辑器：事件行按钮按候选定位脚本（任意 `…/events/…` /
//!   `…/missionsEvents/…`），缺失时按模板创建到 `events/common` 并打开。
//!
//! 决议系统的数据语义（组/事件/复合键「组id:事件id」）见 `docs/决议系统帮助文档.md`；
//! 后端命令（解析/序列化/定位）见 `src-tauri/src/commands/decisions.rs`。

use dioxus::html::input_data::MouseButton;
use dioxus::html::PointerInteraction as _;
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

use super::platform_fs::resolve_event_script_by_id_text;
use super::undo::{UndoRegistration, UndoScope};
use super::{FocusIcon, Shared, WorkDirectory};
use crate::app::tauri_bridge::{invoke, sleep_ms};

/// 决议标签页 id 前缀（与国策标签 id = json 路径区分，共用 `active_tab_id`）。
pub const DECISION_TAB_PREFIX: &str = "decision:";

/// 决议标签页基础名（全项目唯一一份决议定义文件；显示名经 [`decision_tab_title`]
/// 按目录相对位置加模组前缀，区分同一工作区内的多个模组）。
pub const DECISION_TAB_TITLE: &str = "决议配置";

/// 单个决议组（`rainfall/rfEvent_decision.json` 的 `decisions` 数组项）。
///
/// 与后端 `models::DecisionGroup` 镜像；`extra` 保留工具未注册的未知字段，
/// 保存时原样写回，保证「打开一次再保存」不会把未来扩展键抹掉。
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionGroup {
	#[serde(default)]
	pub id: String,
	#[serde(default)]
	pub name: String,
	#[serde(default)]
	pub desc: Vec<String>,
	#[serde(default)]
	pub images: Vec<String>,
	#[serde(default)]
	pub events: Vec<String>,
	#[serde(flatten)]
	pub extra: BTreeMap<String, serde_json::Value>,
}

impl DecisionGroup {
	/// 新建决议组的默认值（全部为空，由用户填写）。
	pub fn new_empty() -> Self {
		Self {
			id: String::new(),
			name: String::new(),
			desc: Vec::new(),
			images: Vec::new(),
			events: Vec::new(),
			extra: BTreeMap::new(),
		}
	}
}

/// 打开的决议编辑标签页（与 `work.rs` 的 `TreeTab` 平行，互不共享状态）。
#[derive(Clone, PartialEq)]
pub struct DecisionTab {
	/// 标签 id（`decision:<工作区相对路径>`）。
	pub id: String,
	/// 标签显示名。
	pub title: String,
	/// 决议文件的工作区相对路径（如 `assets/rainfall/rfEvent_decision.json`）。
	pub relative_path: String,
	/// 决议组数据（指针共享句柄，供 props 记忆化）。
	pub decisions: Shared<Vec<DecisionGroup>>,
	/// 是否有未保存修改。
	pub dirty: bool,
	/// 保存成功计数（父级递增，驱动清除脏标记）。
	pub save_ack: u64,
}

/// 是否为决议定义文件（资源管理器双击判定；大小写不敏感）。
pub fn is_decision_file(relative_path: &str) -> bool {
	relative_path
		.to_ascii_lowercase()
		.ends_with("rainfall/rfevent_decision.json")
}

/// 由工作区相对路径构造决议标签 id。
pub fn decision_tab_id(relative_path: &str) -> String {
	format!("{DECISION_TAB_PREFIX}{relative_path}")
}

/// 标签 id 是否为决议标签。
pub fn is_decision_tab_id(id: &str) -> bool {
	id.starts_with(DECISION_TAB_PREFIX)
}

/// 决议标签页显示名：按「目录相对位置」加模组前缀，区分同一工作区内的多个模组
///（与国策画布 `tree_tab_title` 的命名方式一致）：
/// - `rainfall/rfEvent_decision.json`（工作区根即模组）→ `决议配置`；
/// - `assets/rainfall/…`（模组自带 assets 层）→ `决议配置`；
/// - `<模组>/assets/rainfall/…` 或 `<模组>/rainfall/…` → `<模组>/决议配置`。
pub fn decision_tab_title(relative_path: &str) -> String {
	let lower = relative_path.to_ascii_lowercase();
	// `rainfall/` 需作为完整目录段命中（前缀为空 = 工作区根即模组）。
	let prefix_len = if lower.starts_with("rainfall/") {
		0
	} else if let Some(index) = lower.rfind("/rainfall/") {
		index
	} else {
		return DECISION_TAB_TITLE.to_string();
	};
	let prefix = relative_path[..prefix_len].trim_end_matches('/');
	// 通用包装层 `assets` 不参与区分（与画布的「全局资源」义同）。
	let prefix = if prefix == "assets" {
		""
	} else {
		prefix.strip_suffix("/assets").unwrap_or(prefix)
	};
	if prefix.is_empty() {
		DECISION_TAB_TITLE.to_string()
	} else {
		format!("{prefix}/{DECISION_TAB_TITLE}")
	}
}

/// 按筛选文本过滤决议组（ID / 名称 / **事件 id** 包含关键字，大小写不敏感；
/// 空文本返回全部索引）。事件匹配用于「找出哪些组引用了某个事件」。
/// 供左侧列表使用——大型模组（近百组）下快速定位。
pub fn filter_group_indices(groups: &[DecisionGroup], query: &str) -> Vec<usize> {
	let query = query.trim().to_lowercase();
	if query.is_empty() {
		return (0..groups.len()).collect();
	}
	groups
		.iter()
		.enumerate()
		.filter(|(_, group)| {
			group.id.to_lowercase().contains(&query)
				|| group.name.to_lowercase().contains(&query)
				|| group
					.events
					.iter()
					.any(|event| event.to_lowercase().contains(&query))
		})
		.map(|(index, _)| index)
		.collect()
}

/// 校验决议数据并收集告警（非阻断：仅提示，不阻止保存）。
///
/// 覆盖：空 id / 重复 id / 空 name / 空事件名 / 同一事件跨组复用
///（运行时键为「组id:事件id」，跨组复用会造成键冲突，官方文档亦建议避免）。
pub fn collect_group_warnings(groups: &[DecisionGroup]) -> Vec<String> {
	let mut warnings = Vec::new();
	let mut seen_ids: HashMap<&str, usize> = HashMap::new();
	let mut event_owner: HashMap<&str, &str> = HashMap::new();
	for (index, group) in groups.iter().enumerate() {
		let id = group.id.trim();
		if id.is_empty() {
			warnings.push(format!(
				"第 {} 个决议组的 id 为空（需填组 ID 才能被 add_decision 解锁）",
				index + 1
			));
		} else if let Some(first) = seen_ids.insert(id, index) {
			warnings.push(format!(
				"决议组 id「{id}」重复（第 {} 与第 {} 个组）",
				first + 1,
				index + 1
			));
		}
		if group.name.trim().is_empty() {
			warnings.push(format!(
				"决议组「{}」的名称为空",
				if id.is_empty() { "（未命名）" } else { id }
			));
		}
		for event in &group.events {
			let event = event.trim();
			if event.is_empty() {
				warnings.push(format!("决议组「{id}」存在空的事件名"));
				continue;
			}
			if let Some(owner) = event_owner.insert(event, id) {
				if owner != id {
					warnings.push(format!(
						"事件「{event}」同时出现在「{owner}」与「{id}」组（建议一个事件只属于一个组）"
					));
				}
			}
		}
	}
	warnings
}

/// 从工作区文件列表提取 `gfx/decision/` 下的图片文件名（排序去重，用于图片候选）。
pub fn decision_image_names(files: &[String]) -> Vec<String> {
	let mut names: Vec<String> = files
		.iter()
		.filter(|path| path.to_ascii_lowercase().ends_with(".png"))
		.filter(|path| {
			let lower = path.to_ascii_lowercase();
			lower.contains("/gfx/decision/") || lower.starts_with("gfx/decision/")
		})
		.map(|path| path.rsplit('/').next().unwrap_or(path).to_string())
		.collect();
	names.sort();
	names.dedup();
	names
}

/// 事件脚本候选：`(显示名, 工作区相对路径)`。
///
/// 覆盖 `missionsEvents`（国策事件）与 `events` 目录（全局 / 剧本事件，决议事件脚本
/// 的常用位置），排序去重；同名脚本出现在多个目录时保留全部（编辑器按路径打开）。
/// `root_prefix` 非空时仅保留该前缀下的脚本（多模组工作区按当前模组限定，见
/// [`decision_root_prefix`]）；空串 = 全部（兼容旧行为 / 测试）。
pub fn event_script_candidates(files: &[String], root_prefix: &str) -> Vec<(String, String)> {
	let prefix = root_prefix.to_ascii_lowercase();
	let mut candidates: Vec<(String, String)> = files
		.iter()
		.filter(|path| path.to_ascii_lowercase().ends_with(".txt"))
		.filter(|path| {
			let lower = path.to_ascii_lowercase();
			(prefix.is_empty() || lower.starts_with(&prefix))
				&& (lower.contains("/events/") || lower.contains("missionsevents/"))
		})
		.map(|path| {
			let file_name = path.rsplit('/').next().unwrap_or(path);
			let name = file_name
				.strip_suffix(".txt")
				.unwrap_or(file_name)
				.to_string();
			(name, path.clone())
		})
		.collect();
	candidates.sort();
	candidates.dedup();
	candidates
}

/// 决议文件所在资源根前缀（`<前缀>rainfall/rfEvent_decision.json` → `<前缀>`，含结尾 `/`；
/// 根级 rainfall → 空串；路径中无 rainfall → `None`）。
/// 多模组工作区中事件脚本候选 / 新建目录按此前缀限定，避免串到其他模组。
pub fn decision_root_prefix(relative_path: &str) -> Option<&str> {
	let lower = relative_path.to_ascii_lowercase();
	let index = if lower.starts_with("rainfall/") {
		0
	} else if let Some(found) = lower.rfind("/rainfall/") {
		found + 1
	} else {
		return None;
	};
	Some(&relative_path[..index])
}

/// 决议图片目录（工作区相对）：`<前缀>rainfall/rfEvent_decision.json` → `<前缀>gfx/decision`。
/// 与后端 `commands::decisions::decision_image_relative_dir` 规则一致（双击定位与 SAF 读取共用）。
pub fn decision_image_dir(relative_path: &str) -> Option<String> {
	decision_root_prefix(relative_path).map(|prefix| format!("{prefix}gfx/decision"))
}

/// 图片缓存键：去掉 `.png` 扩展名（SAF 通道按名称主干读取；桌面通道按原名读取，
/// 缓存同时写入两种键，见面板内的图片加载效果）。
pub fn image_stem(name: &str) -> String {
	let lower = name.to_ascii_lowercase();
	if lower.ends_with(".png") {
		name[..name.len() - 4].to_string()
	} else {
		name.to_string()
	}
}

/// 决议组内条目类型（图片 / 事件；长按菜单、拖拽排序、双击共用一套交互）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum EntryKind {
	Image,
	Event,
}

/// 长按判定：按住超过该时长未移动 → 松手弹出「删除 / 添加」菜单。
const LONG_PRESS_MS: u32 = 450;
/// 越过该位移进入拖拽排序（长按后拖动同样生效；阻止触摸滚动）。
const DRAG_SLOP: f64 = 8.0;
/// 拖拽换行步长的回退值（实际在按下时测量条目真实高度；测量失败时用以下常量）。
const EVENT_ROW_HEIGHT: f64 = 30.0;
const IMAGE_ROW_HEIGHT: f64 = 96.0;
/// 组列表虚拟滚动：可视区上下缓冲与滚动信号节流步长（同资源管理器列表模式；
/// 组行高不定——图片高度、展开状态都会改变行高，故用实测行高缓存定位窗口）。
const GROUP_LIST_BUFFER_PX: f64 = 480.0;
const GROUP_SCROLL_STEP: f64 = 24.0;
/// 窗口后方额外渲染的「探针行」数：未实测行的估值可能远大于真实行高
///（图片纵横比未知时按保守值），会把后续行挡在窗口外永远测不到——逐帧多渲染
/// 若干行并实测回写，窗口自然推进（防止「只显示一个决议组」式的死锁）。
const GROUP_LIST_PROBE_ROWS: usize = 8;
/// 决议图片分片加载的每批数量（首次打开时一次性读入全部图片会瞬时卡顿；
/// 分片 + 进度上报让 UI 保持响应，进度条实时反映加载进度）。
const IMAGE_LOAD_BATCH: usize = 6;
/// 行高实测的滚动静止窗口（毫秒）：滚动进行中暂缓逐行测量，停止后统一补测——
/// 避免快速滑动时「滚动 → 测量 → 重渲染」互相争抢布局。
const MEASURE_IDLE_MS: f64 = 120.0;

/// 条目按下 / 拖拽状态（信号中仅存一份）。
///
/// `token` 区分新旧按下（长按计时器只在令牌一致时生效）；
/// `origin` 为按下时的初始下标，目标位置 = 初始下标 ± 位移步数（避免抖动）。
#[derive(Clone, Copy, PartialEq)]
struct EntryPress {
	kind: EntryKind,
	group: usize,
	/// 当前所在下标（拖动换位时随条目更新）。
	index: usize,
	origin: usize,
	start_y: f64,
	/// 换行步长（按下时测量的条目实际布局高度；图片条目高度随宽度变化）。
	step_height: f64,
	started: bool,
	/// 长按计时器已触发（松手时若未拖动则弹菜单）。
	held: bool,
	/// 触摸指针：长按成立前不进入拖拽（滑动优先原生滚动）；拖拽开始时才捕获指针。
	touch: bool,
	/// 指针 id（触摸拖拽开始时用于延迟捕获指针）。
	pointer_id: i32,
	token: u64,
}

/// 行内编辑草稿（条目）：命中的条目渲染为输入框，保证连续输入不丢焦点；
/// 失焦 / 回车结算：新建且为空 → 删除；重命名为空 → 还原原名；否则保留。
#[derive(Clone, PartialEq)]
struct EntryDraft {
	kind: EntryKind,
	group: usize,
	index: usize,
	/// 由「添加」按钮 / 菜单创建（空值失焦即移除）。
	is_new: bool,
	/// 重命名场景的原值（`None` = 占位/新建）。
	original: Option<String>,
}

/// 行内编辑草稿（决议组名称）：新建组在输入有效名称前不算完成，
/// 空名失焦会被移除；重命名为空则还原原名称。
#[derive(Clone, PartialEq)]
struct GroupDraft {
	index: usize,
	is_new: bool,
	original: Option<String>,
}

/// 条目列表长度（拖拽边界判断）。
fn entry_len(state: &Signal<Vec<DecisionGroup>>, kind: EntryKind, group: usize) -> usize {
	state.peek().get(group).map_or(0, |entry| match kind {
		EntryKind::Image => entry.images.len(),
		EntryKind::Event => entry.events.len(),
	})
}

/// 拖拽排序换位（相邻交换，注册撤销；撤销分区与普通编辑一致）。
fn shift_entry(
	state: Signal<Vec<DecisionGroup>>,
	on_undo_push: EventHandler<UndoRegistration>,
	scope_id: &str,
	kind: EntryKind,
	group: usize,
	from: usize,
	to: usize,
) {
	apply_group_edit(state, on_undo_push, scope_id, move |groups| {
		if let Some(entry) = groups.get_mut(group) {
			let list = match kind {
				EntryKind::Image => &mut entry.images,
				EntryKind::Event => &mut entry.events,
			};
			if from < list.len() && to < list.len() {
				list.swap(from, to);
			}
		}
	});
}

/// 条目增删（长按菜单「删除 / 添加」）：`remove = Some(下标)` 删除，`None` 追加空条目。
fn edit_entries(
	state: Signal<Vec<DecisionGroup>>,
	on_undo_push: EventHandler<UndoRegistration>,
	scope_id: &str,
	kind: EntryKind,
	group: usize,
	remove: Option<usize>,
) {
	apply_group_edit(state, on_undo_push, scope_id, move |groups| {
		if let Some(entry) = groups.get_mut(group) {
			let list = match kind {
				EntryKind::Image => &mut entry.images,
				EntryKind::Event => &mut entry.events,
			};
			if let Some(index) = remove {
				if index < list.len() {
					list.remove(index);
				}
			} else {
				list.push(String::new());
			}
		}
	});
}

/// 取条目当前值（草稿结算读取）。
fn entry_value(state: &Signal<Vec<DecisionGroup>>, draft: &EntryDraft) -> String {
	state
		.peek()
		.get(draft.group)
		.map_or_else(String::new, |group| {
			let slot = match draft.kind {
				EntryKind::Image => group.images.get(draft.index),
				EntryKind::Event => group.events.get(draft.index),
			};
			slot.cloned().unwrap_or_default()
		})
}

/// 聚焦并全选元素（渲染完成后稍候执行；id 已带面板作用域，多标签安全）。
fn focus_element(id: &str) {
	let id = id.to_string();
	spawn(async move {
		sleep_ms(40).await;
		let _ = dioxus::document::eval(&format!(
			"(function(){{ const el = document.getElementById('{id}'); if (el) {{ el.focus(); if (el.select) el.select(); }} }})();"
		))
		.await;
	});
}

/// 结算条目草稿（失焦 / 回车）：空值按「新建删除、重命名还原」处理。
fn finalize_entry_draft(
	state: Signal<Vec<DecisionGroup>>,
	on_undo_push: EventHandler<UndoRegistration>,
	mut draft_signal: Signal<Option<EntryDraft>>,
	scope_id: &str,
) {
	let Some(draft) = draft_signal.peek().clone() else {
		return;
	};
	draft_signal.set(None);
	if !entry_value(&state, &draft).trim().is_empty() {
		return;
	}
	if draft.is_new {
		edit_entries(
			state,
			on_undo_push,
			scope_id,
			draft.kind,
			draft.group,
			Some(draft.index),
		);
	} else if let Some(original) = draft.original.clone() {
		apply_group_edit(state, on_undo_push, scope_id, move |groups| {
			if let Some(entry) = groups.get_mut(draft.group) {
				let slot = match draft.kind {
					EntryKind::Image => entry.images.get_mut(draft.index),
					EntryKind::Event => entry.events.get_mut(draft.index),
				};
				if let Some(slot) = slot {
					*slot = original.clone();
				}
			}
		});
	}
}

/// 移除决议组并收敛选中 / 展开 / 行高缓存与两类草稿（详情按钮与右键菜单共用）。
#[allow(clippy::too_many_arguments)]
fn remove_group(
	state: Signal<Vec<DecisionGroup>>,
	on_undo_push: EventHandler<UndoRegistration>,
	scope_id: &str,
	index: usize,
	mut selected: Signal<usize>,
	mut expanded: Signal<Vec<usize>>,
	mut heights: Signal<HashMap<usize, f64>>,
	mut group_draft: Signal<Option<GroupDraft>>,
	mut entry_draft: Signal<Option<EntryDraft>>,
) {
	apply_group_edit(state, on_undo_push, scope_id, move |groups| {
		if index < groups.len() {
			groups.remove(index);
		}
	});
	let len = state.peek().len();
	selected.with_mut(|value| {
		if *value >= len {
			*value = len.saturating_sub(1);
		} else if *value > index {
			*value -= 1;
		}
	});
	expanded.with_mut(|list| {
		list.retain(|item| *item != index);
		for item in list.iter_mut() {
			if *item > index {
				*item -= 1;
			}
		}
	});
	heights.set(HashMap::new());
	if group_draft
		.peek()
		.as_ref()
		.is_some_and(|draft| draft.index == index)
	{
		group_draft.set(None);
	}
	if entry_draft
		.peek()
		.as_ref()
		.is_some_and(|draft| draft.group == index)
	{
		entry_draft.set(None);
	}
}

/// 结算组名草稿（失焦 / 回车）：新建空名 → 移除该组；重命名空名 → 还原原名称。
#[allow(clippy::too_many_arguments)]
fn finalize_group_draft(
	state: Signal<Vec<DecisionGroup>>,
	on_undo_push: EventHandler<UndoRegistration>,
	mut draft_signal: Signal<Option<GroupDraft>>,
	scope_id: &str,
	selected: Signal<usize>,
	expanded: Signal<Vec<usize>>,
	heights: Signal<HashMap<usize, f64>>,
	entry_draft: Signal<Option<EntryDraft>>,
) {
	let Some(draft) = draft_signal.peek().clone() else {
		return;
	};
	draft_signal.set(None);
	let name = state
		.peek()
		.get(draft.index)
		.map(|group| group.name.clone())
		.unwrap_or_default();
	if !name.trim().is_empty() {
		return;
	}
	if draft.is_new {
		remove_group(
			state,
			on_undo_push,
			scope_id,
			draft.index,
			selected,
			expanded,
			heights,
			draft_signal,
			entry_draft,
		);
	} else if let Some(original) = draft.original.clone() {
		apply_group_edit(state, on_undo_push, scope_id, move |groups| {
			if let Some(group) = groups.get_mut(draft.index) {
				group.name = original.clone();
			}
		});
	}
}

/// 进入拖拽 / 长按：标记触摸滚动拦截（`touchmove` 监听器按需注册一次，见 [`start_drag_lock`]）。
///
/// 注入的文档级 `pointerup` / `pointercancel` 监听是**自愈兑底**：条目在拖拽换位时会
/// 因 key 变化被重建、指针捕获随之失效，若在列表外松手，wasm 侧可能收不到 up →
/// 标记残留会让之后所有触摸滑动都被 `preventDefault`（表现为「无法滑动」）。
/// 任何一次松手 / 取消都无条件清除标记（未拖拽时清除无副作用）。
fn start_drag_lock() {
	spawn(async move {
		let _ = dioxus::document::eval(
			"(function(){ if(!window.__decisionDragSetup){ window.__decisionDragSetup = true; document.addEventListener('touchmove', function(evt){ if(window.__decisionDragging){ evt.preventDefault(); } }, { passive: false }); var release = function(){ window.__decisionDragging = false; }; document.addEventListener('pointerup', release, { capture: true }); document.addEventListener('pointercancel', release, { capture: true }); } window.__decisionDragging = true; })();",
		)
		.await;
	});
}

/// 结束拖拽 / 长按：解除触摸滚动拦截。
fn end_drag_lock() {
	spawn(async move {
		let _ = dioxus::document::eval("window.__decisionDragging = false;").await;
	});
}

/// 条目 DOM id 的类别段（与渲染用 id 保持一致）。
fn entry_dom_tag(kind: EntryKind) -> &'static str {
	match kind {
		EntryKind::Image => "img",
		EntryKind::Event => "evt",
	}
}

/// 列表级兑底：在其他条目 / 空白处松手、或触摸被浏览器取消（开始滚动等）时，
/// 清理可能残留的按下状态与拖拽锁（防「滑动被卡死」——触摸漂移出小元素等边界）。
fn entry_press_cleanup(mut press_state: Signal<Option<EntryPress>>) {
	if press_state.peek().is_some() {
		press_state.set(None);
		end_drag_lock();
	}
}

/// 条目指针按下：记录状态并启动长按计时（超时未移动 → `held`，松手弹菜单）。
/// 触摸不立即捕获指针：让浏览器原生滚动不受干扰（拖拽真正开始时才捕获，
/// 见 [`entry_pointer_move`]）；鼠标按下即捕获（拖动越出条目范围仍能收到事件）。
#[allow(clippy::too_many_arguments)]
fn entry_pointer_down(
	mut press_state: Signal<Option<EntryPress>>,
	mut press_token: Signal<u64>,
	kind: EntryKind,
	group: usize,
	index: usize,
	y: f64,
	step_height: f64,
	capture_id: &str,
	pointer_id: i32,
	is_touch: bool,
) {
	let token = *press_token.peek() + 1;
	press_token.set(token);
	press_state.set(Some(EntryPress {
		kind,
		group,
		index,
		origin: index,
		start_y: y,
		step_height,
		started: false,
		held: false,
		touch: is_touch,
		pointer_id,
		token,
	}));
	if !is_touch {
		let script =
			format!("document.getElementById('{capture_id}')?.setPointerCapture({pointer_id})");
		spawn(async move {
			let _ = dioxus::document::eval(&script).await;
		});
	}
	spawn(async move {
		sleep_ms(LONG_PRESS_MS).await;
		let current = *press_state.peek();
		if let Some(mut current) = current {
			if current.token == token && !current.started && !current.held {
				current.held = true;
				press_state.set(Some(current));
				start_drag_lock();
			}
		}
	});
}

/// 条目指针移动：鼠标越阈即进入拖拽；触摸需先长按成立（否则视为滚动意图，
/// 放弃本次按下并交给浏览器原生滚动——安卓滑动不再被拖拽劫持）。
fn entry_pointer_move(
	mut press_state: Signal<Option<EntryPress>>,
	state: Signal<Vec<DecisionGroup>>,
	on_undo_push: EventHandler<UndoRegistration>,
	scope_id: &str,
	kind: EntryKind,
	group: usize,
	y: f64,
) {
	let Some(mut current) = *press_state.peek() else {
		return;
	};
	if current.kind != kind || current.group != group {
		return;
	}
	let delta = y - current.start_y;
	if !current.started {
		if delta.abs() < DRAG_SLOP {
			return;
		}
		if current.touch && !current.held {
			// 触摸滑动优先滚动：长按成立前的越阈位移 = 滚动意图 → 放弃按下状态
			//（不进入拖拽、不锁 touchmove；浏览器原生长列表滚动接管）。
			press_state.set(None);
			return;
		}
		current.started = true;
		start_drag_lock();
		if current.touch {
			// 触摸按下时未捕获指针（避免影响滚动）；拖拽真正开始时才捕获，
			// 保证手指越出条目范围后仍能收到 move/up。id 用按下时的下标（origin）。
			let script = format!(
				"document.getElementById('{scope_id}-entry-{}-{group}-{}')?.setPointerCapture({})",
				entry_dom_tag(current.kind),
				current.origin,
				current.pointer_id,
			);
			spawn(async move {
				let _ = dioxus::document::eval(&script).await;
			});
		}
	}
	let step_height = current.step_height.max(8.0);
	let len = entry_len(&state, kind, current.group);
	if len == 0 {
		return;
	}
	let target = (current.origin as f64 + (delta / step_height).round())
		.clamp(0.0, (len - 1) as f64) as usize;
	let mut guard = 0;
	while current.index != target && guard < 16 {
		let to = if target > current.index {
			current.index + 1
		} else {
			current.index - 1
		};
		shift_entry(state, on_undo_push, scope_id, kind, current.group, current.index, to);
		current.index = to;
		guard += 1;
	}
	press_state.set(Some(current));
}

/// 条目指针松开：未拖动时长按 → 弹出条目菜单，轻点 → 选中该组。
#[allow(clippy::too_many_arguments)]
fn entry_pointer_up(
	mut press_state: Signal<Option<EntryPress>>,
	mut entry_menu: Signal<Option<(EntryKind, usize, usize, f64, f64)>>,
	mut selected: Signal<usize>,
	kind: EntryKind,
	group: usize,
	x: f64,
	y: f64,
) {
	let Some(current) = *press_state.peek() else {
		return;
	};
	if current.kind != kind || current.group != group {
		return;
	}
	press_state.set(None);
	end_drag_lock();
	if current.started {
		return;
	}
	if current.held {
		entry_menu.set(Some((kind, current.group, current.index, x, y)));
	} else {
		selected.set(group);
	}
}

/// 条目指针取消：清除按下状态。
fn entry_pointer_cancel(
	mut press_state: Signal<Option<EntryPress>>,
	kind: EntryKind,
	group: usize,
) {
	if press_state
		.peek()
		.is_some_and(|current| current.kind == kind && current.group == group)
	{
		press_state.set(None);
		end_drag_lock();
	}
}

/// 切换决议组展开状态（命中则移除、未命中则追加；纯函数便于单测）。
fn toggle_group(list: &mut Vec<usize>, index: usize) {
	if let Some(position) = list.iter().position(|item| *item == index) {
		list.remove(position);
	} else {
		list.push(index);
	}
}

/// 点击组标题：选中该组并切换其事件下拉的展开/收起。
fn toggle_group_expanded(mut expanded: Signal<Vec<usize>>, index: usize) {
	expanded.with_mut(|list| toggle_group(list, index));
}

/// 组行高估值（未实测前）：基础行 + 每个图片条目高度（已知尺寸按分栏内宽与
/// `aspect-ratio` 计算，未知时回退常量）+ 展开的事件区。
fn estimate_group_row(
	group: &DecisionGroup,
	is_expanded: bool,
	sizes: &HashMap<String, (u32, u32)>,
	image_inner_width: f64,
) -> f64 {
	let mut height = 44.0;
	if !group.images.is_empty() {
		height += 10.0;
		for image in &group.images {
			let stem = image_stem(image);
			let image_height = sizes
				.get(&stem)
				.filter(|(width, _)| *width > 0)
				.map(|(width, height)| image_inner_width * (*height as f64) / (*width as f64))
				.unwrap_or(190.0);
			height += image_height.clamp(40.0, 900.0) + 4.0;
		}
	}
	if is_expanded {
		height += 34.0 + group.events.len().max(1) as f64 * 30.0;
	}
	height
}

/// 面板 DOM id 命名空间：多个决议标签同时挂载（非激活只隐藏、不卸载）时，
/// `document.get_element_by_id` 只命中**文档中第一个**同名元素——若不隔离，
/// 第二及以后标签的拖动 / 测量 / 滚动复位都会作用到第一个标签的 DOM
///（表现为「只有第一个标签能拖分割栏」）。标签 id 唯一，净化后作 id 前缀。
fn dom_scope_id(tab_id: &str) -> String {
	let mut scope = String::with_capacity(tab_id.len());
	for ch in tab_id.chars() {
		if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
			scope.push(ch);
		} else {
			scope.push('_');
		}
	}
	scope
}

/// 过滤变化时把列表滚动位置复位（DOM 与信号同步，防止窗口错位）。
/// `list_id` 为带面板作用域的元素 id（见 [`dom_scope_id`]）。
fn reset_group_list_scroll(list_id: &str) {
	let script = format!(
		"(function(){{ const el = document.getElementById('{list_id}'); if (el) el.scrollTop = 0; }})();"
	);
	spawn(async move {
		let _ = dioxus::document::eval(&script).await;
	});
}

/// 按 id 取页面元素（拖动 / 测量用）。
fn element_by_id(id: &str) -> Option<web_sys::Element> {
	web_sys::window()
		.and_then(|window| window.document())
		.and_then(|document| document.get_element_by_id(id))
}

/// 元素实际布局高度（拖拽换行步长；测量失败回退常量）。
fn element_height(id: &str) -> Option<f64> {
	element_by_id(id).map(|element| element.get_bounding_client_rect().height())
}

/// 组列表可视高度实测：高度有效且与当前值差超 1px 才写信号（避免无谓重渲染）。
fn measure_group_list_viewport(list_id: &str, mut viewport: Signal<f64>) {
	if let Some(height) = element_height(list_id) {
		if height > 0.0 && (height - *viewport.peek()).abs() > 1.0 {
			viewport.set(height);
		}
	}
}

/// 页面性能时钟（毫秒；不可用时回退 0）。
fn performance_now() -> f64 {
	web_sys::window()
		.and_then(|window| window.performance())
		.map(|performance| performance.now())
		.unwrap_or(0.0)
}

/// data URL → `blob:` 对象 URL（fetch → Blob → createObjectURL）。
///
/// 成功时把大 base64 字符串从“每帧复制进 DOM 属性”改为固定小 URL；失败
///（如 CSP 限制）返回 None，调用方回退原 data URL。
async fn data_url_to_blob_url(data_url: &str) -> Option<String> {
	let window = web_sys::window()?;
	let response = JsFuture::from(window.fetch_with_str(data_url)).await.ok()?;
	let response = response.dyn_into::<web_sys::Response>().ok()?;
	let blob = JsFuture::from(response.blob().ok()?).await.ok()?;
	let blob = blob.dyn_into::<web_sys::Blob>().ok()?;
	web_sys::Url::create_object_url_with_blob(&blob).ok()
}

/// 从 data URL 反解 PNG 宽高（只解析 base64 头部 32 字符 → 24 字节，足以覆盖
/// 签名与 IHDR；SAF 通道不返回尺寸时的兑底）。
fn png_dims_from_data_url(data_url: &str) -> Option<(u32, u32)> {
	let payload = data_url.strip_prefix("data:image/png;base64,")?;
	let chunk = payload.as_bytes().get(..32)?;
	const ALPHABET: &[u8; 64] =
		b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
	let mut values = [0_u8; 32];
	for (index, byte) in chunk.iter().enumerate() {
		values[index] = ALPHABET.iter().position(|candidate| candidate == byte)? as u8;
	}
	let mut bytes = [0_u8; 24];
	for group in 0..8 {
		let v0 = values[group * 4] as u16;
		let v1 = values[group * 4 + 1] as u16;
		let v2 = values[group * 4 + 2] as u16;
		let v3 = values[group * 4 + 3] as u16;
		bytes[group * 3] = ((v0 << 2) | (v1 >> 4)) as u8;
		bytes[group * 3 + 1] = ((v1 << 4) | (v2 >> 2)) as u8;
		bytes[group * 3 + 2] = ((v2 << 6) | v3) as u8;
	}
	if bytes[..8] != [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]
		|| &bytes[12..16] != b"IHDR"
	{
		return None;
	}
	let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
	let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
	Some((width, height))
}

/// 决议组侧栏元素（本标签页 `#{scope}-groups-pane`；多标签隔离见 [`dom_scope_id`]）。
fn pane_element(scope: &str) -> Option<web_sys::Element> {
	element_by_id(&format!("{scope}-groups-pane"))
}

/// 拖动分栏：直接写 DOM 宽度——逐帧写 `groups_width` 信号会重建整棵近百分组列表
/// （拖动卡顿主因）；松手时再把最终值提交回信号触发一次重渲染。
fn set_pane_width(scope: &str, width: f64) {
	if let Some(element) = pane_element(scope) {
		let _ = element.set_attribute("style", &format!("width: {width}px;"));
	}
}

/// 拖动中标记（CSS 光标 / 禁选；同样直接操作 DOM 类名）。
fn set_pane_resizing(scope: &str, active: bool) {
	if let Some(element) = pane_element(scope) {
		let classes = element.class_list();
		let _ = if active {
			classes.add_1("resizing")
		} else {
			classes.remove_1("resizing")
		};
	}
}

/// 结束拖动：提交最终宽度并清理状态标记。
fn end_pane_resize(
	scope: &str,
	state: &std::rc::Rc<std::cell::Cell<Option<(f64, f64, f64)>>>,
	mut groups_width: Signal<f64>,
) {
	if let Some((_, _, width)) = state.get() {
		groups_width.set(width);
	}
	state.set(None);
	set_pane_resizing(scope, false);
}

/// 桌面端决议图片批量读取参数（与后端 `load_decision_images` 对应）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DecisionImagesArgs {
	work_directory: String,
	relative_path: String,
	names: Vec<String>,
}

/// 决议图片预览缓存项：显示 URL（优先 `blob:` 本地对象 URL）+ 原始像素尺寸。
///
/// 大图不以 data URL 直接进 DOM——每张只转换一次 blob URL，渲染时属性值从
/// 数百 KB 的 base64 降为几十字节的稳定字符串（大图列表滚动的关键优化）。
#[derive(Clone, PartialEq)]
struct PreviewImage {
	url: String,
	width: u32,
	height: u32,
}

/// `load_decision_images` 返回项（桌面端；SAF 通道由 data URL 头部兑底解析尺寸）。
#[derive(Deserialize)]
struct DecisionImageResponse {
	name: String,
	data_url: String,
	#[serde(default)]
	width: u32,
	#[serde(default)]
	height: u32,
}

/// `probe_decision_images` 返回项（只探测头部尺寸，不读图片正文）。
#[derive(Deserialize)]
struct DecisionImageSizeResponse {
	name: String,
	#[serde(default)]
	width: u32,
	#[serde(default)]
	height: u32,
}

/// SAF（Android）目录批量读取参数（复用「读取目录内文件」命令，按名称主干匹配）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScopedReadFilesArgs {
	folder_id: String,
	dir_path: String,
	names: Vec<String>,
}

/// 批量加载 `gfx/decision/` 图片（缺失项静默跳过 → 前端保留占位符）。
///
/// 返回宽高供 `aspect-ratio` 预留布局；SAF 通道由 data URL 头部兑底解析。
async fn load_decision_images(
	directory: &WorkDirectory,
	relative_path: &str,
	names: &[String],
) -> Result<Vec<DecisionImageResponse>, String> {
	if let Some(folder_id) = &directory.folder_id {
		let Some(image_dir) = decision_image_dir(relative_path) else {
			return Ok(Vec::new());
		};
		let dir_path = if directory.root_path.is_empty() {
			image_dir
		} else {
			format!("{}/{}", directory.root_path, image_dir)
		};
		// SAF 通道按名称主干（无扩展名）匹配 `<主干>.png`。
		let stems: Vec<String> = names.iter().map(|name| image_stem(name)).collect();
		let args = serde_wasm_bindgen::to_value(&ScopedReadFilesArgs {
			folder_id: folder_id.clone(),
			dir_path,
			names: stems,
		})
		.map_err(|error| error.to_string())?;
		let value = JsFuture::from(invoke("read_scoped_files_in_dir", args))
			.await
			.map_err(|error| format!("读取决议图片失败：{error:?}"))?;
		let icons: Vec<FocusIcon> = serde_wasm_bindgen::from_value(value)
			.map_err(|error| format!("决议图片数据格式错误：{error}"))?;
		Ok(icons
			.into_iter()
			.map(|icon| {
				let (width, height) =
					png_dims_from_data_url(&icon.data_url).unwrap_or((0, 0));
				DecisionImageResponse {
					name: icon.name,
					data_url: icon.data_url,
					width,
					height,
				}
			})
			.collect())
	} else {
		let args = serde_wasm_bindgen::to_value(&DecisionImagesArgs {
			work_directory: directory.root_path.clone(),
			relative_path: relative_path.to_string(),
			names: names.to_vec(),
		})
		.map_err(|error| error.to_string())?;
		let value = JsFuture::from(invoke("load_decision_images", args))
			.await
			.map_err(|error| format!("读取决议图片失败：{error:?}"))?;
		serde_wasm_bindgen::from_value(value)
			.map_err(|error| format!("决议图片数据格式错误：{error}"))
	}
}

/// 只探测图片尺寸（只读 PNG 头部，不传图片正文）。
///
/// 前端在批量加载前先探测一遍：占位符与图片拥有同一宽高比，加载完成
/// 不再改变行高（SAF 通道不适用 → 返回空表，由 data URL 头部兑底）。
async fn probe_decision_images(
	directory: &WorkDirectory,
	relative_path: &str,
	names: &[String],
) -> Result<Vec<DecisionImageSizeResponse>, String> {
	if directory.folder_id.is_some() {
		return Ok(Vec::new());
	}
	let args = serde_wasm_bindgen::to_value(&DecisionImagesArgs {
		work_directory: directory.root_path.clone(),
		relative_path: relative_path.to_string(),
		names: names.to_vec(),
	})
	.map_err(|error| error.to_string())?;
	let value = JsFuture::from(invoke("probe_decision_images", args))
		.await
		.map_err(|error| format!("探测决议图片尺寸失败：{error:?}"))?;
	serde_wasm_bindgen::from_value(value)
		.map_err(|error| format!("决议图片尺寸数据格式错误：{error}"))
}

/// 「打开或创建脚本」共用逻辑（事件条目双击）：
/// 1) 候选按**文件名主干**命中 → 直接打开；
/// 2) 未命中 → 按脚本内部 **id** 解析（决议 `events` 字段索引的是 id，很多模组
///    文件名与 id 不同；见后端 `resolve_event_script_by_id`，按需扫描 + 会话缓存）；
/// 3) 仍未找到 → 交给 Work 侧：源 APK 热导入（按文件名）→ 模板创建。
/// 候选与目录均由调用方按 [`decision_root_prefix`] 限定到决议文件所在模组。
#[allow(clippy::too_many_arguments)]
pub async fn open_or_create_script(
	name: &str,
	scripts: &[(String, String)],
	root_prefix: &str,
	mod_dir: &str,
	work_directory: Option<WorkDirectory>,
	mut message: Signal<String>,
	on_open: EventHandler<(String, String)>,
	on_open_missing: EventHandler<(String, String, String, String)>,
) {
	if let Some(path) = find_script_path(scripts, name) {
		if let Some((dir, file_name)) = path.rsplit_once('/') {
			on_open.call((dir.to_string(), file_name.to_string()));
		} else {
			message.set(format!("脚本路径无效：{path}"));
		}
		return;
	}
	// 按内部 id 解析（文件名与 id 不一致的模组）。
	if let Some(directory) = work_directory.as_ref() {
		let paths: Vec<String> = scripts.iter().map(|(_, path)| path.clone()).collect();
		if let Some(path) = resolve_event_script_by_id_text(directory, &paths, name).await {
			if let Some((dir, file_name)) = path.rsplit_once('/') {
				on_open.call((dir.to_string(), file_name.to_string()));
				return;
			}
		}
	}
	// 优先模组内已存在的 `events/common` / missions 推导目录；
	// 模组内没有任何脚本时回退到推断的全局事件目录（热导入 / 创建时自动建目录）。
	let dir = default_decision_script_dir(scripts)
		.unwrap_or_else(|| mod_script_fallback_dir(root_prefix));
	on_open_missing.call((
		mod_dir.to_string(),
		format!("{name}.txt"),
		dir,
		decision_script_template(name),
	));
}

/// 列表上移（越界不动）。
pub fn list_move_up(list: &mut [String], index: usize) {
	if index > 0 && index < list.len() {
		list.swap(index - 1, index);
	}
}

/// 在事件脚本候选中按名称查找路径（文件名去 `.txt` 与事件名完全相等）。
/// 候选中同名脚本按路径序取第一个。
pub fn find_script_path(candidates: &[(String, String)], event_name: &str) -> Option<String> {
	let name = event_name.trim();
	if name.is_empty() {
		return None;
	}
	candidates
		.iter()
		.find(|(candidate, _)| candidate == name)
		.map(|(_, path)| path.clone())
}

/// 决议脚本的默认目标目录：
/// 1. 已存在的 `…/events/common`（路径序第一个）；
/// 2. 由候选脚本路径中的 missions 根推导 `<父目录>/events/common`
///    （与引擎加载目录一致：全局 = `<assets>/game/events/common`，剧本 = `<剧本>/events/common`）。
pub fn default_decision_script_dir(candidates: &[(String, String)]) -> Option<String> {
	let mut commons: Vec<String> = candidates
		.iter()
		.filter_map(|(_, path)| {
			let (dir, _) = path.rsplit_once('/')?;
			if dir.to_ascii_lowercase().ends_with("/events/common") {
				Some(dir.to_string())
			} else {
				None
			}
		})
		.collect();
	commons.sort();
	commons.dedup();
	if let Some(dir) = commons.first() {
		return Some(dir.clone());
	}
	let mut bases: Vec<String> = Vec::new();
	for (_, path) in candidates {
		let lower = path.to_ascii_lowercase();
		let parent = if let Some(index) = lower.find("/missions/") {
			&path[..index]
		} else if lower.starts_with("missions/") {
			""
		} else {
			continue;
		};
		bases.push(if parent.is_empty() {
			"events/common".to_string()
		} else {
			format!("{parent}/events/common")
		});
	}
	bases.sort();
	bases.dedup();
	bases.first().cloned()
}

/// 无任何候选脚本时的新建目录回退（标准布局自动落到引擎全局事件目录）：
/// 前缀为空（根级 rainfall）或以 `assets/` 结尾（模组资源根）→ `<前缀>game/events/common`；
/// 其余（如剧本子目录）→ `<前缀>events/common`。
pub fn mod_script_fallback_dir(root_prefix: &str) -> String {
	let lower = root_prefix.to_ascii_lowercase();
	if root_prefix.is_empty() || lower.ends_with("assets/") {
		format!("{root_prefix}game/events/common")
	} else {
		format!("{root_prefix}events/common")
	}
}

/// 决议事件脚本模板：决议必需字段 + 一个永远满足的触发器 + 一个空效果选项。
/// 说明：空触发器在引擎侧恒为 false（决议无法开始 / 无法完成），
/// 因此模板预置 `is_player=true`，按需修改。
pub fn decision_script_template(event_name: &str) -> String {
	format!(
		"id={event_name}\ntitle={event_name}\ndesc=\npossible_to_run=false\ndecision_dura=10\nonly_once=true\nrun_in_background=false\n\ntrigger_and\nis_player=true\ntrigger_and_end\n\noption_btn\nai=100\nname=好的\noption_end\n"
	)
}

/// 列表下移（越界不动）。
pub fn list_move_down(list: &mut [String], index: usize) {
	if index + 1 < list.len() {
		list.swap(index, index + 1);
	}
}

/// 应用一次决议数据编辑：快照 → 修改 → 无变化不注册，有变化注册撤销（分区
/// [`UndoScope::DecisionFile`]，连续输入在 Work 侧 800ms 窗口合并）。
fn apply_group_edit(
	mut state: Signal<Vec<DecisionGroup>>,
	on_undo_push: EventHandler<UndoRegistration>,
	scope_id: &str,
	mutate: impl FnOnce(&mut Vec<DecisionGroup>),
) {
	let before = state.read().clone();
	state.with_mut(|groups| mutate(groups));
	let after = state.read().clone();
	if after == before {
		return;
	}
	let mut state_undo = state;
	let mut state_redo = state;
	let undo = EventHandler::new(move |_: ()| state_undo.set(before.clone()));
	let redo = EventHandler::new(move |_: ()| state_redo.set(after.clone()));
	on_undo_push.call((UndoScope::DecisionFile(scope_id.to_string()), undo, redo));
}

/// 单个决议编辑器面板：以 key 挂载保持编辑状态，非激活时隐藏而非卸载。
#[component]
pub fn DecisionPanel(
	tab: DecisionTab,
	active_tab_id: Signal<Option<String>>,
	save_request: Signal<u64>,
	files: Shared<Vec<String>>,
	/// 保存回调：`(标签 id, 决议组快照)`。
	on_save: EventHandler<(String, Vec<DecisionGroup>)>,
	/// 脏状态上报：`(标签 id, 是否有未保存修改)`。
	on_dirty_change: EventHandler<(String, bool)>,
	on_undo_push: EventHandler<UndoRegistration>,
	/// 「打开脚本」：`(事件目录, 文件名)`——目录为工作区相对完整目录。
	on_open_script: EventHandler<(String, String)>,
	/// 缺失脚本双击：`(模组目录, 文件名, 回退目录, 模板内容)`——
	/// Work 侧先尝试从源 APK 热导入该单条脚本，未收录再按模板创建并打开。
	on_open_missing_script: EventHandler<(String, String, String, String)>,
	/// 当前工作区（决议图片读取；SAF 模式走 `read_scoped_files_in_dir`）。
	work_directory: Option<WorkDirectory>,
	/// 双击图片条目：`(工作区相对路径, 是否为目录)`——在内置资源管理器中定位。
	on_reveal_item: EventHandler<(String, bool)>,
) -> Element {
	let is_active = active_tab_id.read().as_deref() == Some(tab.id.as_str());
	// 面板 DOM 作用域：多标签同时挂载时隔离同名元素（见 [`dom_scope_id`]）。
	let dom_scope = dom_scope_id(&tab.id);
	// `move` 闭包 / effect 各持一份克隆（原始 String 留给 rsx 属性格式化用）。
	let scope_viewport = dom_scope.clone();
	let scope_measure = dom_scope.clone();
	let scope_filter = dom_scope.clone();
	let scope_filter_clear = dom_scope.clone();
	let scope_handle_down = dom_scope.clone();
	let scope_handle_move = dom_scope.clone();
	let scope_handle_up = dom_scope.clone();
	let scope_handle_cancel = dom_scope.clone();
	let scope_add_group = dom_scope.clone();
	let scope_add_image = dom_scope.clone();
	let scope_add_event = dom_scope.clone();
	let scope_entry_menu = dom_scope.clone();
	let scope_group_menu = dom_scope.clone();
	let state = use_signal(|| (*tab.decisions).clone());
	let mut initial = use_signal(|| (*tab.decisions).clone());
	let mut selected = use_signal(|| 0_usize);
	// 左侧组列表筛选文本（大型模组近百组时快速定位）。
	let mut group_filter = use_signal(String::new);
	// 组列表虚拟滚动（同资源管理器列表模式）：仅渲染可视窗口 + 上下缓冲，
	// 百余组 / 多图片时保持流畅；行高实测缓存（图片与展开使行高不定）。
	let mut group_list_scroll = use_signal(|| 0.0_f64);
	let group_list_viewport = use_signal(|| 480.0_f64);
	let mut group_row_heights = use_signal(HashMap::<usize, f64>::new);
	// 条目操作的行内提示（图片加载失败 / 脚本定位与创建失败）。
	let mut message = use_signal(String::new);
	// 左右分栏宽度（分隔条拖动，px；限制 150..=640 保证两侧可用）。
	let groups_width = use_signal(|| 236.0_f64);
	// 拖动状态用非响应式存储：逐帧写信号会触发整棵分组列表重渲染（卡顿主因），
	// 拖动期间只直接改 DOM，松手才提交到 `groups_width`。
	// 元组含义：`(按下时的指针 x, 按下时的起始宽度, 最新宽度)`——位移必须以
	// 起始宽度为基准重算，拖动期间只刷新「最新宽度」，不得改写基准
	//（否则每帧位移被累计、分割栏会跑得比指针快）。
	let resize_state = use_hook(|| {
		std::rc::Rc::new(std::cell::Cell::new(None::<(f64, f64, f64)>))
	});
	let resize_for_down = resize_state.clone();
	let resize_for_move = resize_state.clone();
	let resize_for_up = resize_state.clone();
	let resize_for_cancel = resize_state;
	// 已展开（显示事件下拉）的组下标集合（切换走 `toggle_group_expanded`）。
	let expanded_groups = use_signal(Vec::<usize>::new);
	// 条目按下 / 拖拽状态与长按令牌（详见 [`EntryPress`]）。
	let press_state = use_signal(|| None::<EntryPress>);
	let press_token = use_signal(|| 0_u64);
	// 长按松手弹出的条目菜单：`(类型, 组下标, 条目下标, 屏幕 x, 屏幕 y)`。
	let mut entry_menu = use_signal(|| None::<(EntryKind, usize, usize, f64, f64)>);
	// 决议组右键菜单：`(组下标, 屏幕 x, 屏幕 y)`（重命名 / 删除）。
	let mut group_menu = use_signal(|| None::<(usize, f64, f64)>);
	// 行内编辑草稿：命中的行渲染为输入框（新建自动聚焦，空值失焦自动移除 / 还原）。
	let mut entry_draft = use_signal(|| None::<EntryDraft>);
	let mut group_draft = use_signal(|| None::<GroupDraft>);
	// 决议图片缓存（名称主干 → 预览项）与已尝试集合（缺失图片不反复请求）。
	let image_data = use_signal(HashMap::<String, PreviewImage>::new);
	let mut image_attempted = use_signal(Vec::<String>::new);
	// 图片尺寸缓存（名称主干 → 像素宽高；头部探测得到，占位符按 aspect-ratio 预留高度）。
	let image_sizes = use_signal(HashMap::<String, (u32, u32)>::new);
	// 图片加载进度：`None` 空闲；`Some((已完成, 总数))` 正在加载（驱动头部进度条）。
	let mut image_load_progress = use_signal(|| None::<(usize, usize)>);
	// 由 data URL 转换出的 blob 对象 URL（面板卸载时统一 revoke 释放内存）。
	let created_blob_urls =
		use_hook(|| std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new())));
	{
		let created_blob_urls = created_blob_urls.clone();
		use_drop(move || {
			for url in created_blob_urls.borrow().iter() {
				let _ = web_sys::Url::revoke_object_url(url);
			}
			created_blob_urls.borrow_mut().clear();
		});
	}

	// 候选列表（工作区文件列表变化时才重算；Shared 指针比较避免无谓重渲染）。
	let image_candidates = use_memo(use_reactive((&files,), move |(files,)| {
		Shared::new(decision_image_names(&files))
	}));
	// 决议文件所在模组前缀：事件脚本候选限定在当前模组内（多模组工作区互不串扰）。
	let mod_prefix = decision_root_prefix(&tab.relative_path)
		.unwrap_or_default()
		.to_string();
	// 模组目录名（事件热导入定位 `<模组>/.ageciv-source` 用；根级为空串）。
	let mod_dir = mod_prefix
		.split('/')
		.find(|segment| !segment.is_empty())
		.unwrap_or_default()
		.to_string();
	let mod_prefix_for_events = mod_prefix.clone();
	let event_candidates = use_memo(use_reactive((&files,), move |(files,)| {
		Shared::new(event_script_candidates(&files, &mod_prefix_for_events))
	}));

	// 图片懒加载：收集组内引用的图片名（主干去重）→ 先探测尺寸（aspect-ratio 预留
	// 行高）→ 分片读取 `gfx/decision/`；缺失 / 失败静默保留占位符。
	// 大图不以 data URL 直接进 DOM：每张转换一次 `blob:` 对象 URL（把数百 KB 的
	// base64 从「每帧属性复制」降为几十字节的稳定 URL），转换失败回退原 data URL。
	// `work_directory` 是普通 props，需经 use_reactive 声明依赖。
	let relative_path_for_effect = tab.relative_path.clone();
	use_effect(use_reactive((&work_directory,), move |(work_directory,)| {
		// 订阅进度信号：复位（None）时重跑本效果，接力处理加载期间新增的图片名。
		let _ = image_load_progress.read();
		let mut missing: Vec<String> = Vec::new();
		{
			let groups = state.read();
			let cache = image_data.read();
			let attempted = image_attempted.read();
			for group in groups.iter() {
				for image in group.images.iter() {
					let stem = image_stem(image);
					if stem.is_empty()
						|| cache.contains_key(&stem)
						|| attempted.contains(&stem)
						|| missing.contains(&stem)
					{
						continue;
					}
					missing.push(stem);
				}
			}
		}
		// 已有加载进行中时不重复发起；其结束后进度复位会再次触发本效果自动接力。
		if missing.is_empty() || image_load_progress.peek().is_some() {
			return;
		}
		let Some(directory) = work_directory else {
			return;
		};
		image_attempted.with_mut(|attempted| attempted.extend(missing.iter().cloned()));
		let relative_path = relative_path_for_effect.clone();
		let mut image_data = image_data;
		let mut image_sizes = image_sizes;
		let mut message = message;
		let created_blob_urls = created_blob_urls.clone();
		let total = missing.len();
		image_load_progress.set(Some((0, total)));
		spawn(async move {
			// 1) 头部探测尺寸：占位符与图片拥有同一宽高比，加载完成不再改变行高。
			if let Ok(sizes) =
				probe_decision_images(&directory, &relative_path, &missing).await
			{
				image_sizes.with_mut(|map| {
					for item in sizes {
						if item.width > 0 && item.height > 0 {
							map.insert(image_stem(&item.name), (item.width, item.height));
							map.insert(item.name, (item.width, item.height));
						}
					}
				});
			}
			// 2) 分片加载 + 逐张 blob 化（转换失败回退 data URL）。
			let mut completed = 0_usize;
			for chunk in missing.chunks(IMAGE_LOAD_BATCH) {
				match load_decision_images(&directory, &relative_path, chunk).await {
					Ok(loaded) => {
						let mut previews: Vec<(String, String, PreviewImage)> = Vec::new();
						for icon in loaded {
							let (width, height) = if icon.width > 0 && icon.height > 0 {
								(icon.width, icon.height)
							} else {
								png_dims_from_data_url(&icon.data_url).unwrap_or((0, 0))
							};
							let url = match data_url_to_blob_url(&icon.data_url).await {
								Some(url) => {
									created_blob_urls.borrow_mut().push(url.clone());
									url
								}
								None => icon.data_url.clone(),
							};
							previews.push((
								image_stem(&icon.name),
								icon.name,
								PreviewImage { url, width, height },
							));
						}
						image_data.with_mut(|cache| {
							for (stem, name, preview) in previews {
								cache.insert(stem, preview.clone());
								cache.insert(name, preview);
							}
						});
					}
					Err(error) => {
						message.set(format!("决议图片加载失败：{error}"));
						break;
					}
				}
				completed += chunk.len();
				image_load_progress.set(Some((completed, total)));
			}
			image_load_progress.set(None);
		});
	}));

	// 保存请求消费（仅激活标签页落盘；非激活仅标记已消费，避免切换标签时被静默保存）。
	let mut last_save_request = use_signal(|| *save_request.read());
	let mut pending_save_snapshot = use_signal(|| None::<Vec<DecisionGroup>>);
	let tab_id_for_request = tab.id.clone();
	use_effect(move || {
		let request = *save_request.read();
		if request == *last_save_request.read() {
			return;
		}
		last_save_request.set(request);
		let is_active = active_tab_id.read().as_deref() == Some(tab_id_for_request.as_str());
		if !is_active {
			return;
		}
		// 记录「发起保存时」的快照：保存成功后以它为已落盘基准（见 save_ack effect）。
		let snapshot = state.read().clone();
		pending_save_snapshot.set(Some(snapshot.clone()));
		on_save.call((tab_id_for_request.clone(), snapshot));
	});

	// 保存成功（父级递增 save_ack）后，以「发起保存时的快照」为新基准清除脏标记。
	// save_ack 是普通 props——必须经 use_reactive 声明为依赖才会触发重跑（dioxus 0.7 经验）。
	use_effect(use_reactive((&tab.save_ack,), move |(_ack,)| {
		let snapshot = pending_save_snapshot.peek().clone();
		if let Some(snapshot) = snapshot {
			pending_save_snapshot.set(None);
			initial.set(snapshot);
		}
	}));

	// 数据与基准不一致时向父级上报脏状态（关闭标签页前提示用）。
	let mut last_dirty = use_signal(|| false);
	let tab_id_for_dirty = tab.id.clone();
	use_effect(move || {
		let dirty = *state.read() != *initial.read();
		if dirty == *last_dirty.read() {
			return;
		}
		last_dirty.set(dirty);
		on_dirty_change.call((tab_id_for_dirty.clone(), dirty));
	});

	let groups = state.read().clone();
	let warnings = collect_group_warnings(&groups);
	let scope_id = use_signal(|| tab.id.clone());
	// 组列表筛选（空查询返回全部；`is_filtering` 控制计数与清除按钮显示）。
	let group_filter_text = group_filter.read().clone();
	let is_filtering = !group_filter_text.trim().is_empty();
	let visible_indices = filter_group_indices(&groups, &group_filter_text);
	// 图片目录（一次计算，供各图片条目的双击定位使用）。
	let image_dir = decision_image_dir(&tab.relative_path);
	// 渲染期间持有图片缓存读锁：`src` 直接借用 data URL，避免每个条目重复克隆
	// 大体积 base64 字符串（近百分组 + 多图时是展开/拖动的主要开销来源）。
	let sidebar_images = image_data.read();
	// 图片尺寸（头部探测结果）：占位符按 aspect-ratio 预留与加载后一致的高度。
	let sidebar_sizes = image_sizes.read();

	// ===== 组列表虚拟滚动窗口 =====
	// 行高：实测缓存优先（图片解码 / 展开收起会改变行高），未实测时按内容估值。
	let expanded_snapshot: std::collections::HashSet<usize> =
		expanded_groups.read().iter().copied().collect();
	// 行内编辑草稿快照（行循环内比较；避免逐行读锁）。
	let entry_draft_snapshot = entry_draft.read().clone();
	let group_draft_snapshot = group_draft.read().clone();
	let list_len = visible_indices.len();
	let row_heights: Vec<f64> = {
		let heights_guard = group_row_heights.read();
		let sizes_guard = image_sizes.read();
		// 图片条目内宽 = 分栏宽 - 容器左右 padding/边框（估行高用）。
		let image_inner_width = (*groups_width.read() - 22.0).max(60.0);
		(0..list_len)
			.map(|position| {
				let index = visible_indices[position];
				heights_guard.get(&position).copied().unwrap_or_else(|| {
					estimate_group_row(
						&groups[index],
						expanded_snapshot.contains(&index),
						&sizes_guard,
						image_inner_width,
					)
				})
			})
			.collect()
	};
	let scroll_top = *group_list_scroll.read();
	let start_px = (scroll_top - GROUP_LIST_BUFFER_PX).max(0.0);
	let end_px = scroll_top + *group_list_viewport.read() + GROUP_LIST_BUFFER_PX;
	let (mut window_start, mut window_end) = (0_usize, 0_usize);
	let (mut top_spacer, mut total_height) = (0.0_f64, 0.0_f64);
	for (position, height) in row_heights.iter().enumerate() {
		let next = total_height + height;
		if next <= start_px {
			window_start = position + 1;
			top_spacer = next;
		}
		if total_height <= end_px {
			window_end = position + 1;
		}
		total_height = next;
	}
	if list_len > 0 && window_end <= window_start {
		window_start = window_start.min(list_len - 1);
		window_end = window_start + 1;
		top_spacer = row_heights[..window_start].iter().sum();
	}
	// 渲染范围 = 窗口 + 探针行（见 GROUP_LIST_PROBE_ROWS）：多渲染几行供实测回写，
	// 未实测行估值过大时窗口也能逐帧推进而不是卡死。
	let render_end = (window_end + GROUP_LIST_PROBE_ROWS).min(list_len);
	// 探针行渲染在窗口下方：底部占位从「渲染范围末尾」起算（其后为未渲染部分估值）。
	let render_end_px = row_heights.iter().take(render_end).sum::<f64>();
	let bottom_spacer = (total_height - render_end_px).max(0.0);
	// 可视窗口高度实测：挂载后修正默认值。
	// 组数变化（数据加载完成 / 新建 / 删除）后重测并延时补测一次——首次挂载或
	// 大模组导入后可能量到布局未稳定的极小值，若无重测机会，窗口只含一行、
	// 列表表现为「只显示一个决议组」。
	let group_count = use_memo(move || state.read().len());
	let last_viewport_count =
		use_hook(|| std::rc::Rc::new(std::cell::Cell::new(usize::MAX)));
	use_effect(move || {
		let _ = group_list_scroll.read();
		let count = *group_count.read();
		let list_id = format!("{scope_viewport}-groups-list");
		measure_group_list_viewport(&list_id, group_list_viewport);
		if last_viewport_count.replace(count) != count {
			let viewport = group_list_viewport;
			spawn(async move {
				sleep_ms(150).await;
				measure_group_list_viewport(&list_id, viewport);
			});
		}
	});
	// 行高实测回写：窗口内逐行测量，与缓存差超阈值才写入
	//（信号更新触发重渲染 → 偏移校正，直至收敛）。
	// 滚动活跃度（performance.now；滚动期间暂缓行高实测，停止后再补测）。
	let scroll_activity =
		use_hook(|| std::rc::Rc::new(std::cell::Cell::new(-10_000.0_f64)));
	let measure_pending = use_hook(|| std::rc::Rc::new(std::cell::Cell::new(false)));
	let scroll_activity_measure = scroll_activity.clone();
	let measure_epoch = use_signal(|| 0_u32);
	let measured_window = use_hook(|| std::rc::Rc::new(std::cell::Cell::new((0_usize, 0_usize))));
	// 实测范围 = 渲染范围（窗口 + 探针行，见 `GROUP_LIST_PROBE_ROWS`）
	// `render_end` 在窗口计算处已限定在列表长度内。
	measured_window.set((window_start, render_end));
	use_effect(move || {
		let _ = group_list_scroll.read();
		let _ = state.read();
		let _ = image_data.read();
		let _ = expanded_groups.read();
		let _ = group_list_viewport.read();
		let _ = group_row_heights.read();
		let _ = measure_epoch.read();
		// 快速滑动/滚轮滚动期间不逐行测量（避免与滚动争抢布局），
		// 调度一次补测：滚动静止后写 `measure_epoch` 唤醒本 effect。
		if performance_now() - scroll_activity_measure.get() < MEASURE_IDLE_MS {
			if !measure_pending.replace(true) {
				let pending = measure_pending.clone();
				let mut epoch = measure_epoch;
				spawn(async move {
					sleep_ms(MEASURE_IDLE_MS as u32 + 30).await;
					pending.set(false);
					let next = *epoch.peek() + 1;
					epoch.set(next);
				});
			}
			return;
		}
		let (start, end) = measured_window.get();
		let Some(document) = web_sys::window().and_then(|window| window.document()) else {
			return;
		};
		for position in start..end {
			let Some(element) =
				document.get_element_by_id(&format!("{scope_measure}-group-row-{position}"))
			else {
				continue;
			};
			let height = element.get_bounding_client_rect().height();
			let cached = group_row_heights.peek().get(&position).copied();
			if cached.map_or(true, |cached| (cached - height).abs() > 0.5) {
				group_row_heights.with_mut(|heights| {
					heights.insert(position, height);
				});
			}
		}
	});

	// 图片加载进度（驱动头部进度条；空闲时不渲染）。
	let image_progress = *image_load_progress.read();
	let image_progress_percent = image_progress.map_or(0.0, |(completed, total)| {
		if total > 0 {
			completed as f64 / total as f64 * 100.0
		} else {
			100.0
		}
	});

	rsx! {
        div {
            class: "decision-stage",
            style: if is_active { "display: flex;" } else { "display: none;" },
            // 候选列表（图片名 / 事件脚本名），供占位条目行内输入与「添加」补全。
            datalist { id: "{dom_scope}-image-candidates",
                for name in image_candidates.read().iter() {
                    option { key: "img-{name}", value: "{name}" }
                }
            }
            datalist { id: "{dom_scope}-event-candidates",
                for (name , _path) in event_candidates.read().iter().take(1000) {
                    option { key: "evt-{name}-{_path}", value: "{name}" }
                }
            }
            div { class: "decision-layout",
                aside {
                    class: "decision-groups",
                    id: "{dom_scope}-groups-pane",
                    style: "width: {groups_width}px;",
                    div { class: "decision-groups-head",
                        span {
                            if is_filtering {
                                "决议组（{visible_indices.len()} / {groups.len()}）"
                            } else {
                                "决议组（{groups.len()}）"
                            }
                        }
                        if let Some((completed, total)) = image_progress {
                            div {
                                class: "decision-groups-progress",
                                role: "progressbar",
                                aria_valuemin: "0",
                                aria_valuemax: "{total}",
                                aria_valuenow: "{completed}",
                                div { class: "decision-groups-progress-label",
                                    "图片加载中 {completed} / {total}"
                                }
                                div { class: "decision-groups-progress-track",
                                    div {
                                        class: "decision-groups-progress-fill",
                                        style: "width: {image_progress_percent:.1}%;",
                                    }
                                }
                            }
                        }
                    }
                    div { class: "decision-groups-filter",
                        input {
                            r#type: "text",
                            class: "decision-input",
                            value: "{group_filter}",
                            placeholder: "筛选组（ID / 名称 / 事件）...",
                            spellcheck: "false",
                            autocomplete: "off",
                            "data-native-undo": "true",
                            oninput: move |evt: FormEvent| {
                                group_filter.set(evt.value());
                                group_row_heights.set(HashMap::new());
                                group_list_scroll.set(0.0);
                                reset_group_list_scroll(&format!("{scope_filter}-groups-list"));
                            },
                        }
                        if is_filtering {
                            button {
                                class: "decision-filter-clear",
                                r#type: "button",
                                aria_label: "清除筛选",
                                onclick: move |_| {
                                    group_filter.set(String::new());
                                    group_row_heights.set(HashMap::new());
                                    group_list_scroll.set(0.0);
                                    reset_group_list_scroll(&format!("{scope_filter_clear}-groups-list"));
                                },
                                "✕"
                            }
                        }
                    }
                    if !message.read().is_empty() {
                        div {
                            class: "decision-warnings decision-side-message",
                            role: "status",
                            title: "点击关闭",
                            onclick: move |_| message.set(String::new()),
                            "⚠ {message}"
                        }
                    }
                    div {
                        class: "decision-groups-list",
                        id: "{dom_scope}-groups-list",
                        onscroll: move |evt: Event<ScrollData>| {
                            let top = evt.scroll_top();
                            // 记录滚动活跃时间：行高实测在滚动停止后再补测（见测量 effect）。
                            scroll_activity.set(performance_now());
                            if (top - *group_list_scroll.peek()).abs() >= GROUP_SCROLL_STEP {
                                group_list_scroll.set(top);
                            }
                        },
                        // 条目外松手 / 触摸手势被浏览器接管（如开始滚动）时兑底清理按下状态。
                        onpointerup: move |_| entry_press_cleanup(press_state),
                        onpointercancel: move |_| entry_press_cleanup(press_state),
                        if is_filtering && visible_indices.is_empty() {
                            div { class: "decision-empty", "无匹配的决议组" }
                        }
                        if window_start > 0 {
                            div {
                                class: "decision-group-spacer",
                                style: "height: {top_spacer:.1}px;",
                                aria_hidden: "true",
                            }
                        }
                        for position in window_start..render_end {
                            {
                                let index = visible_indices[position];
                                let group = &groups[index];
                                let is_selected = index == *selected.read();
                                let is_expanded = expanded_snapshot.contains(&index);
                                rsx! {
                                    div {
                                        key: "group-{index}",
                                        id: "{dom_scope}-group-row-{position}",
                                        class: if is_selected { "decision-group-block active" } else { "decision-group-block" },
                                        // ===== 图片预览（无图片不显示；长按拖动排序/菜单，双击定位）=====
                                        if !group.images.is_empty() {
                                            div { class: "decision-group-images",
                                                for (image_index , image) in group.images.iter().enumerate() {
                                                    if image.trim().is_empty()
                                                        || entry_draft_snapshot
                                                            .as_ref()
                                                            .is_some_and(|draft| {
                                                                draft.kind == EntryKind::Image && draft.group == index
                                                                    && draft.index == image_index
                                                            })
                                                    {
                                                        input {
                                                            key: "image-input-{index}-{image_index}",
                                                            id: "{dom_scope}-entry-img-{index}-{image_index}",
                                                            class: "decision-input decision-entry-input",
                                                            list: "{dom_scope}-image-candidates",
                                                            placeholder: "图片文件名（如 none.png）",
                                                            value: "{image}",
                                                            "data-native-undo": "true",
                                                            onfocus: move |_| {
                                                                if !entry_draft
                                                                    .peek()
                                                                    .as_ref()
                                                                    .is_some_and(|draft| {
                                                                        draft.kind == EntryKind::Image && draft.group == index
                                                                            && draft.index == image_index
                                                                    })
                                                                {
                                                                    entry_draft
                                                                        .set(
                                                                            Some(EntryDraft {
                                                                                kind: EntryKind::Image,
                                                                                group: index,
                                                                                index: image_index,
                                                                                is_new: false,
                                                                                original: None,
                                                                            }),
                                                                        );
                                                                }
                                                            },
                                                            oninput: {
                                                                let scope_id = scope_id;
                                                                move |evt: FormEvent| {
                                                                    let value = evt.value();
                                                                    apply_group_edit(
                                                                        state,
                                                                        on_undo_push,
                                                                        &scope_id.read(),
                                                                        move |groups| {
                                                                            if let Some(entry) = groups.get_mut(index) {
                                                                                if let Some(slot) = entry.images.get_mut(image_index) {
                                                                                    *slot = value.clone();
                                                                                }
                                                                            }
                                                                        },
                                                                    );
                                                                }
                                                            },
                                                            onblur: move |_| {
                                                                finalize_entry_draft(state, on_undo_push, entry_draft, &scope_id.read());
                                                            },
                                                            onkeydown: move |evt: Event<KeyboardData>| {
                                                                if evt.data().key().to_string() == "Enter" {
                                                                    evt.prevent_default();
                                                                    finalize_entry_draft(state, on_undo_push, entry_draft, &scope_id.read());
                                                                }
                                                            },
                                                        }
                                                    } else {
                                                        {
                                                            let stem = image_stem(image);
                                                            let data = sidebar_images.get(&stem);
                                                            let dragging = press_state
                                                                .read()
                                                                .is_some_and(|press| {
                                                                    press.started && press.kind == EntryKind::Image && press.group == index
                                                                        && press.index == image_index
                                                                });
                                                            let capture_id = format!("{dom_scope}-entry-img-{index}-{image_index}");
                                                            let reveal = image_dir
                                                                .as_ref()
                                                                .map(|dir| format!("{dir}/{image}"));
                                                            rsx! {
                                                                div {
                                                                    key: "image-{index}-{image}",
                                                                    id: "{capture_id}",
                                                                    class: if dragging { "decision-image-entry dragging" } else { "decision-image-entry" },
                                                                    role: "button",
                                                                    title: "长按拖动排序；右键或长按弹出菜单；双击在内置资源管理器中定位",
                                                                    oncontextmenu: move |evt: Event<MouseData>| {
                                                                        evt.prevent_default();
                                                                        let client = evt.client_coordinates();
                                                                        entry_menu
                                                                            .set(
                                                                                Some((EntryKind::Image, index, image_index, client.x, client.y)),
                                                                            );
                                                                    },
                                                                    onpointerdown: move |evt: Event<PointerData>| {
                                                                        // 鼠标右键交给 oncontextmenu；仅左键（或触摸/触控笔）进入长按/拖拽。
                                                                        if matches!(
                                                                            evt.data().trigger_button(),
                                                                            Some(MouseButton::Secondary | MouseButton::Auxiliary)
                                                                        ) {
                                                                            return;
                                                                        }
                                                                        let step_height = element_height(&capture_id)
                                                                            .unwrap_or(IMAGE_ROW_HEIGHT);
                                                                        entry_pointer_down(
                                                                            press_state,
                                                                            press_token,
                                                                            EntryKind::Image,
                                                                            index,
                                                                            image_index,
                                                                            evt.client_coordinates().y,
                                                                            step_height,
                                                                            &capture_id,
                                                                            evt.data().pointer_id(),
                                                                            evt.data().pointer_type() == "touch",
                                                                        );
                                                                    },
                                                                    onpointermove: move |evt: Event<PointerData>| {
                                                                        entry_pointer_move(
                                                                            press_state,
                                                                            state,
                                                                            on_undo_push,
                                                                            &scope_id.read(),
                                                                            EntryKind::Image,
                                                                            index,
                                                                            evt.client_coordinates().y,
                                                                        );
                                                                    },
                                                                    onpointerup: move |evt: Event<PointerData>| {
                                                                        entry_pointer_up(
                                                                            press_state,
                                                                            entry_menu,
                                                                            selected,
                                                                            EntryKind::Image,
                                                                            index,
                                                                            evt.client_coordinates().x,
                                                                            evt.client_coordinates().y,
                                                                        );
                                                                    },
                                                                    onpointercancel: move |_| entry_pointer_cancel(press_state, EntryKind::Image, index),
                                                                    ondoubleclick: move |_| {
                                                                        if let Some(path) = reveal.clone() {
                                                                            on_reveal_item.call((path, false));
                                                                        }
                                                                    },
                                                                    if let Some(data) = data {
                                                                        img {
                                                                            class: "decision-image-preview",
                                                                            src: "{data.url}",
                                                                            alt: "{image}",
                                                                            draggable: "false",
                                                                            decoding: "async",
                                                                            style: if data.width > 0 && data.height > 0 { format!("aspect-ratio: {} / {};", data.width, data.height) } else { String::new() },
                                                                        }
                                                                    } else {
                                                                        div {
                                                                            class: "decision-image-placeholder",
                                                                            style: sidebar_sizes
                                                                                .get(&stem)
                                                                                .filter(|(width, height)| *width > 0 && *height > 0)
                                                                                .map(|(width, height)| format!("aspect-ratio: {width} / {height};"))
                                                                                .unwrap_or_default(),
                                                                            "🖼"
                                                                        }
                                                                    }
                                                                    span { class: "decision-image-name", "{image}" }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        // ===== 组标题（点击选中并展开/收起事件下拉）=====
                                        div {
                                            class: "decision-group-head",
                                            role: "button",
                                            onclick: move |_| {
                                                selected.set(index);
                                                toggle_group_expanded(expanded_groups, index);
                                            },
                                            oncontextmenu: move |evt: Event<MouseData>| {
                                                evt.prevent_default();
                                                let client = evt.client_coordinates();
                                                selected.set(index);
                                                group_menu.set(Some((index, client.x, client.y)));
                                            },
                                            span { class: "decision-group-id",
                                                if group.id.trim().is_empty() {
                                                    "（未命名）"
                                                } else {
                                                    "{group.id}"
                                                }
                                            }
                                            if group_draft_snapshot
                                                .as_ref()
                                                .is_some_and(|draft| draft.index == index)
                                            {
                                                input {
                                                    id: "{dom_scope}-group-name-edit-{index}",
                                                    class: "decision-input decision-group-name-input",
                                                    value: "{group.name}",
                                                    placeholder: "决议组名称（如 政治决策）",
                                                    spellcheck: "false",
                                                    autocomplete: "off",
                                                    "data-native-undo": "true",
                                                    onclick: move |evt: Event<MouseData>| evt.stop_propagation(),
                                                    oninput: {
                                                        let scope_id = scope_id;
                                                        move |evt: FormEvent| {
                                                            let value = evt.value();
                                                            apply_group_edit(
                                                                state,
                                                                on_undo_push,
                                                                &scope_id.read(),
                                                                move |groups| {
                                                                    if let Some(entry) = groups.get_mut(index) {
                                                                        entry.name = value.clone();
                                                                    }
                                                                },
                                                            );
                                                        }
                                                    },
                                                    onblur: move |_| {
                                                        finalize_group_draft(
                                                            state,
                                                            on_undo_push,
                                                            group_draft,
                                                            &scope_id.read(),
                                                            selected,
                                                            expanded_groups,
                                                            group_row_heights,
                                                            entry_draft,
                                                        );
                                                    },
                                                    onkeydown: move |evt: Event<KeyboardData>| {
                                                        if evt.data().key().to_string() == "Enter" {
                                                            evt.prevent_default();
                                                            finalize_group_draft(
                                                                state,
                                                                on_undo_push,
                                                                group_draft,
                                                                &scope_id.read(),
                                                                selected,
                                                                expanded_groups,
                                                                group_row_heights,
                                                                entry_draft,
                                                            );
                                                        }
                                                    },
                                                }
                                            } else {
                                                span { class: "decision-group-name",
                                                    if group.name.trim().is_empty() {
                                                        "未命名决议组"
                                                    } else {
                                                        "{group.name}"
                                                    }
                                                }
                                            }
                                            if is_selected {
                                                button {
                                                    class: "decision-group-image-add",
                                                    r#type: "button",
                                                    title: "添加图片条目（填写文件名；双击可在资源管理器中定位）",
                                                    onclick: {
                                                        let scope_add_image = scope_add_image.clone();
                                                        move |evt: Event<MouseData>| {
                                                            evt.stop_propagation();
                                                            // 先占位后聚焦：新增行进入草稿态，未输入内容失焦即自动移除。
                                                            let new_index = state
                                                                .peek()
                                                                .get(index)
                                                                .map_or(0, |group| group.images.len());
                                                            edit_entries(
                                                                state,
                                                                on_undo_push,
                                                                &scope_id.read(),
                                                                EntryKind::Image,
                                                                index,
                                                                None,
                                                            );
                                                            entry_draft
                                                                .set(
                                                                    Some(EntryDraft {
                                                                        kind: EntryKind::Image,
                                                                        group: index,
                                                                        index: new_index,
                                                                        is_new: true,
                                                                        original: None,
                                                                    }),
                                                                );
                                                            selected.set(index);
                                                            focus_element(&format!("{scope_add_image}-entry-img-{index}-{new_index}"));
                                                        }
                                                    },
                                                    "🖼＋"
                                                }
                                            }
                                            span { class: "decision-group-caret",
                                                if is_expanded {
                                                    "▾"
                                                } else {
                                                    "▸"
                                                }
                                            }
                                        }
                                        // ===== 事件下拉（展开显示；长按拖动排序/菜单，双击在事件编辑器中打开）=====
                                        if is_expanded {
                                            div { class: "decision-group-events",
                                                if group.events.is_empty() {
                                                    div { class: "decision-entry-empty", "暂无决议事件" }
                                                }
                                                for (event_index , event) in group.events.iter().enumerate() {
                                                    if event.trim().is_empty()
                                                        || entry_draft_snapshot
                                                            .as_ref()
                                                            .is_some_and(|draft| {
                                                                draft.kind == EntryKind::Event && draft.group == index
                                                                    && draft.index == event_index
                                                            })
                                                    {
                                                        input {
                                                            key: "event-input-{index}-{event_index}",
                                                            id: "{dom_scope}-entry-evt-{index}-{event_index}",
                                                            class: "decision-input decision-entry-input",
                                                            list: "{dom_scope}-event-candidates",
                                                            placeholder: "事件脚本名（如 决议：战前征兵）",
                                                            value: "{event}",
                                                            "data-native-undo": "true",
                                                            onfocus: move |_| {
                                                                if !entry_draft
                                                                    .peek()
                                                                    .as_ref()
                                                                    .is_some_and(|draft| {
                                                                        draft.kind == EntryKind::Event && draft.group == index
                                                                            && draft.index == event_index
                                                                    })
                                                                {
                                                                    entry_draft
                                                                        .set(
                                                                            Some(EntryDraft {
                                                                                kind: EntryKind::Event,
                                                                                group: index,
                                                                                index: event_index,
                                                                                is_new: false,
                                                                                original: None,
                                                                            }),
                                                                        );
                                                                }
                                                            },
                                                            oninput: {
                                                                let scope_id = scope_id;
                                                                move |evt: FormEvent| {
                                                                    let value = evt.value();
                                                                    apply_group_edit(
                                                                        state,
                                                                        on_undo_push,
                                                                        &scope_id.read(),
                                                                        move |groups| {
                                                                            if let Some(entry) = groups.get_mut(index) {
                                                                                if let Some(slot) = entry.events.get_mut(event_index) {
                                                                                    *slot = value.clone();
                                                                                }
                                                                            }
                                                                        },
                                                                    );
                                                                }
                                                            },
                                                            onblur: move |_| {
                                                                finalize_entry_draft(state, on_undo_push, entry_draft, &scope_id.read());
                                                            },
                                                            onkeydown: move |evt: Event<KeyboardData>| {
                                                                if evt.data().key().to_string() == "Enter" {
                                                                    evt.prevent_default();
                                                                    finalize_entry_draft(state, on_undo_push, entry_draft, &scope_id.read());
                                                                }
                                                            },
                                                        }
                                                    } else {
                                                        {
                                                            let dragging = press_state
                                                                .read()
                                                                .is_some_and(|press| {
                                                                    press.started && press.kind == EntryKind::Event && press.group == index
                                                                        && press.index == event_index
                                                                });
                                                            let capture_id = format!("{dom_scope}-entry-evt-{index}-{event_index}");
                                                            let script_name = event.clone();
                                                            rsx! {
                                                                div {
                                                                    key: "event-{index}-{event}",
                                                                    id: "{capture_id}",
                                                                    class: if dragging { "decision-entry dragging" } else { "decision-entry" },
                                                                    role: "button",
                                                                    title: "长按拖动排序；右键或长按弹出菜单；双击在事件编辑器中打开",
                                                                    oncontextmenu: move |evt: Event<MouseData>| {
                                                                        evt.prevent_default();
                                                                        let client = evt.client_coordinates();
                                                                        entry_menu
                                                                            .set(
                                                                                Some((EntryKind::Event, index, event_index, client.x, client.y)),
                                                                            );
                                                                    },
                                                                    onpointerdown: move |evt: Event<PointerData>| {
                                                                        // 鼠标右键交给 oncontextmenu；仅左键（或触摸/触控笔）进入长按/拖拽。
                                                                        if matches!(
                                                                            evt.data().trigger_button(),
                                                                            Some(MouseButton::Secondary | MouseButton::Auxiliary)
                                                                        ) {
                                                                            return;
                                                                        }
                                                                        let step_height = element_height(&capture_id)
                                                                            .unwrap_or(EVENT_ROW_HEIGHT);
                                                                        entry_pointer_down(
                                                                            press_state,
                                                                            press_token,
                                                                            EntryKind::Event,
                                                                            index,
                                                                            event_index,
                                                                            evt.client_coordinates().y,
                                                                            step_height,
                                                                            &capture_id,
                                                                            evt.data().pointer_id(),
                                                                            evt.data().pointer_type() == "touch",
                                                                        );
                                                                    },
                                                                    onpointermove: move |evt: Event<PointerData>| {
                                                                        entry_pointer_move(
                                                                            press_state,
                                                                            state,
                                                                            on_undo_push,
                                                                            &scope_id.read(),
                                                                            EntryKind::Event,
                                                                            index,
                                                                            evt.client_coordinates().y,
                                                                        );
                                                                    },
                                                                    onpointerup: move |evt: Event<PointerData>| {
                                                                        entry_pointer_up(
                                                                            press_state,
                                                                            entry_menu,
                                                                            selected,
                                                                            EntryKind::Event,
                                                                            index,
                                                                            evt.client_coordinates().x,
                                                                            evt.client_coordinates().y,
                                                                        );
                                                                    },
                                                                    onpointercancel: move |_| entry_pointer_cancel(press_state, EntryKind::Event, index),
                                                                    ondoubleclick: {
                                                                        let mod_prefix = mod_prefix.clone();
                                                                        let mod_dir = mod_dir.clone();
                                                                        let work_directory = work_directory.clone();
                                                                        move |_| {
                                                                            let name = script_name.clone();
                                                                            let mod_prefix = mod_prefix.clone();
                                                                            let mod_dir = mod_dir.clone();
                                                                            let work_directory = work_directory.clone();
                                                                            let candidates = event_candidates;
                                                                            spawn(async move {
                                                                                open_or_create_script(
                                                                                        &name,
                                                                                        &candidates.peek(),
                                                                                        &mod_prefix,
                                                                                        &mod_dir,
                                                                                        work_directory,
                                                                                        message,
                                                                                        on_open_script,
                                                                                        on_open_missing_script,
                                                                                    )
                                                                                    .await;
                                                                            });
                                                                        }
                                                                    },
                                                                    span { class: "decision-entry-index", "{event_index}" }
                                                                    span { class: "decision-entry-name", "{event}" }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                                button {
                                                    class: "decision-entry-add",
                                                    r#type: "button",
                                                    title: "添加决议事件脚本名（长按条目也可添加/删除）",
                                                    onclick: {
                                                        let scope_add_event = scope_add_event.clone();
                                                        move |_| {
                                                            // 先占位后聚焦：新增行进入草稿态，未输入内容失焦即自动移除。
                                                            let new_index = state
                                                                .peek()
                                                                .get(index)
                                                                .map_or(0, |group| group.events.len());
                                                            edit_entries(
                                                                state,
                                                                on_undo_push,
                                                                &scope_id.read(),
                                                                EntryKind::Event,
                                                                index,
                                                                None,
                                                            );
                                                            entry_draft
                                                                .set(
                                                                    Some(EntryDraft {
                                                                        kind: EntryKind::Event,
                                                                        group: index,
                                                                        index: new_index,
                                                                        is_new: true,
                                                                        original: None,
                                                                    }),
                                                                );
                                                            selected.set(index);
                                                            focus_element(&format!("{scope_add_event}-entry-evt-{index}-{new_index}"));
                                                        }
                                                    },
                                                    "＋ 添加事件"
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if render_end < list_len {
                            div {
                                class: "decision-group-spacer",
                                style: "height: {bottom_spacer:.1}px;",
                                aria_hidden: "true",
                            }
                        }
                    }
                    button {
                        class: "decision-add-group",
                        onclick: {
                            let scope_add_group = scope_add_group.clone();
                            move |_| {
                                // 新组追加到末尾：先清筛选/行高缓存保证可见，随后滚动到底并聚焦命名输入；
                                // 未输入有效名称就失焦 → 自动移除（见 `finalize_group_draft`）。
                                group_filter.set(String::new());
                                group_row_heights.set(HashMap::new());
                                let new_index = state.peek().len();
                                apply_group_edit(
                                    state,
                                    on_undo_push,
                                    &scope_id.read(),
                                    move |groups| {
                                        groups.push(DecisionGroup::new_empty());
                                    },
                                );
                                selected.set(new_index);
                                group_draft
                                    .set(
                                        Some(GroupDraft {
                                            index: new_index,
                                            is_new: true,
                                            original: None,
                                        }),
                                    );
                                let list_id = format!("{scope_add_group}-groups-list");
                                let edit_id = format!("{scope_add_group}-group-name-edit-{new_index}");
                                let mut group_list_scroll = group_list_scroll;
                                spawn(async move {
                                    sleep_ms(50).await;
                                    let script = format!(
                                        "(function(){{ const el = document.getElementById('{list_id}'); if (!el) return 0; el.scrollTop = el.scrollHeight; return el.scrollTop; }})();",
                                    );
                                    if let Ok(value) = dioxus::document::eval(&script).join::<f64>().await {
                                        group_list_scroll.set(value);
                                    }
                                    let _ = dioxus::document::eval(
                                            &format!(
                                                "(function(){{ const el = document.getElementById('{edit_id}'); if (el) {{ el.focus(); if (el.select) el.select(); }} }})();",
                                            ),
                                        )
                                        .await;
                                });
                            }
                        },
                        "＋ 新建决议组"
                    }
                }
                div {
                    class: "decision-resize-handle",
                    role: "separator",
                    aria_orientation: "vertical",
                    aria_label: "拖动调整决议组区域宽度",
                    id: "{dom_scope}-resize-handle",
                    onpointerdown: move |evt: Event<PointerData>| {
                        let width = *groups_width.peek();
                        resize_for_down.set(Some((evt.client_coordinates().x, width, width)));
                        set_pane_resizing(&scope_handle_down, true);
                        let pointer_id = evt.data().pointer_id();
                        let handle_id = format!("{scope_handle_down}-resize-handle");
                        spawn(async move {
                            let _ = dioxus::document::eval(
                                    &format!(
                                        "document.getElementById('{handle_id}')?.setPointerCapture({pointer_id})",
                                    ),
                                )
                                .await;
                        });
                    },
                    onpointermove: move |evt: Event<PointerData>| {
                        let Some((start_x, base_width, _)) = resize_for_move.get() else {
                            return;
                        };
                        // 始终以「按下时的起始宽度」为基准计算位移（勿用最新宽度做基准：
                        // 那样每帧位移会累加，分割栏跑得比指针快）。
                        let next = (base_width + evt.client_coordinates().x - start_x)
                            .clamp(150.0, 640.0);
                        resize_for_move.set(Some((start_x, base_width, next)));
                        set_pane_width(&scope_handle_move, next);
                    },
                    onpointerup: move |_| end_pane_resize(&scope_handle_up, &resize_for_up, groups_width),
                    onpointercancel: move |_| end_pane_resize(&scope_handle_cancel, &resize_for_cancel, groups_width),
                }
                section { class: "decision-detail",
                    if warnings.is_empty() {
                        div { class: "decision-warnings ok", "未发现明显问题" }
                    } else if warnings.len() <= 3 {
                        div { class: "decision-warnings",
                            for (warning_index , warning) in warnings.iter().enumerate() {
                                div {
                                    key: "{warning_index}",
                                    class: "decision-warning",
                                    "⚠ {warning}"
                                }
                            }
                        }
                    } else {
                        // 大型模组（近百组）告警可达十余条，默认折叠避免占满编辑区。
                        details { class: "decision-warnings decision-warnings-collapsed",
                            summary { "⚠ {warnings.len()} 条告警（点击展开）" }
                            for (warning_index , warning) in warnings.iter().enumerate() {
                                div {
                                    key: "{warning_index}",
                                    class: "decision-warning",
                                    "⚠ {warning}"
                                }
                            }
                        }
                    }
                    if groups.is_empty() {
                        div { class: "decision-empty",
                            "还没有决议组。点击左下角「新建决议组」开始编辑。"
                        }
                    }
                    // 借用选中组渲染（skip/take 只取一行；`index` 供下方编辑闭包捕获）。
                    for (index , group) in groups.iter().enumerate().skip(*selected.read()).take(1) {
                        // ===== 基本信息 =====
                        div { class: "decision-section",
                            div { class: "decision-section-title", "基本信息" }
                            label { class: "decision-field",
                                span { class: "decision-field-label",
                                    "组 ID（add_decision 解锁、复合键前半段）"
                                }
                                input {
                                    class: "decision-input",
                                    value: "{group.id}",
                                    "data-native-undo": "true",
                                    oninput: {
                                        let scope_id = scope_id;
                                        move |evt: FormEvent| {
                                            let value = evt.value();
                                            apply_group_edit(
                                                state,
                                                on_undo_push,
                                                &scope_id.read(),
                                                move |groups| {
                                                    if let Some(group) = groups.get_mut(index) {
                                                        group.id = value.clone();
                                                    }
                                                },
                                            );
                                        }
                                    },
                                }
                            }
                            label { class: "decision-field",
                                span { class: "decision-field-label", "显示名称" }
                                input {
                                    class: "decision-input",
                                    value: "{group.name}",
                                    "data-native-undo": "true",
                                    oninput: {
                                        let scope_id = scope_id;
                                        move |evt: FormEvent| {
                                            let value = evt.value();
                                            apply_group_edit(
                                                state,
                                                on_undo_push,
                                                &scope_id.read(),
                                                move |groups| {
                                                    if let Some(group) = groups.get_mut(index) {
                                                        group.name = value.clone();
                                                    }
                                                },
                                            );
                                        }
                                    },
                                }
                            }
                        }
                        // ===== 描述（可多条，decision_desc 切换显示）=====
                        div { class: "decision-section",
                            div { class: "decision-section-title",
                                "描述（第 1 条为初始；decision_desc 按序号切换）"
                            }
                            for (desc_index , text) in group.desc.iter().enumerate() {
                                div {
                                    key: "desc-{desc_index}",
                                    class: "decision-desc-row",
                                    span { class: "decision-index", "{desc_index}" }
                                    textarea {
                                        class: "decision-desc-input",
                                        rows: "3",
                                        value: "{text}",
                                        "data-native-undo": "true",
                                        oninput: {
                                            let scope_id = scope_id;
                                            move |evt: FormEvent| {
                                                let value = evt.value();
                                                apply_group_edit(
                                                    state,
                                                    on_undo_push,
                                                    &scope_id.read(),
                                                    move |groups| {
                                                        if let Some(group) = groups.get_mut(index) {
                                                            if let Some(text) = group.desc.get_mut(desc_index) {
                                                                *text = value.clone();
                                                            }
                                                        }
                                                    },
                                                );
                                            }
                                        },
                                    }
                                    div { class: "decision-row-actions",
                                        button {
                                            onclick: {
                                                let scope_id = scope_id;
                                                move |_| apply_group_edit(
                                                    state,
                                                    on_undo_push,
                                                    &scope_id.read(),
                                                    move |groups| {
                                                        if let Some(group) = groups.get_mut(index) {
                                                            list_move_up(&mut group.desc, desc_index);
                                                        }
                                                    },
                                                )
                                            },
                                            title: "上移",
                                            "↑"
                                        }
                                        button {
                                            onclick: {
                                                let scope_id = scope_id;
                                                move |_| apply_group_edit(
                                                    state,
                                                    on_undo_push,
                                                    &scope_id.read(),
                                                    move |groups| {
                                                        if let Some(group) = groups.get_mut(index) {
                                                            list_move_down(&mut group.desc, desc_index);
                                                        }
                                                    },
                                                )
                                            },
                                            title: "下移",
                                            "↓"
                                        }
                                        button {
                                            onclick: {
                                                let scope_id = scope_id;
                                                move |_| apply_group_edit(
                                                    state,
                                                    on_undo_push,
                                                    &scope_id.read(),
                                                    move |groups| {
                                                        if let Some(group) = groups.get_mut(index) {
                                                            if desc_index < group.desc.len() {
                                                                group.desc.remove(desc_index);
                                                            }
                                                        }
                                                    },
                                                )
                                            },
                                            title: "删除",
                                            "✕"
                                        }
                                    }
                                }
                            }
                            button {
                                class: "decision-add-row",
                                onclick: {
                                    let scope_id = scope_id;
                                    move |_| apply_group_edit(
                                        state,
                                        on_undo_push,
                                        &scope_id.read(),
                                        move |groups| {
                                            if let Some(group) = groups.get_mut(index) {
                                                group.desc.push(String::new());
                                            }
                                        },
                                    )
                                },
                                "＋ 添加描述"
                            }
                        }
                    }
                }
            }
            // 决议组菜单（右键弹出：重命名 / 删除）。
            if let Some((menu_group_index, menu_x, menu_y)) = *group_menu.read() {
                div {
                    class: "menu-dismiss",
                    style: "z-index: 57;",
                    aria_hidden: "true",
                    onclick: move |_| group_menu.set(None),
                }
                div {
                    class: "context-menu",
                    role: "menu",
                    style: "position: fixed; z-index: 58; left: max(8px, min({menu_x}px, calc(100vw - 176px))); top: max(8px, min({menu_y}px, calc(100vh - 104px))); width: min(168px, calc(100vw - 16px));",
                    button {
                        r#type: "button",
                        onclick: {
                            let scope_group_menu = scope_group_menu.clone();
                            move |_| {
                                group_menu.set(None);
                                let original = state
                                    .peek()
                                    .get(menu_group_index)
                                    .map(|group| group.name.clone());
                                group_draft
                                    .set(
                                        Some(GroupDraft {
                                            index: menu_group_index,
                                            is_new: false,
                                            original,
                                        }),
                                    );
                                selected.set(menu_group_index);
                                focus_element(
                                    &format!("{scope_group_menu}-group-name-edit-{menu_group_index}"),
                                );
                            }
                        },
                        "重命名"
                    }
                    button {
                        r#type: "button",
                        onclick: move |_| {
                            group_menu.set(None);
                            remove_group(
                                state,
                                on_undo_push,
                                &scope_id.read(),
                                menu_group_index,
                                selected,
                                expanded_groups,
                                group_row_heights,
                                group_draft,
                                entry_draft,
                            );
                        },
                        "删除"
                    }
                }
            }
            // 条目菜单（鼠标右键 / 长按松手弹出；固定定位在触发点；点空白处关闭）。
            // 样式复用资源管理器 / 画布的 `.context-menu`（见 files.rs::ExplorerContextMenu）。
            if let Some((menu_kind, menu_group, menu_index, menu_x, menu_y)) = *entry_menu.read() {
                div {
                    class: "menu-dismiss",
                    style: "z-index: 55;",
                    aria_hidden: "true",
                    onclick: move |_| entry_menu.set(None),
                }
                div {
                    class: "context-menu",
                    role: "menu",
                    style: "position: fixed; z-index: 56; left: max(8px, min({menu_x}px, calc(100vw - 176px))); top: max(8px, min({menu_y}px, calc(100vh - 132px))); width: min(168px, calc(100vw - 16px));",
                    button {
                        r#type: "button",
                        onclick: {
                            let scope_entry_menu = scope_entry_menu.clone();
                            move |_| {
                                // 重命名：改为行内输入框并聚焦；清空失焦时还原原名称。
                                entry_menu.set(None);
                                let value = state
                                    .peek()
                                    .get(menu_group)
                                    .and_then(|group| match menu_kind {
                                        EntryKind::Image => group.images.get(menu_index),
                                        EntryKind::Event => group.events.get(menu_index),
                                    })
                                    .cloned()
                                    .unwrap_or_default();
                                entry_draft
                                    .set(
                                        Some(EntryDraft {
                                            kind: menu_kind,
                                            group: menu_group,
                                            index: menu_index,
                                            is_new: false,
                                            original: Some(value),
                                        }),
                                    );
                                let kind_tag = entry_dom_tag(menu_kind);
                                focus_element(
                                    &format!("{scope_entry_menu}-entry-{kind_tag}-{menu_group}-{menu_index}"),
                                );
                            }
                        },
                        "重命名"
                    }
                    button {
                        r#type: "button",
                        onclick: move |_| {
                            edit_entries(
                                state,
                                on_undo_push,
                                &scope_id.read(),
                                menu_kind,
                                menu_group,
                                Some(menu_index),
                            );
                            entry_menu.set(None);
                        },
                        "删除"
                    }
                    button {
                        r#type: "button",
                        onclick: {
                            let scope_entry_menu = scope_entry_menu.clone();
                            move |_| {
                                // 菜单添加与「＋」按钮一致：先占位后聚焦，未输入失焦即自动移除。
                                entry_menu.set(None);
                                let new_index = state
                                    .peek()
                                    .get(menu_group)
                                    .map_or(
                                        0,
                                        |group| match menu_kind {
                                            EntryKind::Image => group.images.len(),
                                            EntryKind::Event => group.events.len(),
                                        },
                                    );
                                edit_entries(
                                    state,
                                    on_undo_push,
                                    &scope_id.read(),
                                    menu_kind,
                                    menu_group,
                                    None,
                                );
                                entry_draft
                                    .set(
                                        Some(EntryDraft {
                                            kind: menu_kind,
                                            group: menu_group,
                                            index: new_index,
                                            is_new: true,
                                            original: None,
                                        }),
                                    );
                                selected.set(menu_group);
                                if menu_kind == EntryKind::Event
                                    && !expanded_groups.peek().contains(&menu_group)
                                {
                                    toggle_group_expanded(expanded_groups, menu_group);
                                }
                                let kind_tag = entry_dom_tag(menu_kind);
                                focus_element(
                                    &format!("{scope_entry_menu}-entry-{kind_tag}-{menu_group}-{new_index}"),
                                );
                            }
                        },
                        "添加"
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn decision_file_detection_and_tab_id() {
		assert!(is_decision_file("rainfall/rfEvent_decision.json"));
		assert!(is_decision_file("assets/rainfall/rfEvent_decision.json"));
		assert!(is_decision_file("MyMod/assets/Rainfall/RFEvent_Decision.JSON"));
		assert!(!is_decision_file("assets/game/missions/Missions.json"));
		assert!(!is_decision_file("rainfall/rfEvent.json"));

		let id = decision_tab_id("assets/rainfall/rfEvent_decision.json");
		assert_eq!(id, "decision:assets/rainfall/rfEvent_decision.json");
		assert!(is_decision_tab_id(&id));
		assert!(!is_decision_tab_id("assets/game/missions/Missions.json"));
	}

	/// 决议图片目录推导与缓存键（与后端 `decision_image_relative_dir` 规则一致）。
	#[test]
	fn decision_image_dir_and_stem() {
		assert_eq!(
			decision_image_dir("rainfall/rfEvent_decision.json").as_deref(),
			Some("gfx/decision")
		);
		assert_eq!(
			decision_image_dir("assets/rainfall/rfEvent_decision.json").as_deref(),
			Some("assets/gfx/decision")
		);
		assert_eq!(
			decision_image_dir("MyMod/assets/Rainfall/RFEvent_Decision.JSON").as_deref(),
			Some("MyMod/assets/gfx/decision")
		);
		assert_eq!(decision_image_dir("assets/game/missions/Missions.json"), None);
		assert_eq!(image_stem("none.png"), "none");
		assert_eq!(image_stem("SC.PNG"), "SC");
		assert_eq!(image_stem("plain"), "plain");
	}

	/// 组展开切换：未展开则加入、已展开则移除（点击组标题的纯逻辑）。
	#[test]
	fn toggle_group_adds_and_removes() {
		let mut list = Vec::new();
		toggle_group(&mut list, 3);
		toggle_group(&mut list, 1);
		assert_eq!(list, vec![3, 1]);
		toggle_group(&mut list, 3);
		assert_eq!(list, vec![1]);
		toggle_group(&mut list, 9);
		assert_eq!(list, vec![1, 9]);
	}

	#[test]
	fn dom_scope_sanitizes_tab_id() {
		assert_eq!(
			dom_scope_id("decision:assets/rainfall/rfEvent_decision.json"),
			"decision_assets_rainfall_rfEvent_decision_json"
		);
		assert_eq!(dom_scope_id("a-b_c1"), "a-b_c1");
	}

	#[test]
	fn parses_png_dims_from_data_url_header() {
		// 100×100 PNG：前 32 个 base64 字符已覆盖签名与 IHDR 宽高（100, 100）。
		let url = "data:image/png;base64,\
iVBORw0KGgoAAAANSUhEUgAAAGQAAABkCAYAAABw4pVUAAAAAXNSR0IArs4c6QAAAARnQU1BAACxjwv8YQU=";
		assert_eq!(png_dims_from_data_url(url), Some((100, 100)));
		// 非 PNG 前缀 / 过短内容 / 非 base64 字符均返回 None。
		assert_eq!(png_dims_from_data_url("data:image/jpeg;base64,AAAA"), None);
		assert_eq!(png_dims_from_data_url("data:image/png;base64,AAAA"), None);
		assert_eq!(png_dims_from_data_url("data:image/png;base64,!!!!"), None);
	}

	#[test]
	fn warns_on_duplicate_ids_and_cross_group_events() {
		let mut first = DecisionGroup::new_empty();
		first.id = "RUS".to_string();
		first.name = "政治决策".to_string();
		first.events = vec!["决议：清洗".to_string(), "决议：投资".to_string()];
		let mut second = DecisionGroup::new_empty();
		second.id = "rus".to_string(); // 大小写不同视为不同 id（引擎按原样比较）
		second.events = vec!["决议：清洗".to_string()];
		let mut third = DecisionGroup::new_empty();
		third.id = "UKR".to_string();
		third.name = "未命名？".to_string();
		third.events = vec!["".to_string(), "决议：清洗".to_string()];

		let warnings = collect_group_warnings(&[first, second, third]);
		assert!(warnings.iter().any(|warning| warning.contains("名称为空")));
		assert!(warnings.iter().any(|warning| warning.contains("空的事件名")));
		assert!(warnings.iter().any(|warning| warning.contains("同时出现在")));
		assert!(!warnings.iter().any(|warning| warning.contains("重复")));
	}

	#[test]
	fn warns_on_duplicate_id_with_missing_name() {
		let mut first = DecisionGroup::new_empty();
		first.id = "A".to_string();
		first.name = "甲".to_string();
		let mut second = DecisionGroup::new_empty();
		second.id = "A".to_string();

		let warnings = collect_group_warnings(&[first, second]);
		assert!(warnings.iter().any(|warning| warning.contains("重复")));
		assert!(warnings.iter().any(|warning| warning.contains("名称为空")));
	}

	#[test]
	fn filters_image_and_script_candidates() {
		let files = vec![
			"assets/gfx/decision/俄参谋.png".to_string(),
			"assets/gfx/decision/none.png".to_string(),
			"assets/gfx/flags/x.png".to_string(),
			"assets/map/ES/scenarios/RusUkrWar/events/common/决议：战前征兵.txt".to_string(),
			"assets/game/missions/missionsEvents/事件脚本.txt".to_string(),
			"assets/game/events/other.txt".to_string(),
			"assets/audio/music/list.txt".to_string(),
			"gfx/decision/root.png".to_string(),
		];

		let images = decision_image_names(&files);
		assert_eq!(
			images,
			vec!["none.png".to_string(), "root.png".to_string(), "俄参谋.png".to_string()]
		);

		let scripts = event_script_candidates(&files, "");
		let names: Vec<&str> = scripts.iter().map(|(name, _)| name.as_str()).collect();
		assert!(names.contains(&"决议：战前征兵"));
		assert!(names.contains(&"事件脚本"));
		assert!(names.contains(&"other"));
		assert!(!names.contains(&"list"), "音频目录不算事件脚本");
		let war = scripts
			.iter()
			.find(|(name, _)| name == "决议：战前征兵")
			.expect("应含决议事件脚本");
		assert!(war.1.ends_with("决议：战前征兵.txt"));
	}

	#[test]
	fn list_move_helpers_clamp() {
		let mut list = vec!["a".to_string(), "b".to_string(), "c".to_string()];
		list_move_up(&mut list, 0);
		assert_eq!(list, vec!["a", "b", "c"]);
		list_move_up(&mut list, 2);
		assert_eq!(list, vec!["a", "c", "b"]);
		list_move_down(&mut list, 2);
		assert_eq!(list, vec!["a", "c", "b"]);
		list_move_down(&mut list, 0);
		assert_eq!(list, vec!["c", "a", "b"]);
	}

	#[test]
	fn find_script_path_matches_by_file_name() {
		let candidates = vec![
			(
				"决议：清洗".to_string(),
				"assets/game/events/common/决议：清洗.txt".to_string(),
			),
			(
				"决议：战前征兵".to_string(),
				"assets/1566/events/common/决议：战前征兵.txt".to_string(),
			),
		];
		assert_eq!(
			find_script_path(&candidates, "决议：清洗").as_deref(),
			Some("assets/game/events/common/决议：清洗.txt")
		);
		assert!(find_script_path(&candidates, "  决议：清洗  ").is_some());
		assert!(find_script_path(&candidates, "不存在").is_none());
		assert!(find_script_path(&candidates, "  ").is_none());
		assert!(find_script_path(&[], "决议：清洗").is_none());
	}

	#[test]
	fn default_decision_script_dir_prefers_existing_common() {
		let with_common = vec![
			(
				"决议：清洗".to_string(),
				"assets/game/events/common/决议：清洗.txt".to_string(),
			),
			("任务".to_string(), "assets/game/missions/test.txt".to_string()),
		];
		assert_eq!(
			default_decision_script_dir(&with_common).as_deref(),
			Some("assets/game/events/common")
		);
		// missions 根推导：全局事件 → `<assets>/game/events/common`。
		let from_global = vec![(
			"任务".to_string(),
			"assets/game/missions/test.txt".to_string(),
		)];
		assert_eq!(
			default_decision_script_dir(&from_global).as_deref(),
			Some("assets/game/events/common")
		);
		// 剧本 missions 根推导。
		let from_scenario = vec![(
			"任务".to_string(),
			"assets/1566/missions/events/test.txt".to_string(),
		)];
		assert_eq!(
			default_decision_script_dir(&from_scenario).as_deref(),
			Some("assets/1566/events/common")
		);
		assert!(default_decision_script_dir(&[]).is_none());
	}

	/// 多模组工作区：事件脚本候选 / 新建目录按决议文件所在模组前缀限定（不串到其他模组）。
	#[test]
	fn scopes_event_candidates_and_script_dir_to_mod_prefix() {
		let files = vec![
			"modA/assets/rainfall/rfEvent_decision.json".to_string(),
			"modA/assets/game/events/common/决议：甲.txt".to_string(),
			"modA/assets/game/missionsEvents/国策事件.txt".to_string(),
			"modB/assets/rainfall/rfEvent_decision.json".to_string(),
			"modB/assets/game/events/common/决议：乙.txt".to_string(),
		];
		// 无前缀 = 全部（旧行为）。
		assert_eq!(event_script_candidates(&files, "").len(), 3);
		// 限定到 modB：候选仅 modB 的脚本，新建目录也只落到 modB。
		let prefix = decision_root_prefix("modB/assets/rainfall/rfEvent_decision.json").unwrap();
		assert_eq!(prefix, "modB/assets/");
		let scoped = event_script_candidates(&files, prefix);
		assert_eq!(scoped.len(), 1);
		assert_eq!(scoped[0].0, "决议：乙");
		assert_eq!(
			default_decision_script_dir(&scoped).as_deref(),
			Some("modB/assets/game/events/common")
		);
		// 模组内无任何脚本时的回退目录（推断到引擎全局事件目录）。
		assert_eq!(
			mod_script_fallback_dir("modB/assets/"),
			"modB/assets/game/events/common"
		);
		assert_eq!(mod_script_fallback_dir(""), "game/events/common");
		assert_eq!(
			mod_script_fallback_dir("modB/assets/1566/"),
			"modB/assets/1566/events/common"
		);
	}

	#[test]
	fn decision_root_prefix_marks_mod_boundary() {
		assert_eq!(
			decision_root_prefix("modB/assets/rainfall/rfEvent_decision.json"),
			Some("modB/assets/")
		);
		assert_eq!(
			decision_root_prefix("rainfall/rfEvent_decision.json"),
			Some("")
		);
		assert_eq!(
			decision_root_prefix("assets/rainfall/x.json"),
			Some("assets/")
		);
		assert_eq!(decision_root_prefix("assets/gfx/decision/x.png"), None);
	}

	#[test]
	fn decision_script_template_contains_required_fields() {
		let template = decision_script_template("决议：清洗");
		assert!(template.contains("id=决议：清洗"));
		assert!(template.contains("title=决议：清洗"));
		assert!(template.contains("decision_dura="));
		assert!(template.contains("trigger_and"));
		assert!(template.contains("option_end"));
	}

	#[test]
	fn group_serde_round_trip_keeps_unknown_fields() {
		let json = r#"{
			"id":"RUS",
			"name":"政治决策",
			"desc":["第一段"],
			"events":["决议：清洗"],
			"customKey": 7,
			"扩展":{"a":true}
		}"#;
		let group: DecisionGroup = serde_json::from_str(json).unwrap();
		assert_eq!(group.extra.get("customKey"), Some(&serde_json::json!(7)));
		let back = serde_json::to_string(&group).unwrap();
		let again: DecisionGroup = serde_json::from_str(&back).unwrap();
		assert_eq!(again.extra.get("customKey"), Some(&serde_json::json!(7)));
		assert_eq!(again.extra.get("扩展"), Some(&serde_json::json!({"a":true})));
		assert_eq!(again.events, vec!["决议：清洗".to_string()]);
	}

	#[test]
	fn decision_tab_title_distinguishes_mod_directories() {
		assert_eq!(decision_tab_title("rainfall/rfEvent_decision.json"), "决议配置");
		assert_eq!(decision_tab_title("assets/rainfall/rfEvent_decision.json"), "决议配置");
		assert_eq!(
			decision_tab_title("europe/assets/rainfall/rfEvent_decision.json"),
			"europe/决议配置"
		);
		assert_eq!(
			decision_tab_title("1566AuroraPrever2/assets/Rainfall/RFEvent_Decision.json"),
			"1566AuroraPrever2/决议配置"
		);
		assert_eq!(
			decision_tab_title("白日升/rainfall/rfEvent_decision.json"),
			"白日升/决议配置"
		);
		assert_eq!(
			decision_tab_title("a/b/assets/rainfall/rfEvent_decision.json"),
			"a/b/决议配置"
		);
		assert_eq!(decision_tab_title("docs/readme.md"), "决议配置");
	}

	#[test]
	fn filter_group_indices_matches_id_name_and_event() {
		let mut first = DecisionGroup::new_empty();
		first.id = "yucl四川之争".to_string();
		first.name = "四川之争".to_string();
		first.events = vec!["改任维新派1".to_string()];
		let mut second = DecisionGroup::new_empty();
		second.id = "chi教育".to_string();
		second.name = "教育现代化".to_string();
		second.events = vec!["教育经费".to_string()];
		let groups = vec![first, second];
		assert_eq!(filter_group_indices(&groups, ""), vec![0, 1]);
		assert_eq!(filter_group_indices(&groups, "YUCL"), vec![0]);
		assert_eq!(filter_group_indices(&groups, "教育"), vec![1]);
		assert_eq!(filter_group_indices(&groups, "  四川  "), vec![0]);
		// 事件 id 匹配：搜到含有该事件的决议组。
		assert_eq!(filter_group_indices(&groups, "维新派"), vec![0]);
		assert_eq!(filter_group_indices(&groups, "教育经费"), vec![1]);
		assert!(filter_group_indices(&groups, "不存在").is_empty());
	}
}
