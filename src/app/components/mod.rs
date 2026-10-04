pub mod mind;
pub mod work;
pub mod files;
pub mod frame;
pub mod event;
pub mod event_grid;
pub mod event_parser;
pub mod event_schema;
pub mod game_text;
pub use mind::{FocusIcon, FocusTarget, MindMapCanvas, MissionRecord};
pub use work::Work;

/// 已打开的工作区信息，供各组件访问文件时使用。
#[derive(Clone, PartialEq)]
pub struct WorkDirectory {
	pub display_path: String,
	pub folder_id: Option<String>,
	pub root_path: String,
}
