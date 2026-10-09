//! 工作区文件管理命令：列举、复制、移动、删除与在文件管理器中定位。
//!
//! 复制/移动/删除大目录可能耗时数秒到数分钟：命令统一为 async 并在
//! `spawn_blocking` 中执行（同步命令会阻塞主线程导致界面卡死），同时通过
//! `file-op-progress` 事件上报逐条目进度，前端在浮动进度卡中展示。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::Emitter;

use crate::paths::workspace_abs_path;

/// 工作区内的一个条目（文件或文件夹）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFile {
    name: String,
    relative_path: String,
    is_directory: bool,
}

/// 递归列举工作目录下的全部文件/文件夹（相对路径统一用 `/` 分隔）。
/// 大工作区（数万文件）的目录扫描同样在后台线程执行，避免卡住界面。
#[tauri::command(async)]
pub fn list_workspace_files(work_directory: String) -> Result<Vec<WorkspaceFile>, String> {
    let root = PathBuf::from(work_directory);
    if !root.is_dir() {
        return Err(format!("工作目录不存在：{}", root.display()));
    }
    let mut entries = Vec::new();
    append_workspace_entries(&root, &root, &mut entries)?;
    Ok(entries)
}

/// 递归收集目录下的条目。每层目录内「文件夹优先、其次文件」，
/// 同组内按名称不区分大小写排序（与 Android SAF 目录列表保持一致）。
fn append_workspace_entries(
    root: &Path,
    directory: &Path,
    entries: &mut Vec<WorkspaceFile>,
) -> Result<(), String> {
    // 先收集（路径, 是否文件夹, 名称）再排序：避免在比较函数里反复 stat。
    let mut children: Vec<(PathBuf, bool, String)> = fs::read_dir(directory)
        .map_err(|error| error.to_string())?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|path| {
            let is_directory = fs::symlink_metadata(&path)
                .map_err(|error| error.to_string())?
                .file_type()
                .is_dir();
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            Ok((path, is_directory, name))
        })
        .collect::<Result<Vec<_>, String>>()?;
    children.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| left.2.to_lowercase().cmp(&right.2.to_lowercase()))
            .then_with(|| left.2.cmp(&right.2))
    });

    for (path, is_directory, name) in children {
        let relative_path = path
            .strip_prefix(root)
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        entries.push(WorkspaceFile {
            name,
            relative_path,
            is_directory,
        });
        if is_directory {
            append_workspace_entries(root, &path, entries)?;
        }
    }
    Ok(())
}

/// 文件操作进度事件（前端监听 `file-op-progress`）：单位为「文件/目录条目数」，
/// `total = 0` 表示总量未知（SAF 模式不做预统计），前端改用不确定进度条。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FileOpProgressEvent {
    kind: &'static str,
    stage: &'static str,
    completed: u64,
    total: u64,
}

/// 进度接收端：复制/删除遍历过程中把进度报告给调用方。
/// 生产环境由 `FileOpProgress` 转成 Tauri 事件；测试中用记录型实现断言统计口径。
pub(crate) trait FileOpSink {
    fn emit(&self, stage: &'static str, completed: u64, total: u64);
}

/// 文件操作进度发送器：按时间节流（约 80ms 一条），避免事件洪水拖慢前端。
pub(crate) struct FileOpProgress<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
    kind: &'static str,
    last: Mutex<Instant>,
}

impl<R: tauri::Runtime> FileOpProgress<R> {
    pub(crate) fn new(app: tauri::AppHandle<R>, kind: &'static str) -> Self {
        Self {
            app,
            kind,
            last: Mutex::new(Instant::now() - Duration::from_millis(200)),
        }
    }
}

impl<R: tauri::Runtime> FileOpSink for FileOpProgress<R> {
    fn emit(&self, stage: &'static str, completed: u64, total: u64) {
        {
            let mut last = self.last.lock().unwrap();
            // total = 0 表示未知总量（SAF 模式）：只按时间节流。
            let due = (total > 0 && completed >= total)
                || last.elapsed() >= Duration::from_millis(80);
            if !due {
                return;
            }
            *last = Instant::now();
        }
        let _ = self.app.emit(
            "file-op-progress",
            FileOpProgressEvent {
                kind: self.kind,
                stage,
                completed,
                total,
            },
        );
    }
}

/// 统计目录树内的全部子条目数（文件+目录；符号链接按文件计入、不跟随）。
pub(crate) fn count_entries(path: &Path) -> u64 {
    let mut total = 0_u64;
    let mut pending = vec![path.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            total += 1;
            let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if metadata.file_type().is_dir() {
                pending.push(entry.path());
            }
        }
    }
    total
}

/// 递归复制（带进度）：`from` 为文件或目录，目标必须不存在。
pub(crate) fn copy_path(sink: &dyn FileOpSink, from: &Path, to: &Path) -> Result<(), String> {
    if !from.exists() {
        return Err(format!("源不存在：{}", from.display()));
    }
    if to.exists() {
        return Err(format!("目标已存在：{}", to.display()));
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    if from.is_dir() {
        // 先统计总量再复制，进度条为确定型（大目录统计只是一次廉价遍历）。
        let total = count_entries(from);
        sink.emit("正在复制文件...", 0, total);
        fs::create_dir_all(to).map_err(|error| error.to_string())?;
        let mut completed = 0_u64;
        copy_dir_contents(sink, from, to, &mut completed, total)?;
        sink.emit("正在复制文件...", completed, total);
        Ok(())
    } else {
        sink.emit("正在复制文件...", 0, 1);
        fs::copy(from, to).map_err(|error| format!("复制失败 {}：{error}", from.display()))?;
        sink.emit("正在复制文件...", 1, 1);
        Ok(())
    }
}

fn copy_dir_contents(
    sink: &dyn FileOpSink,
    from: &Path,
    to: &Path,
    completed: &mut u64,
    total: u64,
) -> Result<(), String> {
    let children = fs::read_dir(from)
        .map_err(|error| error.to_string())?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    for path in children {
        let name = path
            .file_name()
            .ok_or_else(|| format!("无效路径：{}", path.display()))?;
        let target = to.join(name);
        if path.is_dir() {
            fs::create_dir_all(&target).map_err(|error| error.to_string())?;
            *completed += 1;
            sink.emit("正在复制文件...", *completed, total);
            copy_dir_contents(sink, &path, &target, completed, total)?;
        } else {
            fs::copy(&path, &target)
                .map_err(|error| format!("复制失败 {}：{error}", path.display()))?;
            *completed += 1;
            sink.emit("正在复制文件...", *completed, total);
        }
    }
    Ok(())
}

/// 删除文件（Windows 只读文件先清除只读属性再重试）。
fn remove_file_robust(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(first) => {
            if let Ok(metadata) = fs::metadata(path) {
                let mut permissions = metadata.permissions();
                if permissions.readonly() {
                    #[allow(clippy::permissions_set_readonly_false)]
                    permissions.set_readonly(false);
                    let _ = fs::set_permissions(path, permissions);
                    return fs::remove_file(path);
                }
            }
            Err(first)
        }
    }
}

/// 递归删除（带进度）：`path` 为文件或目录。
pub(crate) fn delete_path(sink: &dyn FileOpSink, path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("目标不存在：{}（{error}）", path.display()))?;
    if metadata.file_type().is_dir() {
        let total = count_entries(path);
        sink.emit("正在删除文件...", 0, total);
        let mut completed = 0_u64;
        remove_dir_contents(sink, path, &mut completed, total)?;
        fs::remove_dir(path).map_err(|error| error.to_string())?;
        sink.emit("正在删除文件...", completed, total);
        Ok(())
    } else {
        sink.emit("正在删除文件...", 0, 1);
        remove_file_robust(path).map_err(|error| error.to_string())?;
        sink.emit("正在删除文件...", 1, 1);
        Ok(())
    }
}

fn remove_dir_contents(
    sink: &dyn FileOpSink,
    directory: &Path,
    completed: &mut u64,
    total: u64,
) -> Result<(), String> {
    let children = fs::read_dir(directory)
        .map_err(|error| error.to_string())?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    for path in children {
        let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
        if metadata.file_type().is_dir() {
            // 目录最后删除：先清空内容再删目录本身。
            remove_dir_contents(sink, &path, completed, total)?;
            fs::remove_dir(&path).map_err(|error| error.to_string())?;
        } else {
            remove_file_robust(&path).map_err(|error| error.to_string())?;
        }
        *completed += 1;
        sink.emit("正在删除文件...", *completed, total);
    }
    Ok(())
}

/// 移动（带进度）：优先单次 `rename`（同卷瞬时完成）；失败（跨卷等）退回「复制 + 删除」。
/// 目标已存在时报错（大小写改名视为同一目标放行）。
pub(crate) fn move_path(sink: &dyn FileOpSink, from: &Path, to: &Path) -> Result<(), String> {
    if !from.exists() {
        return Err(format!("源不存在：{}", from.display()));
    }
    if to.exists() {
        let same_file = from
            .canonicalize()
            .ok()
            .is_some_and(|source| to.canonicalize().ok() == Some(source));
        if !same_file {
            return Err(format!("目标已存在：{}", to.display()));
        }
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    if fs::rename(from, to).is_ok() {
        // 重命名瞬时完成：无需进度事件，命令返回即结束。
        return Ok(());
    }
    copy_path(sink, from, to)?;
    delete_path(sink, from)
}

/// 复制工作区内的文件/文件夹到新位置（目标已存在时报错；后台线程执行并上报进度）。
#[tauri::command]
pub async fn copy_workspace_item<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    work_directory: String,
    source: String,
    target: String,
) -> Result<(), String> {
    let from = workspace_abs_path(&work_directory, &source)?;
    let to = workspace_abs_path(&work_directory, &target)?;
    let progress = FileOpProgress::new(app, "copy");
    tauri::async_runtime::spawn_blocking(move || copy_path(&progress, &from, &to))
        .await
        .map_err(|error| format!("复制任务失败：{error}"))?
}

/// 移动工作区内的文件/文件夹到新位置（后台线程执行并上报进度）。
#[tauri::command]
pub async fn move_workspace_item<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    work_directory: String,
    source: String,
    target: String,
) -> Result<(), String> {
    let from = workspace_abs_path(&work_directory, &source)?;
    let to = workspace_abs_path(&work_directory, &target)?;
    let progress = FileOpProgress::new(app, "move");
    tauri::async_runtime::spawn_blocking(move || move_path(&progress, &from, &to))
        .await
        .map_err(|error| format!("移动任务失败：{error}"))?
}

/// 删除工作区内的文件/文件夹（文件夹递归删除；后台线程执行并上报进度）。
#[tauri::command]
pub async fn delete_workspace_item<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    work_directory: String,
    target: String,
) -> Result<(), String> {
    let path = workspace_abs_path(&work_directory, &target)?;
    let progress = FileOpProgress::new(app, "delete");
    tauri::async_runtime::spawn_blocking(move || delete_path(&progress, &path))
        .await
        .map_err(|error| format!("删除任务失败：{error}"))?
}

/// 打开文件位置：桌面端在系统文件管理器中选中条目（Windows 经 SHOpenFolderAndSelectItems，
/// 带空格/中文的路径也能正确高亮）；Android 端通过目录树 URI 解析出条目的 content:// URI，
/// 再弹出「用哪个应用打开」选择器（文件用 VIEW、目录用目录视图）。
/// `tree_uri` 为 Android 工作目录（或其父目录）的 SAF 授权 URI；桌面端忽略。
#[tauri::command]
pub async fn reveal_workspace_item<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    work_directory: String,
    target: String,
    tree_uri: Option<String>,
    is_directory: bool,
) -> Result<(), String> {
    #[cfg(not(target_os = "android"))]
    {
        let _ = (tree_uri, is_directory);
        let path = workspace_abs_path(&work_directory, &target)?;
        if !path.exists() {
            return Err(format!("目标不存在：{}", path.display()));
        }
        use tauri_plugin_opener::OpenerExt;
        app.opener()
            .reveal_item_in_dir(&path)
            .map_err(|error| format!("无法定位该条目：{error}"))
    }
    #[cfg(target_os = "android")]
    {
        let _ = work_directory;
        use tauri_plugin_android_fs::AndroidFsExt;
        let tree = tree_uri
            .filter(|tree| !tree.is_empty())
            .ok_or_else(|| "缺少目录访问授权：请通过「打开工作区」重新选择该目录后再试".to_string())?;
        let api = app.android_fs_async();
        let dir = crate::android_fs_bridge::root_uri(&tree);
        let uri = if is_directory {
            api.resolve_dir_uri(&dir, &target).await
        } else {
            api.resolve_file_uri(&dir, &target).await
        }
        .map_err(|error| format!("无法定位所选条目（可能已被移动或删除）：{error}"))?;
        let opener = api.opener();
        let result = if is_directory {
            opener.open_dir(&uri).await
        } else {
            opener.open_file(&uri).await
        };
        result.map_err(|error| format!("系统无法打开该条目：{error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ageciv-workspace-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 每层目录：文件夹优先、其次文件（同组按名称不区分大小写），
    /// 且子目录内容紧跟父目录行（前序 DFS，资源管理器按此顺序渲染）。
    #[test]
    fn workspace_listing_puts_directories_before_files() {
        let dir = temp_dir("order");
        fs::write(dir.join("aardvark.json"), "{}").unwrap();
        fs::write(dir.join("Apple.json"), "{}").unwrap();
        fs::create_dir_all(dir.join("missionsEvents")).unwrap();
        fs::write(dir.join("missionsEvents").join("zz.txt"), "z").unwrap();
        fs::write(dir.join("missionsEvents").join("aa.txt"), "a").unwrap();
        fs::create_dir_all(dir.join("missionsImages")).unwrap();

        let files = list_workspace_files(dir.to_string_lossy().into_owned()).unwrap();
        let order: Vec<String> = files
            .iter()
            .map(|file| file.relative_path.clone())
            .collect();
        assert_eq!(
            order,
            vec![
                "missionsEvents",
                "missionsEvents/aa.txt",
                "missionsEvents/zz.txt",
                "missionsImages",
                "aardvark.json",
                "Apple.json",
            ]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// 记录进度事件（不含节流），验证统计口径闭合。
    #[derive(Default)]
    struct RecordingSink {
        events: Mutex<Vec<(u64, u64)>>,
    }

    impl FileOpSink for RecordingSink {
        fn emit(&self, _stage: &'static str, completed: u64, total: u64) {
            self.events.lock().unwrap().push((completed, total));
        }
    }

    impl RecordingSink {
        fn last(&self) -> (u64, u64) {
            *self.events.lock().unwrap().last().unwrap()
        }
    }

    /// 复制与删除的进度总量口径一致：结尾事件都是「(总量, 总量)」，
    /// 且删除完成后目录彻底清空、复制内容逐字节一致。
    #[test]
    fn copy_and_delete_report_matching_totals() {
        let dir = temp_dir("progress");
        let source = dir.join("source");
        fs::create_dir_all(source.join("nested/deeper")).unwrap();
        fs::write(source.join("a.txt"), "alpha").unwrap();
        fs::write(source.join("nested/b.txt"), "beta").unwrap();
        fs::write(source.join("nested/deeper/c.txt"), "gamma").unwrap();
        // 条目数：nested、nested/b.txt、nested/deeper、nested/deeper/c.txt、a.txt = 5。
        assert_eq!(count_entries(&source), 5);

        let target = dir.join("target");
        let sink = RecordingSink::default();
        copy_path(&sink, &source, &target).unwrap();
        assert_eq!(sink.last(), (5, 5));
        assert_eq!(fs::read_to_string(target.join("nested/deeper/c.txt")).unwrap(), "gamma");

        // 目标已存在：报错而不是静默覆盖。
        assert!(copy_path(&sink, &source, &target).is_err());

        let delete_sink = RecordingSink::default();
        delete_path(&delete_sink, &target).unwrap();
        assert_eq!(delete_sink.last(), (5, 5));
        assert!(!target.exists());

        // 文件级：单个文件一次事件。
        let single_sink = RecordingSink::default();
        delete_path(&single_sink, &source.join("a.txt")).unwrap();
        assert_eq!(single_sink.last(), (1, 1));

        let _ = fs::remove_dir_all(&dir);
    }

    /// 移动的「快速路径」：同卷改名瞬时完成且不产生进度事件；
    /// 目标已存在（且非大小写改名）时报错。
    #[test]
    fn move_renames_in_place_and_rejects_existing_target() {
        let dir = temp_dir("move");
        fs::write(dir.join("from.txt"), "x").unwrap();
        fs::write(dir.join("other.txt"), "y").unwrap();

        let sink = RecordingSink::default();
        assert!(move_path(&sink, &dir.join("from.txt"), &dir.join("other.txt")).is_err());

        move_path(&sink, &dir.join("from.txt"), &dir.join("to.txt")).unwrap();
        assert!(dir.join("to.txt").exists());
        assert!(!dir.join("from.txt").exists());
        assert!(sink.events.lock().unwrap().is_empty());

        let _ = fs::remove_dir_all(&dir);
    }
}
