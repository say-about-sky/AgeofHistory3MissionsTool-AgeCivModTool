//! 分区撤销/重做：画布、事件编辑器、资源管理器各自独立的操作记录。
//!
//! 每个可撤销操作注册一对执行器（undo / redo，用于应用对应状态快照），
//! 由 `Work` 按操作所属分区归档；标题栏按钮与 Ctrl+Z / Ctrl+Y 只作用于
//! 当前焦点（鼠标/焦点最后落在）的分区。

use dioxus::prelude::*;

/// 操作所属的编辑范围：决定其归档到哪个分区，以及同分区内的合并与清理规则。
#[derive(Clone, PartialEq, Eq)]
pub enum UndoScope {
    /// 国策树标签页（按 tab id，归入画布分区）。
    Tab(String),
    /// 事件脚本文件（按「资源根/missionsEvents/文件名」，归入事件编辑分区）。
    EventFile(String),
    /// 决议定义文件（按决议标签页 id，归入决议编辑分区）。
    DecisionFile(String),
    /// 资源管理器的文件操作（复制/剪切粘贴等），独立分区。
    Explorer,
}

/// 撤销分区：标题栏按钮与 Ctrl+Z/Y 只作用于当前焦点所在分区。
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum UndoZone {
    #[default]
    Canvas,
    Events,
    Decisions,
    Explorer,
}

impl UndoZone {
    /// 供标题栏提示的分区名称。
    pub fn label(self) -> &'static str {
        match self {
            UndoZone::Canvas => "画布",
            UndoZone::Events => "事件编辑",
            UndoZone::Decisions => "决议编辑",
            UndoZone::Explorer => "资源管理器",
        }
    }
}

impl UndoScope {
    /// 该操作归入的分区。
    pub fn zone(&self) -> UndoZone {
        match self {
            UndoScope::Tab(_) => UndoZone::Canvas,
            UndoScope::EventFile(_) => UndoZone::Events,
            UndoScope::DecisionFile(_) => UndoZone::Decisions,
            UndoScope::Explorer => UndoZone::Explorer,
        }
    }
}

/// 三个分区的栈深度快照（驱动标题栏按钮状态）。
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct UndoDepths {
    pub canvas_undo: usize,
    pub canvas_redo: usize,
    pub events_undo: usize,
    pub events_redo: usize,
    pub decisions_undo: usize,
    pub decisions_redo: usize,
    pub explorer_undo: usize,
    pub explorer_redo: usize,
}

/// 一次注册：作用范围 + 撤销执行器 + 重做执行器。
pub type UndoRegistration = (UndoScope, EventHandler<()>, EventHandler<()>);

/// 栈内一条可撤销操作。
#[derive(Clone)]
pub struct UndoEntry {
    pub scope: UndoScope,
    /// 注册（或合并）时间戳（毫秒）：事件编辑在时间窗内合并为一步。
    pub timestamp: f64,
    pub undo: EventHandler<()>,
    pub redo: EventHandler<()>,
}

/// 撤销栈容量上限（超出时丢弃最旧操作）。
pub const UNDO_LIMIT: usize = 200;

/// 事件编辑连续输入的合并窗口（毫秒）。
pub const UNDO_MERGE_WINDOW_MS: f64 = 800.0;

/// 从栈中取出最新一个满足 `applicable` 的条目（跳过当前不可应用的，如其它事件文件的操作）。
pub fn pop_applicable<F: Fn(&UndoEntry) -> bool>(
    stack: &mut Vec<UndoEntry>,
    applicable: F,
) -> Option<UndoEntry> {
    let position = stack.iter().rposition(|entry| applicable(entry))?;
    Some(stack.remove(position))
}

/// 撤销安全网判定：画布条目（`Tab` 作用域）是否仍对应打开的标签页。
///
/// 条目的执行器由子组件作用域持有，标签页关闭 / 画布卸载后执行器与信号即被释放，
/// 再执行会 panic（`Dropped(ValueDroppedError)`，并可能连锁 `RefCell already borrowed`）。
/// 其它作用域（事件、资源管理器）的条目不受此判定影响，由各自分区处理。
pub fn canvas_scope_is_open(scope: &UndoScope, open_ids: &[String]) -> bool {
    match scope {
        UndoScope::Tab(id) => open_ids.iter().any(|open| open == id),
        _ => true,
    }
}

/// 撤销安全网判定：决议编辑条目（`DecisionFile` 作用域）是否仍对应打开的决议标签页。
/// 与画布条目同理：决议标签页关闭后其组件作用域释放，旧的撤销执行器不得再执行。
pub fn decision_scope_is_open(scope: &UndoScope, open_ids: &[String]) -> bool {
    match scope {
        UndoScope::DecisionFile(id) => open_ids.iter().any(|open| open == id),
        _ => true,
    }
}

/// 当前时间（毫秒）。
pub fn now_ms() -> f64 {
    js_sys::Date::now()
}

/// 键盘焦点是否位于显式声明“使用浏览器原生撤销”的输入框内（如资源管理器搜索、
/// 新工作区名称）：这些输入框里的 Ctrl+Z/Y 交给浏览器。
/// 未标记的输入框（如事件表格单元格）仍走分区撤销（即事件编辑器自己的操作记录）。
pub fn focus_wants_native_undo() -> bool {
    web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.active_element())
        .and_then(|element| element.closest("[data-native-undo]").ok().flatten())
        .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canvas_scope_retention_follows_open_tabs() {
        let open = vec!["a.json".to_string()];
        // 仍打开的标签页保留。
        assert!(canvas_scope_is_open(
            &UndoScope::Tab("a.json".to_string()),
            &open
        ));
        // 已关闭 / 被剪除的标签页剪掉。
        assert!(!canvas_scope_is_open(
            &UndoScope::Tab("b.json".to_string()),
            &open
        ));
        // 标签页列表为空（工作区清空）时画布条目全部失效。
        assert!(!canvas_scope_is_open(
            &UndoScope::Tab("a.json".to_string()),
            &[]
        ));
        // 其它分区条目不受影响。
        assert!(canvas_scope_is_open(&UndoScope::Explorer, &open));
        assert!(canvas_scope_is_open(
            &UndoScope::EventFile("missions/missionsEvents/x.txt".to_string()),
            &open
        ));
    }

    #[test]
    fn decision_scope_retention_follows_open_tabs() {
        let open = vec!["decision:a/rainfall/rfEvent_decision.json".to_string()];
        assert!(decision_scope_is_open(
            &UndoScope::DecisionFile("decision:a/rainfall/rfEvent_decision.json".to_string()),
            &open
        ));
        assert!(!decision_scope_is_open(
            &UndoScope::DecisionFile("decision:b/rainfall/rfEvent_decision.json".to_string()),
            &open
        ));
        // 其它分区条目不受该判定影响。
        assert!(decision_scope_is_open(&UndoScope::Tab("a.json".to_string()), &open));
        assert!(decision_scope_is_open(&UndoScope::Explorer, &[]));
    }
}
