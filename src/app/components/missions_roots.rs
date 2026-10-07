//! 国策资源根目录识别工具。
//!
//! 解包游戏 APK 后，国策资源可能位于：
//! - `missions/`（经典工作区）；
//! - `assets/game/missions/`（全局国策）；
//! - `assets/map/<地图>/scenarios/<剧本>/missions/`（每个剧本一套，数量不定）。
//!
//! 本模块提供统一的路径识别与拆分，供资源管理器、标签页与事件面板复用。

/// 判断 `path` 所属的国策资源根目录（返回根目录的工作区相对路径）。
///
/// 支持三种布局（前两种允许任意前缀，即解包 APK 后的 `<APK 名称>/assets/...`）：
/// - 工作区根级经典 `missions/`；
/// - 任意前缀下的 `assets/game/missions/`；
/// - 任意前缀下的 `assets/map/<地图>/scenarios/<剧本>/missions/`。
pub fn missions_root_of(path: &str) -> Option<String> {
	let segments: Vec<&str> = path
		.split('/')
		.filter(|segment| !segment.is_empty())
		.collect();
	for index in 0..segments.len() {
		if segments[index] != "assets" {
			continue;
		}
		if segments.get(index + 1) == Some(&"game")
			&& segments.get(index + 2) == Some(&"missions")
		{
			return Some(segments[..index + 3].join("/"));
		}
		if segments.get(index + 1).is_some()
			&& segments.get(index + 3) == Some(&"scenarios")
			&& segments.get(index + 4).is_some()
			&& segments.get(index + 5) == Some(&"missions")
		{
			return Some(segments[..index + 6].join("/"));
		}
	}
	if segments.first() == Some(&"missions") {
		return Some("missions".to_string());
	}
	None
}

/// 国策树文件判定：资源根目录下的直接 `.json` 子文件。返回（根目录，文件名）。
pub fn missions_tree_file(path: &str) -> Option<(String, String)> {
	let root = missions_root_of(path)?;
	let rest = path.strip_prefix(&root)?.trim_start_matches('/');
	if rest.is_empty() || rest.contains('/') || !rest.to_ascii_lowercase().ends_with(".json") {
		return None;
	}
	Some((root, rest.to_string()))
}

/// 资源根目录内子目录下的文件（如 `missionsEvents/x.txt`、`missionsImages/H/x.png`）。
/// 返回（根目录，子目录内相对名）。
pub fn missions_subfile(path: &str, subdir: &str) -> Option<(String, String)> {
	let root = missions_root_of(path)?;
	let rest = path.strip_prefix(&root)?.trim_start_matches('/');
	let name = rest.strip_prefix(subdir)?.strip_prefix('/')?;
	if name.is_empty() {
		return None;
	}
	Some((root, name.to_string()))
}

/// 资源根目录的展示标签（用于标签页标题）：经典 `missions` → missions，
/// 全局资源 → 解包目录前缀（无前缀时为 game），剧本资源 → 剧本目录名。
pub fn missions_root_label(root: &str) -> String {
	if root == "missions" {
		return "missions".to_string();
	}
	if let Some(prefix) = root.strip_suffix("/assets/game/missions") {
		if !prefix.is_empty() {
			return prefix.to_string();
		}
	}
	if root == "assets/game/missions" {
		return "game".to_string();
	}
	root.rsplit('/').nth(1).unwrap_or(root).to_string()
}

/// 资源根目录本身及其全部上级目录（用于资源管理器自动展开）。
pub fn root_ancestors(root: &str) -> Vec<String> {
	let mut paths = Vec::new();
	let mut cursor = String::new();
	for segment in root.split('/').filter(|segment| !segment.is_empty()) {
		if !cursor.is_empty() {
			cursor.push('/');
		}
		cursor.push_str(segment);
		paths.push(cursor.clone());
	}
	paths
}

/// 从工作区文件列表中选出默认打开的国策树：
/// 优先经典 `missions/Missions.json`，其次 `assets/game/missions/Missions.json`，
/// 再次任意剧本资源目录下的 `Missions.json`，最后任意第一个 `.json`（按路径排序）。
pub fn default_tree_path<'a>(paths: impl Iterator<Item = &'a str>) -> Option<String> {
	let mut trees: Vec<(usize, usize, String, String)> = paths
		.filter_map(|path| {
			let (root, name) = missions_tree_file(path)?;
			let root_rank = if root == "missions" {
				0
			} else if root == "assets/game/missions" || root.ends_with("/assets/game/missions") {
				1
			} else {
				2
			};
			let name_rank = if name == "Missions.json" { 0 } else { 1 };
			Some((root_rank, name_rank, root, name))
		})
		.collect();
	trees.sort();
	trees
		.into_iter()
		.next()
		.map(|(_, _, root, name)| format!("{root}/{name}"))
}
