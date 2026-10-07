//! 工作区目录监视：外部修改后自动刷新资源管理器列表。
//!
//! 用户通过系统文件管理器等外部程序修改工作区内容后，前端资源管理器需要自动刷新。
//! 采用「自适应间隔轮询目录签名」方案（不用 inotify 等原生监视）：
//! - 签名 = 全部目录（相对路径 + 修改时间）哈希之和；目录的增删改名、目录内文件的
//!   增删改名都会更新其父目录 mtime，因此只看目录级元数据即可覆盖全部结构变化，
//!   不逐文件 stat——解压后的 APK（数万文件）也能廉价扫描；
//! - 间隔 = 上次扫描耗时 ×4，范围 3s~60s（大树自动降频，把轮询开销控制在约 1/4）；
//!   扫描失败（如目录暂时不可读）按 3s~60s 翻倍退避；
//! - 仅真实路径模式可用（SAF 目录经 ContentProvider 逐目录查询，轮询成本过高）。
//!
//! 前端在打开工作区且空闲时调用 [`start_workspace_watch`]，工作区切换/加载中/关闭时调
//! [`stop_workspace_watch`]；后端仅在签名变化时发送 `workspace-changed` 事件。
//! 文件内容（不改变目录结构）的修改不触发刷新——列表只展示目录结构。

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::Emitter;

/// 正在运行的工作区监视线程（同一时刻至多一个；重复启动会先停止上一个）。
static WORKSPACE_WATCH: Mutex<Option<WatchControl>> = Mutex::new(None);

/// 监视控制句柄：通知旧线程退出。
struct WatchControl {
    stop: Arc<AtomicBool>,
}

/// 开始监视工作区目录：后台线程轮询目录签名，变化时发送 `workspace-changed` 事件。
/// 重复调用安全（会先停止上一个监视）；目录不存在时静默忽略。
#[tauri::command]
pub fn start_workspace_watch(app: tauri::AppHandle, work_directory: String) -> Result<(), String> {
    stop_workspace_watch_inner();
    let root = PathBuf::from(&work_directory);
    if !root.is_dir() {
        return Ok(());
    }
    let stop = Arc::new(AtomicBool::new(false));
    {
        let mut guard = WORKSPACE_WATCH
            .lock()
            .map_err(|_| "工作区监视状态不可用".to_string())?;
        *guard = Some(WatchControl {
            stop: Arc::clone(&stop),
        });
    }
    std::thread::Builder::new()
        .name("workspace-watch".to_string())
        .spawn(move || watch_loop(app, root, stop))
        .map_err(|error| format!("启动工作区监视失败：{error}"))?;
    Ok(())
}

/// 停止工作区监视（工作区关闭/切换、加载中暂停时调用；无监视时静默成功）。
#[tauri::command]
pub fn stop_workspace_watch() {
    stop_workspace_watch_inner();
}

fn stop_workspace_watch_inner() {
    if let Ok(mut guard) = WORKSPACE_WATCH.lock() {
        if let Some(control) = guard.take() {
            control.stop.store(true, Ordering::Relaxed);
        }
    }
}

/// 轮询循环：按自适应间隔扫描目录签名，仅在签名变化时发送事件。
/// 线程启动时立即记录基线（对应前端刚加载完列表的状态，不发送事件），
/// 之后每个间隔扫描一次——启动到首次扫描之间的外部变化因此也会被感知。
fn watch_loop(app: tauri::AppHandle, root: PathBuf, stop: Arc<AtomicBool>) {
    let mut interval = Duration::from_secs(3);
    let mut last: Option<u64> = workspace_signature(&root).ok();
    loop {
        // 分片睡眠：停止请求最多 200ms 内生效，不必等完整间隔。
        let mut slept = Duration::ZERO;
        while slept < interval {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            let chunk = Duration::from_millis(200).min(interval - slept);
            std::thread::sleep(chunk);
            slept += chunk;
        }
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let started = Instant::now();
        let Ok(signature) = workspace_signature(&root) else {
            // 目录暂时不可读（被删除/占用中）：退避重试。
            interval = (interval * 2)
                .max(Duration::from_secs(3))
                .min(Duration::from_secs(60));
            continue;
        };
        let elapsed = started.elapsed();
        if matches!(last, Some(previous) if previous != signature) {
            // 应用自身操作也可能触发（此时前端处于加载中，会忽略该事件；
            // 加载结束后的首次扫描只建立新基线，不重复通知）。
            let _ = app.emit("workspace-changed", ());
        }
        last = Some(signature);
        interval = (elapsed * 4)
            .max(Duration::from_secs(3))
            .min(Duration::from_secs(60));
    }
}

/// 计算目录结构签名：递归枚举全部目录，把「相对路径 + 修改时间」逐项混入哈希。
/// 用 wrapping_add 合并子哈希，与遍历顺序无关（`read_dir` 顺序不稳定）；
/// 不跟随符号链接，避免目录环导致死循环。
fn workspace_signature(root: &Path) -> Result<u64, String> {
    let mut accumulator = 0_u64;
    let mut stack = vec![(root.to_path_buf(), String::new())];
    while let Some((directory, relative)) = stack.pop() {
        let metadata = std::fs::metadata(&directory).map_err(|error| error.to_string())?;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        relative.hash(&mut hasher);
        metadata.modified().ok().hash(&mut hasher);
        accumulator = accumulator.wrapping_add(hasher.finish());

        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            // 读不动的子目录跳过：其自身及父目录的 mtime 变化仍会被感知。
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_dir() {
                continue; // 文件无需 stat：文件增删改名由所属目录的 mtime 覆盖。
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let child = if relative.is_empty() {
                name
            } else {
                format!("{relative}/{name}")
            };
            stack.push((entry.path(), child));
        }
    }
    Ok(accumulator)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ageciv-watch-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn signature_tracks_directory_structure_changes() {
        let dir = temp_dir("signature");
        fs::write(dir.join("a.txt"), "a").unwrap();
        let base = workspace_signature(&dir).unwrap();

        // 同目录内改写文件内容：目录 mtime 不变，签名不变（列表结构未变化）。
        std::thread::sleep(Duration::from_millis(10));
        fs::write(dir.join("a.txt"), "a-updated-content").unwrap();
        assert_eq!(workspace_signature(&dir).unwrap(), base);

        // 新增文件 → 所属目录 mtime 更新，签名变化。
        std::thread::sleep(Duration::from_millis(10));
        fs::write(dir.join("b.txt"), "b").unwrap();
        let added = workspace_signature(&dir).unwrap();
        assert_ne!(added, base);

        // 新增子目录（含深层文件）→ 签名变化。
        std::thread::sleep(Duration::from_millis(10));
        fs::create_dir_all(dir.join("sub/deep")).unwrap();
        fs::write(dir.join("sub/deep/c.txt"), "c").unwrap();
        let nested = workspace_signature(&dir).unwrap();
        assert_ne!(nested, added);

        // 删除子目录 → 签名变化。
        std::thread::sleep(Duration::from_millis(10));
        fs::remove_dir_all(dir.join("sub")).unwrap();
        let removed = workspace_signature(&dir).unwrap();
        assert_ne!(removed, nested);

        // 目录不存在：报错（轮询循环据此退避重试）。
        assert!(workspace_signature(&dir.join("missing")).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
