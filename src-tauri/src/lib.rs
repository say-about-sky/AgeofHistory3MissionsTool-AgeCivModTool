//! AgeCivModTool 的 Tauri 后端入口。
//!
//! 代码按职责拆分到以下模块：
//! - `commands`：暴露给前端的全部 Tauri 命令，按功能域细分为子模块；
//! - `mission_format`：国策文件的宽松语法修复、解析与序列化（含单元测试）；
//! - `apk_extract`：APK 宽容解压（忽略不合规的 extra field；桌面与安卓共用，
//!   安卓端通过 android-fs 插件取得文件句柄后共享同一套多线程解压器）；
//! - `apk_signing`：APK v1+v2+v3 签名（纯 Rust，规范与 apksig 对齐）；
//! - `bks`：BKS（BouncyCastle）密钥库解析，用于导入 .bks 签名密钥；
//! - `models`：前后端共享的数据结构；
//! - `paths`：工作区路径校验与定位工具；
//! - `android_fs_bridge`：Android SAF 文件操作桥（tauri-plugin-android-fs 封装）；
//! - `all_files_access`：Android 全盘文件访问权限与 SAF 真实路径解析（仅 Android 编译）。

mod apk_extract;
mod apk_pack;
mod apk_signing;
mod apk_update;
mod android_fs_bridge;
mod bks;
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
        // Android SAF 文件访问（选择器/读写/目录操作；非 Android 为桩实现，
        // API 调用返回 NOT_ANDROID）。
        .plugin(tauri_plugin_android_fs::init());

    #[cfg(target_os = "android")]
    let builder = builder.plugin(all_files_access::init());

    builder
        .invoke_handler(tauri::generate_handler![
            commands::platform::greet,
            commands::platform::is_android,
            commands::platform::has_all_files_access,
            commands::platform::request_all_files_access,
            commands::platform::pick_android_file,
            commands::scoped_storage::pick_android_folder,
            commands::scoped_storage::list_scoped_dir,
            commands::scoped_storage::read_scoped_files_in_dir,
            commands::scoped_storage::read_scoped_text_in_dir,
            commands::scoped_storage::write_scoped_text_in_dir,
            commands::scoped_storage::read_scoped_text_file,
            commands::scoped_storage::write_scoped_text_file,
            commands::scoped_storage::mkdir_scoped_dir,
            commands::scoped_storage::remove_scoped_file,
            commands::scoped_storage::remove_scoped_dir,
            commands::scoped_storage::copy_scoped_item,
            commands::scoped_storage::move_scoped_item,
            commands::scoped_storage::resolve_scoped_workspace_path,
            commands::scoped_storage::is_workspace_path_readable,
            commands::scoped_storage::resolve_scoped_child_dir_uri,
            commands::apk::extract_apk_to_workspace,
            commands::apk::import_apk_sections,
            commands::apk::export_apk_sections,
            commands::apk::package_workspace_as_apk,
            commands::apk::sign_apk_file,
            commands::apk::import_signing_key,
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
            commands::missions::find_unused_event_scripts,
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
            commands::workspace_watch::start_workspace_watch,
            commands::workspace_watch::stop_workspace_watch,
            commands::missions_db::load_event_lookup,
            commands::missions_db::load_event_lookup_scoped,
            commands::missions_db::set_lookup_source_apk,
            commands::missions_db::set_lookup_source_apk_scoped,
            commands::missions_db::list_event_assets,
            commands::missions_db::list_event_assets_scoped,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
