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
