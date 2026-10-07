//! Tauri 命令层：按功能域拆分的 `#[tauri::command]` 集合。
//!
//! 每个子模块的命令都会在 crate 根（`lib.rs`）的 `generate_handler!` 中注册；
//! 命令名与前端 `invoke("...")` 使用的字符串一一对应，重命名时需同步前端。

pub mod apk;
pub mod events;
pub mod icons;
pub mod missions;
pub mod platform;
pub mod scoped_storage;
pub mod work_directory;
pub mod workspace;
pub mod workspace_watch;
