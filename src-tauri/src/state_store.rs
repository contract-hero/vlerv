// State store — JSON-backed settings document at
// `~/Library/Application Support/Vlerv/state.json`.
//
// - Schema-versioned with unknown-field-preserving round-trip (R12.4)
// - VLERV_STATE_DIR env-var override for tests
// - Atomic write (tmp + rename)
// - Debounced writer (~250 ms quiet window)
// - Corrupt-file recovery (rename to .broken.<unix-ts> and return defaults)

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentEntry {
    pub path: PathBuf,
    pub opened_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookmarkEntry {
    pub path: PathBuf,
    pub bookmarked_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowGeom {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Default for WindowGeom {
    fn default() -> Self {
        Self { x: 0, y: 0, width: 1280, height: 800 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PaneSizes {
    pub sidebar_px: u32,
    pub preview_px: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub ignore_globs: Vec<String>,
    pub drag_out_mode: String,
    /// Slack share target: a full `slack://…` URL or a `TEAMID/CHANNELID`
    /// shorthand the frontend expands. None = the Open-in-Slack affordance
    /// stays hidden.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slack_target: Option<String>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self { ignore_globs: Vec::new(), drag_out_mode: "file".to_string(), slack_target: None }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub schema_version: u32,
    pub roots: Vec<PathBuf>,
    pub recents: Vec<RecentEntry>,
    pub bookmarks: Vec<BookmarkEntry>,
    pub window: WindowGeom,
    pub panes: PaneSizes,
    pub preferences: Preferences,
}

impl Default for State {
    fn default() -> Self {
        Self {
            schema_version: 1,
            roots: Vec::new(),
            recents: Vec::new(),
            bookmarks: Vec::new(),
            window: WindowGeom::default(),
            panes: PaneSizes::default(),
            preferences: Preferences::default(),
        }
    }
}

/// Resolve the directory that holds `state.json`. Honors the
/// `VLERV_STATE_DIR` env var when set (tests use this to point at tempdirs).
pub fn state_dir() -> PathBuf {
    if let Ok(s) = std::env::var("VLERV_STATE_DIR") {
        return PathBuf::from(s);
    }
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("Vlerv")
}

/// Path to the active `state.json` (under `state_dir()`).
pub fn state_path() -> PathBuf {
    state_dir().join("state.json")
}

// ────────────────────────────────────────────────────────────────────────────
// Global in-memory state, write counter, and debounce machinery.
// We use serde_json::Value as intermediate so unknown fields survive round-trips.
// ────────────────────────────────────────────────────────────────────────────

static GLOBAL_STATE: OnceLock<Arc<Mutex<serde_json::Value>>> = OnceLock::new();
static WRITE_COUNTER: OnceLock<Arc<AtomicU64>> = OnceLock::new();
static PENDING_WRITE: OnceLock<Arc<Mutex<Option<std::time::Instant>>>> = OnceLock::new();

const DEBOUNCE_MS: u64 = 250;

pub(crate) fn global_state() -> &'static Arc<Mutex<serde_json::Value>> {
    GLOBAL_STATE.get_or_init(|| Arc::new(Mutex::new(serde_json::Value::Null)))
}

fn write_counter_arc() -> &'static Arc<AtomicU64> {
    WRITE_COUNTER.get_or_init(|| Arc::new(AtomicU64::new(0)))
}

fn pending_write() -> &'static Arc<Mutex<Option<std::time::Instant>>> {
    PENDING_WRITE.get_or_init(|| Arc::new(Mutex::new(None)))
}

/// Read the current in-memory state as a raw `serde_json::Value` (clones the
/// global). Preserves unknown fields. Use this for the `get_state` Tauri
/// command so the frontend sees the full document including any keys this
/// build doesn't know about.
pub fn current_state_value() -> serde_json::Value {
    global_state().lock().unwrap_or_else(|p| p.into_inner()).clone()
}

/// Read the current in-memory state as a structured State.
/// Does NOT read from disk — reads from the in-memory global Value.
pub fn current_state() -> State {
    let val = global_state().lock().unwrap_or_else(|p| p.into_inner()).clone();
    if val.is_object() {
        serde_json::from_value(val).unwrap_or_default()
    } else {
        State::default()
    }
}

/// Load the state document.
/// - Missing file → returns default state.
/// - Corrupt file → renames to .broken.<unix-ts>, returns default state.
/// - Valid file → deserializes, updates global cache, returns structured State.
pub fn load() -> State {
    let path = state_path();

    if !path.exists() {
        let default_state = State::default();
        let val = serde_json::to_value(&default_state)
            .unwrap_or_else(|_| serde_json::Value::Object(Default::default()));
        *global_state().lock().unwrap_or_else(|p| p.into_inner()) = val;
        return default_state;
    }

    let raw = match std::fs::read_to_string(&path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("vlerv: failed to read state.json: {e}");
            let default_state = State::default();
            let val = serde_json::to_value(&default_state).unwrap_or_default();
            *global_state().lock().unwrap_or_else(|p| p.into_inner()) = val;
            return default_state;
        }
    };

    match serde_json::from_str::<serde_json::Value>(&raw) {
        Ok(val) => {
            // Parse as structured State (with defaults for missing fields).
            let state: State = serde_json::from_value(val.clone()).unwrap_or_default();
            // Store the raw Value so unknown fields are preserved on save.
            *global_state().lock().unwrap_or_else(|p| p.into_inner()) = val;
            state
        }
        Err(e) => {
            eprintln!("vlerv: state.json is corrupt ({e}), recovering");
            // Rename corrupt file.
            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let broken_path = path.with_file_name(format!("state.json.broken.{ts}"));
            if let Err(re) = std::fs::rename(&path, &broken_path) {
                eprintln!("vlerv: failed to rename corrupt state.json: {re}");
            }
            let default_state = State::default();
            let val = serde_json::to_value(&default_state).unwrap_or_default();
            *global_state().lock().unwrap_or_else(|p| p.into_inner()) = val;
            default_state
        }
    }
}

/// Save the state document atomically (write-tmp + rename).
/// Merges the given State into the current global Value to preserve unknown fields.
pub fn save(state: &State) -> Result<(), String> {
    let dir = state_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let path = state_path();
    let tmp_path = path.with_extension("json.tmp");

    // Merge: start from current global value (preserves unknown fields),
    // then overwrite known keys from the provided state.
    let mut base_val = global_state()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();

    if !base_val.is_object() {
        base_val = serde_json::Value::Object(Default::default());
    }

    let state_val = serde_json::to_value(state).map_err(|e| e.to_string())?;
    if let (serde_json::Value::Object(base_map), serde_json::Value::Object(state_map)) =
        (&mut base_val, state_val)
    {
        for (k, v) in state_map {
            base_map.insert(k, v);
        }
    }

    // Update global state to reflect the merged value.
    *global_state().lock().unwrap_or_else(|p| p.into_inner()) = base_val.clone();

    let json = serde_json::to_string_pretty(&base_val).map_err(|e| e.to_string())?;

    std::fs::write(&tmp_path, &json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp_path, &path).map_err(|e| e.to_string())?;

    write_counter_arc().fetch_add(1, Ordering::SeqCst);
    Ok(())
}

/// Update one field by key path; schedules a debounced disk write.
/// Supports top-level keys ("roots") and dot-nested keys ("preferences.ignore_globs").
pub fn set_state_field(key: &str, value: serde_json::Value) -> Result<(), String> {
    {
        let mut global = global_state().lock().unwrap_or_else(|p| p.into_inner());
        if !global.is_object() {
            // Initialize with default state if not yet loaded.
            let default_val = serde_json::to_value(&State::default()).unwrap_or_default();
            *global = default_val;
        }
        set_nested_key(&mut global, key, value)?;
    }

    // Schedule debounced write.
    {
        let mut pending = pending_write().lock().unwrap_or_else(|p| p.into_inner());
        let now = std::time::Instant::now();
        let was_none = pending.is_none();
        *pending = Some(now);
        if was_none {
            // Spawn a thread to flush after the debounce window.
            let state_arc = Arc::clone(global_state());
            let pending_arc = Arc::clone(pending_write());
            let counter_arc = Arc::clone(write_counter_arc());
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_millis(DEBOUNCE_MS));
                    let mut pending_guard = pending_arc.lock().unwrap_or_else(|p| p.into_inner());
                    if let Some(last) = *pending_guard {
                        if last.elapsed() >= Duration::from_millis(DEBOUNCE_MS) {
                            // Time to flush.
                            *pending_guard = None;
                            drop(pending_guard);
                            let val = state_arc.lock().unwrap_or_else(|p| p.into_inner()).clone();
                            do_write_value(&val, &counter_arc);
                            break;
                        }
                    } else {
                        break;
                    }
                }
            });
        }
    }

    Ok(())
}

fn set_nested_key(
    val: &mut serde_json::Value,
    key: &str,
    value: serde_json::Value,
) -> Result<(), String> {
    let parts: Vec<&str> = key.splitn(2, '.').collect();
    match parts.as_slice() {
        [top_key] => {
            if let serde_json::Value::Object(map) = val {
                map.insert(top_key.to_string(), value);
                Ok(())
            } else {
                Err("global state is not an object".to_string())
            }
        }
        [top_key, rest] => {
            if let serde_json::Value::Object(map) = val {
                let sub = map
                    .entry(top_key.to_string())
                    .or_insert_with(|| serde_json::Value::Object(Default::default()));
                set_nested_key(sub, rest, value)
            } else {
                Err("global state is not an object".to_string())
            }
        }
        _ => Err("empty key".to_string()),
    }
}

fn do_write_value(val: &serde_json::Value, counter: &AtomicU64) {
    let dir = state_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("vlerv: cannot create state dir: {e}");
        return;
    }
    let path = state_path();
    let tmp_path = path.with_extension("json.tmp");
    let json = match serde_json::to_string_pretty(val) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("vlerv: cannot serialize state: {e}");
            return;
        }
    };
    if let Err(e) = std::fs::write(&tmp_path, &json) {
        eprintln!("vlerv: cannot write state.json.tmp: {e}");
        return;
    }
    if let Err(e) = std::fs::rename(&tmp_path, &path) {
        eprintln!("vlerv: cannot rename state.json.tmp: {e}");
        return;
    }
    counter.fetch_add(1, Ordering::SeqCst);
}

/// Test/inspection: count of disk writes since process start.
pub fn write_count() -> u64 {
    write_counter_arc().load(Ordering::SeqCst)
}

/// Flush any pending debounced write immediately.
/// Only writes if there is a pending (unsent) write scheduled.
pub fn flush() {
    let has_pending = {
        let mut pending = pending_write().lock().unwrap_or_else(|p| p.into_inner());
        let had = pending.is_some();
        *pending = None; // Cancel the background write.
        had
    };
    if has_pending {
        let val = global_state().lock().unwrap_or_else(|p| p.into_inner()).clone();
        if val.is_object() {
            do_write_value(&val, write_counter_arc());
        }
    }
}
