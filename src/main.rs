mod app;

use app::App;
use dioxus::prelude::*;
use dioxus_logger::tracing::Level;

fn main() {
    // println!("Hello, world!");
    dioxus_logger::init(Level::INFO).expect("failed to init logger");
    launch(App);
}
