pub mod auth_types;

#[cfg(feature = "csr")]
mod app;
#[cfg(feature = "csr")]
mod deptree;

#[cfg(feature = "csr")]
use leptos::{mount::mount_to_body, prelude::*};
#[cfg(feature = "csr")]
use wasm_bindgen::prelude::wasm_bindgen;

#[cfg(feature = "csr")]
#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    mount_to_body(|| view! { <app::WebApp /> });
}
