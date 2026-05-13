// Recents store — dedup by absolute path, cap at 10, persistence flows through
// `state_store.rs`.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub use crate::state_store::RecentEntry;

/// Maximum number of Recents entries retained.
pub const MAX_RECENTS: usize = 10;

/// Push a path to the head of the Recents list.
/// - Deduplicates by absolute path: if already present, removes and re-inserts at head.
/// - Caps at MAX_RECENTS: older entries are dropped from the tail.
pub fn push(path: &Path) -> Result<(), String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let new_entry = RecentEntry {
        path: path.to_path_buf(),
        opened_at: now,
    };

    // Load current recents from in-memory global state.
    let current = list_from_global();

    // Remove any existing entry for this path.
    let mut updated: Vec<RecentEntry> = current
        .into_iter()
        .filter(|e| e.path != path)
        .collect();

    // Insert at head.
    updated.insert(0, new_entry);

    // Cap at MAX_RECENTS (keep the most-recently-pushed = lowest index).
    updated.truncate(MAX_RECENTS);

    // Write back to global state.
    let val = serde_json::to_value(&updated).map_err(|e| e.to_string())?;
    crate::state_store::set_state_field("recents", val)?;

    Ok(())
}

/// Clear all Recents.
pub fn clear() -> Result<(), String> {
    let val = serde_json::Value::Array(Vec::new());
    crate::state_store::set_state_field("recents", val)?;
    Ok(())
}

/// List Recents, most-recent-first (by opened_at desc).
/// Reads from the in-memory global state (not disk).
pub fn list() -> Vec<RecentEntry> {
    let mut entries = list_from_global();
    // Sort descending by opened_at (most recent first).
    entries.sort_by(|a, b| b.opened_at.cmp(&a.opened_at));
    entries
}

/// Read recents from the in-memory global Value (not disk).
fn list_from_global() -> Vec<RecentEntry> {
    let val = crate::state_store::global_state()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    if let serde_json::Value::Object(map) = &val {
        if let Some(recents_val) = map.get("recents") {
            if let Ok(entries) = serde_json::from_value::<Vec<RecentEntry>>(recents_val.clone()) {
                return entries;
            }
        }
    }
    Vec::new()
}

/// Helper used by tests / lib.rs: construct an entry without invoking the
/// global store.
pub fn make_entry(path: PathBuf, opened_at: u64) -> RecentEntry {
    RecentEntry { path, opened_at }
}
