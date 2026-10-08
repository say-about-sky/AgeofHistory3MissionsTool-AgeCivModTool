use dioxus::prelude::*;

#[component]
pub fn Frame(
	on_open: EventHandler<()>,
	on_new: EventHandler<()>,
    on_save: EventHandler<()>,
	on_toggle_files: EventHandler<()>,
	on_toggle_events: EventHandler<()>,
        on_manage_all_files: EventHandler<()>,
	on_extract_apk: EventHandler<()>,
	on_import_apk: EventHandler<()>,
	on_export_apk: EventHandler<()>,
	/// 「指定补全数据 APK」：写入工作区根标记，供事件编辑器补全读取。
	on_set_lookup_apk: EventHandler<()>,
	on_package_apk: EventHandler<()>,
	on_sign_apk: EventHandler<()>,
	on_import_signing_key: EventHandler<()>,
	has_workspace: bool,
    workspace_name: String,
	is_loading: bool,
        is_android: bool,
        all_files_access_granted: Option<bool>,
	tabs: Vec<(String, String)>,
	active_tab_id: Option<String>,
	on_select_tab: EventHandler<String>,
	on_close_tab: EventHandler<String>,
	/// 全局撤销/重做（标题栏按钮，Ctrl+Z / Ctrl+Y 同步）。
	can_undo: bool,
	can_redo: bool,
	/// 当前撤销目标分区名称（画布 / 事件编辑 / 资源管理器）。
	zone_label: String,
	on_undo: EventHandler<()>,
	on_redo: EventHandler<()>,
) -> Element {
	let mut file_menu_open = use_signal(|| false);
	// “apk操作” 子菜单（嵌套在文件菜单内）。
	let mut apk_menu_open = use_signal(|| false);
	// “导入-导出” 子菜单（嵌套在文件菜单内，位于 apk操作 下方）。
	let mut io_menu_open = use_signal(|| false);
	// 预克隆标签数据，供 rsx 循环体中的多个闭包各自持有。
	// 注意：select_id 与 close_id 都必须填 id，不能填 title，否则关闭匹配不到标签。
	let tab_items: Vec<(String, String, String, String)> = tabs
		.iter()
		.map(|(id, title)| (id.clone(), title.clone(), id.clone(), id.clone()))
		.collect();

	rsx! {
        header { class: "editor-header",
            if *file_menu_open.read() {
                div {
                    class: "menu-dismiss",
                    aria_hidden: "true",
                    onclick: move |_| {
                        file_menu_open.set(false);
                        apk_menu_open.set(false);
                        io_menu_open.set(false);
                    },
                }
            }
            div { class: "editor-titlebar",
                div { class: "app-mark", "🗃️" }
                span { class: "app-title", "{workspace_name}" }
                div { class: "titlebar-history",
                    button {
                        class: "titlebar-history-button",
                        r#type: "button",
                        disabled: !can_undo,
                        title: "上一步 - {zone_label} (Ctrl+Z)",
                        aria_label: "上一步",
                        onclick: move |_| on_undo.call(()),
                        "↶"
                    }
                    button {
                        class: "titlebar-history-button",
                        r#type: "button",
                        disabled: !can_redo,
                        title: "下一步 - {zone_label} (Ctrl+Y)",
                        aria_label: "下一步",
                        onclick: move |_| on_redo.call(()),
                        "↷"
                    }
                    button {
                        class: "titlebar-history-button",
                        r#type: "button",
                        disabled: !has_workspace || is_loading,
                        title: "保存工作区 (Ctrl+S)",
                        aria_label: "保存工作区",
                        onclick: move |_| on_save.call(()),
                        "💾"
                    }
                }
            }
            nav { class: "menu-bar", "aria-label": "主菜单",
                div { class: "menu-wrap",
                    button {
                        class: "menu-trigger",
                        r#type: "button",
                        aria_expanded: "{file_menu_open}",
                        onclick: move |_| file_menu_open.toggle(),
                        "文件"
                    }
                    if *file_menu_open.read() {
                        div { class: "menu-popover",
                            button {
                                class: "menu-item",
                                r#type: "button",
                                disabled: is_loading,
                                onclick: move |_| {
                                    file_menu_open.set(false);
                                    on_new.call(());
                                },
                                span { class: "menu-symbol", "+" }
                                span { "新建工作区..." }
                                span { class: "menu-shortcut", "Ctrl+N" }
                            }
                            button {
                                class: "menu-item",
                                r#type: "button",
                                disabled: is_loading,
                                onclick: move |_| {
                                    file_menu_open.set(false);
                                    on_open.call(());
                                },
                                span { class: "menu-symbol", "↗" }
                                span { "打开工作区..." }
                                span { class: "menu-shortcut", "Ctrl+O" }
                            }
                            if is_android {
                                button {
                                    class: "menu-item",
                                    r#type: "button",
                                    onclick: move |_| {
                                        file_menu_open.set(false);
                                        on_manage_all_files.call(());
                                    },
                                    span { class: "menu-symbol", "⇧" }
                                    span { "全盘文件访问权限..." }
                                    span { class: "menu-shortcut",
                                        if all_files_access_granted == Some(true) {
                                            "已授权"
                                        } else {
                                            "未授权"
                                        }
                                    }
                                }
                            }
                            div { class: "menu-separator" }
                            button {
                                class: if *apk_menu_open.read() { "menu-item menu-item-parent open" } else { "menu-item menu-item-parent" },
                                r#type: "button",
                                disabled: is_loading,
                                onclick: move |_| {
                                    apk_menu_open.toggle();
                                    io_menu_open.set(false);
                                },
                                span { class: "menu-symbol", "▣" }
                                span { "apk操作" }
                                span { class: "menu-caret",
                                    if *apk_menu_open.read() {
                                        "▾"
                                    } else {
                                        "▸"
                                    }
                                }
                            }
                            if *apk_menu_open.read() {
                                div { class: "menu-submenu",
                                    button {
                                        class: "menu-item",
                                        r#type: "button",
                                        disabled: !has_workspace || is_loading,
                                        onclick: move |_| {
                                            file_menu_open.set(false);
                                            apk_menu_open.set(false);
                                            on_extract_apk.call(());
                                        },
                                        span { class: "menu-symbol", "⇩" }
                                        span { "解压 apk 到工作区" }
                                    }
                                    button {
                                        class: "menu-item",
                                        r#type: "button",
                                        disabled: !has_workspace || is_loading,
                                        onclick: move |_| {
                                            file_menu_open.set(false);
                                            apk_menu_open.set(false);
                                            on_package_apk.call(());
                                        },
                                        span { class: "menu-symbol", "⇧" }
                                        span { "打包 apk" }
                                    }
                                    button {
                                        class: "menu-item",
                                        r#type: "button",
                                        disabled: !has_workspace || is_loading,
                                        onclick: move |_| {
                                            file_menu_open.set(false);
                                            apk_menu_open.set(false);
                                            on_sign_apk.call(());
                                        },
                                        span { class: "menu-symbol", "✒" }
                                        span { "签名 apk" }
                                    }
                                    button {
                                        class: "menu-item",
                                        r#type: "button",
                                        disabled: !has_workspace || is_loading,
                                        onclick: move |_| {
                                            file_menu_open.set(false);
                                            apk_menu_open.set(false);
                                            on_import_signing_key.call(());
                                        },
                                        span { class: "menu-symbol", "⚙" }
                                        span { "添加默认密钥" }
                                    }
                                }
                            }
                            button {
                                class: if *io_menu_open.read() { "menu-item menu-item-parent open" } else { "menu-item menu-item-parent" },
                                r#type: "button",
                                disabled: is_loading,
                                onclick: move |_| {
                                    io_menu_open.toggle();
                                    apk_menu_open.set(false);
                                },
                                span { class: "menu-symbol", "⇄" }
                                span { "导入-导出" }
                                span { class: "menu-caret",
                                    if *io_menu_open.read() {
                                        "▾"
                                    } else {
                                        "▸"
                                    }
                                }
                            }
                            if *io_menu_open.read() {
                                div { class: "menu-submenu",
                                    button {
                                        class: "menu-item",
                                        r#type: "button",
                                        disabled: !has_workspace || is_loading,
                                        onclick: move |_| {
                                            file_menu_open.set(false);
                                            io_menu_open.set(false);
                                            on_import_apk.call(());
                                        },
                                        span { class: "menu-symbol", "⇲" }
                                        span { "从 apk 中导入" }
                                    }
                                    button {
                                        class: "menu-item",
                                        r#type: "button",
                                        disabled: !has_workspace || is_loading,
                                        onclick: move |_| {
                                            file_menu_open.set(false);
                                            io_menu_open.set(false);
                                            on_export_apk.call(());
                                        },
                                        span { class: "menu-symbol", "⇱" }
                                        span { "导出到 apk" }
                                    }
                                    button {
                                        class: "menu-item",
                                        r#type: "button",
                                        disabled: !has_workspace || is_loading,
                                        title: "选择游戏 APK 作为补全数据源（工作区缺少游戏数据文件时使用）",
                                        onclick: move |_| {
                                            file_menu_open.set(false);
                                            io_menu_open.set(false);
                                            on_set_lookup_apk.call(());
                                        },
                                        span { class: "menu-symbol", "⌖" }
                                        span { "指定补全数据 APK" }
                                    }
                                }
                            }
                        }
                    }
                }
                button {
                    class: "menu-trigger",
                    r#type: "button",
                    onclick: move |_| on_toggle_files.call(()),
                    span { class: "toolbar-icon", "▤" }
                    "资源管理器"
                }
                if !tabs.is_empty() {
                    div {
                        class: "tab-strip",
                        role: "tablist",
                        aria_label: "国策树标签页",
                        for (id , title , select_id , close_id) in tab_items {
                            div {
                                class: if active_tab_id.as_ref() == Some(&id) { "editor-tab active" } else { "editor-tab" },
                                role: "tab",
                                aria_selected: "{active_tab_id.as_ref() == Some(&id)}",
                                title: "{title}",
                                span {
                                    class: "tab-select",
                                    title: "{title}",
                                    onclick: move |_| on_select_tab.call(select_id.clone()),
                                    span { class: "tab-label", "{title}" }
                                }
                                button {
                                    class: "tab-close",
                                    r#type: "button",
                                    aria_label: "关闭 {title}",
                                    onclick: move |_| {
                                        dioxus_logger::tracing::info!("TAB-CLOSE clicked: {close_id}");
                                        on_close_tab.call(close_id.clone());
                                    },
                                    "✕"
                                }
                            }
                        }
                    }
                }
                if is_loading {
                    span { class: "header-status", role: "status", "正在载入..." }
                }
                button {
                    class: "menu-trigger events-toggle",
                    r#type: "button",
                    onclick: move |_| on_toggle_events.call(()),
                    span { class: "toolbar-icon", "✎" }
                    "事件编辑器"
                }
            }
        }
    }
}
