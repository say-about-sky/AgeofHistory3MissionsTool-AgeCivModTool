//! APK 操作命令：解压 APK 到工作区、把工作区打包为 APK 并尝试签名。
//!
//! 解压/打包/签名均为耗时 IO 操作：命令统一为 async 并在 `spawn_blocking`
//! 中执行，同时通过 `apk-progress` 事件上报细粒度进度，避免阻塞 UI 线程。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::Emitter;

use crate::apk_signing::ApkSigningKey;

/// APK 操作结果（返回给前端展示）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApkOperationSummary {
    /// 面向用户的提示文本。
    pub message: String,
    /// 涉及的文件数量（解压或打包）。
    pub entries: u64,
    /// 是否成功签名（仅打包时有意义）。
    pub signed: bool,
    /// 产物路径（仅打包时有值）。
    pub output_path: Option<String>,
}

/// APK 操作进度事件（前端监听 `apk-progress`）：单位为 `files` 或 `bytes`。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ApkProgressEvent {
    kind: &'static str,
    stage: &'static str,
    unit: &'static str,
    completed: u64,
    total: u64,
}

/// 进度事件发送器：按时间节流（约 80ms 一条），避免事件洪水拖慢前端。
/// `pub(crate)`：供其他命令复用（如 missions 的未使用脚本扫描）。
pub(crate) struct ProgressEmitter<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
    kind: &'static str,
    last: Mutex<Instant>,
}

impl<R: tauri::Runtime> ProgressEmitter<R> {
    pub(crate) fn new(app: tauri::AppHandle<R>, kind: &'static str) -> Self {
        Self {
            app,
            kind,
            last: Mutex::new(Instant::now() - Duration::from_millis(200)),
        }
    }

    pub(crate) fn emit(&self, stage: &'static str, unit: &'static str, completed: u64, total: u64) {
        {
            let mut last = self.last.lock().unwrap();
            let due = completed >= total || last.elapsed() >= Duration::from_millis(80);
            if !due {
                return;
            }
            *last = Instant::now();
        }
        let _ = self.app.emit(
            "apk-progress",
            ApkProgressEvent {
                kind: self.kind,
                stage,
                unit,
                completed,
                total,
            },
        );
    }
}

/// 解压 APK 到工作区下的 `<APK 名称>` 子目录（覆盖同名文件），在后台线程执行。
///
/// Android 端 `apk_path` 为系统文件选择器返回的 `content://` URI：交给 android-fs 插件
/// 取出真实文件句柄后，与桌面端共用同一套多线程宽容解压器（定位读取共享句柄），
/// 进度统一由本命令通过 `apk-progress` 事件上报（单位：files）。
/// `apk_name` 为系统选择器提供的显示文件名（Android content:// URI 无法直接解析
/// 文件名，解压目录命名优先使用它；桌面端为 None）。
#[tauri::command]
pub async fn extract_apk_to_workspace<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    work_directory: String,
    apk_path: String,
    apk_name: Option<String>,
) -> Result<ApkOperationSummary, String> {
    let work_directory = PathBuf::from(&work_directory);
    if !work_directory.is_dir() {
        return Err(format!("工作区目录不存在：{}", work_directory.display()));
    }
    let destination =
        apk_extract_destination(&work_directory, apk_name.as_deref(), Path::new(&apk_path))?;
    fs::create_dir_all(&destination)
        .map_err(|error| format!("创建解压目录失败 {}：{error}", destination.display()))?;
    #[cfg(target_os = "android")]
    {
        // .nomedia：让 Android 媒体库跳过整棵解压树的逐文件索引登记（新文件创建时
        // MediaProvider 的登记工作是建文件开销的主要来源），同时避免游戏资源被
        // 相册/音乐库收录。失败不影响解压。
        let _ = fs::write(destination.join(".nomedia"), []);
    }
    let destination_name = destination
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let destination_string = destination.to_string_lossy().into_owned();

    let outcome = tauri::async_runtime::spawn_blocking(move || {
        #[cfg(target_os = "android")]
        {
            // Android：APK 可能来自系统选择器的 content:// URI，先通过 android-fs
            // 插件取出真实文件句柄，再用同一套多线程宽容解压器（定位读取共享句柄）。
            use tauri_plugin_android_fs::{AndroidFsExt, FsUri};
            let uri = if apk_path.contains("://") {
                FsUri::from_uri(apk_path.clone())
            } else {
                FsUri::from_path(Path::new(&apk_path))
            };
            let file = app
                .android_fs()
                .open_file_readable(&uri)
                .map_err(|error| {
                    format!("打开 APK 失败（若 APK 位于云盘等虚拟目录，请先下载到设备本地）：{error}")
                })?;
            let emitter = ProgressEmitter::new(app, "extract");
            crate::apk_extract::extract_from_file(&file, &destination, &|completed, total| {
                emitter.emit("正在解压 APK 到工作区", "files", completed, total)
            })
        }
        #[cfg(not(target_os = "android"))]
        {
            let emitter = ProgressEmitter::new(app, "extract");
            crate::apk_extract::extract_apk_contents_with_progress(
                Path::new(&apk_path),
                &destination,
                &|completed, total| {
                    emitter.emit("正在解压 APK 到工作区", "files", completed, total)
                },
            )
        }
    })
    .await
    .map_err(|error| format!("解压任务失败：{error}"))??;
    let entries = outcome.entries;
    let skipped = outcome.skipped;

    let skip_note = if skipped > 0 {
        format!("（{skipped} 个条目无法处理，已跳过）")
    } else {
        String::new()
    };
    Ok(ApkOperationSummary {
        message: format!("已解压 {entries} 个文件到「{destination_name}/」{skip_note}"),
        entries,
        signed: false,
        output_path: Some(destination_string),
    })
}

/// 「从 apk 中导入」：解压全局国策（`assets/game/missions/`）与全部剧本目录
/// （`assets/map/<地图>/scenarios/<剧本>/`——地图 / 剧本目录名按 APK 实际结构
/// 自动识别，适配各模组自定义命名）到工作区（目录保留完整路径，落点与整体解压
/// 一致：`<工作区>/<APK 名称>/`），其余条目静默跳过；进度经 `apk-progress` 事件上报（单位：files）。
#[tauri::command]
pub async fn import_apk_sections<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    work_directory: String,
    apk_path: String,
    apk_name: Option<String>,
) -> Result<ApkOperationSummary, String> {
    let work_directory = PathBuf::from(&work_directory);
    if !work_directory.is_dir() {
        return Err(format!("工作区目录不存在：{}", work_directory.display()));
    }
    let destination =
        apk_extract_destination(&work_directory, apk_name.as_deref(), Path::new(&apk_path))?;
    fs::create_dir_all(&destination)
        .map_err(|error| format!("创建导入目录失败 {}：{error}", destination.display()))?;
    // 记录源 APK 位置：事件编辑器补全在该工作区缺少游戏数据文件时直接从源 APK 读取
    // （路径或 `content://` URI；打包时该标记文件会被跳过）。
    let _ = fs::write(
        destination.join(crate::apk_pack::SOURCE_APK_MARKER),
        &apk_path,
    );
    #[cfg(target_os = "android")]
    {
        // 与整体解压一致：写入 .nomedia 避免媒体库逐文件登记。
        let _ = fs::write(destination.join(".nomedia"), []);
    }
    let destination_name = destination
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let destination_string = destination.to_string_lossy().into_owned();

    let outcome = tauri::async_runtime::spawn_blocking(move || {
        #[cfg(target_os = "android")]
        let file = {
            use tauri_plugin_android_fs::{AndroidFsExt, FsUri};
            let uri = if apk_path.contains("://") {
                FsUri::from_uri(apk_path.clone())
            } else {
                FsUri::from_path(Path::new(&apk_path))
            };
            app.android_fs()
                .open_file_readable(&uri)
                .map_err(|error| {
                    format!("打开 APK 失败（若 APK 位于云盘等虚拟目录，请先下载到设备本地）：{error}")
                })?
        };
        #[cfg(not(target_os = "android"))]
        let file = std::fs::File::open(Path::new(&apk_path))
            .map_err(|error| format!("打开 APK 失败 {apk_path}：{error}"))?;

        // 版块前缀按 APK 实际目录结构识别（自适应各模组自定义的地图 / 剧本目录名）。
        let prefixes = crate::apk_extract::discover_section_prefixes(&file)?;
        let prefix_refs: Vec<&str> = prefixes.iter().map(String::as_str).collect();
        let emitter = ProgressEmitter::new(app, "import");
        crate::apk_extract::extract_selected_from_file(
            &file,
            &destination,
            &prefix_refs,
            &|completed, total| {
                emitter.emit("正在从 APK 导入 missions/scenarios", "files", completed, total)
            },
        )
    })
    .await
    .map_err(|error| format!("导入任务失败：{error}"))??;
    let skip_note = if outcome.skipped > 0 {
        format!("（{} 个条目无法处理，已跳过）", outcome.skipped)
    } else {
        String::new()
    };
    Ok(ApkOperationSummary {
        message: format!(
            "已从 APK 导入 {} 个版块文件到「{destination_name}/」{skip_note}",
            outcome.entries
        ),
        entries: outcome.entries,
        signed: false,
        output_path: Some(destination_string),
    })
}

/// 「导出到 apk」：把工作区 `<APK 名称>/` 下的全局国策与各剧本版块
/// （`assets/game/missions/` 与 `assets/map/<地图>/scenarios/<剧本>/`，目录名按
/// 工作区实际结构自动识别）以更新替换式覆盖写回 APK——工作区存在、APK 中没有的
/// 文件追加为新增条目，其余原条目保留不变；进度经 `apk-progress` 事件上报（单位：files）。
#[tauri::command]
pub async fn export_apk_sections<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    work_directory: String,
    apk_path: String,
    apk_name: Option<String>,
) -> Result<ApkOperationSummary, String> {
    let work_directory = PathBuf::from(&work_directory);
    if !work_directory.is_dir() {
        return Err(format!("工作区目录不存在：{}", work_directory.display()));
    }
    let source_root =
        apk_extract_destination(&work_directory, apk_name.as_deref(), Path::new(&apk_path))?;
    if !source_root.is_dir() {
        let folder = source_root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        return Err(format!(
            "工作区中未找到「{folder}/」目录：请先对目标 APK 执行「从 apk 中导入」（或整体解压）后再导出"
        ));
    }
    let apk_display = apk_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| apk_path.clone());
    let apk_location = apk_path.clone();

    let outcome = tauri::async_runtime::spawn_blocking(move || {
        // 版块前缀按工作区实际目录识别（自适应各模组自定义的地图 / 剧本目录名）。
        let prefixes = crate::apk_update::discover_workspace_section_prefixes(&source_root);
        let prefix_refs: Vec<&str> = prefixes.iter().map(String::as_str).collect();
        #[cfg(target_os = "android")]
        {
            use tauri_plugin_android_fs::{AndroidFsExt, FileAccessMode, FsUri};
            let uri = if apk_path.contains("://") {
                FsUri::from_uri(apk_path.clone())
            } else {
                FsUri::from_path(Path::new(&apk_path))
            };
            let mut file = app
                .android_fs()
                .open_file(&uri, FileAccessMode::ReadWrite)
                .map_err(|error| {
                    format!(
                        "无法写入所选 APK：请将 APK 复制到工作区，然后在资源管理器中选中它再试（{error}）"
                    )
                })?;
            let emitter = ProgressEmitter::new(app, "export");
            crate::apk_update::update_apk_sections(
                &mut file,
                &source_root,
                &prefix_refs,
                &|completed, total| {
                    emitter.emit("正在导出到 APK（更新替换）", "files", completed, total)
                },
            )
        }
        #[cfg(not(target_os = "android"))]
        {
            let mut file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(Path::new(&apk_path))
                .map_err(|error| {
                    format!(
                        "无法写入所选 APK（请确认文件可写，或在资源管理器中选中工作区内的 APK）：{error}"
                    )
                })?;
            let emitter = ProgressEmitter::new(app, "export");
            crate::apk_update::update_apk_sections(
                &mut file,
                &source_root,
                &prefix_refs,
                &|completed, total| {
                    emitter.emit("正在导出到 APK（更新替换）", "files", completed, total)
                },
            )
        }
    })
    .await
    .map_err(|error| format!("导出任务失败：{error}"))??;

    Ok(ApkOperationSummary {
        message: format!(
            "已导出到 APK：替换 {} 个、新增 {} 个文件，其余 {} 个条目保持不变（{apk_display}）",
            outcome.replaced, outcome.added, outcome.kept
        ),
        entries: outcome.replaced + outcome.added,
        signed: false,
        output_path: Some(apk_location),
    })
}

/// 把工作区内的目录打包为 APK（不签名）。
/// `source_directory` 为工作区内的相对路径（空串表示工作区根）；
/// 输出到源目录同级：`<源目录名>-repacked.apk`。
/// 打包在后台线程执行，并通过 `apk-progress` 事件上报字节级进度。
#[tauri::command]
pub async fn package_workspace_as_apk<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    work_directory: String,
    source_directory: String,
) -> Result<ApkOperationSummary, String> {
    let work_directory = PathBuf::from(&work_directory);
    if !work_directory.is_dir() {
        return Err(format!("工作区目录不存在：{}", work_directory.display()));
    }
    let source_directory = source_directory.trim();
    let source = if source_directory.is_empty() {
        work_directory.clone()
    } else {
        crate::paths::validate_relative_path(source_directory)?;
        let source = work_directory.join(source_directory);
        if !source.is_dir() {
            return Err(format!("打包目录不存在：{}", source.display()));
        }
        source
    };
    let output = package_output_path(&source)?;

    let (output, entries) = tauri::async_runtime::spawn_blocking({
        let app = app.clone();
        let source = source.clone();
        let output = output.clone();
        move || -> Result<(PathBuf, u64), String> {
            let emitter = ProgressEmitter::new(app, "package");
            let entries = crate::apk_pack::package_workspace(
                &source,
                &output,
                &|completed, total| emitter.emit("正在打包 APK", "bytes", completed, total),
            )?;
            Ok((output, entries))
        }
    })
    .await
    .map_err(|error| format!("打包任务失败：{error}"))??;

    Ok(ApkOperationSummary {
        message: format!(
            "已打包 {entries} 个文件（未签名）：{}（可在资源管理器中选中该 APK 后使用「签名 apk」）",
            output.display()
        ),
        entries,
        signed: false,
        output_path: Some(output.to_string_lossy().into_owned()),
    })
}

/// 对选中的 APK 就地执行 v1+v2+v3 签名。
/// `relative_path` 为工作区内相对路径（资源管理器选中 / 右键菜单），
/// 或桌面端文件选择器返回的绝对路径（外部 APK）；Android 的 `content://` URI
/// 无法就地签名，会给出复制到工作区的指引。
/// 密钥查找顺序：APK 所在目录的 `signing.pem` → 工作区根 `signing.pem` → 内置测试密钥。
#[tauri::command]
pub async fn sign_apk_file<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    work_directory: String,
    relative_path: String,
) -> Result<ApkOperationSummary, String> {
    let work_directory = PathBuf::from(&work_directory);
    if !work_directory.is_dir() {
        return Err(format!("工作区目录不存在：{}", work_directory.display()));
    }
    let apk_path = resolve_sign_target(&work_directory, &relative_path)?;
    if !apk_path
        .to_string_lossy()
        .to_ascii_lowercase()
        .ends_with(".apk")
    {
        return Err(format!("请先选中要签名的 APK 文件：{relative_path}"));
    }
    if !apk_path.is_file() {
        return Err(format!("APK 文件不存在：{}", apk_path.display()));
    }
    let primary = apk_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| work_directory.clone());

    let message = tauri::async_runtime::spawn_blocking({
        let app = app.clone();
        let primary = primary.clone();
        let work_directory = work_directory.clone();
        let apk_path = apk_path.clone();
        move || -> Result<String, String> {
            let key = load_signing_key(&primary, &work_directory)?;
            let emitter = ProgressEmitter::new(app, "sign");
            key.sign_apk_with_progress(&apk_path, &|stage, unit, completed, total| {
                emitter.emit(stage, unit, completed, total)
            })?;
            Ok(format!("已签名（v1+v2+v3）：{}", apk_path.display()))
        }
    })
    .await
    .map_err(|error| format!("签名任务失败：{error}"))??;

    Ok(ApkOperationSummary {
        message,
        entries: 0,
        signed: true,
        output_path: Some(apk_path.to_string_lossy().into_owned()),
    })
}

/// 解析签名目标路径：工作区内相对路径校验后拼接；绝对路径（桌面端文件管理器选择）
/// 直接使用；`content://` URI 不支持（签名需在文件原地重写，Android 引导复制到工作区）。
fn resolve_sign_target(work_directory: &Path, location: &str) -> Result<PathBuf, String> {
    if location.contains("://") {
        return Err(
            "无法就地签名通过系统文件管理器选择的 APK：请将 APK 复制到工作区，然后在资源管理器中选中它再签名"
                .to_string(),
        );
    }
    if Path::new(location).is_absolute() {
        return Ok(PathBuf::from(location));
    }
    crate::paths::validate_relative_path(location)?;
    Ok(work_directory.join(location))
}

/// 导入默认签名密钥：支持 PEM（兼容 UTF-16/BOM）与 BKS（BouncyCastle，需密码）文件，
/// 统一转换后写入工作区根的 `signing.pem`。
#[tauri::command]
pub async fn import_signing_key<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    work_directory: String,
    key_path: String,
    password: Option<String>,
) -> Result<ApkOperationSummary, String> {
    let work_directory = PathBuf::from(&work_directory);
    if !work_directory.is_dir() {
        return Err(format!("工作区目录不存在：{}", work_directory.display()));
    }
    let bytes = tauri::async_runtime::spawn_blocking({
        let app = app.clone();
        let key_path = key_path.clone();
        move || -> Result<Vec<u8>, String> {
            #[cfg(target_os = "android")]
            {
                // Android：密钥文件可能是系统选择器返回的 content:// URI，
                // 通过 android-fs 插件直接读取二进制内容（不再经 Base64 过 IPC）。
                use tauri_plugin_android_fs::{AndroidFsExt, FsUri};
                let uri = if key_path.contains("://") {
                    FsUri::from_uri(key_path.clone())
                } else {
                    FsUri::from_path(Path::new(&key_path))
                };
                app.android_fs()
                    .read(&uri)
                    .map_err(|error| format!("读取密钥文件失败：{error}"))
            }
            #[cfg(not(target_os = "android"))]
            {
                let _ = app;
                fs::read(&key_path)
                    .map_err(|error| format!("读取密钥文件失败 {key_path}：{error}"))
            }
        }
    })
    .await
    .map_err(|error| format!("读取密钥文件失败：{error}"))??;

    let is_bks = crate::bks::is_bks(&bytes);
    let pem = if is_bks {
        let password = password
            .filter(|password| !password.is_empty())
            .ok_or_else(|| "该文件是 BKS 密钥库，请输入密钥库密码".to_string())?;
        crate::bks::bks_to_pem(&bytes, &password)?
    } else if key_path.to_ascii_lowercase().ends_with(".bks") {
        return Err("该文件扩展名为 .bks，但不是有效的 BKS 密钥库".to_string());
    } else {
        decode_pem_bytes(&bytes, &key_path)?
    };
    ApkSigningKey::from_pem(&pem).map_err(|error| {
        format!("密钥文件无效（需为包含证书与私钥的 PEM/BKS）：{error}")
    })?;
    let target = work_directory.join("signing.pem");
    fs::write(&target, pem.as_bytes())
        .map_err(|error| format!("写入默认密钥失败 {}：{error}", target.display()))?;

    Ok(ApkOperationSummary {
        message: format!(
            "已添加默认签名密钥（{}）：{}",
            if is_bks { "BKS" } else { "PEM" },
            target.display()
        ),
        entries: 0,
        signed: false,
        output_path: Some(target.to_string_lossy().into_owned()),
    })
}

/// 加载签名密钥：优先源目录下的 `signing.pem`，其次工作区根目录，
/// 都没有时回退到内置测试密钥。
fn load_signing_key(source: &Path, work_directory: &Path) -> Result<ApkSigningKey, String> {
    for directory in [source, work_directory] {
        let custom = directory.join("signing.pem");
        if custom.is_file() {
            let pem = read_pem_text(&custom)?;
            return ApkSigningKey::from_pem(&pem)
                .map_err(|error| format!("加载签名文件失败 {}：{error}", custom.display()));
        }
    }
    ApkSigningKey::builtin().map_err(|error| format!("加载内置签名密钥失败：{error}"))
}

/// 读取 PEM 文本，兼容 UTF-8（含 BOM）与 UTF-16 LE/BE（含 BOM）等常见 Windows 编码。
fn read_pem_text(path: &Path) -> Result<String, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("读取签名文件失败 {}：{error}", path.display()))?;
    decode_pem_bytes(&bytes, &path.display().to_string())
}

/// 解码可能为 UTF-8（含 BOM）或 UTF-16 LE/BE（含 BOM）的 PEM 文本。
fn decode_pem_bytes(bytes: &[u8], name: &str) -> Result<String, String> {
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let units = bytes[2..]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<u16>>();
        return String::from_utf16(&units)
            .map_err(|error| format!("签名文件编码无效 {name}：{error}"));
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let units = bytes[2..]
            .chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect::<Vec<u16>>();
        return String::from_utf16(&units)
            .map_err(|error| format!("签名文件编码无效 {name}：{error}"));
    }
    let mut text = bytes;
    if text.len() >= 3 && text[..3] == [0xEF, 0xBB, 0xBF] {
        text = &text[3..];
    }
    String::from_utf8(text.to_vec())
        .map_err(|error| format!("签名文件编码无效（应为文本 PEM） {name}：{error}"))
}

/// 解压目标目录：工作区下的 `<APK 名称>` 子目录（去掉 `.apk` 扩展名）。
/// 优先使用系统选择器提供的显示文件名（Android `content://` URI 无法直接解析文件名）。
fn apk_extract_destination(
    work_directory: &Path,
    apk_name: Option<&str>,
    apk_path: &Path,
) -> Result<PathBuf, String> {
    let derived;
    let name: &str = match apk_name.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => name,
        None => {
            derived = apk_path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| format!("无法解析 APK 文件名：{}", apk_path.display()))?
                .to_string();
            derived.as_str()
        }
    };
    let stem = if name.len() > 4 && name.to_ascii_lowercase().ends_with(".apk") {
        &name[..name.len() - 4]
    } else {
        name
    };
    let stem = stem.trim().trim_end_matches([' ', '.']);
    if stem.is_empty()
        || stem == "."
        || stem == ".."
        || stem.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|'])
    {
        return Err(format!("APK 文件名无法用作目录名：{name}"));
    }
    Ok(work_directory.join(stem))
}

/// 打包输出路径：源目录同级的 `<目录名>-repacked.apk`。
fn package_output_path(work_directory: &Path) -> Result<PathBuf, String> {
    let name = work_directory
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("无法确定工作区名称：{}", work_directory.display()))?;
    let parent = work_directory
        .parent()
        .ok_or_else(|| format!("无法确定工作区上级目录：{}", work_directory.display()))?;
    Ok(parent.join(format!("{name}-repacked.apk")))
}

/// 打包工作区为 APK（测试用，无进度回调），返回产物路径与文件数。
#[cfg(test)]
fn package_workspace(work_directory: &Path) -> Result<(PathBuf, u64), String> {
    let output = package_output_path(work_directory)?;
    let entries = crate::apk_pack::package_workspace(work_directory, &output, &|_, _| {})?;
    Ok((output, entries))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ageciv-apk-cmd-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn package_skips_old_signatures_and_signing_key() {
        let dir = temp_dir("package");
        let workspace = dir.join("DemoMod");
        fs::create_dir_all(workspace.join("missions")).unwrap();
        fs::create_dir_all(workspace.join("META-INF")).unwrap();
        fs::write(workspace.join("missions/Missions.json"), "{}").unwrap();
        fs::write(workspace.join("META-INF/MANIFEST.MF"), "old-signature").unwrap();
        fs::write(workspace.join("META-INF/CERT.SF"), "old-signature").unwrap();
        fs::write(workspace.join("signing.pem"), "custom-key").unwrap();
        fs::write(
            workspace.join(crate::apk_pack::SOURCE_APK_MARKER),
            "D:\\源包.apk",
        )
        .unwrap();
        fs::write(workspace.join("game.txt"), "hello").unwrap();

        let (output, entries) = package_workspace(&workspace).unwrap();
        assert_eq!(entries, 2);
        assert!(output.is_file());

        let archive = zip::ZipArchive::new(std::fs::File::open(&output).unwrap()).unwrap();
        let names: Vec<&str> = archive.file_names().collect();
        assert!(names.contains(&"missions/Missions.json"));
        assert!(names.contains(&"game.txt"));
        assert!(!names.iter().any(|name| name.starts_with("META-INF")));
        assert!(!names.contains(&"signing.pem"));
        // 「从 apk 中导入」写入的源 APK 标记只服务本地补全，不进入产物。
        assert!(!names.contains(&crate::apk_pack::SOURCE_APK_MARKER));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolves_sign_target_paths() {
        let workspace = PathBuf::from("workspace");
        // 工作区内相对路径：校验后拼接。
        let resolved = resolve_sign_target(&workspace, "sub/demo.apk").unwrap();
        assert!(resolved.ends_with("sub/demo.apk"));
        // 绝对路径（桌面端文件管理器选择）：原样使用。
        let absolute = if cfg!(windows) {
            "C:\\tmp\\ext.apk"
        } else {
            "/tmp/ext.apk"
        };
        assert_eq!(
            resolve_sign_target(&workspace, absolute).unwrap(),
            PathBuf::from(absolute)
        );
        // content:// URI：明确报错引导复制到工作区。
        let error = resolve_sign_target(&workspace, "content://downloads/demo.apk").unwrap_err();
        assert!(error.contains("复制到工作区"));
        // 越权相对路径：拒绝。
        assert!(resolve_sign_target(&workspace, "../escape.apk").is_err());
    }

    #[test]
    fn package_aligns_arsc_and_native_libs_and_utf8_names() {
        use std::io::Read as _;

        let dir = temp_dir("align");
        let workspace = dir.join("AlignMod");
        fs::create_dir_all(workspace.join("lib/arm64-v8a")).unwrap();
        fs::write(workspace.join("resources.arsc"), vec![0xABu8; 1234]).unwrap();
        fs::write(workspace.join("lib/arm64-v8a/libdemo.so"), vec![0x7Fu8; 4097]).unwrap();
        fs::write(workspace.join("剧本.txt"), "中文内容").unwrap();

        let (output, entries) = package_workspace(&workspace).unwrap();
        assert_eq!(entries, 3);

        let mut archive = zip::ZipArchive::new(std::fs::File::open(&output).unwrap()).unwrap();
        for (name, align) in [
            ("resources.arsc", 4_u64),
            ("lib/arm64-v8a/libdemo.so", 16 * 1024),
        ] {
            let entry = archive.by_name(name).unwrap();
            assert_eq!(entry.compression(), zip::CompressionMethod::Stored);
            assert_eq!(entry.data_start() % align, 0, "{name} 未按 {align} 字节对齐");
        }
        let mut text = String::new();
        archive
            .by_name("剧本.txt")
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text, "中文内容");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn apk_extract_destination_uses_apk_stem() {
        let workspace = Path::new("C:/GameCivs");
        assert_eq!(
            apk_extract_destination(workspace, None, Path::new("C:/apks/1566AuroraPrever2.apk"))
                .unwrap(),
            workspace.join("1566AuroraPrever2")
        );
        assert_eq!(
            apk_extract_destination(workspace, None, Path::new("C:/apks/暮色黄昏.APK")).unwrap(),
            workspace.join("暮色黄昏")
        );
        // 无 .apk 扩展名时使用完整文件名。
        assert_eq!(
            apk_extract_destination(workspace, None, Path::new("C:/apks/gamepack")).unwrap(),
            workspace.join("gamepack")
        );
        // Android：显示名优先（content:// URI 无法解析文件名）。
        assert_eq!(
            apk_extract_destination(
                workspace,
                Some("暮色黄昏_世界大战0.25.1.apk"),
                Path::new("content://com.android.providers.downloads.documents/document/msf%3A1000000123"),
            )
            .unwrap(),
            workspace.join("暮色黄昏_世界大战0.25.1")
        );
        // 非法名称（无法用作目录名）应报错。
        assert!(apk_extract_destination(workspace, None, Path::new("C:/apks/bad:name.apk")).is_err());
        assert!(apk_extract_destination(workspace, None, Path::new("C:/apks/...apk")).is_err());
    }

    #[test]
    fn package_reports_progress() {
        let dir = temp_dir("pack-progress");
        let workspace = dir.join("ProgressMod");
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("a.txt"), "hello").unwrap();
        fs::write(workspace.join("b.txt"), "world").unwrap();
        let output = dir.join("ProgressMod-repacked.apk");

        let emissions = std::sync::Mutex::new(Vec::new());
        let entries = crate::apk_pack::package_workspace(&workspace, &output, &|done, total| {
            emissions.lock().unwrap().push((done, total));
        })
        .unwrap();
        assert_eq!(entries, 2);
        let emissions = emissions.into_inner().unwrap();
        assert_eq!(emissions.first(), Some(&(0, 10)));
        assert_eq!(emissions.last(), Some(&(10, 10)));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn extract_restores_packaged_workspace() {
        let dir = temp_dir("roundtrip");
        let workspace = dir.join("DemoMod");
        fs::create_dir_all(workspace.join("assets")).unwrap();
        fs::write(workspace.join("assets/mode.txt"), "mod-content").unwrap();
        fs::write(workspace.join("root.txt"), "root-content").unwrap();

        let (output, entries) = package_workspace(&workspace).unwrap();
        assert_eq!(entries, 2);

        let restored = dir.join("Restored");
        fs::create_dir_all(&restored).unwrap();
        let outcome = crate::apk_extract::extract_apk_contents(&output, &restored).unwrap();
        assert_eq!(outcome.entries, 2);
        assert_eq!(
            fs::read_to_string(restored.join("assets/mode.txt")).unwrap(),
            "mod-content"
        );
        assert_eq!(
            fs::read_to_string(restored.join("root.txt")).unwrap(),
            "root-content"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_pem_text_handles_utf16_and_bom() {
        let dir = temp_dir("pem-encoding");
        let utf16_path = dir.join("utf16.pem");
        let mut utf16_bytes = vec![0xFF, 0xFE];
        for unit in "-----BEGIN CERTIFICATE-----\nABC\n".encode_utf16() {
            utf16_bytes.extend_from_slice(&unit.to_le_bytes());
        }
        fs::write(&utf16_path, &utf16_bytes).unwrap();
        assert_eq!(
            read_pem_text(&utf16_path).unwrap(),
            "-----BEGIN CERTIFICATE-----\nABC\n"
        );

        let bom_path = dir.join("bom.pem");
        fs::write(&bom_path, b"\xEF\xBB\xBFPEM".to_vec()).unwrap();
        assert_eq!(read_pem_text(&bom_path).unwrap(), "PEM");

        let _ = fs::remove_dir_all(&dir);
    }

    /// 实测打包速度（忽略）：对已解包的真实模组目录执行 APK 打包并计时。
    /// `cargo test --release -p age_civ_mod_tool --lib benchmark_pack_real_workspace -- --ignored --nocapture`
    #[test]
    #[ignore = "需要真实工作区 A:\\android\\GameCivs\\暮色黄昏_世界大战0.25.1"]
    fn benchmark_pack_real_workspace() {
        let workspace = PathBuf::from(r"A:\android\GameCivs\暮色黄昏_世界大战0.25.1");
        if !workspace.is_dir() {
            println!("未找到工作区，跳过");
            return;
        }
        let dir = temp_dir("bench-pack");
        let output = dir.join("pack-bench.apk");
        let started = Instant::now();
        let entries =
            crate::apk_pack::package_workspace(&workspace, &output, &|_, _| {}).unwrap();
        let elapsed = started.elapsed();
        let size = fs::metadata(&output)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        println!(
            "BENCH pack: entries={entries} size={:.1}MB in {:?}",
            size as f64 / 1_048_576.0,
            elapsed
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// 本机实测：真实 APK 解压 → 重新打包 → 签名 → apksigner 校验。
    ///
    /// 默认忽略（大文件耗时较长）：`cargo test -p age_civ_mod_tool --lib -- --ignored`
    /// 可用环境变量 `AGECIV_TEST_APK` 指定 APK 路径，否则尝试常见位置。
    /// 可用环境变量 `AGECIV_SIGNING_PEM` 指定自定义签名密钥 PEM（复制为工作区 signing.pem）。
    #[test]
    #[ignore = "依赖本机真实 APK，耗时较长"]
    fn real_apk_round_trip() {
        let Some(source) = find_test_apk() else {
            println!("未找到测试 APK，跳过");
            return;
        };
        println!("测试 APK：{}", source.display());
        let dir = temp_dir("real");
        let workspace = dir.join("RealApk");
        fs::create_dir_all(&workspace).unwrap();

        // 1. 解压到工作区。
        let started = std::time::Instant::now();
        let outcome = crate::apk_extract::extract_apk_contents(&source, &workspace).unwrap();
        let extracted = outcome.entries;
        println!(
            "解压 {extracted} 个文件（跳过 {}），用时 {:?}",
            outcome.skipped,
            started.elapsed()
        );
        assert!(extracted > 0);
        assert!(workspace.join("AndroidManifest.xml").is_file());

        // 可选的自定义签名密钥：复制为工作区 signing.pem（打包时会自动跳过该文件）。
        if let Ok(pem) = std::env::var("AGECIV_SIGNING_PEM") {
            let pem = PathBuf::from(pem);
            fs::copy(&pem, workspace.join("signing.pem")).unwrap();
            println!("使用自定义签名密钥：{}", pem.display());
        } else {
            println!("使用内置签名密钥");
        }

        // 2. 重新打包（跳过旧签名残留）。
        let started = std::time::Instant::now();
        let (output, entries) = package_workspace(&workspace).unwrap();
        println!(
            "打包 {entries} 个文件 -> {}，用时 {:?}",
            output.display(),
            started.elapsed()
        );
        assert!(entries > 0);
        {
            let archive = zip::ZipArchive::new(std::fs::File::open(&output).unwrap()).unwrap();
            assert!(archive.index_for_name("AndroidManifest.xml").is_some());
            assert!(!archive
                .file_names()
                .any(|name| name.to_ascii_uppercase().ends_with(".SF")));
            assert!(!archive
                .file_names()
                .any(|name| name.to_ascii_uppercase().ends_with(".RSA")));
        }

        // 3. 签名（优先使用工作区 signing.pem，与生产逻辑一致）。
        let started = std::time::Instant::now();
        load_signing_key(&workspace, &workspace)
            .unwrap()
            .sign_apk(&output)
            .unwrap();
        println!("签名完成，用时 {:?}", started.elapsed());

        // 4. 尽可能用真实 apksigner 复核（指定低 minSdk 以强制校验 v1）。
        if let Some(apksigner) = crate::apk_signing::find_apksigner() {
            let result = std::process::Command::new("cmd")
                .arg("/C")
                .arg(&apksigner)
                .arg("verify")
                .arg("--verbose")
                .arg("--min-sdk-version")
                .arg("18")
                .arg(&output)
                .output()
                .expect("运行 apksigner 失败");
            let stdout = String::from_utf8_lossy(&result.stdout);
            let stderr = String::from_utf8_lossy(&result.stderr);
            println!("{stdout}\n{stderr}");
            assert!(result.status.success(), "apksigner verify 未通过");
            assert!(
                stdout.contains("Verified using v1 scheme (JAR signing): true"),
                "apksigner 未确认 v1 签名"
            );
            assert!(
                stdout.contains("Verified using v2 scheme (APK Signature Scheme v2): true"),
                "apksigner 未确认 v2 签名"
            );
            assert!(
                stdout.contains("Verified using v3 scheme (APK Signature Scheme v3): true"),
                "apksigner 未确认 v3 签名"
            );
        } else {
            println!("未找到 apksigner，跳过官方校验");
        }

        let _ = fs::remove_dir_all(&dir);
    }

    /// 查找可用的测试 APK：优先环境变量 `AGECIV_TEST_APK`，否则尝试常见位置。
    fn find_test_apk() -> Option<PathBuf> {
        if let Ok(path) = std::env::var("AGECIV_TEST_APK") {
            let path = PathBuf::from(path);
            if path.is_file() {
                return Some(path);
            }
        }
        [
            r"A:\android\GameCivs\AgeofHistory3MissionsTool.apk",
            r"A:\android\GameCivs\1566AuroraPrever2.apk",
            r"A:\android\GameCivs\暮色黄昏_世界大战0.25.1.apk",
        ]
        .iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
    }
}
