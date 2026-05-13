// File-system watcher — `notify-rs` wrapper with ignore-set filtering and
// 250 ms debounce.

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TreeChangeKind {
    Add,
    Modify,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeChange {
    pub project_root: PathBuf,
    pub kind: TreeChangeKind,
    pub path: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum WatcherError {
    #[error("notify error: {0}")]
    Notify(String),
    #[error("invalid root: {0:?}")]
    InvalidRoot(PathBuf),
}

/// Handle returned by `start_watching`. Dropping it stops the watcher.
pub struct WatcherHandle {
    _watcher: RecommendedWatcher,
}

impl std::fmt::Debug for WatcherHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WatcherHandle").finish()
    }
}

impl WatcherHandle {
    pub fn stop(self) {
        // Dropping the watcher stops it. `self` is moved here and dropped.
    }
}

const DEBOUNCE_MS: u64 = 250;

/// Start watching the given roots.
/// `ignore_globs` filters emitted events (glob patterns like "*.log").
/// `tx` receives `TreeChange` payloads after the debounce window.
pub fn start_watching(
    roots: Vec<PathBuf>,
    ignore_globs: Vec<String>,
    tx: Sender<TreeChange>,
) -> Result<WatcherHandle, WatcherError> {
    if roots.is_empty() {
        return Err(WatcherError::Notify("no roots provided".to_string()));
    }

    // Validate roots exist before setting up the watcher.
    for root in &roots {
        if !root.exists() {
            return Err(WatcherError::InvalidRoot(root.clone()));
        }
    }

    // Debounce: track last-seen event per (path, kind) and a deadline.
    // We store: path -> (kind, Instant of last event).
    let pending: Arc<Mutex<HashMap<PathBuf, (TreeChangeKind, Instant)>>> =
        Arc::new(Mutex::new(HashMap::new()));

    let roots_for_handler = roots.clone();
    let pending_for_handler = Arc::clone(&pending);
    let ignore_globs_clone = ignore_globs.clone();
    let tx_clone = tx.clone();

    // Debounce flush thread: polls the pending map and emits when quiet.
    let pending_for_flush = Arc::clone(&pending);
    let roots_for_flush = roots.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(50));
            let now = Instant::now();
            let mut map = pending_for_flush.lock().unwrap_or_else(|p| p.into_inner());
            let ready: Vec<(PathBuf, TreeChangeKind)> = map
                .iter()
                .filter(|(_, (_, t))| now.duration_since(*t) >= Duration::from_millis(DEBOUNCE_MS))
                .map(|(p, (k, _))| (p.clone(), k.clone()))
                .collect();
            for (path, kind) in ready {
                map.remove(&path);
                let project_root = find_root(&path, &roots_for_flush);
                let _ = tx_clone.send(TreeChange { project_root, kind, path });
            }
        }
    });

    let (raw_tx, raw_rx) = channel();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res {
            let _ = raw_tx.send(event);
        }
    })
    .map_err(|e| WatcherError::Notify(e.to_string()))?;

    for root in &roots {
        watcher
            .watch(root.as_path(), RecursiveMode::Recursive)
            .map_err(|e| WatcherError::Notify(e.to_string()))?;
    }

    // Event processing thread: reads from raw_rx, applies ignore filter,
    // and updates the debounce map.
    std::thread::spawn(move || {
        for event in raw_rx {
            let kind = match event.kind {
                EventKind::Create(_) => TreeChangeKind::Add,
                EventKind::Modify(_) => TreeChangeKind::Modify,
                EventKind::Remove(_) => TreeChangeKind::Remove,
                _ => continue,
            };

            for path in &event.paths {
                // Apply ignore_globs filter.
                if is_ignored(path, &ignore_globs_clone) {
                    continue;
                }
                // Update debounce map: last event wins.
                let mut map = pending_for_handler.lock().unwrap_or_else(|p| p.into_inner());
                map.insert(path.clone(), (kind.clone(), Instant::now()));
            }
        }
    });

    Ok(WatcherHandle { _watcher: watcher })
}

fn is_ignored(path: &PathBuf, ignore_globs: &[String]) -> bool {
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    for glob in ignore_globs {
        if matches_glob(file_name, glob) {
            return true;
        }
    }
    false
}

/// Simple glob matcher supporting `*` as wildcard for file name matching.
fn matches_glob(name: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(suffix) = pattern.strip_prefix('*') {
        return name.ends_with(suffix);
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return name.starts_with(prefix);
    }
    name == pattern
}

fn find_root(path: &PathBuf, roots: &[PathBuf]) -> PathBuf {
    for root in roots {
        if path.starts_with(root) {
            return root.clone();
        }
    }
    roots.first().cloned().unwrap_or_else(|| path.clone())
}
