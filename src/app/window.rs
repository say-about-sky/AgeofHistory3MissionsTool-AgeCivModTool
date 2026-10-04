use dioxus::prelude::*;

#[component]
pub fn Window(children: Element) -> Element {
	rsx! {
        document::Link { rel: "stylesheet", href: asset!("/assets/styles.css") }
        div { class: "editor-window", {children} }
    }
}
