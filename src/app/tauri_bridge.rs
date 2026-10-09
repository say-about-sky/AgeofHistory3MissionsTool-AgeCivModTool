use js_sys::Promise;
use serde::Deserialize;
use wasm_bindgen::{prelude::*, JsCast};
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen]
extern "C" {
	#[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"])]
	pub fn invoke(cmd: &str, args: JsValue) -> Promise;

	/// 打开系统文件/目录选择器（`options.directory` 控制模式）。
	#[wasm_bindgen(js_namespace = ["window", "__TAURI__", "dialog"], js_name = open)]
	pub fn open_dialog(options: JsValue) -> Promise;

	/// 监听应用事件（`window.__TAURI__.event.listen`）。
	#[wasm_bindgen(js_namespace = ["window", "__TAURI__", "event"], js_name = listen)]
	fn listen_event(event: &str, handler: &JsValue) -> Promise;
}

/// APK 操作进度事件负载（Rust 命令 `emit` 与 Android 插件 `trigger` 格式一致）。
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApkProgressPayload {
	/// 操作类型：extract / package / sign（保留字段，便于后续细分展示）。
	#[allow(dead_code)]
	pub kind: String,
	pub stage: String,
	pub unit: String,
	pub completed: f64,
	pub total: f64,
}

/// 监听 APK 操作进度（解压/打包/签名）。
///
/// 进度由 Rust 命令通过 `apk-progress` 事件上报（`window.__TAURI__.event.listen`）；
/// 事件只在应用生命周期内注册一次（调用方用 `use_hook` 包一次）。
pub fn listen_apk_progress(on_progress: impl FnMut(ApkProgressPayload) + 'static) {
	let callback = std::rc::Rc::new(std::cell::RefCell::new(on_progress));

	// 回调参数为事件对象（{ event, id, payload }）。
	let event_listener = Closure::<dyn FnMut(JsValue)>::new(move |event: JsValue| {
		let Ok(payload) = js_sys::Reflect::get(&event, &JsValue::from_str("payload")) else {
			return;
		};
		if let Ok(parsed) = serde_wasm_bindgen::from_value::<ApkProgressPayload>(payload) {
			(callback.borrow_mut())(parsed);
		}
	});
	let _ = listen_event("apk-progress", event_listener.as_ref());
	event_listener.forget();
}

/// 监听工作区目录变化事件（外部修改后自动刷新资源管理器列表）。
///
/// 事件无负载，由后端 `start_workspace_watch` 启动的轮询在目录签名变化时发送；
/// 事件只在应用生命周期内注册一次（调用方用 `use_hook` 包一次）。
/// **回调在 dioxus 作用域之外执行**：只允许做不依赖作用域的操作（如信号 `.set()`），
/// 禁止 `spawn` / 读信号 / 调用 hook——否则会触发 dioxus 运行时 panic。
pub fn listen_workspace_changed(on_changed: impl FnMut() + 'static) {
	let callback = std::rc::Rc::new(std::cell::RefCell::new(on_changed));
	let event_listener = Closure::<dyn FnMut(JsValue)>::new(move |_event: JsValue| {
		(callback.borrow_mut())();
	});
	let _ = listen_event("workspace-changed", event_listener.as_ref());
	event_listener.forget();
}

/// 资源管理器文件操作进度事件负载（复制/移动/删除大目录时后端上报）。
/// `total = 0` 表示总量未知（SAF 模式），前端显示不确定进度条。
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileOpProgressPayload {
	/// 操作类型：copy / move / delete（展示用）。
	#[allow(dead_code)]
	pub kind: String,
	pub stage: String,
	pub completed: u64,
	pub total: u64,
}

/// 监听资源管理器文件操作进度（复制/移动/删除）。
///
/// 进度由后端命令通过 `file-op-progress` 事件上报；事件只在应用生命周期内注册
/// 一次（调用方用 `use_hook` 包一次）。**回调在 dioxus 作用域之外执行**：
/// 只能做不依赖作用域的操作（仅 `Signal::set`），禁止 `spawn`/读信号/调用 hook。
pub fn listen_file_op_progress(on_progress: impl FnMut(FileOpProgressPayload) + 'static) {
	let callback = std::rc::Rc::new(std::cell::RefCell::new(on_progress));

	let event_listener = Closure::<dyn FnMut(JsValue)>::new(move |event: JsValue| {
		let Ok(payload) = js_sys::Reflect::get(&event, &JsValue::from_str("payload")) else {
			return;
		};
		if let Ok(parsed) = serde_wasm_bindgen::from_value::<FileOpProgressPayload>(payload) {
			(callback.borrow_mut())(parsed);
		}
	});
	let _ = listen_event("file-op-progress", event_listener.as_ref());
	event_listener.forget();
}

/// 等待指定毫秒（`setTimeout` 包装成 Promise）：用于「短暂提示后自动收起」等场景。
pub async fn sleep_ms(milliseconds: u32) {
	let promise = Promise::new(&mut |resolve, _reject| {
		let callback = Closure::once_into_js(move || {
			let _ = resolve.call0(&JsValue::UNDEFINED);
		});
		if let Some(window) = web_sys::window() {
			let handler: &js_sys::Function = callback.unchecked_ref();
			let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
				handler,
				milliseconds as i32,
			);
		}
	});
	let _ = JsFuture::from(promise).await;
}
