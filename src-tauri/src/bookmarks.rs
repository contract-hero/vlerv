// Bookmarks store — explicit user-starred file paths. Mirrors recents.rs but
// (1) idempotent add/remove (frontend decides toggle semantics), (2) no cap,
// (3) ordered by bookmarked_at desc on read.

use std::path::Path;

pub use crate::state_store::BookmarkEntry;

/// Add `path` to bookmarks at the head (idempotent — re-adding a bookmarked
/// path is a no-op except for refreshing `bookmarked_at`).
pub fn add(path: &Path) -> Result<(), String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let new_entry = BookmarkEntry {
        path: path.to_path_buf(),
        bookmarked_at: now,
    };

    let mut updated: Vec<BookmarkEntry> = list_from_global()
        .into_iter()
        .filter(|e| e.path != path)
        .collect();
    updated.insert(0, new_entry);

    let val = serde_json::to_value(&updated).map_err(|e| e.to_string())?;
    crate::state_store::set_state_field("bookmarks", val)?;
    Ok(())
}

/// Remove `path` from bookmarks (idempotent — removing a non-bookmarked path
/// is a no-op).
pub fn remove(path: &Path) -> Result<(), String> {
    let updated: Vec<BookmarkEntry> = list_from_global()
        .into_iter()
        .filter(|e| e.path != path)
        .collect();

    let val = serde_json::to_value(&updated).map_err(|e| e.to_string())?;
    crate::state_store::set_state_field("bookmarks", val)?;
    Ok(())
}

/// List bookmarks in stored order. New bookmarks are inserted at the head by
/// `add`, so the default order is most-recent-first; an explicit `reorder`
/// (drag-and-drop in the Sidebar) overrides that and is preserved verbatim.
pub fn list() -> Vec<BookmarkEntry> {
    list_from_global()
}

/// Rewrite the bookmark list in the order given by `ordered_paths`. Paths are
/// matched against existing entries (preserving each entry's `bookmarked_at`);
/// unknown paths are ignored and any existing entry not named in
/// `ordered_paths` is appended at the end, keeping its relative order. This is
/// idempotent and never invents or drops bookmarks.
pub fn reorder(ordered_paths: &[String]) -> Result<(), String> {
    let mut remaining = list_from_global();
    let mut updated: Vec<BookmarkEntry> = Vec::with_capacity(remaining.len());

    for raw in ordered_paths {
        let wanted = Path::new(raw);
        if let Some(pos) = remaining.iter().position(|e| e.path == wanted) {
            updated.push(remaining.remove(pos));
        }
    }
    // Preserve any bookmark the caller did not name, keeping its relative order
    // (e.g. one present at read time but absent from `ordered_paths`).
    updated.append(&mut remaining);

    let val = serde_json::to_value(&updated).map_err(|e| e.to_string())?;
    crate::state_store::set_state_field("bookmarks", val)?;
    Ok(())
}

fn list_from_global() -> Vec<BookmarkEntry> {
    let val = crate::state_store::global_state()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    if let serde_json::Value::Object(map) = &val {
        if let Some(bookmarks_val) = map.get("bookmarks") {
            if let Ok(entries) = serde_json::from_value::<Vec<BookmarkEntry>>(bookmarks_val.clone()) {
                return entries;
            }
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    // Bookmarks mutate the process-global state object, so these tests must run
    // serially and against the crate-shared isolated state dir.
    fn guard() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        crate::state_store::ensure_shared_test_state_dir();
        LOCK.get_or_init(|| Mutex::new(())).lock().unwrap_or_else(|p| p.into_inner())
    }

    fn reset() {
        let _ = crate::state_store::set_state_field("bookmarks", serde_json::json!([]));
    }

    #[test]
    fn reorder_applies_requested_order_and_preserves_timestamps() {
        let _g = guard();
        reset();
        add(Path::new("/ws/a.md")).unwrap();
        add(Path::new("/ws/b.md")).unwrap();
        add(Path::new("/ws/c.md")).unwrap();

        let before: std::collections::HashMap<_, _> = list()
            .into_iter()
            .map(|e| (e.path.clone(), e.bookmarked_at))
            .collect();

        reorder(&[
            "/ws/a.md".into(),
            "/ws/c.md".into(),
            "/ws/b.md".into(),
        ])
        .unwrap();

        let after = list();
        let order: Vec<String> = after
            .iter()
            .map(|e| e.path.to_string_lossy().into_owned())
            .collect();
        assert_eq!(order, vec!["/ws/a.md", "/ws/c.md", "/ws/b.md"]);
        // The documented invariant: reorder never alters bookmarked_at.
        for entry in &after {
            assert_eq!(before.get(&entry.path), Some(&entry.bookmarked_at));
        }
    }

    #[test]
    fn reorder_with_empty_input_is_a_noop() {
        let _g = guard();
        reset();
        add(Path::new("/ws/a.md")).unwrap();
        add(Path::new("/ws/b.md")).unwrap();

        reorder(&[]).unwrap();

        let order: Vec<String> = list()
            .into_iter()
            .map(|e| e.path.to_string_lossy().into_owned())
            .collect();
        // Nothing named → every bookmark kept in its existing (insertion) order.
        assert_eq!(order, vec!["/ws/b.md", "/ws/a.md"]);
    }

    #[test]
    fn reorder_appends_unmentioned_and_ignores_unknown() {
        let _g = guard();
        reset();
        add(Path::new("/ws/x.md")).unwrap();
        add(Path::new("/ws/y.md")).unwrap();

        // Only mention y, plus an unknown path; x must survive at the end.
        reorder(&["/ws/y.md".into(), "/ws/ghost.md".into()]).unwrap();

        let order: Vec<String> = list()
            .into_iter()
            .map(|e| e.path.to_string_lossy().into_owned())
            .collect();
        assert_eq!(order, vec!["/ws/y.md", "/ws/x.md"]);
    }
}
