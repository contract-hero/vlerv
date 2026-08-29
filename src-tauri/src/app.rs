// Vlerv Tauri app entry point. Boots tauri::Builder with the deep-link plugin
// and the workspace/reader/security state.
//
// This lives in the library, not in `main.rs`, because iOS has no `main`:
// the generated Xcode project links the crate as a static library and calls
// the `start_app` symbol that `#[tauri::mobile_entry_point]` emits from
// `run()`. `main.rs` is now a desktop-only shim that calls the same `run()`.

use std::sync::Mutex;
use tauri::Emitter;
// `Manager::get_webview_window` is used only by the desktop foreground hop in
// `setup` — there is no window to raise on iOS.
#[cfg(desktop)]
use tauri::Manager;
use tauri_plugin_deep_link::DeepLinkExt;

pub struct AppState {
    pub scanner: Mutex<crate::workspace::Scanner>,
    pub roots: crate::security::RootSet,
    pub watcher: Mutex<Option<crate::watcher::WatcherHandle>>,
    /// Watcher covering individual out-of-root files open in tabs.
    pub external_watcher: Mutex<Option<crate::watcher::WatcherHandle>>,
}

#[tauri::command]
fn list_dir(
    state: tauri::State<AppState>,
    path: String,
) -> Result<Vec<crate::workspace::Entry>, String> {
    let scanner = state.scanner.lock().map_err(|e| e.to_string())?;
    scanner
        .list_dir(std::path::Path::new(&path))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn list_workspace_roots(
    state: tauri::State<AppState>,
    path: String,
) -> Result<Vec<crate::workspace::Entry>, String> {
    let scanner = state.scanner.lock().map_err(|e| e.to_string())?;
    scanner
        .list_workspace_roots(std::path::Path::new(&path))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn read_file(
    path: String,
) -> Result<crate::reader::FilePayload, String> {
    crate::reader::read_file(std::path::Path::new(&path))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn list_files_recursive(
    path: String,
) -> Result<crate::workspace::FileIndex, String> {
    crate::workspace::list_files_recursive(std::path::Path::new(&path))
        .map_err(|e| e.to_string())
}

// ─── state_store + recents + bookmarks IPC ───────────────────────────────────
// These wrap the existing module functions as Tauri commands. The frontend
// (src/ipc.ts, src/state/recents-context.tsx, src/state/bookmarks-context.tsx,
// src/hooks/useBookmarks.ts) consumes them through invoke().

#[tauri::command]
fn get_state() -> serde_json::Value {
    crate::state_store::current_state_value()
}

#[tauri::command]
fn set_state_field(app: tauri::AppHandle, key: String, value: serde_json::Value) -> Result<(), String> {
    crate::state_store::set_state_field(&key, value)?;
    // Broadcast the updated document so useSettings subscribers (e.g. the
    // Preview header's Slack button) pick up preference changes without an
    // app restart — the listener predates this emitter and was dead code.
    let _ = app.emit("vlerv://state-updated", crate::state_store::current_state_value());
    Ok(())
}

#[tauri::command]
fn list_recents() -> Vec<crate::state_store::RecentEntry> {
    crate::recents::list()
}

#[tauri::command]
fn push_recent(path: String) -> Result<(), String> {
    crate::recents::push(std::path::Path::new(&path))
}

#[tauri::command]
fn list_bookmarks() -> Vec<crate::state_store::BookmarkEntry> {
    crate::bookmarks::list()
}

#[tauri::command]
fn add_bookmark(app: tauri::AppHandle, path: String) -> Result<(), String> {
    crate::bookmarks::add(std::path::Path::new(&path))?;
    // Broadcast the updated list so every useBookmarks subscriber (Explorer
    // star state, Preview-header star, Sidebar Bookmarks section) stays in
    // sync without each instance maintaining its own optimistic state.
    let _ = app.emit("vlerv://bookmarks-updated", crate::bookmarks::list());
    Ok(())
}

#[tauri::command]
fn remove_bookmark(app: tauri::AppHandle, path: String) -> Result<(), String> {
    crate::bookmarks::remove(std::path::Path::new(&path))?;
    let _ = app.emit("vlerv://bookmarks-updated", crate::bookmarks::list());
    Ok(())
}

#[tauri::command]
fn reorder_bookmarks(app: tauri::AppHandle, paths: Vec<String>) -> Result<(), String> {
    crate::bookmarks::reorder(&paths)?;
    let _ = app.emit("vlerv://bookmarks-updated", crate::bookmarks::list());
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
    roots: tauri::State<crate::security::RootSet>,
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

    let ignore_globs: Vec<String> = crate::workspace::DEFAULT_IGNORED
        .iter()
        .map(|s| (*s).to_string())
        .collect();

    let (tx, rx) = std::sync::mpsc::channel();
    let handle = crate::watcher::start_watching(vec![canonical], ignore_globs, tx)
        .map_err(|e| format!("{e:?}"))?;

    {
        let mut guard = state.watcher.lock().map_err(|e| e.to_string())?;
        *guard = Some(handle);
    }

    std::thread::spawn(move || {
        for change in rx {
            // Cross-machine live reload (design §6): the same watcher event
            // that refreshes the local tree also fans out to subscribed
            // peers, re-hashed so it carries the new content address.
            crate::remote::note_file_change(
                &app,
                crate::watcher::FileChange {
                    kind: change.kind.clone(),
                    path: change.path.clone(),
                },
            );
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

        let handle = crate::watcher::watch_files(path_bufs, tx)
            .map_err(|e| format!("{e:?}"))?;
        *guard = Some(handle);

        std::thread::spawn(move || {
            for change in rx {
                let path = originals
                    .get(&change.path)
                    .cloned()
                    .unwrap_or(change.path);
                let event = crate::watcher::FileChange { kind: change.kind, path };
                crate::remote::note_file_change(&app, event.clone());
                let _ = app.emit("vlerv://file-changed", event);
            }
        });
    }

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let home = std::env::var("HOME").unwrap_or_else(|_| String::from("/"));
    let default_root = std::path::PathBuf::from(format!("{home}/workspace"));
    let roots = crate::security::RootSet::new(vec![default_root]);
    let roots_for_setup = roots.clone();
    // The scope server gates every remote read against the SAME shared root
    // set the local IPC layer uses, so a workspace picked later is visible to
    // both (RootSet is Arc-shared).
    let roots_for_remote = roots.clone();

    // Eager-load the on-disk state.json into the in-memory global state so the
    // very first `get_state` / `list_recents` / `list_bookmarks` call after
    // launch returns the persisted values, not Default. Without this the
    // frontend silently rehydrates to empty on every cold start.
    crate::state_store::load();

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init());

    // Desktop-only plugins. `window-state` has no window geometry to persist
    // on iOS and `drag` (drag-out to Finder) has no iOS platform impl at all,
    // so both are macOS-target dependencies (see Cargo.toml) and both are
    // registered only here.
    #[cfg(target_os = "macos")]
    let builder = builder
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_drag::init());

    builder
        .manage(AppState {
            scanner: Mutex::new(crate::workspace::Scanner::new()),
            roots: roots.clone(),
            watcher: Mutex::new(None),
            external_watcher: Mutex::new(None),
        })
        .manage(roots)
        .manage(crate::remote::RemoteState::new(roots_for_remote))
        .invoke_handler(tauri::generate_handler![
            list_dir,
            list_workspace_roots,
            read_file,
            list_files_recursive,
            crate::platform::platform_info,
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
            crate::share::share_file,
            crate::remote::beam_offer,
            crate::remote::beam_stop,
            crate::remote::beam_list_offers,
            crate::remote::beam_receive,
            crate::remote::beam_received_dir,
            crate::remote::beam_list_received,
            crate::remote::remote_list_peers,
            crate::remote::remote_pair_begin,
            crate::remote::remote_pair_complete,
            crate::remote::remote_pair_confirm,
            crate::remote::remote_unpair,
            crate::remote::remote_set_scope,
            crate::remote::remote_publish_tabs,
            crate::remote::remote_list_tabs,
            crate::remote::remote_list_bookmarks,
            crate::remote::remote_list_recents,
            crate::remote::remote_list_tree,
            crate::remote::remote_get,
            crate::remote::remote_open_on_host,
            crate::remote::remote_push_artifact,
            crate::remote::remote_subscribe,
            crate::remote::remote_unsubscribe,
        ])
        .setup(move |app| {
            let app_handle = app.handle().clone();
            // Lazy boot, launch half (design §4): sockets at startup only
            // when this install has peers AND `preferences.remote_listen`
            // is on. Otherwise the app still dials nothing until an action.
            crate::remote::listen_at_launch(&app_handle);
            let roots = roots_for_setup;
            app.deep_link().on_open_url(move |event| {
                // Bring the window to the foreground before dispatching, so a
                // deep-link click from another app raises Vlervtifacts instead of
                // silently delivering the file to a backgrounded / minimized /
                // hidden window. On macOS, `set_focus` activates the app via
                // NSApp.activate(ignoringOtherApps:) — `macosPrivateApi` is
                // already enabled in tauri.conf.json. Calls are idempotent, so
                // an already-focused window sees no flicker.
                //
                // Desktop only: iOS has no window to unminimize, and the
                // system already foregrounds the app when it hands over a
                // URL, so `unminimize`/`show`/`set_focus` do not exist there.
                #[cfg(desktop)]
                if let Some(window) = app_handle.get_webview_window("main") {
                    let _ = window.unminimize();
                    let _ = window.show();
                    let _ = window.set_focus();
                }
                for url in event.urls() {
                    let url_str = url.to_string();
                    crate::handle_deep_link(&url_str);
                    match crate::dispatch_deep_link(&url_str, &roots) {
                        Ok(crate::DeepLinkAction::OpenFile(open_event)) => {
                            let _ = app_handle.emit("vlerv://open-file", open_event);
                        }
                        Ok(crate::DeepLinkAction::BeamReceive(req)) => {
                            let _ = app_handle.emit("vlerv://beam-receive-request", req);
                        }
                        Ok(crate::DeepLinkAction::BeamSend(req)) => {
                            let _ = app_handle.emit("vlerv://beam-send-request", req);
                        }
                        Ok(crate::DeepLinkAction::Pair(req)) => {
                            // Debug-only E2E hook (third arm, same env contract
                            // as `remote::test_autopair`): a simulator cannot
                            // be tapped, so the arriving link dials without the
                            // confirm UI. Absent from release builds.
                            #[cfg(debug_assertions)]
                            if std::env::var("VLERV_TEST_AUTOPAIR").is_ok() {
                                crate::remote::test_autopair_dial(
                                    app_handle.clone(),
                                    req.ticket.clone(),
                                );
                            }
                            // Joins the frozen `vlerv://*` namespace through
                            // the one remote event name, discriminated by
                            // `kind` — no new event for one more verb.
                            let _ = app_handle.emit(
                                "vlerv://remote-event",
                                crate::remote::RemoteEvent::PairLink {
                                    peer: req.host_id,
                                    peer_short: req.host_id_short,
                                    device: req.device,
                                    ticket: req.ticket,
                                },
                            );
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
                crate::state_store::flush();
            }
        });
}
