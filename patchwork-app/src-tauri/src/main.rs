#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

fn main() {
    if patchwork_app_tauri::remote_cache::wrapper_requested() {
        patchwork_app_tauri::remote_cache::wrapper_main();
        return;
    }
    patchwork_app_tauri::run();
}
