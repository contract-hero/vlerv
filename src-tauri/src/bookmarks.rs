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

/// List bookmarks ordered by bookmarked_at desc (most-recent-first).
pub fn list() -> Vec<BookmarkEntry> {
    let mut entries = list_from_global();
    entries.sort_by(|a, b| b.bookmarked_at.cmp(&a.bookmarked_at));
    entries
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
