pub mod mind;
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
pub use mind::{FocusIcon, FocusTarget, MindMapCanvas, MissionRecord, Shared};
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
