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

/// Payload for the `vlerv://file-changed` event covering individually
/// watched out-of-root files, where a project root is meaningless.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    pub kind: TreeChangeKind,
    pub path: PathBuf,
}

/// How the raw-event thread decides which paths produce TreeChange emissions.
enum EventFilter {
    /// Drop paths with any component matching one of these globs.
    IgnoreGlobs(Vec<String>),
    /// Keep only paths in this exact set (individually watched files).
    ExactPaths(std::collections::HashSet<PathBuf>),
}

impl EventFilter {
    fn keeps(&self, path: &PathBuf) -> bool {
        match self {
            EventFilter::IgnoreGlobs(globs) => !is_ignored(path, globs),
            EventFilter::ExactPaths(set) => set.contains(path),
        }
    }
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

/// Start watching the given roots recursively.
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

    let targets: Vec<(PathBuf, RecursiveMode)> = roots
        .iter()
        .map(|r| (r.clone(), RecursiveMode::Recursive))
        .collect();

    spawn_pipeline(targets, EventFilter::IgnoreGlobs(ignore_globs), roots, tx)
}

/// Watch a set of individual files (typically out-of-root files open in
/// tabs). Watches each file's PARENT directory non-recursively and filters
/// events to the exact registered set — watching parents, not files, is
/// load-bearing: atomic saves (write-temp-then-rename, what editors and
/// Claude's Write tool do) detach per-file watches, while parent-dir watches
/// survive them.
///
/// Inputs are canonicalized and deduped; nonexistent paths are silently
/// skipped (a tab may reference a since-deleted file). An empty effective
/// set yields a valid no-op handle.
pub fn watch_files(
    paths: Vec<PathBuf>,
    tx: Sender<TreeChange>,
) -> Result<WatcherHandle, WatcherError> {
    let mut file_set = std::collections::HashSet::new();
    let mut parent_dirs = Vec::new();

    for path in paths {
        let Ok(canonical) = path.canonicalize() else {
            continue;
        };
        let Some(parent) = canonical.parent().map(|p| p.to_path_buf()) else {
            continue;
        };
        if file_set.insert(canonical) && !parent_dirs.contains(&parent) {
            parent_dirs.push(parent);
        }
    }

    let targets: Vec<(PathBuf, RecursiveMode)> = parent_dirs
        .iter()
        .map(|d| (d.clone(), RecursiveMode::NonRecursive))
        .collect();

    spawn_pipeline(targets, EventFilter::ExactPaths(file_set), parent_dirs, tx)
}

/// Shared watcher pipeline: notify watcher → raw-event thread (filter +
/// debounce map) → flush thread (250 ms quiet window → `tx`).
fn spawn_pipeline(
    targets: Vec<(PathBuf, RecursiveMode)>,
    filter: EventFilter,
    payload_roots: Vec<PathBuf>,
    tx: Sender<TreeChange>,
) -> Result<WatcherHandle, WatcherError> {
    // Debounce: track last-seen event per path. path -> (kind, last Instant).
    let pending: Arc<Mutex<HashMap<PathBuf, (TreeChangeKind, Instant)>>> =
        Arc::new(Mutex::new(HashMap::new()));

    let shutdown = Arc::new(AtomicBool::new(false));

    let pending_for_handler = Arc::clone(&pending);
    let tx_clone = tx.clone();

    // Debounce flush thread: polls the pending map and emits when quiet.
    // Exits when the WatcherHandle is dropped; the original `tx` drops at the
    // end of this function, so the flush thread's exit releases the last
    // Sender and disconnects the caller's receiver.
    let pending_for_flush = Arc::clone(&pending);
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
                let project_root = find_root(&path, &payload_roots);
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

    for (target, mode) in &targets {
        watcher
            .watch(target.as_path(), *mode)
            .map_err(|e| WatcherError::Notify(e.to_string()))?;
    }

    // Event processing thread: reads from raw_rx, applies the filter, and
    // updates the debounce map. Exits when the watcher (and with it the
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
                if !filter.keeps(path) {
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
    fn watch_files_emits_for_registered_file_only() {
        let dir = TempDir::new().expect("tempdir");
        let target = dir.path().join("watched.html");
        let sibling = dir.path().join("sibling.html");
        std::fs::write(&target, "a").unwrap();
        std::fs::write(&sibling, "a").unwrap();

        let (tx, rx) = channel();
        let _handle = watch_files(vec![target.clone()], tx).expect("watch");
        std::thread::sleep(Duration::from_millis(300));

        // Sibling change in the same (watched) parent dir must be filtered out.
        std::fs::write(&sibling, "changed").unwrap();
        std::fs::write(&target, "changed").unwrap();

        let canonical_target = target.canonicalize().unwrap();
        let change = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("expected a change for the watched file");
        assert_eq!(change.path, canonical_target);
        // And nothing further (the sibling event was dropped).
        match rx.recv_timeout(Duration::from_millis(600)) {
            Err(_) => {}
            Ok(extra) => assert_eq!(extra.path, canonical_target, "unexpected sibling event"),
        }
    }

    #[test]
    fn watch_files_survives_atomic_replace() {
        let dir = TempDir::new().expect("tempdir");
        let target = dir.path().join("artifact.html");
        std::fs::write(&target, "v1").unwrap();

        let (tx, rx) = channel();
        let _handle = watch_files(vec![target.clone()], tx).expect("watch");
        std::thread::sleep(Duration::from_millis(300));

        // Atomic save: write temp file, rename over the target — the pattern
        // used by editors and by Claude's Write tool.
        let tmp = dir.path().join("artifact.html.tmp");
        std::fs::write(&tmp, "v2").unwrap();
        std::fs::rename(&tmp, &target).unwrap();

        let canonical_target = target.canonicalize().unwrap();
        let change = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("expected a change after atomic replace");
        assert_eq!(change.path, canonical_target);
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
