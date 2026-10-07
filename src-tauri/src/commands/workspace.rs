//! 工作区文件管理命令：列举、复制、移动、删除与在文件管理器中定位。

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::paths::workspace_abs_path;

/// 工作区内的一个条目（文件或文件夹）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFile {
    name: String,
    relative_path: String,
    is_directory: bool,
}

/// 递归列举工作目录下的全部文件/文件夹（相对路径统一用 `/` 分隔）。
#[tauri::command]
pub fn list_workspace_files(work_directory: String) -> Result<Vec<WorkspaceFile>, String> {
    let root = PathBuf::from(work_directory);
    if !root.is_dir() {
        return Err(format!("工作目录不存在：{}", root.display()));
    }
    let mut entries = Vec::new();
    append_workspace_entries(&root, &root, &mut entries)?;
    Ok(entries)
}

/// 递归收集目录下的条目。每层目录内「文件夹优先、其次文件」，
/// 同组内按名称不区分大小写排序（与 Android SAF 目录列表保持一致）。
fn append_workspace_entries(
    root: &Path,
    directory: &Path,
    entries: &mut Vec<WorkspaceFile>,
) -> Result<(), String> {
    // 先收集（路径, 是否文件夹, 名称）再排序：避免在比较函数里反复 stat。
    let mut children: Vec<(PathBuf, bool, String)> = fs::read_dir(directory)
        .map_err(|error| error.to_string())?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|path| {
            let is_directory = fs::symlink_metadata(&path)
                .map_err(|error| error.to_string())?
                .file_type()
                .is_dir();
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            Ok((path, is_directory, name))
        })
        .collect::<Result<Vec<_>, String>>()?;
    children.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| left.2.to_lowercase().cmp(&right.2.to_lowercase()))
            .then_with(|| left.2.cmp(&right.2))
    });

    for (path, is_directory, name) in children {
        let relative_path = path
            .strip_prefix(root)
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        entries.push(WorkspaceFile {
            name,
            relative_path,
            is_directory,
        });
        if is_directory {
            append_workspace_entries(root, &path, entries)?;
        }
    }
    Ok(())
}

/// 递归复制目录及其内容。
fn copy_dir_recursive(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|error| error.to_string())?;
    let children = fs::read_dir(from)
        .map_err(|error| error.to_string())?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    for path in children {
        let name = path
            .file_name()
            .ok_or_else(|| format!("无效路径：{}", path.display()))?;
        let target = to.join(name);
        if path.is_dir() {
            copy_dir_recursive(&path, &target)?;
        } else {
            fs::copy(&path, &target)
                .map_err(|error| format!("复制失败 {}：{error}", path.display()))?;
        }
    }
    Ok(())
}

/// 复制工作区内的文件/文件夹到新位置（目标已存在时报错）。
#[tauri::command]
pub fn copy_workspace_item(
    work_directory: String,
    source: String,
    target: String,
) -> Result<(), String> {
    let from = workspace_abs_path(&work_directory, &source)?;
    let to = workspace_abs_path(&work_directory, &target)?;
    if !from.exists() {
        return Err(format!("源不存在：{}", from.display()));
    }
    if to.exists() {
        return Err(format!("目标已存在：{}", to.display()));
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    if from.is_dir() {
        copy_dir_recursive(&from, &to)
    } else {
        fs::copy(&from, &to)
            .map(|_| ())
            .map_err(|error| format!("复制失败 {}：{error}", from.display()))
    }
}

/// 移动工作区内的文件/文件夹到新位置。
#[tauri::command]
pub fn move_workspace_item(
    work_directory: String,
    source: String,
    target: String,
) -> Result<(), String> {
    let from = workspace_abs_path(&work_directory, &source)?;
    let to = workspace_abs_path(&work_directory, &target)?;
    if !from.exists() {
        return Err(format!("源不存在：{}", from.display()));
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::rename(&from, &to).map_err(|error| format!("移动失败 {}：{error}", from.display()))
}

/// 删除工作区内的文件/文件夹（文件夹递归删除）。
#[tauri::command]
pub fn delete_workspace_item(work_directory: String, target: String) -> Result<(), String> {
    let path = workspace_abs_path(&work_directory, &target)?;
    if !path.exists() {
        return Err(format!("目标不存在：{}", path.display()));
    }
    if path.is_dir() {
        fs::remove_dir_all(&path).map_err(|error| error.to_string())
    } else {
        fs::remove_file(&path).map_err(|error| error.to_string())
    }
}

/// 打开文件位置：桌面端在系统文件管理器中选中条目（Windows 经 SHOpenFolderAndSelectItems，
/// 带空格/中文的路径也能正确高亮）；Android 端通过目录树 URI 解析出条目的 content:// URI，
/// 再弹出「用哪个应用打开」选择器（文件用 VIEW、目录用目录视图）。
/// `tree_uri` 为 Android 工作目录（或其父目录）的 SAF 授权 URI；桌面端忽略。
#[tauri::command]
pub async fn reveal_workspace_item<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    work_directory: String,
    target: String,
    tree_uri: Option<String>,
    is_directory: bool,
) -> Result<(), String> {
    #[cfg(not(target_os = "android"))]
    {
        let _ = (tree_uri, is_directory);
        let path = workspace_abs_path(&work_directory, &target)?;
        if !path.exists() {
            return Err(format!("目标不存在：{}", path.display()));
        }
        use tauri_plugin_opener::OpenerExt;
        app.opener()
            .reveal_item_in_dir(&path)
            .map_err(|error| format!("无法定位该条目：{error}"))
    }
    #[cfg(target_os = "android")]
    {
        let _ = work_directory;
        use tauri_plugin_android_fs::AndroidFsExt;
        let tree = tree_uri
            .filter(|tree| !tree.is_empty())
            .ok_or_else(|| "缺少目录访问授权：请通过「打开工作区」重新选择该目录后再试".to_string())?;
        let api = app.android_fs_async();
        let dir = crate::android_fs_bridge::root_uri(&tree);
        let uri = if is_directory {
            api.resolve_dir_uri(&dir, &target).await
        } else {
            api.resolve_file_uri(&dir, &target).await
        }
        .map_err(|error| format!("无法定位所选条目（可能已被移动或删除）：{error}"))?;
        let opener = api.opener();
        let result = if is_directory {
            opener.open_dir(&uri).await
        } else {
            opener.open_file(&uri).await
        };
        result.map_err(|error| format!("系统无法打开该条目：{error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ageciv-workspace-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 每层目录：文件夹优先、其次文件（同组按名称不区分大小写），
    /// 且子目录内容紧跟父目录行（前序 DFS，资源管理器按此顺序渲染）。
    #[test]
    fn workspace_listing_puts_directories_before_files() {
        let dir = temp_dir("order");
        fs::write(dir.join("aardvark.json"), "{}").unwrap();
        fs::write(dir.join("Apple.json"), "{}").unwrap();
        fs::create_dir_all(dir.join("missionsEvents")).unwrap();
        fs::write(dir.join("missionsEvents").join("zz.txt"), "z").unwrap();
        fs::write(dir.join("missionsEvents").join("aa.txt"), "a").unwrap();
        fs::create_dir_all(dir.join("missionsImages")).unwrap();

        let files = list_workspace_files(dir.to_string_lossy().into_owned()).unwrap();
        let order: Vec<String> = files
            .iter()
            .map(|file| file.relative_path.clone())
            .collect();
        assert_eq!(
            order,
            vec![
                "missionsEvents",
                "missionsEvents/aa.txt",
                "missionsEvents/zz.txt",
                "missionsImages",
                "aardvark.json",
                "Apple.json",
            ]
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
