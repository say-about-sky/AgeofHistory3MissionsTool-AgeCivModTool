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

/// 把工作区内的国策资源根目录（末段必须是 `missions`，如 `missions`、
/// `assets/game/missions`、`assets/map/<地图>/scenarios/<剧本>/missions`）
/// 解析为绝对路径，校验合法且目录存在。
pub fn missions_root_path(work_directory: &str, missions_root: &str) -> Result<PathBuf, String> {
    validate_relative_path(missions_root)?;
    let is_missions_dir = Path::new(missions_root)
        .file_name()
        .is_some_and(|name| name == "missions");
    if !is_missions_dir {
        return Err(format!("不是有效的国策资源目录：{missions_root}"));
    }
    let root = PathBuf::from(work_directory);
    if !root.is_dir() {
        return Err(format!("工作目录不存在：{}", root.display()));
    }
    let directory = root.join(missions_root);
    if !directory.is_dir() {
        return Err(format!(
            "所选目录下未找到国策资源目录：{}",
            directory.display()
        ));
    }
    Ok(directory)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missions_root_path_accepts_nested_and_rejects_escapes() {
        let dir = std::env::temp_dir().join(format!("ageciv-paths-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("assets/game/missions")).unwrap();
        let work = dir.to_string_lossy().into_owned();

        assert!(missions_root_path(&work, "assets/game/missions").is_ok());
        // 目录不存在
        assert!(missions_root_path(&work, "missions").is_err());
        // 不允许逃逸工作区
        assert!(missions_root_path(&work, "../outside").is_err());
        // 末段必须是 missions
        assert!(missions_root_path(&work, "assets/game").is_err());
        // 不允许绝对路径
        assert!(missions_root_path(&work, "C:/absolute").is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
