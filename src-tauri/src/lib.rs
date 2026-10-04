use base64::{engine::general_purpose::STANDARD, Engine as _};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::{env, fs, path::{Component, Path, PathBuf}};

#[cfg(target_os = "android")]
mod all_files_access;

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

fn validate_work_directory_name(directory_name: &str) -> Result<&str, String> {
    let directory_name = directory_name.trim();
    let mut components = Path::new(directory_name).components();
    if directory_name.is_empty()
        || !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
    {
        return Err("目录名称必须是单个有效文件夹名".to_string());
    }
    Ok(directory_name)
}

#[tauri::command]
fn is_android() -> bool {
    cfg!(target_os = "android")
}

#[tauri::command]
fn has_all_files_access<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> Result<bool, String> {
    #[cfg(target_os = "android")]
    {
        all_files_access::has_access(app)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Ok(false)
    }
}

#[tauri::command]
fn request_all_files_access<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> Result<bool, String> {
    #[cfg(target_os = "android")]
    {
        all_files_access::request_access(app)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Ok(false)
    }
}

/// Android 工作区扫描条目（由 App 自带插件快速列目录返回，避免逐文件属性查询）。
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannedEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

#[tauri::command]
fn list_scoped_dir<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    folder_id: String,
    path: Option<String>,
) -> Result<Vec<ScannedEntry>, String> {
    #[cfg(target_os = "android")]
    {
        all_files_access::list_dir(app, folder_id, path)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, folder_id, path);
        Err("当前平台不支持 Android 工作区目录读取".to_string())
    }
}

#[tauri::command]
fn read_scoped_files_in_dir<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    folder_id: String,
    dir_path: String,
    names: Vec<String>,
) -> Result<Vec<FocusIcon>, String> {
    #[cfg(target_os = "android")]
    {
        all_files_access::read_files_in_dir(app, folder_id, dir_path, names).map(|files| {
            files
                .into_iter()
                .map(|file| FocusIcon {
                    name: file.name,
                    data_url: file.data_url,
                })
                .collect()
        })
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, folder_id, dir_path, names);
        Err("当前平台不支持 Android 工作区图标读取".to_string())
    }
}

#[tauri::command]
fn read_scoped_text_in_dir<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    folder_id: String,
    dir_path: String,
    file_name: String,
) -> Result<String, String> {
    #[cfg(target_os = "android")]
    {
        all_files_access::read_text_in_dir(app, folder_id, dir_path, file_name)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, folder_id, dir_path, file_name);
        Err("当前平台不支持 Android 工作区文件读取".to_string())
    }
}

#[tauri::command]
fn write_scoped_text_in_dir<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    folder_id: String,
    dir_path: String,
    file_name: String,
    contents: String,
) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        all_files_access::write_text_in_dir(app, folder_id, dir_path, file_name, contents)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, folder_id, dir_path, file_name, contents);
        Err("当前平台不支持 Android 工作区文件写入".to_string())
    }
}

/// 尝试把 Android SAF 工作区解析为真实文件系统路径。
/// 成功时前端会走与桌面端相同的原生命令（Rust 直接读写、可多线程），彻底绕过缓慢的
/// ContentProvider 逐项查询；失败时自动退回原有的 scoped-storage 通道。
#[tauri::command]
fn resolve_scoped_workspace_path<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    folder_id: String,
) -> Result<Option<String>, String> {
    #[cfg(target_os = "android")]
    {
        all_files_access::resolve_folder_path(app, folder_id)
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, folder_id);
        Ok(None)
    }
}

/// 探测真实路径是否可读（选目录后做一次廉价校验，失败则退回 scoped 通道）。
#[tauri::command]
fn is_workspace_path_readable(path: String) -> bool {
    let path = PathBuf::from(path);
    path.is_dir() && fs::read_dir(&path).is_ok()
}

#[tauri::command]
fn validate_work_directory_name_command(directory_name: String) -> Result<(), String> {
    validate_work_directory_name(&directory_name).map(|_| ())
}

#[derive(Serialize)]
struct FocusIcon {
    name: String,
    data_url: String,
}

#[derive(Deserialize, Serialize)]
struct MissionRecord {
    #[serde(rename = "ID")]
    id: i64,
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "ImageName")]
    image_name: String,
    #[serde(rename = "MissionEvent")]
    mission_event: String,
    #[serde(rename = "TreeColumn")]
    tree_column: u32,
    #[serde(rename = "TreeRow")]
    tree_row: u32,
    #[serde(rename = "RequiredMission")]
    required_mission: i64,
    #[serde(rename = "RequiredMission2")]
    required_mission2: i64,
    #[serde(rename = "AI")]
    ai: i32,
}

#[derive(Deserialize, Serialize)]
struct MissionFile {
    #[serde(rename = "Mission")]
    mission: Vec<MissionRecord>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceFile {
    name: String,
    relative_path: String,
    is_directory: bool,
}

fn parse_mission_file(content: &str) -> Result<MissionFile, String> {
    let normalized = content.replace("Age_of_History: Mission", "Age_of_History: \"Mission\"");
    json5::from_str(&normalized).map_err(|error| error.to_string())
}

fn serialize_mission_file(missions: &[MissionRecord]) -> Result<String, String> {
    let mut content = String::from("{\n\tMission:\n\t[\n");
    for mission in missions {
        let name = serde_json::to_string(&mission.name).map_err(|error| error.to_string())?;
        let image_name =
            serde_json::to_string(&mission.image_name).map_err(|error| error.to_string())?;
        let mission_event =
            serde_json::to_string(&mission.mission_event).map_err(|error| error.to_string())?;
        content.push_str(&format!(
            "\t\t{{\n\t\t\tID: {},\n\t\t\tName: {},\n\t\t\tImageName: {},\n\t\t\tMissionEvent: {},\n\t\t\tTreeColumn: {},\n\t\t\tTreeRow: {},\n\t\t\tRequiredMission: {},\n\t\t\tRequiredMission2: {},\n\t\t\tAI: {},\n\t\t}},\n",
            mission.id,
            name,
            image_name,
            mission_event,
            mission.tree_column,
            mission.tree_row,
            mission.required_mission,
            mission.required_mission2,
            mission.ai,
        ));
    }
    content.push_str("\t],\n\tAge_of_History: Mission\n}\n");
    Ok(content)
}

#[tauri::command]
fn parse_missions(contents: String) -> Result<Vec<MissionRecord>, String> {
    parse_mission_file(&contents).map(|file| file.mission)
}

#[tauri::command]
fn serialize_missions(mut missions: Vec<MissionRecord>) -> Result<String, String> {
    for (index, mission) in missions.iter_mut().enumerate() {
        mission.id = index as i64;
        if mission.mission_event.trim().is_empty() {
            mission.mission_event = format!("{}.txt", mission.name);
        }
        mission.ai = 100;
    }
    serialize_mission_file(&missions)
}

#[tauri::command]
fn encode_mission_icon(name: String, data: Vec<u8>) -> FocusIcon {
    FocusIcon {
        name,
        data_url: format!("data:image/png;base64,{}", STANDARD.encode(data)),
    }
}

#[derive(Deserialize)]
struct IconPayload {
    name: String,
    data: Vec<u8>,
}

#[tauri::command]
fn encode_mission_icons(icons: Vec<IconPayload>) -> Vec<FocusIcon> {
    icons
        .into_iter()
        .map(|icon| FocusIcon {
            name: icon.name,
            data_url: format!("data:image/png;base64,{}", STANDARD.encode(icon.data)),
        })
        .collect()
}

fn missions_directory(work_directory: &str) -> Result<PathBuf, String> {
    let directory = PathBuf::from(work_directory).join("missions");
    if directory.is_dir() {
        Ok(directory)
    } else {
        Err(format!("所选目录下未找到 missions 文件夹：{}", directory.display()))
    }
}

fn append_workspace_entries(
    root: &Path,
    directory: &Path,
    entries: &mut Vec<WorkspaceFile>,
) -> Result<(), String> {
    let mut children = fs::read_dir(directory)
        .map_err(|error| error.to_string())?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    children.sort();

    for path in children {
        let file_type = fs::symlink_metadata(&path)
            .map_err(|error| error.to_string())?
            .file_type();
        let relative_path = path
            .strip_prefix(root)
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        let is_directory = file_type.is_dir();
        entries.push(WorkspaceFile {
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            relative_path,
            is_directory,
        });
        if is_directory {
            append_workspace_entries(root, &path, entries)?;
        }
    }
    Ok(())
}

#[tauri::command]
fn list_workspace_files(work_directory: String) -> Result<Vec<WorkspaceFile>, String> {
    let root = PathBuf::from(work_directory);
    if !root.is_dir() {
        return Err(format!("工作目录不存在：{}", root.display()));
    }
    let mut entries = Vec::new();
    append_workspace_entries(&root, &root, &mut entries)?;
    Ok(entries)
}

#[tauri::command]
fn load_missions(work_directory: String) -> Result<Vec<MissionRecord>, String> {
    let path = missions_directory(&work_directory)?.join("Missions.json");
    let content = fs::read_to_string(&path).map_err(|error| error.to_string())?;
    let file = parse_mission_file(&content)?;
    Ok(file.mission)
}

#[tauri::command]
fn save_missions(work_directory: String, missions: Vec<MissionRecord>) -> Result<(), String> {
    let content = serialize_missions(missions)?;
    fs::write(missions_directory(&work_directory)?.join("Missions.json"), content)
        .map_err(|error| error.to_string())
}

/// 按文件名读取 missions 目录下的单个国策树配置（多国策树工作区）。
#[tauri::command]
fn load_missions_file(work_directory: String, file_name: String) -> Result<Vec<MissionRecord>, String> {
    validate_work_directory_name(&file_name)?;
    let path = missions_directory(&work_directory)?.join(&file_name);
    let content = fs::read_to_string(&path)
        .map_err(|error| format!("读取国策配置失败 {}：{error}", path.display()))?;
    let file = parse_mission_file(&content)?;
    Ok(file.mission)
}

/// 按文件名保存单个国策树配置到 missions 目录下。
#[tauri::command]
fn save_missions_file(
    work_directory: String,
    file_name: String,
    missions: Vec<MissionRecord>,
) -> Result<(), String> {
    validate_work_directory_name(&file_name)?;
    let content = serialize_missions(missions)?;
    fs::write(missions_directory(&work_directory)?.join(&file_name), content)
        .map_err(|error| error.to_string())
}

/// 按名称加载 missionsImages/H 下的指定图标（缺失的图标静默跳过）。
/// 使用 rayon 并行读取并 base64 编码，多核设备上批量图标载入显著加速。
#[tauri::command]
fn load_mission_icons(work_directory: String, names: Vec<String>) -> Result<Vec<FocusIcon>, String> {
    let icon_directory = missions_directory(&work_directory)?.join("missionsImages").join("H");
    let icons = names
        .into_par_iter()
        .filter_map(|name| {
            if validate_work_directory_name(&name).is_err() {
                return None;
            }
            let path = icon_directory.join(format!("{name}.png"));
            let bytes = fs::read(&path).ok()?;
            Some(FocusIcon {
                name,
                data_url: format!("data:image/png;base64,{}", STANDARD.encode(bytes)),
            })
        })
        .collect();
    Ok(icons)
}

#[tauri::command]
fn load_mission_event(work_directory: String, file_name: String) -> Result<String, String> {
    validate_work_directory_name(&file_name)?;
    let path = missions_directory(&work_directory)?
        .join("missionsEvents")
        .join(&file_name);
    fs::read_to_string(&path)
        .map_err(|error| format!("读取事件脚本失败 {}：{error}", path.display()))
}

#[tauri::command]
fn save_mission_event(
    work_directory: String,
    file_name: String,
    contents: String,
) -> Result<(), String> {
    validate_work_directory_name(&file_name)?;
    let path = missions_directory(&work_directory)?
        .join("missionsEvents")
        .join(&file_name);
    fs::write(&path, contents)
        .map_err(|error| format!("保存事件脚本失败 {}：{error}", path.display()))
}

#[tauri::command]
fn delete_mission_event(work_directory: String, file_name: String) -> Result<(), String> {
    validate_work_directory_name(&file_name)?;
    let path = missions_directory(&work_directory)?
        .join("missionsEvents")
        .join(&file_name);
    fs::remove_file(&path)
        .map_err(|error| format!("删除事件脚本失败 {}：{error}", path.display()))
}

#[tauri::command]
fn rename_mission_event(
    work_directory: String,
    old_file_name: String,
    new_file_name: String,
) -> Result<(), String> {
    validate_work_directory_name(&old_file_name)?;
    validate_work_directory_name(&new_file_name)?;
    let directory = missions_directory(&work_directory)?.join("missionsEvents");
    let from = directory.join(&old_file_name);
    let to = directory.join(&new_file_name);
    fs::rename(&from, &to)
        .map_err(|error| format!("重命名事件脚本失败 {}：{error}", from.display()))
}

/// 校验工作区内相对路径（拒绝空路径、绝对路径与 ..）。
fn validate_relative_path(relative: &str) -> Result<(), String> {
    let path = Path::new(relative);
    if relative.is_empty() || path.is_absolute() {
        return Err(format!("无效的工作区相对路径：{relative}"));
    }
    for component in path.components() {
        if !matches!(component, Component::Normal(_)) {
            return Err(format!("工作区路径不能包含 .. 或特殊段：{relative}"));
        }
    }
    Ok(())
}

fn workspace_abs_path(work_directory: &str, relative: &str) -> Result<PathBuf, String> {
    validate_relative_path(relative)?;
    let root = PathBuf::from(work_directory);
    if !root.is_dir() {
        return Err(format!("工作目录不存在：{}", root.display()));
    }
    Ok(root.join(relative))
}

fn copy_dir_recursive(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|error| error.to_string())?;
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
            copy_dir_recursive(&path, &target)?;
        } else {
            fs::copy(&path, &target)
                .map_err(|error| format!("复制失败 {}：{error}", path.display()))?;
        }
    }
    Ok(())
}

#[tauri::command]
fn copy_workspace_item(
    work_directory: String,
    source: String,
    target: String,
) -> Result<(), String> {
    let from = workspace_abs_path(&work_directory, &source)?;
    let to = workspace_abs_path(&work_directory, &target)?;
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
        copy_dir_recursive(&from, &to)
    } else {
        fs::copy(&from, &to)
            .map(|_| ())
            .map_err(|error| format!("复制失败 {}：{error}", from.display()))
    }
}

#[tauri::command]
fn move_workspace_item(
    work_directory: String,
    source: String,
    target: String,
) -> Result<(), String> {
    let from = workspace_abs_path(&work_directory, &source)?;
    let to = workspace_abs_path(&work_directory, &target)?;
    if !from.exists() {
        return Err(format!("源不存在：{}", from.display()));
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::rename(&from, &to).map_err(|error| format!("移动失败 {}：{error}", from.display()))
}

#[tauri::command]
fn delete_workspace_item(work_directory: String, target: String) -> Result<(), String> {
    let path = workspace_abs_path(&work_directory, &target)?;
    if !path.exists() {
        return Err(format!("目标不存在：{}", path.display()));
    }
    if path.is_dir() {
        fs::remove_dir_all(&path).map_err(|error| error.to_string())
    } else {
        fs::remove_file(&path).map_err(|error| error.to_string())
    }
}

#[tauri::command]
fn reveal_workspace_item(work_directory: String, target: String) -> Result<(), String> {
    let path = workspace_abs_path(&work_directory, &target)?;
    if !path.exists() {
        return Err(format!("目标不存在：{}", path.display()));
    }
    #[cfg(target_os = "windows")]
    {
        // explorer 的 /select 参数需要完整引号包裹，经 cmd 转发最可靠，
        // 否则带空格/中文的路径只会打开资源管理器而不选中文件。
        let quoted = format!("explorer.exe /select,\"{}\"", path.display());
        std::process::Command::new("cmd")
            .arg("/C")
            .arg(&quoted)
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("打开文件位置失败：{error}"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
        Err("当前系统暂不支持打开文件位置".to_string())
    }
}

#[tauri::command]
fn list_mission_icons(work_directory: String) -> Result<Vec<FocusIcon>, String> {
    let icon_directory = missions_directory(&work_directory)?.join("missionsImages").join("H");
    let mut paths = fs::read_dir(&icon_directory)
        .map_err(|error| error.to_string())?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    paths.retain(|path| {
        path.extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
    });
    paths.sort();

    paths
        .into_iter()
        .map(|path| {
            let name = path
                .file_stem()
                .ok_or_else(|| format!("无效图标路径：{}", path.display()))?
                .to_string_lossy()
                .into_owned();
            let bytes = fs::read(&path).map_err(|error| error.to_string())?;
            Ok(FocusIcon {
                name,
                data_url: format!("data:image/png;base64,{}", STANDARD.encode(bytes)),
            })
        })
        .collect()
}

#[tauri::command]
fn create_work_directory(parent_directory: String, directory_name: String) -> Result<String, String> {
    let directory_name = validate_work_directory_name(&directory_name)?;

    let work_directory = PathBuf::from(parent_directory).join(directory_name);
    fs::create_dir(&work_directory).map_err(|error| error.to_string())?;
    let missions_directory = work_directory.join("missions");
    fs::create_dir_all(missions_directory.join("missionsImages").join("H"))
        .map_err(|error| error.to_string())?;
    fs::create_dir_all(missions_directory.join("missionsEvents"))
        .map_err(|error| error.to_string())?;
    let content = serialize_mission_file(&[])?;
    fs::write(missions_directory.join("Missions.json"), content)
        .map_err(|error| error.to_string())?;

    Ok(work_directory.to_string_lossy().into_owned())
}

#[tauri::command]
fn list_focus_icons() -> Vec<FocusIcon> {
    let mut directories = Vec::new();
    if let Ok(current_dir) = env::current_dir() {
        directories.push(current_dir.join("png"));
    }
    if let Ok(executable) = env::current_exe() {
        if let Some(executable_dir) = executable.parent() {
            directories.push(executable_dir.join("png"));
        }
    }
    directories.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../png"));

    let Some(icon_dir) = directories.into_iter().find(|path| path.is_dir()) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(icon_dir) else {
        return Vec::new();
    };

    let mut paths: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
        })
        .collect();
    paths.sort();

    paths
        .into_iter()
        .filter_map(|path| {
            let name = path.file_stem()?.to_string_lossy().into_owned();
            let bytes = fs::read(path).ok()?;
            Some(FocusIcon {
                name,
                data_url: format!("data:image/png;base64,{}", STANDARD.encode(bytes)),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{parse_mission_file, serialize_mission_file};

    #[test]
    fn mission_file_round_trips_json5_syntax() {
        let source = r#"{
                Mission: [
                    {
                        ID: 7,
                        Name: "测试国策",
                        ImageName: "测试国策.png",
                        MissionEvent: "测试国策.txt",
                        TreeColumn: 0,
                        TreeRow: 2,
                        RequiredMission: -1,
                        RequiredMission2: -1,
                        AI: 100,
                    },
                ],
                Age_of_History: Mission
            }"#;

            let parsed = parse_mission_file(source).expect("valid mission data");
        assert_eq!(parsed.mission.len(), 1);
        assert_eq!(parsed.mission[0].id, 7);
        assert_eq!(parsed.mission[0].tree_column, 0);

        let serialized = serialize_mission_file(&parsed.mission).expect("serialize mission data");
        assert!(serialized.contains("Age_of_History: Mission"));
        let round_tripped = parse_mission_file(&serialized).expect("valid serialized mission data");
        assert_eq!(round_tripped.mission[0].name, "测试国策");
    }
}

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
            greet,
            is_android,
            has_all_files_access,
            request_all_files_access,
            list_scoped_dir,
            read_scoped_files_in_dir,
            read_scoped_text_in_dir,
            write_scoped_text_in_dir,
            resolve_scoped_workspace_path,
            is_workspace_path_readable,
            validate_work_directory_name_command,
            list_focus_icons,
            parse_missions,
            serialize_missions,
            encode_mission_icon,
            encode_mission_icons,
            load_missions,
            save_missions,
            load_missions_file,
            save_missions_file,
            load_mission_icons,
            load_mission_event,
            save_mission_event,
            delete_mission_event,
            rename_mission_event,
            copy_workspace_item,
            move_workspace_item,
            delete_workspace_item,
            reveal_workspace_item,
            list_mission_icons,
            create_work_directory,
            list_workspace_files
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
