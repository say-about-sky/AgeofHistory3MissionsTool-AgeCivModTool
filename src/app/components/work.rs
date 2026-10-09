use dioxus::prelude::*;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

use super::{
	FocusIcon, FocusTarget, MindClipboard, MindMapCanvas, MissionRecord, Shared, WorkDirectory,
	event::{delete_event_text, rename_event_text, save_event_text, EventPanel},
	files::{ExplorerClipboard, ExplorerCommand, Files, WorkspaceFile},
	frame::Frame,
	missions_roots::{
		default_tree_path, missions_root_label, missions_root_of, missions_subfile, missions_tree_file,
	},
	undo::{
		focus_wants_native_undo, now_ms, pop_applicable, UndoDepths, UndoEntry, UndoRegistration,
		UndoScope, UndoZone, UNDO_LIMIT, UNDO_MERGE_WINDOW_MS,
	},
};
use crate::app::tauri_bridge::{
	invoke, listen_apk_progress, listen_file_op_progress, listen_workspace_changed, open_dialog,
	sleep_ms,
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MissionFileArgs {
	work_directory: String,
	missions_root: String,
	file_name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SaveMissionsFileArgs {
	work_directory: String,
	missions_root: String,
	file_name: String,
	missions: Vec<MissionRecord>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MissionIconsArgs {
	work_directory: String,
	missions_root: String,
	names: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkDirectoryArgs {
	work_directory: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateWorkDirectoryArgs {
	parent_directory: String,
	directory_name: String,
}

#[derive(Serialize)]
struct FolderDialogOptions {
	directory: bool,
	multiple: bool,
	title: String,
}

#[derive(Serialize)]
struct FileDialogFilter {
	name: String,
	extensions: Vec<String>,
}

#[derive(Serialize)]
struct FileDialogOptions {
	directory: bool,
	multiple: bool,
	title: String,
	filters: Vec<FileDialogFilter>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExtractApkArgs {
	work_directory: String,
	apk_path: String,
	/// 系统选择器提供的显示文件名（Android content:// URI 无法解析文件名；桌面端为 None）。
	apk_name: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageApkArgs {
	work_directory: String,
	/// 打包源目录（工作区内相对路径，空串表示工作区根）。
	source_directory: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LookupSourceArgs {
	work_directory: String,
	/// 作为补全数据源的 APK 位置（绝对路径或 content:// URI）。
	apk_path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LookupSourceScopedArgs {
	folder_id: String,
	/// 作为补全数据源的 APK 位置（content:// URI、绝对路径或工作区内相对路径）。
	apk_path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SignApkArgs {
	work_directory: String,
	/// 要签名的 APK（工作区内相对路径）。
	relative_path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportSigningKeyArgs {
	work_directory: String,
	/// 所选密钥文件路径（Android 端为 content:// URI）。
	key_path: String,
	/// BKS 密钥库密码（PEM 文件为 None）。
	password: Option<String>,
}

/// APK 操作命令的返回：仅取用面向用户的提示文本。
#[derive(Deserialize)]
struct ApkOperationSummary {
	message: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScopedFolder {
	id: String,
	name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScopedEntry {
	name: String,
	path: String,
	is_dir: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScopedFolderPath {
	folder_id: String,
	path: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScopedReadFilesArgs {
	folder_id: String,
	dir_path: String,
	names: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScopedFilePath {
	folder_id: String,
	path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScopedWriteTextFile {
	folder_id: String,
	path: String,
	contents: String,
	recursive: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScopedMkdir {
	folder_id: String,
	path: String,
	recursive: bool,
}

async fn is_android() -> Result<bool, String> {
	let value = JsFuture::from(invoke("is_android", js_sys::Object::new().into()))
		.await
		.map_err(|error| format!("检测运行平台失败：{error:?}"))?;
	value
		.as_bool()
		.ok_or_else(|| "运行平台检测结果无效".to_string())
}

async fn query_all_files_access() -> Result<bool, String> {
	let value = JsFuture::from(invoke("has_all_files_access", js_sys::Object::new().into()))
		.await
		.map_err(|error| format!("读取全盘文件授权状态失败：{error:?}"))?;
	value
		.as_bool()
		.ok_or_else(|| "全盘文件授权状态无效".to_string())
}

async fn open_all_files_access_settings() -> Result<bool, String> {
	let value = JsFuture::from(invoke("request_all_files_access", js_sys::Object::new().into()))
		.await
		.map_err(|error| format!("打开全盘文件授权设置失败：{error:?}"))?;
	value
		.as_bool()
		.ok_or_else(|| "全盘文件授权结果无效".to_string())
}

/// 调用 Android scoped 命令（参数为 camelCase serde 结构，返回 void）。
async fn invoke_scoped<T: Serialize>(command: &str, context: &str, args: &T) -> Result<(), String> {
	let args = serde_wasm_bindgen::to_value(args).map_err(|error| error.to_string())?;
	JsFuture::from(invoke(command, args))
		.await
		.map_err(|error| format!("{context}：{error:?}"))?;
	Ok(())
}


/// 读取 Android 工作区目录（走 App 自带快速插件：单次子项游标查询列出一层目录）。
async fn scoped_read_dir(
	folder_id: &str,
	path: Option<String>,
) -> Result<Vec<ScopedEntry>, String> {
	let args = serde_wasm_bindgen::to_value(&ScopedFolderPath {
		folder_id: folder_id.to_string(),
		path,
	})
	.map_err(|error| error.to_string())?;
	let value = JsFuture::from(invoke("list_scoped_dir", args))
		.await
		.map_err(|error| format!("读取 Android 目录失败：{error:?}"))?;
	serde_wasm_bindgen::from_value(value).map_err(|error| format!("读取目录列表失败：{error}"))
}

/// SAF（folder_id）模式的工作区递归列举（前序 DFS）。
/// 桥接层保证每层目录内「文件夹优先、其次文件」；这里逆序压栈，
/// 使展开顺序与目录内排列一致，子树行紧跟父目录行（资源管理器虚拟列表依赖前序顺序）。
async fn scan_scoped_workspace_files(
	folder_id: &str,
	root_path: &str,
) -> Result<Vec<WorkspaceFile>, String> {
	let mut entries = Vec::new();
	let mut pending = vec![root_path.to_string()];
	while let Some(directory) = pending.pop() {
		let path = if directory.is_empty() {
			None
		} else {
			Some(directory)
		};
		let mut subdirectories = Vec::new();
		for entry in scoped_read_dir(folder_id, path).await? {
			let relative_path = entry
				.path
				.strip_prefix(root_path)
				.unwrap_or(&entry.path)
				.trim_start_matches('/')
				.to_string();
			entries.push(WorkspaceFile {
				name: entry.name,
				relative_path,
				is_directory: entry.is_dir,
			});
			if entry.is_dir {
				subdirectories.push(entry.path);
			}
		}
		// 逆序入栈：弹出时按目录内排列顺序深度优先展开。
		pending.extend(subdirectories.into_iter().rev());
	}
	Ok(entries)
}

/// 批量读取 Android 工作区图标（快速路径：单次列目录 + 直读文件，返回 data URL；缺失项自动跳过）。
async fn scoped_read_files_in_dir(
	folder_id: &str,
	dir_path: &str,
	names: &[String],
) -> Result<Vec<FocusIcon>, String> {
	let args = serde_wasm_bindgen::to_value(&ScopedReadFilesArgs {
		folder_id: folder_id.to_string(),
		dir_path: dir_path.to_string(),
		names: names.to_vec(),
	})
	.map_err(|error| error.to_string())?;
	let value = JsFuture::from(invoke("read_scoped_files_in_dir", args))
		.await
		.map_err(|error| format!("读取 Android 工作区图标失败：{error:?}"))?;
	serde_wasm_bindgen::from_value(value).map_err(|error| format!("图标数据格式错误：{error}"))
}

async fn scoped_read_text_file(folder_id: &str, path: String) -> Result<String, String> {
	let args = serde_wasm_bindgen::to_value(&ScopedFilePath {
		folder_id: folder_id.to_string(),
		path,
	})
	.map_err(|error| error.to_string())?;
	let value = JsFuture::from(invoke("read_scoped_text_file", args))
		.await
		.map_err(|error| format!("读取工作区文本失败：{error:?}"))?;
	value
		.as_string()
		.ok_or_else(|| "工作区文本读取结果无效".to_string())
}

async fn scoped_write_text_file(
	folder_id: &str,
	path: String,
	contents: String,
) -> Result<(), String> {
	let args = serde_wasm_bindgen::to_value(&ScopedWriteTextFile {
		folder_id: folder_id.to_string(),
		path,
		contents,
		recursive: true,
	})
	.map_err(|error| error.to_string())?;
	JsFuture::from(invoke("write_scoped_text_file", args))
		.await
		.map_err(|error| format!("写入工作区文本失败：{error:?}"))?;
	Ok(())
}

async fn scoped_mkdir(folder_id: &str, path: String) -> Result<(), String> {
	let args = serde_wasm_bindgen::to_value(&ScopedMkdir {
		folder_id: folder_id.to_string(),
		path,
		recursive: true,
	})
	.map_err(|error| error.to_string())?;
	JsFuture::from(invoke("mkdir_scoped_dir", args))
		.await
		.map_err(|error| format!("创建工作区目录失败：{error:?}"))?;
	Ok(())
}

fn join_scoped_path(parent: &str, child: &str) -> String {
	if parent.is_empty() {
		child.to_string()
	} else {
		format!("{parent}/{child}")
	}
}

fn join_rel_path(parent: &str, name: &str) -> String {
	if parent.is_empty() {
		name.to_string()
	} else {
		format!("{parent}/{name}")
	}
}

/// 资源管理器文件操作进度（浮动进度卡的数据；`total = 0` 表示总量未知）。
#[derive(Clone)]
struct FileOpProgressView {
	stage: String,
	completed: u64,
	total: u64,
}

/// 执行资源管理器文件操作（复制/移动/删除）并维护浮动进度卡：
/// 操作计数归零时由最后一个结束的操作清除卡片（并行操作不会互相清除对方仍在显示的内容）。
async fn run_file_op<T>(
	mut active: Signal<usize>,
	mut progress: Signal<Option<FileOpProgressView>>,
	future: impl std::future::Future<Output = T>,
) -> T {
	active.with_mut(|count| *count += 1);
	let result = future.await;
	active.with_mut(|count| *count = count.saturating_sub(1));
	let remaining = *active.peek();
	if remaining == 0 {
		progress.set(None);
	}
	result
}

/// 重命名条目后计算「条目本身或其后代」迁移到的新相对路径。
fn moved_path(path: &str, from: &str, to: &str) -> Option<String> {
	if path == from {
		return Some(to.to_string());
	}
	let rest = path.strip_prefix(from).filter(|rest| rest.starts_with('/'))?;
	Some(format!("{to}{rest}"))
}

/// 重命名（同目录移动）成功后的编辑器同步：选中行、国策树标签页路径、
/// 事件面板根目录跟随改名；事件脚本改名时通知事件面板；
/// 最后重载文件列表并剪除失效标签页（标签页路径已先行更新，不会误剪）。
async fn sync_after_rename(
	directory: &WorkDirectory,
	from: &str,
	to: &str,
	is_directory: bool,
	mut workspace_files: Signal<Vec<WorkspaceFile>>,
	mut selected_file: Signal<Option<String>>,
	mut open_tabs: Signal<Vec<TreeTab>>,
	mut events_root: Signal<Option<String>>,
	mut rename_event_request: Signal<Option<(String, String)>>,
	prune_tabs: EventHandler<Vec<WorkspaceFile>>,
) {
	// 先快照再写入：读守卫需先释放（直接在 if let 判定里 read 会与后续 set 冲突）。
	let selected_snapshot = selected_file.read().clone();
	if let Some(selected) = selected_snapshot {
		if let Some(moved) = moved_path(&selected, from, to) {
			selected_file.set(Some(moved));
		}
	}
	open_tabs.with_mut(|tabs| {
		for tab in tabs.iter_mut() {
			if let Some(moved) = moved_path(&tab.json_path, from, to) {
				tab.json_path = moved.clone();
				tab.title = tree_tab_title(&moved);
			}
		}
	});
	let root_snapshot = events_root.peek().clone();
	if let Some(root) = root_snapshot {
		if let Some(moved) = moved_path(&root, from, to) {
			events_root.set(Some(moved));
		}
	}
	// 事件脚本文件本身改名：通知事件面板跟随（正在编辑且未修改时自动切换）。
	if !is_directory {
		if let (Some((_, old_name)), Some((_, new_name))) = (
			missions_subfile(from, "missionsEvents"),
			missions_subfile(to, "missionsEvents"),
		) {
			if !old_name.contains('/') && !new_name.contains('/') {
				rename_event_request.set(Some((old_name, new_name)));
			}
		}
	}
	if let Ok(files) = reload_file_list(directory).await {
		workspace_files.set(files.clone());
		prune_tabs.call(files);
	}
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceTransferArgs {
	work_directory: String,
	source: String,
	target: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkspacePathArgs {
	work_directory: String,
	target: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScopedTransfer {
	from_folder_id: String,
	from_path: String,
	to_folder_id: String,
	to_path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScopedRemoveFile {
	folder_id: String,
	path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScopedRemoveDir {
	folder_id: String,
	path: String,
	recursive: bool,
}

async fn copy_item(directory: &WorkDirectory, source: &str, target: &str) -> Result<(), String> {
	if let Some(folder_id) = &directory.folder_id {
		invoke_scoped(
			"copy_scoped_item",
			"复制文件失败",
			&ScopedTransfer {
				from_folder_id: folder_id.clone(),
				from_path: join_scoped_path(&directory.root_path, source),
				to_folder_id: folder_id.clone(),
				to_path: join_scoped_path(&directory.root_path, target),
			},
		)
		.await
	} else {
		let args = serde_wasm_bindgen::to_value(&WorkspaceTransferArgs {
			work_directory: directory.root_path.clone(),
			source: source.to_string(),
			target: target.to_string(),
		})
		.map_err(|error| error.to_string())?;
		JsFuture::from(invoke("copy_workspace_item", args))
			.await
			.map_err(|error| format!("复制文件失败：{error:?}"))?;
		Ok(())
	}
}

async fn move_item(directory: &WorkDirectory, source: &str, target: &str) -> Result<(), String> {
	if let Some(folder_id) = &directory.folder_id {
		invoke_scoped(
			"move_scoped_item",
			"移动文件失败",
			&ScopedTransfer {
				from_folder_id: folder_id.clone(),
				from_path: join_scoped_path(&directory.root_path, source),
				to_folder_id: folder_id.clone(),
				to_path: join_scoped_path(&directory.root_path, target),
			},
		)
		.await
	} else {
		let args = serde_wasm_bindgen::to_value(&WorkspaceTransferArgs {
			work_directory: directory.root_path.clone(),
			source: source.to_string(),
			target: target.to_string(),
		})
		.map_err(|error| error.to_string())?;
		JsFuture::from(invoke("move_workspace_item", args))
			.await
			.map_err(|error| format!("移动文件失败：{error:?}"))?;
		Ok(())
	}
}

async fn delete_item(
	directory: &WorkDirectory,
	target: &str,
	is_directory: bool,
) -> Result<(), String> {
	if let Some(folder_id) = &directory.folder_id {
		let path = join_scoped_path(&directory.root_path, target);
		if is_directory {
			invoke_scoped(
				"remove_scoped_dir",
				"删除工作区目录失败",
				&ScopedRemoveDir {
					folder_id: folder_id.clone(),
					path,
					recursive: true,
				},
			)
			.await
		} else {
			invoke_scoped(
				"remove_scoped_file",
				"删除工作区文件失败",
				&ScopedRemoveFile {
					folder_id: folder_id.clone(),
					path,
				},
			)
			.await
		}
	} else {
		let args = serde_wasm_bindgen::to_value(&WorkspacePathArgs {
			work_directory: directory.root_path.clone(),
			target: target.to_string(),
		})
		.map_err(|error| error.to_string())?;
		JsFuture::from(invoke("delete_workspace_item", args))
			.await
			.map_err(|error| format!("删除文件失败：{error:?}"))?;
		Ok(())
	}
}

/// 「打开文件位置」参数：work_directory 为真实路径模式的工作区根（SAF 模式为空），
/// tree_uri 为 Android SAF 授权 URI（SAF 模式取 folder_id，真实路径模式取保留的 tree_uri）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RevealItemArgs {
	work_directory: String,
	target: String,
	tree_uri: Option<String>,
	is_directory: bool,
}

/// 解析 SAF 目录下已存在子目录的文档 URI（Android 真实路径模式新建工作区后记录）。
async fn resolve_scoped_child_uri(folder_id: &str, relative: &str) -> Result<String, String> {
	let args = serde_wasm_bindgen::to_value(&ScopedChildDirArgs {
		folder_id: folder_id.to_string(),
		relative: relative.to_string(),
	})
	.map_err(|error| error.to_string())?;
	let value = JsFuture::from(invoke("resolve_scoped_child_dir_uri", args))
		.await
		.map_err(|error| format!("解析目录位置失败：{}", describe_picker_error(&error)))?;
	value
		.as_string()
		.ok_or_else(|| "目录 URI 无效".to_string())
}

/// 打开文件位置：桌面端在系统文件管理器中选中条目；Android 端弹出「用哪个应用打开」选择器。
async fn reveal_item(
	directory: &WorkDirectory,
	target: &str,
	is_directory: bool,
) -> Result<(), String> {
	let args = serde_wasm_bindgen::to_value(&RevealItemArgs {
		work_directory: directory.root_path.clone(),
		target: target.to_string(),
		tree_uri: directory
			.folder_id
			.clone()
			.or_else(|| directory.tree_uri.clone()),
		is_directory,
	})
	.map_err(|error| error.to_string())?;
	JsFuture::from(invoke("reveal_workspace_item", args))
		.await
		.map_err(|error| describe_picker_error(&error))?;
	Ok(())
}

/// 重新扫描工作区文件列表（复制/移动/删除后刷新用）。
async fn reload_file_list(directory: &WorkDirectory) -> Result<Vec<WorkspaceFile>, String> {
	if let Some(folder_id) = &directory.folder_id {
		scan_scoped_workspace_files(folder_id, &directory.root_path).await
	} else {
		let args = serde_wasm_bindgen::to_value(&WorkDirectoryArgs {
			work_directory: directory.root_path.clone(),
		})
		.map_err(|error| error.to_string())?;
		let value = JsFuture::from(invoke("list_workspace_files", args))
			.await
			.map_err(|error| format!("读取工作区文件失败：{error:?}"))?;
		serde_wasm_bindgen::from_value(value).map_err(|error| format!("工作区文件格式错误：{error}"))
	}
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScopedFolderIdArgs {
	folder_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScopedChildDirArgs {
	folder_id: String,
	relative: String,
}

#[derive(Serialize)]
struct ReadablePathArgs {
	path: String,
}

/// 解析 SAF 工作区的真实文件系统路径（Android 11+ 且已授予所有文件访问时可用）。
async fn resolve_scoped_real_path(folder_id: &str) -> Result<Option<String>, String> {
	let args = serde_wasm_bindgen::to_value(&ScopedFolderIdArgs {
		folder_id: folder_id.to_string(),
	})
	.map_err(|error| error.to_string())?;
	let value = JsFuture::from(invoke("resolve_scoped_workspace_path", args))
		.await
		.map_err(|error| format!("解析工作区真实路径失败：{error:?}"))?;
	if value.is_null() || value.is_undefined() {
		return Ok(None);
	}
	Ok(value.as_string().filter(|path| !path.is_empty()))
}

/// 校验真实路径可读；失败则退回 scoped 通道。
async fn is_workspace_path_readable(path: &str) -> Result<bool, String> {
	let args = serde_wasm_bindgen::to_value(&ReadablePathArgs {
		path: path.to_string(),
	})
	.map_err(|error| error.to_string())?;
	let value = JsFuture::from(invoke("is_workspace_path_readable", args))
		.await
		.map_err(|error| format!("校验工作区路径失败：{error:?}"))?;
	Ok(value.as_bool().unwrap_or(false))
}

/// 读取 JS 错误对象上的字符串字段（如 `code`/`message`）。
fn js_error_string_field(error: &JsValue, name: &str) -> Option<String> {
	js_sys::Reflect::get(error, &JsValue::from_str(name))
		.ok()
		.and_then(|field| field.as_string())
}

/// 判断文件选择器失败是否只是"用户取消"（Android 插件返回 `code: CANCELLED`）。
fn is_picker_cancel_error(error: &JsValue) -> bool {
	if js_error_string_field(error, "code").is_some_and(|code| {
		code.eq_ignore_ascii_case("cancelled") || code.eq_ignore_ascii_case("canceled")
	}) {
		return true;
	}
	js_error_string_field(error, "message").is_some_and(|message| {
		let message = message.trim().to_ascii_lowercase();
		message == "cancelled" || message == "canceled"
	})
}

/// 所选文件：桌面端为真实路径、Android 端为系统选择器返回的 content:// URI，
/// 并携带显示文件名（用于 .apk/.bks 扩展名判断与解压目录命名）。
struct PickedImportFile {
	location: String,
	name: String,
}

/// 从路径字符串取文件名（桌面选择器返回值；兼容 / 与 \ 分隔）。
fn file_name_of_path(path: &str) -> String {
	path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
}

/// Android：调用原生 SAF 文件选择器（与「打开工作区」同一系统组件），
/// 返回所选的 content:// URI 与显示文件名；用户取消返回 None。
async fn pick_android_file(mime_types: Vec<String>) -> Result<Option<PickedImportFile>, String> {
	let args = serde_wasm_bindgen::to_value(&PickAndroidFileArgs { mime_types })
		.map_err(|error| error.to_string())?;
	let value = JsFuture::from(invoke("pick_android_file", args))
		.await
		.map_err(|error| {
			format!(
				"无法打开 Android 文件选择器：{}",
				describe_picker_error(&error)
			)
		})?;
	if value.is_null() || value.is_undefined() {
		return Ok(None);
	}
	let picked: PickedAndroidFile = serde_wasm_bindgen::from_value(value)
		.map_err(|error| format!("读取所选文件失败：{error}"))?;
	Ok(Some(PickedImportFile {
		location: picked.uri,
		name: picked.name,
	}))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PickAndroidFileArgs {
	/// 供系统选择器过滤的 MIME 类型（空数组表示不过滤）。
	mime_types: Vec<String>,
}

#[derive(Deserialize)]
struct PickedAndroidFile {
	uri: String,
	name: String,
}

/// 把 JS 侧错误转成尽量可读的提示（优先 `message`，避免直接输出对象调试串）。
fn describe_picker_error(error: &JsValue) -> String {
	if let Some(message) =
		js_error_string_field(error, "message").filter(|message| !message.trim().is_empty())
	{
		return message;
	}
	if let Some(message) = error.as_string().filter(|message| !message.trim().is_empty()) {
		return message;
	}
	format!("{error:?}")
}

async fn pick_folder(title: &str) -> Result<Option<WorkDirectory>, String> {
	if is_android().await? {
		let value = match JsFuture::from(invoke(
			"pick_android_folder",
			js_sys::Object::new().into(),
		))
		.await
		{
			Ok(value) => value,
			// 用户在系统文件管理器里点返回/取消属于正常操作，按"未选择"处理。
			Err(error) if is_picker_cancel_error(&error) => return Ok(None),
			Err(error) => {
				return Err(format!(
					"无法打开 Android 文件管理器：{}",
					describe_picker_error(&error)
				));
			}
		};
		let folder: ScopedFolder = serde_wasm_bindgen::from_value(value)
			.map_err(|error| format!("读取所选目录失败：{error}"))?;
		let name = folder.name.unwrap_or_else(|| "工作区".to_string());
		// 已授予"所有文件访问"时，把 SAF 目录解析为真实路径：之后所有读写走 Rust 原生命令，
		// 绕过 ContentProvider 逐目录查询（事件 txt 打开/保存、图标批量载入都会快一个量级）。
		if let Some(real_path) = resolve_scoped_real_path(&folder.id).await? {
			if is_workspace_path_readable(&real_path).await.unwrap_or(false) {
				return Ok(Some(WorkDirectory {
					display_path: name,
					folder_id: None,
					root_path: real_path,
					tree_uri: Some(folder.id.clone()),
				}));
			}
		}
		return Ok(Some(WorkDirectory {
			display_path: name,
			folder_id: Some(folder.id),
			root_path: String::new(),
			tree_uri: None,
		}));
	}

	let options = serde_wasm_bindgen::to_value(&FolderDialogOptions {
		directory: true,
		multiple: false,
		title: title.to_string(),
	})
	.map_err(|error| error.to_string())?;
	let value = match JsFuture::from(open_dialog(options)).await {
		Ok(value) => value,
		Err(error) if is_picker_cancel_error(&error) => return Ok(None),
		Err(error) => return Err(format!("无法打开文件管理器：{}", describe_picker_error(&error))),
	};
	Ok(value.as_string().map(|path| WorkDirectory {
		display_path: path.clone(),
		folder_id: None,
		root_path: path,
		tree_uri: None,
	}))
}

/// 选择单个 APK 文件；用户取消时返回 None。
/// 桌面端用系统文件对话框（返回真实路径）；Android 端用原生 SAF 选择器（返回 content:// URI）。
async fn pick_apk_file(title: &str) -> Result<Option<PickedImportFile>, String> {
	if is_android().await? {
		// APK 供方 MIME 不一（部分文件管理器报 octet-stream/zip），宽松过滤 + 后续按显示名校验。
		return pick_android_file(vec![
			"application/vnd.android.package-archive".to_string(),
			"application/octet-stream".to_string(),
			"application/zip".to_string(),
		])
		.await;
	}
	let options = serde_wasm_bindgen::to_value(&FileDialogOptions {
		directory: false,
		multiple: false,
		title: title.to_string(),
		filters: vec![FileDialogFilter {
			name: "APK 安装包".to_string(),
			extensions: vec!["apk".to_string()],
		}],
	})
	.map_err(|error| error.to_string())?;
	let value = match JsFuture::from(open_dialog(options)).await {
		Ok(value) => value,
		Err(error) if is_picker_cancel_error(&error) => return Ok(None),
		Err(error) => {
			return Err(format!("无法打开文件选择器：{}", describe_picker_error(&error)))
		}
	};
	if value.is_null() || value.is_undefined() {
		return Ok(None);
	}
	let path = value
		.as_string()
		.filter(|path| !path.is_empty())
		.or_else(|| {
			// 个别平台单选也会返回数组。
			serde_wasm_bindgen::from_value::<Vec<String>>(value.clone())
				.ok()
				.and_then(|paths| paths.into_iter().find(|path| !path.is_empty()))
		});
	Ok(path.map(|path| PickedImportFile {
		name: file_name_of_path(&path),
		location: path,
	}))
}

/// 解析「从 apk 中导入 / 导出到 apk」的目标 APK：
/// 优先使用资源管理器中高亮选中的 apk（工作区相对路径，真实路径模式下才可用）；
/// 未正确选中时打开系统文件管理器让用户选择（Android 端为 content:// URI）。
/// 返回（位置, 显示文件名），用户取消时返回 None。
async fn resolve_target_apk(
	work_directory: &WorkDirectory,
	selected: Option<String>,
	title: &str,
) -> Result<Option<(String, Option<String>)>, String> {
	if let Some(selected) = selected.filter(|path| path.to_ascii_lowercase().ends_with(".apk")) {
		return Ok(Some((
			join_scoped_path(&work_directory.root_path, &selected),
			None,
		)));
	}
	match pick_apk_file(title).await? {
		Some(apk) => Ok(Some((
			apk.location,
			(!apk.name.is_empty()).then_some(apk.name),
		))),
		None => Ok(None),
	}
}

/// 选择 PEM / BKS 签名密钥文件；用户取消时返回 None。
/// 桌面端用系统文件对话框；Android 端用原生 SAF 选择器（不按类型过滤）。
async fn pick_pem_file(title: &str) -> Result<Option<PickedImportFile>, String> {
	if is_android().await? {
		return pick_android_file(Vec::new()).await;
	}
	let options = serde_wasm_bindgen::to_value(&FileDialogOptions {
		directory: false,
		multiple: false,
		title: title.to_string(),
		filters: vec![FileDialogFilter {
			name: "签名密钥".to_string(),
			extensions: vec![
				"pem".to_string(),
				"bks".to_string(),
				"key".to_string(),
				"crt".to_string(),
				"cer".to_string(),
			],
		}],
	})
	.map_err(|error| error.to_string())?;
	let value = match JsFuture::from(open_dialog(options)).await {
		Ok(value) => value,
		Err(error) if is_picker_cancel_error(&error) => return Ok(None),
		Err(error) => {
			return Err(format!("无法打开文件选择器：{}", describe_picker_error(&error)))
		}
	};
	if value.is_null() || value.is_undefined() {
		return Ok(None);
	}
	let path = value
		.as_string()
		.filter(|path| !path.is_empty())
		.or_else(|| {
			serde_wasm_bindgen::from_value::<Vec<String>>(value.clone())
				.ok()
				.and_then(|paths| paths.into_iter().find(|path| !path.is_empty()))
		});
	Ok(path.map(|path| PickedImportFile {
		name: file_name_of_path(&path),
		location: path,
	}))
}

/// 解压 APK 到工作区根目录（apk_name 为系统选择器显示名，用于解压目录命名）。
async fn extract_apk_into_workspace(
	work_directory: &str,
	apk_path: &str,
	apk_name: Option<&str>,
) -> Result<String, String> {
	let args = serde_wasm_bindgen::to_value(&ExtractApkArgs {
		work_directory: work_directory.to_string(),
		apk_path: apk_path.to_string(),
		apk_name: apk_name.map(str::to_string),
	})
	.map_err(|error| error.to_string())?;
	let value = JsFuture::from(invoke("extract_apk_to_workspace", args))
		.await
		.map_err(|error| format!("解压 APK 失败：{}", describe_picker_error(&error)))?;
	let summary: ApkOperationSummary = serde_wasm_bindgen::from_value(value)
		.map_err(|error| format!("解压结果格式错误：{error}"))?;
	Ok(summary.message)
}

/// 「从 apk 中导入」：把全局国策与各剧本版块解压到工作区（地图 / 剧本目录名按
/// APK 实际结构自动识别，适配各模组自定义命名；保留完整路径）。
async fn import_apk_sections_into_workspace(
	work_directory: &str,
	apk_path: &str,
	apk_name: Option<&str>,
) -> Result<String, String> {
	let args = serde_wasm_bindgen::to_value(&ExtractApkArgs {
		work_directory: work_directory.to_string(),
		apk_path: apk_path.to_string(),
		apk_name: apk_name.map(str::to_string),
	})
	.map_err(|error| error.to_string())?;
	let value = JsFuture::from(invoke("import_apk_sections", args))
		.await
		.map_err(|error| format!("导入 APK 失败：{}", describe_picker_error(&error)))?;
	let summary: ApkOperationSummary = serde_wasm_bindgen::from_value(value)
		.map_err(|error| format!("导入结果格式错误：{error}"))?;
	Ok(summary.message)
}

/// 「导出到 apk」：把工作区版块以更新替换方式写回目标 APK。
async fn export_apk_sections_to_apk(
	work_directory: &str,
	apk_path: &str,
	apk_name: Option<&str>,
) -> Result<String, String> {
	let args = serde_wasm_bindgen::to_value(&ExtractApkArgs {
		work_directory: work_directory.to_string(),
		apk_path: apk_path.to_string(),
		apk_name: apk_name.map(str::to_string),
	})
	.map_err(|error| error.to_string())?;
	let value = JsFuture::from(invoke("export_apk_sections", args))
		.await
		.map_err(|error| format!("导出 APK 失败：{}", describe_picker_error(&error)))?;
	let summary: ApkOperationSummary = serde_wasm_bindgen::from_value(value)
		.map_err(|error| format!("导出结果格式错误：{error}"))?;
	Ok(summary.message)
}

/// 「指定补全数据 APK」：把 APK 位置写入工作区根标记（事件编辑器补全用）。
/// 真实路径模式走 `set_lookup_source_apk`，SAF 模式走 `set_lookup_source_apk_scoped`。
async fn set_lookup_source_apk(
	directory: &WorkDirectory,
	apk_path: &str,
) -> Result<String, String> {
	let value = match &directory.folder_id {
		Some(folder_id) => {
			let args = serde_wasm_bindgen::to_value(&LookupSourceScopedArgs {
				folder_id: folder_id.clone(),
				apk_path: apk_path.to_string(),
			})
			.map_err(|error| error.to_string())?;
			JsFuture::from(invoke("set_lookup_source_apk_scoped", args)).await
		}
		None => {
			let args = serde_wasm_bindgen::to_value(&LookupSourceArgs {
				work_directory: directory.root_path.clone(),
				apk_path: apk_path.to_string(),
			})
			.map_err(|error| error.to_string())?;
			JsFuture::from(invoke("set_lookup_source_apk", args)).await
		}
	}
	.map_err(|error| format!("设置补全数据源失败：{}", describe_picker_error(&error)))?;
	value
		.as_string()
		.ok_or_else(|| "设置结果格式错误".to_string())
}

/// 打包工作区内的目录为 APK（产物位于源目录同级，不签名）。
async fn package_workspace_apk(
	work_directory: &str,
	source_directory: &str,
) -> Result<String, String> {
	let args = serde_wasm_bindgen::to_value(&PackageApkArgs {
		work_directory: work_directory.to_string(),
		source_directory: source_directory.to_string(),
	})
	.map_err(|error| error.to_string())?;
	let value = JsFuture::from(invoke("package_workspace_as_apk", args))
		.await
		.map_err(|error| format!("打包 APK 失败：{}", describe_picker_error(&error)))?;
	let summary: ApkOperationSummary = serde_wasm_bindgen::from_value(value)
		.map_err(|error| format!("打包结果格式错误：{error}"))?;
	Ok(summary.message)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UnusedScriptsArgs {
	work_directory: String,
	scope: String,
}

/// 打包 / 导出前「未使用脚本副本」确认后要继续执行的操作。
#[derive(Clone)]
enum PendingApkOperation {
	/// 导出到 APK：目标路径与显示名。
	Export { apk_path: String, apk_name: Option<String> },
	/// 打包 APK：源目录（工作区相对路径，空串 = 工作区根）。
	Package { source_directory: String },
}

/// 检查工作区（或指定子目录）内 `missionsEvents` 中未被引用的脚本副本（返回相对路径列表）。
async fn find_unused_event_scripts(
	directory: &WorkDirectory,
	scope: &str,
) -> Result<Vec<String>, String> {
	let args = serde_wasm_bindgen::to_value(&UnusedScriptsArgs {
		work_directory: directory.root_path.clone(),
		scope: scope.to_string(),
	})
	.map_err(|error| error.to_string())?;
	let value = JsFuture::from(invoke("find_unused_event_scripts", args))
		.await
		.map_err(|error| format!("检查未使用脚本失败：{error:?}"))?;
	serde_wasm_bindgen::from_value(value).map_err(|error| format!("检查结果格式错误：{error}"))
}

/// 执行打包 / 导出（「未使用脚本副本」对话框确认后调用）：进度与状态写入传入信号。
async fn perform_apk_operation(
	directory: &WorkDirectory,
	operation: PendingApkOperation,
	mut loading: Signal<bool>,
	mut load_progress: Signal<Option<LoadProgress>>,
	mut progress_counter: Signal<Option<String>>,
	mut apk_status: Signal<String>,
	mut load_error: Signal<String>,
	mut workspace_files: Signal<Vec<WorkspaceFile>>,
) {
	loading.set(true);
	match operation {
		PendingApkOperation::Package { source_directory } => {
			apk_status.set("正在打包 APK...".to_string());
			progress_counter.set(None);
			load_progress.set(Some(LoadProgress {
				stage: "正在打包 APK...".to_string(),
				completed: 0,
				total: 0,
			}));
			match package_workspace_apk(&directory.root_path, &source_directory).await {
				Ok(message) => apk_status.set(message),
				Err(error) => apk_status.set(error),
			}
		}
		PendingApkOperation::Export { apk_path, apk_name } => {
			load_error.set(String::new());
			apk_status.set("正在导出到 APK（更新替换）...".to_string());
			progress_counter.set(None);
			load_progress.set(Some(LoadProgress {
				stage: "正在导出到 APK（更新替换）...".to_string(),
				completed: 0,
				total: 0,
			}));
			match export_apk_sections_to_apk(&directory.root_path, &apk_path, apk_name.as_deref())
				.await
			{
				Ok(message) => {
					apk_status.set(message);
					if let Ok(files) = load_workspace_files(directory).await {
						workspace_files.set(files);
					}
				}
				Err(error) => apk_status.set(error),
			}
		}
	}
	loading.set(false);
	load_progress.set(None);
	progress_counter.set(None);
}

/// 「未使用脚本副本」列表行高（px；必须与行内样式的 height 匹配，虚拟滚动按行高换算窗口）。
const UNUSED_ROW_HEIGHT: f64 = 32.0;
/// 虚拟滚动：可视区域行数估算（列表容器 max-height 180px / 行高 32px）。
const UNUSED_VIEWPORT_ROWS: usize = 6;
/// 虚拟滚动：可视区上下各保留的缓冲行数（快速滚动与节流滞后时不露白）。
const UNUSED_BUFFER_ROWS: usize = 24;

#[derive(Props, Clone, PartialEq)]
struct UnusedScriptsListProps {
	/// 待处理脚本快照（工作区相对路径，已按时间排序；父层删除后重新传入）。
	scripts: Vec<String>,
	/// 勾选状态：行点击/全选切换；父层删除按钮点击时读取。
	checked: Signal<Vec<String>>,
}

/// 「未使用脚本副本」对话框中的勾选列表。
/// 安卓优化：虚拟滚动——只渲染可视窗口 ± 缓冲行（上千条目也能瞬时弹出）；
/// 勾选/滚动信号都在本组件内读取，切换勾选只重渲染本列表，不重渲染整个工作区。
#[component]
fn UnusedScriptsList(props: UnusedScriptsListProps) -> Element {
	let scripts = props.scripts;
	let checked = props.checked;
	let mut list_scroll_top = use_signal(|| 0.0_f64);
	let checked_snapshot = checked.read().clone();
	let total = scripts.len();
	let checked_count = checked_snapshot.len();
	let all_selected = total > 0 && checked_count == total;
	let scripts_for_toggle = scripts.clone();
	// 虚拟滚动窗口：仅渲染可视区域及上下缓冲行。
	let scroll_top = *list_scroll_top.read();
	let max_start = total.saturating_sub(1);
	let start_offset =
		((scroll_top / UNUSED_ROW_HEIGHT).floor().max(0.0) as usize).min(max_start);
	let window_start = start_offset.saturating_sub(UNUSED_BUFFER_ROWS);
	let window_end = (window_start + UNUSED_VIEWPORT_ROWS + UNUSED_BUFFER_ROWS * 2).min(total);
	let top_spacer = window_start as f64 * UNUSED_ROW_HEIGHT;
	let bottom_spacer = (total - window_end) as f64 * UNUSED_ROW_HEIGHT;
	// 列表变短（删除后停留）时滚动信号可能超出内容高度，回夹防窗口空白。
	let max_scroll = total as f64 * UNUSED_ROW_HEIGHT;
	if scroll_top > max_scroll {
		list_scroll_top.set(max_scroll);
	}
	rsx! {
        div { style: "display: flex; align-items: center; justify-content: space-between; gap: 8px; margin: 0 0 4px; font-size: 12px;",
            span { style: "opacity: 0.75;", "已选 {checked_count} / {total} 项" }
            button {
                class: "event-revert",
                r#type: "button",
                style: "font-size: 12px; padding: 1px 8px; line-height: 1.6;",
                onclick: move |_| {
                    let mut checked = checked;
                    if all_selected {
                        checked.set(Vec::new());
                    } else {
                        checked.set(scripts_for_toggle.clone());
                    }
                },
                if all_selected {
                    "全不选"
                } else {
                    "全选"
                }
            }
        }
        div {
            style: "max-height: 180px; overflow-y: auto; overscroll-behavior: contain; overflow-anchor: none; margin: 0 0 12px; padding: 6px 8px; border-radius: 4px; background: var(--surface-blue); font-size: 12px;",
            onscroll: move |evt: Event<ScrollData>| {
                let top = evt.scroll_top();
                let mut list_scroll_top = list_scroll_top;
                // 按 4 行节流：窗口带上下缓冲，无需跟随每个像素重渲染。
                if (top - list_scroll_top.cloned()).abs() >= UNUSED_ROW_HEIGHT * 4.0 {
                    list_scroll_top.set(top);
                }
            },
            if top_spacer > 0.0 {
                div { style: "height: {top_spacer}px;", aria_hidden: "true" }
            }
            for script_index in window_start..window_end {
                {
                    let script_name = scripts[script_index].clone();
                    let click_name = script_name.clone();
                    let is_checked = checked_snapshot.contains(&scripts[script_index]);
                    rsx! {
                        div {
                            style: "display: flex; align-items: flex-start; gap: 6px; cursor: pointer; height: {UNUSED_ROW_HEIGHT}px; overflow: hidden; user-select: none;",
                            onclick: move |_| {
                                let mut checked = checked;
                                let mut current = checked.read().clone();
                                if let Some(index) = current.iter().position(|item| item == &click_name) {
                                    current.remove(index);
                                } else {
                                    current.push(click_name.clone());
                                }
                                checked.set(current);
                            },
                            span { style: "flex: 0 0 auto; line-height: 16px;",
                                if is_checked {
                                    "☑"
                                } else {
                                    "☐"
                                }
                            }
                            span { style: "flex: 1 1 auto; word-break: break-all; line-height: 16px; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; text-overflow: ellipsis;",
                                "{script_name}"
                            }
                        }
                    }
                }
            }
            if bottom_spacer > 0.0 {
                div { style: "height: {bottom_spacer}px;", aria_hidden: "true" }
            }
        }
    }
}

/// 对选中的 APK 就地签名（v1+v2+v3）。
/// `relative_path` 可为工作区内相对路径（资源管理器选中）或桌面端文件选择器返回的绝对路径。
async fn sign_workspace_apk(work_directory: &str, relative_path: &str) -> Result<String, String> {
	let args = serde_wasm_bindgen::to_value(&SignApkArgs {
		work_directory: work_directory.to_string(),
		relative_path: relative_path.to_string(),
	})
	.map_err(|error| error.to_string())?;
	let value = JsFuture::from(invoke("sign_apk_file", args))
		.await
		.map_err(|error| format!("签名 APK 失败：{}", describe_picker_error(&error)))?;
	let summary: ApkOperationSummary = serde_wasm_bindgen::from_value(value)
		.map_err(|error| format!("签名结果格式错误：{error}"))?;
	Ok(summary.message)
}

/// 把所选 PEM/BKS 密钥导入为工作区默认签名密钥（写入工作区 signing.pem）。
async fn import_signing_key_to_workspace(
	work_directory: &str,
	key_path: &str,
	password: Option<String>,
) -> Result<String, String> {
	let args = serde_wasm_bindgen::to_value(&ImportSigningKeyArgs {
		work_directory: work_directory.to_string(),
		key_path: key_path.to_string(),
		password,
	})
	.map_err(|error| error.to_string())?;
	let value = JsFuture::from(invoke("import_signing_key", args))
		.await
		.map_err(|error| format!("添加签名密钥失败：{}", describe_picker_error(&error)))?;
	let summary: ApkOperationSummary = serde_wasm_bindgen::from_value(value)
		.map_err(|error| format!("密钥导入结果格式错误：{error}"))?;
	Ok(summary.message)
}

async fn load_workspace_files(
	work_directory: &WorkDirectory,
) -> Result<Vec<WorkspaceFile>, String> {
	if let Some(folder_id) = &work_directory.folder_id {
		return scan_scoped_workspace_files(folder_id, &work_directory.root_path).await;
	}

	let args = serde_wasm_bindgen::to_value(&WorkDirectoryArgs {
		work_directory: work_directory.root_path.clone(),
	})
	.map_err(|error| error.to_string())?;
	let files_value = JsFuture::from(invoke("list_workspace_files", args))
		.await
		.map_err(|error| format!("读取工作区文件失败：{error:?}"))?;
	serde_wasm_bindgen::from_value(files_value)
		.map_err(|error| format!("工作区文件格式错误：{error}"))
}

/// 读取单个国策树配置（missions 目录下的 .json 文件）。
async fn load_tree_records(
	work_directory: &WorkDirectory,
	json_path: &str,
) -> Result<Vec<MissionRecord>, String> {
	if let Some(folder_id) = &work_directory.folder_id {
		let path = join_scoped_path(&work_directory.root_path, json_path);
		let contents = scoped_read_text_file(folder_id, path.clone()).await?;
		let args = serde_wasm_bindgen::to_value(&ParseMissionsArgs { contents })
			.map_err(|error| error.to_string())?;
		let value = JsFuture::from(invoke("parse_missions", args))
			.await
			.map_err(|error| format!("读取国策配置失败：{error:?}"))?;
		let parsed: ParsedMissionsFile = serde_wasm_bindgen::from_value(value)
			.map_err(|error| format!("国策配置格式错误：{error}"))?;
		// 宽松语法的文件在打开时被自动纠正：把规范化内容写回原文件。
		if let Some(corrected) = parsed.corrected_contents {
			scoped_write_text_file(folder_id, path, corrected)
				.await
				.map_err(|error| format!("自动纠正语法后保存失败：{error}"))?;
		}
		Ok(parsed.missions)
	} else {
		let (missions_root, file_name) = missions_tree_file(json_path)
			.ok_or_else(|| format!("不是有效的国策树文件：{json_path}"))?;
		let args = serde_wasm_bindgen::to_value(&MissionFileArgs {
			work_directory: work_directory.root_path.clone(),
			missions_root,
			file_name,
		})
		.map_err(|error| error.to_string())?;
		let value = JsFuture::from(invoke("load_missions_file", args))
			.await
			.map_err(|error| format!("读取国策配置失败：{error:?}"))?;
		serde_wasm_bindgen::from_value(value)
			.map_err(|error| format!("国策配置格式错误：{error}"))
	}
}

/// 仅加载该树实际引用到的图标（缺失的图标静默跳过）。
async fn load_tree_icons(
	work_directory: &WorkDirectory,
	json_path: &str,
	missions: &[MissionRecord],
	mut progress: Signal<Option<LoadProgress>>,
) -> Result<Vec<FocusIcon>, String> {
	let mut names: Vec<String> = missions
		.iter()
		.map(|mission| {
			mission
				.image_name
				.strip_suffix(".png")
				.unwrap_or(&mission.image_name)
				.to_string()
		})
		.filter(|name| !name.is_empty())
		.collect();
	names.sort();
	names.dedup();

	let missions_root = missions_root_of(json_path).unwrap_or_else(|| "missions".to_string());
	if let Some(folder_id) = &work_directory.folder_id {
		let icon_directory = join_scoped_path(
			&work_directory.root_path,
			&format!("{missions_root}/missionsImages/H"),
		);
		let total_icons = names.len();
		let mut icons = Vec::with_capacity(total_icons);
		let mut completed = 0_usize;
		for chunk in names.chunks(ICON_READ_CHUNK_SIZE) {
			icons.extend(scoped_read_files_in_dir(folder_id, &icon_directory, chunk).await?);
			completed += chunk.len();
			progress.set(Some(LoadProgress {
				stage: format!("正在载入国策图标（{completed}/{total_icons}）..."),
				completed,
				total: total_icons,
			}));
		}
		Ok(icons)
	} else {
		let args = serde_wasm_bindgen::to_value(&MissionIconsArgs {
			work_directory: work_directory.root_path.clone(),
			missions_root: missions_root.clone(),
			names,
		})
		.map_err(|error| error.to_string())?;
		let value = JsFuture::from(invoke("load_mission_icons", args))
			.await
			.map_err(|error| format!("读取国策图标失败：{error:?}"))?;
		serde_wasm_bindgen::from_value(value)
			.map_err(|error| format!("国策图标格式错误：{error}"))
	}
}

/// 手动加载单个图标（资源管理器双击 .png 时调用）；文件缺失时返回 None。
async fn load_single_icon(
	work_directory: &WorkDirectory,
	name: &str,
	relative_path: &str,
) -> Result<Option<FocusIcon>, String> {
	if let Some(folder_id) = &work_directory.folder_id {
		let (dir_part, file_name) = relative_path
			.rsplit_once('/')
			.map_or(("", relative_path), |(dir, file)| (dir, file));
		let dir_path = join_scoped_path(&work_directory.root_path, dir_part);
		let stem = file_name
			.rsplit_once('.')
			.map_or(file_name, |(stem, _)| stem);
		let found = scoped_read_files_in_dir(folder_id, &dir_path, &[stem.to_string()]).await?;
		Ok(found.into_iter().next().map(|icon| FocusIcon {
			name: name.to_string(),
			data_url: icon.data_url,
		}))
	} else {
		let missions_root =
			missions_root_of(relative_path).unwrap_or_else(|| "missions".to_string());
		let args = serde_wasm_bindgen::to_value(&MissionIconsArgs {
			work_directory: work_directory.root_path.clone(),
			missions_root,
			names: vec![name.to_string()],
		})
		.map_err(|error| error.to_string())?;
		let value = JsFuture::from(invoke("load_mission_icons", args))
			.await
			.map_err(|error| format!("读取国策图标失败：{error:?}"))?;
		let icons: Vec<FocusIcon> = serde_wasm_bindgen::from_value(value)
			.map_err(|error| format!("国策图标格式错误：{error}"))?;
		Ok(icons.into_iter().next())
	}
}

/// 保存单个国策树到对应的 .json 文件。
async fn save_tree_file(
	work_directory: &WorkDirectory,
	json_path: &str,
	records: Vec<MissionRecord>,
) -> Result<(), String> {
	if let Some(folder_id) = &work_directory.folder_id {
		let args = serde_wasm_bindgen::to_value(&SerializeMissionsArgs { missions: records })
			.map_err(|error| error.to_string())?;
		let value = JsFuture::from(invoke("serialize_missions", args))
			.await
			.map_err(|error| format!("序列化国策配置失败：{error:?}"))?;
		let contents = value
			.as_string()
			.ok_or_else(|| "国策配置序列化结果无效".to_string())?;
		scoped_write_text_file(
			folder_id,
			join_scoped_path(&work_directory.root_path, json_path),
			contents,
		)
		.await
	} else {
		let (missions_root, file_name) = missions_tree_file(json_path)
			.ok_or_else(|| format!("不是有效的国策树文件：{json_path}"))?;
		let args = serde_wasm_bindgen::to_value(&SaveMissionsFileArgs {
			work_directory: work_directory.root_path.clone(),
			missions_root,
			file_name,
			missions: records,
		})
		.map_err(|error| error.to_string())?;
		JsFuture::from(invoke("save_missions_file", args))
			.await
			.map_err(|error| format!("保存国策配置失败：{error:?}"))?;
		Ok(())
	}
}

/// 标签页显示名：经典资源用文件名，嵌套资源加资源标签前缀避免同名混淆。
fn tree_tab_title(json_path: &str) -> String {
	match missions_tree_file(json_path) {
		Some((root, name)) => {
			let stem = name.trim_end_matches(".json").to_string();
			if root == "missions" {
				stem
			} else {
				format!("{}/{}", missions_root_label(&root), stem)
			}
		}
		None => json_path.to_string(),
	}
}

/// 字节数格式化为 MB（保留一位小数，进度条字节单位用）。
fn format_size(bytes: u64) -> String {
	format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

/// 打包候选目录：工作区一级子目录中直接包含 `AndroidManifest.xml` 的目录。
fn package_candidates(files: &[WorkspaceFile]) -> Vec<String> {
	let mut directories: Vec<String> = files
		.iter()
		.filter(|file| !file.is_directory && file.name == "AndroidManifest.xml")
		.filter_map(|file| {
			let parent = file.relative_path.rsplit_once('/')?.0;
			if parent.contains('/') {
				None
			} else {
				Some(parent.to_string())
			}
		})
		.collect();
	directories.sort();
	directories.dedup();
	directories
}

#[derive(Serialize)]
struct ParseMissionsArgs {
	contents: String,
}

/// `parse_missions` 的返回：解析出的国策记录；文件语法被自动纠正时附带规范化后的内容。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ParsedMissionsFile {
	missions: Vec<MissionRecord>,
	#[serde(default)]
	corrected_contents: Option<String>,
}

#[derive(Clone)]
struct LoadProgress {
	stage: String,
	completed: usize,
	total: usize,
}

/// 单批并发读取的图标数量，平衡 IPC 并发度与内存占用。
const ICON_READ_CHUNK_SIZE: usize = 8;

/// 一个已打开的国策树标签页：持有该树的配置与引用图标，关闭标签页即释放。
/// missions / icons 用共享句柄存储：父级重渲染时只做引用计数克隆与指针比较，
/// 不再深拷贝/深比较整棵数据（大图标库可达数 MB）。
#[derive(Clone, PartialEq)]
struct TreeTab {
	id: String,
	title: String,
	json_path: String,
	missions: Shared<Vec<MissionRecord>>,
	icons: Shared<Vec<FocusIcon>>,
	dirty: bool,
	save_ack: u64,
}

/// 按分区存放的撤销/重做栈（非响应式容器，避免高频注册触发整树重渲染）。
#[derive(Default)]
struct ZoneUndoStacks {
	canvas_undo: Vec<UndoEntry>,
	canvas_redo: Vec<UndoEntry>,
	events_undo: Vec<UndoEntry>,
	events_redo: Vec<UndoEntry>,
	explorer_undo: Vec<UndoEntry>,
	explorer_redo: Vec<UndoEntry>,
}

impl ZoneUndoStacks {
	fn undo_mut(&mut self, zone: UndoZone) -> &mut Vec<UndoEntry> {
		match zone {
			UndoZone::Canvas => &mut self.canvas_undo,
			UndoZone::Events => &mut self.events_undo,
			UndoZone::Explorer => &mut self.explorer_undo,
		}
	}

	fn redo_mut(&mut self, zone: UndoZone) -> &mut Vec<UndoEntry> {
		match zone {
			UndoZone::Canvas => &mut self.canvas_redo,
			UndoZone::Events => &mut self.events_redo,
			UndoZone::Explorer => &mut self.explorer_redo,
		}
	}

	fn clear(&mut self) {
		*self = Self::default();
	}

	fn depths(&self) -> UndoDepths {
		UndoDepths {
			canvas_undo: self.canvas_undo.len(),
			canvas_redo: self.canvas_redo.len(),
			events_undo: self.events_undo.len(),
			events_redo: self.events_redo.len(),
			explorer_undo: self.explorer_undo.len(),
			explorer_redo: self.explorer_redo.len(),
		}
	}
}

/// 单个国策树画布：以 key 挂载保持编辑状态，非激活时隐藏而非卸载。
#[component]
fn TreeCanvas(
	tab: TreeTab,
	active_tab_id: Signal<Option<String>>,
	save_request: Signal<u64>,
	save_status: String,
	focus_request: Signal<Option<(FocusTarget, u64)>>,
	pending_icon: Signal<Option<(String, FocusIcon)>>,
	// 画布卡片剪贴板（跨标签页复制/粘贴）。
	mind_clipboard: Signal<Option<MindClipboard>>,
	// 粘贴事件脚本（空白菜单「粘贴国策+事件」/ 卡片菜单「粘贴事件」共用）：
	// 把源事件脚本复制到目标资源根下的新脚本。
	on_copy_event_script: EventHandler<(String, String, String, String)>,
	on_undo_push: EventHandler<UndoRegistration>,
	on_save: EventHandler<(String, Vec<MissionRecord>)>,
	on_edit_event: EventHandler<(String, String)>,
	on_create_event_file: EventHandler<(String, String, String)>,
	on_delete_event_file: EventHandler<(String, String)>,
	on_rename_event_file: EventHandler<(String, String, String)>,
	on_nodes_change: EventHandler<(Vec<String>, Vec<String>)>,
	on_dirty_change: EventHandler<(String, bool)>,
	// 工作区现有事件脚本相对路径（画布判断「空国策」用）。
	event_files: Shared<Vec<String>>,
) -> Element {
	let save_tab_id = tab.id.clone();
	let dirty_tab_id = tab.id.clone();
	let canvas_tab_id = tab.id.clone();
	let tab_missions_root =
		missions_root_of(&tab.json_path).unwrap_or_else(|| "missions".to_string());
	let event_root_edit = tab_missions_root.clone();
	let event_root_create = tab_missions_root.clone();
	let event_root_delete = tab_missions_root.clone();
	let event_root_rename = tab_missions_root.clone();
	let is_active = active_tab_id.read().as_deref() == Some(tab.id.as_str());
	rsx! {
        div {
            class: "tab-canvas",
            style: if is_active { "display: flex;" } else { "display: none;" },
            MindMapCanvas {
                initial_icons: tab.icons.clone(),
                missions: tab.missions.clone(),
                missions_root: tab_missions_root.clone(),
                event_files: event_files.clone(),
                clipboard: mind_clipboard,
                on_copy_event_script,
                active_tab_id,
                tab_id: canvas_tab_id,
                on_save: move |records| on_save.call((save_tab_id.clone(), records)),
                save_request,
                save_status,
                on_edit_event: move |file_name: String| {
                    on_edit_event.call((event_root_edit.clone(), file_name))
                },
                on_create_event_file: move |(file_name, contents): (String, String)| {
                    on_create_event_file.call((event_root_create.clone(), file_name, contents))
                },
                on_delete_event_file: move |file_name: String| {
                    on_delete_event_file.call((event_root_delete.clone(), file_name))
                },
                on_rename_event_file: move |(old_name, new_name): (String, String)| {
                    on_rename_event_file.call((event_root_rename.clone(), old_name, new_name))
                },
                focus_request,
                on_undo_push,
                on_nodes_change,
                on_dirty_change: move |dirty| on_dirty_change.call((dirty_tab_id.clone(), dirty)),
                pending_icon,
                save_ack: tab.save_ack,
            }
        }
    }
}

/// 可拖拽调整宽度的侧边面板。
#[derive(Clone, Copy, PartialEq)]
enum ResizeSide {
	Explorer,
	Events,
}

/// 一次拖拽调整宽度的进行状态。
#[derive(Clone, Copy)]
struct PanelResize {
	side: ResizeSide,
	start_x: f64,
	start_width: f64,
}

/// 左右侧边面板的宽度限制（像素）。
const EXPLORER_MIN_WIDTH: f64 = 170.0;
const EXPLORER_MAX_WIDTH: f64 = 640.0;
const EVENTS_MIN_WIDTH: f64 = 260.0;
const EVENTS_MAX_WIDTH: f64 = 760.0;

#[derive(Serialize)]
struct SerializeMissionsArgs {
	missions: Vec<MissionRecord>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DirectoryNameArgs {
	directory_name: String,
}

#[component]
pub fn Work() -> Element {
	let work_directory = use_signal(|| None::<WorkDirectory>);
	let workspace_files = use_signal(Vec::<WorkspaceFile>::new);
	let mut open_tabs = use_signal(Vec::<TreeTab>::new);
	let mut active_tab_id = use_signal(|| None::<String>);
	// 资源管理器手动加载的图标：双击 .png 时推送给对应标签页画布并入其图标列表。
	let pending_icon = use_signal(|| None::<(String, FocusIcon)>);
	let mut event_release_request = use_signal(|| 0_u64);
	let mut tab_close_prompt = use_signal(|| None::<String>);
	let loading = use_signal(|| false);
	// 后端监视线程检测到工作区目录变化（外部修改）时置真，由刷新 effect 消费。
	let workspace_changed = use_signal(|| false);
	let load_progress = use_signal(|| None::<LoadProgress>);
	// APK 操作进度条的文字覆盖（如“512.3 MB / 870.1 MB”），为空时用默认计数。
	let progress_counter = use_signal(|| None::<String>);
	// 多个“APK 内容目录”并存时的打包选择对话框选项。
	let mut pack_choice = use_signal(|| None::<Vec<String>>);
	// 打包/导出前发现的「未使用脚本副本」：Some((相对路径列表, 用户确认后继续的操作))。
	let unused_scripts_prompt = use_signal(|| None::<(Vec<String>, PendingApkOperation)>);
	// 「未使用脚本副本」对话框中已勾选的脚本（空 = 未选择；点「删除」时未选择 = 全部删除）。
	let unused_scripts_checked = use_signal(Vec::<String>::new);
	// BKS 密钥库密码输入：Some(key_path) 时显示密码对话框。
	let mut signing_key_prompt = use_signal(|| None::<String>);
	let mut signing_key_password = use_signal(String::new);
	let mut signing_key_error = use_signal(String::new);
	let mut files_open = use_signal(|| true);
	let mut events_open = use_signal(|| false);
	let events_root = use_signal(|| None::<String>);
	let open_event_request = use_signal(|| None::<(String, u64)>);
	let rename_event_request = use_signal(|| None::<(String, String)>);
	let mut selected_file = use_signal(|| None::<String>);
	let explorer_clipboard = use_signal(|| None::<ExplorerClipboard>);
	// 资源管理器文件操作（复制/移动/删除）的浮动进度卡：
	// active 为并发操作计数，归零时清空 progress。
	let file_op_active = use_signal(|| 0_usize);
	let file_op_progress = use_signal(|| None::<FileOpProgressView>);
	// 资源管理器「重命名」对话框：(相对路径, 是否目录)。
	let mut rename_prompt = use_signal(|| None::<(String, bool)>);
	let mut rename_value = use_signal(String::new);
	let mut rename_error = use_signal(String::new);
	// 画布卡片剪贴板（跨标签页复制/粘贴：含图标数据与事件脚本引用）。
	let mind_clipboard = use_signal(|| None::<MindClipboard>);
	let mut mind_node_names = use_signal(Vec::<String>::new);
	let mut mind_node_images = use_signal(Vec::<String>::new);
	let mut delete_prompt = use_signal(|| None::<(String, String)>);
	let mind_focus_request = use_signal(|| None::<(FocusTarget, u64)>);
	let focus_seq = use_signal(|| 0_u64);
	let mut explorer_width = use_signal(|| 246.0);
	let mut events_width = use_signal(|| 340.0);
	let mut resizing = use_signal(|| None::<PanelResize>);
	let load_error = use_signal(String::new);
	let save_status = use_signal(String::new);
	// APK 操作（解压/打包并签名）的最近结果提示，显示在底部状态栏。
	let apk_status = use_signal(String::new);
	// 补全数据刷新纪元：设置「补全数据源 APK」后 +1，事件编辑器据此重新加载对照表。
	let lookup_epoch = use_signal(|| 0_u64);
	let mut save_request = use_signal(|| 0_u64);
	let mut directory_name = use_signal(|| "GameCivs".to_string());
	let android_platform = use_signal(|| false);
	let all_files_access_granted = use_signal(|| None::<bool>);
	let permission_error = use_signal(String::new);
	// 分区撤销/重做：画布、事件编辑、资源管理器各自独立记录操作；
	// 标题栏按钮与 Ctrl+Z/Y 只作用于“当前焦点所在分区”。
	// 栈内容放非响应式容器：事件编辑逐键合并时不触发整树重渲染；仅深度用信号驱动按钮状态。
	let undo_stacks: Rc<RefCell<ZoneUndoStacks>> =
		use_hook(|| Rc::new(RefCell::new(ZoneUndoStacks::default())));
	let undo_depths = use_signal(UndoDepths::default);
	// 当前撤销目标分区：由鼠标/焦点最后落在哪个面板决定（点画布→只撤画布，等等）。
	let mut active_zone = use_signal(|| UndoZone::Canvas);
	// 事件面板当前打开的文件（「根/missionsEvents/文件」）：事件分区条目的可用性据此判断。
	let event_active_scope = use_signal(|| None::<String>);

	// 子组件注册可撤销操作：按 scope 分派到对应分区；连续事件编辑在时间窗内合并为一步。
	// 注：Rc 非 Copy，每个执行器闭包都先用 clone 捕获自己的句柄。
	// use_callback 保持稳定身份：Work 重渲染时子组件 props 可整体相等而被记忆化跳过。
	let on_undo_push = use_callback({
		let undo_stacks = undo_stacks.clone();
		move |(scope, undo, redo): UndoRegistration| {
			let mut undo_depths = undo_depths;
			let zone = scope.zone();
			let now = now_ms();
			let mut depth_changed = true;
			{
				let mut stacks = undo_stacks.borrow_mut();
				// 新操作使该分区的重做栈失效（有内容被清空则需刷新深度）。
				let redo_stack = stacks.redo_mut(zone);
				let redo_cleared = !redo_stack.is_empty();
				redo_stack.clear();
				let is_event_edit = matches!(&scope, UndoScope::EventFile(_));
				let undo_stack = stacks.undo_mut(zone);
				let mergeable = is_event_edit
					&& undo_stack.last().is_some_and(|last| {
						last.scope == scope && now - last.timestamp < UNDO_MERGE_WINDOW_MS
					});
				if mergeable {
					if let Some(last) = undo_stack.last_mut() {
						last.redo = redo;
						last.timestamp = now;
					}
					depth_changed = false;
				} else {
					undo_stack.push(UndoEntry {
						scope,
						timestamp: now,
						undo,
						redo,
					});
					if undo_stack.len() > UNDO_LIMIT {
						undo_stack.remove(0);
					}
				}
				if depth_changed || redo_cleared {
					// 同值写入不会触发订阅，按钮状态按需更新。
					let depths = stacks.depths();
					undo_depths.set(depths);
				}
			}
		}
	});

	// 撤销 / 重做：只作用于当前焦点分区；跳过当前不可应用的条目（如其它事件文件的操作）。
	let on_undo = EventHandler::new({
		let undo_stacks = undo_stacks.clone();
		move |_: ()| {
			let mut undo_depths = undo_depths;
			let mut active_tab_id = active_tab_id;
			let zone = *active_zone.read();
			let event_scope = event_active_scope.read().clone();
			let entry = {
				let mut stacks = undo_stacks.borrow_mut();
				pop_applicable(stacks.undo_mut(zone), |entry| match &entry.scope {
					UndoScope::EventFile(file) => {
						event_scope.as_deref() == Some(file.as_str())
					}
					_ => true,
				})
			};
			let Some(entry) = entry else {
				return;
			};
			// 画布条目：先激活对应标签页再应用，保证用户能看到变化。
			if let UndoScope::Tab(tab_id) = &entry.scope {
				active_tab_id.set(Some(tab_id.clone()));
			}
			entry.undo.call(());
			let mut stacks = undo_stacks.borrow_mut();
			stacks.redo_mut(zone).push(entry);
			undo_depths.set(stacks.depths());
		}
	});
	let on_redo = EventHandler::new({
		let undo_stacks = undo_stacks.clone();
		move |_: ()| {
			let mut undo_depths = undo_depths;
			let mut active_tab_id = active_tab_id;
			let zone = *active_zone.read();
			let event_scope = event_active_scope.read().clone();
			let entry = {
				let mut stacks = undo_stacks.borrow_mut();
				pop_applicable(stacks.redo_mut(zone), |entry| match &entry.scope {
					UndoScope::EventFile(file) => {
						event_scope.as_deref() == Some(file.as_str())
					}
					_ => true,
				})
			};
			let Some(entry) = entry else {
				return;
			};
			if let UndoScope::Tab(tab_id) = &entry.scope {
				active_tab_id.set(Some(tab_id.clone()));
			}
			entry.redo.call(());
			let mut stacks = undo_stacks.borrow_mut();
			stacks.undo_mut(zone).push(entry);
			undo_depths.set(stacks.depths());
		}
	});

	// 监听 APK 操作进度事件，驱动加载面板的进度条（解压/打包/签名）。
	use_hook(move || {
		let mut load_progress = load_progress;
		let mut progress_counter = progress_counter;
		listen_apk_progress(move |payload| {
			let completed = payload.completed.max(0.0) as u64;
			let total = payload.total.max(0.0) as u64;
			let counter = if payload.unit == "bytes" {
				format!("{} / {}", format_size(completed), format_size(total))
			} else {
				format!("{completed} / {total} 个文件")
			};
			progress_counter.set(Some(counter));
			load_progress.set(Some(LoadProgress {
				stage: payload.stage,
				completed: completed as usize,
				total: total as usize,
			}));
		});
	});

	// 监听资源管理器文件操作进度（复制/移动/删除大文件夹），驱动浮动进度卡。
	// 同样处于 dioxus 作用域之外：回调内只置信号。
	use_hook(move || {
		let mut file_op_progress = file_op_progress;
		listen_file_op_progress(move |payload| {
			file_op_progress.set(Some(FileOpProgressView {
				stage: payload.stage,
				completed: payload.completed,
				total: payload.total,
			}));
		});
	});

	// 监听工作区目录变化事件（后端轮询目录签名）：外部修改（如系统文件管理器）后
	// 自动刷新资源管理器列表。注意：该回调由 JS 事件触发、处于 dioxus 作用域之外，
	// 只能做不依赖作用域的操作——这里仅置一个信号（与 apk-progress 监听同款做法）；
	// 在回调里直接 `spawn`/读信号会触发 dioxus 运行时 panic（RefCell already borrowed）。
	use_hook(move || {
		let mut workspace_changed = workspace_changed;
		listen_workspace_changed(move || {
			workspace_changed.set(true);
		});
	});

	// 工作区变化刷新：事件请求且未处于加载中时重载文件列表（在响应式上下文里执行）。
	// 加载中保留请求直接返回——loading 随后变化会让本 effect 重跑，届时再刷新；
	// 刷新失败（如目录刚被删除）静默忽略，下一轮轮询会再通知。
	use_effect(move || {
		let mut workspace_changed = workspace_changed;
		if !*workspace_changed.read() {
			return;
		}
		if *loading.read() {
			return;
		}
		let Some(directory) = work_directory.read().clone() else {
			return;
		};
		workspace_changed.set(false);
		let mut workspace_files = workspace_files;
		spawn(async move {
			if let Ok(files) = load_workspace_files(&directory).await {
				workspace_files.set(files);
			}
		});
	});

	// 工作区监视开关：打开工作区且空闲（真实路径模式）时启动后端轮询；
	// 加载中/未打开/SAF 模式停止（加载期间的变化由操作自身结束后的刷新覆盖）。
	use_effect(move || {
		let directory = work_directory.read().clone();
		let is_loading = *loading.read();
		spawn(async move {
			let watching = matches!(
				&directory,
				Some(directory) if !is_loading && directory.folder_id.is_none()
			);
			if watching {
				let root = directory
					.as_ref()
					.map(|directory| directory.root_path.clone())
					.unwrap_or_default();
				if let Ok(args) = serde_wasm_bindgen::to_value(&WorkDirectoryArgs {
					work_directory: root,
				}) {
					let _ = JsFuture::from(invoke("start_workspace_watch", args)).await;
				}
			} else {
				let _ = JsFuture::from(invoke(
					"stop_workspace_watch",
					js_sys::Object::new().into(),
				))
				.await;
			}
		});
	});

	use_effect(move || {
		let mut android_platform = android_platform;
		let mut all_files_access_granted = all_files_access_granted;
		spawn(async move {
			if matches!(is_android().await, Ok(true)) {
				android_platform.set(true);
				all_files_access_granted.set(query_all_files_access().await.ok());
			}
		});
	});

	let on_manage_all_files = move |_| {
		let mut all_files_access_granted = all_files_access_granted;
		let mut permission_error = permission_error;
		spawn(async move {
			permission_error.set(String::new());
			match open_all_files_access_settings().await {
				Ok(granted) => all_files_access_granted.set(Some(granted)),
				Err(error) => permission_error.set(error),
			}
		});
	};

	let on_choose_directory = EventHandler::new({
		let undo_stacks = undo_stacks.clone();
		move |_: ()| {
		let mut work_directory = work_directory;
		let mut workspace_files = workspace_files;
		let mut open_tabs = open_tabs;
		let mut active_tab_id = active_tab_id;
		let mut loading = loading;
		let mut load_progress = load_progress;
		let mut load_error = load_error;
		let mut selected_file = selected_file;
		let mut event_release_request = event_release_request;
		let mut mind_node_names = mind_node_names;
		let mut mind_node_images = mind_node_images;
		let mut events_root = events_root;
		let mut undo_depths = undo_depths;
		let undo_stacks = undo_stacks.clone();
		spawn(async move {
			loading.set(true);
			load_error.set(String::new());
			load_progress.set(Some(LoadProgress {
				stage: "正在扫描工作区文件...".to_string(),
				completed: 0,
				total: 0,
			}));
			match pick_folder("选择 GameCivs 工作目录").await {
				Ok(Some(selected_directory)) => {
					match load_workspace_files(&selected_directory).await {
						Ok(loaded_files) => {
							open_tabs.set(Vec::new());
							active_tab_id.set(None);
							selected_file.set(None);
							event_release_request.with_mut(|request| {
								*request = request.wrapping_add(1);
							});
							mind_node_names.set(Vec::new());
							mind_node_images.set(Vec::new());
							events_root.set(None);
							// 切换工作区后旧撤销步骤不再可用。
							undo_stacks.borrow_mut().clear();
							undo_depths.set(UndoDepths::default());
							workspace_files.set(loaded_files);
							work_directory.set(Some(selected_directory));
						}
						Err(error) => load_error.set(error),
					}
				}
				Ok(None) => {}
				Err(error) => load_error.set(error),
			}
			loading.set(false);
			load_progress.set(None);
		});
	}
	});

	let on_create_directory = EventHandler::new({
		let undo_stacks = undo_stacks.clone();
		move |_: ()| {
		let mut work_directory = work_directory;
		let mut workspace_files = workspace_files;
		let mut open_tabs = open_tabs;
		let mut active_tab_id = active_tab_id;
		let mut selected_file = selected_file;
		let mut event_release_request = event_release_request;
		let mut mind_node_names = mind_node_names;
		let mut mind_node_images = mind_node_images;
		let mut events_root = events_root;
		let mut loading = loading;
		let mut load_progress = load_progress;
		let mut load_error = load_error;
		let mut undo_depths = undo_depths;
		let undo_stacks = undo_stacks.clone();
		let directory_name = directory_name.read().trim().to_string();
		spawn(async move {
			loading.set(true);
			load_error.set(String::new());
			load_progress.set(Some(LoadProgress {
				stage: "正在创建并载入工作区...".to_string(),
				completed: 0,
				total: 0,
			}));
			let result = async {
				let Some(parent_directory) =
					pick_folder("选择新工作目录的存放位置").await?
				else {
					return Ok(None);
				};
				let created_directory = if let Some(folder_id) = parent_directory.folder_id {
					let validation = serde_wasm_bindgen::to_value(&DirectoryNameArgs {
						directory_name: directory_name.clone(),
					})
					.map_err(|error| error.to_string())?;
					JsFuture::from(invoke("validate_work_directory_name_command", validation))
						.await
						.map_err(|error| format!("工作区名称无效：{error:?}"))?;
					let root_path = join_scoped_path(&parent_directory.root_path, &directory_name);
					scoped_mkdir(&folder_id, root_path.clone()).await?;
					scoped_mkdir(
						&folder_id,
						join_scoped_path(&root_path, "missions/missionsImages/H"),
					)
					.await?;
					scoped_mkdir(
						&folder_id,
						join_scoped_path(&root_path, "missions/missionsEvents"),
					)
					.await?;
					let args = serde_wasm_bindgen::to_value(&SerializeMissionsArgs {
						missions: Vec::new(),
					})
					.map_err(|error| error.to_string())?;
					let value = JsFuture::from(invoke("serialize_missions", args))
						.await
						.map_err(|error| format!("初始化任务配置失败：{error:?}"))?;
					let contents = value
						.as_string()
						.ok_or_else(|| "任务配置序列化结果无效".to_string())?;
					scoped_write_text_file(
						&folder_id,
						join_scoped_path(&root_path, "missions/Missions.json"),
						contents,
					)
					.await?;
					WorkDirectory {
						display_path: format!(
							"{}/{}",
							parent_directory.display_path, directory_name
						),
						folder_id: Some(folder_id),
						root_path,
						tree_uri: None,
					}
				} else {
					let parent_tree = parent_directory.tree_uri.clone();
					let child_name = directory_name.clone();
					let args = serde_wasm_bindgen::to_value(&CreateWorkDirectoryArgs {
						parent_directory: parent_directory.root_path,
						directory_name,
					})
					.map_err(|error| error.to_string())?;
					let value = JsFuture::from(invoke("create_work_directory", args))
						.await
						.map_err(|error| format!("创建工作目录失败：{error:?}"))?;
					let path = value
						.as_string()
						.ok_or_else(|| "创建目录命令未返回路径".to_string())?;
					// Android 真实路径模式：记录新建目录在 SAF 授权树中的文档 URI（「打开文件位置」用）。
					let tree_uri = match parent_tree {
						Some(parent_tree) => {
							resolve_scoped_child_uri(&parent_tree, &child_name).await.ok()
						}
						None => None,
					};
					WorkDirectory {
						display_path: path.clone(),
						folder_id: None,
						root_path: path,
						tree_uri,
					}
				};
				Ok(Some(created_directory))
			}
			.await;

			match result {
				Ok(Some(created_directory)) => {
					match load_workspace_files(&created_directory).await {
						Ok(loaded_files) => {
							open_tabs.set(Vec::new());
							active_tab_id.set(None);
							selected_file.set(None);
							event_release_request.with_mut(|request| {
								*request = request.wrapping_add(1);
							});
							mind_node_names.set(Vec::new());
							mind_node_images.set(Vec::new());
							events_root.set(None);
							// 切换工作区后旧撤销步骤不再可用。
							undo_stacks.borrow_mut().clear();
							undo_depths.set(UndoDepths::default());
							let default_tree = default_tree_path(
								loaded_files.iter().map(|file| file.relative_path.as_str()),
							);
							workspace_files.set(loaded_files);
							work_directory.set(Some(created_directory));
							// 自动打开默认国策树（新建工作区为 missions/Missions.json）。
							if let Some(tree_path) = default_tree {
								load_progress.set(Some(LoadProgress {
									stage: "正在打开初始国策树...".to_string(),
									completed: 0,
									total: 0,
								}));
								let starter_directory = work_directory.read().clone();
								if let Some(starter_directory) = starter_directory {
									match load_tree_records(&starter_directory, &tree_path).await {
										Ok(records) => {
											let title = tree_tab_title(&tree_path);
											open_tabs.with_mut(|tabs| {
												tabs.push(TreeTab {
													id: tree_path.clone(),
													title,
													json_path: tree_path.clone(),
													missions: Shared::new(records),
													icons: Shared::new(Vec::new()),
													dirty: false,
													save_ack: 0,
												});
											});
											active_tab_id.set(Some(tree_path));
										}
										Err(error) => load_error.set(error),
									}
								}
							}
						}
						Err(error) => load_error.set(error),
					}
				}
				Ok(None) => {}
				Err(error) => load_error.set(error),
			}
			loading.set(false);
			load_progress.set(None);
		});
	}
	});

	// 解压 APK 到工作区（Android scoped 模式无法进行，需要真实路径）。
	let on_extract_apk = EventHandler::new(move |_: ()| {
		let work_directory = work_directory;
		let mut workspace_files = workspace_files;
		let mut loading = loading;
		let mut load_progress = load_progress;
		let mut progress_counter = progress_counter;
		let mut load_error = load_error;
		let mut apk_status = apk_status;
		spawn(async move {
			let Some(directory) = work_directory.read().clone() else {
				apk_status.set("解压失败：请先打开工作区".to_string());
				return;
			};
			if directory.folder_id.is_some() {
				apk_status.set(
					"解压 APK 需要真实路径模式：请在「文件 → 全盘文件访问权限」中授权后重新打开工作区"
						.to_string(),
				);
				return;
			}
			match pick_apk_file("选择要解压的 APK").await {
				Ok(Some(apk)) => {
					// Android 选择器无法按扩展名强过滤：显示名明确非 .apk 时提示重选。
					if !apk.name.is_empty() && !apk.name.to_ascii_lowercase().ends_with(".apk") {
						apk_status.set(format!(
							"「{}」不是 apk 文件，请重新选择 .apk 安装包",
							apk.name
						));
						return;
					}
					loading.set(true);
					load_error.set(String::new());
					apk_status.set("正在解压 APK...".to_string());
					progress_counter.set(None);
					load_progress.set(Some(LoadProgress {
						stage: "正在解压 APK 到工作区...".to_string(),
						completed: 0,
						total: 0,
					}));
					let apk_name = (!apk.name.is_empty()).then_some(apk.name.as_str());
					match extract_apk_into_workspace(&directory.root_path, &apk.location, apk_name).await {
						Ok(message) => {
							apk_status.set(message);
							// 解压后仅刷新文件列表，不自动打开任何 Missions.json（由用户自行选择）。
							if let Ok(files) = load_workspace_files(&directory).await {
								workspace_files.set(files);
							}
						}
						Err(error) => apk_status.set(error),
					}
					loading.set(false);
					load_progress.set(None);
					progress_counter.set(None);
				}
				Ok(None) => {}
				Err(error) => apk_status.set(error),
			}
		});
	});

	// 「从 apk 中导入」：优先资源管理器高亮的 apk，否则打开文件管理器选择；
	// 导入全局国策与各剧本版块（地图 / 剧本目录名自动识别，目录保留完整路径）。
	let on_import_apk = EventHandler::new(move |_: ()| {
		let work_directory = work_directory;
		let mut workspace_files = workspace_files;
		let mut loading = loading;
		let mut load_progress = load_progress;
		let mut progress_counter = progress_counter;
		let mut load_error = load_error;
		let mut apk_status = apk_status;
		let selected = selected_file.read().clone();
		spawn(async move {
			let Some(directory) = work_directory.read().clone() else {
				apk_status.set("导入失败：请先打开工作区".to_string());
				return;
			};
			if directory.folder_id.is_some() {
				apk_status.set(
					"导入 APK 需要真实路径模式：请在「文件 → 全盘文件访问权限」中授权后重新打开工作区"
						.to_string(),
				);
				return;
			}
			let (apk_path, apk_name) =
				match resolve_target_apk(&directory, selected, "选择要导入版块的 APK").await {
					Ok(Some(target)) => target,
					Ok(None) => return,
					Err(error) => {
						apk_status.set(error);
						return;
					}
				};
			if let Some(name) = apk_name.as_deref() {
				if !name.to_ascii_lowercase().ends_with(".apk") {
					apk_status.set(format!("「{name}」不是 apk 文件，请重新选择 .apk 安装包"));
					return;
				}
			}
			loading.set(true);
			load_error.set(String::new());
			apk_status.set("正在从 APK 导入版块...".to_string());
			progress_counter.set(None);
			load_progress.set(Some(LoadProgress {
				stage: "正在从 APK 导入 missions/scenarios...".to_string(),
				completed: 0,
				total: 0,
			}));
			match import_apk_sections_into_workspace(
				&directory.root_path,
				&apk_path,
				apk_name.as_deref(),
			)
			.await
			{
				Ok(message) => {
					apk_status.set(message);
					if let Ok(files) = load_workspace_files(&directory).await {
						workspace_files.set(files);
					}
				}
				Err(error) => apk_status.set(error),
			}
			loading.set(false);
			load_progress.set(None);
			progress_counter.set(None);
		});
	});

	// 「导出到 apk」：优先资源管理器高亮的 apk，否则打开文件管理器选择；
	// 以更新替换方式把工作区版块写回（APK 中多余条目保留，不清空）。
	let on_export_apk = EventHandler::new(move |_: ()| {
		let work_directory = work_directory;
		let workspace_files = workspace_files;
		let mut loading = loading;
		let mut load_progress = load_progress;
		let mut progress_counter = progress_counter;
		let mut load_error = load_error;
		let mut apk_status = apk_status;
		let unused_scripts_prompt = unused_scripts_prompt;
		let unused_scripts_checked = unused_scripts_checked;
		let selected = selected_file.read().clone();
		spawn(async move {
			let Some(directory) = work_directory.read().clone() else {
				apk_status.set("导出失败：请先打开工作区".to_string());
				return;
			};
			let (apk_path, apk_name) =
				match resolve_target_apk(&directory, selected, "选择要导出到的 APK").await {
					Ok(Some(target)) => target,
					Ok(None) => return,
					Err(error) => {
						apk_status.set(error);
						return;
					}
				};
			if let Some(name) = apk_name.as_deref() {
				if !name.to_ascii_lowercase().ends_with(".apk") {
					apk_status.set(format!("「{name}」不是 apk 文件，请重新选择 .apk 安装包"));
					return;
				}
			}
			// 导出前检查：missionsEvents 内未被国策引用的脚本副本，交给用户处理后再继续。
			let mut unused_scripts_prompt = unused_scripts_prompt;
			let mut unused_scripts_checked = unused_scripts_checked;
			let scan_note = if directory.folder_id.is_some() {
				// SAF 模式导出本身会失败；检查也依赖真实路径，跳过。
				None
			} else {
				// 检查期间显示加载进度（后端通过 apk-progress 事件上报扫描进度）。
				loading.set(true);
				load_error.set(String::new());
				apk_status.set("正在检查未使用脚本...".to_string());
				progress_counter.set(None);
				load_progress.set(Some(LoadProgress {
					stage: "正在检查未使用脚本...".to_string(),
					completed: 0,
					total: 0,
				}));
				let scan_result = find_unused_event_scripts(&directory, "").await;
				loading.set(false);
				load_progress.set(None);
				progress_counter.set(None);
				match scan_result {
					Ok(scripts) if !scripts.is_empty() => {
						unused_scripts_checked.set(Vec::new());
						unused_scripts_prompt.set(Some((
							scripts,
							PendingApkOperation::Export {
								apk_path: apk_path.clone(),
								apk_name: apk_name.clone(),
							},
						)));
						return;
					}
					Ok(_) => None,
					Err(error) => Some(format!("未使用脚本检查失败：{error}")),
				}
			};
			perform_apk_operation(
				&directory,
				PendingApkOperation::Export { apk_path, apk_name },
				loading,
				load_progress,
				progress_counter,
				apk_status,
				load_error,
				workspace_files,
			)
			.await;
			if let Some(note) = scan_note {
				apk_status.with_mut(|status| *status = format!("{status}（{note}）"));
			}
		});
	});

	// 「指定补全数据 APK」：把游戏 APK 位置记入工作区根标记；事件编辑器在缺少游戏数据文件
	// （如只有 missions / scenarios 版块的工作区）时直接从该 APK 读取补全数据。
	let on_set_lookup_apk = EventHandler::new(move |_: ()| {
		let work_directory = work_directory;
		let mut apk_status = apk_status;
		let mut lookup_epoch = lookup_epoch;
		let selected = selected_file.read().clone();
		spawn(async move {
			let Some(directory) = work_directory.read().clone() else {
				apk_status.set("设置失败：请先打开工作区".to_string());
				return;
			};
			let (apk_path, apk_name) = match resolve_target_apk(
				&directory,
				selected,
				"选择作为补全数据源的 APK",
			)
			.await
			{
				Ok(Some(target)) => target,
				Ok(None) => return,
				Err(error) => {
					apk_status.set(error);
					return;
				}
			};
			if let Some(name) = apk_name.as_deref() {
				if !name.to_ascii_lowercase().ends_with(".apk") {
					apk_status.set(format!("「{name}」不是 apk 文件，请重新选择 .apk 安装包"));
					return;
				}
			}
			apk_status.set("正在设置补全数据源...".to_string());
			match set_lookup_source_apk(&directory, &apk_path).await {
				Ok(message) => {
					apk_status.set(message);
					// 事件编辑器按（工作区 | 资源根）缓存对照表：纪元 +1 强制重新加载。
					lookup_epoch.with_mut(|value| *value = value.wrapping_add(1));
				}
				Err(error) => apk_status.set(error),
			}
		});
	});

	let run_package_apk = EventHandler::new(move |source_directory: Option<String>| {
		let work_directory = work_directory;
		let workspace_files = workspace_files;
		let mut loading = loading;
		let mut load_progress = load_progress;
		let mut progress_counter = progress_counter;
		let mut load_error = load_error;
		let mut apk_status = apk_status;
		let unused_scripts_prompt = unused_scripts_prompt;
		let unused_scripts_checked = unused_scripts_checked;
		spawn(async move {
			let Some(directory) = work_directory.read().clone() else {
				apk_status.set("打包失败：请先打开工作区".to_string());
				return;
			};
			if directory.folder_id.is_some() {
				apk_status.set(
					"打包 APK 需要真实路径模式：请在「文件 → 全盘文件访问权限」中授权后重新打开工作区"
						.to_string(),
				);
				return;
			}
			let source = source_directory.unwrap_or_default();
			// 打包前检查：打包范围（源目录子树）内未被国策引用的脚本副本，交给用户处理后再继续。
			let mut unused_scripts_prompt = unused_scripts_prompt;
			let mut unused_scripts_checked = unused_scripts_checked;
			// 检查期间显示加载进度（后端通过 apk-progress 事件上报扫描进度）。
			loading.set(true);
			load_error.set(String::new());
			apk_status.set("正在检查未使用脚本...".to_string());
			progress_counter.set(None);
			load_progress.set(Some(LoadProgress {
				stage: "正在检查未使用脚本...".to_string(),
				completed: 0,
				total: 0,
			}));
			let scan_result = find_unused_event_scripts(&directory, &source).await;
			loading.set(false);
			load_progress.set(None);
			progress_counter.set(None);
			let scan_note = match scan_result {
				Ok(scripts) if !scripts.is_empty() => {
					unused_scripts_checked.set(Vec::new());
					unused_scripts_prompt.set(Some((
						scripts,
						PendingApkOperation::Package {
							source_directory: source.clone(),
						},
					)));
					return;
				}
				Ok(_) => None,
				Err(error) => Some(format!("未使用脚本检查失败：{error}")),
			};
			perform_apk_operation(
				&directory,
				PendingApkOperation::Package {
					source_directory: source,
				},
				loading,
				load_progress,
				progress_counter,
				apk_status,
				load_error,
				workspace_files,
			)
			.await;
			if let Some(note) = scan_note {
				apk_status.with_mut(|status| *status = format!("{status}（{note}）"));
			}
		});
	});

	// 打包入口：优先选“APK 内容目录”（一级子目录中含 AndroidManifest.xml）；
	// 仅一个时直接打包，多个时弹窗选择，没有则回退打包工作区根。
	let on_package_apk = EventHandler::new(move |_: ()| {
		let mut pack_choice = pack_choice;
		let candidates = package_candidates(&workspace_files.read());
		match candidates.len() {
			0 => run_package_apk.call(None),
			1 => run_package_apk.call(candidates.into_iter().next()),
			_ => pack_choice.set(Some(candidates)),
		}
	});

	// 实际执行签名（relative_path 为工作区内相对路径，或桌面端选择器返回的绝对路径）。
	let perform_sign_apk = EventHandler::new(move |relative_path: String| {
		let work_directory = work_directory;
		let mut loading = loading;
		let mut load_progress = load_progress;
		let mut progress_counter = progress_counter;
		let mut apk_status = apk_status;
		spawn(async move {
			let Some(directory) = work_directory.read().clone() else {
				apk_status.set("签名失败：请先打开工作区".to_string());
				return;
			};
			if directory.folder_id.is_some() {
				apk_status.set(
					"签名 APK 需要真实路径模式：请在「文件 → 全盘文件访问权限」中授权后重新打开工作区"
						.to_string(),
				);
				return;
			}
			loading.set(true);
			apk_status.set(format!("正在签名 {relative_path}（v1+v2+v3）..."));
			progress_counter.set(None);
			load_progress.set(Some(LoadProgress {
				stage: "正在签名 APK...".to_string(),
				completed: 0,
				total: 0,
			}));
			let result = sign_workspace_apk(&directory.root_path, &relative_path).await;
			loading.set(false);
			load_progress.set(None);
			progress_counter.set(None);
			match result {
				Ok(message) => apk_status.set(message),
				Err(error) => apk_status.set(error),
			}
		});
	});

	// 签名入口（菜单）：资源管理器高亮选中的 apk 优先；未正确选中时打开文件管理器选择。
	let on_sign_apk = EventHandler::new(move |_: ()| {
		let work_directory = work_directory;
		let mut apk_status = apk_status;
		let selected = selected_file.read().clone();
		spawn(async move {
			let Some(directory) = work_directory.read().clone() else {
				apk_status.set("签名失败：请先打开工作区".to_string());
				return;
			};
			if directory.folder_id.is_some() {
				apk_status.set(
					"签名 APK 需要真实路径模式：请在「文件 → 全盘文件访问权限」中授权后重新打开工作区"
						.to_string(),
				);
				return;
			}
			if let Some(selected) =
				selected.filter(|path| path.to_ascii_lowercase().ends_with(".apk"))
			{
				perform_sign_apk.call(selected);
				return;
			}
			match pick_apk_file("选择要签名的 APK").await {
				Ok(Some(apk)) => {
					// Android 选择器无法按扩展名强过滤：显示名明确非 .apk 时提示重选。
					if !apk.name.is_empty() && !apk.name.to_ascii_lowercase().ends_with(".apk") {
						apk_status.set(format!(
							"「{}」不是 apk 文件，请重新选择 .apk 安装包",
							apk.name
						));
						return;
					}
					perform_sign_apk.call(apk.location);
				}
				Ok(None) => {}
				Err(error) => apk_status.set(error),
			}
		});
	});

	// 执行密钥导入（BKS 携带密码）；成功关闭密码框，失败时优先显示在密码框内。
	let run_import_key = EventHandler::new(move |(key_path, password): (String, Option<String>)| {
		let work_directory = work_directory;
		let mut loading = loading;
		let mut load_progress = load_progress;
		let mut apk_status = apk_status;
		let mut signing_key_prompt = signing_key_prompt;
		let mut signing_key_password = signing_key_password;
		let mut signing_key_error = signing_key_error;
		spawn(async move {
			let Some(directory) = work_directory.read().clone() else {
				apk_status.set("添加密钥失败：请先打开工作区".to_string());
				return;
			};
			loading.set(true);
			load_progress.set(Some(LoadProgress {
				stage: "正在导入签名密钥...".to_string(),
				completed: 0,
				total: 0,
			}));
			let result =
				import_signing_key_to_workspace(&directory.root_path, &key_path, password).await;
			loading.set(false);
			load_progress.set(None);
			match result {
				Ok(message) => {
					signing_key_prompt.set(None);
					signing_key_error.set(String::new());
					apk_status.set(message);
				}
				Err(error) => {
					if signing_key_prompt.read().is_some() {
						signing_key_error.set(error);
					} else if error.contains("BKS") {
						// 未带密码导入 BKS（如 Android 选择器无法按扩展名判断）：自动弹密码框重试。
						signing_key_password.set(String::new());
						signing_key_error.set(String::new());
						signing_key_prompt.set(Some(key_path.clone()));
					} else {
						apk_status.set(error);
					}
				}
			}
		});
	});

	// 添加默认密钥入口：资源管理器高亮选中的 PEM / BKS 优先；未正确选中时打开文件管理器选择。
	let on_import_signing_key = EventHandler::new(move |_: ()| {
		let work_directory = work_directory;
		let mut apk_status = apk_status;
		let mut signing_key_prompt = signing_key_prompt;
		let mut signing_key_password = signing_key_password;
		let mut signing_key_error = signing_key_error;
		let selected = selected_file.read().clone();
		spawn(async move {
			let Some(directory) = work_directory.read().clone() else {
				apk_status.set("添加密钥失败：请先打开工作区".to_string());
				return;
			};
			if directory.folder_id.is_some() {
				apk_status.set(
					"添加密钥需要真实路径模式：请在「文件 → 全盘文件访问权限」中授权后重新打开工作区"
						.to_string(),
				);
				return;
			}
			// 资源管理器已高亮选中密钥文件（.pem/.bks）时直接使用（需要绝对路径）。
			if let Some(selected) = selected.filter(|path| {
				let lower = path.to_ascii_lowercase();
				lower.ends_with(".pem") || lower.ends_with(".bks")
			}) {
				let key_path = join_scoped_path(&directory.root_path, &selected);
				if selected.to_ascii_lowercase().ends_with(".bks") {
					signing_key_password.set(String::new());
					signing_key_error.set(String::new());
					signing_key_prompt.set(Some(key_path));
				} else {
					run_import_key.call((key_path, None));
				}
				return;
			}
			match pick_pem_file("选择签名密钥（PEM / BKS）").await {
				Ok(Some(key)) => {
					if key.name.to_ascii_lowercase().ends_with(".bks") {
						signing_key_password.set(String::new());
						signing_key_error.set(String::new());
						signing_key_prompt.set(Some(key.location));
					} else {
						run_import_key.call((key.location, None));
					}
				}
				Ok(None) => {}
				Err(error) => apk_status.set(error),
			}
		});
	});

	// 释放不再被任何标签页引用的事件脚本内存。
	let release_event_if_unused = EventHandler::new(move |_: ()| {
		let Some(path) = selected_file.read().clone() else {
			return;
		};
		let name = missions_subfile(&path, "missionsEvents")
			.map(|(_, name)| name)
			.unwrap_or_else(|| path.clone());
		let referenced = open_tabs.read().iter().any(|tab| {
			tab.missions
				.iter()
				.any(|mission| mission.mission_event == name)
		});
		if !referenced {
			event_release_request.with_mut(|request| *request = request.wrapping_add(1));
			selected_file.set(None);
		}
	});

	// 关闭标签页：从列表移除（对应画布卸载，图标与国策数据随之释放）。
	let on_close_tab = EventHandler::new({
		let undo_stacks = undo_stacks.clone();
		move |tab_id: String| {
		let mut undo_depths = undo_depths;
		dioxus_logger::tracing::info!("CLOSE-TAB: {tab_id}");
		// 先快照全部信号值（读守卫随语句结束释放），再统一写入，避免重入借用。
		let tabs_snapshot: Vec<TreeTab> = open_tabs.read().clone();
		let current_active = active_tab_id.read().clone();
		let was_active = current_active.as_deref() == Some(tab_id.as_str());
		let remaining: Vec<TreeTab> = tabs_snapshot
			.iter()
			.filter(|tab| tab.id != tab_id)
			.cloned()
			.collect();
		let next_active = if was_active {
			let removed_index = tabs_snapshot
				.iter()
				.position(|tab| tab.id == tab_id)
				.unwrap_or(0);
			remaining
				.get(removed_index.min(remaining.len().saturating_sub(1)))
				.map(|tab| tab.id.clone())
		} else {
			current_active
		};
		open_tabs.set(remaining);
		active_tab_id.set(next_active);
		if open_tabs.read().is_empty() {
			mind_node_names.set(Vec::new());
			mind_node_images.set(Vec::new());
		}
		// 丢弃该标签页的画布撤销步骤（画布已卸载，其信号不再有界面）。
		{
			let mut stacks = undo_stacks.borrow_mut();
			stacks
				.canvas_undo
				.retain(|entry| entry.scope != UndoScope::Tab(tab_id.clone()));
			stacks
				.canvas_redo
				.retain(|entry| entry.scope != UndoScope::Tab(tab_id.clone()));
			undo_depths.set(stacks.depths());
		}
		release_event_if_unused.call(());
	}
	});

	// 点 X 关闭：未保存修改先弹出确认，否则直接关闭。
	let on_close_tab_requested = EventHandler::new(move |tab_id: String| {
		dioxus_logger::tracing::info!("CLOSE-REQUESTED: {tab_id}");
		let dirty = open_tabs
			.read()
			.iter()
			.find(|tab| tab.id == tab_id)
			.map(|tab| tab.dirty)
			.unwrap_or(false);
		if dirty {
			tab_close_prompt.set(Some(tab_id));
		} else {
			on_close_tab.call(tab_id);
		}
	});

	// 双击 .json 打开（或激活）国策树标签页。
	let on_open_tree = EventHandler::new(move |json_path: String| {
		let mut load_error = load_error;
		let Some(directory) = work_directory.read().clone() else {
			load_error.set("打开国策树失败：请先打开工作区".to_string());
			return;
		};
		if let Some(tab) = open_tabs
			.read()
			.iter()
			.find(|tab| tab.json_path == json_path)
		{
			active_tab_id.set(Some(tab.id.clone()));
			return;
		}
		if *loading.read() {
			return;
		}
		let mut loading = loading;
		let mut load_progress = load_progress;
		let mut open_tabs = open_tabs;
		let mut active_tab_id = active_tab_id;
		spawn(async move {
			loading.set(true);
			load_error.set(String::new());
			load_progress.set(Some(LoadProgress {
				stage: "正在读取国策配置...".to_string(),
				completed: 0,
				total: 0,
			}));
			let result = async {
				let records = load_tree_records(&directory, &json_path).await?;
				load_progress.set(Some(LoadProgress {
					stage: "正在扫描国策图标...".to_string(),
					completed: 0,
					total: 0,
				}));
				let icons = load_tree_icons(&directory, &json_path, &records, load_progress).await?;
				Ok::<_, String>((records, icons))
			}
			.await;
			match result {
				Ok((records, icons)) => {
					let title = tree_tab_title(&json_path);
					open_tabs.with_mut(|tabs| {
						tabs.push(TreeTab {
							id: json_path.clone(),
							title,
							json_path: json_path.clone(),
							missions: Shared::new(records),
							icons: Shared::new(icons),
							dirty: false,
							save_ack: 0,
						});
					});
					active_tab_id.set(Some(json_path));
				}
				Err(error) => load_error.set(format!("打开国策树失败：{error}")),
			}
			loading.set(false);
			load_progress.set(None);
		});
	});

	// 文件列表变化后剪掉已不存在的国策树标签页。
	let prune_tabs = EventHandler::new(move |files: Vec<WorkspaceFile>| {
		let removed: Vec<String> = open_tabs
			.read()
			.iter()
			.filter(|tab| !files.iter().any(|file| file.relative_path == tab.json_path))
			.map(|tab| tab.id.clone())
			.collect();
		if removed.is_empty() {
			return;
		}
		let remaining: Vec<TreeTab> = open_tabs
			.read()
			.iter()
			.filter(|tab| !removed.contains(&tab.id))
			.cloned()
			.collect();
		let active_removed = active_tab_id
			.read()
			.as_ref()
			.map(|id| removed.contains(id))
			.unwrap_or(false);
		let next_active = if active_removed {
			remaining.first().map(|tab| tab.id.clone())
		} else {
			active_tab_id.read().clone()
		};
		open_tabs.set(remaining);
		active_tab_id.set(next_active);
		if open_tabs.read().is_empty() {
			mind_node_names.set(Vec::new());
			mind_node_images.set(Vec::new());
		}
		release_event_if_unused.call(());
	});

	// 保存单个国策树（仅激活标签页响应 Ctrl+S / 保存按钮）。
	let on_save_tab = EventHandler::new(move |(tab_id, records): (String, Vec<MissionRecord>)| {
		let Some(directory) = work_directory.read().clone() else {
			return;
		};
		let Some(json_path) = open_tabs
			.read()
			.iter()
			.find(|tab| tab.id == tab_id)
			.map(|tab| tab.json_path.clone())
		else {
			return;
		};
		let mut save_status = save_status;
		let mut open_tabs = open_tabs;
		spawn(async move {
			save_status.set("正在保存...".to_string());
			match save_tree_file(&directory, &json_path, records).await {
				Ok(()) => {
					open_tabs.with_mut(|tabs| {
						if let Some(tab) = tabs.iter_mut().find(|tab| tab.json_path == json_path) {
							tab.save_ack = tab.save_ack.wrapping_add(1);
						}
					});
					save_status.set("保存完成".to_string());
					// 「保存完成」短暂提示后自动收起；期间若状态又被改写（如新一轮「正在保存...」），
					// 按当前文本比对，延时回调不会误清。（保存失败信息保留供阅读。）
					sleep_ms(1500).await;
					if save_status.peek().as_str() == "保存完成" {
						save_status.set(String::new());
					}
				}
				Err(error) => save_status.set(format!("保存失败：{error}")),
			}
		});
	});

	// 画布脏状态回传，关闭标签页前提示未保存修改。
	let on_dirty_change = EventHandler::new(move |(tab_id, dirty): (String, bool)| {
		open_tabs.with_mut(|tabs| {
			if let Some(tab) = tabs.iter_mut().find(|tab| tab.id == tab_id) {
				tab.dirty = dirty;
			}
		});
	});

	// 点击标签页切换激活国策树。
	let on_select_tab = EventHandler::new(move |tab_id: String| {
		active_tab_id.set(Some(tab_id));
	});

	let on_edit_event = move |(missions_root, file_name): (String, String)| {
		let mut events_open = events_open;
		let mut events_root = events_root;
		let mut open_event_request = open_event_request;
		let mut focus_seq = focus_seq;
		events_open.set(true);
		events_root.set(Some(missions_root));
		// 请求携带自增序号：即使重复点击同一脚本也能再次触发打开与重读。
		let seq = focus_seq.read().wrapping_add(1);
		focus_seq.set(seq);
		open_event_request.set(Some((file_name, seq)));
	};

	let on_create_event_file =
		move |(missions_root, file_name, contents): (String, String, String)| {
			let Some(directory) = work_directory.read().clone() else {
				let mut load_error = load_error;
				load_error.set("创建事件脚本失败：请先打开工作区".to_string());
				return;
			};
			spawn(async move {
				let mut load_error = load_error;
				let mut workspace_files = workspace_files;
				match save_event_text(&directory, &missions_root, &file_name, &contents).await {
					Ok(()) => {
						let relative_path =
							format!("{missions_root}/missionsEvents/{file_name}");
						workspace_files.with_mut(|files| {
							if !files.iter().any(|file| file.relative_path == relative_path) {
								files.push(WorkspaceFile {
									name: file_name.clone(),
									relative_path,
									is_directory: false,
								});
							}
						});
					}
					Err(error) => load_error.set(format!("创建事件脚本失败：{error}")),
				}
			});
		};

	let on_delete_event_file = move |(missions_root, file_name): (String, String)| {
		let Some(directory) = work_directory.read().clone() else {
			return;
		};
		spawn(async move {
			let mut load_error = load_error;
			let mut workspace_files = workspace_files;
			match delete_event_text(&directory, &missions_root, &file_name).await {
				Ok(()) => {
					let relative_path = format!("{missions_root}/missionsEvents/{file_name}");
					workspace_files.with_mut(|files| {
						files.retain(|file| file.relative_path != relative_path);
					});
				}
				Err(error) => load_error.set(format!("删除事件脚本失败：{error}")),
			}
		});
	};

	let on_rename_event_file =
		move |(missions_root, old_name, new_name): (String, String, String)| {
			if old_name == new_name {
				return;
			}
			let Some(directory) = work_directory.read().clone() else {
				return;
			};
			spawn(async move {
				let mut load_error = load_error;
				let mut workspace_files = workspace_files;
				let mut rename_event_request = rename_event_request;
				match rename_event_text(&directory, &missions_root, &old_name, &new_name).await {
					Ok(()) => {
						workspace_files.with_mut(|files| {
							let old_relative_path =
								format!("{missions_root}/missionsEvents/{old_name}");
							for file in files.iter_mut() {
								if file.relative_path == old_relative_path {
									file.name = new_name.clone();
									file.relative_path =
										format!("{missions_root}/missionsEvents/{new_name}");
								}
							}
						});
						rename_event_request.set(Some((old_name, new_name)));
					}
					Err(error) => load_error.set(format!("重命名事件脚本失败：{error}")),
				}
			});
		};

	// 画布粘贴事件脚本（「粘贴国策+事件」/「粘贴事件」共用）：复制到目标树资源根下的新脚本
	//（文件名已由画布按计数后缀生成，如 事件脚本.txt → 事件脚本0.txt → 事件脚本1.txt）。
	let on_copy_event_script = EventHandler::new(
		move |(source_root, source_file, target_root, target_file): (String, String, String, String)| {
			let Some(directory) = work_directory.read().clone() else {
				return;
			};
			let mut load_error = load_error;
			let mut workspace_files = workspace_files;
			let source_path = format!("{source_root}/missionsEvents/{source_file}");
			let target_path = format!("{target_root}/missionsEvents/{target_file}");
			spawn(async move {
				// 目标条目先登记：连续粘贴的计数命名据此避开已占用的名字。
				let mut registered = false;
				workspace_files.with_mut(|files| {
					if !files.iter().any(|file| file.relative_path == target_path) {
						files.push(WorkspaceFile {
							name: target_file.clone(),
							relative_path: target_path.clone(),
							is_directory: false,
						});
						registered = true;
					}
				});
				if !workspace_files
					.read()
					.iter()
					.any(|file| file.relative_path == source_path)
				{
					load_error.set(format!("复制事件脚本失败：源脚本不存在（{source_file}）"));
					if registered {
						workspace_files.with_mut(|files| {
							files.retain(|file| file.relative_path != target_path);
						});
					}
					return;
				}
				let result = run_file_op(
					file_op_active,
					file_op_progress,
					copy_item(&directory, &source_path, &target_path),
				)
				.await;
				if let Err(error) = result {
					load_error.set(format!("复制事件脚本失败：{error}"));
					if registered {
						workspace_files.with_mut(|files| {
							files.retain(|file| file.relative_path != target_path);
						});
					}
				}
			});
		},
	);

	// 资源管理器「重命名」：同目录改名（复用移动通道，重名在提交前拦截），
	// 成功后同步标签页/事件面板路径并登记撤销（撤销 = 改回原名）。
	let run_rename = EventHandler::new(move |_: ()| {
		let Some((path, is_directory)) = rename_prompt.read().clone() else {
			return;
		};
		let new_name = rename_value.read().trim().to_string();
		if new_name.is_empty()
			|| new_name.contains('/')
			|| new_name.contains('\\')
			|| new_name == "."
			|| new_name == ".."
		{
			rename_error.set("名称无效：不能为空，也不能包含 / \\ 等字符".to_string());
			return;
		}
		let parent = path.rsplit_once('/').map(|(parent, _)| parent).unwrap_or("");
		let target = join_rel_path(parent, &new_name);
		if target == path {
			rename_prompt.set(None);
			return;
		}
		if workspace_files
			.read()
			.iter()
			.any(|file| file.relative_path == target)
		{
			rename_error.set(format!("已存在同名条目：{new_name}"));
			return;
		}
		let Some(directory) = work_directory.read().clone() else {
			return;
		};
		rename_prompt.set(None);
		rename_error.set(String::new());
		let mut load_error = load_error;
		let workspace_files = workspace_files;
		let selected_file = selected_file;
		let open_tabs = open_tabs;
		let events_root = events_root;
		let rename_event_request = rename_event_request;
		spawn(async move {
			match move_item(&directory, &path, &target).await {
				Ok(()) => {
					sync_after_rename(
						&directory,
						&path,
						&target,
						is_directory,
						workspace_files,
						selected_file,
						open_tabs,
						events_root,
						rename_event_request,
						prune_tabs,
					)
					.await;
					// 撤销/重做 = 反向/正向再执行一次改名。
					let directory_undo = directory.clone();
					let target_undo = target.clone();
					let original_undo = path.clone();
					let workspace_files_undo = workspace_files;
					let selected_file_undo = selected_file;
					let open_tabs_undo = open_tabs;
					let events_root_undo = events_root;
					let rename_request_undo = rename_event_request;
					let undo = EventHandler::new(move |_: ()| {
						let directory = directory_undo.clone();
						let target = target_undo.clone();
						let original = original_undo.clone();
						spawn(async move {
							match move_item(&directory, &target, &original).await {
								Ok(()) => {
									sync_after_rename(
										&directory,
										&target,
										&original,
										is_directory,
										workspace_files_undo,
										selected_file_undo,
										open_tabs_undo,
										events_root_undo,
										rename_request_undo,
										prune_tabs,
									)
									.await;
								}
								Err(error) => load_error.set(format!("撤销重命名失败：{error}")),
							}
						});
					});
					let directory_redo = directory.clone();
					let target_redo = target.clone();
					let original_redo = path.clone();
					let workspace_files_redo = workspace_files;
					let selected_file_redo = selected_file;
					let open_tabs_redo = open_tabs;
					let events_root_redo = events_root;
					let rename_request_redo = rename_event_request;
					let redo = EventHandler::new(move |_: ()| {
						let directory = directory_redo.clone();
						let target = target_redo.clone();
						let original = original_redo.clone();
						spawn(async move {
							match move_item(&directory, &original, &target).await {
								Ok(()) => {
									sync_after_rename(
										&directory,
										&original,
										&target,
										is_directory,
										workspace_files_redo,
										selected_file_redo,
										open_tabs_redo,
										events_root_redo,
										rename_request_redo,
										prune_tabs,
									)
									.await;
								}
								Err(error) => load_error.set(format!("重做重命名失败：{error}")),
							}
						});
					});
					on_undo_push.call((UndoScope::Explorer, undo, redo));
				}
				Err(error) => load_error.set(format!("重命名失败：{error}")),
			}
		});
	});

	// 事件面板选中文件变化 → 资源管理器定位并高亮。
	// use_callback 保持稳定身份：Work 重渲染时事件面板与资源管理器可记忆化跳过。
	let on_file_selected = use_callback(move |path: Option<String>| {
		let mut selected_file = selected_file;
		selected_file.set(path);
	});

	// 思维导图节点变化 → 记录当前使用的标题与图标（删除前占用检查用）。
	let on_nodes_change = move |(names, images): (Vec<String>, Vec<String>)| {
		let mut mind_node_names = mind_node_names;
		let mut mind_node_images = mind_node_images;
		if *mind_node_names.read() != names {
			mind_node_names.set(names);
		}
		if *mind_node_images.read() != images {
			mind_node_images.set(images);
		}
	};

	// 资源管理器双击：json → 打开国策树标签页；txt → 打开事件面板并聚焦卡片；
	// png → 手动载入当前国策树图标库（供创建卡片菜单选用）并聚焦使用该图标的卡片。
	// 路径兼容经典工作区与解包 APK 布局（assets/…/missions、assets/…/scenarios/…/missions）。
	let on_open_file = use_callback(move |relative_path: String| {
		// 国策树：资源根目录下的直接 .json 子文件。
		if missions_tree_file(&relative_path).is_some() {
			on_open_tree.call(relative_path.clone());
			return;
		}
		// 事件脚本：<资源根>/missionsEvents/*.txt。
		if let Some((missions_root, file_name)) = missions_subfile(&relative_path, "missionsEvents")
			.filter(|(_, name)| {
				!name.contains('/') && name.to_ascii_lowercase().ends_with(".txt")
			})
		{
			let mut events_open = events_open;
			let mut events_root = events_root;
			let mut open_event_request = open_event_request;
			let mut focus_seq = focus_seq;
			events_open.set(true);
			events_root.set(Some(missions_root));
			let seq = focus_seq.read().wrapping_add(1);
			focus_seq.set(seq);
			open_event_request.set(Some((file_name.clone(), seq)));

			let title = file_name.strip_suffix(".txt").unwrap_or(&file_name).to_string();
			let mut mind_focus_request = mind_focus_request;
			mind_focus_request.set(Some((FocusTarget::Title(title), seq)));
			return;
		}
		// 图标：<资源根>/missionsImages/…/*.png。
		if let Some((_, name)) = missions_subfile(&relative_path, "missionsImages")
			.filter(|(_, name)| name.to_ascii_lowercase().ends_with(".png"))
		{
			// 图标在 H/ 子目录下，取末段文件名与卡片 image_name 匹配。
			let file_name = name.rsplit('/').next().unwrap_or(&name).to_string();
			let mut focus_seq = focus_seq;
			let seq = focus_seq.read().wrapping_add(1);
			focus_seq.set(seq);
			let mut mind_focus_request = mind_focus_request;
			mind_focus_request.set(Some((FocusTarget::Image(file_name.clone()), seq)));

			// 手动加载图标：双击 .png 时把该图标载入当前激活国策树，供创建卡片菜单选用。
			// 图标选择器不再自动加载完整图标库，避免图标过多时页面卡死。
			let file_exists = workspace_files
				.read()
				.iter()
				.any(|file| file.relative_path == relative_path);
			if !file_exists {
				return;
			}
			let Some(tab_id) = active_tab_id.read().clone() else {
				return;
			};
			let Some(directory) = work_directory.read().clone() else {
				return;
			};
			let icon_name = file_name
				.rsplit_once('.')
				.map_or_else(|| file_name.clone(), |(stem, _)| stem.to_string());
			let mut pending_icon = pending_icon;
			let mut load_error = load_error;
			spawn(async move {
				match load_single_icon(&directory, &icon_name, &relative_path).await {
					Ok(Some(icon)) => pending_icon.set(Some((tab_id, icon))),
					Ok(None) => {}
					Err(error) => load_error.set(format!("载入图标失败：{error}")),
				}
			});
		}
	});

	// 资源管理器右键命令：剪切/复制/粘贴/删除/打开文件位置。
	// use_callback 保持稳定身份：Work 重渲染时资源管理器可记忆化跳过。
	let on_explorer_command = use_callback(move |(command, argument): (ExplorerCommand, String)| {
		match command {
			ExplorerCommand::Cut | ExplorerCommand::Copy => {
				let Some(entry) = workspace_files
					.read()
					.iter()
					.find(|file| file.relative_path == argument)
					.cloned()
				else {
					return;
				};
				let mut explorer_clipboard = explorer_clipboard;
				explorer_clipboard.set(Some(ExplorerClipboard {
					source_path: entry.relative_path,
					source_name: entry.name,
					is_directory: entry.is_directory,
					is_cut: command == ExplorerCommand::Cut,
				}));
			}
			ExplorerCommand::Paste => {
				let Some(directory) = work_directory.read().clone() else {
					return;
				};
				let Some(clipboard) = explorer_clipboard.read().clone() else {
					return;
				};
				let target_dir = argument;
				let source_path = clipboard.source_path.clone();
				let source_name = clipboard.source_name.clone();
				let is_cut = clipboard.is_cut;
				let is_directory = clipboard.is_directory;
				spawn(async move {
					let mut load_error = load_error;
					let mut workspace_files = workspace_files;
					let mut explorer_clipboard = explorer_clipboard;
					let mut selected_file = selected_file;
					// 目标名冲突时生成“xx 副本.txt”形式的唯一名。
					let (stem, ext) = source_name
						.rsplit_once('.')
						.map(|(s, e)| (s.to_string(), format!(".{e}")))
						.unwrap_or((source_name.clone(), String::new()));
					let exists = |name: &str| {
						workspace_files
							.read()
							.iter()
							.any(|file| file.relative_path == join_rel_path(&target_dir, name))
					};
					let mut target_name = source_name.clone();
					let mut counter = 1;
					while exists(&target_name) {
						counter += 1;
						target_name = if counter == 2 {
							format!("{stem} 副本{ext}")
						} else {
							format!("{stem} 副本 {counter}{ext}")
						};
					}
					let target_path = join_rel_path(&target_dir, &target_name);
					let result = run_file_op(file_op_active, file_op_progress, async {
						if is_cut {
							if source_path == target_path {
								Ok(())
							} else {
								move_item(&directory, &source_path, &target_path).await
							}
						} else {
							copy_item(&directory, &source_path, &target_path).await
						}
					})
					.await;
					match result {
						Ok(()) => {
							if is_cut {
								explorer_clipboard.set(None);
								if selected_file.read().clone() == Some(source_path.clone()) {
									selected_file.set(Some(target_path.clone()));
								}
							}
							// 登记到资源管理器分区的撤销栈：剪切粘贴=移回原处；复制粘贴=删除/重新复制。
							if target_path != source_path {
								let undo = EventHandler::new({
									let directory = directory.clone();
									let source = source_path.clone();
									let target = target_path.clone();
									let mut workspace_files = workspace_files;
									let mut load_error = load_error;
									move |_: ()| {
										let directory = directory.clone();
										let source = source.clone();
										let target = target.clone();
										spawn(async move {
											let result = run_file_op(
												file_op_active,
												file_op_progress,
												async {
													if is_cut {
														move_item(&directory, &target, &source).await
													} else {
														delete_item(&directory, &target, is_directory).await
													}
												},
											)
											.await;
											match result {
												Ok(()) => {
													if let Ok(files) = reload_file_list(&directory).await {
														workspace_files.set(files.clone());
														prune_tabs.call(files);
													}
												}
												Err(error) => {
													load_error.set(format!("撤销粘贴失败：{error}"))
												}
											}
										});
									}
								});
								let redo = EventHandler::new({
									let directory = directory.clone();
									let source = source_path.clone();
									let target = target_path.clone();
									let mut workspace_files = workspace_files;
									let mut load_error = load_error;
									move |_: ()| {
										let directory = directory.clone();
										let source = source.clone();
										let target = target.clone();
										spawn(async move {
											let result = run_file_op(
												file_op_active,
												file_op_progress,
												async {
													if is_cut {
														move_item(&directory, &source, &target).await
													} else {
														copy_item(&directory, &source, &target).await
													}
												},
											)
											.await;
											match result {
												Ok(()) => {
													if let Ok(files) = reload_file_list(&directory).await {
														workspace_files.set(files.clone());
														prune_tabs.call(files);
													}
												}
												Err(error) => {
													load_error.set(format!("重做粘贴失败：{error}"))
												}
											}
										});
									}
								});
								on_undo_push.call((UndoScope::Explorer, undo, redo));
							}
							if let Ok(files) = reload_file_list(&directory).await {
								workspace_files.set(files.clone());
								prune_tabs.call(files);
							}
						}
						Err(error) => load_error.set(format!("粘贴失败：{error}")),
					}
				});
			}
			ExplorerCommand::Delete => {
				let name = argument.rsplit('/').next().unwrap_or(&argument).to_string();
				let stem = name.strip_suffix(".txt").unwrap_or(&name).to_string();
				let is_events = missions_subfile(&argument, "missionsEvents").is_some();
				let is_image = missions_subfile(&argument, "missionsImages").is_some();

				// 占用检查：画布中正在使用的脚本/图标删除前给出警告。
				let mut warning = String::new();
				let is_open_tree = argument.to_ascii_lowercase().ends_with(".json")
					&& open_tabs.read().iter().any(|tab| tab.json_path == argument);
				let is_canvas_config = missions_tree_file(&argument)
					.is_some_and(|(_, name)| name == "Missions.json");
				if is_canvas_config {
					warning =
						"Missions.json 是国策树画布配置文件，删除后画布将无法加载。".to_string();
				} else if is_open_tree {
					warning = "该文件已作为国策树在标签页中打开，删除后对应标签页将关闭。".to_string();
				} else if is_events && mind_node_names.read().contains(&stem) {
					warning = format!(
						"该脚本正被国策「{stem}」使用，删除后对应国策将没有事件脚本。"
					);
				} else if is_image && mind_node_images.read().contains(&name) {
					warning = format!("该图标正被国策使用，删除后对应国策将无法显示图标。");
				}
				let message = if warning.is_empty() {
					format!("确定删除 {name} 吗？此操作不可恢复。")
				} else {
					format!("{warning}\n\n确定仍然删除 {name} 吗？此操作不可恢复。")
				};
				let mut delete_prompt = delete_prompt;
				delete_prompt.set(Some((argument, message)));
			}
			ExplorerCommand::Rename => {
				let Some(entry) = workspace_files
					.read()
					.iter()
					.find(|file| file.relative_path == argument)
					.cloned()
				else {
					return;
				};
				let mut rename_prompt = rename_prompt;
				let mut rename_value = rename_value;
				let mut rename_error = rename_error;
				rename_prompt.set(Some((entry.relative_path, entry.is_directory)));
				rename_value.set(entry.name);
				rename_error.set(String::new());
			}
			ExplorerCommand::Reveal => {
				let Some(directory) = work_directory.read().clone() else {
					return;
				};
				let is_directory = workspace_files
					.read()
					.iter()
					.any(|file| file.relative_path == argument && file.is_directory);
				spawn(async move {
					let mut load_error = load_error;
					if let Err(error) = reveal_item(&directory, &argument, is_directory).await {
						load_error.set(format!("打开文件位置失败：{error}"));
					}
				});
			}
			ExplorerCommand::Sign => {
				perform_sign_apk.call(argument);
			}
		}
	});

	// 标题栏撤销按钮状态：只反映当前焦点分区（事件分区再按“当前打开文件”细化可用性）。
	let undo_zone = *active_zone.read();
	let undo_zone_label = undo_zone.label().to_string();
	let undo_depths_snapshot = *undo_depths.read();
	let event_scope_snapshot = event_active_scope.read().clone();
	let (can_undo, can_redo) = match undo_zone {
		UndoZone::Canvas => (
			undo_depths_snapshot.canvas_undo > 0,
			undo_depths_snapshot.canvas_redo > 0,
		),
		UndoZone::Explorer => (
			undo_depths_snapshot.explorer_undo > 0,
			undo_depths_snapshot.explorer_redo > 0,
		),
		UndoZone::Events => {
			let stacks = undo_stacks.borrow();
			let applies = |entry: &UndoEntry| match &entry.scope {
				UndoScope::EventFile(file) => {
					event_scope_snapshot.as_deref() == Some(file.as_str())
				}
				_ => true,
			};
			(
				stacks.events_undo.iter().any(&applies),
				stacks.events_redo.iter().any(&applies),
			)
		}
	};
	let has_workspace = work_directory.read().is_some();
	let current_directory = work_directory.read().clone();
	let workspace_name = current_directory
		.as_ref()
		.map(|directory| {
			let path = &directory.display_path;
			path.trim_end_matches(|character| character == '\\' || character == '/')
				.rsplit(|character| character == '\\' || character == '/')
				.next()
				.unwrap_or_default()
		})
		.filter(|name| !name.is_empty())
		.unwrap_or("AgeCivModTool")
		.to_string();
	let progress_snapshot = load_progress.read().clone();
	let (loading_stage, loading_percent, default_counter) = match progress_snapshot {
		Some(progress) if progress.total > 0 => {
			let percent = (progress.completed as f64 / progress.total as f64 * 100.0)
				.clamp(0.0, 100.0);
			(
				progress.stage,
				Some(percent),
				Some(format!("{} / {}", progress.completed, progress.total)),
			)
		}
		Some(progress) => (progress.stage, None, None),
		None => ("正在准备...".to_string(), None, None),
	};
	let loading_counter = progress_counter.read().clone().or(default_counter);
	// 资源管理器文件操作进度卡快照（复制/移动/删除大文件夹时显示）。
	let file_op_view = file_op_progress.read().clone();
	let file_op_percent = file_op_view
		.as_ref()
		.map(|view| {
			if view.total > 0 {
				(view.completed as f64 / view.total as f64 * 100.0).clamp(0.0, 100.0)
			} else {
				0.0
			}
		})
		.unwrap_or(0.0);
	let tab_close_prompt_data = tab_close_prompt.read().clone().map(|tab_id| {
		let title = open_tabs
			.read()
			.iter()
			.find(|tab| tab.id == tab_id)
			.map(|tab| tab.title.clone())
			.unwrap_or_default();
		(tab_id, title)
	});
	// 工作区现有事件脚本（相对路径）：传给画布判断「空国策」并提供 新建事件 / 链接事件。
	// Shared 句柄按指针比较：事件脚本列表未变化时，画布不会因此 prop 重渲染。
	let event_files = use_memo(move || {
		Shared::new(
			workspace_files
				.read()
				.iter()
				.filter(|file| missions_subfile(&file.relative_path, "missionsEvents").is_some())
				.map(|file| file.relative_path.clone())
				.collect::<Vec<String>>(),
		)
	});
	let event_files_snapshot = (*event_files.read()).clone();
	let canvas_tabs: Vec<(String, TreeTab)> = open_tabs
		.read()
		.iter()
		.cloned()
		.map(|tab| (tab.id.clone(), tab))
		.collect();
	rsx! {
        div {
            class: if resizing.read().is_some() { "editor-shell resizing" } else { "editor-shell" },
            tabindex: "0",
            autofocus: true,
            onpointermove: move |evt: Event<PointerData>| {
                let Some(state) = *resizing.read() else {
                    return;
                };
                let delta = evt.client_coordinates().x - state.start_x;
                match state.side {
                    ResizeSide::Explorer => {
                        explorer_width
                            .set(
                                (state.start_width + delta)
                                    .clamp(EXPLORER_MIN_WIDTH, EXPLORER_MAX_WIDTH),
                            );
                    }
                    ResizeSide::Events => {
                        events_width
                            .set(
                                (state.start_width - delta)
                                    .clamp(EVENTS_MIN_WIDTH, EVENTS_MAX_WIDTH),
                            );
                    }
                }
            },
            onpointerup: move |_| resizing.set(None),
            onpointercancel: move |_| resizing.set(None),
            onkeydown: move |evt: Event<KeyboardData>| {
                let data = evt.data();
                let key = data.key().to_string();
                let modifiers = data.modifiers();
                if modifiers.contains(Modifiers::CONTROL) || modifiers.contains(Modifiers::META)
                {
                    match key.to_ascii_lowercase().as_str() {
                        "s" => {
                            evt.prevent_default();
                            save_request.with_mut(|request| *request = request.wrapping_add(1));
                        }
                        "o" => {
                            evt.prevent_default();
                            if !*loading.read() {
                                on_choose_directory.call(());
                            }
                        }
                        "n" => {
                            evt.prevent_default();
                            if !*loading.read() {
                                on_create_directory.call(());
                            }
                        }
                        "w" => {
                            evt.prevent_default();
                            let active_tab = active_tab_id.read().clone();
                            if let Some(tab_id) = active_tab {
                                on_close_tab_requested.call(tab_id);
                            }
                        }
                        "z" => {
                            // 标记了 data-native-undo 的输入框（搜索框等）交回浏览器原生撤销。
                            if !focus_wants_native_undo() {
                                evt.prevent_default();
                                if modifiers.contains(Modifiers::SHIFT) {
                                    on_redo.call(());
                                } else {
                                    on_undo.call(());
                                }
                            }
                        }
                        "y" => {
                            if !focus_wants_native_undo() {
                                evt.prevent_default();
                                on_redo.call(());
                            }
                        }
                        _ => {}
                    }
                }
            },
            Frame {
                on_open: move |_| on_choose_directory.call(()),
                on_new: move |_| on_create_directory.call(()),
                on_manage_all_files,
                on_extract_apk,
                on_import_apk,
                on_export_apk,
                on_set_lookup_apk,
                on_package_apk,
                on_sign_apk,
                on_import_signing_key,
                on_save: move |_| save_request.with_mut(|request| *request = request.wrapping_add(1)),
                on_toggle_files: move |_| {
                    let next = !*files_open.read();
                    files_open.set(next);
                },
                on_toggle_events: move |_| {
                    let next = !*events_open.read();
                    events_open.set(next);
                },
                has_workspace,
                workspace_name,
                is_loading: *loading.read(),
                is_android: *android_platform.read(),
                all_files_access_granted: *all_files_access_granted.read(),
                tabs: open_tabs
                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        .read()
                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        .iter()
                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        .map(|tab| (tab.id.clone(), tab.title.clone()))
                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        .collect(),
                active_tab_id: active_tab_id.read().clone(),
                on_select_tab,
                on_close_tab: on_close_tab_requested,
                can_undo,
                can_redo,
                zone_label: undo_zone_label,
                on_undo,
                on_redo,
            }
            div { class: "editor-body",
                if *files_open.read() {
                    div {
                        class: "drawer explorer-drawer",
                        onfocusin: move |_| {
                            if *active_zone.peek() != UndoZone::Explorer {
                                active_zone.set(UndoZone::Explorer);
                            }
                        },
                        onpointerdown: move |_| {
                            if *active_zone.peek() != UndoZone::Explorer {
                                active_zone.set(UndoZone::Explorer);
                            }
                        },
                        Files {
                            files: workspace_files,
                            work_directory: current_directory.as_ref().map(|directory| directory.display_path.clone()),
                            width: *explorer_width.read(),
                            selected_path: selected_file,
                            clipboard: explorer_clipboard,
                            on_open_file,
                            on_command: on_explorer_command,
                        }
                        div {
                            class: "resize-handle",
                            role: "separator",
                            aria_orientation: "vertical",
                            aria_hidden: "true",
                            id: "resize-explorer",
                            onpointerdown: move |evt: Event<PointerData>| {
                                resizing
                                    .set(
                                        Some(PanelResize {
                                            side: ResizeSide::Explorer,
                                            start_x: evt.client_coordinates().x,
                                            start_width: *explorer_width.read(),
                                        }),
                                    );
                                let pointer_id = evt.data().pointer_id();
                                spawn(async move {
                                    let _ = dioxus::document::eval(
                                            &format!(
                                                "document.getElementById('resize-explorer')?.setPointerCapture({pointer_id})",
                                            ),
                                        )
                                        .await;
                                });
                            },
                        }
                    }
                }
                main { class: "editor-main",
                    if *loading.read() {
                        div { class: "loading-panel", role: "status",
                            div { class: "loading-stage", "{loading_stage}" }
                            if let Some(percent) = loading_percent {
                                div {
                                    class: "loading-progress",
                                    role: "progressbar",
                                    aria_valuemin: "0",
                                    aria_valuemax: "100",
                                    aria_valuenow: "{percent:.0}",
                                    div {
                                        class: "loading-progress-fill",
                                        style: "width: {percent}%;",
                                    }
                                }
                            } else {
                                div { class: "loading-progress indeterminate",
                                    div { class: "loading-progress-fill" }
                                }
                            }
                            if let Some(counter) = loading_counter {
                                div { class: "loading-progress-text", "{counter}" }
                            }
                        }
                    } else if !open_tabs.read().is_empty() {
                        div {
                            class: "canvas-stage",
                            onfocusin: move |_| {
                                if *active_zone.peek() != UndoZone::Canvas {
                                    active_zone.set(UndoZone::Canvas);
                                }
                            },
                            onpointerdown: move |_| {
                                if *active_zone.peek() != UndoZone::Canvas {
                                    active_zone.set(UndoZone::Canvas);
                                }
                            },
                            for (tab_id , tab) in canvas_tabs {
                                TreeCanvas {
                                    key: "{tab_id}",
                                    tab,
                                    event_files: event_files_snapshot.clone(),
                                    active_tab_id,
                                    save_request,
                                    save_status: save_status.read().clone(),
                                    focus_request: mind_focus_request,
                                    pending_icon,
                                    mind_clipboard,
                                    on_copy_event_script,
                                    on_undo_push,
                                    on_save: on_save_tab,
                                    on_edit_event,
                                    on_create_event_file,
                                    on_delete_event_file,
                                    on_rename_event_file,
                                    on_nodes_change,
                                    on_dirty_change,
                                }
                            }
                        }
                    } else {
                        div { class: "welcome-panel",
                            div { class: "welcome-mark", "A" }
                            h1 { "国策树工作区" }
                            p {
                                if work_directory.read().is_some() {
                                    "在资源管理器中双击国策资源目录（missions / assets/game/missions / 剧本 missions）下的 .json 文件打开国策树。"
                                } else {
                                    "从文件菜单打开现有工作区，或创建一个新的工作区。"
                                }
                            }
                            label { class: "workspace-name",
                                span { "新工作区名称" }
                                input {
                                    value: "{directory_name}",
                                    "data-native-undo": "true",
                                    oninput: move |event: FormEvent| directory_name.set(event.value()),
                                }
                            }
                            if !load_error.read().is_empty() {
                                p { class: "workspace-error", role: "alert", "{load_error.read()}" }
                            }
                        }
                    }
                }
                if *events_open.read() {
                    div {
                        class: "drawer events-drawer",
                        onfocusin: move |_| {
                            if *active_zone.peek() != UndoZone::Events {
                                active_zone.set(UndoZone::Events);
                            }
                        },
                        onpointerdown: move |_| {
                            if *active_zone.peek() != UndoZone::Events {
                                active_zone.set(UndoZone::Events);
                            }
                        },
                        div {
                            class: "resize-handle",
                            role: "separator",
                            aria_orientation: "vertical",
                            aria_hidden: "true",
                            id: "resize-events",
                            onpointerdown: move |evt: Event<PointerData>| {
                                resizing
                                    .set(
                                        Some(PanelResize {
                                            side: ResizeSide::Events,
                                            start_x: evt.client_coordinates().x,
                                            start_width: *events_width.read(),
                                        }),
                                    );
                                let pointer_id = evt.data().pointer_id();
                                spawn(async move {
                                    let _ = dioxus::document::eval(
                                            &format!(
                                                "document.getElementById('resize-events')?.setPointerCapture({pointer_id})",
                                            ),
                                        )
                                        .await;
                                });
                            },
                        }
                        EventPanel {
                            work_directory: current_directory.clone(),
                            missions_root: events_root,
                            save_request,
                            open_request: open_event_request,
                            rename_request: rename_event_request,
                            release_request: event_release_request,
                            on_selection_change: on_file_selected,
                            on_undo_push,
                            active_scope: event_active_scope,
                            lookup_epoch,
                            // 安卓端禁用原生 datalist（弹层定位不可靠）：改页面内自绘下拉。
                            native_autocomplete: !*android_platform.read(),
                            width: *events_width.read(),
                        }
                    }
                }
            }
            footer { class: "editor-statusbar",
                span {
                    if has_workspace {
                        "工作区已载入"
                    } else {
                        "未打开工作区"
                    }
                }
                if let Some(tab_id) = active_tab_id.read().clone() {
                    if let Some(tab) = open_tabs.read().iter().find(|tab| tab.id == tab_id) {
                        span { class: "status-path", "国策树：{tab.title}" }
                    }
                }
                if let Some(directory) = current_directory {
                    span {
                        class: "status-path",
                        title: "{directory.display_path}",
                        "{directory.display_path}"
                    }
                }
                if !apk_status.read().is_empty() {
                    span {
                        class: "status-path",
                        role: "status",
                        title: "{apk_status.read()}",
                        "{apk_status.read()}"
                    }
                }
                if !permission_error.read().is_empty() {
                    span {
                        class: "status-error",
                        role: "alert",
                        title: "{permission_error.read()}",
                        "{permission_error.read()}"
                    }
                }
                if !load_error.read().is_empty() {
                    span {
                        class: "status-error",
                        role: "alert",
                        title: "{load_error.read()}",
                        "{load_error.read()}"
                    }
                }
            }
            if let Some(view) = file_op_view {
                div { class: "file-op-panel", role: "status",
                    div { class: "loading-stage", "{view.stage}" }
                    if view.total > 0 {
                        div {
                            class: "loading-progress",
                            role: "progressbar",
                            aria_valuemin: "0",
                            aria_valuemax: "{view.total}",
                            aria_valuenow: "{view.completed}",
                            div {
                                class: "loading-progress-fill",
                                style: "width: {file_op_percent:.1}%;",
                            }
                        }
                        div { class: "loading-progress-text", "{view.completed} / {view.total}" }
                    } else {
                        div { class: "loading-progress indeterminate",
                            div { class: "loading-progress-fill" }
                        }
                        div { class: "loading-progress-text", "已处理 {view.completed} 项" }
                    }
                }
            }
            if let Some((path, message)) = delete_prompt.read().clone() {
                div {
                    class: "menu-dismiss",
                    style: "z-index: 55;",
                    aria_hidden: "true",
                    onclick: move |_| delete_prompt.set(None),
                }
                div { class: "confirm-dialog", role: "alertdialog",
                    p { class: "confirm-message", "{message}" }
                    div { class: "confirm-actions",
                        button {
                            class: "event-save",
                            r#type: "button",
                            onclick: move |_| {
                                let directory = work_directory.read().clone();
                                let is_directory = workspace_files
                                    .read()
                                    .iter()
                                    .find(|file| file.relative_path == path)
                                    .map(|file| file.is_directory)
                                    .unwrap_or(false);
                                delete_prompt.set(None);
                                if let Some(directory) = directory {
                                    let path = path.clone();
                                    spawn(async move {
                                        let mut load_error = load_error;
                                        let mut workspace_files = workspace_files;
                                        let result = run_file_op(
                                                file_op_active,
                                                file_op_progress,
                                                delete_item(&directory, &path, is_directory),
                                            )
                                            .await;
                                        match result {
                                            Ok(()) => {
                                                if let Ok(files) = reload_file_list(&directory).await {
                                                    workspace_files.set(files.clone());
                                                    prune_tabs.call(files);
                                                }
                                            }
                                            Err(error) => load_error.set(format!("删除失败：{error}")),
                                        }
                                    });
                                }
                            },
                            "确认删除"
                        }
                        button {
                            class: "event-revert",
                            r#type: "button",
                            onclick: move |_| delete_prompt.set(None),
                            "取消"
                        }
                    }
                }
            }
            if let Some((path, _is_directory)) = rename_prompt.read().clone() {
                {
                    let current_name = path.rsplit('/').next().unwrap_or(&path).to_string();
                    rsx! {
                        div {
                            class: "menu-dismiss",
                            style: "z-index: 55;",
                            aria_hidden: "true",
                            onclick: move |_| {
                                rename_prompt.set(None);
                                rename_error.set(String::new());
                            },
                        }
                        div { class: "confirm-dialog", role: "alertdialog",
                            p { class: "confirm-message", "重命名「{current_name}」为：" }
                            input {
                                r#type: "text",
                                value: "{rename_value}",
                                spellcheck: "false",
                                autocomplete: "off",
                                "data-native-undo": "true",
                                aria_label: "新名称",
                                style: "width: 100%; box-sizing: border-box; padding: 6px 10px; margin: 0 0 10px; background: var(--input-bg, #26262e); color: inherit; border: 1px solid #4a4a55; border-radius: 4px;",
                                oninput: move |evt: FormEvent| rename_value.set(evt.value()),
                                onkeydown: move |evt: Event<KeyboardData>| {
                                    if evt.data().key().to_string() == "Enter" {
                                        evt.prevent_default();
                                        run_rename.call(());
                                    }
                                },
                            }
                            if !rename_error.read().is_empty() {
                                p { class: "confirm-message", style: "color: #ff8a8a;", "{rename_error}" }
                            }
                            div { class: "confirm-actions",
                                button {
                                    class: "event-save",
                                    r#type: "button",
                                    onclick: move |_| run_rename.call(()),
                                    "确定"
                                }
                                button {
                                    class: "event-revert",
                                    r#type: "button",
                                    onclick: move |_| {
                                        rename_prompt.set(None);
                                        rename_error.set(String::new());
                                    },
                                    "取消"
                                }
                            }
                        }
                    }
                }
            }
            if let Some((unused_scripts, pending_operation)) = unused_scripts_prompt.read().clone() {
                {
                    let unused_count = unused_scripts.len();
                    let file_op_busy = *file_op_active.read() > 0;
                    rsx! {
                        div { class: "menu-dismiss", style: "z-index: 55;", aria_hidden: "true" }
                        div { class: "confirm-dialog", role: "alertdialog",
                            p { class: "confirm-message",
                                "检测到 {unused_count} 个未被任何国策引用的脚本副本（missionsEvents，按创建/修改时间从新到旧排列）："
                            }
                            UnusedScriptsList { scripts: unused_scripts, checked: unused_scripts_checked }
                            p { class: "confirm-message", style: "font-size: 12px; opacity: 0.75;",
                                "「删除」只删除选中项（未选择任何项时 = 全部删除）；未删完时停留本界面，全部删完后自动继续，或点「继续」直接继续打包/导出；「取消」取消本次操作。"
                            }
                            div { class: "confirm-actions",
                                button {
                                    class: "event-delete",
                                    r#type: "button",
                                    disabled: file_op_busy,
                                    onclick: move |_| {
                                        let Some(directory) = work_directory.read().clone() else {
                                            return;
                                        };
                                        // 点击时取最新列表与待继续操作（停留期间列表会随删除更新）。
                                        let Some((all_scripts, operation)) =
                                            unused_scripts_prompt.read().clone()
                                        else {
                                            return;
                                        };
                                        let checked = unused_scripts_checked.read().clone();
                                        // 未选择任何项时 = 全部删除。
                                        let scripts = if checked.is_empty() { all_scripts.clone() } else { checked };
                                        spawn(async move {
                                            let mut unused_scripts_prompt = unused_scripts_prompt;
                                            let mut unused_scripts_checked = unused_scripts_checked;
                                            let mut apk_status = apk_status;
                                            let mut workspace_files = workspace_files;
                                            // 批量删除（包一层进度计数；单个失败不中断）；记录失败项用于计算剩余列表。
                                            let failed_scripts = run_file_op(
                                                    file_op_active,
                                                    file_op_progress,
                                                    async {
                                                        let mut failed_scripts = Vec::<String>::new();
                                                        for script in &scripts {
                                                            if delete_item(&directory, script, false)
                                                                .await
                                                                .is_err()
                                                            {
                                                                failed_scripts.push(script.clone());
                                                            }
                                                        }
                                                        failed_scripts
                                                    },
                                                )
                                                .await;
                                            if let Ok(files) = reload_file_list(&directory).await {
                                                workspace_files.set(files);
                                            }
                                            // 剩余 = 原列表 − 成功删除项（失败项保留在列表内）。
                                            let remaining: Vec<String> = all_scripts
                                                .iter()
                                                .filter(|script| {
                                                    !scripts.contains(script)
                                                        || failed_scripts.contains(script)
                                                })
                                                .cloned()
                                                .collect();
                                            unused_scripts_checked.set(Vec::new());
                                            if remaining.is_empty() {
                                                // 全部删除完成：关闭对话框并继续打包/导出。
                                                unused_scripts_prompt.set(None);
                                                perform_apk_operation(
                                                        &directory,
                                                        operation,
                                                        loading,
                                                        load_progress,
                                                        progress_counter,
                                                        apk_status,
                                                        load_error,
                                                        workspace_files,
                                                    )
                                                    .await;
                                                if !failed_scripts.is_empty() {
                                                    let failed = failed_scripts.len();
                                                    apk_status
                                                        .with_mut(|status| {
                                                            *status = format!(
                                                                "{status}（有 {failed} 个脚本副本删除失败）",
                                                            );
                                                        });
                                                }
                                            } else {
                                                // 未删完：停留在对话框等待用户处理；全部删完或点「继续」才执行打包/导出。
                                                let deleted = all_scripts.len() - remaining.len();
                                                let note = if failed_scripts.is_empty() {
                                                    format!(
                                                        "已删除 {deleted} 个脚本副本，还有 {} 个未删除；可继续选择删除，或点「继续」直接打包/导出",
                                                        remaining.len(),
                                                    )
                                                } else {
                                                    format!(
                                                        "已删除 {deleted} 个脚本副本（有 {} 个删除失败），还有 {} 个未删除；可继续选择删除，或点「继续」直接打包/导出",
                                                        failed_scripts.len(),
                                                        remaining.len(),
                                                    )
                                                };
                                                apk_status.set(note);
                                                unused_scripts_prompt.set(Some((remaining, operation)));
                                            }
                                        });
                                    },
                                    "删除"
                                }
                                button {
                                    class: "event-save",
                                    r#type: "button",
                                    disabled: file_op_busy,
                                    onclick: move |_| {
                                        let mut unused_scripts_prompt = unused_scripts_prompt;
                                        unused_scripts_prompt.set(None);
                                        let Some(directory) = work_directory.read().clone() else {
                                            return;
                                        };
                                        let operation = pending_operation.clone();
                                        spawn(async move {
                                            perform_apk_operation(
                                                    &directory,
                                                    operation,
                                                    loading,
                                                    load_progress,
                                                    progress_counter,
                                                    apk_status,
                                                    load_error,
                                                    workspace_files,
                                                )
                                                .await;
                                        });
                                    },
                                    "继续"
                                }
                                button {
                                    class: "event-revert",
                                    r#type: "button",
                                    disabled: file_op_busy,
                                    onclick: move |_| {
                                        let mut unused_scripts_prompt = unused_scripts_prompt;
                                        unused_scripts_prompt.set(None);
                                    },
                                    "取消"
                                }
                            }
                        }
                    }
                }
            }
            if let Some(key_path) = signing_key_prompt.read().clone() {
                div {
                    class: "menu-dismiss",
                    style: "z-index: 55;",
                    aria_hidden: "true",
                    onclick: move |_| {
                        signing_key_prompt.set(None);
                        signing_key_error.set(String::new());
                    },
                }
                div { class: "confirm-dialog", role: "alertdialog",
                    p { class: "confirm-message", "请输入 BKS 密钥库密码：" }
                    p {
                        class: "confirm-message",
                        style: "font-size: 12px; opacity: 0.75; word-break: break-all;",
                        "{key_path}"
                    }
                    input {
                        r#type: "password",
                        value: "{signing_key_password}",
                        placeholder: "密钥库密码",
                        "data-native-undo": "true",
                        style: "width: 100%; box-sizing: border-box; padding: 6px 10px; margin: 8px 0; background: var(--input-bg, #26262e); color: inherit; border: 1px solid #4a4a55; border-radius: 4px;",
                        oninput: move |evt: FormEvent| signing_key_password.set(evt.value()),
                        onkeydown: move |evt: Event<KeyboardData>| {
                            if evt.data().key().to_string() == "Enter" {
                                evt.prevent_default();
                                if let Some(key_path) = signing_key_prompt.read().clone() {
                                    run_import_key
                                        .call((key_path, Some(signing_key_password.read().clone())));
                                }
                            }
                        },
                    }
                    if !signing_key_error.read().is_empty() {
                        p {
                            class: "confirm-message",
                            style: "color: #ff8a8a;",
                            "{signing_key_error}"
                        }
                    }
                    div { class: "confirm-actions",
                        button {
                            class: "event-save",
                            r#type: "button",
                            onclick: move |_| {
                                if let Some(key_path) = signing_key_prompt.read().clone() {
                                    run_import_key.call((key_path, Some(signing_key_password.read().clone())));
                                }
                            },
                            "确定"
                        }
                        button {
                            class: "event-revert",
                            r#type: "button",
                            onclick: move |_| {
                                signing_key_prompt.set(None);
                                signing_key_error.set(String::new());
                            },
                            "取消"
                        }
                    }
                }
            }
            if let Some(directories) = pack_choice.read().clone() {
                div {
                    class: "menu-dismiss",
                    style: "z-index: 55;",
                    aria_hidden: "true",
                    onclick: move |_| pack_choice.set(None),
                }
                div { class: "confirm-dialog", role: "alertdialog",
                    p { class: "confirm-message",
                        "检测到多个 APK 内容目录，请选择要打包的目录："
                    }
                    div {
                        class: "confirm-actions",
                        style: "flex-wrap: wrap; justify-content: flex-start;",
                        for directory in directories {
                            {
                                let choice = directory.clone();
                                rsx! {
                                    button {
                                        class: "event-save",
                                        r#type: "button",
                                        onclick: move |_| {
                                            pack_choice.set(None);
                                            run_package_apk.call(Some(choice.clone()));
                                        },
                                        "{directory}"
                                    }
                                }
                            }
                        }
                        button {
                            class: "event-revert",
                            r#type: "button",
                            onclick: move |_| pack_choice.set(None),
                            "取消"
                        }
                    }
                }
            }
            if let Some((tab_id, tab_title)) = tab_close_prompt_data {
                div {
                    class: "menu-dismiss",
                    style: "z-index: 55;",
                    aria_hidden: "true",
                    onclick: move |_| tab_close_prompt.set(None),
                }
                div { class: "confirm-dialog", role: "alertdialog",
                    p { class: "confirm-message",
                        "国策树「{tab_title}」有未保存的修改，关闭标签页将丢失这些修改。"
                    }
                    div { class: "confirm-actions",
                        button {
                            class: "event-save",
                            r#type: "button",
                            onclick: move |_| {
                                tab_close_prompt.set(None);
                                on_close_tab.call(tab_id.clone());
                            },
                            "关闭标签页"
                        }
                        button {
                            class: "event-revert",
                            r#type: "button",
                            onclick: move |_| tab_close_prompt.set(None),
                            "取消"
                        }
                    }
                }
            }
        }
    }
}
