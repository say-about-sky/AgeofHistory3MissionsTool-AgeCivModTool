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

/// 从事件脚本目录推导其「missions 资源根」（事件编辑器补全与资源候选按资源根加载，
/// 决议编辑器打开的全局 / 剧本事件需要落到该事件**所属模组**的资源根上）：
/// - `…/missionsEvents` → 去掉该段（如 `<mod>/assets/game/missions/missionsEvents`
///   → `<mod>/assets/game/missions`）；
/// - `…/events/<子目录…>` → `events` 段替换为 `missions`（如
///   `<mod>/assets/game/events/common` → `<mod>/assets/game/missions`；
///   剧本 `<mod>/assets/map/X/scenarios/Y/events/common` → `…/scenarios/Y/missions`）；
/// - 其他布局 / 根级无前缀的裸目录 → `None`（调用方回退默认树路径）。
pub fn missions_root_of_events_dir(events_dir: &str) -> Option<String> {
	let segments: Vec<&str> = events_dir
		.split('/')
		.filter(|segment| !segment.is_empty())
		.collect();
	if let Some(position) = segments.iter().position(|segment| *segment == "missionsEvents") {
		let head = segments[..position].join("/");
		return (!head.is_empty()).then_some(head);
	}
	if let Some(position) = segments.iter().position(|segment| *segment == "events") {
		let head = segments[..position].join("/");
		return Some(if head.is_empty() {
			"missions".to_string()
		} else {
			format!("{head}/missions")
		});
	}
	None
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn missions_root_of_events_dir_maps_layouts() {
		assert_eq!(
			missions_root_of_events_dir("modB/assets/game/missions/missionsEvents").as_deref(),
			Some("modB/assets/game/missions")
		);
		assert_eq!(
			missions_root_of_events_dir("modB/assets/game/events/common").as_deref(),
			Some("modB/assets/game/missions")
		);
		assert_eq!(
			missions_root_of_events_dir(
				"modB/assets/map/Earth3/scenarios/TheGreatWar/events/common"
			)
			.as_deref(),
			Some("modB/assets/map/Earth3/scenarios/TheGreatWar/missions")
		);
		assert_eq!(
			missions_root_of_events_dir("assets/game/events/common").as_deref(),
			Some("assets/game/missions")
		);
		// 根级无前缀 / 非事件目录 → 无法推导。
		assert_eq!(missions_root_of_events_dir("missionsEvents"), None);
		assert_eq!(missions_root_of_events_dir("assets/gfx/decision"), None);
	}
}
