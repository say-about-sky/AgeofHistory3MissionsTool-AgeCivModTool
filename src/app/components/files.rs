use dioxus::prelude::*;
use serde::Deserialize;
use std::collections::HashSet;

use super::missions_roots::{missions_root_of, root_ancestors};

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFile {
	pub name: String,
	pub relative_path: String,
	pub is_directory: bool,
}

/// 资源管理器操作命令。命令元组的第二个元素语义：
/// - Cut/Copy/Delete/Rename/Reveal/Sign：操作对象的相对路径；
/// - Paste：粘贴目标目录（空串表示工作区根）。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ExplorerCommand {
	Cut,
	Copy,
	Paste,
	Delete,
	/// 就地重命名（同目录改名；由 Work 弹出名称输入框）。
	Rename,
	Reveal,
	/// 对所选 APK 就地执行 v1+v2+v3 签名。
	Sign,
}

/// 资源管理器剪贴板（剪切/复制状态）。
#[derive(Clone, PartialEq, Debug)]
pub struct ExplorerClipboard {
	pub source_path: String,
	pub source_name: String,
	pub is_directory: bool,
	pub is_cut: bool,
}

fn parent_of(path: &str) -> &str {
	path.rsplit_once('/').map(|(parent, _)| parent).unwrap_or("")
}

/// 行的所有上级目录均已展开时该行可见（虚拟滚动窗口据此选择行）。
fn row_ancestors_expanded(path: &str, expanded: &HashSet<&str>) -> bool {
	let mut cursor = path.rsplit_once('/').map(|(parent, _)| parent);
	while let Some(directory) = cursor {
		if !expanded.contains(directory) {
			return false;
		}
		cursor = directory.rsplit_once('/').map(|(parent, _)| parent);
	}
	true
}

/// 文件行固定高度（与 styles.css 的 .file-row 保持一致），虚拟滚动按行高换算窗口。
const FILE_ROW_HEIGHT: f64 = 27.0;
/// 虚拟滚动：可视区域行数估算（128 行 ≈ 3456px，覆盖任意窗口高度）。
const VIRTUAL_VIEWPORT_ROWS: usize = 128;
/// 虚拟滚动：可视区上下各保留的缓冲行数（快速滚动与节流滞后时不露白）。
const VIRTUAL_BUFFER_ROWS: usize = 32;

/// 右键菜单（仿 VSCode）。
#[component]
fn ExplorerContextMenu(
	x: f64,
	y: f64,
	path: String,
	paste_target_dir: String,
	clipboard_ready: bool,
	/// 所选对象是否为 APK 文件（控制「签名 apk」菜单项显隐）。
	is_apk_file: bool,
	on_command: EventHandler<(ExplorerCommand, String)>,
	on_close: EventHandler<()>,
) -> Element {
	let dispatch = use_callback(move |command: ExplorerCommand| {
		let target = match command {
			ExplorerCommand::Paste => paste_target_dir.clone(),
			_ => path.clone(),
		};
		on_command.call((command, target));
		on_close.call(());
	});

	rsx! {
        div {
            class: "menu-dismiss",
            aria_hidden: "true",
            onclick: move |_| on_close.call(()),
        }
        div {
            class: "context-menu explorer-context-menu",
            role: "menu",
            style: "position: fixed; z-index: 20; left: max(8px, min({x}px, calc(100vw - 200px))); top: max(8px, min({y}px, calc(100vh - 200px)));",
            button {
                r#type: "button",
                onclick: move |_| dispatch.call(ExplorerCommand::Cut),
                "剪切"
            }
            button {
                r#type: "button",
                onclick: move |_| dispatch.call(ExplorerCommand::Copy),
                "复制"
            }
            button {
                r#type: "button",
                disabled: !clipboard_ready,
                onclick: move |_| dispatch.call(ExplorerCommand::Paste),
                "粘贴"
            }
            button {
                r#type: "button",
                onclick: move |_| dispatch.call(ExplorerCommand::Rename),
                "重命名"
            }
            if is_apk_file {
                button {
                    r#type: "button",
                    onclick: move |_| dispatch.call(ExplorerCommand::Sign),
                    "✒ 签名 apk（v1+v2+v3）"
                }
            }
            button {
                r#type: "button",
                onclick: move |_| dispatch.call(ExplorerCommand::Delete),
                "删除"
            }
            button {
                r#type: "button",
                onclick: move |_| dispatch.call(ExplorerCommand::Reveal),
                "打开文件位置"
            }
        }
    }
}

/// 资源管理器单行（虚拟滚动把可视行渲染为带 key 的独立组件）：
/// 滚动/选中变化时只有实际变化的行会重渲染，大目录下滚动与点选开销近似 O(窗口行数)。
#[component]
fn FileRow(
	entry: WorkspaceFile,
	index: usize,
	depth: usize,
	is_searching: bool,
	is_expanded: bool,
	is_selected: bool,
	on_activate: EventHandler<(String, bool)>,
	on_context: EventHandler<(String, f64, f64)>,
	on_open_file: EventHandler<String>,
) -> Element {
	let path = entry.relative_path.clone();
	let name = entry.name.clone();
	let is_directory = entry.is_directory;
	let on_row_click = {
		let path = path.clone();
		move |_| on_activate.call((path.clone(), is_directory))
	};
	let on_right_click = {
		let path = path.clone();
		move |evt: Event<MouseData>| {
			evt.prevent_default();
			evt.stop_propagation();
			let client = evt.client_coordinates();
			on_context.call((path.clone(), client.x, client.y));
		}
	};
	let on_double_click = {
		let path = path.clone();
		move |_| on_open_file.call(path.clone())
	};
	let caret = if is_expanded { "▾" } else { "▸" };
	// 仅在搜索模式下展示上级路径（按需构造，避免逐行多余分配）。
	let parent = if is_searching {
		parent_of(&path).to_string()
	} else {
		String::new()
	};
	rsx! {
        if is_directory {
            button {
                class: "file-row directory-row",
                r#type: "button",
                style: "--depth: {depth};",
                onclick: on_row_click,
                oncontextmenu: on_right_click,
                if !is_searching {
                    span { class: "tree-caret", "{caret}" }
                }
                span { class: "file-icon folder-icon", "▰" }
                span { class: "file-name", "{name}" }
                if is_searching {
                    span { class: "search-dir", "{parent}" }
                }
            }
        } else {
            div {
                class: if is_selected { "file-row selected" } else { "file-row" },
                id: if is_searching { String::new() } else { format!("explorer-row-{index}") },
                style: "--depth: {depth};",
                onclick: on_row_click,
                ondoubleclick: on_double_click,
                oncontextmenu: on_right_click,
                if !is_searching {
                    span { class: "tree-caret" }
                }
                span { class: "file-icon", "·" }
                span { class: "file-name", "{name}" }
                if is_searching {
                    span { class: "search-dir", "{parent}" }
                }
            }
        }
    }
}
#[component]
pub fn Files(
	/// 工作区文件列表信号：组件内用 memo 派生可见行，滚动重渲染零重算。
	files: Signal<Vec<WorkspaceFile>>,
	work_directory: Option<String>,
	width: f64,
	selected_path: Signal<Option<String>>,
	clipboard: Signal<Option<ExplorerClipboard>>,
	on_open_file: EventHandler<String>,
	on_command: EventHandler<(ExplorerCommand, String)>,
) -> Element {
	// 选中项需同时写回父级信号：菜单栏的「签名 apk」会读取该高亮项。
	let mut selected_path = selected_path;
	let mut expanded = use_signal(Vec::<String>::new);
	// 国策资源目录自动展开（解包 APK 后资源位于 assets/... 深层目录）：
	// 资源根集合变化时（打开新工作区 / 解压 APK 后），合并展开其全部上级目录。
	let mut expanded_roots_signature = use_signal(String::new);
	let roots_signature = use_memo(move || {
		let mut roots: Vec<String> = files
			.read()
			.iter()
			.filter_map(|entry| missions_root_of(&entry.relative_path))
			.collect();
		roots.sort();
		roots.dedup();
		roots.join("\n")
	});
	if *roots_signature.read() != *expanded_roots_signature.read() {
		let signature = roots_signature.read().clone();
		expanded_roots_signature.set(signature.clone());
		if !signature.is_empty() {
			let mut current = expanded.write();
			for root in signature.split('\n') {
				for path in root_ancestors(root) {
					if !current.iter().any(|item| item == &path) {
						current.push(path);
					}
				}
			}
		}
	}
	let mut active = use_signal(|| None::<String>);
	let mut context_menu = use_signal(|| None::<(f64, f64, String)>);
	// 搜索关键词：非空时平铺展示整个工作区的匹配项（名称或相对路径），便于快速定位。
	let mut search_query = use_signal(String::new);
	// 文件列表滚动位置：虚拟滚动窗口据此渲染可视区域行。
	let mut list_scroll_top = use_signal(|| 0.0_f64);

	// 行激活（单击）：记录本地点选；目录再按当前状态展开/收起（搜索中则退出搜索并展开定位）。
	// use_callback 保持跨渲染稳定身份，行组件 props 未变化时可被记忆化跳过。
	let on_activate = use_callback(move |(path, is_directory): (String, bool)| {
		active.set(Some(path.clone()));
		selected_path.set(Some(path.clone()));
		if !is_directory {
			return;
		}
		if !search_query.peek().trim().is_empty() {
			// 搜索结果中点击目录：退出搜索并展开该目录（含所有上级）以便在树中定位。
			search_query.set(String::new());
			let mut current = expanded.write();
			let mut cursor = Some(path);
			while let Some(directory) = cursor {
				if !current.iter().any(|item| item == &directory) {
					current.push(directory.clone());
				}
				cursor = directory.rsplit_once('/').map(|(parent, _)| parent.to_string());
			}
		} else {
			let mut current = expanded.write();
			if current.iter().any(|item| item == &path) {
				current.retain(|item| item != &path);
			} else {
				current.push(path);
			}
		}
	});

	// 行右键：记录本地点选并打开上下文菜单。
	let on_context = use_callback(move |(path, client_x, client_y): (String, f64, f64)| {
		active.set(Some(path.clone()));
		selected_path.set(Some(path.clone()));
		context_menu.set(Some((client_x, client_y, path)));
	});

	// 外部选中某文件时，自动展开其所有上级目录并高亮该行。
	use_effect(move || {
		let Some(path) = selected_path.read().clone() else {
			return;
		};
		let mut current = expanded.write();
		let mut parent = path.rsplit_once('/').map(|(parent, _)| parent);
		while let Some(directory) = parent {
			if !current.iter().any(|item| item == directory) {
				current.push(directory.to_string());
			}
			parent = directory.rsplit_once('/').map(|(parent, _)| parent);
		}
	});

	// 选中变化时把该行滚动到可视区域；虚拟滚动下直接按可见行号换算容器滚动位置。
	let mut last_scroll_target = use_signal(|| None::<(String, usize)>);
	use_effect(move || {
		let Some(path) = selected_path.read().clone() else {
			return;
		};
		// 本地点选（单击/右键已在 active 记录同一路径）只更新高亮，不做滚动定位，
		// 避免单击就把行滚到固定位置而干扰双击、右键菜单等操作。
		if active.read().as_deref() == Some(path.as_str()) {
			return;
		}
		// 外部联动（打开事件脚本 / 画布联动等）：清除本地点选记忆，
		// 让高亮与滚动定位都跟随联动目标。
		if active.read().is_some() {
			active.set(None);
		}
		// 搜索模式下列表为平铺结果，按树行号定位不适用，跳过。
		if !search_query.read().trim().is_empty() {
			return;
		}
		let expanded_len = expanded.read().len();
		let target = (path.clone(), expanded_len);
		if *last_scroll_target.read() == Some(target.clone()) {
			return;
		}
		last_scroll_target.set(Some(target));
		let entries = files.read();
		let expanded_snapshot = expanded.read().clone();
		let expanded_set: HashSet<&str> = expanded_snapshot
			.iter()
			.map(|item| item.as_str())
			.collect();
		let mut visible_position = None;
		let mut position = 0_usize;
		for entry in entries.iter() {
			if !row_ancestors_expanded(&entry.relative_path, &expanded_set) {
				continue;
			}
			if !entry.is_directory && entry.relative_path == path {
				visible_position = Some(position);
				break;
			}
			position += 1;
		}
		let Some(position) = visible_position else {
			return;
		};
		let top = (position as f64 * FILE_ROW_HEIGHT - FILE_ROW_HEIGHT * 3.0).max(0.0);
		spawn(async move {
			let _ = dioxus::document::eval(&format!(
				"document.getElementById('explorer-list')?.scrollTo({{ top: {top} }});"
			))
			.await;
		});
	});

	// 高亮 = 外部联动选中优先（如打开事件脚本时定位并高亮目标行），否则本地点选。
	// 外部联动会同时滚动到目标位置，若本地点选优先会出现“定位到了却没有高亮”。
	let external_selected = selected_path.read().clone();
	let active_path = active.read().clone();
	let highlight_path = external_selected.or(active_path);

	// 可见行序列（展开态 / 搜索词 / 文件列表的纯函数）：用 memo 缓存，
	// 滚动等无关重渲染直接复用结果，不再每帧重扫全表（MT 管理器同款“按需取行”）。
	let is_searching = !search_query.read().trim().is_empty();
	let visible_rows = use_memo(move || -> Vec<usize> {
		let entries = files.read();
		let query = search_query.read().trim().to_lowercase();
		if !query.is_empty() {
			return entries
				.iter()
				.enumerate()
				.filter(|(_, entry)| {
					entry.name.to_lowercase().contains(&query)
						|| entry.relative_path.to_lowercase().contains(&query)
				})
				.map(|(index, _)| index)
				.collect();
		}
		let expanded = expanded.read();
		let expanded_set: HashSet<&str> = expanded.iter().map(|item| item.as_str()).collect();
		entries
			.iter()
			.enumerate()
			.filter(|(_, entry)| row_ancestors_expanded(&entry.relative_path, &expanded_set))
			.map(|(index, _)| index)
			.collect()
	});
	let visible_indexes = visible_rows.read();
	let total_rows = visible_indexes.len();
	let search_total = if is_searching { total_rows } else { 0 };
	// 展开目录集合：整帧只构建一次（引用借用，不克隆字符串），
	// 每行判断 is_expanded 为 O(1)（此前逐行线性扫描全部展开目录）。
	let expanded_guard = expanded.read();
	let expanded_set: HashSet<&str> = expanded_guard.iter().map(|item| item.as_str()).collect();

	// 虚拟滚动窗口：仅渲染可视区域及上下缓冲行，滚动时按行回收复用。
	let scroll_top = *list_scroll_top.read();
	let max_start = total_rows.saturating_sub(1);
	let start_offset = ((scroll_top / FILE_ROW_HEIGHT).floor().max(0.0) as usize).min(max_start);
	let window_start = start_offset.saturating_sub(VIRTUAL_BUFFER_ROWS);
	let window_end =
		(window_start + VIRTUAL_VIEWPORT_ROWS + VIRTUAL_BUFFER_ROWS * 2).min(total_rows);
	let window_rows: Vec<usize> = visible_indexes[window_start..window_end].to_vec();
	let top_spacer = window_start as f64 * FILE_ROW_HEIGHT;
	let bottom_spacer = (total_rows - window_end) as f64 * FILE_ROW_HEIGHT;
	// 内容变短（折叠目录 / 退出搜索）后滚动信号可能超出内容高度，回夹防窗口空白。
	let max_scroll = total_rows as f64 * FILE_ROW_HEIGHT;
	if scroll_top > max_scroll {
		list_scroll_top.set(max_scroll);
	}

	let menu_position = context_menu.read().clone();
	let clipboard_ready = clipboard.read().is_some();
	// 工作区文件列表整帧借用一次（不克隆）：面板计数、行数据与菜单数据共用。
	let entries = files.read();
	let menu_data = menu_position.map(|(client_x, client_y, path)| {
		let paste_target_dir = match entries.iter().find(|entry| entry.relative_path == path) {
			Some(entry) if entry.is_directory => path.clone(),
			Some(_) => parent_of(&path).to_string(),
			None => String::new(),
		};
		let is_apk_file = entries.iter().any(|entry| {
			!entry.is_directory
				&& entry.relative_path == path
				&& entry.name.to_ascii_lowercase().ends_with(".apk")
		});
		(client_x, client_y, path, paste_target_dir, is_apk_file)
	});

	rsx! {
        aside { class: "explorer-pane", style: "width: {width}px;",
            div { class: "pane-heading",
                span { "资源管理器" }
                span { class: "pane-count", "{entries.len()}" }
            }
            if work_directory.is_some() {
                div { class: "explorer-search",
                    input {
                        r#type: "text",
                        value: "{search_query}",
                        placeholder: "搜索文件...",
                        spellcheck: "false",
                        autocomplete: "off",
                        "data-native-undo": "true",
                        aria_label: "搜索文件",
                        title: "按名称或路径搜索，Esc 清除",
                        oninput: move |evt: FormEvent| search_query.set(evt.value()),
                        onkeydown: move |evt: Event<KeyboardData>| {
                            if evt.data().key().to_string() == "Escape" {
                                search_query.set(String::new());
                            }
                        },
                    }
                    if !search_query.read().is_empty() {
                        button {
                            class: "search-clear",
                            r#type: "button",
                            aria_label: "清除搜索",
                            onclick: move |_| search_query.set(String::new()),
                            "✕"
                        }
                    }
                }
                if is_searching {
                    div { class: "search-status",
                        if search_total == 0 {
                            "无匹配结果"
                        } else {
                            "匹配 {search_total} 个"
                        }
                    }
                }
                div {
                    class: "file-list",
                    id: "explorer-list",
                    onscroll: move |evt: Event<ScrollData>| {
                        let top = evt.scroll_top();
                        let mut list_scroll_top = list_scroll_top;
                        // 按 2 行节流：窗口带 32 行上下缓冲，无需跟随每个像素重渲染。
                        if (top - list_scroll_top.cloned()).abs() >= FILE_ROW_HEIGHT * 2.0 {
                            list_scroll_top.set(top);
                        }
                    },
                    if top_spacer > 0.0 {
                        div {
                            style: "height: {top_spacer}px;",
                            aria_hidden: "true",
                        }
                    }
                    for entry_index in window_rows {
                        {
                            let entry = entries[entry_index].clone();
                            let depth = if is_searching {
                                0
                            } else {
                                entry.relative_path.matches('/').count()
                            };
                            let is_expanded = expanded_set.contains(entry.relative_path.as_str());
                            let is_selected = Some(&entry.relative_path) == highlight_path.as_ref();
                            rsx! {
                                FileRow {
                                    key: "{entry_index}",
                                    entry,
                                    index: entry_index,
                                    depth,
                                    is_searching,
                                    is_expanded,
                                    is_selected,
                                    on_activate,
                                    on_context,
                                    on_open_file,
                                }
                            }
                        }
                    }
                    if bottom_spacer > 0.0 {
                        div {
                            style: "height: {bottom_spacer}px;",
                            aria_hidden: "true",
                        }
                    }
                }
            } else {
                div { class: "explorer-empty", "打开工作区以浏览文件" }
            }

            if let Some((client_x, client_y, path, paste_target_dir, is_apk_file)) = menu_data {
                ExplorerContextMenu {
                    x: client_x,
                    y: client_y,
                    path,
                    paste_target_dir,
                    clipboard_ready,
                    is_apk_file,
                    on_command,
                    on_close: move |_| context_menu.set(None),
                }
            }
        }
    }
}
