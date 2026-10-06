//! 前后端共享的数据结构。
//!
//! 这些类型通过 Tauri 命令在 Rust 与前端（Dioxus UI）之间传递，
//! 字段上的 `serde` 属性决定 JSON 键名，修改时需同步检查前端代码。

use serde::{Deserialize, Serialize};

/// Android 工作区扫描条目（由 App 自带插件快速列目录返回，避免逐文件属性查询）。
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannedEntry {
    /// 文件/文件夹名称。
    pub name: String,
    /// 绝对路径。
    pub path: String,
    /// 是否为文件夹。
    pub is_dir: bool,
}

/// 国策图标：图标名称 + 可直接用于 `<img src>` 的 base64 data URL。
#[derive(Serialize)]
pub struct FocusIcon {
    pub name: String,
    pub data_url: String,
}

/// 单条国策记录，对应国策文件 `Mission` 数组中的一项。
#[derive(Deserialize, Serialize)]
pub struct MissionRecord {
    /// 国策序号（保存时按数组顺序重写）。
    #[serde(rename = "ID")]
    pub id: i64,
    /// 国策名称。
    #[serde(rename = "Name")]
    pub name: String,
    /// 图标文件名。
    #[serde(rename = "ImageName")]
    pub image_name: String,
    /// 关联的事件脚本文件名。
    #[serde(rename = "MissionEvent")]
    pub mission_event: String,
    /// 国策树列坐标。
    #[serde(rename = "TreeColumn")]
    pub tree_column: u32,
    /// 国策树行坐标。
    #[serde(rename = "TreeRow")]
    pub tree_row: u32,
    /// 前置国策 ID（-1 表示无）。
    #[serde(rename = "RequiredMission")]
    pub required_mission: i64,
    /// 第二个前置国策 ID（-1 表示无）。
    #[serde(rename = "RequiredMission2")]
    pub required_mission2: i64,
    /// AI 权重（保存时统一写入 100）。
    #[serde(rename = "AI")]
    pub ai: i32,
}
