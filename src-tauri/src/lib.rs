//! AgeCivModTool 的 Tauri 后端入口。
//!
//! 代码按职责拆分到以下模块：
//! - `commands`：暴露给前端的全部 Tauri 命令，按功能域细分为子模块；
//! - `mission_format`：国策文件的宽松语法修复、解析与序列化（含单元测试）；
//! - `models`：前后端共享的数据结构；
//! - `paths`：工作区路径校验与定位工具；
//! - `all_files_access`：Android 全盘文件访问插件（仅 Android 编译）。

mod commands;
mod mission_format;
mod models;
mod paths;

#[cfg(target_os = "android")]
mod all_files_access;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_scoped_storage::init());

    #[cfg(target_os = "android")]
    let builder = builder.plugin(all_files_access::init());

    builder
        .invoke_handler(tauri::generate_handler![
            commands::platform::greet,
            commands::platform::is_android,
            commands::platform::has_all_files_access,
            commands::platform::request_all_files_access,
            commands::scoped_storage::list_scoped_dir,
            commands::scoped_storage::read_scoped_files_in_dir,
            commands::scoped_storage::read_scoped_text_in_dir,
            commands::scoped_storage::write_scoped_text_in_dir,
            commands::scoped_storage::resolve_scoped_workspace_path,
            commands::scoped_storage::is_workspace_path_readable,
            commands::work_directory::validate_work_directory_name_command,
            commands::icons::list_focus_icons,
            commands::missions::parse_missions,
            commands::missions::serialize_missions,
            commands::icons::encode_mission_icon,
            commands::icons::encode_mission_icons,
            commands::missions::load_missions,
            commands::missions::save_missions,
            commands::missions::load_missions_file,
            commands::missions::save_missions_file,
            commands::icons::load_mission_icons,
            commands::events::load_mission_event,
            commands::events::save_mission_event,
            commands::events::delete_mission_event,
            commands::events::rename_mission_event,
            commands::workspace::copy_workspace_item,
            commands::workspace::move_workspace_item,
            commands::workspace::delete_workspace_item,
            commands::workspace::reveal_workspace_item,
            commands::icons::list_mission_icons,
            commands::work_directory::create_work_directory,
            commands::workspace::list_workspace_files,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
