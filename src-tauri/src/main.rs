// Vlerv Tauri app entry point. Boots tauri::Builder with the deep-link plugin
// and the workspace/reader/security state.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;
use tauri::{Emitter, Manager};
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

// ─── state_store + recents + bookmarks IPC ───────────────────────────────────
// These wrap the existing module functions as Tauri commands. The frontend
// (src/ipc.ts, src/hooks/useSettings.ts, src/hooks/useRecents.ts,
// src/hooks/useBookmarks.ts) consumes them through invoke().

#[tauri::command]
fn get_state() -> serde_json::Value {
    src_tauri::state_store::current_state_value()
}

#[tauri::command]
fn set_state_field(key: String, value: serde_json::Value) -> Result<(), String> {
    src_tauri::state_store::set_state_field(&key, value)
}

#[tauri::command]
fn list_recents() -> Vec<src_tauri::state_store::RecentEntry> {
    src_tauri::recents::list()
}

#[tauri::command]
fn push_recent(path: String) -> Result<(), String> {
    src_tauri::recents::push(std::path::Path::new(&path))
}

#[tauri::command]
fn list_bookmarks() -> Vec<src_tauri::state_store::BookmarkEntry> {
    src_tauri::bookmarks::list()
}

#[tauri::command]
fn add_bookmark(app: tauri::AppHandle, path: String) -> Result<(), String> {
    src_tauri::bookmarks::add(std::path::Path::new(&path))?;
    // Broadcast the updated list so every useBookmarks subscriber (Explorer
    // star state, Preview-header star, Sidebar Bookmarks section) stays in
    // sync without each instance maintaining its own optimistic state.
    let _ = app.emit("vlerv://bookmarks-updated", src_tauri::bookmarks::list());
    Ok(())
}

#[tauri::command]
fn remove_bookmark(app: tauri::AppHandle, path: String) -> Result<(), String> {
    src_tauri::bookmarks::remove(std::path::Path::new(&path))?;
    let _ = app.emit("vlerv://bookmarks-updated", src_tauri::bookmarks::list());
    Ok(())
}

#[tauri::command]
fn reorder_bookmarks(app: tauri::AppHandle, paths: Vec<String>) -> Result<(), String> {
    src_tauri::bookmarks::reorder(&paths)?;
    let _ = app.emit("vlerv://bookmarks-updated", src_tauri::bookmarks::list());
    Ok(())
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
    roots: tauri::State<src_tauri::security::RootSet>,
    path: String,
) -> Result<(), String> {
    let root = std::path::PathBuf::from(&path);
    let canonical = src_tauri::security::canonicalize_and_check_root(&root, &roots)
        .map_err(|e| e.to_string())?;

    let ignore_globs: Vec<String> = src_tauri::workspace::DEFAULT_IGNORED
        .iter()
        .map(|s| (*s).to_string())
        .collect();

    let (tx, rx) = std::sync::mpsc::channel();
    let handle = src_tauri::watcher::start_watching(vec![canonical], ignore_globs, tx)
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

    // Eager-load the on-disk state.json into the in-memory global state so the
    // very first `get_state` / `list_recents` / `list_bookmarks` call after
    // launch returns the persisted values, not Default. Without this the
    // frontend silently rehydrates to empty on every cold start.
    src_tauri::state_store::load();

    tauri::Builder::default()
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_drag::init())
        .plugin(tauri_plugin_opener::init())
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
            get_state,
            set_state_field,
            list_recents,
            push_recent,
            list_bookmarks,
            add_bookmark,
            remove_bookmark,
            reorder_bookmarks,
            src_tauri::share::share_file,
        ])
        .setup(move |app| {
            let app_handle = app.handle().clone();
            let roots = roots_for_setup;
            app.deep_link().on_open_url(move |event| {
                // Bring the window to the foreground before dispatching, so a
                // deep-link click from another app raises Vlervcode instead of
                // silently delivering the file to a backgrounded / minimized /
                // hidden window. On macOS, `set_focus` activates the app via
                // NSApp.activate(ignoringOtherApps:) — `macosPrivateApi` is
                // already enabled in tauri.conf.json. Calls are idempotent, so
                // an already-focused window sees no flicker.
                if let Some(window) = app_handle.get_webview_window("main") {
                    let _ = window.unminimize();
                    let _ = window.show();
                    let _ = window.set_focus();
                }
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
