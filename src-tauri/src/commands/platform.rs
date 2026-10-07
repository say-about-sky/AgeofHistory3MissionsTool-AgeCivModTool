//! 平台检测与全盘访问权限命令。

/// 演示用问候命令。
#[tauri::command]
pub fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

/// 当前是否运行在 Android 平台。
#[tauri::command]
pub fn is_android() -> bool {
    cfg!(target_os = "android")
}

/// 是否已获得 Android 全盘文件访问（All files access）权限。
#[tauri::command]
pub fn has_all_files_access<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> Result<bool, String> {
    #[cfg(target_os = "android")]
    {
        crate::all_files_access::has_access(app)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Ok(false)
    }
}

/// 请求 Android 全盘文件访问权限（会跳转系统设置页）。
#[tauri::command]
pub fn request_all_files_access<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
) -> Result<bool, String> {
    #[cfg(target_os = "android")]
    {
        crate::all_files_access::request_access(app)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Ok(false)
    }
}

/// 所选文件：系统选择器返回的 `content://` URI 与显示文件名。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PickedAndroidFile {
    pub uri: String,
    /// 显示文件名（部分提供方可能为空串）。
    pub name: String,
}

/// 打开 Android 系统文件选择器（SAF），返回所选文件的 `content://` URI 与显示名；
/// 用户取消时返回 None。`mimeTypes` 为空数组表示不过滤类型。用于「解压 APK 到工作区」
/// 与「添加默认密钥」等需要让用户浏览设备文件的场景（与「打开工作区」同一系统组件）。
///
/// 实现基于 `tauri-plugin-android-fs`（经实测的 SAF 封装）：`pick_file` 打开
/// `ACTION_OPEN_DOCUMENT` 系统文件选择器，返回的 URI 在本应用内可直接读取。
#[tauri::command]
pub async fn pick_android_file<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    mime_types: Vec<String>,
) -> Result<Option<PickedAndroidFile>, String> {
    #[cfg(target_os = "android")]
    {
        use tauri_plugin_android_fs::AndroidFsExt;
        let api = app.android_fs_async();
        let mime_refs: Vec<&str> = mime_types.iter().map(String::as_str).collect();
        let picked = api
            .picker()
            .pick_file(None, &mime_refs, true)
            .await
            .map_err(|error| format!("无法打开 Android 文件选择器：{error}"))?;
        let Some(uri) = picked else {
            // 用户取消：正常返回空结果。
            return Ok(None);
        };
        // 显示名缺失时用空串（前端会跳过扩展名校验，BKS 再由导入错误兜底弹密码框）。
        let name = api.get_name(&uri).await.unwrap_or_default();
        Ok(Some(PickedAndroidFile {
            uri: uri.uri,
            name,
        }))
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, mime_types);
        Ok(None)
    }
}
