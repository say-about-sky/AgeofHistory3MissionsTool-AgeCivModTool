#![allow(non_snake_case)]

use dioxus::prelude::*;

mod components;
mod window;
pub mod tauri_bridge;

pub fn App() -> Element {
    rsx! {
        document::Meta {
            name: "viewport",
            content: "width=device-width, initial-scale=1.0, maximum-scale=1.0, user-scalable=no",
        }
        window::Window { components::Work {} }
    }
}
