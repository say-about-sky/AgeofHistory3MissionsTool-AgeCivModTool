#![allow(non_snake_case)]

use dioxus::prelude::*;

mod components;
mod window;
pub mod tauri_bridge;

pub fn App() -> Element {
    rsx! {
        window::Window { components::Work {} }
    }
}
