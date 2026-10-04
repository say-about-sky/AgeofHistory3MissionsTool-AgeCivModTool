use dioxus::prelude::*;
use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFile {
	pub name: String,
	pub relative_path: String,
	pub is_directory: bool,
}

/// 资源管理器操作命令。命令元组的第二个元素语义：
/// - Cut/Copy/Delete/Reveal：操作对象的相对路径；
/// - Paste：粘贴目标目录（空串表示工作区根）。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ExplorerCommand {
	Cut,
	Copy,
	Paste,
	Delete,
	Reveal,
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

/// 搜索结果单次渲染的最大行数，避免上千行同时渲染造成卡顿。
const SEARCH_RESULT_LIMIT: usize = 300;

/// 右键菜单（仿 VSCode）。
#[component]
fn ExplorerContextMenu(
	x: f64,
	y: f64,
	path: String,
	paste_target_dir: String,
	clipboard_ready: bool,
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

#[component]
pub fn Files(
	entries: Vec<WorkspaceFile>,
	work_directory: Option<String>,
	width: f64,
	selected_path: Signal<Option<String>>,
	clipboard: Signal<Option<ExplorerClipboard>>,
	on_open_file: EventHandler<String>,
	on_command: EventHandler<(ExplorerCommand, String)>,
) -> Element {
	let mut expanded = use_signal(|| vec!["missions".to_string()]);
	let mut active = use_signal(|| None::<String>);
	let mut context_menu = use_signal(|| None::<(f64, f64, String)>);
	// 搜索关键词：非空时平铺展示整个工作区的匹配项（名称或相对路径），便于快速定位。
	let mut search_query = use_signal(String::new);

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

	// 选中变化时，待目录展开完成后把该行滚动到可见区域。
	let entries_for_scroll = entries.clone();
	let mut last_scroll_target = use_signal(|| None::<(String, usize)>);
	use_effect(move || {
		let Some(path) = selected_path.read().clone() else {
			return;
		};
		let expanded_len = expanded.read().len();
		let target = (path.clone(), expanded_len);
		if *last_scroll_target.read() == Some(target.clone()) {
			return;
		}
		last_scroll_target.set(Some(target));
		let Some((row_index, _)) = entries_for_scroll
			.iter()
			.enumerate()
			.find(|(_, entry)| !entry.is_directory && entry.relative_path == path)
		else {
			return;
		};
		spawn(async move {
			let _ = dioxus::document::eval(&format!(
				"document.getElementById('explorer-row-{row_index}')?.scrollIntoView({{ block: 'nearest', inline: 'nearest' }});"
			))
			.await;
		});
	});

	// 高亮 = 本地点选（active）优先，否则外部联动选中。
	let external_selected = selected_path.read().clone();
	let active_path = active.read().clone();
	let highlight_path = active_path.or(external_selected);

	let query = search_query.read().trim().to_lowercase();
	let is_searching = !query.is_empty();
	// 搜索模式：对整个工作区做大小写不敏感的匹配，平铺展示（限制渲染行数）。
	let search_indexes: Vec<usize> = if is_searching {
		entries
			.iter()
			.enumerate()
			.filter(|(_, entry)| {
				entry.name.to_lowercase().contains(&query)
					|| entry.relative_path.to_lowercase().contains(&query)
			})
			.map(|(index, _)| index)
			.collect()
	} else {
		Vec::new()
	};
	let search_total = search_indexes.len();
	// 单目录渲染上限：展开超大目录（如 missionsEvents 的 4807 个文件）时仅渲染前 N 项，避免卡顿。
	const DIRECTORY_RENDER_LIMIT: usize = 300;
	let (visible_entries, truncated_note): (
		Vec<(usize, WorkspaceFile)>,
		Option<(String, usize, usize)>,
	) = if is_searching {
		(
			search_indexes
				.iter()
				.take(SEARCH_RESULT_LIMIT)
				.map(|index| (*index, entries[*index].clone()))
				.collect(),
			None,
		)
	} else {
		let mut totals: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
		for entry in &entries {
			let parent = entry.relative_path.rsplit_once('/').map_or("", |(parent, _)| parent);
			*totals.entry(parent).or_insert(0) += 1;
		}
		let expanded_snapshot = expanded.read().clone();
		let mut shown_per_parent: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
		let mut visible = Vec::new();
		let mut truncated = None;
		for (index, entry) in entries.iter().enumerate() {
			let mut cursor = entry.relative_path.rsplit_once('/').map(|(parent, _)| parent);
			let mut row_visible = true;
			while let Some(path) = cursor {
				if !expanded_snapshot.iter().any(|item| item == path) {
					row_visible = false;
					break;
				}
				cursor = path.rsplit_once('/').map(|(parent, _)| parent);
			}
			if !row_visible {
				continue;
			}
			let parent = entry.relative_path.rsplit_once('/').map_or("", |(parent, _)| parent);
			let shown = shown_per_parent.entry(parent).or_insert(0);
			if *shown >= DIRECTORY_RENDER_LIMIT {
				if truncated.is_none() {
					let total = totals.get(parent).copied().unwrap_or(0);
					truncated = Some((parent.to_string(), DIRECTORY_RENDER_LIMIT, total));
				}
				continue;
			}
			*shown += 1;
			visible.push((index, entry.clone()));
		}
		(visible, truncated)
	};

	let menu_position = context_menu.read().clone();
	let clipboard_ready = clipboard.read().is_some();
	let menu_data = menu_position.map(|(client_x, client_y, path)| {
		let paste_target_dir = match entries.iter().find(|entry| entry.relative_path == path) {
			Some(entry) if entry.is_directory => path.clone(),
			Some(_) => parent_of(&path).to_string(),
			None => String::new(),
		};
		(client_x, client_y, path, paste_target_dir)
	});

	rsx! {
        aside { class: "explorer-pane", style: "width: {width}px;",
            div { class: "pane-heading",
                span { "资源管理器" }
                span { class: "pane-count", "{entries.len()}" }
            }
            if let Some(path) = work_directory {
                div { class: "explorer-search",
                    input {
                        r#type: "text",
                        value: "{search_query}",
                        placeholder: "搜索文件...",
                        spellcheck: "false",
                        autocomplete: "off",
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
                        } else if search_total > SEARCH_RESULT_LIMIT {
                            "匹配 {search_total} 个，仅显示前 {SEARCH_RESULT_LIMIT} 个"
                        } else {
                            "匹配 {search_total} 个"
                        }
                    }
                }
                div { class: "workspace-root", title: "{path}", "工作区" }
                if let Some((directory, shown, total)) = truncated_note.clone() {
                    div { class: "search-status",
                        "「{directory}」共 {total} 项，仅显示前 {shown} 项，可用搜索快速定位"
                    }
                }
                div { class: "file-list",
                    for (row_index , entry) in visible_entries {
                        {
                            let depth = if is_searching {
                                0
                            } else {
                                entry.relative_path.matches('/').count()
                            };
                            let parent = parent_of(&entry.relative_path).to_string();
                            let path = entry.relative_path.clone();
                            let toggle_path = path.clone();
                            let select_path = path.clone();
                            let open_path = path.clone();
                            let menu_path = path.clone();
                            let is_expanded = expanded.read().iter().any(|item| item == &path);
                            let is_selected = Some(&path) == highlight_path.as_ref();
                            let caret = if is_expanded { "▾" } else { "▸" };
                            rsx! {
                                if entry.is_directory {
                                    button {
                                        class: "file-row directory-row",
                                        r#type: "button",
                                        style: "--depth: {depth};",
                                        onclick: move |_| {
                                            active.set(Some(toggle_path.clone()));
                                            if is_searching {
                                                // 搜索结果中点击目录：退出搜索并展开该目录（含所有上级）以便在树中定位。
                                                search_query.set(String::new());
                                                let mut current = expanded.write();
                                                let mut cursor = Some(toggle_path.clone());
                                                while let Some(directory) = cursor {
                                                    if !current.iter().any(|item| item == &directory) {
                                                        current.push(directory.clone());
                                                    }
                                                    cursor = directory
                                                        .rsplit_once('/')
                                                        .map(|(parent, _)| parent.to_string());
                                                }
                                            } else {
                                                let mut current = expanded.write();
                                                if is_expanded {
                                                    current.retain(|item| item != &toggle_path);
                                                } else {
                                                    current.push(toggle_path.clone());
                                                }
                                            }
                                        },
                                        oncontextmenu: move |evt: Event<MouseData>| {
                                            evt.prevent_default();
                                            evt.stop_propagation();
                                            let client = evt.client_coordinates();
                                            active.set(Some(menu_path.clone()));
                                            context_menu.set(Some((client.x, client.y, menu_path.clone())));
                                        },
                                        if !is_searching {
                                            span { class: "tree-caret", "{caret}" }
                                        }
                                        span { class: "file-icon folder-icon", "▰" }
                                        span { class: "file-name", "{entry.name}" }
                                        if is_searching {
                                            span { class: "search-dir", "{parent}" }
                                        }
                                    }
                                } else {
                                    div {
                                        class: if is_selected { "file-row selected" } else { "file-row" },
                                        id: if is_searching { String::new() } else { format!("explorer-row-{row_index}") },
                                        style: "--depth: {depth};",
                                        onclick: move |_| active.set(Some(select_path.clone())),
                                        ondoubleclick: move |_| on_open_file.call(open_path.clone()),
                                        oncontextmenu: move |evt: Event<MouseData>| {
                                            evt.prevent_default();
                                            evt.stop_propagation();
                                            let client = evt.client_coordinates();
                                            active.set(Some(menu_path.clone()));
                                            context_menu.set(Some((client.x, client.y, menu_path.clone())));
                                        },
                                        if !is_searching {
                                            span { class: "tree-caret" }
                                        }
                                        span { class: "file-icon", "·" }
                                        span { class: "file-name", "{entry.name}" }
                                        if is_searching {
                                            span { class: "search-dir", "{parent}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                div { class: "explorer-empty", "打开工作区以浏览文件" }
            }

            if let Some((client_x, client_y, path, paste_target_dir)) = menu_data {
                ExplorerContextMenu {
                    x: client_x,
                    y: client_y,
                    path,
                    paste_target_dir,
                    clipboard_ready,
                    on_command,
                    on_close: move |_| context_menu.set(None),
                }
            }
        }
    }
}
