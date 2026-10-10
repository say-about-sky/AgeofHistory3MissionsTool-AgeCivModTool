use dioxus::prelude::*;
use serde::Deserialize;
use std::collections::HashSet;

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

/// 行的小写索引项（与 `WorkspaceFile` 平行、同序）：文件列表变化时构建一次，
/// 输入时的筛选/定位/补全只做低成本的 contains/starts_with，避免逐键重建小写串。
#[derive(Clone, PartialEq)]
struct FileLowerIndex {
	name: String,
	path: String,
	is_directory: bool,
}

/// 构建小写索引（仅全表变更时调用）。
fn lowercase_index(entries: &[WorkspaceFile]) -> Vec<FileLowerIndex> {
	entries
		.iter()
		.map(|entry| FileLowerIndex {
			name: entry.name.to_lowercase(),
			path: entry.relative_path.to_lowercase(),
			is_directory: entry.is_directory,
		})
		.collect()
}

/// 自动补全候选（轻量下拉；`insert` 为 Tab / 点击后写入输入框的文本）。
#[derive(Clone, PartialEq)]
struct SearchSuggestion {
	insert: String,
	label: String,
	hint: String,
}

/// 自动补全下拉最多展示的候选数。
const SEARCH_SUGGEST_LIMIT: usize = 8;

/// 生成自动补全候选：
/// - 路径模式（`/` 或 `\` 开头）：**只列「下一级」**——把输入拆成「目录部分 +
///   待补全段」，列出目录部分的直接子级中前缀匹配该段的项（目录带尾分隔符；
///   输入以分隔符结尾时直接列子级）。避免输入长目录名（尤其中文/特殊符号）时
///   被其全部后代刷屏；分隔符风格跟随输入；已完整输入的项不再自我建议。
/// - 关键词模式：按「名称前缀 → 名称包含 → 路径包含」三档取前 N 个，
///   补全文本为名称（`hint` 显示上级目录）。
/// 仅在预建小写索引上比较；为大命中量设计为分档限额（不做全排序）。
fn search_suggestions(
	entries: &[WorkspaceFile],
	index: &[FileLowerIndex],
	raw: &str,
) -> Vec<SearchSuggestion> {
	let trimmed = raw.trim();
	if trimmed.is_empty() {
		return Vec::new();
	}
	let mut out = Vec::new();
	if let Some(query) = parse_path_query(raw) {
		let backslash = trimmed.starts_with('\\');
		let separator = if backslash { '\\' } else { '/' };
		let lowered = query.to_lowercase();
		// 目录部分 / 待补全段：带尾分隔符 = 列出该目录的直接子级；否则补全最后一段。
		let ends_with_separator = trimmed.ends_with('/') || trimmed.ends_with('\\');
		let (dir_lower, segment) = if ends_with_separator {
			(lowered.as_str(), "")
		} else {
			match lowered.rfind('/') {
				Some(position) => (&lowered[..position], &lowered[position + 1..]),
				None => ("", lowered.as_str()),
			}
		};
		// 目录部分必须是已存在的目录（空 = 工作区根）；否则无候选。
		let parent_display = if dir_lower.is_empty() {
			None
		} else {
			match index
				.iter()
				.position(|item| item.is_directory && item.path == dir_lower)
			{
				Some(position) => Some(entries[position].relative_path.clone()),
				None => return out,
			}
		};
		let hint = match &parent_display {
			Some(parent) => {
				let parent = if backslash {
					parent.replace('/', "\\")
				} else {
					parent.clone()
				};
				format!("{separator}{parent}")
			}
			None => separator.to_string(),
		};
		for (position, item) in index.iter().enumerate() {
			// 仅直接子级：根级不含分隔符；子级去掉目录前缀后不再含分隔符。
			let child_lower = if dir_lower.is_empty() {
				if item.path.contains('/') {
					continue;
				}
				item.path.as_str()
			} else {
				match item
					.path
					.strip_prefix(dir_lower)
					.and_then(|rest| rest.strip_prefix('/'))
				{
					Some(rest) if !rest.contains('/') => rest,
					_ => continue,
				}
			};
			if !child_lower.starts_with(segment) {
				continue;
			}
			let entry = &entries[position];
			let mut path_text = entry.relative_path.clone();
			if backslash {
				path_text = path_text.replace('/', "\\");
			}
			let mut insert = format!("{separator}{path_text}");
			if entry.is_directory {
				insert.push(separator);
			}
			// 已是完整输入（文件全名 / 目录加尾分隔符）不再自我建议。
			if insert == trimmed {
				continue;
			}
			let mut label = entry.name.clone();
			if entry.is_directory {
				label.push(separator);
			}
			out.push(SearchSuggestion {
				insert,
				label,
				hint: hint.clone(),
			});
			if out.len() >= SEARCH_SUGGEST_LIMIT {
				break;
			}
		}
	} else {
		let lowered = trimmed.to_lowercase();
		// 三档桶各留前 N（同档内按原序）；前缀档已够 N 时可直接停止全表扫描。
		let mut buckets: [Vec<usize>; 3] = [Vec::new(), Vec::new(), Vec::new()];
		for (position, item) in index.iter().enumerate() {
			if item.name == lowered {
				continue;
			}
			let rank = if item.name.starts_with(&lowered) {
				0
			} else if item.name.contains(&lowered) {
				1
			} else if item.path.contains(&lowered) {
				2
			} else {
				continue;
			};
			if buckets[rank].len() < SEARCH_SUGGEST_LIMIT {
				buckets[rank].push(position);
			}
			if buckets[0].len() >= SEARCH_SUGGEST_LIMIT
				|| buckets[0].len() + buckets[1].len() >= SEARCH_SUGGEST_LIMIT
			{
				break;
			}
		}
		for bucket in &buckets {
			for position in bucket {
				let entry = &entries[*position];
				out.push(SearchSuggestion {
					insert: entry.name.clone(),
					label: entry.name.clone(),
					hint: parent_of(&entry.relative_path).to_string(),
				});
				if out.len() >= SEARCH_SUGGEST_LIMIT {
					return out;
				}
			}
		}
	}
	out
}

/// 搜索框路径定位解析：输入以 `/` 或 `\` 开头时视为工作区相对路径
///（反斜杠归一为 `/`、去首尾分隔符）；普通文本返回 None（走关键词过滤）。
fn parse_path_query(raw: &str) -> Option<String> {
	let trimmed = raw.trim();
	if !(trimmed.starts_with('/') || trimmed.starts_with('\\')) {
		return None;
	}
	let normalized = trimmed.replace('\\', "/");
	Some(
		normalized
			.trim_start_matches('/')
			.trim_end_matches('/')
			.to_string(),
	)
}

/// 路径定位匹配：优先完全相等（大小写不敏感），否则取首个前缀匹配项
///（如 `modA/ass` → `modA/assets`，支持边输入边收敛；同前缀多项按列表前序取第一个）。
/// `index` 为同序小写索引（避免每次按键重建小写串）。
fn find_locate_target(
	entries: &[WorkspaceFile],
	index: &[FileLowerIndex],
	query: &str,
) -> Option<String> {
	if query.is_empty() {
		return None;
	}
	let lowered = query.to_lowercase();
	let mut prefix = None;
	for (position, item) in index.iter().enumerate() {
		if item.path == lowered {
			return Some(entries[position].relative_path.clone());
		}
		if prefix.is_none() && item.path.starts_with(&lowered) {
			prefix = Some(entries[position].relative_path.clone());
		}
	}
	prefix
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
	let mut active = use_signal(|| None::<String>);
	let mut context_menu = use_signal(|| None::<(f64, f64, String)>);
	// 搜索关键词：非空时平铺展示整个工作区的匹配项（名称或相对路径），便于快速定位。
	let mut search_query = use_signal(String::new);	// 搜索/定位/补全共用的小写索引：文件列表变化时构建一次（避免每个按键对全表重复 to_lowercase）。
	let lower_index = use_memo(move || lowercase_index(&files.read()));
	// 自动补全下拉：候选（轻量）、当前高亮项与输入框聚焦状态。
	let suggestions = use_memo(move || {
		search_suggestions(&files.read(), &lower_index.read(), &search_query.read())
	});
	let mut suggest_index = use_signal(|| 0_usize);
	let mut search_focused = use_signal(|| false);
	// 路径定位目标（`/` 或 `\` 开头）：memo 单次扫描，状态行与定位 effect 共用。
	let locate_target = use_memo(move || -> Option<(String, bool)> {
		let query = parse_path_query(&search_query.read())?;
		let entries = files.read();
		let index = lower_index.read();
		let path = find_locate_target(&entries, &index, &query)?;
		let is_directory = entries
			.iter()
			.any(|entry| entry.is_directory && entry.relative_path == path);
		Some((path, is_directory))
	});	// 文件列表滚动位置：虚拟滚动窗口据此渲染可视区域行。
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

	// 搜索框「/ 或 \ 开头」= 相对路径定位：展开目标（目录含自身）与全部上级，
	// 选中并由下方滚动 effect 定位到可视区（目标由 `locate_target` memo 单次扫描得出）。
	use_effect(move || {
		let Some((path, is_directory)) = locate_target.read().clone() else {
			return;
		};
		{
			let mut current = expanded.write();
			if is_directory && !current.iter().any(|item| item == &path) {
				current.push(path.clone());
			}
			let mut cursor = Some(path.as_str());
			while let Some(directory) = cursor {
				if !current.iter().any(|item| item == directory) {
					current.push(directory.to_string());
				}
				cursor = directory.rsplit_once('/').map(|(parent, _)| parent);
			}
		}
		// 清除本地点选记忆，否则滚动 effect 会把它当作“单击”而跳过定位。
		if active.read().is_some() {
			active.set(None);
		}
		selected_path.set(Some(path));
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
		// 关键词搜索为平铺结果，按树行号定位不适用，跳过；路径定位（/ 开头）仍走树定位。
		let raw_query = search_query.read().clone();
		if parse_path_query(&raw_query).is_none() && !raw_query.trim().is_empty() {
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
			if entry.relative_path == path {
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
	let path_query = parse_path_query(&search_query.read());
	let is_path_mode = path_query.is_some();
	let is_searching = !search_query.read().trim().is_empty() && !is_path_mode;
	let visible_rows = use_memo(move || -> Vec<usize> {
		let raw_query = search_query.read().clone();
		let is_path_mode = parse_path_query(&raw_query).is_some();
		let query = raw_query.trim().to_lowercase();
		if !query.is_empty() && !is_path_mode {
			let index = lower_index.read();
			return index
				.iter()
				.enumerate()
				.filter(|(_, item)| item.name.contains(&query) || item.path.contains(&query))
				.map(|(position, _)| position)
				.collect();
		}
		let entries = files.read();
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

	let locate_query_display = path_query.clone().unwrap_or_default();
	// 自动补全下拉：聚焦 + 有候选时展示；高亮下标钳制在候选范围内。
	let suggest_items = suggestions.read();
	let show_suggest = *search_focused.read() && !suggest_items.is_empty();
	let active_suggest = (*suggest_index.read()).min(suggest_items.len().saturating_sub(1));
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
                        placeholder: "搜索文件；/ 开头定位路径",
                        spellcheck: "false",
                        autocomplete: "off",
                        "data-native-undo": "true",
                        aria_label: "搜索文件",
                        title: "按名称或路径搜索；以 / 或 \\ 开头=定位相对路径；Tab 补全；Esc 清除",
                        oninput: move |evt: FormEvent| {
                            search_query.set(evt.value());
                            suggest_index.set(0);
                        },
                        onfocus: move |_| search_focused.set(true),
                        onblur: move |_| search_focused.set(false),
                        onkeydown: move |evt: Event<KeyboardData>| {
                            let key = evt.data().key().to_string();
                            let count = suggestions.read().len();
                            match key.as_str() {
                                // Tab / Enter：补全当前高亮项（默认第一个候选）。
                                "Tab" | "Enter" if count > 0 => {
                                    evt.prevent_default();
                                    let index = (*suggest_index.peek()).min(count - 1);
                                    if let Some(item) = suggestions.read().get(index).cloned() {
                                        search_query.set(item.insert);
                                        suggest_index.set(0);
                                    }
                                }
                                "ArrowDown" if count > 0 => {
                                    evt.prevent_default();
                                    let next = (*suggest_index.peek() + 1).min(count - 1);
                                    suggest_index.set(next);
                                }
                                "ArrowUp" if count > 0 => {
                                    evt.prevent_default();
                                    let previous = (*suggest_index.peek()).saturating_sub(1);
                                    suggest_index.set(previous);
                                }
                                "Escape" => {
                                    search_query.set(String::new());
                                    suggest_index.set(0);
                                }
                                _ => {}
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
                    if show_suggest {
                        div { class: "explorer-suggest", role: "listbox",
                            for (item_index , item) in suggest_items.iter().enumerate() {
                                button {
                                    key: "{item_index}-{item.insert}",
                                    class: if item_index == active_suggest { "explorer-suggest-item active" } else { "explorer-suggest-item" },
                                    r#type: "button",
                                    role: "option",
                                    onmousedown: move |evt: Event<MouseData>| evt.prevent_default(),
                                    onclick: {
                                        let insert = item.insert.clone();
                                        move |_| {
                                            search_query.set(insert.clone());
                                            suggest_index.set(0);
                                        }
                                    },
                                    span { class: "explorer-suggest-label", "{item.label}" }
                                    if !item.hint.is_empty() {
                                        span { class: "explorer-suggest-hint", "{item.hint}" }
                                    }
                                }
                            }
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
                } else if is_path_mode {
                    div { class: "search-status",
                        if let Some((target, _)) = locate_target.read().clone() {
                            "定位：{target}"
                        } else if locate_query_display.is_empty() {
                            "输入相对路径后自动定位"
                        } else {
                            "未找到匹配的路径"
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

#[cfg(test)]
mod tests {
	use super::*;

	fn file(name: &str, path: &str, is_directory: bool) -> WorkspaceFile {
		WorkspaceFile {
			name: name.to_string(),
			relative_path: path.to_string(),
			is_directory,
		}
	}

	/// `/` 与 `\` 开头都识别为路径定位；归一化分隔符与首尾斜杠。
	#[test]
	fn parses_path_query_with_both_separators() {
		assert_eq!(
			parse_path_query("/assets/rainfall"),
			Some("assets/rainfall".to_string())
		);
		assert_eq!(
			parse_path_query("\\modA\\assets\\"),
			Some("modA/assets".to_string())
		);
		assert_eq!(parse_path_query("  /a/b  "), Some("a/b".to_string()));
		assert_eq!(parse_path_query("/"), Some(String::new()));
		assert_eq!(parse_path_query("abc"), None);
		assert_eq!(parse_path_query(""), None);
	}

	/// 定位匹配：完全相等（大小写不敏感）优先，否则首个前缀（支持边输边收敛）。
	#[test]
	fn finds_locate_target_by_exact_then_prefix() {
		let entries = vec![
			file("modA", "modA", true),
			file("assets", "modA/assets", true),
			file("assetsX", "modA/assetsX", true),
			file("rainfall", "modA/assets/rainfall", true),
			file(
				"rfEvent_decision.json",
				"modA/assets/rainfall/rfEvent_decision.json",
				false,
			),
		];
		let index = lowercase_index(&entries);
		assert_eq!(
			find_locate_target(&entries, &index, "MODA/assets"),
			Some("modA/assets".to_string())
		);
		assert_eq!(
			find_locate_target(&entries, &index, "modA/ass"),
			Some("modA/assets".to_string())
		);
		assert_eq!(
			find_locate_target(&entries, &index, "modA/assets/rainfall/rf"),
			Some("modA/assets/rainfall/rfEvent_decision.json".to_string())
		);
		assert_eq!(find_locate_target(&entries, &index, "modA/nope"), None);
		assert_eq!(find_locate_target(&entries, &index, ""), None);
	}

	/// 自动补全：路径模式只列直接子级（不刷后代）、分隔符风格跟随；
	/// 关键词三档排序、完整名称跳过。
	#[test]
	fn suggests_path_completions_and_keyword_names() {
		let entries = vec![
			file("modA", "modA", true),
			file("assets", "modA/assets", true),
			file("rainfall", "modA/assets/rainfall", true),
			file("banner.png", "modA/assets/banner.png", false),
		];
		let index = lowercase_index(&entries);
		// 同级前缀：只补全目录自身（label 为子名 + 尾 `/`，hint 为父级 `/`）。
		let slash = search_suggestions(&entries, &index, "/mod");
		assert_eq!(slash.len(), 1);
		assert_eq!(slash[0].insert, "/modA/");
		assert_eq!(slash[0].label, "modA/");
		assert_eq!(slash[0].hint, "/");
		// 反斜杠风格保持一致。
		let backslash = search_suggestions(&entries, &index, "\\mod");
		assert_eq!(backslash[0].insert, "\\modA\\");
		assert_eq!(backslash[0].label, "modA\\");
		assert_eq!(backslash[0].hint, "\\");
		// 目录内（带尾分隔符）：只列直接子级（assets），不下钻到 rainfall/banner。
		let deeper = search_suggestions(&entries, &index, "/moda/");
		assert_eq!(deeper.len(), 1);
		assert_eq!(deeper[0].insert, "/modA/assets/");
		assert_eq!(deeper[0].hint, "/modA");
		// 再下一层：rainfall 目录 + banner.png 文件。
		let leaf = search_suggestions(&entries, &index, "/moda/assets/");
		assert_eq!(leaf.len(), 2);
		assert_eq!(leaf[0].insert, "/modA/assets/rainfall/");
		assert_eq!(leaf[1].insert, "/modA/assets/banner.png");
		// 未知目录部分 → 无候选。
		assert!(search_suggestions(&entries, &index, "/nope/x").is_empty());
		// 关键词模式：名称前缀匹配 + 上级目录提示。
		let keyword = search_suggestions(&entries, &index, "rain");
		assert_eq!(keyword[0].insert, "rainfall");
		assert_eq!(keyword[0].hint, "modA/assets");
		// 名称已完整输入时不再建议该名称。
		assert!(search_suggestions(&entries, &index, "rainfall").is_empty());
		// 空输入无候选。
		assert!(search_suggestions(&entries, &index, "  ").is_empty());
	}

	/// 中文/长目录名：只给同级与直接子级，不被全部后代刷屏（用户反馈场景）。
	#[test]
	fn path_completion_lists_direct_children_only() {
		let entries = vec![
			file(
				"历史时代 三 - 虚 革 - 3.4.2 [昔日的暮色]_3.4.2",
				"历史时代 三 - 虚 革 - 3.4.2 [昔日的暮色]_3.4.2",
				true,
			),
			file(
				"assets",
				"历史时代 三 - 虚 革 - 3.4.2 [昔日的暮色]_3.4.2/assets",
				true,
			),
			file(
				"game",
				"历史时代 三 - 虚 革 - 3.4.2 [昔日的暮色]_3.4.2/assets/game",
				true,
			),
			file(
				"missions",
				"历史时代 三 - 虚 革 - 3.4.2 [昔日的暮色]_3.4.2/assets/game/missions",
				true,
			),
			file("modB", "modB", true),
		];
		let index = lowercase_index(&entries);
		// 不完整目录名：只在同级匹配（只补全目录自身，不刷出全部后代）。
		let partial = search_suggestions(&entries, &index, "/历史时代 三 - 虚");
		assert_eq!(partial.len(), 1);
		assert_eq!(partial[0].label, "历史时代 三 - 虚 革 - 3.4.2 [昔日的暮色]_3.4.2/");
		assert_eq!(partial[0].hint, "/");
		// 目录内（带尾分隔符）：只列直接子级（assets），不下钻 game/missions。
		let children =
			search_suggestions(&entries, &index, "/历史时代 三 - 虚 革 - 3.4.2 [昔日的暮色]_3.4.2/");
		assert_eq!(children.len(), 1);
		assert_eq!(
			children[0].insert,
			"/历史时代 三 - 虚 革 - 3.4.2 [昔日的暮色]_3.4.2/assets/"
		);
	}
}
