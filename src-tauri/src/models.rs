//! 前后端共享的数据结构。
//!
//! 这些类型通过 Tauri 命令在 Rust 与前端（Dioxus UI）之间传递，
//! 字段上的 `serde` 属性决定 JSON 键名，修改时需同步检查前端代码。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
///
/// 前置国策有两种写法（模组可混用，解析与写回时都按原样保留）：
/// - 标量写法：`RequiredMission` / `RequiredMission2`（各 1 个 ID，-1 表示无）；
/// - 列表写法：`RequiredMissions`（需全部完成）、`RequiredMissionsOR` /
///   `RequiredMissionsOR2` / `RequiredMissionsOR3`（各为一组「任一满足」）、
///   `MutuallyExclusiveMissions`（互斥，不能同时选择）。
#[derive(Deserialize, Serialize)]
pub struct MissionRecord {
    /// 国策序号（保存时按数组顺序重写）。
    #[serde(rename = "ID")]
    pub id: i64,
    /// 国策名称。
    #[serde(rename = "Name", default)]
    pub name: String,
    /// 图标文件名。
    #[serde(rename = "ImageName", default)]
    pub image_name: String,
    /// 关联的事件脚本文件名。
    #[serde(rename = "MissionEvent", default)]
    pub mission_event: String,
    /// 国策树列坐标。
    #[serde(rename = "TreeColumn", default)]
    pub tree_column: u32,
    /// 国策树行坐标。
    #[serde(rename = "TreeRow", default)]
    pub tree_row: u32,
    /// 前置国策 ID（-1 表示无）；`None` = 原文件未写该字段。
    #[serde(
        rename = "RequiredMission",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub required_mission: Option<i64>,
    /// 第二个前置国策 ID（-1 表示无）；`None` = 原文件未写该字段。
    #[serde(
        rename = "RequiredMission2",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub required_mission2: Option<i64>,
    /// 前置国策 ID 列表（需全部完成；列表写法）。
    #[serde(
        rename = "RequiredMissions",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub required_missions: Option<Vec<i64>>,
    /// 前置国策 ID 列表（满足其中任一即可；OR 组 1）。
    #[serde(
        rename = "RequiredMissionsOR",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub required_missions_or: Option<Vec<i64>>,
    /// OR 组 2（满足其中任一即可）。
    #[serde(
        rename = "RequiredMissionsOR2",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub required_missions_or2: Option<Vec<i64>>,
    /// OR 组 3（满足其中任一即可）。
    #[serde(
        rename = "RequiredMissionsOR3",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub required_missions_or3: Option<Vec<i64>>,
    /// 互斥国策 ID 列表（不能与这些国策同时选择）。
    #[serde(
        rename = "MutuallyExclusiveMissions",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub mutually_exclusive_missions: Option<Vec<i64>>,
    /// AI 权重（0~100；缺失时按 100 读取，保存时沿用原值）。
    #[serde(rename = "AI", default = "default_mission_ai")]
    pub ai: i32,
    /// 工具尚未注册的未知字段（未来游戏 / 模组新增键）：解析时按名收集、保存时原样
    /// 写出，避免「打开-保存一次就把新字段抹掉」。已知字段优先匹配，不会被吞。
    /// 值保持 JSON 原样（数字 / 布尔 / 字符串 / 数组 / 对象）。
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// 缺少 `AI` 字段时的默认权重（与工具新建卡片的默认一致）。
fn default_mission_ai() -> i32 {
    100
}
