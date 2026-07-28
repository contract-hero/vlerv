// File-system watcher — `notify-rs` wrapper with ignore-set filtering and
// 250 ms debounce.

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
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

/// Handle returned by `start_watching`. Dropping it stops the watcher AND
/// terminates the whole pipeline: the flush thread exits via the shutdown
/// flag, the raw-event thread exits when the dropped watcher releases its
/// handler closure (disconnecting `raw_rx`), and any downstream bridge
/// thread exits when the flush thread drops the last `Sender<TreeChange>`.
pub struct WatcherHandle {
    _watcher: RecommendedWatcher,
    shutdown: Arc<AtomicBool>,
}

impl Drop for WatcherHandle {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }
}

impl std::fmt::Debug for WatcherHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WatcherHandle").finish()
    }
}

impl WatcherHandle {
    pub fn stop(self) {
        // Moving `self` here drops it, which triggers the shutdown cascade.
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

    let shutdown = Arc::new(AtomicBool::new(false));

    let pending_for_handler = Arc::clone(&pending);
    let ignore_globs_clone = ignore_globs.clone();
    let tx_clone = tx.clone();

    // Debounce flush thread: polls the pending map and emits when quiet.
    // Exits when the WatcherHandle is dropped; the original `tx` drops at the
    // end of this function, so the flush thread's exit releases the last
    // Sender and disconnects the caller's receiver.
    let pending_for_flush = Arc::clone(&pending);
    let roots_for_flush = roots.clone();
    let shutdown_for_flush = Arc::clone(&shutdown);
    std::thread::spawn(move || {
        loop {
            if shutdown_for_flush.load(Ordering::SeqCst) {
                return;
            }
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
    // and updates the debounce map. Exits when the watcher (and with it the
    // handler closure owning `raw_tx`) is dropped.
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

    Ok(WatcherHandle {
        _watcher: watcher,
        shutdown,
    })
}

fn is_ignored(path: &PathBuf, ignore_globs: &[String]) -> bool {
    for component in path.components() {
        if let Some(name) = component.as_os_str().to_str() {
            for glob in ignore_globs {
                if matches_glob(name, glob) {
                    return true;
                }
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::RecvTimeoutError;
    use tempfile::TempDir;

    #[test]
    fn dropping_handle_disconnects_channel() {
        let dir = TempDir::new().expect("tempdir");
        let (tx, rx) = channel();
        let handle =
            start_watching(vec![dir.path().to_path_buf()], vec![], tx).expect("start");
        drop(handle);

        // The flush thread notices the shutdown flag within one 50 ms tick and
        // exits, dropping the last Sender. Drain any stray events first.
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Err(RecvTimeoutError::Disconnected) => return, // pass
                Ok(_) => continue,                             // stray event, keep draining
                Err(RecvTimeoutError::Timeout) => {
                    if Instant::now() > deadline {
                        panic!("channel never disconnected after handle drop");
                    }
                }
            }
        }
    }

    #[test]
    fn event_delivery_still_works() {
        let dir = TempDir::new().expect("tempdir");
        let (tx, rx) = channel();
        let _handle =
            start_watching(vec![dir.path().to_path_buf()], vec![], tx).expect("start");

        // FSEvents needs a beat to arm before it reports changes.
        std::thread::sleep(Duration::from_millis(300));
        std::fs::write(dir.path().join("artifact.html"), "<html></html>").expect("write");

        // Generous timeout: FSEvents latency + 250 ms debounce + flush tick.
        let change = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("expected a TreeChange event");
        assert!(change.path.ends_with("artifact.html"));
    }
}
