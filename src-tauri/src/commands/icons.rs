//! 国策图标相关命令：加载、列出与 base64 编码。

use std::env;
use std::fs;
use std::path::PathBuf;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use rayon::prelude::*;
use serde::Deserialize;

use crate::models::FocusIcon;
use crate::paths::{missions_directory, validate_work_directory_name};

/// 把单个图标字节编码为 data URL。
#[tauri::command]
pub fn encode_mission_icon(name: String, data: Vec<u8>) -> FocusIcon {
    FocusIcon {
        name,
        data_url: format!("data:image/png;base64,{}", STANDARD.encode(data)),
    }
}

/// `encode_mission_icons` 的入参：图标名 + 原始字节。
#[derive(Deserialize)]
pub struct IconPayload {
    name: String,
    data: Vec<u8>,
}

/// 批量把图标字节编码为 data URL。
#[tauri::command]
pub fn encode_mission_icons(icons: Vec<IconPayload>) -> Vec<FocusIcon> {
    icons
        .into_iter()
        .map(|icon| FocusIcon {
            name: icon.name,
            data_url: format!("data:image/png;base64,{}", STANDARD.encode(icon.data)),
        })
        .collect()
}

/// 按名称加载 missionsImages/H 下的指定图标（缺失的图标静默跳过）。
/// 使用 rayon 并行读取并 base64 编码，多核设备上批量图标载入显著加速。
#[tauri::command]
pub fn load_mission_icons(
    work_directory: String,
    names: Vec<String>,
) -> Result<Vec<FocusIcon>, String> {
    let icon_directory = missions_directory(&work_directory)?.join("missionsImages").join("H");
    let icons = names
        .into_par_iter()
        .filter_map(|name| {
            if validate_work_directory_name(&name).is_err() {
                return None;
            }
            let path = icon_directory.join(format!("{name}.png"));
            let bytes = fs::read(&path).ok()?;
            Some(FocusIcon {
                name,
                data_url: format!("data:image/png;base64,{}", STANDARD.encode(bytes)),
            })
        })
        .collect();
    Ok(icons)
}

/// 列出 missionsImages/H 下的全部图标。
#[tauri::command]
pub fn list_mission_icons(work_directory: String) -> Result<Vec<FocusIcon>, String> {
    let icon_directory = missions_directory(&work_directory)?.join("missionsImages").join("H");
    let mut paths = fs::read_dir(&icon_directory)
        .map_err(|error| error.to_string())?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    paths.retain(|path| {
        path.extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
    });
    paths.sort();

    paths
        .into_iter()
        .map(|path| {
            let name = path
                .file_stem()
                .ok_or_else(|| format!("无效图标路径：{}", path.display()))?
                .to_string_lossy()
                .into_owned();
            let bytes = fs::read(&path).map_err(|error| error.to_string())?;
            Ok(FocusIcon {
                name,
                data_url: format!("data:image/png;base64,{}", STANDARD.encode(bytes)),
            })
        })
        .collect()
}

/// 列出内置 `png/` 目录下的图标（依次在运行目录、可执行文件目录与源码目录旁查找）。
#[tauri::command]
pub fn list_focus_icons() -> Vec<FocusIcon> {
    let mut directories = Vec::new();
    if let Ok(current_dir) = env::current_dir() {
        directories.push(current_dir.join("png"));
    }
    if let Ok(executable) = env::current_exe() {
        if let Some(executable_dir) = executable.parent() {
            directories.push(executable_dir.join("png"));
        }
    }
    directories.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../png"));

    let Some(icon_dir) = directories.into_iter().find(|path| path.is_dir()) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(icon_dir) else {
        return Vec::new();
    };

    let mut paths: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
        })
        .collect();
    paths.sort();

    paths
        .into_iter()
        .filter_map(|path| {
            let name = path.file_stem()?.to_string_lossy().into_owned();
            let bytes = fs::read(path).ok()?;
            Some(FocusIcon {
                name,
                data_url: format!("data:image/png;base64,{}", STANDARD.encode(bytes)),
            })
        })
        .collect()
}
