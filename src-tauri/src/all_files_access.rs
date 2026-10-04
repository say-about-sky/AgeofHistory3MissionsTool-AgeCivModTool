use serde::{Deserialize, Serialize};
use tauri::{
    plugin::{Builder, PluginHandle, TauriPlugin},
    AppHandle, Manager, Runtime,
};

use crate::ScannedEntry;

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
struct ListDirRequest {
    folder_id: String,
    path: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListDirResponse {
    entries: Vec<ScannedEntry>,
}

pub fn list_dir<R: Runtime>(
    app: AppHandle<R>,
    folder_id: String,
    path: Option<String>,
) -> Result<Vec<ScannedEntry>, String> {
    app.state::<AndroidAccess<R>>()
        .0
        .run_mobile_plugin::<ListDirResponse>("listDir", ListDirRequest { folder_id, path })
        .map(|response| response.entries)
        .map_err(|error| error.to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadFilesInDirRequest {
    folder_id: String,
    dir_path: String,
    names: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadFilesInDirResponse {
    files: Vec<ReadFileEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadFileEntry {
    pub name: String,
    pub data_url: String,
}

pub fn read_files_in_dir<R: Runtime>(
    app: AppHandle<R>,
    folder_id: String,
    dir_path: String,
    names: Vec<String>,
) -> Result<Vec<ReadFileEntry>, String> {
    app.state::<AndroidAccess<R>>()
        .0
        .run_mobile_plugin::<ReadFilesInDirResponse>(
            "readFilesInDir",
            ReadFilesInDirRequest {
                folder_id,
                dir_path,
                names,
            },
        )
        .map(|response| response.files)
        .map_err(|error| error.to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReadTextInDirRequest {
    folder_id: String,
    dir_path: String,
    file_name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadTextInDirResponse {
    contents: String,
}

pub fn read_text_in_dir<R: Runtime>(
    app: AppHandle<R>,
    folder_id: String,
    dir_path: String,
    file_name: String,
) -> Result<String, String> {
    app.state::<AndroidAccess<R>>()
        .0
        .run_mobile_plugin::<ReadTextInDirResponse>(
            "readTextFileInDir",
            ReadTextInDirRequest {
                folder_id,
                dir_path,
                file_name,
            },
        )
        .map(|response| response.contents)
        .map_err(|error| error.to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WriteTextInDirRequest {
    folder_id: String,
    dir_path: String,
    file_name: String,
    contents: String,
}

pub fn write_text_in_dir<R: Runtime>(
    app: AppHandle<R>,
    folder_id: String,
    dir_path: String,
    file_name: String,
    contents: String,
) -> Result<(), String> {
    app.state::<AndroidAccess<R>>()
        .0
        .run_mobile_plugin::<()>(
            "writeTextFileInDir",
            WriteTextInDirRequest {
                folder_id,
                dir_path,
                file_name,
                contents,
            },
        )
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
