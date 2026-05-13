// Vlerv Tauri app entry point. Boots tauri::Builder with the deep-link plugin
// and the workspace/reader/security state.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;
use tauri::Emitter;
use tauri_plugin_deep_link::DeepLinkExt;

pub struct AppState {
    pub scanner: Mutex<src_tauri::workspace::Scanner>,
    pub roots: src_tauri::security::RootSet,
}

#[tauri::command]
fn list_dir(
    state: tauri::State<AppState>,
    path: String,
) -> Result<Vec<src_tauri::workspace::Entry>, String> {
    let scanner = state.scanner.lock().map_err(|e| e.to_string())?;
    scanner
        .list_dir(std::path::Path::new(&path))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn list_workspace_roots(
    state: tauri::State<AppState>,
    path: String,
) -> Result<Vec<src_tauri::workspace::Entry>, String> {
    let scanner = state.scanner.lock().map_err(|e| e.to_string())?;
    scanner
        .list_workspace_roots(std::path::Path::new(&path))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn read_file(
    path: String,
) -> Result<src_tauri::reader::FilePayload, String> {
    src_tauri::reader::read_file(std::path::Path::new(&path))
        .map_err(|e| e.to_string())
}

fn main() {
    let home = std::env::var("HOME").unwrap_or_else(|_| String::from("/"));
    let default_root = std::path::PathBuf::from(format!("{home}/workspace"));
    let roots = src_tauri::security::RootSet::new(vec![default_root]);

    tauri::Builder::default()
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_drag::init())
        .manage(AppState {
            scanner: Mutex::new(src_tauri::workspace::Scanner::new()),
            roots: roots.clone(),
        })
        .manage(roots)
        .invoke_handler(tauri::generate_handler![
            list_dir,
            list_workspace_roots,
            read_file,
        ])
        .setup(|app| {
            let app_handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                for url in event.urls() {
                    let url_str = url.to_string();
                    src_tauri::handle_deep_link(&url_str);
                    let _ = app_handle.emit("vlerv://deep-link", url_str);
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Tauri application");
}
