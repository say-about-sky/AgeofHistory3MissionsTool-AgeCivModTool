use js_sys::Promise;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
	#[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"])]
	pub fn invoke(cmd: &str, args: JsValue) -> Promise;

	#[wasm_bindgen(js_namespace = ["window", "__TAURI__", "dialog"], js_name = open)]
	pub fn open_directory_dialog(options: JsValue) -> Promise;
}
