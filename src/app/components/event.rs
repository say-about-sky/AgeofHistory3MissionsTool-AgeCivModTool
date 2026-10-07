use dioxus::prelude::*;
use serde::Serialize;
use wasm_bindgen_futures::JsFuture;

use super::undo::{UndoRegistration, UndoScope};
use super::{event_grid::EventGrid, event_parser::{diagnostics, MissionEvent}, WorkDirectory};
use crate::app::tauri_bridge::invoke;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScopedReadTextInDir {
	folder_id: String,
	dir_path: String,
	file_name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScopedWriteTextInDir {
	folder_id: String,
	dir_path: String,
	file_name: String,
	contents: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EventFileArgs {
	work_directory: String,
	missions_root: String,
	file_name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EventFileWriteArgs {
	work_directory: String,
	missions_root: String,
	file_name: String,
	contents: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScopedRemoveFile {
	folder_id: String,
	path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScopedMove {
	from_folder_id: String,
	from_path: String,
	to_folder_id: String,
	to_path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EventRenameArgs {
	work_directory: String,
	missions_root: String,
	old_file_name: String,
	new_file_name: String,
}

fn event_file_path(root_path: &str, missions_root: &str, file_name: &str) -> String {
	if root_path.is_empty() {
		format!("{missions_root}/missionsEvents/{file_name}")
	} else {
		format!("{root_path}/{missions_root}/missionsEvents/{file_name}")
	}
}

fn event_dir_path(root_path: &str, missions_root: &str) -> String {
	if root_path.is_empty() {
		format!("{missions_root}/missionsEvents")
	} else {
		format!("{root_path}/{missions_root}/missionsEvents")
	}
}

async fn load_event_text(
	directory: &WorkDirectory,
	missions_root: &str,
	file_name: &str,
) -> Result<String, String> {
	if let Some(folder_id) = &directory.folder_id {
		let args = serde_wasm_bindgen::to_value(&ScopedReadTextInDir {
			folder_id: folder_id.clone(),
			dir_path: event_dir_path(&directory.root_path, missions_root),
			file_name: file_name.to_string(),
		})
		.map_err(|error| error.to_string())?;
		let value = JsFuture::from(invoke("read_scoped_text_in_dir", args))
			.await
			.map_err(|error| format!("读取事件文件失败：{error:?}"))?;
		value
			.as_string()
			.ok_or_else(|| "事件文件读取结果无效".to_string())
	} else {
		let args = serde_wasm_bindgen::to_value(&EventFileArgs {
			work_directory: directory.root_path.clone(),
			missions_root: missions_root.to_string(),
			file_name: file_name.to_string(),
		})
		.map_err(|error| error.to_string())?;
		let value = JsFuture::from(invoke("load_mission_event", args))
			.await
			.map_err(|error| format!("读取事件文件失败：{error:?}"))?;
		value
			.as_string()
			.ok_or_else(|| "事件文件读取结果无效".to_string())
	}
}

pub(crate) async fn save_event_text(
	directory: &WorkDirectory,
	missions_root: &str,
	file_name: &str,
	contents: &str,
) -> Result<(), String> {
	if let Some(folder_id) = &directory.folder_id {
		let args = serde_wasm_bindgen::to_value(&ScopedWriteTextInDir {
			folder_id: folder_id.clone(),
			dir_path: event_dir_path(&directory.root_path, missions_root),
			file_name: file_name.to_string(),
			contents: contents.to_string(),
		})
		.map_err(|error| error.to_string())?;
		JsFuture::from(invoke("write_scoped_text_in_dir", args))
			.await
			.map_err(|error| format!("保存事件文件失败：{error:?}"))?;
		Ok(())
	} else {
		let args = serde_wasm_bindgen::to_value(&EventFileWriteArgs {
			work_directory: directory.root_path.clone(),
			missions_root: missions_root.to_string(),
			file_name: file_name.to_string(),
			contents: contents.to_string(),
		})
		.map_err(|error| error.to_string())?;
		JsFuture::from(invoke("save_mission_event", args))
			.await
			.map_err(|error| format!("保存事件文件失败：{error:?}"))?;
		Ok(())
	}
}

pub(crate) async fn delete_event_text(
	directory: &WorkDirectory,
	missions_root: &str,
	file_name: &str,
) -> Result<(), String> {
	if let Some(folder_id) = &directory.folder_id {
		let path = event_file_path(&directory.root_path, missions_root, file_name);
		let args = serde_wasm_bindgen::to_value(&ScopedRemoveFile {
			folder_id: folder_id.clone(),
			path,
		})
		.map_err(|error| error.to_string())?;
		JsFuture::from(invoke("remove_scoped_file", args))
			.await
			.map_err(|error| format!("删除事件文件失败：{error:?}"))?;
		Ok(())
	} else {
		let args = serde_wasm_bindgen::to_value(&EventFileArgs {
			work_directory: directory.root_path.clone(),
			missions_root: missions_root.to_string(),
			file_name: file_name.to_string(),
		})
		.map_err(|error| error.to_string())?;
		JsFuture::from(invoke("delete_mission_event", args))
			.await
			.map_err(|error| format!("删除事件文件失败：{error:?}"))?;
		Ok(())
	}
}

pub(crate) async fn rename_event_text(
	directory: &WorkDirectory,
	missions_root: &str,
	old_file_name: &str,
	new_file_name: &str,
) -> Result<(), String> {
	if let Some(folder_id) = &directory.folder_id {
		let from_path = event_file_path(&directory.root_path, missions_root, old_file_name);
		let to_path = event_file_path(&directory.root_path, missions_root, new_file_name);
		let args = serde_wasm_bindgen::to_value(&ScopedMove {
			from_folder_id: folder_id.clone(),
			from_path,
			to_folder_id: folder_id.clone(),
			to_path,
		})
		.map_err(|error| error.to_string())?;
		JsFuture::from(invoke("move_scoped_item", args))
			.await
			.map_err(|error| format!("重命名事件文件失败：{error:?}"))?;
		Ok(())
	} else {
		let args = serde_wasm_bindgen::to_value(&EventRenameArgs {
			work_directory: directory.root_path.clone(),
			missions_root: missions_root.to_string(),
			old_file_name: old_file_name.to_string(),
			new_file_name: new_file_name.to_string(),
		})
		.map_err(|error| error.to_string())?;
		JsFuture::from(invoke("rename_mission_event", args))
			.await
			.map_err(|error| format!("重命名事件文件失败：{error:?}"))?;
		Ok(())
	}
}

/// 国策事件脚本编辑面板，挂在主编辑区右侧；文件选择由资源管理器/思维导图联动。
#[component]
pub fn EventPanel(
	work_directory: Option<WorkDirectory>,
	missions_root: Signal<Option<String>>,
	save_request: Signal<u64>,
	open_request: Signal<Option<(String, u64)>>,
	rename_request: Signal<Option<(String, String)>>,
	release_request: Signal<u64>,
	on_selection_change: EventHandler<Option<String>>,
	width: f64,
	/// 编辑操作注册到分区撤销栈（由 Work 统一管理）。
	on_undo_push: EventHandler<UndoRegistration>,
	/// 事件面板当前打开文件的标识（「根/missionsEvents/文件」）：供 Work 判断事件分区的撤销可用性。
	active_scope: Signal<Option<String>>,
) -> Element {
	let mut selected = use_signal(|| None::<String>);
	let mut event = use_signal(|| None::<MissionEvent>);
	let mut original = use_signal(String::new);
	let mut status = use_signal(String::new);
	let busy = use_signal(|| false);
	let mut last_save_request = use_signal(|| *save_request.read());

	// —— 全局撤销集成 ——
	// 待应用的撤销/重做快照：由注册的执行器写入，effect 统一落地（避免直接 set 触发追踪误注册）。
	let mut pending_apply = use_signal(|| None::<MissionEvent>);
	// 内容追踪基线：与当前内容出现差异时即产生一次可撤销操作。
	let mut last_recorded = use_signal(|| None::<MissionEvent>);
	use_effect(move || {
		let Some(target) = pending_apply.read().clone() else {
			return;
		};
		last_recorded.set(Some(target.clone()));
		event.set(Some(target));
		pending_apply.set(None);
	});
	use_effect(move || {
		let current = event.read().clone();
		let previous = last_recorded.read().clone();
		if current == previous {
			return;
		}
		// 载入 / 释放边界（None ↔ Some）只更新基线，不产生撤销步骤。
		let (Some(before), Some(after)) = (previous, current.clone()) else {
			last_recorded.set(current);
			return;
		};
		last_recorded.set(current);
		let Some(file) = selected.read().clone() else {
			return;
		};
		let root = missions_root
			.read()
			.clone()
			.unwrap_or_else(|| "missions".to_string());
		let scope_key = format!("{root}/missionsEvents/{file}");
		let scope = UndoScope::EventFile(scope_key.clone());
		// 执行器只在仍打开同一文件时应用快照（避免把内容串写到其它文件）；
		// Work 层还会按「当前打开文件」过滤，这里为双保险。
		let mut pending_for_undo = pending_apply;
		let active_scope_for_undo = active_scope;
		let key_for_undo = scope_key.clone();
		let before_for_undo = before.clone();
		let undo = EventHandler::new(move |_: ()| {
			if active_scope_for_undo.read().as_deref() == Some(key_for_undo.as_str()) {
				pending_for_undo.set(Some(before_for_undo.clone()));
			}
		});
		let mut pending_for_redo = pending_apply;
		let active_scope_for_redo = active_scope;
		let key_for_redo = scope_key;
		let after_for_redo = after;
		let redo = EventHandler::new(move |_: ()| {
			if active_scope_for_redo.read().as_deref() == Some(key_for_redo.as_str()) {
				pending_for_redo.set(Some(after_for_redo.clone()));
			}
		});
		on_undo_push.call((scope, undo, redo));
	});

	let directory_for_open = work_directory.clone();
	let open_event = use_callback(move |file_name: String| {
		// 已是当前文件且已载入时跳过重读，避免覆盖未保存的编辑。
		if selected.read().clone() == Some(file_name.clone()) && event.read().is_some() {
			return;
		}
		let Some(directory) = directory_for_open.clone() else {
			status.set("请先打开工作区".to_string());
			return;
		};
		let missions_root = missions_root
			.read()
			.clone()
			.unwrap_or_else(|| "missions".to_string());
		let mut busy = busy;
		let mut selected = selected;
		let mut event = event;
		let mut original = original;
		let mut status = status;
		spawn(async move {
			busy.set(true);
			status.set("正在读取...".to_string());
			event.set(None);
			match load_event_text(&directory, &missions_root, &file_name).await {
				Ok(text) => {
					selected.set(Some(file_name.clone()));
					original.set(text.clone());
					event.set(Some(MissionEvent::parse(&text)));
					status.set(String::new());
					// 供 Work 判断事件分区的撤销可用性。
					active_scope
						.set(Some(format!("{missions_root}/missionsEvents/{file_name}")));
					on_selection_change
						.call(Some(format!("{missions_root}/missionsEvents/{file_name}")));
				}
				Err(error) => status.set(error),
			}
			busy.set(false);
		});
	});

	// Ctrl+S 与"保存脚本"按钮都通过 save_request 触发，落盘当前修改。
	let directory_for_save = work_directory.clone();
	use_effect(move || {
		let request = *save_request.read();
		if request == *last_save_request.read() {
			return;
		}
		last_save_request.set(request);
		let Some(directory) = directory_for_save.clone() else {
			return;
		};
		let missions_root = missions_root
			.read()
			.clone()
			.unwrap_or_else(|| "missions".to_string());
		let Some(file_name) = selected.read().clone() else {
			return;
		};
		let Some(current) = event.read().clone() else {
			return;
		};
		let text = current.to_text();
		if text == original.read().clone() {
			return;
		}
		let mut original = original;
		let mut status = status;
		spawn(async move {
			status.set("正在保存...".to_string());
			match save_event_text(&directory, &missions_root, &file_name, &text).await {
				Ok(()) => {
					original.set(text);
					status.set("已保存".to_string());
				}
				Err(error) => status.set(format!("保存失败：{error}")),
			}
		});
	});

	let mut last_release_request = use_signal(|| *release_request.read());
	use_effect(move || {
		let request = *release_request.read();
		if request == *last_release_request.read() {
			return;
		}
		last_release_request.set(request);
		selected.set(None);
		event.set(None);
		original.set(String::new());
		status.set("已关闭".to_string());
		active_scope.set(None);
		on_selection_change.call(None);
	});

	let mut last_open_request = use_signal(|| None::<(String, u64)>);
	use_effect(move || {
		let request = open_request.read().clone();
		if request.is_none() || request == *last_open_request.read() {
			return;
		}
		last_open_request.set(request.clone());
		if let Some((file_name, _)) = request {
			open_event.call(file_name);
		}
	});

	// 卡片改名后同步：若正在编辑旧文件，未修改则切到新文件，否则提示用户。
	let mut last_rename_request = use_signal(|| None::<(String, String)>);
	use_effect(move || {
		let request = rename_request.read().clone();
		if request.is_none() || request == *last_rename_request.read() {
			return;
		}
		last_rename_request.set(request.clone());
		if let Some((old_name, new_name)) = request {
			if selected.read().clone() == Some(old_name.clone()) {
				let current_text = event.read().as_ref().map(MissionEvent::to_text);
				let original_snapshot = original.read().clone();
				let dirty = current_text.as_deref() != Some(original_snapshot.as_str());
				if dirty {
					status.set(format!(
						"脚本已重命名为 {new_name}；当前未保存的修改仍指向旧文件"
					));
				} else {
					open_event.call(new_name);
				}
			}
		}
	});

	let selected_name = selected.read().clone();
	let original_text = original.read().clone();
	let current_text = event.read().as_ref().map(MissionEvent::to_text);
	let is_dirty = selected_name.is_some() && current_text.as_deref() != Some(original_text.as_str());
	let status_text = status.read().clone();
	let busy_now = *busy.read();
	let (invalid_count, unknown_count, typo_count) = event
		.read()
		.as_ref()
		.map(diagnostics)
		.unwrap_or((0, 0, 0));
	let issue_count = invalid_count + unknown_count + typo_count;

	rsx! {
        aside { class: "events-pane", style: "width: {width}px;",
            div { class: "pane-heading",
                span { "国策事件" }
                span {
                    class: "pane-count",
                    title: "{selected_name.as_deref().unwrap_or_default()}",
                    "{selected_name.as_deref().unwrap_or_default()}"
                }
            }
            if let Some(file_name) = selected_name.clone() {
                div { class: "event-editor",
                    label { "missionsEvents / {file_name}" }
                    if event.read().is_some() {
                        EventGrid { event }
                    } else if busy_now {
                        div { class: "event-grid-empty", "正在读取..." }
                    } else {
                        div { class: "event-grid-empty", "脚本尚未载入" }
                    }
                    div { class: "event-actions",
                        button {
                            class: "event-save",
                            r#type: "button",
                            disabled: busy_now || !is_dirty,
                            onclick: move |_| save_request.with_mut(|request| *request = request.wrapping_add(1)),
                            "保存脚本"
                        }
                        button {
                            class: "event-revert",
                            r#type: "button",
                            disabled: busy_now || !is_dirty,
                            onclick: move |_| {
                                let text = original.read().clone();
                                event.set(Some(MissionEvent::parse(&text)));
                            },
                            "还原"
                        }
                        span { class: "event-status", role: "status",
                            if busy_now {
                                "正在处理..."
                            } else if is_dirty {
                                "已修改"
                            } else {
                                "{status_text}"
                            }
                        }
                        if issue_count > 0 && !busy_now {
                            span { class: "event-diagnostics", role: "status",
                                if invalid_count > 0 {
                                    "⚠ {invalid_count} 处类型不符"
                                }
                                if unknown_count > 0 {
                                    " · {unknown_count} 个未识别键"
                                }
                                if typo_count > 0 {
                                    " · {typo_count} 处疑似拼写错误"
                                }
                            }
                        }
                    }
                }
            } else if work_directory.is_some() {
                div { class: "event-empty",
                    "在资源管理器中双击 missionsEvents 下的 .txt 脚本进行编辑"
                }
            } else {
                div { class: "event-empty", "打开工作区后可编辑国策事件脚本" }
            }
        }
    }
}
