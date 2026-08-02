use leptos::{mount::mount_to_body, prelude::*};
use wasm_bindgen::prelude::wasm_bindgen;

mod app;
mod home;
mod icons;
mod model;
mod settings;
mod tauri_bridge;
mod terminal;

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    mount_to_body(|| view! { <app::App /> });
}
