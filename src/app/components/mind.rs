use dioxus::prelude::*;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use super::event_parser::default_event;
use super::undo::{UndoRegistration, UndoScope};

const GRID_COLUMN_STEP: f64 = 340.0;
const GRID_ROW_STEP: f64 = 220.0;
const NODE_WIDTH: f64 = 200.0;
const NODE_HEIGHT: f64 = 130.0;
const CANVAS_WIDTH: f64 = 1200.0;
const CANVAS_HEIGHT: f64 = 800.0;
const INITIAL_TREE_COLUMNS: f64 = 12.0;
const INITIAL_TREE_ROWS: f64 = 12.0;

#[derive(Clone, PartialEq, Deserialize)]
pub struct FocusIcon {
    pub name: String,
    pub data_url: String,
}

#[derive(Clone, PartialEq, Deserialize, Serialize)]
pub struct MissionRecord {
    #[serde(rename = "ID")]
    pub id: i64,
    #[serde(rename = "Name", default)]
    pub name: String,
    #[serde(rename = "ImageName", default)]
    pub image_name: String,
    #[serde(rename = "MissionEvent", default)]
    pub mission_event: String,
    #[serde(rename = "TreeColumn", default)]
    pub tree_column: u32,
    #[serde(rename = "TreeRow", default)]
    pub tree_row: u32,
    /// 前置国策（标量写法，-1 表示无）；`None` = 原文件未写该字段（列表写法）。
    #[serde(
        rename = "RequiredMission",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub required_mission: Option<i64>,
    #[serde(
        rename = "RequiredMission2",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub required_mission2: Option<i64>,
    /// 前置国策列表（需全部完成；白日升等模组的列表写法）。
    #[serde(
        rename = "RequiredMissions",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub required_missions: Option<Vec<i64>>,
    /// 前置国策列表（满足其中任一即可；OR 组 1）。
    #[serde(
        rename = "RequiredMissionsOR",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub required_missions_or: Option<Vec<i64>>,
    /// OR 组 2。
    #[serde(
        rename = "RequiredMissionsOR2",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub required_missions_or2: Option<Vec<i64>>,
    /// OR 组 3。
    #[serde(
        rename = "RequiredMissionsOR3",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub required_missions_or3: Option<Vec<i64>>,
    /// 互斥国策 ID 列表（不能同时选择）。
    #[serde(
        rename = "MutuallyExclusiveMissions",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub mutually_exclusive_missions: Option<Vec<i64>>,
    /// AI 权重（0~100；缺失时按 100 读取，保存时沿用原值）。
    #[serde(rename = "AI", default = "default_mission_ai")]
    pub ai: i32,
    /// 工具尚未注册的未知字段（未来游戏 / 模组新增键）：解析时收集、保存时原样带回。
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// 缺少 `AI` 字段时的默认权重（与新建卡片的默认一致）。
fn default_mission_ai() -> i32 {
    100
}

fn initial_zoom() -> f64 {
    let horizontal_fit = CANVAS_WIDTH / (GRID_COLUMN_STEP * INITIAL_TREE_COLUMNS);
    let vertical_fit = CANVAS_HEIGHT / (GRID_ROW_STEP * INITIAL_TREE_ROWS);
    horizontal_fit.min(vertical_fit).clamp(0.2, 1.0)
}

fn grid_to_world(column: u32, row: u32) -> (f64, f64) {
    (
        f64::from(column) * GRID_COLUMN_STEP,
        f64::from(row) * GRID_ROW_STEP,
    )
}

fn world_to_grid(x: f64, y: f64) -> (u32, u32) {
    (
        (x / GRID_COLUMN_STEP).round().max(0.0) as u32,
        (y / GRID_ROW_STEP).round().max(0.0) as u32,
    )
}

#[derive(Clone, PartialEq, Debug)]
pub struct MindNode {
    pub id: usize,
    pub text: String,
    pub image_name: String,
    pub mission_event: String,
    pub icon_index: usize,
    pub tree_column: u32,
    pub tree_row: u32,
    /// AI 权重（0~100；原样保留，新建卡片默认 100）。
    pub ai: i32,
    pub children: Vec<usize>,
    /// 原本是否使用「标量写法」的标量前置字段（RequiredMission / RequiredMission2）：
    /// 保存时列表写法的条目保持原样，不会被补上 `-1` 标量。
    pub scalar_style: bool,
    /// 列表写法的前置 / 互斥关系（原样保留；保存时按节点新下标重映射）。
    pub required_missions: Option<Vec<i64>>,
    pub required_missions_or: Option<Vec<i64>>,
    pub required_missions_or2: Option<Vec<i64>>,
    pub required_missions_or3: Option<Vec<i64>>,
    pub mutually_exclusive_missions: Option<Vec<i64>>,
    /// 工具未注册的未知字段（原样保留，保存时带回）。
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// 共享只读数据句柄：克隆只增加引用计数，相等按指针判断。
/// 用于标签页数据（missions / icons），避免父级每次重渲染都深拷贝整棵树。
#[derive(Clone)]
pub struct Shared<T>(Rc<T>);

impl<T> Shared<T> {
    pub fn new(value: T) -> Self {
        Self(Rc::new(value))
    }
}

impl<T> std::ops::Deref for Shared<T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.0.as_ref()
    }
}

impl<T> PartialEq for Shared<T> {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

/// 图标数据 URL 的共享句柄：克隆只增加引用计数、相等按指针判断，
/// 画布平移/缩放帧不再反复深拷贝与深比较较大的 base64 字符串。
#[derive(Clone)]
pub struct SharedIconData(Rc<str>);

impl SharedIconData {
    fn new(data_url: &str) -> Self {
        Self(Rc::from(data_url))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for SharedIconData {
    fn default() -> Self {
        Self(Rc::from(""))
    }
}

impl PartialEq for SharedIconData {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

/// 解析卡片实际使用的事件脚本文件名：优先使用 MissionEvent 字段，缺失时回退为「标题.txt」。
fn mission_event_file(mission_event: &str, name: &str) -> String {
    let trimmed = mission_event.trim();
    if trimmed.is_empty() {
        format!("{}.txt", name)
    } else {
        trimmed.to_string()
    }
}

/// 保存时把引用旧节点编号的 ID 列表重映射为新的节点下标；引用了已删除节点的
/// 条目被丢弃，重映射后为空时返回 `None`（不产生空数组字段）。
fn remap_id_list(ids: Option<&[i64]>, remap: &HashMap<i64, i64>) -> Option<Vec<i64>> {
    let ids = ids?;
    let mapped: Vec<i64> = ids
        .iter()
        .filter_map(|id| remap.get(id).copied())
        .collect();
    if mapped.is_empty() {
        None
    } else {
        Some(mapped)
    }
}

fn nearest_free_grid_position(
    nodes: &[MindNode],
    column: u32,
    row: u32,
    ignored_node_id: Option<usize>,
) -> (u32, u32) {
    let is_free = |candidate_column, candidate_row| {
        !nodes.iter().any(|node| {
            Some(node.id) != ignored_node_id
                && node.tree_column == candidate_column
                && node.tree_row == candidate_row
        })
    };

    let max_radius = nodes.len().min(u32::MAX as usize) as u32;
    for radius in 0..=max_radius {
        for column_offset in 0..=radius {
            let row_offset = radius - column_offset;
            let candidate_columns = [
                column.checked_sub(column_offset),
                column.checked_add(column_offset),
            ];
            let candidate_rows = [row.checked_sub(row_offset), row.checked_add(row_offset)];

            for candidate_column in candidate_columns
                .into_iter()
                .flatten()
            {
                for candidate_row in candidate_rows
                    .into_iter()
                    .flatten()
                {
                    if is_free(candidate_column, candidate_row) {
                        return (candidate_column, candidate_row);
                    }
                }
            }
        }
    }

    unreachable!("a free grid position must exist")
}

#[derive(Clone, PartialEq)]
pub struct MindMapState {
    pub nodes: Vec<MindNode>,
    pub pan_x: f64,
    pub pan_y: f64,
    pub zoom: f64,
    pub dragging: Option<usize>,
    pub panning_from: Option<(f64, f64, f64, f64)>,
    pub connecting_from: Option<usize>,
    pub drag_offset_x: f64,
    pub drag_offset_y: f64,
    pub active_pointers: BTreeMap<i32, (f64, f64)>,
    pub pinch_from: Option<(f64, f64, f64, f64)>,
    pub highlighted: Option<usize>,
}

impl MindMapState {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            pan_x: NODE_WIDTH / 2.0,
            pan_y: NODE_HEIGHT / 2.0,
            zoom: initial_zoom(),
            dragging: None,
            panning_from: None,
            connecting_from: None,
            drag_offset_x: 0.0,
            drag_offset_y: 0.0,
            active_pointers: BTreeMap::new(),
            pinch_from: None,
            highlighted: None,
        }
    }

    pub fn from_missions(missions: &[MissionRecord], icons: &[FocusIcon]) -> Self {
        let mut state = Self::new();
        let icon_index_by_name: HashMap<String, usize> = icons
            .iter()
            .enumerate()
            .map(|(index, icon)| (format!("{}.png", icon.name), index))
            .collect();
        state.nodes = missions
            .iter()
            .enumerate()
            .map(|(id, mission)| MindNode {
                id,
                text: mission.name.clone(),
                image_name: mission.image_name.clone(),
                mission_event: mission_event_file(&mission.mission_event, &mission.name),
                // 找不到图片（图标文件缺失 / ImageName 为空）标记为「无图标」：
                // 按游戏原版逻辑，没有图片的国策显示为纯黑卡片。
                icon_index: icon_index_by_name
                    .get(&mission.image_name)
                    .copied()
                    .unwrap_or(usize::MAX),
                tree_column: mission.tree_column,
                tree_row: mission.tree_row,
                ai: mission.ai,
                children: Vec::new(),
                scalar_style: mission.required_mission.is_some()
                    || mission.required_mission2.is_some(),
                required_missions: mission.required_missions.clone(),
                required_missions_or: mission.required_missions_or.clone(),
                required_missions_or2: mission.required_missions_or2.clone(),
                required_missions_or3: mission.required_missions_or3.clone(),
                mutually_exclusive_missions: mission.mutually_exclusive_missions.clone(),
                extra: mission.extra.clone(),
            })
            .collect();

        for (child_index, mission) in missions.iter().enumerate() {
            for parent_id in [mission.required_mission, mission.required_mission2]
                .into_iter()
                .flatten()
            {
                if parent_id < 0 {
                    continue;
                }
                if let Some(parent_index) = missions.iter().position(|item| item.id == parent_id) {
                    let child_id = state.nodes[child_index].id;
                    let parent_count = state
                        .nodes
                        .iter()
                        .filter(|node| node.children.contains(&child_id))
                        .count();
                    if parent_count >= 2 {
                        break;
                    }
                    let parent = &mut state.nodes[parent_index];
                    if !parent.children.contains(&child_id) {
                        parent.children.push(child_id);
                    }
                }
            }
        }

        state
    }

    pub fn node(&self, id: usize) -> Option<&MindNode> {
        self.nodes.iter().find(|node| node.id == id)
    }

    pub fn node_mut(&mut self, id: usize) -> Option<&mut MindNode> {
        self.nodes.iter_mut().find(|node| node.id == id)
    }
}

/// 应用节点快照（供全局撤销/重做执行器调用）。
fn apply_nodes_snapshot(state: &mut Signal<MindMapState>, nodes: Vec<MindNode>) {
    state.with_mut(|current| {
        current.nodes = nodes;
        current.dragging = None;
        current.panning_from = None;
        current.connecting_from = None;
        current.highlighted = None;
    });
}

/// 思维导图聚焦目标：按卡片标题或按图标文件名定位。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum FocusTarget {
    Title(String),
    Image(String),
}

#[component]
pub fn MindMapCanvas(
    initial_icons: Shared<Vec<FocusIcon>>,
    missions: Shared<Vec<MissionRecord>>,
    // 本树所属资源根（missions / assets/game/missions / 剧本 missions）。
    missions_root: String,
    // 工作区现有事件脚本的相对路径（判断「空国策」并提供 新建事件 / 链接事件）。
    event_files: Shared<Vec<String>>,
    active_tab_id: Signal<Option<String>>,
    tab_id: String,
    on_save: EventHandler<Vec<MissionRecord>>,
    save_request: Signal<u64>,
    save_status: String,
    on_edit_event: EventHandler<String>,
    on_create_event_file: EventHandler<(String, String)>,
    on_delete_event_file: EventHandler<String>,
    on_rename_event_file: EventHandler<(String, String)>,
    focus_request: Signal<Option<(FocusTarget, u64)>>,
    on_nodes_change: EventHandler<(Vec<String>, Vec<String>)>,
    on_dirty_change: EventHandler<bool>,
    // 资源管理器双击 .png 手动载入的图标：(目标标签页 id, 图标)。
    pending_icon: Signal<Option<(String, FocusIcon)>>,
    save_ack: u64,
    // 编辑操作注册到全局撤销栈（由 Work 统一管理栈与快捷键/按钮）。
    on_undo_push: EventHandler<UndoRegistration>,
) -> Element {
    // 每个标签页独立持有本树的图标列表：关闭标签页时随组件一起释放。
    let icons = use_signal(|| (*initial_icons).clone());
    // 图标数据 URL 的共享句柄表：仅在图标列表变化时重建；
    // 画布交互帧内每个节点只克隆引用计数（Rc），不再深拷贝 base64 字符串。
    let icon_sources = use_memo(move || {
        icons
            .read()
            .iter()
            .map(|icon| SharedIconData::new(&icon.data_url))
            .collect::<Vec<SharedIconData>>()
    });
    let tab_id = use_signal(|| tab_id);
    let mut state = use_signal(|| {
        let mission_slice: &[MissionRecord] = &missions;
        let icon_ref = icons.peek();
        let icon_slice: &[FocusIcon] = &icon_ref;
        MindMapState::from_missions(mission_slice, icon_slice)
    });
    // 与磁盘内容一致的节点快照：脏标记与保存确认都以它为基准。
    let mut initial_nodes = use_signal(|| state.read().nodes.clone());
    let mut context_menu = use_signal(|| None::<(f64, f64, Option<usize>)>);
    let mut new_node_text = use_signal(|| "新国策".to_string());
    let mut selected_icon = use_signal(|| 0_usize);
    let mut icon_picker_open = use_signal(|| false);
    // 「链接事件」选择器：为没有事件的空国策列出本资源根下现有的事件脚本。
    let mut linking_event_open = use_signal(|| false);
    let mut editing_node = use_signal(|| None::<usize>);
    let mut confirming_delete = use_signal(|| None::<usize>);
    let mut new_node_column = use_signal(|| 0_u32);
    let mut new_node_row = use_signal(|| 0_u32);
    let mut last_save_request = use_signal(|| *save_request.read());

    // 待登记的变更前快照：commit_history 只记录「操作前的节点状态」，
    // 真正的撤销步骤由下面的 effect 在节点确实发生变化后再注册。
    // 这样第一次修改就能撤销（旧实现用「初始快照」去重，导致第一次修改被跳过），
    // 且点击卡片（未拖动）等无变化操作不会产生空记录。
    let mut pending_commit = use_signal(|| None::<Vec<MindNode>>);
    // 每次结构变更前记录「变更前快照」。
    // 注：闭包内重绑定为 mut 局部副本，保持闭包整体为 Fn/Copy（可从多个回调中调用）。
    let commit_history = move || {
        let mut pending_commit = pending_commit;
        pending_commit.set(Some(state.read().nodes.clone()));
    };
    // 节点实际变化时注册一步撤销：无变化不注册。
    // 撤销/重做应用快照前会清空待登记状态，避免把撤销本身再次登记为新编辑。
    use_effect(move || {
        let before = { pending_commit.read().as_ref().cloned() };
        let Some(before) = before else {
            return;
        };
        if state.read().nodes == before {
            return;
        }
        pending_commit.set(None);
        // 撤销/重做执行器共享一个槽：撤销时记下「被撤销掉的状态」，重做时恢复它。
        let slot = Rc::new(RefCell::new(Vec::<MindNode>::new()));
        let mut state_signal = state;
        let slot_for_undo = Rc::clone(&slot);
        let before_for_undo = before;
        let mut pending_for_undo = pending_commit;
        let undo = EventHandler::new(move |_: ()| {
            let current = state_signal.read().nodes.clone();
            *slot_for_undo.borrow_mut() = current;
            pending_for_undo.set(None);
            apply_nodes_snapshot(&mut state_signal, before_for_undo.clone());
        });
        let slot_for_redo = Rc::clone(&slot);
        let mut pending_for_redo = pending_commit;
        let redo = EventHandler::new(move |_: ()| {
            let target = slot_for_redo.borrow().clone();
            pending_for_redo.set(None);
            apply_nodes_snapshot(&mut state_signal, target);
        });
        on_undo_push.call((UndoScope::Tab(tab_id.read().clone()), undo, redo));
    });

    // 节点变化时上报（标题列表、图标列表），供资源管理器删除前检查占用。
    // 仅激活中的标签页上报，避免多个挂载中的画布互相覆盖。
    let last_reported_nodes = use_signal(|| None::<(Vec<String>, Vec<String>)>);
    use_effect(move || {
        let mut last_reported_nodes = last_reported_nodes;
        let is_active =
            active_tab_id.read().as_deref() == Some(tab_id.read().as_str());
        if !is_active {
            if last_reported_nodes.peek().is_some() {
                last_reported_nodes.set(None);
            }
            return;
        }
        // 与上次上报的快照逐项比较（不克隆全量列表）：拖动/平移帧的开销恒定且极小。
        let current = state.read();
        let reported_matches = last_reported_nodes
            .read()
            .as_ref()
            .is_some_and(|(names, images)| {
                names.len() == current.nodes.len()
                    && images.len() == current.nodes.len()
                    && names.iter().zip(&current.nodes).all(|(name, node)| *name == node.text)
                    && images
                        .iter()
                        .zip(&current.nodes)
                        .all(|(image, node)| *image == node.image_name)
            });
        if reported_matches {
            return;
        }
        let names: Vec<String> = current.nodes.iter().map(|node| node.text.clone()).collect();
        let images: Vec<String> = current
            .nodes
            .iter()
            .map(|node| node.image_name.clone())
            .collect();
        let snapshot = (names, images);
        last_reported_nodes.set(Some(snapshot.clone()));
        on_nodes_change.call(snapshot);
    });

    // 保存成功（父级递增 save_ack）后，以「发起保存时的快照」为新基准清除脏标记。
    // 注意：`save_ack` 是普通 props 而非信号——必须经 `use_reactive` 声明为依赖，
    // 否则组件因父级状态重渲染时本 effect 不会重跑，脏标记永久残留
    //（表现为「保存后关闭标签页仍提示有未保存的修改」）。
    // 用保存请求时的快照而非当前节点：保存期间的新编辑仍保持脏标记。
    let mut pending_save_snapshot = use_signal(|| None::<Vec<MindNode>>);
    use_effect(use_reactive((&save_ack,), move |(_ack,)| {
        let snapshot = pending_save_snapshot.peek().clone();
        if let Some(snapshot) = snapshot {
            pending_save_snapshot.set(None);
            initial_nodes.set(snapshot);
        }
    }));

    // 节点与基准不一致时向父级上报脏状态（关闭标签页前提示用）。
    let mut last_dirty = use_signal(|| false);
    use_effect(move || {
        // 直接借用比较（不克隆）：拖动/平移帧不再复制两份全部节点。
        let dirty = state.read().nodes != *initial_nodes.read();
        if dirty == *last_dirty.read() {
            return;
        }
        last_dirty.set(dirty);
        on_dirty_change.call(dirty);
    });

    use_effect(move || {
        let request = *save_request.read();
        if request == *last_save_request.read() {
            return;
        }
        // 非激活标签页消费保存请求但不落盘，避免切换标签时被静默保存。
        last_save_request.set(request);
        let is_active =
            active_tab_id.read().as_deref() == Some(tab_id.read().as_str());
        if !is_active {
            return;
        }

        // 记录「发起保存时」的节点快照：保存成功后以它为已落盘基准（见 save_ack effect），
        // 保存期间的新编辑仍保持脏标记，避免把未落盘的改动当作已保存。
        let snapshot = state.read().nodes.clone();
        pending_save_snapshot.set(Some(snapshot));

        let current = state.read();
        // 保存时节点按新下标重排 ID：列表写法的引用按「旧编号 → 新下标」重映射
        // （引用了已删除节点的条目被丢弃）。
        let id_remap: HashMap<i64, i64> = current
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.id as i64, index as i64))
            .collect();
        let records = current
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| {
                let parents: Vec<i64> = current
                    .nodes
                    .iter()
                    .enumerate()
                    .filter(|(_, parent)| parent.children.contains(&node.id))
                    .map(|(parent_index, _)| parent_index as i64)
                    .take(2)
                    .collect();
                // 老写法（标量）只在「有连线」或「原本就用标量」时写出；
                // 列表写法的条目保持原样，不被补上 -1 标量。
                let (required_mission, required_mission2) = if !parents.is_empty() {
                    (Some(parents[0]), Some(parents.get(1).copied().unwrap_or(-1)))
                } else if node.scalar_style {
                    (Some(-1), Some(-1))
                } else {
                    (None, None)
                };
                MissionRecord {
                    id: index as i64,
                    name: node.text.clone(),
                    image_name: node.image_name.clone(),
                    mission_event: mission_event_file(&node.mission_event, &node.text),
                    tree_column: node.tree_column,
                    tree_row: node.tree_row,
                    required_mission,
                    required_mission2,
                    required_missions: remap_id_list(node.required_missions.as_deref(), &id_remap),
                    required_missions_or: remap_id_list(
                        node.required_missions_or.as_deref(),
                        &id_remap,
                    ),
                    required_missions_or2: remap_id_list(
                        node.required_missions_or2.as_deref(),
                        &id_remap,
                    ),
                    required_missions_or3: remap_id_list(
                        node.required_missions_or3.as_deref(),
                        &id_remap,
                    ),
                    mutually_exclusive_missions: remap_id_list(
                        node.mutually_exclusive_missions.as_deref(),
                        &id_remap,
                    ),
                    ai: node.ai,
                    extra: node.extra.clone(),
                }
            })
            .collect();
        on_save.call(records);
    });

    // 资源管理器双击 → 视图聚焦到对应卡片并高亮（仅激活标签页响应）。
    let mut last_focus_request = use_signal(|| None::<(FocusTarget, u64)>);
    use_effect(move || {
        let is_active =
            active_tab_id.read().as_deref() == Some(tab_id.read().as_str());
        if !is_active {
            return;
        }
        let Some(request) = focus_request.read().clone() else {
            return;
        };
        if *last_focus_request.read() == Some(request.clone()) {
            return;
        }
        last_focus_request.set(Some(request.clone()));
        let (target, _) = request;
        let mut current = state.write();
        let focus_target = match &target {
            FocusTarget::Title(title) => current
                .nodes
                .iter()
                .find(|node| node.text == *title)
                .map(|node| (node.tree_column, node.tree_row, node.id)),
            FocusTarget::Image(image) => current
                .nodes
                .iter()
                .find(|node| node.image_name == *image)
                .map(|node| (node.tree_column, node.tree_row, node.id)),
        };
        if let Some((column, row, node_id)) = focus_target {
            let (world_x, world_y) = grid_to_world(column, row);
            current.pan_x = CANVAS_WIDTH / (2.0 * current.zoom) - world_x;
            current.pan_y = CANVAS_HEIGHT / (2.0 * current.zoom) - world_y;
            current.highlighted = Some(node_id);
        }
    });

    // 资源管理器手动载入图标 → 并入对应标签页的图标列表（其余画布忽略）。
    let mut last_pending_icon = use_signal(|| None::<(String, FocusIcon)>);
    use_effect(move || {
        let Some(request) = pending_icon.read().clone() else {
            return;
        };
        if *last_pending_icon.read() == Some(request.clone()) {
            return;
        }
        let (request_tab, icon) = request;
        if request_tab != *tab_id.read() {
            return;
        }
        last_pending_icon.set(Some((request_tab, icon.clone())));
        let mut icons = icons;
        let mut own = icons.write();
        let index = own
            .iter()
            .position(|existing| existing.name == icon.name)
            .unwrap_or_else(|| {
                own.push(icon);
                own.len() - 1
            });
        let mut selected_icon = selected_icon;
        selected_icon.set(index);
    });

    let canvas_w = CANVAS_WIDTH;
    let canvas_h = CANVAS_HEIGHT;

    let screen_to_world = move |screen_x: f64, screen_y: f64| -> (f64, f64) {
        let current = state.read();
        (
            screen_x / current.zoom - current.pan_x,
            screen_y / current.zoom - current.pan_y,
        )
    };

    // 节点拖拽回调：use_callback 保持跨渲染稳定身份，画布重渲染时
    // NodeView 的 props 可整体比较相等而被记忆化跳过（拖拽只重渲染被拖动的卡片）。
    let on_node_drag_start = use_callback(
        move |(node_id, screen_x, screen_y, node_x, node_y): (usize, f64, f64, f64, f64)| {
            let (world_x, world_y) = screen_to_world(screen_x, screen_y);
            commit_history();
            let mut current = state.write();
            if let Some(source_id) = current.connecting_from {
                let parent_count = current
                    .nodes
                    .iter()
                    .filter(|node| node.children.contains(&node_id))
                    .count();
                if source_id != node_id && parent_count < 2 {
                    if let Some(source) = current.node_mut(source_id) {
                        if !source.children.contains(&node_id) {
                            source.children.push(node_id);
                        }
                    }
                }
                current.connecting_from = None;
                current.dragging = None;
                return;
            }
            current.dragging = Some(node_id);
            current.drag_offset_x = world_x - node_x;
            current.drag_offset_y = world_y - node_y;
        },
    );

    let on_pointer_move = move |evt: Event<PointerData>| {
        let position = evt.client_coordinates();
        // 悬停（指针未按下）直接返回：跳过信号写入，
        // 避免鼠标/触摸悬停移动就触发整画布重渲染与派生计算。
        if !state.peek().active_pointers.contains_key(&evt.pointer_id()) {
            return;
        }
        let mut current = state.write();
        current
            .active_pointers
            .insert(evt.pointer_id(), (position.x, position.y));
        if current.active_pointers.len() >= 2 {
            let mut pointers = current.active_pointers.values();
            let (first_x, first_y) = pointers.next().copied().unwrap();
            let (second_x, second_y) = pointers.next().copied().unwrap();
            if let Some((start_distance, start_zoom, world_x, world_y)) = current.pinch_from {
                let midpoint_x = (first_x + second_x) / 2.0;
                let midpoint_y = (first_y + second_y) / 2.0;
                let distance = (second_x - first_x).hypot(second_y - first_y);
                current.zoom = (start_zoom * distance / start_distance).clamp(0.2, 4.0);
                current.pan_x = midpoint_x / current.zoom - world_x;
                current.pan_y = midpoint_y / current.zoom - world_y;
            }
            return;
        }
        if current.dragging.is_none() && current.panning_from.is_none() {
            return;
        }
        let (world_x, world_y) = (
            position.x / current.zoom - current.pan_x,
            position.y / current.zoom - current.pan_y,
        );
        if let Some(node_id) = current.dragging {
            let (column, row) = world_to_grid(
                world_x - current.drag_offset_x,
                world_y - current.drag_offset_y,
            );
            let (column, row) =
                nearest_free_grid_position(&current.nodes, column, row, Some(node_id));
            if let Some(node) = current.node_mut(node_id) {
                node.tree_column = column;
                node.tree_row = row;
            }
        } else if let Some((start_x, start_y, start_pan_x, start_pan_y)) = current.panning_from {
            current.pan_x = (start_pan_x + (position.x - start_x) / current.zoom)
                .min(NODE_WIDTH / 2.0);
            current.pan_y = (start_pan_y + (position.y - start_y) / current.zoom)
                .min(NODE_HEIGHT / 2.0);
        }
    };

    let on_pointer_up = move |evt: Event<PointerData>| {
        let mut current = state.write();
        current.dragging = None;
        current.panning_from = None;
        current.pinch_from = None;
        current.active_pointers.remove(&evt.pointer_id());
        if current.active_pointers.len() >= 2 {
            let mut pointers = current.active_pointers.values();
            let (first_x, first_y) = pointers.next().copied().unwrap();
            let (second_x, second_y) = pointers.next().copied().unwrap();
            let midpoint_x = (first_x + second_x) / 2.0;
            let midpoint_y = (first_y + second_y) / 2.0;
            let distance = (second_x - first_x).hypot(second_y - first_y).max(1.0);
            let world_x = midpoint_x / current.zoom - current.pan_x;
            let world_y = midpoint_y / current.zoom - current.pan_y;
            current.pinch_from = Some((distance, current.zoom, world_x, world_y));
        } else if let Some((x, y)) = current.active_pointers.values().next().copied() {
            current.panning_from = Some((x, y, current.pan_x, current.pan_y));
        }
    };

    let on_canvas_pointer_down = move |evt: Event<PointerData>| {
        let position = evt.client_coordinates();
        let mut current = state.write();
        current
            .active_pointers
            .insert(evt.pointer_id(), (position.x, position.y));
        if current.active_pointers.len() >= 2 {
            let mut pointers = current.active_pointers.values();
            let (first_x, first_y) = pointers.next().copied().unwrap();
            let (second_x, second_y) = pointers.next().copied().unwrap();
            let midpoint_x = (first_x + second_x) / 2.0;
            let midpoint_y = (first_y + second_y) / 2.0;
            let distance = (second_x - first_x).hypot(second_y - first_y).max(1.0);
            let world_x = midpoint_x / current.zoom - current.pan_x;
            let world_y = midpoint_y / current.zoom - current.pan_y;
            current.dragging = None;
            current.panning_from = None;
            current.pinch_from = Some((distance, current.zoom, world_x, world_y));
        } else if current.dragging.is_none() {
            current.panning_from = Some((position.x, position.y, current.pan_x, current.pan_y));
        }
        // 菜单未打开时跳过写入，避免每次按下都触发一次多余重渲染。
        if context_menu.peek().is_some() {
            context_menu.set(None);
        }
    };

    let on_context_menu = move |evt: Event<MouseData>| {
        evt.prevent_default();
        let position = evt.client_coordinates();
        let (world_x, world_y) = screen_to_world(position.x, position.y);
        let (column, row) = world_to_grid(world_x, world_y);
        new_node_text.set("新国策".to_string());
        selected_icon.set(0);
        icon_picker_open.set(false);
        linking_event_open.set(false);
        editing_node.set(None);
        confirming_delete.set(None);
        new_node_column.set(column);
        new_node_row.set(row);
        context_menu.set(Some((position.x, position.y, None)));
    };

    // 节点右键回调同样用 use_callback 保持稳定身份。
    let on_node_context_menu = use_callback(move |(node_id, screen_x, screen_y): (usize, f64, f64)| {
        if let Some(node) = state.read().node(node_id) {
            new_node_column.set(node.tree_column);
            new_node_row.set(node.tree_row.saturating_add(1));
            selected_icon.set(node.icon_index);
        }
        new_node_text.set("新国策".to_string());
        icon_picker_open.set(false);
        linking_event_open.set(false);
        editing_node.set(None);
        confirming_delete.set(None);
        context_menu.set(Some((screen_x, screen_y, Some(node_id))));
    });

    let on_wheel = move |evt: Event<WheelData>| {
        let position = evt.client_coordinates();
        let (world_x, world_y) = screen_to_world(position.x, position.y);
        let mut current = state.write();
        let delta = evt.delta().strip_units().y;
        current.zoom = (current.zoom * (1.0 - delta * 0.001)).clamp(0.2, 4.0);
        current.pan_x = (position.x / current.zoom - world_x).min(NODE_WIDTH / 2.0);
        current.pan_y = (position.y / current.zoom - world_y).min(NODE_HEIGHT / 2.0);
    };

    // 图标句柄表整体借用一次：交互帧内每个节点只克隆引用计数，不再深拷贝 base64 字符串。
    let icon_sources = icon_sources.read();
    let (pan_x, pan_y, zoom, edges, node_items) = {
        let current = state.read();
        let pan_x = current.pan_x;
        let pan_y = current.pan_y;
        let zoom = current.zoom;
        let min_x = -pan_x;
        let min_y = -pan_y;
        let max_x = canvas_w / zoom - pan_x;
        let max_y = canvas_h / zoom - pan_y;
        let positions: HashMap<_, _> = current
            .nodes
            .iter()
            .map(|node| (node.id, grid_to_world(node.tree_column, node.tree_row)))
            .collect();
        let mut edges = Vec::new();
        for node in &current.nodes {
            let from = positions[&node.id];
            for child_id in &node.children {
                let Some(to) = positions.get(child_id).copied() else {
                    continue;
                };
                let edge_min_x = from.0.min(to.0);
                let edge_max_x = from.0.max(to.0);
                let edge_min_y = from.1.min(to.1);
                let edge_max_y = from.1.max(to.1);
                if edge_max_x >= min_x
                    && edge_min_x <= max_x
                    && edge_max_y >= min_y
                    && edge_min_y <= max_y
                {
                    edges.push((from, to));
                }
            }
        }
        let highlighted_id = current.highlighted;
        let node_items: Vec<(MindNode, bool, SharedIconData)> = current
            .nodes
            .iter()
            .filter_map(|node| {
                let (center_x, center_y) = positions[&node.id];
                let is_visible = center_x + NODE_WIDTH / 2.0 >= min_x
                    && center_x - NODE_WIDTH / 2.0 <= max_x
                    && center_y + NODE_HEIGHT / 2.0 >= min_y
                    && center_y - NODE_HEIGHT / 2.0 <= max_y;
                is_visible.then(|| {
                    let icon = icon_sources.get(node.icon_index).cloned().unwrap_or_default();
                    (node.clone(), highlighted_id == Some(node.id), icon)
                })
            })
            .collect();
        (pan_x, pan_y, zoom, edges, node_items)
    };
    let menu_position = *context_menu.read();
    let node_text = new_node_text.read().clone();
    let node_column = *new_node_column.read();
    let node_row = *new_node_row.read();
    let selected_icon_index = *selected_icon.read();
    let picker_open = *icon_picker_open.read();
    let linking_open = *linking_event_open.read();
    // 右键目标卡片是否没有事件脚本（空国策：菜单提供 新建事件 / 链接事件）。
    let target_event_missing = menu_position
        .and_then(|(_, _, target)| target)
        .is_some_and(|node_id| {
            state.read().node(node_id).is_some_and(|node| {
                let file = mission_event_file(&node.mission_event, &node.text);
                let path = format!("{missions_root}/missionsEvents/{file}");
                !event_files.iter().any(|existing| existing == &path)
            })
        });
    // 右键目标卡片是否连有依赖线（双向）：「取消连接」据此决定是否可用。
    let target_has_edges = menu_position
        .and_then(|(_, _, target)| target)
        .is_some_and(|node_id| {
            state.read().nodes.iter().any(|node| {
                (node.id == node_id && !node.children.is_empty())
                    || node.children.contains(&node_id)
            })
        });
    // 「链接事件」候选：本资源根下现有的事件脚本名（仅在选择器打开时快照）。
    let link_event_items: Vec<(String, EventHandler<()>)> = if linking_open {
        let prefix = format!("{missions_root}/missionsEvents/");
        let mut names: Vec<String> = event_files
            .iter()
            .filter_map(|path| path.strip_prefix(&prefix).map(str::to_string))
            .collect();
        names.sort();
        let target = menu_position.and_then(|(_, _, target)| target);
        names
            .into_iter()
            .map(|name| {
                let chosen = name.clone();
                let mut state = state;
                let mut context_menu = context_menu;
                let mut linking_event_open = linking_event_open;
                let commit_history = commit_history;
                let target = target;
                let handler = EventHandler::new(move |_: ()| {
                    commit_history();
                    let mut current = state.write();
                    if let Some(node_id) = target {
                        if let Some(node) = current.node_mut(node_id) {
                            node.mission_event = chosen.clone();
                        }
                    }
                    linking_event_open.set(false);
                    context_menu.set(None);
                });
                (name, handler)
            })
            .collect()
    } else {
        Vec::new()
    };
    // 仅在图标选择器打开时快照图标列表，避免画布平移/缩放时反复克隆所有图标数据。
    // 图标来源：本树已载入的图标 + 用户在资源管理器中双击手动载入的图标；
    // 不再自动加载完整图标目录，避免图标过多时渲染卡死。
    // 选择回调预先克隆名称，避免在 rsx 循环体中移动变量。
    let icon_items: Vec<(String, String, EventHandler<()>)> = if picker_open {
        icons
            .read()
            .iter()
            .map(|icon| {
                let name = icon.name.clone();
                let icons = icons;
                let mut selected_icon = selected_icon;
                let mut new_node_text = new_node_text;
                let handler = EventHandler::new(move |_: ()| {
                    let index = icons
                        .read()
                        .iter()
                        .position(|icon| icon.name == name)
                        .unwrap_or(0);
                    selected_icon.set(index);
                    new_node_text.set(name.clone());
                });
                (icon.name.clone(), icon.data_url.clone(), handler)
            })
            .collect()
    } else {
        Vec::new()
    };
    let editing_node_id = *editing_node.read();
    let menu_max_height = if picker_open || linking_open {
        344
    } else if editing_node_id.is_some() {
        150
    } else if matches!(menu_position, Some((_, _, Some(_)))) {
        // 卡片菜单：空国策多出「新建事件 / 链接事件」两项（每项约 34px）。
        if target_event_missing {
            244
        } else {
            210
        }
    } else {
        52
    };
    let menu_vertical_offset = menu_max_height + 16;
    let is_connecting = state.read().connecting_from.is_some();
    // 本组件不再自管撤销历史：所有编辑操作已注册到 Work 的全局撤销栈。

    let grid_x = -pan_x - GRID_COLUMN_STEP;
    let grid_y = -pan_y - GRID_ROW_STEP;
    let grid_w = canvas_w / zoom + GRID_COLUMN_STEP * 2.0;
    let grid_h = canvas_h / zoom + GRID_ROW_STEP * 2.0;

    rsx! {
        div {
            style: "position: relative; width: 100%; height: 100%; min-width: 0; min-height: 0; overflow: hidden;",
            tabindex: "0",
            // Ctrl+Z / Ctrl+Y 由 Work 的全局键盘处理（统一撤销整个编辑器的操作）。
            // 卡片其它交互（拖动/连线等）不受影响。
            svg {
                width: "100%",
                height: "100%",
                view_box: "0 0 {canvas_w} {canvas_h}",
                style: "display: block; background: #f7f9ff; cursor: grab; user-select: none; touch-action: none;",
                onpointermove: on_pointer_move,
                onpointerup: on_pointer_up,
                onpointercancel: on_pointer_up,
                onpointerdown: on_canvas_pointer_down,
                oncontextmenu: on_context_menu,
                onwheel: on_wheel,

                defs {
                    pattern {
                        id: "grid",
                        width: "{GRID_COLUMN_STEP}",
                        height: "{GRID_ROW_STEP}",
                        pattern_units: "userSpaceOnUse",
                        path {
                            d: "M {GRID_COLUMN_STEP} 0 L 0 0 0 {GRID_ROW_STEP}",
                            fill: "none",
                            stroke: "#dce5f4",
                            stroke_width: "1",
                        }
                    }
                }

                g { transform: "translate({pan_x * zoom}, {pan_y * zoom}) scale({zoom})",
                    rect {
                        x: "{grid_x}",
                        y: "{grid_y}",
                        width: "{grid_w}",
                        height: "{grid_h}",
                        fill: "url(#grid)",
                    }

                    for (from , to) in edges {
                        EdgeView { from, to }
                    }

                    for (node , highlighted , icon) in node_items {
                        NodeView {
                            node,
                            icon,
                            highlighted,
                            on_drag_start: on_node_drag_start,
                            on_context_menu: on_node_context_menu,
                        }
                    }
                }
            }

            if !save_status.is_empty() {
                div {
                    role: "status",
                    // z-index 25：浮在资源管理器/事件面板（.drawer z 20）之上；标题栏(30)、菜单(40)、弹窗(60)仍在更上层。
                    style: "position: absolute; top: 12px; right: 12px; z-index: 25; padding: 8px; background: white; border: 1px solid #dce5f4; border-radius: 6px; box-shadow: 0 2px 8px rgba(71, 98, 145, 0.12);",
                    "{save_status}"
                }
            }

            if is_connecting {
                div {
                    role: "status",
                    // 连接提示同样浮在所有面板之上，避免被资源管理器遮住。
                    style: "position: absolute; top: 12px; left: 12px; z-index: 25; display: flex; align-items: center; gap: 10px; padding: 8px 10px; background: white; border: 1px solid #80a6e6; border-radius: 6px; box-shadow: 0 2px 8px rgba(71, 98, 145, 0.12);",
                    span { "连接模式：点击另一张卡片完成连线" }
                    button {
                        r#type: "button",
                        onclick: move |_| state.write().connecting_from = None,
                        "取消"
                    }
                }
            }

            // 上一步 / 下一步按钮已迁移到主窗口标题栏（Frame）。
            if let Some((client_x, client_y, target_node)) = menu_position {
                div {
                    role: "menu",
                    class: "context-menu",
                    style: "position: fixed; z-index: 40; left: max(8px, min({client_x}px, calc(100vw - 216px))); top: max(8px, min({client_y}px, calc(100vh - {menu_vertical_offset}px))); width: min(200px, calc(100vw - 16px)); max-height: min({menu_max_height}px, calc(100vh - 16px));",
                    if picker_open {
                        button {
                            r#type: "button",
                            onclick: move |_| icon_picker_open.set(false),
                            "返回"
                        }
                        input {
                            value: "{node_text}",
                            placeholder: "卡片标题（仅用于识别）",
                            style: "width: 100%; min-width: 0; box-sizing: border-box;",
                            oninput: move |evt: FormEvent| new_node_text.set(evt.value()),
                        }
                        div {
                            class: "context-icon-list",
                            style: "max-height: 136px; overflow-y: auto; overscroll-behavior: contain; display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 6px;",
                            if icons.read().is_empty() {
                                span { style: "grid-column: 1 / -1; padding: 6px 4px; font-size: 11px; line-height: 1.4; text-align: center;",
                                    "暂无图标：请在资源管理器中双击 .png 图标文件加载"
                                }
                            }
                            for (icon_name , icon_source , select_handler) in icon_items {
                                button {
                                    r#type: "button",
                                    title: "{icon_name}",
                                    aria_label: "{icon_name}",
                                    onclick: move |_| select_handler.call(()),
                                    style: if icons.read().get(selected_icon_index).is_some_and(|icon| icon.name == icon_name) { "position: relative; min-width: 0; height: 60px; padding: 0; overflow: hidden; border: none; border-radius: 2px; background: rgba(128, 166, 230, 0.55); box-shadow: none; color: white;" } else { "position: relative; min-width: 0; height: 60px; padding: 0; overflow: hidden; border: none; border-radius: 2px; background: transparent; box-shadow: none; color: white;" },
                                    img {
                                        src: "{icon_source}",
                                        alt: "",
                                        style: "display: block; width: 100%; height: 100%; object-fit: contain; pointer-events: none;",
                                    }
                                    span { style: "position: absolute; right: 2px; bottom: 6px; left: 2px; display: block; overflow: hidden; text-align: center; text-overflow: ellipsis; white-space: nowrap; font-size: 11px; line-height: 1; color: #ffffff; text-shadow: -1px -1px 1px #33415c, 1px -1px 1px #33415c, -1px 1px 1px #33415c, 1px 1px 1px #33415c; pointer-events: none;",
                                        "{icon_name}"
                                    }
                                }
                            }
                        }
                        if !icons.read().is_empty() {
                            p { style: "margin: 0; font-size: 10px; line-height: 1.3; text-align: center; opacity: 0.75;",
                                "双击资源管理器中的 .png 可载入更多图标"
                            }
                        }
                        div { style: "display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 8px; min-width: 0; overflow-x: hidden;",
                            input {
                                r#type: "number",
                                min: "0",
                                step: "1",
                                value: "{node_column}",
                                title: "列",
                                aria_label: "列",
                                style: "width: 100%; min-width: 0; box-sizing: border-box; padding: 4px 6px;",
                                oninput: move |evt: FormEvent| {
                                    new_node_column.set(evt.value().parse::<u32>().unwrap_or(0));
                                },
                            }
                            input {
                                r#type: "number",
                                min: "0",
                                step: "1",
                                value: "{node_row}",
                                title: "行",
                                aria_label: "行",
                                style: "width: 100%; min-width: 0; box-sizing: border-box; padding: 4px 6px;",
                                oninput: move |evt: FormEvent| {
                                    new_node_row.set(evt.value().parse::<u32>().unwrap_or(0));
                                },
                            }
                        }
                        button {
                            r#type: "button",
                            disabled: icons.read().is_empty(),
                            onclick: move |_| {
                                let text = new_node_text.read().trim().to_string();
                                if !text.is_empty() && !icons.read().is_empty() {
                                    let event_file_name = format!("{text}.txt");
                                    let event_contents = default_event(&text).to_text();
                                    let id = state
                                        .read()
                                        .nodes
                                        .iter()
                                        .map(|node| node.id)
                                        .max()
                                        .map_or(0, |id| id + 1);
                                    commit_history();
                                    let mut current = state.write();
                                    let (column, row) = nearest_free_grid_position(
                                        &current.nodes,
                                        *new_node_column.read(),
                                        *new_node_row.read(),
                                        None,
                                    );
                                    let icon_index = (*selected_icon.read()).min(icons.read().len() - 1);
                                    current
                                        .nodes
                                        .push(MindNode {
                                            id,
                                            text,
                                            image_name: icons
                                                .read()
                                                .get(icon_index)
                                                .map(|icon| format!("{}.png", icon.name))
                                                .unwrap_or_default(),
                                            mission_event: event_file_name.clone(),
                                            icon_index,
                                            tree_column: column,
                                            tree_row: row,
                                            ai: 100,
                                            children: Vec::new(),
                                            // 新建卡片默认沿用标量写法（与工具历史行为一致）。
                                            scalar_style: true,
                                            required_missions: None,
                                            required_missions_or: None,
                                            required_missions_or2: None,
                                            required_missions_or3: None,
                                            mutually_exclusive_missions: None,
                                            extra: BTreeMap::new(),
                                        });
                                    if let Some(parent_id) = target_node {
                                        if let Some(parent) = current.node_mut(parent_id) {
                                            if !parent.children.contains(&id) {
                                                parent.children.push(id);
                                            }
                                        }
                                    }
                                    current.connecting_from = None;
                                    on_create_event_file.call((event_file_name, event_contents));
                                }
                                context_menu.set(None);
                            },
                            if target_node.is_some() {
                                "创建分支"
                            } else {
                                "创建国策"
                            }
                        }
                    } else if linking_open {
                        button {
                            r#type: "button",
                            onclick: move |_| linking_event_open.set(false),
                            "返回"
                        }
                        div { class: "context-event-list",
                            if link_event_items.is_empty() {
                                span { style: "padding: 6px 4px; font-size: 11px; line-height: 1.4; text-align: center; opacity: 0.75;",
                                    "该资源根下暂无可链接的事件脚本"
                                }
                            }
                            for (event_name , link_handler) in link_event_items {
                                button {
                                    r#type: "button",
                                    title: "{event_name}",
                                    onclick: move |_| link_handler.call(()),
                                    "{event_name}"
                                }
                            }
                        }
                    } else if let Some(edit_node_id) = editing_node_id {
                        input {
                            value: "{node_text}",
                            placeholder: "卡片标题",
                            style: "width: 100%; min-width: 0; box-sizing: border-box;",
                            oninput: move |evt: FormEvent| new_node_text.set(evt.value()),
                        }
                        button {
                            r#type: "button",
                            onclick: move |_| {
                                let text = new_node_text.read().trim().to_string();
                                if !text.is_empty() {
                                    let old_state = state
                                        .read()
                                        .node(edit_node_id)
                                        .map(|node| (node.text.clone(), node.mission_event.clone()));
                                    if let Some((old_text, old_event_file)) = old_state {
                                        if old_text != text {
                                            commit_history();
                                            let new_event_file = format!("{text}.txt");
                                            let mut current = state.write();
                                            if let Some(node) = current.node_mut(edit_node_id) {
                                                node.text = text.clone();
                                                node.mission_event = new_event_file.clone();
                                            }
                                            on_rename_event_file
                                                .call((
                                                    mission_event_file(&old_event_file, &old_text),
                                                    new_event_file,
                                                ));
                                        }
                                    }
                                }
                                editing_node.set(None);
                                context_menu.set(None);
                            },
                            "保存标题"
                        }
                        button {
                            r#type: "button",
                            onclick: move |_| editing_node.set(None),
                            "返回"
                        }
                    } else if let Some(confirm_id) = *confirming_delete.read() {
                        // 仅删除卡片：保留事件脚本文件。
                        button {
                            r#type: "button",
                            onclick: move |_| {
                                commit_history();
                                let mut current = state.write();
                                current.nodes.retain(|node| node.id != confirm_id);
                                for node in &mut current.nodes {
                                    node.children.retain(|child_id| *child_id != confirm_id);
                                }
                                if current.dragging == Some(confirm_id) {
                                    current.dragging = None;
                                }
                                if current.connecting_from == Some(confirm_id) {
                                    current.connecting_from = None;
                                }
                                confirming_delete.set(None);
                                context_menu.set(None);
                            },
                            "删除卡片"
                        }
                        // 仅删除事件脚本文件：保留卡片（卡片仍指向同名脚本）。
                        button {
                            r#type: "button",
                            onclick: move |_| {
                                let event_file_name = state
                                    .read()
                                    .node(confirm_id)
                                    .map(|node| mission_event_file(&node.mission_event, &node.text));
                                if let Some(file_name) = event_file_name {
                                    on_delete_event_file.call(file_name);
                                }
                                confirming_delete.set(None);
                                context_menu.set(None);
                            },
                            "删除事件"
                        }
                        // 删除卡片与事件脚本（原行为）。
                        button {
                            r#type: "button",
                            onclick: move |_| {
                                let event_file_name = state
                                    .read()
                                    .node(confirm_id)
                                    .map(|node| mission_event_file(&node.mission_event, &node.text));
                                commit_history();
                                let mut current = state.write();
                                current.nodes.retain(|node| node.id != confirm_id);
                                for node in &mut current.nodes {
                                    node.children.retain(|child_id| *child_id != confirm_id);
                                }
                                if current.dragging == Some(confirm_id) {
                                    current.dragging = None;
                                }
                                if current.connecting_from == Some(confirm_id) {
                                    current.connecting_from = None;
                                }
                                if let Some(file_name) = event_file_name {
                                    on_delete_event_file.call(file_name);
                                }
                                confirming_delete.set(None);
                                context_menu.set(None);
                            },
                            "删除国策和事件"
                        }
                        button {
                            r#type: "button",
                            onclick: move |_| confirming_delete.set(None),
                            "取消"
                        }
                    } else if let Some(parent_id) = target_node {
                        if target_event_missing {
                            // 空国策（没有事件）：提供 新建事件 / 链接事件。
                            button {
                                r#type: "button",
                                onclick: move |_| {
                                    let node_info = state
                                        .read()
                                        .node(parent_id)
                                        .map(|node| {
                                            (mission_event_file(&node.mission_event, &node.text), node.text.clone())
                                        });
                                    if let Some((file_name, text)) = node_info {
                                        on_create_event_file
                                            .call((file_name, default_event(&text).to_text()));
                                    }
                                    context_menu.set(None);
                                },
                                "新建事件"
                            }
                            button {
                                r#type: "button",
                                onclick: move |_| linking_event_open.set(true),
                                "链接事件"
                            }
                        } else {
                            button {
                                r#type: "button",
                                onclick: move |_| {
                                    if let Some(node) = state.read().node(parent_id) {
                                        on_edit_event.call(mission_event_file(&node.mission_event, &node.text));
                                    }
                                    context_menu.set(None);
                                },
                                "编辑事件"
                            }
                        }
                        button {
                            r#type: "button",
                            onclick: move |_| {
                                if let Some(node) = state.read().node(parent_id) {
                                    new_node_text.set(node.text.clone());
                                }
                                editing_node.set(Some(parent_id));
                            },
                            "修改标题"
                        }
                        button {
                            r#type: "button",
                            onclick: move |_| {
                                new_node_text.set("新国策".to_string());
                                icon_picker_open.set(true);
                            },
                            "创建分支"
                        }
                        button {
                            r#type: "button",
                            onclick: move |_| {
                                let mut current = state.write();
                                current.connecting_from = Some(parent_id);
                                current.dragging = None;
                                context_menu.set(None);
                            },
                            "连接国策"
                        }
                        button {
                            r#type: "button",
                            disabled: !target_has_edges,
                            title: "断开该卡片与其它卡片之间的全部依赖线，使其成为独立卡片",
                            onclick: move |_| {
                                commit_history();
                                let mut current = state.write();
                                // 双向断开：其它卡片不再以它为前置，它自身也不再有前置。
                                for node in current.nodes.iter_mut() {
                                    node.children.retain(|child| *child != parent_id);
                                }
                                if let Some(node) = current.node_mut(parent_id) {
                                    node.children.clear();
                                }
                                context_menu.set(None);
                            },
                            "取消连接"
                        }
                        button {
                            r#type: "button",
                            onclick: move |_| {
                                confirming_delete.set(Some(parent_id));
                            },
                            "删除"
                        }
                    } else {
                        button {
                            r#type: "button",
                            onclick: move |_| {
                                icon_picker_open.set(true);
                            },
                            "创建国策"
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn EdgeView(from: (f64, f64), to: (f64, f64)) -> Element {
    let (x1, y1) = from;
    let (x2, y2) = to;
    let path_d = if (y2 - y1).abs() < f64::EPSILON {
        if x2 >= x1 {
            format!(
                "M {} {y1} L {} {y2}",
                x1 + NODE_WIDTH / 2.0,
                x2 - NODE_WIDTH / 2.0
            )
        } else {
            format!(
                "M {} {y1} L {} {y2}",
                x1 - NODE_WIDTH / 2.0,
                x2 + NODE_WIDTH / 2.0
            )
        }
    } else {
        let (start_y, end_y) = if y2 > y1 {
            (y1 + NODE_HEIGHT / 2.0, y2 - NODE_HEIGHT / 2.0)
        } else {
            (y1 - NODE_HEIGHT / 2.0, y2 + NODE_HEIGHT / 2.0)
        };
        let bend_y = (start_y + end_y) / 2.0;
        format!(
            "M {x1} {start_y} L {x1} {bend_y} L {x2} {bend_y} L {x2} {end_y}"
        )
    };
    rsx! {
        path {
            d: "{path_d}",
            fill: "none",
            stroke: "#000000",
            stroke_width: "2",
            stroke_linecap: "round",
        }
    }
}

#[component]
fn NodeView(
    node: MindNode,
    icon: SharedIconData,
    highlighted: bool,
    on_drag_start: EventHandler<(usize, f64, f64, f64, f64)>,
    on_context_menu: EventHandler<(usize, f64, f64)>,
) -> Element {
    let (center_x, center_y) = grid_to_world(node.tree_column, node.tree_row);
    let x = center_x - NODE_WIDTH / 2.0;
    let y = center_y - NODE_HEIGHT / 2.0;
    let display_text = if node.text.chars().count() > 8 {
        format!("{}…", node.text.chars().take(7).collect::<String>())
    } else {
        node.text.clone()
    };
    let node_id = node.id;

    let on_pointer_down = move |evt: Event<PointerData>| {
        let client = evt.client_coordinates();
        on_drag_start.call((node_id, client.x, client.y, center_x, center_y));
    };
    let on_right_click = move |evt: Event<MouseData>| {
        evt.prevent_default();
        evt.stop_propagation();
        let client = evt.client_coordinates();
        on_context_menu.call((node_id, client.x, client.y));
    };

    rsx! {
        g {
            onpointerdown: on_pointer_down,
            oncontextmenu: on_right_click,
            style: "cursor: move;",
            if highlighted {
                rect {
                    x: "{x - 6.0}",
                    y: "{y - 6.0}",
                    width: "{NODE_WIDTH + 12.0}",
                    height: "{NODE_HEIGHT + 12.0}",
                    rx: "8",
                    fill: "none",
                    stroke: "#f4a261",
                    stroke_width: "3",
                }
            }
            if icon.as_str().is_empty() {
                // 没有图片的国策：按游戏原版逻辑用纯黑卡片代替
                // （也让卡片保持可点击/可拖动：空 <image> 不参与命中测试）。
                rect {
                    x: "{x}",
                    y: "{y}",
                    width: "{NODE_WIDTH}",
                    height: "{NODE_HEIGHT}",
                    rx: "2",
                    fill: "#000000",
                }
            } else {
                image {
                    href: "{icon.as_str()}",
                    x: "{x}",
                    y: "{y}",
                    width: "{NODE_WIDTH}",
                    height: "{NODE_HEIGHT}",
                    preserve_aspect_ratio: "xMidYMid meet",
                }
            }
            text {
                x: "{center_x}",
                y: "{y + NODE_HEIGHT - 16.0}",
                text_anchor: "middle",
                font_size: "22",
                fill: "#ffffff",
                stroke: "#33415c",
                stroke_width: "1.5",
                style: "paint-order: stroke; stroke-linejoin: round; pointer-events: none; font-family: serif;",
                "{display_text}"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mission_record_reads_required_mission_list_style() {
        let record: MissionRecord = serde_json::from_str(
            r#"{"ID":0,"Name":"n","ImageName":"i.png","MissionEvent":"e.txt","TreeColumn":1,"TreeRow":2,"AI":5,"RequiredMissions":[2,3],"RequiredMissionsOR2":[4,5],"MutuallyExclusiveMissions":[6]}"#,
        )
        .expect("列表写法（缺标量字段）应可解析");
        assert_eq!(record.required_mission, None);
        assert_eq!(record.required_mission2, None);
        assert_eq!(record.required_missions.as_deref(), Some(&[2, 3][..]));
        assert_eq!(record.required_missions_or2.as_deref(), Some(&[4, 5][..]));
        assert_eq!(record.mutually_exclusive_missions.as_deref(), Some(&[6][..]));
        // 回传后端时：未写过的字段不产生 null。
        let back = serde_json::to_string(&record).unwrap();
        assert!(back.contains("\"RequiredMissions\":[2,3]"));
        assert!(!back.contains("\"RequiredMission\":"));
    }

    #[test]
    fn mission_record_defaults_missing_base_fields() {
        // 缺省的基础字段按空串 / 0 / 100 读取（road_to_56 等模组存在缺 AI 的条目）。
        let record: MissionRecord = serde_json::from_str(r#"{"ID":5}"#).expect("缺省字段应可解析");
        assert_eq!(record.name, "");
        assert_eq!(record.tree_column, 0);
        assert_eq!(record.tree_row, 0);
        assert_eq!(record.ai, 100);
        assert!(record.required_mission.is_none());
    }

    #[test]
    fn mission_record_preserves_unknown_fields() {
        // 工具未注册的未知字段（未来游戏 / 模组新增键）解析后保留、回传后端时写出，
        // 且会随卡片原样带到保存环节（from_missions 镜像）。
        let record: MissionRecord = serde_json::from_str(
            r#"{"ID":0,"Name":"n","NewRequirement":[1,2],"FutureFlag":true,"ExtraNote":"x"}"#,
        )
        .expect("未知字段应被 flatten 收集");
        assert_eq!(record.extra.len(), 3);
        assert_eq!(record.extra.get("FutureFlag"), Some(&serde_json::json!(true)));
        let back = serde_json::to_string(&record).unwrap();
        assert!(back.contains("\"NewRequirement\":[1,2]"));
        assert!(back.contains("\"FutureFlag\":true"));

        let icons: Vec<FocusIcon> = Vec::new();
        let state = MindMapState::from_missions(&[record], &icons);
        assert_eq!(state.nodes[0].extra.len(), 3);
        assert_eq!(
            state.nodes[0].extra.get("ExtraNote"),
            Some(&serde_json::Value::String("x".to_string()))
        );
    }

    #[test]
    fn remap_id_list_remaps_and_drops_missing() {
        let mut remap: HashMap<i64, i64> = HashMap::new();
        remap.insert(0, 0);
        remap.insert(2, 1);
        // 引用已删除节点（1）的条目被丢弃；结果为空返回 None（不写空数组）。
        assert_eq!(remap_id_list(Some(&[0, 1, 2][..]), &remap), Some(vec![0, 1]));
        assert_eq!(remap_id_list(Some(&[1][..]), &remap), None);
        assert_eq!(remap_id_list(None, &remap), None);
    }
}