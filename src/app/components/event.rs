use dioxus::prelude::*;
use serde::Serialize;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

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
	file_name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EventFileWriteArgs {
	work_directory: String,
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
struct ScopedRename {
	folder_id: String,
	from_path: String,
	to_path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EventRenameArgs {
	work_directory: String,
	old_file_name: String,
	new_file_name: String,
}

async fn scoped_request<T: Serialize>(command: &str, request: &T) -> Result<JsValue, String> {
	let request = serde_wasm_bindgen::to_value(request).map_err(|error| error.to_string())?;
	let args = js_sys::Object::new();
	js_sys::Reflect::set(&args, &JsValue::from_str("req"), &request)
		.map_err(|error| format!("准备事件文件请求失败：{error:?}"))?;
	JsFuture::from(invoke(
		&format!("plugin:scoped-storage|{command}"),
		args.into(),
	))
	.await
	.map_err(|error| format!("访问 Android 事件文件失败：{error:?}"))
}

fn event_file_path(root_path: &str, file_name: &str) -> String {
	if root_path.is_empty() {
		format!("missions/missionsEvents/{file_name}")
	} else {
		format!("{root_path}/missions/missionsEvents/{file_name}")
	}
}

fn event_dir_path(root_path: &str) -> String {
	if root_path.is_empty() {
		"missions/missionsEvents".to_string()
	} else {
		format!("{root_path}/missions/missionsEvents")
	}
}

async fn load_event_text(directory: &WorkDirectory, file_name: &str) -> Result<String, String> {
	if let Some(folder_id) = &directory.folder_id {
		let args = serde_wasm_bindgen::to_value(&ScopedReadTextInDir {
			folder_id: folder_id.clone(),
			dir_path: event_dir_path(&directory.root_path),
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
	file_name: &str,
	contents: &str,
) -> Result<(), String> {
	if let Some(folder_id) = &directory.folder_id {
		let args = serde_wasm_bindgen::to_value(&ScopedWriteTextInDir {
			folder_id: folder_id.clone(),
			dir_path: event_dir_path(&directory.root_path),
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
	file_name: &str,
) -> Result<(), String> {
	if let Some(folder_id) = &directory.folder_id {
		let path = event_file_path(&directory.root_path, file_name);
		scoped_request(
			"remove_file",
			&ScopedRemoveFile {
				folder_id: folder_id.clone(),
				path,
			},
		)
		.await?;
		Ok(())
	} else {
		let args = serde_wasm_bindgen::to_value(&EventFileArgs {
			work_directory: directory.root_path.clone(),
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
	old_file_name: &str,
	new_file_name: &str,
) -> Result<(), String> {
	if let Some(folder_id) = &directory.folder_id {
		let from_path = event_file_path(&directory.root_path, old_file_name);
		let to_path = event_file_path(&directory.root_path, new_file_name);
		scoped_request(
			"rename",
			&ScopedRename {
				folder_id: folder_id.clone(),
				from_path,
				to_path,
			},
		)
		.await?;
		Ok(())
	} else {
		let args = serde_wasm_bindgen::to_value(&EventRenameArgs {
			work_directory: directory.root_path.clone(),
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
	save_request: Signal<u64>,
	open_request: Signal<Option<(String, u64)>>,
	rename_request: Signal<Option<(String, String)>>,
	release_request: Signal<u64>,
	on_selection_change: EventHandler<Option<String>>,
	width: f64,
) -> Element {
	let mut selected = use_signal(|| None::<String>);
	let mut event = use_signal(|| None::<MissionEvent>);
	let mut original = use_signal(String::new);
	let mut status = use_signal(String::new);
	let busy = use_signal(|| false);
	let mut last_save_request = use_signal(|| *save_request.read());

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
		let mut busy = busy;
		let mut selected = selected;
		let mut event = event;
		let mut original = original;
		let mut status = status;
		spawn(async move {
			busy.set(true);
			status.set("正在读取...".to_string());
			event.set(None);
			match load_event_text(&directory, &file_name).await {
				Ok(text) => {
					selected.set(Some(file_name.clone()));
					original.set(text.clone());
					event.set(Some(MissionEvent::parse(&text)));
					status.set(String::new());
					on_selection_change
						.call(Some(format!("missions/missionsEvents/{file_name}")));
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
			match save_event_text(&directory, &file_name, &text).await {
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
