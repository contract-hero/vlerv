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
    /// Watcher covering individual out-of-root files open in tabs.
    pub external_watcher: Mutex<Option<src_tauri::watcher::WatcherHandle>>,
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

#[tauri::command]
fn list_files_recursive(
    path: String,
) -> Result<src_tauri::workspace::FileIndex, String> {
    src_tauri::workspace::list_files_recursive(std::path::Path::new(&path))
        .map_err(|e| e.to_string())
}

// ─── state_store + recents + bookmarks IPC ───────────────────────────────────
// These wrap the existing module functions as Tauri commands. The frontend
// (src/ipc.ts, src/state/recents-context.tsx, src/state/bookmarks-context.tsx,
// src/hooks/useBookmarks.ts) consumes them through invoke().

#[tauri::command]
fn get_state() -> serde_json::Value {
    src_tauri::state_store::current_state_value()
}

#[tauri::command]
fn set_state_field(app: tauri::AppHandle, key: String, value: serde_json::Value) -> Result<(), String> {
    src_tauri::state_store::set_state_field(&key, value)?;
    // Broadcast the updated document so useSettings subscribers (e.g. the
    // Preview header's Slack button) pick up preference changes without an
    // app restart — the listener predates this emitter and was dead code.
    let _ = app.emit("vlerv://state-updated", src_tauri::state_store::current_state_value());
    Ok(())
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
/// call drops any previous watcher handle, which shuts down its entire
/// pipeline (watcher, flush thread, raw-event thread, and the bridge thread
/// below via channel disconnect), then spawns a fresh notify-rs watcher plus
/// a bridge thread that forwards `TreeChange` events to the webview as
/// `vlerv://tree-changed`.
#[tauri::command]
fn set_workspace_root(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    roots: tauri::State<src_tauri::security::RootSet>,
    path: String,
) -> Result<(), String> {
    // The picked workspace must be *addable* to the root set, not gated by
    // the boot-time set — gating here would reject any workspace outside
    // ~/workspace and defeat the folder picker. RootSet is Arc-shared, so
    // the deep-link callback's clone sees the addition immediately and deep
    // links into the picked workspace classify as in-root.
    let root = std::path::PathBuf::from(&path);
    let canonical = root.canonicalize().map_err(|e| e.to_string())?;
    if !canonical.is_dir() {
        return Err(format!("not a directory: {canonical:?}"));
    }
    roots.add_root(&canonical);

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

/// Replace the set of individually watched out-of-root files. The frontend
/// calls this with the full set of open external-tab files whenever a tab
/// opens or closes; an empty set clears the watcher. Changes are emitted as
/// `vlerv://file-changed` with a `{ kind, path }` payload — a dedicated
/// event so tab auto-reload stays decoupled from tree refresh.
#[tauri::command]
fn watch_external_paths(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    paths: Vec<String>,
) -> Result<(), String> {
    // Drop the previous watcher first — its pipeline shuts down via the
    // handle's Drop cascade.
    {
        let mut guard = state.external_watcher.lock().map_err(|e| e.to_string())?;
        *guard = None;

        if paths.is_empty() {
            return Ok(());
        }

        let (tx, rx) = std::sync::mpsc::channel();
        let path_bufs: Vec<std::path::PathBuf> =
            paths.iter().map(std::path::PathBuf::from).collect();

        // The watcher canonicalizes inputs and emits CANONICAL paths, but the
        // frontend keys reloads by the exact string stored in its tab state
        // (e.g. "/tmp/x.html", which canonicalizes to "/private/tmp/x.html"
        // on macOS). Map canonical → caller-supplied so emitted events match
        // what the frontend is listening for.
        let originals: std::collections::HashMap<std::path::PathBuf, std::path::PathBuf> =
            path_bufs
                .iter()
                .filter_map(|p| p.canonicalize().ok().map(|c| (c, p.clone())))
                .collect();

        let handle = src_tauri::watcher::watch_files(path_bufs, tx)
            .map_err(|e| format!("{e:?}"))?;
        *guard = Some(handle);

        std::thread::spawn(move || {
            for change in rx {
                let path = originals
                    .get(&change.path)
                    .cloned()
                    .unwrap_or(change.path);
                let _ = app.emit(
                    "vlerv://file-changed",
                    src_tauri::watcher::FileChange {
                        kind: change.kind,
                        path,
                    },
                );
            }
        });
    }

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
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_drag::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState {
            scanner: Mutex::new(src_tauri::workspace::Scanner::new()),
            roots: roots.clone(),
            watcher: Mutex::new(None),
            external_watcher: Mutex::new(None),
        })
        .manage(roots)
        .manage(src_tauri::remote::RemoteState::default())
        .invoke_handler(tauri::generate_handler![
            list_dir,
            list_workspace_roots,
            read_file,
            list_files_recursive,
            set_workspace_root,
            watch_external_paths,
            get_state,
            set_state_field,
            list_recents,
            push_recent,
            list_bookmarks,
            add_bookmark,
            remove_bookmark,
            reorder_bookmarks,
            src_tauri::share::share_file,
            src_tauri::remote::beam_offer,
            src_tauri::remote::beam_stop,
            src_tauri::remote::beam_list_offers,
            src_tauri::remote::beam_receive,
            src_tauri::remote::beam_received_dir,
            src_tauri::remote::beam_list_received,
        ])
        .setup(move |app| {
            let app_handle = app.handle().clone();
            let roots = roots_for_setup;
            app.deep_link().on_open_url(move |event| {
                // Bring the window to the foreground before dispatching, so a
                // deep-link click from another app raises Vlervtifacts instead of
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
                        Ok(src_tauri::DeepLinkAction::OpenFile(open_event)) => {
                            let _ = app_handle.emit("vlerv://open-file", open_event);
                        }
                        Ok(src_tauri::DeepLinkAction::BeamReceive(req)) => {
                            let _ = app_handle.emit("vlerv://beam-receive-request", req);
                        }
                        Ok(src_tauri::DeepLinkAction::BeamSend(req)) => {
                            let _ = app_handle.emit("vlerv://beam-send-request", req);
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
        .build(tauri::generate_context!())
        .expect("error while building Tauri application")
        .run(|_app, event| {
            if let tauri::RunEvent::Exit = event {
                // Flush any debounced state_store write so a quit within the
                // 250 ms window doesn't lose bookmarks/recents/pane sizes.
                src_tauri::state_store::flush();
            }
        });
}
