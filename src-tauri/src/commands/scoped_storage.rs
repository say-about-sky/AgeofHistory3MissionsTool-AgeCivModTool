//! Android scoped storage（SAF）读写命令。
//!
//! 当工作区目录只能通过 `content://` 授权访问（无法解析出真实路径）时，
//! 前端改走这些命令；实际文件操作由 `android_fs_bridge`（tauri-plugin-android-fs）
//! 完成。若能解析出真实路径，前端会改走与桌面端相同的原生 std::fs 命令。

use std::fs;
use std::path::PathBuf;

use crate::android_fs_bridge as bridge;
use crate::models::{FocusIcon, ScannedEntry};

/// 弹出系统目录选择器（返回目录树 URI 作为 `folder_id`，并持久化授权）。
#[tauri::command]
pub async fn pick_android_folder<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<Option<bridge::PickedFolder>, String> {
    bridge::pick_folder(&app).await
}

/// 解析 SAF 目录下已存在子目录的文档 URI（Android 真实路径模式下「打开文件位置」用）。
#[tauri::command]
pub async fn resolve_scoped_child_dir_uri<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    folder_id: String,
    relative: String,
) -> Result<String, String> {
    bridge::resolve_child_dir_uri(&app, &folder_id, &relative).await
}

/// 列出 scoped 目录下的条目。
#[tauri::command]
pub async fn list_scoped_dir<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    folder_id: String,
    path: Option<String>,
) -> Result<Vec<ScannedEntry>, String> {
    bridge::list_dir(&app, &folder_id, path).await
}

/// 批量读取 scoped 目录内指定文件的图标（base64 data URL）。
#[tauri::command]
pub async fn read_scoped_files_in_dir<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    folder_id: String,
    dir_path: String,
    names: Vec<String>,
) -> Result<Vec<FocusIcon>, String> {
    bridge::read_files_in_dir(&app, &folder_id, &dir_path, names).await
}

/// 读取 scoped 目录内的单个文本文件。
#[tauri::command]
pub async fn read_scoped_text_in_dir<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    folder_id: String,
    dir_path: String,
    file_name: String,
) -> Result<String, String> {
    bridge::read_text_in_dir(&app, &folder_id, &dir_path, &file_name).await
}

/// 写入 scoped 目录内的单个文本文件（必要时自动创建目录与文件）。
#[tauri::command]
pub async fn write_scoped_text_in_dir<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    folder_id: String,
    dir_path: String,
    file_name: String,
    contents: String,
) -> Result<(), String> {
    bridge::write_text_in_dir(&app, &folder_id, &dir_path, &file_name, &contents).await?;
    // 内容可能修改了事件 id：失效该脚本的索引缓存。
    crate::commands::events::invalidate_event_script_cache(&format!(
        "{}/{}",
        dir_path.trim_matches('/'),
        file_name
    ));
    Ok(())
}

/// 读取工作区相对路径下的文本文件。
#[tauri::command]
pub async fn read_scoped_text_file<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    folder_id: String,
    path: String,
) -> Result<String, String> {
    bridge::read_text_file(&app, &folder_id, &path).await
}

/// 写入工作区相对路径下的文本文件（`recursive` 为 true 时自动创建父目录）。
#[tauri::command]
pub async fn write_scoped_text_file<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    folder_id: String,
    path: String,
    contents: String,
    recursive: bool,
) -> Result<(), String> {
    bridge::write_text_file(&app, &folder_id, &path, &contents, recursive).await?;
    crate::commands::events::invalidate_event_script_cache(&path);
    Ok(())
}

/// 创建目录（`recursive` 为 false 时要求父目录已存在）。
#[tauri::command]
pub async fn mkdir_scoped_dir<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    folder_id: String,
    path: String,
    recursive: bool,
) -> Result<(), String> {
    bridge::create_dir(&app, &folder_id, &path, recursive).await
}

/// 删除文件。
#[tauri::command]
pub async fn remove_scoped_file<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    folder_id: String,
    path: String,
) -> Result<(), String> {
    bridge::remove_file(&app, &folder_id, &path).await?;
    crate::commands::events::invalidate_event_script_cache(&path);
    Ok(())
}

/// 删除目录（`recursive` 为 true 时连同内容删除）。
#[tauri::command]
pub async fn remove_scoped_dir<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    folder_id: String,
    path: String,
    recursive: bool,
) -> Result<(), String> {
    bridge::remove_dir(&app, &folder_id, &path, recursive).await
}

/// 复制文件或目录（支持跨工作区目录；同名文件覆盖、同名目录合并）。
#[tauri::command]
pub async fn copy_scoped_item<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    from_folder_id: String,
    from_path: String,
    to_folder_id: String,
    to_path: String,
) -> Result<(), String> {
    bridge::copy_item(&app, &from_folder_id, &from_path, &to_folder_id, &to_path).await
}

/// 移动/重命名文件或目录（同目录改名走单次 rename，跨目录走复制 + 删除）。
#[tauri::command]
pub async fn move_scoped_item<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    from_folder_id: String,
    from_path: String,
    to_folder_id: String,
    to_path: String,
) -> Result<(), String> {
    bridge::move_item(&app, &from_folder_id, &from_path, &to_folder_id, &to_path).await?;
    crate::commands::events::invalidate_event_script_cache(&from_path);
    crate::commands::events::invalidate_event_script_cache(&to_path);
    Ok(())
}

/// 尝试把 Android SAF 工作区解析为真实文件系统路径。
///
/// 成功时前端会走与桌面端相同的原生命令（Rust 直接读写、可多线程），彻底绕过缓慢的
/// ContentProvider 逐项查询；失败时自动退回 scoped 通道。
#[tauri::command]
pub fn resolve_scoped_workspace_path<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    folder_id: String,
) -> Result<Option<String>, String> {
    #[cfg(target_os = "android")]
    {
        crate::all_files_access::resolve_folder_path(app, folder_id)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, folder_id);
        Ok(None)
    }
}

/// 探测真实路径是否可读（选目录后做一次廉价校验，失败则退回 scoped 通道）。
#[tauri::command]
pub fn is_workspace_path_readable(path: String) -> bool {
    let path = PathBuf::from(path);
    path.is_dir() && fs::read_dir(&path).is_ok()
}
