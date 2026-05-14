// Vlerv Tauri app entry point. Boots tauri::Builder with the deep-link plugin
// and the workspace/reader/security state.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;
use tauri::Emitter;
use tauri_plugin_deep_link::DeepLinkExt;

pub struct AppState {
    pub scanner: Mutex<src_tauri::workspace::Scanner>,
    pub roots: src_tauri::security::RootSet,
    pub watcher: Mutex<Option<src_tauri::watcher::WatcherHandle>>,
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

/// Start (or replace) the filesystem watcher rooted at `path`. Each successful
/// call drops any previous watcher handle (stopping its OS-level watch) and
/// spawns a fresh notify-rs watcher plus a bridge thread that forwards
/// `TreeChange` events to the webview as `vlerv://tree-changed`.
///
/// KNOWN LEAK: the previous run's flush thread (inside `watcher.rs`) and its
/// matching bridge thread here block forever — `start_watching` has no
/// shutdown signal for those. In practice this means a few stranded threads
/// per workspace switch, which is fine for a single-user dev tool but
/// warrants a proper shutdown channel before that pattern multiplies.
#[tauri::command]
fn set_workspace_root(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    path: String,
) -> Result<(), String> {
    let root = std::path::PathBuf::from(&path);
    if !root.exists() {
        return Err(format!("path does not exist: {path}"));
    }

    let (tx, rx) = std::sync::mpsc::channel();
    let handle = src_tauri::watcher::start_watching(vec![root], Vec::new(), tx)
        .map_err(|e| format!("{e:?}"))?;

    {
        let mut guard = state.watcher.lock().map_err(|e| e.to_string())?;
        *guard = Some(handle);
    }

    std::thread::spawn(move || {
        for change in rx {
            let _ = app.emit("vlerv://tree-changed", change);
        }
    });

    Ok(())
}

fn main() {
    let home = std::env::var("HOME").unwrap_or_else(|_| String::from("/"));
    let default_root = std::path::PathBuf::from(format!("{home}/workspace"));
    let roots = src_tauri::security::RootSet::new(vec![default_root]);
    let roots_for_setup = roots.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_drag::init())
        .manage(AppState {
            scanner: Mutex::new(src_tauri::workspace::Scanner::new()),
            roots: roots.clone(),
            watcher: Mutex::new(None),
        })
        .manage(roots)
        .invoke_handler(tauri::generate_handler![
            list_dir,
            list_workspace_roots,
            read_file,
            set_workspace_root,
        ])
        .setup(move |app| {
            let app_handle = app.handle().clone();
            let roots = roots_for_setup;
            app.deep_link().on_open_url(move |event| {
                for url in event.urls() {
                    let url_str = url.to_string();
                    src_tauri::handle_deep_link(&url_str);
                    match src_tauri::dispatch_deep_link(&url_str, &roots) {
                        Ok(open_event) => {
                            let _ = app_handle.emit("vlerv://open-file", open_event);
                        }
                        Err(err_event) => {
                            eprintln!(
                                "vlerv: deep-link rejected: {} ({})",
                                err_event.reason, err_event.url
                            );
                            let _ = app_handle.emit("vlerv://deep-link-error", err_event);
                        }
                    }
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Tauri application");
}
