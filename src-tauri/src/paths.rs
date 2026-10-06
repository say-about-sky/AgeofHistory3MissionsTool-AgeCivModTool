//! 路径校验与定位工具。
//!
//! 集中处理「用户/前端提供的名称或相对路径 → 安全的磁盘路径」的转换，
//! 防止 `..`、绝对路径等写法逃逸出工作目录。

use std::path::{Component, Path, PathBuf};

/// 校验目录/文件名是单个有效名称（非空、不含路径分隔符或 `..`）。
pub fn validate_work_directory_name(directory_name: &str) -> Result<&str, String> {
    let directory_name = directory_name.trim();
    let mut components = Path::new(directory_name).components();
    if directory_name.is_empty()
        || !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
    {
        return Err("目录名称必须是单个有效文件夹名".to_string());
    }
    Ok(directory_name)
}

/// 校验工作区内相对路径（拒绝空路径、绝对路径与 `..`）。
pub fn validate_relative_path(relative: &str) -> Result<(), String> {
    let path = Path::new(relative);
    if relative.is_empty() || path.is_absolute() {
        return Err(format!("无效的工作区相对路径：{relative}"));
    }
    for component in path.components() {
        if !matches!(component, Component::Normal(_)) {
            return Err(format!("工作区路径不能包含 .. 或特殊段：{relative}"));
        }
    }
    Ok(())
}

/// 把工作区相对路径解析为绝对路径，同时确认工作目录存在。
pub fn workspace_abs_path(work_directory: &str, relative: &str) -> Result<PathBuf, String> {
    validate_relative_path(relative)?;
    let root = PathBuf::from(work_directory);
    if !root.is_dir() {
        return Err(format!("工作目录不存在：{}", root.display()));
    }
    Ok(root.join(relative))
}

/// 定位工作目录下的 `missions` 文件夹（不存在时报错）。
pub fn missions_directory(work_directory: &str) -> Result<PathBuf, String> {
    let directory = PathBuf::from(work_directory).join("missions");
    if directory.is_dir() {
        Ok(directory)
    } else {
        Err(format!(
            "所选目录下未找到 missions 文件夹：{}",
            directory.display()
        ))
    }
}
