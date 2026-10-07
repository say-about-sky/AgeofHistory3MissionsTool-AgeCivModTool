use js_sys::Promise;
use serde::Deserialize;
use wasm_bindgen::prelude::*;

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
