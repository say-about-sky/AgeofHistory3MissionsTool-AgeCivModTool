//! 国策事件脚本（`<国策资源根>/missionsEvents` 目录）的读写命令。

use std::fs;

use crate::paths::{missions_root_path, validate_work_directory_name};

/// 读取事件脚本内容。`missions_root` 为工作区内的国策资源根目录相对路径。
#[tauri::command]
pub fn load_mission_event(
    work_directory: String,
    missions_root: String,
    file_name: String,
) -> Result<String, String> {
    validate_work_directory_name(&file_name)?;
    let path = missions_root_path(&work_directory, &missions_root)?
        .join("missionsEvents")
        .join(&file_name);
    fs::read_to_string(&path)
        .map_err(|error| format!("读取事件脚本失败 {}：{error}", path.display()))
}

/// 保存事件脚本内容。
#[tauri::command]
pub fn save_mission_event(
    work_directory: String,
    missions_root: String,
    file_name: String,
    contents: String,
) -> Result<(), String> {
    validate_work_directory_name(&file_name)?;
    let path = missions_root_path(&work_directory, &missions_root)?
        .join("missionsEvents")
        .join(&file_name);
    fs::write(&path, contents)
        .map_err(|error| format!("保存事件脚本失败 {}：{error}", path.display()))
}

/// 删除事件脚本。
#[tauri::command]
pub fn delete_mission_event(
    work_directory: String,
    missions_root: String,
    file_name: String,
) -> Result<(), String> {
    validate_work_directory_name(&file_name)?;
    let path = missions_root_path(&work_directory, &missions_root)?
        .join("missionsEvents")
        .join(&file_name);
    fs::remove_file(&path)
        .map_err(|error| format!("删除事件脚本失败 {}：{error}", path.display()))
}

/// 重命名事件脚本（同目录内改名）。
#[tauri::command]
pub fn rename_mission_event(
    work_directory: String,
    missions_root: String,
    old_file_name: String,
    new_file_name: String,
) -> Result<(), String> {
    validate_work_directory_name(&old_file_name)?;
    validate_work_directory_name(&new_file_name)?;
    let directory = missions_root_path(&work_directory, &missions_root)?.join("missionsEvents");
    let from = directory.join(&old_file_name);
    let to = directory.join(&new_file_name);
    fs::rename(&from, &to)
        .map_err(|error| format!("重命名事件脚本失败 {}：{error}", from.display()))
}
