//! 国策事件脚本的"特化 Excel 表格"可视化编辑器。
//!
//! 每个脚本按来源拆成表格分区：
//! - 必填项目 / 可填项目 / 其他（未识别键，原样保留）；
//! - 触发条件块（可增删块，块内可增删条件行）；
//! - 收益选项块（可增删选项，选项内可增删效果行）。
//!
//! 表格列为：键 | 值 | 说明 | 删除。
//! 键输入提供 Schema 自动补全，值按类型渲染（bool 为下拉框，其余文本），
//! 与 Schema 类型不符的单元格红框提示，保存时仍会原样写入。

use dioxus::prelude::*;

use super::event_parser::{EntryLine, MissionEvent, NextOp, TriggerBlock, TriggerKind};
use super::event_schema::{self, FieldCategory, ValueSpec, ValueType};
use super::game_text::{has_game_codes, GameTextPreview};

/// 值单元格的输入方式。
#[derive(Clone, Copy, PartialEq, Eq)]
enum CellInput {
	Text,
	Bool,
}

fn cell_input_for(key: &str) -> CellInput {
	match event_schema::lookup(key).map(|spec| spec.spec) {
		Some(ValueSpec::Single(ValueType::Bool)) => CellInput::Bool,
		_ => CellInput::Text,
	}
}

fn note_for(key: &str) -> String {
	if let Some(spec) = event_schema::lookup(key) {
		return spec.annotation.to_string();
	}
	if let Some((_, fix)) = event_schema::KNOWN_TYPOS.iter().find(|(typo, _)| *typo == key) {
		return format!("疑似拼写错误，应为 {fix}（保存时原样保留）");
	}
	"未识别键（保存时原样保留）".to_string()
}

fn is_invalid(key: &str, value: &str) -> bool {
	event_schema::lookup(key).is_some_and(|spec| !event_schema::is_valid_value(spec.spec, value))
}

/// 头部字段分组（按 Schema 分类）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum HeaderGroup {
	Required,
	Optional,
	Unknown,
}

fn header_group_of(key: &str) -> HeaderGroup {
	match event_schema::lookup(key).map(|spec| spec.category) {
		Some(FieldCategory::Required | FieldCategory::Header) => HeaderGroup::Required,
		Some(FieldCategory::Optional) => HeaderGroup::Optional,
		_ => HeaderGroup::Unknown,
	}
}

fn push_used_keys<'a>(entries: impl Iterator<Item = &'a EntryLine>, keys: &mut Vec<String>) {
	for entry in entries {
		if !keys.iter().any(|key| key == &entry.key) {
			keys.push(entry.key.clone());
		}
	}
}

/// 头部三组的自动补全候选（必填+可填 Schema 键，加当前使用的未知键）。
fn header_suggestions(event: &MissionEvent) -> Vec<String> {
	let mut keys: Vec<String> = event_schema::all_specs()
		.filter(|spec| {
			matches!(
				spec.category,
				FieldCategory::Required | FieldCategory::Header | FieldCategory::Optional
			)
		})
		.map(|spec| spec.key.to_string())
		.collect();
	push_used_keys(event.header.iter(), &mut keys);
	keys
}

fn trigger_suggestions(event: &MissionEvent) -> Vec<String> {
	let mut keys: Vec<String> = event_schema::all_specs()
		.filter(|spec| spec.category == FieldCategory::Trigger)
		.map(|spec| spec.key.to_string())
		.collect();
	for block in &event.triggers {
		push_used_keys(block.conditions.iter(), &mut keys);
	}
	keys
}

fn effect_suggestions(event: &MissionEvent) -> Vec<String> {
	let mut keys: Vec<String> = event_schema::all_specs()
		.filter(|spec| spec.category == FieldCategory::Effect)
		.map(|spec| spec.key.to_string())
		.collect();
	for block in &event.options {
		push_used_keys(block.effects.iter(), &mut keys);
	}
	keys
}

/// 表格行（键 | 值 | 说明 | 删除）。`show_delete` 为 false 时删除格留空。
#[component]
fn GridRow(
	field_key: String,
	value: String,
	input_kind: CellInput,
	invalid: bool,
	note: String,
	datalist: &'static str,
	show_delete: bool,
	key_readonly: bool,
	on_key: EventHandler<String>,
	on_key_focus: EventHandler<()>,
	on_key_blur: EventHandler<()>,
	on_value: EventHandler<String>,
	on_remove: EventHandler<()>,
) -> Element {
	let show_preview = input_kind == CellInput::Text && has_game_codes(&value);
	let key_title = if key_readonly { "必填项目的键固定不可修改" } else { "" };
	rsx! {
        div { class: "event-grid-row",
            div { class: "event-cell event-cell-key",
                input {
                    class: "event-cell-input",
                    list: "{datalist}",
                    value: "{field_key}",
                    spellcheck: "false",
                    readonly: key_readonly,
                    title: "{key_title}",
                    oninput: move |evt: FormEvent| on_key.call(evt.value()),
                    onfocus: move |_| on_key_focus.call(()),
                    onblur: move |_| on_key_blur.call(()),
                }
            }
            div { class: "event-cell event-cell-value",
                if input_kind == CellInput::Bool {
                    select {
                        class: if invalid { "event-cell-input invalid" } else { "event-cell-input" },
                        value: "{value}",
                        oninput: move |evt: FormEvent| on_value.call(evt.value()),
                        option { value: "", "（留空）" }
                        option { value: "true", "true" }
                        option { value: "false", "false" }
                    }
                } else {
                    input {
                        class: if invalid { "event-cell-input invalid" } else { "event-cell-input" },
                        value: "{value}",
                        spellcheck: "false",
                        oninput: move |evt: FormEvent| on_value.call(evt.value()),
                    }
                    if show_preview {
                        GameTextPreview { text: value.clone() }
                    }
                }
            }
            div { class: "event-cell event-cell-note",
                span { class: "event-note-text", title: "{note}", "{note}" }
            }
            if show_delete {
                div { class: "event-cell event-cell-del",
                    button {
                        class: "event-del",
                        r#type: "button",
                        title: "删除此行",
                        aria_label: "删除此行",
                        onclick: move |_| on_remove.call(()),
                        "×"
                    }
                }
            } else {
                div { class: "event-cell event-cell-del" }
            }
        }
    }
}

/// 表格表头（键 | 值 | 说明 | 空）。
#[component]
fn GridHead() -> Element {
	rsx! {
        div { class: "event-grid-head",
            span { "键" }
            span { "值" }
            span { "说明" }
            span {}
        }
    }
}

/// 一个表格分区（标题 + 表头 + 行 + 添加按钮）。
/// `can_add`/`can_remove` 控制该分区是否允许增删行（如必填项目固定不可增删）。
#[component]
fn GridSection(
	title: String,
	rows: Vec<(usize, String, String)>,
	datalist: &'static str,
	can_add: bool,
	can_remove: bool,
	editable_key: bool,
	on_key: EventHandler<(usize, String)>,
	on_key_focus: EventHandler<usize>,
	on_key_blur: EventHandler<()>,
	on_value: EventHandler<(usize, String)>,
	on_remove: EventHandler<usize>,
	on_add: EventHandler<()>,
	add_label: String,
) -> Element {
	rsx! {
        section { class: "event-section",
            div { class: "event-section-title",
                span { "{title}" }
                if can_add {
                    button {
                        class: "event-title-add",
                        r#type: "button",
                        title: "{add_label}",
                        aria_label: "{add_label}",
                        onclick: move |_| on_add.call(()),
                        "＋"
                    }
                }
            }
            GridHead {}
            for (index , key , value) in rows {
                GridRow {
                    field_key: key.clone(),
                    value: value.clone(),
                    input_kind: cell_input_for(&key),
                    invalid: is_invalid(&key, &value),
                    note: note_for(&key),
                    datalist,
                    show_delete: can_remove,
                    key_readonly: !editable_key,
                    on_key: move |new_key: String| on_key.call((index, new_key)),
                    on_key_focus: move |_| on_key_focus.call(index),
                    on_key_blur: move |_| on_key_blur.call(()),
                    on_value: move |new_value: String| on_value.call((index, new_value)),
                    on_remove: move |_| on_remove.call(index),
                }
            }
            if can_add {
                button {
                    class: "event-add-row",
                    r#type: "button",
                    onclick: move |_| on_add.call(()),
                    "{add_label}"
                }
            }
        }
    }
}

/// 以可变方式访问当前事件；未载入时忽略。
/// Signal 按值复制后重新绑定为可变，使调用方闭包保持 Fn 语义、可被多个分区复用。
fn mutate_event(event: Signal<Option<MissionEvent>>, f: impl FnOnce(&mut MissionEvent)) {
	let mut event = event;
	event.with_mut(|current| {
		if let Some(data) = current.as_mut() {
			f(data);
		}
	});
}

/// 特化 Excel 表格编辑器。直接编辑 `MissionEvent` 信号。
#[component]
pub fn EventGrid(event: Signal<Option<MissionEvent>>) -> Element {
	let mut editing_key: Signal<Option<(usize, HeaderGroup)>> = use_signal(|| None);
	let snapshot = event.read().clone();
	let Some(data) = snapshot else {
		return rsx! {
            div { class: "event-grid-empty", "脚本尚未载入" }
        };
	};

	// ===== 头部字段分组 =====
	// 键正在编辑的行冻结在编辑开始时的分区里（输入过程中不重新归类，
	// 避免行从当前分区"消失"）；失焦后再按新键重新分组。
	let frozen_edit = *editing_key.read();
	let mut required_rows: Vec<(usize, String, String)> = Vec::new();
	let mut optional_rows: Vec<(usize, String, String)> = Vec::new();
	let mut unknown_rows: Vec<(usize, String, String)> = Vec::new();
	for (index, entry) in data.header.iter().enumerate() {
		let row = (index, entry.key.clone(), entry.value_or_default().to_string());
		let group = match frozen_edit {
			Some((frozen_index, frozen)) if frozen_index == index => frozen,
			_ => header_group_of(&entry.key),
		};
		match group {
			HeaderGroup::Required => required_rows.push(row),
			HeaderGroup::Optional => optional_rows.push(row),
			HeaderGroup::Unknown => unknown_rows.push(row),
		}
	}

	let header_datalist = header_suggestions(&data);
	let trigger_datalist = trigger_suggestions(&data);
	let effect_datalist = effect_suggestions(&data);

	// ===== 头部字段编辑 =====
	// 通过 mutate_event 共享访问，保证闭包是 Fn（可被多个表格分区复用）。
	let set_header_key = move |(index, key): (usize, String)| {
		mutate_event(event, |data| {
			if let Some(entry) = data.header.get_mut(index) {
				entry.key = key;
			}
		});
	};
	let set_header_value = move |(index, value): (usize, String)| {
		mutate_event(event, |data| {
			if let Some(entry) = data.header.get_mut(index) {
				entry.set_value(&value);
			}
		});
	};
	let remove_header = move |index: usize| {
		mutate_event(event, |data| {
			data.header.remove(index);
		});
	};
	let add_header_field = move |default_key: String| {
		mutate_event(event, |data| {
			data.header.push(EntryLine::new(&default_key, ""));
		});
	};

	// 记录"键编辑会话"：冻结该行的分区，失焦后恢复实时归类。
	let begin_key_edit = move |index: usize| {
		let group = {
			let guard = event.read();
			match &*guard {
				Some(data) => data
					.header
					.get(index)
					.map_or(HeaderGroup::Unknown, |entry| header_group_of(&entry.key)),
				None => HeaderGroup::Unknown,
			}
		};
		editing_key.set(Some((index, group)));
	};
	let end_key_edit = move |_: ()| {
		editing_key.set(None);
	};

	// ===== 触发块编辑 =====
	let set_trigger_kind = move |(block_index, kind): (usize, TriggerKind)| {
		mutate_event(event, |data| {
			if let Some(block) = data.triggers.get_mut(block_index) {
				block.kind = kind;
			}
		});
	};
	let set_trigger_join = move |(block_index, join): (usize, Option<NextOp>)| {
		mutate_event(event, |data| {
			if let Some(block) = data.triggers.get_mut(block_index) {
				block.join = join;
			}
		});
	};
	let set_condition_key = move |(block_index, row_index, key): (usize, usize, String)| {
		mutate_event(event, |data| {
			if let Some(entry) = data
				.triggers
				.get_mut(block_index)
				.and_then(|block| block.conditions.get_mut(row_index))
			{
				entry.key = key;
			}
		});
	};
	let set_condition_value = move |(block_index, row_index, value): (usize, usize, String)| {
		mutate_event(event, |data| {
			if let Some(entry) = data
				.triggers
				.get_mut(block_index)
				.and_then(|block| block.conditions.get_mut(row_index))
			{
				entry.set_value(&value);
			}
		});
	};
	let remove_condition = move |(block_index, row_index): (usize, usize)| {
		mutate_event(event, |data| {
			if let Some(block) = data.triggers.get_mut(block_index) {
				block.conditions.remove(row_index);
			}
		});
	};
	let add_condition = move |block_index: usize| {
		mutate_event(event, |data| {
			if let Some(block) = data.triggers.get_mut(block_index) {
				block.conditions.push(EntryLine::new("is_civ", ""));
			}
		});
	};
	let remove_trigger_block = move |block_index: usize| {
		mutate_event(event, |data| {
			data.triggers.remove(block_index);
		});
	};
	let add_trigger_block = move |_| {
		mutate_event(event, |data| {
			data.triggers.push(TriggerBlock {
				blank_before: 1,
				kind: TriggerKind::And,
				join: Some(NextOp::And),
				conditions: vec![EntryLine::new("is_civ", "")],
			});
		});
	};

	// ===== 选项块编辑 =====
	let set_option_name = move |(option_index, name): (usize, String)| {
		mutate_event(event, |data| {
			if let Some(block) = data.options.get_mut(option_index) {
				block.name = Some(name);
			}
		});
	};
	let set_effect_key = move |(option_index, row_index, key): (usize, usize, String)| {
		mutate_event(event, |data| {
			if let Some(entry) = data
				.options
				.get_mut(option_index)
				.and_then(|block| block.effects.get_mut(row_index))
			{
				entry.key = key;
			}
		});
	};
	let set_effect_value = move |(option_index, row_index, value): (usize, usize, String)| {
		mutate_event(event, |data| {
			if let Some(entry) = data
				.options
				.get_mut(option_index)
				.and_then(|block| block.effects.get_mut(row_index))
			{
				entry.set_value(&value);
			}
		});
	};
	let remove_effect = move |(option_index, row_index): (usize, usize)| {
		mutate_event(event, |data| {
			if let Some(block) = data.options.get_mut(option_index) {
				block.effects.remove(row_index);
			}
		});
	};
	let add_effect = move |option_index: usize| {
		mutate_event(event, |data| {
			if let Some(block) = data.options.get_mut(option_index) {
				block.effects.push(EntryLine::new("legacy", ""));
			}
		});
	};
	let remove_option_block = move |option_index: usize| {
		mutate_event(event, |data| {
			data.options.remove(option_index);
		});
	};
	let add_option_block = move |_| {
		mutate_event(event, |data| {
			data.options.push(super::event_parser::OptionBlock {
				blank_before: 1,
				name: Some(String::new()),
				effects: Vec::new(),
			});
		});
	};

	let trigger_blocks = data.triggers.clone();
	let option_blocks = data.options.clone();

	rsx! {
        div { class: "event-grid",
            // 自动补全候选（每个分区类型一个 datalist）
            datalist { id: "evdl-header",
                for key in header_datalist {
                    option { value: "{key}" }
                }
            }
            datalist { id: "evdl-trigger",
                for key in trigger_datalist {
                    option { value: "{key}" }
                }
            }
            datalist { id: "evdl-effect",
                for key in effect_datalist {
                    option { value: "{key}" }
                }
            }

            GridSection {
                title: "必填项目".to_string(),
                rows: required_rows,
                datalist: "evdl-header",
                can_add: false,
                can_remove: false,
                editable_key: false,
                on_key: set_header_key,
                on_key_focus: begin_key_edit,
                on_key_blur: end_key_edit,
                on_value: set_header_value,
                on_remove: remove_header,
                on_add: move |_| add_header_field("id".to_string()),
                add_label: "＋ 添加必填项".to_string(),
            }
            GridSection {
                title: "可填项目".to_string(),
                rows: optional_rows,
                datalist: "evdl-header",
                can_add: true,
                can_remove: true,
                editable_key: true,
                on_key: set_header_key,
                on_key_focus: begin_key_edit,
                on_key_blur: end_key_edit,
                on_value: set_header_value,
                on_remove: remove_header,
                on_add: move |_| add_header_field("mission_image".to_string()),
                add_label: "＋ 添加可选项".to_string(),
            }
            GridSection {
                title: "其他（未识别字段）".to_string(),
                rows: unknown_rows,
                datalist: "evdl-header",
                can_add: true,
                can_remove: true,
                editable_key: true,
                on_key: set_header_key,
                on_key_focus: begin_key_edit,
                on_key_blur: end_key_edit,
                on_value: set_header_value,
                on_remove: remove_header,
                on_add: move |_| add_header_field(String::new()),
                add_label: "＋ 添加字段".to_string(),
            }

            for (block_index , block) in trigger_blocks.iter().enumerate() {
                section { class: "event-section",
                    div { class: "event-section-title",
                        span { "触发条件 #{block_index + 1}" }
                        div { class: "event-block-controls",
                            label { "块类型" }
                            select {
                                class: "event-block-select",
                                value: "{block.kind.open_token()}",
                                oninput: move |evt: FormEvent| {
                                    set_trigger_kind((block_index, TriggerKind::from_open_token(&evt.value())));
                                },
                                option { value: "trigger_and", "and" }
                                option { value: "trigger_or", "or" }
                                option { value: "trigger_and_not", "and_not" }
                            }
                            label { "连接" }
                            select {
                                class: "event-block-select",
                                value: "{block.join.map_or(String::new(), |join| join.token().to_string())}",
                                oninput: move |evt: FormEvent| {
                                    let join = NextOp::from_token(&evt.value());
                                    set_trigger_join((block_index, join));
                                },
                                option { value: "", "（不写）" }
                                option { value: "next_and", "next_and" }
                                option { value: "next_or", "next_or" }
                                option { value: "next_and_not", "next_and_not" }
                            }
                            button {
                                class: "event-del event-block-del",
                                r#type: "button",
                                title: "删除此块",
                                aria_label: "删除此块",
                                onclick: move |_| remove_trigger_block(block_index),
                                "×"
                            }
                        }
                    }
                    GridHead {}
                    for (row_index , entry) in block.conditions.iter().enumerate() {
                        GridRow {
                            field_key: entry.key.clone(),
                            value: entry.value_or_default().to_string(),
                            input_kind: cell_input_for(&entry.key),
                            invalid: is_invalid(&entry.key, entry.value_or_default()),
                            note: note_for(&entry.key),
                            datalist: "evdl-trigger",
                            show_delete: true,
                            key_readonly: false,
                            on_key: move |new_key: String| set_condition_key((block_index, row_index, new_key)),
                            on_key_focus: move |_| {},
                            on_key_blur: move |_| {},
                            on_value: move |new_value: String| set_condition_value((block_index, row_index, new_value)),
                            on_remove: move |_| remove_condition((block_index, row_index)),
                        }
                    }
                    button {
                        class: "event-add-row",
                        r#type: "button",
                        onclick: move |_| add_condition(block_index),
                        "＋ 添加条件"
                    }
                }
            }

            for (option_index , block) in option_blocks.iter().enumerate() {
                section { class: "event-section",
                    div { class: "event-section-title",
                        span { "收益选项 #{option_index + 1}" }
                        button {
                            class: "event-del event-block-del",
                            r#type: "button",
                            title: "删除此选项",
                            aria_label: "删除此选项",
                            onclick: move |_| remove_option_block(option_index),
                            "×"
                        }
                    }
                    div { class: "event-option-name",
                        label { "name" }
                        input {
                            class: "event-cell-input",
                            value: "{block.name.clone().unwrap_or_default()}",
                            spellcheck: "false",
                            oninput: move |evt: FormEvent| set_option_name((option_index, evt.value())),
                        }
                        if has_game_codes(block.name.as_deref().unwrap_or_default()) {
                            GameTextPreview { text: block.name.clone().unwrap_or_default() }
                        }
                    }
                    GridHead {}
                    for (row_index , entry) in block.effects.iter().enumerate() {
                        GridRow {
                            field_key: entry.key.clone(),
                            value: entry.value_or_default().to_string(),
                            input_kind: cell_input_for(&entry.key),
                            invalid: is_invalid(&entry.key, entry.value_or_default()),
                            note: note_for(&entry.key),
                            datalist: "evdl-effect",
                            show_delete: true,
                            key_readonly: false,
                            on_key: move |new_key: String| set_effect_key((option_index, row_index, new_key)),
                            on_key_focus: move |_| {},
                            on_key_blur: move |_| {},
                            on_value: move |new_value: String| set_effect_value((option_index, row_index, new_value)),
                            on_remove: move |_| remove_effect((option_index, row_index)),
                        }
                    }
                    button {
                        class: "event-add-row",
                        r#type: "button",
                        onclick: move |_| add_effect(option_index),
                        "＋ 添加效果"
                    }
                }
            }

            section { class: "event-section event-section-actions",
                button {
                    class: "event-add-row",
                    r#type: "button",
                    onclick: add_trigger_block,
                    "＋ 添加触发条件块"
                }
                button {
                    class: "event-add-row",
                    r#type: "button",
                    onclick: add_option_block,
                    "＋ 添加收益选项"
                }
            }
        }
    }
}
