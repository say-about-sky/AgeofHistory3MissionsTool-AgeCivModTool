//! 工作目录创建与名称校验命令。

use std::fs;
use std::path::PathBuf;

use crate::mission_format::serialize_mission_file;
use crate::paths::validate_work_directory_name;

/// 校验待创建的工作目录名称（前端提前校验用）。
#[tauri::command]
pub fn validate_work_directory_name_command(directory_name: String) -> Result<(), String> {
    validate_work_directory_name(&directory_name).map(|_| ())
}

/// 在父目录下创建标准工作目录：`missions/` 结构 + 空的 `Missions.json`，返回新目录路径。
#[tauri::command]
pub fn create_work_directory(
    parent_directory: String,
    directory_name: String,
) -> Result<String, String> {
    let directory_name = validate_work_directory_name(&directory_name)?;

    let work_directory = PathBuf::from(parent_directory).join(directory_name);
    fs::create_dir(&work_directory).map_err(|error| error.to_string())?;
    let missions_directory = work_directory.join("missions");
    fs::create_dir_all(missions_directory.join("missionsImages").join("H"))
        .map_err(|error| error.to_string())?;
    fs::create_dir_all(missions_directory.join("missionsEvents"))
        .map_err(|error| error.to_string())?;
    let content = serialize_mission_file(&[])?;
    fs::write(missions_directory.join("Missions.json"), content)
        .map_err(|error| error.to_string())?;

    Ok(work_directory.to_string_lossy().into_owned())
}
