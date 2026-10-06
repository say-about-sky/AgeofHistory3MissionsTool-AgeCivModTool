//! Android scoped storage（SAF）读写通道。
//!
//! 当工作区目录只能通过 `content://` 授权访问（无法解析出真实路径）时，
//! 前端改走这些命令，由 App 自带的存储插件逐文件读写；若能解析出真实路径，
//! 则改走与桌面端相同的原生命令。

use std::fs;
use std::path::PathBuf;

use crate::models::{FocusIcon, ScannedEntry};

/// 列出 scoped 目录下的条目（由 App 自带插件快速列目录返回，避免逐文件属性查询）。
#[tauri::command]
pub fn list_scoped_dir<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    folder_id: String,
    path: Option<String>,
) -> Result<Vec<ScannedEntry>, String> {
    #[cfg(target_os = "android")]
    {
        crate::all_files_access::list_dir(app, folder_id, path)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, folder_id, path);
        Err("当前平台不支持 Android 工作区目录读取".to_string())
    }
}

/// 批量读取 scoped 目录内指定文件的图标（base64 data URL）。
#[tauri::command]
pub fn read_scoped_files_in_dir<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    folder_id: String,
    dir_path: String,
    names: Vec<String>,
) -> Result<Vec<FocusIcon>, String> {
    #[cfg(target_os = "android")]
    {
        crate::all_files_access::read_files_in_dir(app, folder_id, dir_path, names).map(|files| {
            files
                .into_iter()
                .map(|file| FocusIcon {
                    name: file.name,
                    data_url: file.data_url,
                })
                .collect()
        })
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, folder_id, dir_path, names);
        Err("当前平台不支持 Android 工作区图标读取".to_string())
    }
}

/// 读取 scoped 目录内的单个文本文件。
#[tauri::command]
pub fn read_scoped_text_in_dir<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    folder_id: String,
    dir_path: String,
    file_name: String,
) -> Result<String, String> {
    #[cfg(target_os = "android")]
    {
        crate::all_files_access::read_text_in_dir(app, folder_id, dir_path, file_name)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, folder_id, dir_path, file_name);
        Err("当前平台不支持 Android 工作区文件读取".to_string())
    }
}

/// 写入 scoped 目录内的单个文本文件。
#[tauri::command]
pub fn write_scoped_text_in_dir<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    folder_id: String,
    dir_path: String,
    file_name: String,
    contents: String,
) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        crate::all_files_access::write_text_in_dir(app, folder_id, dir_path, file_name, contents)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, folder_id, dir_path, file_name, contents);
        Err("当前平台不支持 Android 工作区文件写入".to_string())
    }
}

/// 尝试把 Android SAF 工作区解析为真实文件系统路径。
///
/// 成功时前端会走与桌面端相同的原生命令（Rust 直接读写、可多线程），彻底绕过缓慢的
/// ContentProvider 逐项查询；失败时自动退回原有的 scoped-storage 通道。
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
