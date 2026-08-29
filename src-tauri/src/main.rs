// Desktop entry point. The whole Tauri builder lives in `src_tauri::app::run`
// so the iOS build can reach it: on mobile there is no `main`, the Xcode
// project links this crate as a static library and calls the `start_app`
// symbol that `#[tauri::mobile_entry_point]` emits from `run`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    src_tauri::app::run();
}
