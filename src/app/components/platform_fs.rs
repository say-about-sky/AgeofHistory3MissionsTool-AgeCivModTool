//! 平台分派文件操作（桌面真实路径 / 安卓 SAF）统一入口与集中实现。
//!
//! 本项目多数「文件读写」在两种运行模式下各有一套实现：
//! - **桌面 / 安卓全盘访问模式**：`root_path` 为真实目录，命令直接以
//!   `work_directory` + 相对路径操作（如 `load_mission_event`、`save_missions_file`）；
//! - **安卓 SAF 模式**：只有 `folder_id`（document tree URI），一切文件操作经
//!   `android_fs_bridge` 桥接命令完成（如 `read_scoped_text_in_dir`、`write_scoped_text_file`），
//!   需要先把相对路径拼成完整路径、写文件要带 `recursive`。
//!
//! 调用方不应再各写 `if let Some(folder_id) = &directory.folder_id { ... } else { ... }`
//! 的分派（漏改一处就会出现两模式行为分叉）——**所有跨平台文件操作集中到本模块**：
//! - 「事件脚本」组：读取 / 保存 / 删除 / 重命名 / 热导入 / 按 id 解析；
//! - 「SAF 原语」组：目录列举、批量读图、文本读写、建目录、路径拼接；
//! - 「国策 / 决议」组：树配置读写、图标读取、决议配置读写。
//!
//! 维护约定：
//! 1. 新增跨平台文件操作时在本模块追加函数（按上面分组），不要回到调用方分派；
//! 2. 每组内「桌面分支」与「SAF 分支」写在同一函数里、相邻排列，注释标注两边命令名；
//! 3. 仅支持真实路径的功能（导入 / 打包 / 签名等 APK 操作）不做分派，
//!    在调用方用 `folder_id.is_some()` 守卫拒绝并提示，不属于本模块范畴。

use dioxus::prelude::{Signal, WritableExt};
use serde::{Deserialize, Serialize};
use wasm_bindgen_futures::JsFuture;

use super::decision::DecisionGroup;
use super::files::WorkspaceFile;
use super::missions_roots::{missions_root_of, missions_tree_file};
use super::{FocusIcon, MissionRecord, WorkDirectory};
use crate::app::tauri_bridge::invoke;

// ===== 事件脚本组（事件编辑器的全部文件操作；桌面命令 / SAF 桥接命令成对实现）=====

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScopedReadTextInDir {
	folder_id: String,
	dir_path: String,
	file_name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScopedWriteTextInDir {
	folder_id: String,
	dir_path: String,
	file_name: String,
	contents: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EventFileArgs {
	work_directory: String,
	events_dir: String,
	file_name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportEventArgs {
	work_directory: String,
	mod_dir: String,
	file_name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportEventArgsScoped {
	folder_id: String,
	mod_dir: String,
	file_name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ResolveEventArgs {
	work_directory: String,
	paths: Vec<String>,
	event_name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ResolveEventArgsScoped {
	folder_id: String,
	paths: Vec<String>,
	event_name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EventFileWriteArgs {
	work_directory: String,
	events_dir: String,
	file_name: String,
	contents: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScopedRemoveFile {
	folder_id: String,
	path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScopedMove {
	from_folder_id: String,
	from_path: String,
	to_folder_id: String,
	to_path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EventRenameArgs {
	work_directory: String,
	events_dir: String,
	old_file_name: String,
	new_file_name: String,
}

/// 事件文件路径（`events_dir` 为完整的工作区相对事件目录，如
/// `assets/game/missions/missionsEvents` 或 `assets/game/events/common`）。
fn event_file_path(root_path: &str, events_dir: &str, file_name: &str) -> String {
	if root_path.is_empty() {
		format!("{events_dir}/{file_name}")
	} else {
		format!("{root_path}/{events_dir}/{file_name}")
	}
}

fn event_dir_path(root_path: &str, events_dir: &str) -> String {
	if root_path.is_empty() {
		events_dir.to_string()
	} else {
		format!("{root_path}/{events_dir}")
	}
}

/// 读取事件脚本：桌面 `load_mission_event` / SAF `read_scoped_text_in_dir`。
pub(crate) async fn load_event_text(
	directory: &WorkDirectory,
	events_dir: &str,
	file_name: &str,
) -> Result<String, String> {
	if let Some(folder_id) = &directory.folder_id {
		let args = serde_wasm_bindgen::to_value(&ScopedReadTextInDir {
			folder_id: folder_id.clone(),
			dir_path: event_dir_path(&directory.root_path, events_dir),
			file_name: file_name.to_string(),
		})
		.map_err(|error| error.to_string())?;
		let value = JsFuture::from(invoke("read_scoped_text_in_dir", args))
			.await
			.map_err(|error| format!("读取事件文件失败：{error:?}"))?;
		value
			.as_string()
			.ok_or_else(|| "事件文件读取结果无效".to_string())
	} else {
		let args = serde_wasm_bindgen::to_value(&EventFileArgs {
			work_directory: directory.root_path.clone(),
			events_dir: events_dir.to_string(),
			file_name: file_name.to_string(),
		})
		.map_err(|error| error.to_string())?;
		let value = JsFuture::from(invoke("load_mission_event", args))
			.await
			.map_err(|error| format!("读取事件文件失败：{error:?}"))?;
		value
			.as_string()
			.ok_or_else(|| "事件文件读取结果无效".to_string())
	}
}

/// 保存事件脚本：桌面 `save_mission_event` / SAF `write_scoped_text_in_dir`。
pub(crate) async fn save_event_text(
	directory: &WorkDirectory,
	events_dir: &str,
	file_name: &str,
	contents: &str,
) -> Result<(), String> {
	if let Some(folder_id) = &directory.folder_id {
		let args = serde_wasm_bindgen::to_value(&ScopedWriteTextInDir {
			folder_id: folder_id.clone(),
			dir_path: event_dir_path(&directory.root_path, events_dir),
			file_name: file_name.to_string(),
			contents: contents.to_string(),
		})
		.map_err(|error| error.to_string())?;
		JsFuture::from(invoke("write_scoped_text_in_dir", args))
			.await
			.map_err(|error| format!("保存事件文件失败：{error:?}"))?;
		Ok(())
	} else {
		let args = serde_wasm_bindgen::to_value(&EventFileWriteArgs {
			work_directory: directory.root_path.clone(),
			events_dir: events_dir.to_string(),
			file_name: file_name.to_string(),
			contents: contents.to_string(),
		})
		.map_err(|error| error.to_string())?;
		JsFuture::from(invoke("save_mission_event", args))
			.await
			.map_err(|error| format!("保存事件文件失败：{error:?}"))?;
		Ok(())
	}
}

/// 从源 APK 热导入单个事件脚本（决议编辑器双击缺失事件）：
/// 返回写入的工作区相对路径；`None` = 无源 APK / APK 内没有该脚本
/// （调用方回退模板创建）；环境 / 授权等问题同样静默按 `None` 处理。
/// 桌面 `import_source_apk_event` / SAF `import_source_apk_event_scoped`。
pub(crate) async fn import_source_apk_event_text(
	directory: &WorkDirectory,
	mod_dir: &str,
	file_name: &str,
) -> Option<String> {
	let (cmd, args) = if let Some(folder_id) = &directory.folder_id {
		(
			"import_source_apk_event_scoped",
			serde_wasm_bindgen::to_value(&ImportEventArgsScoped {
				folder_id: folder_id.clone(),
				mod_dir: mod_dir.to_string(),
				file_name: file_name.to_string(),
			})
			.ok()?,
		)
	} else {
		(
			"import_source_apk_event",
			serde_wasm_bindgen::to_value(&ImportEventArgs {
				work_directory: directory.root_path.clone(),
				mod_dir: mod_dir.to_string(),
				file_name: file_name.to_string(),
			})
			.ok()?,
		)
	};
	let value = JsFuture::from(invoke(cmd, args)).await.ok()?;
	value.as_string()
}

/// 按事件 **id** 解析事件脚本路径（决议 `events` 字段索引的是脚本内部 id，
/// 很多模组文件名与 id 不同，如 `chi改任改革派1.txt` 的 id 为 `改任维新派1`）：
/// 返回命中脚本的工作区相对路径；`None` = 候选范围内没有该 id
///（含命令失败 / 未授权，静默降级由调用方回退后续流程）。
/// `paths` 为候选脚本路径（调用方已按模组前缀过滤）；后端按需扫描并会话级缓存。
/// 桌面 `resolve_event_script_by_id` / SAF `resolve_event_script_by_id_scoped`。
pub(crate) async fn resolve_event_script_by_id_text(
	directory: &WorkDirectory,
	paths: &[String],
	event_name: &str,
) -> Option<String> {
	let (cmd, args) = if let Some(folder_id) = &directory.folder_id {
		(
			"resolve_event_script_by_id_scoped",
			serde_wasm_bindgen::to_value(&ResolveEventArgsScoped {
				folder_id: folder_id.clone(),
				paths: paths.to_vec(),
				event_name: event_name.to_string(),
			})
			.ok()?,
		)
	} else {
		(
			"resolve_event_script_by_id",
			serde_wasm_bindgen::to_value(&ResolveEventArgs {
				work_directory: directory.root_path.clone(),
				paths: paths.to_vec(),
				event_name: event_name.to_string(),
			})
			.ok()?,
		)
	};
	let value = JsFuture::from(invoke(cmd, args)).await.ok()?;
	value.as_string()
}

/// 删除事件脚本：桌面 `delete_mission_event` / SAF `remove_scoped_file`。
pub(crate) async fn delete_event_text(
	directory: &WorkDirectory,
	events_dir: &str,
	file_name: &str,
) -> Result<(), String> {
	if let Some(folder_id) = &directory.folder_id {
		let path = event_file_path(&directory.root_path, events_dir, file_name);
		let args = serde_wasm_bindgen::to_value(&ScopedRemoveFile {
			folder_id: folder_id.clone(),
			path,
		})
		.map_err(|error| error.to_string())?;
		JsFuture::from(invoke("remove_scoped_file", args))
			.await
			.map_err(|error| format!("删除事件文件失败：{error:?}"))?;
		Ok(())
	} else {
		let args = serde_wasm_bindgen::to_value(&EventFileArgs {
			work_directory: directory.root_path.clone(),
			events_dir: events_dir.to_string(),
			file_name: file_name.to_string(),
		})
		.map_err(|error| error.to_string())?;
		JsFuture::from(invoke("delete_mission_event", args))
			.await
			.map_err(|error| format!("删除事件文件失败：{error:?}"))?;
		Ok(())
	}
}

/// 重命名事件脚本：桌面 `rename_mission_event` / SAF `move_scoped_item`。
pub(crate) async fn rename_event_text(
	directory: &WorkDirectory,
	events_dir: &str,
	old_file_name: &str,
	new_file_name: &str,
) -> Result<(), String> {
	if let Some(folder_id) = &directory.folder_id {
		let from_path = event_file_path(&directory.root_path, events_dir, old_file_name);
		let to_path = event_file_path(&directory.root_path, events_dir, new_file_name);
		let args = serde_wasm_bindgen::to_value(&ScopedMove {
			from_folder_id: folder_id.clone(),
			from_path,
			to_folder_id: folder_id.clone(),
			to_path,
		})
		.map_err(|error| error.to_string())?;
		JsFuture::from(invoke("move_scoped_item", args))
			.await
			.map_err(|error| format!("重命名事件文件失败：{error:?}"))?;
		Ok(())
	} else {
		let args = serde_wasm_bindgen::to_value(&EventRenameArgs {
			work_directory: directory.root_path.clone(),
			events_dir: events_dir.to_string(),
			old_file_name: old_file_name.to_string(),
			new_file_name: new_file_name.to_string(),
		})
		.map_err(|error| error.to_string())?;
		JsFuture::from(invoke("rename_mission_event", args))
			.await
			.map_err(|error| format!("重命名事件文件失败：{error:?}"))?;
		Ok(())
	}
}

// ===== SAF 原语组（安卓 SAF 模式的低层桥接；桌面模式不经过这里）=====

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScopedEntry {
	pub(crate) name: String,
	pub(crate) path: String,
	pub(crate) is_dir: bool,
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

/// 调用 Android scoped 命令（参数为 camelCase serde 结构，返回 void）。
pub(crate) async fn invoke_scoped<T: Serialize>(
	command: &str,
	context: &str,
	args: &T,
) -> Result<(), String> {
	let args = serde_wasm_bindgen::to_value(args).map_err(|error| error.to_string())?;
	JsFuture::from(invoke(command, args))
		.await
		.map_err(|error| format!("{context}：{error:?}"))?;
	Ok(())
}

/// 读取 Android 工作区目录（走 App 自带快速插件：单次子项游标查询列出一层目录）。
pub(crate) async fn scoped_read_dir(
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
pub(crate) async fn scan_scoped_workspace_files(
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
pub(crate) async fn scoped_read_files_in_dir(
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

pub(crate) async fn scoped_read_text_file(folder_id: &str, path: String) -> Result<String, String> {
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

pub(crate) async fn scoped_write_text_file(
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

pub(crate) async fn scoped_mkdir(folder_id: &str, path: String) -> Result<(), String> {
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

/// 在 SAF 父目录下创建工作区骨架（`missions/missionsImages/H`、`missions/missionsEvents`
/// 与空的 `missions/Missions.json`）；桌面模式由 `create_work_directory` 命令一次完成，
/// 不走这里。返回创建的目录（`root_path` 已在调用方拼好）。
pub(crate) async fn create_scoped_workspace_skeleton(
	folder_id: &str,
	root_path: &str,
) -> Result<(), String> {
	scoped_mkdir(folder_id, root_path.to_string()).await?;
	scoped_mkdir(
		folder_id,
		join_scoped_path(root_path, "missions/missionsImages/H"),
	)
	.await?;
	scoped_mkdir(
		folder_id,
		join_scoped_path(root_path, "missions/missionsEvents"),
	)
	.await?;
	// 空国策树初始文件（serialize_missions 保证与官方排版一致）。
	let args = serde_wasm_bindgen::to_value(&SerializeMissionsArgs { missions: Vec::new() })
		.map_err(|error| error.to_string())?;
	let value = JsFuture::from(invoke("serialize_missions", args))
		.await
		.map_err(|error| format!("初始化任务配置失败：{error:?}"))?;
	let contents = value
		.as_string()
		.ok_or_else(|| "任务配置序列化结果无效".to_string())?;
	scoped_write_text_file(
		folder_id,
		join_scoped_path(root_path, "missions/Missions.json"),
		contents,
	)
	.await
}

pub(crate) fn join_scoped_path(parent: &str, child: &str) -> String {
	if parent.is_empty() {
		child.to_string()
	} else {
		format!("{parent}/{child}")
	}
}

// ===== 国策 / 决议文件组 =====

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkDirectoryArgs {
	pub(crate) work_directory: String,
}

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
struct SerializeMissionsArgs {
	missions: Vec<MissionRecord>,
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

/// 单批并发读取的图标数量，平衡 IPC 并发度与内存占用。
const ICON_READ_CHUNK_SIZE: usize = 8;

/// 加载进度（工作区扫描 / 国策树 / 图标载入等；Work 的全局 `load_progress` 信号元素）。
#[derive(Clone)]
pub(crate) struct LoadProgress {
	pub(crate) stage: String,
	pub(crate) completed: usize,
	pub(crate) total: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LoadDecisionsFileArgs {
	work_directory: String,
	relative_path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SaveDecisionsFileArgs {
	work_directory: String,
	relative_path: String,
	decisions: Vec<DecisionGroup>,
}

#[derive(Serialize)]
struct ParseDecisionsArgs {
	contents: String,
}

#[derive(Serialize)]
struct SerializeDecisionsArgs {
	decisions: Vec<DecisionGroup>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ParsedDecisionsFile {
	decisions: Vec<DecisionGroup>,
	corrected_contents: Option<String>,
}

/// 列举工作区文件：桌面 `list_workspace_files` / SAF 递归 `list_scoped_dir`。
pub(crate) async fn load_workspace_files(
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
/// 桌面 `load_missions_file`；SAF 读文本 + `parse_missions`（宽松语法纠正时写回）。
pub(crate) async fn load_tree_records(
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
/// 桌面 `load_mission_icons`；SAF 分批 `read_scoped_files_in_dir`（带进度回调）。
pub(crate) async fn load_tree_icons(
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
/// 桌面 `load_mission_icons`（单个）；SAF `read_scoped_files_in_dir`（按主干名）。
pub(crate) async fn load_single_icon(
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
/// 桌面 `save_missions_file`；SAF `serialize_missions` + `write_scoped_text_file`。
pub(crate) async fn save_tree_file(
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

/// 读取决议配置（真实路径模式走 `load_decisions_file` 命令；SAF 通道读文本后
/// 经 `parse_decisions` 解析，宽松语法被纠正时写回原文件）。
pub(crate) async fn load_decision_groups(
	work_directory: &WorkDirectory,
	relative_path: &str,
) -> Result<Vec<DecisionGroup>, String> {
	if let Some(folder_id) = &work_directory.folder_id {
		let path = join_scoped_path(&work_directory.root_path, relative_path);
		let contents = scoped_read_text_file(folder_id, path.clone()).await?;
		let args = serde_wasm_bindgen::to_value(&ParseDecisionsArgs { contents })
			.map_err(|error| error.to_string())?;
		let value = JsFuture::from(invoke("parse_decisions", args))
			.await
			.map_err(|error| format!("读取决议配置失败：{error:?}"))?;
		let parsed: ParsedDecisionsFile = serde_wasm_bindgen::from_value(value)
			.map_err(|error| format!("决议配置格式错误：{error}"))?;
		// 宽松语法的文件在打开时被自动纠正：把规范化内容写回原文件。
		if let Some(corrected) = parsed.corrected_contents {
			scoped_write_text_file(folder_id, path, corrected)
				.await
				.map_err(|error| format!("自动纠正语法后保存失败：{error}"))?;
		}
		Ok(parsed.decisions)
	} else {
		let args = serde_wasm_bindgen::to_value(&LoadDecisionsFileArgs {
			work_directory: work_directory.root_path.clone(),
			relative_path: relative_path.to_string(),
		})
		.map_err(|error| error.to_string())?;
		let value = JsFuture::from(invoke("load_decisions_file", args))
			.await
			.map_err(|error| format!("读取决议配置失败：{error:?}"))?;
		serde_wasm_bindgen::from_value(value)
			.map_err(|error| format!("决议配置格式错误：{error}"))
	}
}

/// 保存决议配置到工作区相对路径（真实路径模式走 `save_decisions_file` 命令；
/// SAF 通道先经 `serialize_decisions` 序列化再写入）。
pub(crate) async fn save_decision_groups(
	work_directory: &WorkDirectory,
	relative_path: &str,
	decisions: Vec<DecisionGroup>,
) -> Result<(), String> {
	if let Some(folder_id) = &work_directory.folder_id {
		let args = serde_wasm_bindgen::to_value(&SerializeDecisionsArgs { decisions })
			.map_err(|error| error.to_string())?;
		let value = JsFuture::from(invoke("serialize_decisions", args))
			.await
			.map_err(|error| format!("序列化决议配置失败：{error:?}"))?;
		let contents = value
			.as_string()
			.ok_or_else(|| "决议配置序列化结果无效".to_string())?;
		scoped_write_text_file(
			folder_id,
			join_scoped_path(&work_directory.root_path, relative_path),
			contents,
		)
		.await
	} else {
		let args = serde_wasm_bindgen::to_value(&SaveDecisionsFileArgs {
			work_directory: work_directory.root_path.clone(),
			relative_path: relative_path.to_string(),
			decisions,
		})
		.map_err(|error| error.to_string())?;
		JsFuture::from(invoke("save_decisions_file", args))
			.await
			.map_err(|error| format!("保存决议配置失败：{error:?}"))?;
		Ok(())
	}
}
