//! Android SAF（scoped storage）文件操作桥。
//!
//! 统一封装 `tauri-plugin-android-fs` 的异步 API：全部走系统 SAF 授权目录
//! （`content://` tree URI），不再使用任何自研 Kotlin 文件命令。
//!
//! - `folder_id` 即目录树的 URI 字符串（由 `pick_folder` 的选择器返回）；
//! - 参数中的相对路径用 `/` 分隔，空串表示根目录；
//! - 桌面端本模块可以编译（插件为桩实现），但调用会返回 NOT_ANDROID 错误，
//!   前端只在 Android 上使用这些命令。

use std::io::Write;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::Serialize;
use tauri_plugin_android_fs::api::api_async::AndroidFs;
use tauri_plugin_android_fs::{AndroidFsExt, FileAccessMode, FsUri};

use crate::commands::workspace::{FileOpProgress, FileOpSink};
use crate::models::{FocusIcon, ScannedEntry};

type Api<'a, R> = &'a AndroidFs<R>;

/// 目录选择结果（与前端 `ScopedFolder` 对应，serde camelCase）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PickedFolder {
    /// 目录树的 URI 字符串（后续命令的 `folder_id`）。
    pub id: String,
    /// 显示名（获取失败或为空时为 None）。
    pub name: Option<String>,
}

/// 把 `folder_id` 还原为插件的目录 FsUri。
///
/// 目录选择器返回的是「树内文档 URI」（`…/tree/<treeId>/document/<treeId>`），
/// 但插件对目录类操作（readDir / createDirAll / resolve* 等）**要求同时携带
/// `documentTopTreeUri`**（`…/tree/<treeId>`）——缺失时插件内部会直接空指针崩溃
/// （release 构建下 R8 把 Kotlin 空检查改写成 `Object.getClass()` 调用，报
/// “Attempt to invoke virtual method … getClass() … on a null object reference”）。
///
/// 而 `folder_id` 是跨命令/持久化传递的单个字符串，只保存了文档 URI，
/// 因此这里统一从 URI 反推出目录树 URI；旧格式（folder_id 直接是目录树 URI）同时兼容。
pub fn root_uri(folder_id: &str) -> FsUri {
    let Some(tree_pos) = folder_id.find("/tree/") else {
        return FsUri::from_uri(folder_id);
    };
    let rest = &folder_id[tree_pos + "/tree/".len()..];
    let tree_id = rest.split('/').next().unwrap_or("");
    if tree_id.is_empty() {
        return FsUri::from_uri(folder_id);
    }
    let tree_uri = format!("{}/tree/{}", &folder_id[..tree_pos], tree_id);
    let document_uri = if rest[tree_id.len()..].starts_with("/document/") {
        folder_id.to_string()
    } else {
        format!("{tree_uri}/document/{tree_id}")
    };
    FsUri {
        uri: document_uri,
        document_top_tree_uri: Some(tree_uri),
    }
}

/// 相对路径规范化：去掉首尾的 `/`。
fn normalize(rel: &str) -> &str {
    rel.trim_matches('/')
}

/// 拼接相对路径（根目录用空串表示）。
fn join_rel(dir: &str, name: &str) -> String {
    let dir = dir.trim_matches('/');
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

/// 取相对路径的父目录（无父目录时返回空串）。
fn parent_of(rel: &str) -> &str {
    rel.trim_matches('/')
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or("")
}

/// 取相对路径的最后一段名称。
fn base_name(rel: &str) -> &str {
    rel.trim_end_matches('/')
        .rsplit_once('/')
        .map(|(_, name)| name)
        .unwrap_or(rel)
}

fn describe(error: impl std::fmt::Display, context: &str) -> String {
    format!("{context}：{error}")
}

/// 弹出系统目录选择器并持久化授权（应用重启后仍可读写该目录）。
pub async fn pick_folder<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<Option<PickedFolder>, String> {
    let api = app.android_fs_async();
    let Some(uri) = api
        .picker()
        .pick_dir(None, true)
        .await
        .map_err(|error| describe(error, "打开目录选择器失败"))?
    else {
        return Ok(None);
    };
    // 持久化失败仅影响重启后的可用性，不阻断本次会话。
    let _ = api.picker().persist_uri_permission(&uri).await;
    let name = api.get_name(&uri).await.ok().filter(|name| !name.is_empty());
    Ok(Some(PickedFolder { id: uri.uri, name }))
}

/// 解析相对根目录的目录 URI（空串=根）。
async fn resolve_dir<R: tauri::Runtime>(
    api: Api<'_, R>,
    root: &FsUri,
    rel: &str,
) -> Result<FsUri, String> {
    let rel = normalize(rel);
    if rel.is_empty() {
        return Ok(root.clone());
    }
    api.resolve_dir_uri(root, rel)
        .await
        .map_err(|error| describe(error, &format!("目录不存在（{rel}）")))
}

/// 解析 SAF 授权目录下已存在子目录的文档 URI（「打开文件位置」在真实路径模式下
/// 保留的父目录 URI 上解析新建工作目录，后续再以其为基准解析具体条目）。
pub async fn resolve_child_dir_uri<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    folder_id: &str,
    relative: &str,
) -> Result<String, String> {
    let api = app.android_fs_async();
    let root = root_uri(folder_id);
    let dir = resolve_dir(api, &root, relative).await?;
    Ok(dir.uri)
}

/// 确保相对目录存在（不存在则递归创建）。
async fn ensure_dir<R: tauri::Runtime>(
    api: Api<'_, R>,
    root: &FsUri,
    rel: &str,
) -> Result<(), String> {
    let rel = normalize(rel);
    if rel.is_empty() || api.resolve_dir_uri(root, rel).await.is_ok() {
        return Ok(());
    }
    api.create_dir_all(root, rel)
        .await
        .map(|_| ())
        .map_err(|error| describe(error, &format!("创建目录失败（{rel}）")))
}

/// 覆盖写入（截断）一个已存在的文件。
async fn write_truncate<R: tauri::Runtime>(
    api: Api<'_, R>,
    uri: &FsUri,
    contents: &str,
) -> Result<(), String> {
    let mut file = api
        .open_file(uri, FileAccessMode::WriteTruncate)
        .await
        .map_err(|error| describe(error, "打开文件失败"))?;
    file.write_all(contents.as_bytes())
        .map_err(|error| describe(error, "写入失败"))
}

/// 列出目录下一层条目（目录优先、名称不区分大小写排序）。
pub async fn list_dir<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    folder_id: &str,
    path: Option<String>,
) -> Result<Vec<ScannedEntry>, String> {
    let api = app.android_fs_async();
    let root = root_uri(folder_id);
    let relative = normalize(&path.unwrap_or_default()).to_string();
    let dir = resolve_dir(api, &root, &relative).await?;
    let entries = api
        .read_dir(&dir)
        .await
        .map_err(|error| describe(error, "读取目录失败"))?;
    let mut out: Vec<ScannedEntry> = entries
        .iter()
        .map(|entry| {
            let name = entry.name().to_string();
            ScannedEntry {
                path: join_rel(&relative, &name),
                is_dir: entry.is_dir(),
                name,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(out)
}

/// 批量读取目录内的图标：名称按 `{name}.png` 不区分大小写匹配，缺失项跳过，
/// 返回可直接用于 `<img src>` 的 base64 data URL。
pub async fn read_files_in_dir<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    folder_id: &str,
    dir_path: &str,
    names: Vec<String>,
) -> Result<Vec<FocusIcon>, String> {
    let api = app.android_fs_async();
    let root = root_uri(folder_id);
    let dir = resolve_dir(api, &root, dir_path).await?;

    // 期望的 {小写名}.png → 原始名（去重、保持请求顺序）。
    let mut wanted: Vec<(String, String)> = Vec::new();
    for name in names {
        if name.trim().is_empty() || name.contains('/') || name.contains('\\') {
            continue;
        }
        let key = format!("{}.png", name.to_lowercase());
        if !wanted.iter().any(|(existing, _)| existing == &key) {
            wanted.push((key, name));
        }
    }
    if wanted.is_empty() {
        return Ok(Vec::new());
    }

    let entries = api
        .read_dir(&dir)
        .await
        .map_err(|error| describe(error, "读取图标目录失败"))?;
    let mut out = Vec::new();
    for (key, original) in &wanted {
        let Some(entry) = entries
            .iter()
            .find(|entry| !entry.is_dir() && entry.name().to_lowercase() == *key)
        else {
            continue;
        };
        // 单个图标读取失败不影响整体。
        let Ok(bytes) = api.read(entry.uri()).await else {
            continue;
        };
        let mime = entry.file_mime_type().unwrap_or("image/png");
        out.push(FocusIcon {
            name: original.clone(),
            data_url: format!("data:{mime};base64,{}", STANDARD.encode(&bytes)),
        });
    }
    Ok(out)
}

/// 读取目录内单个文本文件。
pub async fn read_text_in_dir<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    folder_id: &str,
    dir_path: &str,
    file_name: &str,
) -> Result<String, String> {
    let api = app.android_fs_async();
    let root = root_uri(folder_id);
    let dir = resolve_dir(api, &root, dir_path).await?;
    let file = api
        .resolve_file_uri(&dir, file_name)
        .await
        .map_err(|error| describe(error, &format!("文件不存在（{file_name}）")))?;
    api.read_to_string(&file)
        .await
        .map_err(|error| describe(error, &format!("读取失败（{file_name}）")))
}

/// 批量读取目录内的文本文件（目录只解析一次；缺失/读取失败的文件跳过），
/// 返回（文件名，内容）列表。用于事件编辑器的文明对照表等成批小文件场景。
pub async fn read_text_files_in_dir<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    folder_id: &str,
    dir_path: &str,
    names: Vec<String>,
) -> Result<Vec<(String, String)>, String> {
    let api = app.android_fs_async();
    let root = root_uri(folder_id);
    let dir = resolve_dir(api, &root, dir_path).await?;
    let mut out = Vec::new();
    for name in names {
        if name.trim().is_empty() || name.contains('/') || name.contains('\\') {
            continue;
        }
        let Ok(file) = api.resolve_file_uri(&dir, &name).await else {
            continue;
        };
        if let Ok(text) = api.read_to_string(&file).await {
            out.push((name, text));
        }
    }
    Ok(out)
}

/// 写入（必要时创建目录与文件）目录内单个文本文件。
pub async fn write_text_in_dir<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    folder_id: &str,
    dir_path: &str,
    file_name: &str,
    contents: &str,
) -> Result<(), String> {
    let api = app.android_fs_async();
    let root = root_uri(folder_id);
    let dir = match resolve_dir(api, &root, dir_path).await {
        Ok(dir) => dir,
        Err(_) => {
            // 目录缺失：自动创建（历史行为：写入时按需建目录）。
            api.create_dir_all(&root, normalize(dir_path))
                .await
                .map_err(|error| describe(error, "创建目录失败"))?
        }
    };
    match api.resolve_file_uri(&dir, file_name).await {
        Ok(existing) => write_truncate(api, &existing, contents).await,
        Err(_) => {
            let created = api
                .create_new_file(&dir, file_name, Some("text/plain"))
                .await
                .map_err(|error| describe(error, &format!("创建文件失败（{file_name}）")))?;
            write_truncate(api, &created, contents).await
        }
    }
}

/// 读取工作区相对路径下的文本文件。
pub async fn read_text_file<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    folder_id: &str,
    path: &str,
) -> Result<String, String> {
    let api = app.android_fs_async();
    let root = root_uri(folder_id);
    let file = api
        .resolve_file_uri(&root, normalize(path))
        .await
        .map_err(|error| describe(error, &format!("文件不存在（{path}）")))?;
    api.read_to_string(&file)
        .await
        .map_err(|error| describe(error, "读取文件失败"))
}

/// 写入工作区相对路径下的文本文件（`recursive` 为 true 时自动创建父目录）。
pub async fn write_text_file<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    folder_id: &str,
    path: &str,
    contents: &str,
    recursive: bool,
) -> Result<(), String> {
    let api = app.android_fs_async();
    let root = root_uri(folder_id);
    let rel = normalize(path);
    if let Ok(existing) = api.resolve_file_uri(&root, rel).await {
        return write_truncate(api, &existing, contents).await;
    }
    if recursive {
        let parent = parent_of(rel);
        if !parent.is_empty() {
            ensure_dir(api, &root, parent).await?;
        }
    }
    let created = api
        .create_new_file(&root, rel, Some("text/plain"))
        .await
        .map_err(|error| describe(error, &format!("创建文件失败（{rel}）")))?;
    write_truncate(api, &created, contents).await
}

/// 创建目录（`recursive` 为 false 时要求父目录已存在）。
pub async fn create_dir<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    folder_id: &str,
    path: &str,
    recursive: bool,
) -> Result<(), String> {
    let api = app.android_fs_async();
    let root = root_uri(folder_id);
    let rel = normalize(path);
    if rel.is_empty() || api.resolve_dir_uri(&root, rel).await.is_ok() {
        return Ok(());
    }
    if recursive {
        api.create_dir_all(&root, rel)
            .await
            .map(|_| ())
            .map_err(|error| describe(error, &format!("创建目录失败（{rel}）")))
    } else {
        api.create_new_dir(&root, rel)
            .await
            .map(|_| ())
            .map_err(|error| describe(error, &format!("创建目录失败（{rel}）")))
    }
}

/// 删除工作区相对路径下的文件。
pub async fn remove_file<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    folder_id: &str,
    path: &str,
) -> Result<(), String> {
    let api = app.android_fs_async();
    let root = root_uri(folder_id);
    let file = api
        .resolve_file_uri(&root, normalize(path))
        .await
        .map_err(|error| describe(error, &format!("文件不存在（{path}）")))?;
    api.remove_file(&file)
        .await
        .map_err(|error| describe(error, "删除文件失败"))
}

/// 删除工作区相对路径下的目录（`recursive` 为 true 时连同内容删除）。
/// 递归删除按「后序」手工逐项删除（目录在内容清空后再删），并上报逐条目进度：
/// 插件自带的 `removeDirAll` 无进度且大目录会长时间无反馈。
pub async fn remove_dir<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    folder_id: &str,
    path: &str,
    recursive: bool,
) -> Result<(), String> {
    let api = app.android_fs_async();
    let root = root_uri(folder_id);
    let dir = resolve_dir(api, &root, path).await?;
    if !recursive {
        return api
            .remove_dir(&dir)
            .await
            .map_err(|error| describe(error, "删除目录失败"));
    }

    enum Task {
        Enter(FsUri),
        DeleteFile(FsUri),
        DeleteDir(FsUri),
    }
    let progress = FileOpProgress::new(app.clone(), "delete");
    progress.emit("正在删除文件...", 0, 0);
    let mut completed = 0_u64;
    let mut stack = vec![Task::Enter(dir)];
    while let Some(task) = stack.pop() {
        match task {
            Task::Enter(directory) => {
                // 先压入目录自身的删除（在内容之后执行），再逆序压入子项。
                stack.push(Task::DeleteDir(directory.clone()));
                let entries = api
                    .read_dir(&directory)
                    .await
                    .map_err(|error| describe(error, "读取目录失败"))?;
                for entry in entries.iter().rev() {
                    if entry.is_dir() {
                        stack.push(Task::Enter(entry.uri().clone()));
                    } else {
                        stack.push(Task::DeleteFile(entry.uri().clone()));
                    }
                }
            }
            Task::DeleteFile(uri) => {
                api.remove_file(&uri)
                    .await
                    .map_err(|error| describe(error, "删除文件失败"))?;
                completed += 1;
                progress.emit("正在删除文件...", completed, 0);
            }
            Task::DeleteDir(uri) => {
                api.remove_dir(&uri)
                    .await
                    .map_err(|error| describe(error, "删除目录失败"))?;
                completed += 1;
                progress.emit("正在删除文件...", completed, 0);
            }
        }
    }
    Ok(())
}

/// 递归复制（文件或目录）到目标相对路径（可跨 folder_id）。
/// 目标已存在同名文件时先删除再新建（避免残留旧内容）；同名目录按合并处理。
pub async fn copy_item<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    from_folder_id: &str,
    from_path: &str,
    to_folder_id: &str,
    to_path: &str,
) -> Result<(), String> {
    let api = app.android_fs_async();
    let from_root = root_uri(from_folder_id);
    let to_root = root_uri(to_folder_id);
    let from_rel = normalize(from_path).to_string();
    let to_rel = normalize(to_path).to_string();
    if from_rel.is_empty() || to_rel.is_empty() {
        return Err("复制源或目标路径无效".to_string());
    }

    // 迭代遍历（避免 async 递归）：队列元素为（源相对路径，目标相对路径）。
    let progress = FileOpProgress::new(app.clone(), "copy");
    progress.emit("正在复制文件...", 0, 0);
    let mut completed = 0_u64;
    let mut queue: Vec<(String, String)> = vec![(from_rel, to_rel)];
    while let Some((from_rel, to_rel)) = queue.pop() {
        if let Ok(source) = api.resolve_file_uri(&from_root, &from_rel).await {
            let parent = parent_of(&to_rel);
            if !parent.is_empty() {
                ensure_dir(api, &to_root, parent).await?;
            }
            let target = match api.create_new_file(&to_root, &to_rel, None).await {
                Ok(created) => created,
                Err(_) => {
                    // 同名文件已存在（createNewFile 遇重名会自动加序号，不能依赖）：
                    // 先删除旧文件再新建，保证内容完全替换。
                    if let Ok(previous) = api.resolve_file_uri(&to_root, &to_rel).await {
                        let _ = api.remove_file(&previous).await;
                    }
                    api.create_new_file(&to_root, &to_rel, None)
                        .await
                        .map_err(|error| {
                            describe(error, &format!("创建目标文件失败（{to_rel}）"))
                        })?
                }
            };
            api.copy(&source, &target)
                .await
                .map_err(|error| describe(error, &format!("复制失败（{to_rel}）")))?;
            completed += 1;
            progress.emit("正在复制文件...", completed, 0);
            continue;
        }

        let source_dir = resolve_dir(api, &from_root, &from_rel)
            .await
            .map_err(|_| format!("源不存在：{from_rel}"))?;
        ensure_dir(api, &to_root, &to_rel).await?;
        completed += 1;
        progress.emit("正在复制文件...", completed, 0);
        for entry in api
            .read_dir(&source_dir)
            .await
            .map_err(|error| describe(error, "读取源目录失败"))?
        {
            let name = entry.name().to_string();
            queue.push((join_rel(&from_rel, &name), join_rel(&to_rel, &name)));
        }
    }
    Ok(())
}

/// 移动/重命名文件或目录（可跨 folder_id）。
/// 同目录改名走单次 `rename`（最快）；跨目录走「复制 + 删除」。
pub async fn move_item<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    from_folder_id: &str,
    from_path: &str,
    to_folder_id: &str,
    to_path: &str,
) -> Result<(), String> {
    let api = app.android_fs_async();
    let from_rel = normalize(from_path).to_string();
    let to_rel = normalize(to_path).to_string();
    if from_rel.is_empty() || to_rel.is_empty() {
        return Err("移动源或目标路径无效".to_string());
    }
    let from_root = root_uri(from_folder_id);

    if from_folder_id == to_folder_id && parent_of(&from_rel) == parent_of(&to_rel) {
        let uri = match api.resolve_file_uri(&from_root, &from_rel).await {
            Ok(file) => file,
            Err(_) => resolve_dir(api, &from_root, &from_rel).await?,
        };
        api.rename(&uri, base_name(&to_rel))
            .await
            .map_err(|error| describe(error, "重命名失败"))?;
        return Ok(());
    }

    copy_item(app, from_folder_id, &from_rel, to_folder_id, &to_rel).await?;
    if let Ok(file) = api.resolve_file_uri(&from_root, &from_rel).await {
        api.remove_file(&file)
            .await
            .map_err(|error| describe(error, "删除源文件失败"))?;
    } else {
        // 跨目录移动的收尾删除：复用带进度的递归删除。
        remove_dir(app, from_folder_id, &from_rel, true).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 目录选择器返回的文档 URI：反推出目录树 URI，文档 URI 原样保留。
    #[test]
    fn derives_tree_uri_from_document_uri() {
        let folder =
            "content://com.android.externalstorage.documents/tree/primary%3AGameCivs/document/primary%3AGameCivs";
        let uri = root_uri(folder);
        assert_eq!(uri.uri, folder);
        assert_eq!(
            uri.document_top_tree_uri.as_deref(),
            Some("content://com.android.externalstorage.documents/tree/primary%3AGameCivs")
        );
    }

    /// 旧格式（folder_id 直接是目录树 URI）：补全为文档 URI + 目录树 URI。
    #[test]
    fn upgrades_legacy_tree_uri_to_document_uri() {
        let uri = root_uri("content://com.android.externalstorage.documents/tree/primary%3AGameCivs");
        assert_eq!(
            uri.uri,
            "content://com.android.externalstorage.documents/tree/primary%3AGameCivs/document/primary%3AGameCivs"
        );
        assert_eq!(
            uri.document_top_tree_uri.as_deref(),
            Some("content://com.android.externalstorage.documents/tree/primary%3AGameCivs")
        );
    }

    /// 非目录树 URI：原样返回、不附加目录树信息。
    #[test]
    fn leaves_non_tree_uris_unchanged() {
        let uri = root_uri("content://media/external/images/media/42");
        assert_eq!(uri.uri, "content://media/external/images/media/42");
        assert!(uri.document_top_tree_uri.is_none());
    }
}
