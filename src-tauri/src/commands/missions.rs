//! 国策树配置（Missions.json 及多国策树文件）的读写命令。

use std::fs;
use std::path::Path;

use serde::Serialize;

use crate::mission_format::{parse_mission_file_with_correction, serialize_mission_file};
use crate::models::MissionRecord;
use crate::paths::{missions_directory, validate_work_directory_name};

/// `parse_missions` 的返回：解析出的国策记录；语法被自动纠正时附带规范化后的完整内容，
/// 由前端（Android scoped 通道）负责写回原文件。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedMissions {
    missions: Vec<MissionRecord>,
    corrected_contents: Option<String>,
}

/// 解析国策配置文本（宽松语法会自动纠正，并返回规范化内容供调用方写回）。
#[tauri::command]
pub fn parse_missions(contents: String) -> Result<ParsedMissions, String> {
    let (file, corrected) = parse_mission_file_with_correction(&contents)?;
    Ok(ParsedMissions {
        missions: file.mission,
        corrected_contents: corrected,
    })
}

/// 规范化并序列化国策列表（按顺序重写 ID、补默认事件文件名、AI 统一为 100）。
#[tauri::command]
pub fn serialize_missions(mut missions: Vec<MissionRecord>) -> Result<String, String> {
    for (index, mission) in missions.iter_mut().enumerate() {
        mission.id = index as i64;
        if mission.mission_event.trim().is_empty() {
            mission.mission_event = format!("{}.txt", mission.name);
        }
        mission.ai = 100;
    }
    serialize_mission_file(&missions)
}

/// 读取并解析国策配置文件；若是宽松语法，自动纠正并把规范内容写回原文件。
fn read_missions_file_with_autofix(path: &Path) -> Result<Vec<MissionRecord>, String> {
    let content = fs::read_to_string(path)
        .map_err(|error| format!("读取国策配置失败 {}：{error}", path.display()))?;
    let (file, corrected) = parse_mission_file_with_correction(&content)?;
    if let Some(corrected) = corrected {
        fs::write(path, corrected).map_err(|error| {
            format!(
                "国策文件语法已自动纠正，但写回失败 {}：{error}",
                path.display()
            )
        })?;
    }
    Ok(file.mission)
}

/// 加载工作目录下的 `missions/Missions.json`。
#[tauri::command]
pub fn load_missions(work_directory: String) -> Result<Vec<MissionRecord>, String> {
    let path = missions_directory(&work_directory)?.join("Missions.json");
    read_missions_file_with_autofix(&path)
}

/// 保存国策列表到工作目录下的 `missions/Missions.json`。
#[tauri::command]
pub fn save_missions(work_directory: String, missions: Vec<MissionRecord>) -> Result<(), String> {
    let content = serialize_missions(missions)?;
    fs::write(missions_directory(&work_directory)?.join("Missions.json"), content)
        .map_err(|error| error.to_string())
}

/// 按文件名读取 missions 目录下的单个国策树配置（多国策树工作区）。
/// 若文件使用宽松语法（缺逗号/裸文本值等），打开时自动纠正为规范格式并写回原文件。
#[tauri::command]
pub fn load_missions_file(
    work_directory: String,
    file_name: String,
) -> Result<Vec<MissionRecord>, String> {
    validate_work_directory_name(&file_name)?;
    let path = missions_directory(&work_directory)?.join(&file_name);
    read_missions_file_with_autofix(&path)
}

/// 按文件名保存单个国策树配置到 missions 目录下。
#[tauri::command]
pub fn save_missions_file(
    work_directory: String,
    file_name: String,
    missions: Vec<MissionRecord>,
) -> Result<(), String> {
    validate_work_directory_name(&file_name)?;
    let content = serialize_missions(missions)?;
    fs::write(missions_directory(&work_directory)?.join(&file_name), content)
        .map_err(|error| error.to_string())
}
