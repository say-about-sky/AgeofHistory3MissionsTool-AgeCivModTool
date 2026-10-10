pub mod mind;
pub mod decision;
pub mod undo;
pub mod work;
pub mod files;
pub mod frame;
pub mod event;
pub mod event_grid;
pub mod event_lookup;
pub mod event_parser;
pub mod event_schema;
pub mod game_text;
pub mod missions_roots;
/// 平台分派文件操作（桌面真实路径 / 安卓 SAF 两套实现的集中地；新增文件操作请加在这里）。
pub mod platform_fs;
pub use mind::{FocusIcon, FocusTarget, MindClipboard, MindMapCanvas, MissionRecord, Shared};
pub use work::Work;

/// 已打开的工作区信息，供各组件访问文件时使用。
#[derive(Clone, PartialEq)]
pub struct WorkDirectory {
	pub display_path: String,
	pub folder_id: Option<String>,
	pub root_path: String,
	/// Android：真实路径模式下保留的 SAF 授权 URI（工作目录或其父目录的文档 URI，
	/// 供「打开文件位置」解析条目的 content:// URI；SAF 模式用 `folder_id`，桌面端为 None）。
	pub tree_uri: Option<String>,
}
