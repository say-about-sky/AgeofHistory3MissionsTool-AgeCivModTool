//! Android「所有文件访问」插件封装。
//!
//! 通过 App 内置的 `AllFilesAccessPlugin` 调用系统 API：申请权限、快速列目录、
//! 按目录批量读写文件（绕过逐文件 ContentProvider 查询）。

use serde::{Deserialize, Serialize};
use tauri::{
    plugin::{Builder, PluginHandle, TauriPlugin},
    AppHandle, Manager, Runtime,
};

struct AndroidAccess<R: Runtime>(PluginHandle<R>);

#[derive(Deserialize)]
struct AccessStatus {
    granted: bool,
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("android-all-files-access")
        .setup(|app, api| {
            let handle =
                api.register_android_plugin("com.saysky.agecivmodtool", "AllFilesAccessPlugin")?;
            app.manage(AndroidAccess(handle));
            Ok(())
        })
        .build()
}

pub fn has_access<R: Runtime>(app: AppHandle<R>) -> Result<bool, String> {
    app.state::<AndroidAccess<R>>()
        .0
        .run_mobile_plugin::<AccessStatus>("hasAllFilesAccess", ())
        .map(|status| status.granted)
        .map_err(|error| error.to_string())
}

pub fn request_access<R: Runtime>(app: AppHandle<R>) -> Result<bool, String> {
    app.state::<AndroidAccess<R>>()
        .0
        .run_mobile_plugin::<AccessStatus>("requestAllFilesAccess", ())
        .map(|status| status.granted)
        .map_err(|error| error.to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ResolveFolderPathRequest {
    folder_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResolveFolderPathResponse {
    path: Option<String>,
}

/// 把 SAF 工作区解析为真实文件系统路径（仅 Android 11+ 且已授予所有文件访问时可用）。
/// 拿到路径后 Rust 侧可用 std::fs 直接多线程读写，绕过 ContentProvider 慢通道。
pub fn resolve_folder_path<R: Runtime>(
    app: AppHandle<R>,
    folder_id: String,
) -> Result<Option<String>, String> {
    app.state::<AndroidAccess<R>>()
        .0
        .run_mobile_plugin::<ResolveFolderPathResponse>(
            "resolveFolderPath",
            ResolveFolderPathRequest { folder_id },
        )
        .map(|response| response.path)
        .map_err(|error| error.to_string())
}
