// Vlerv Tauri core library. Modules are declared here so integration tests
// under tests/ can import them as `src_tauri::workspace::...` etc.

pub mod workspace;
pub mod reader;
pub mod deeplink;
pub mod drag_spike;
pub mod security;
pub mod state_store;
pub mod recents;
pub mod watcher;

// `url` crate is used for robust URL parsing in the deep-link handler path.
use url::Url;

/// Deep-link error event payload emitted by the lib to the webview when a
/// deep-link path fails the read-side security check or otherwise fails to
/// produce a usable open intent. C2 stub.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DeepLinkEvent {
    Open { path: std::path::PathBuf },
    Error { path: std::path::PathBuf, reason: String },
}

/// C3: event emitted to the webview after a successful Reveal deep-link.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RevealEvent {
    pub path: std::path::PathBuf,
}

impl RevealEvent {
    pub fn path(&self) -> &std::path::PathBuf {
        &self.path
    }
}

/// Intent kind surfaced to the webview as a lowercase string in JSON
/// (`"open"` or `"reveal"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeepLinkIntentKind {
    Open,
    Reveal,
}

/// Typed payload emitted on `vlerv://open-file` after a deep-link is parsed
/// and the path is canonicalized. `out_of_root` is true when the path
/// canonicalizes successfully but falls outside every configured root — the
/// frontend renders these as ad-hoc external files with a visible badge.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OpenFileEvent {
    pub path: std::path::PathBuf,
    pub intent: DeepLinkIntentKind,
    pub out_of_root: bool,
}

/// Typed payload emitted on `vlerv://deep-link-error` when the URL is
/// unparseable, the path is rejected by the root check, or the path does
/// not exist.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeepLinkErrorEvent {
    pub url: String,
    pub reason: String,
}

/// C3: Tauri IPC command — read a file gated by the active RootSet.
/// Every call passes through canonicalize_and_check_root via tauri::State<RootSet>.
/// T-008: no _with_roots escape hatch exists in the exported command surface.
#[tauri::command]
fn read_file_cmd(
    path: std::path::PathBuf,
    roots: tauri::State<security::RootSet>,
) -> Result<reader::SecuredFilePayload, reader::ReadError> {
    reader::read_file_with_roots(&path, &roots)
}

/// C3: handle a `vlerv://reveal?path=<abs>` deep-link by canonicalizing the
/// path through the root check and returning a Reveal event on success.
/// Out-of-root paths return Err (without leaking existence info).
pub fn handle_reveal_deep_link(
    url: &str,
    roots: &security::RootSet,
) -> Result<RevealEvent, String> {
    let intent = deeplink::parse(url).map_err(|e| e.to_string())?;
    let path = match intent {
        deeplink::DeepLinkIntent::Reveal { path } => path,
        other => return Err(format!("expected Reveal intent, got {other:?}")),
    };

    let canonical = security::canonicalize_and_check_root(&path, roots)
        .map_err(|_| "path not found or out of root".to_string())?;

    Ok(RevealEvent { path: canonical })
}

/// C3: handle a `vlerv://open?path=<abs>` deep-link by canonicalizing the
/// path through the root check and pushing the resolved path to Recents.
pub fn handle_deep_link_resolve_and_push(
    url: &str,
    roots: &security::RootSet,
) -> Result<std::path::PathBuf, String> {
    let intent = deeplink::parse(url).map_err(|e| e.to_string())?;
    let path = match intent {
        deeplink::DeepLinkIntent::Open { path, .. } => path,
        other => return Err(format!("expected Open intent, got {other:?}")),
    };

    let canonical = security::canonicalize_and_check_root(&path, roots)
        .map_err(|e| e.to_string())?;

    // Push to Recents.
    recents::push(&canonical).map_err(|e| e.to_string())?;

    Ok(canonical)
}

/// C2: like `handle_deep_link` but gated on `canonicalize_and_check_root`.
/// Returns the event the webview should receive.
pub fn handle_deep_link_with_roots(
    url: &str,
    roots: &security::RootSet,
) -> DeepLinkEvent {
    let intent = match deeplink::parse(url) {
        Ok(i) => i,
        Err(e) => {
            // Extract path from URL if possible for the error event.
            let path = extract_path_from_url(url);
            return DeepLinkEvent::Error {
                path: std::path::PathBuf::from(path),
                reason: e.to_string(),
            };
        }
    };

    let path = match intent {
        deeplink::DeepLinkIntent::Open { path, .. } => path,
        deeplink::DeepLinkIntent::Reveal { path } => path,
    };

    match security::canonicalize_and_check_root(&path, roots) {
        Ok(_canonical) => DeepLinkEvent::Open { path },
        Err(e) => DeepLinkEvent::Error {
            path,
            reason: e.to_string(),
        },
    }
}

/// Best-effort extraction of the path parameter from a vlerv:// URL for error reporting.
/// Uses the `url` crate for robust query-string parsing.
fn extract_path_from_url(raw_url: &str) -> String {
    // Attempt to parse with the `url` crate; fall back to manual extraction.
    let normalised = raw_url.replacen("vlerv://", "https://vlerv-app/", 1);
    if let Ok(parsed) = Url::parse(&normalised) {
        for (k, v) in parsed.query_pairs() {
            if k == "path" {
                return v.into_owned();
            }
        }
    }
    // Manual fallback.
    if let Some(rest) = raw_url.strip_prefix("vlerv://") {
        if let Some(q_pos) = rest.find('?') {
            let query = &rest[q_pos + 1..];
            for pair in query.split('&') {
                let mut parts = pair.splitn(2, '=');
                if let (Some(k), Some(v)) = (parts.next(), parts.next()) {
                    if k == "path" {
                        return percent_encoding::percent_decode_str(v)
                            .decode_utf8_lossy()
                            .into_owned();
                    }
                }
            }
        }
    }
    raw_url.to_string()
}

/// C2: truncate-at-char-boundary helper exposed to tests (B5 carry-forward).
/// Uses `chars().take(N).collect()` semantics so multibyte boundaries never
/// panic. Public for T-019.
pub fn snippet_chars_take(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

/// Parse a `vlerv://` URL, canonicalize the path against `roots`, push the
/// path to Recents on Open intent (best-effort), and return a typed event
/// the webview can consume. Single entry point used by `main.rs`'s
/// `on_open_url` callback.
pub fn dispatch_deep_link(
    url: &str,
    roots: &security::RootSet,
) -> Result<OpenFileEvent, DeepLinkErrorEvent> {
    let make_err = |reason: String| DeepLinkErrorEvent {
        url: url.to_string(),
        reason,
    };

    let intent = deeplink::parse(url).map_err(|e| make_err(e.to_string()))?;

    let (path, kind) = match intent {
        deeplink::DeepLinkIntent::Open { path, .. } => (path, DeepLinkIntentKind::Open),
        deeplink::DeepLinkIntent::Reveal { path } => (path, DeepLinkIntentKind::Reveal),
    };

    // Branch on the security gate: an `OutOfRoot(canonical)` error means the
    // path *does* exist and was canonicalized, but lies outside every
    // configured root. Those become ad-hoc external opens with
    // `out_of_root: true`. Any other variant (CanonicalizeFailed, EmptyRoots)
    // is still a hard error — we never open a path the OS can't resolve.
    let (canonical, out_of_root) = match security::canonicalize_and_check_root(&path, roots) {
        Ok(canonical) => (canonical, false),
        Err(security::OutOfRootError::OutOfRoot(canonical)) => (canonical, true),
        Err(e) => return Err(make_err(e.to_string())),
    };

    if matches!(kind, DeepLinkIntentKind::Open) && !out_of_root {
        // Recents is scoped to in-root files; ad-hoc external opens stay
        // ephemeral until the user adopts a multi-root model.
        let _ = recents::push(&canonical);
    }

    Ok(OpenFileEvent {
        path: canonical,
        intent: kind,
        out_of_root,
    })
}

/// Handle a deep-link URL: parse it and (when the `e2e-hooks` feature is
/// enabled AND `VLERV_E2E_ECHO_LOG` is set) write a content snippet to
/// that log file. The feature gate ensures production builds cannot use
/// the env var as a write-anywhere primitive (B3 / R6-001 fix).
pub fn handle_deep_link(url: &str) {
    let intent = match deeplink::parse(url) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("vlerv: deep-link parse error: {e}");
            return;
        }
    };

    let path = match intent {
        deeplink::DeepLinkIntent::Open { path, .. } => path,
        deeplink::DeepLinkIntent::Reveal { path } => path,
    };
    eprintln!("vlerv: deep-link: {}", path.display());

    #[cfg(any(feature = "e2e-hooks", debug_assertions))]
    e2e_echo(&path);
    #[cfg(not(any(feature = "e2e-hooks", debug_assertions)))]
    let _ = path;
}

#[cfg(any(feature = "e2e-hooks", debug_assertions))]
fn e2e_echo(path: &std::path::Path) {
    use std::io::Write;
    let Ok(log_path) = std::env::var("VLERV_E2E_ECHO_LOG") else {
        return;
    };
    let raw = std::fs::read_to_string(path).unwrap_or_else(|_| String::from("(unreadable)"));
    let snippet = snippet_chars_take(&raw, 200);
    let content = format!("{}\n{}\n", path.display(), snippet);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&log_path)
    {
        let _ = f.write_all(content.as_bytes());
    }
}

/// Entry point invoked by `main.rs`. C2: boots a real Tauri app.
pub fn run() {
    eprintln!("vlerv: run() called — Tauri Builder wired in main.rs");
}

#[cfg(test)]
mod dispatch_deep_link_tests {
    use super::*;
    use std::fs;
    use std::sync::OnceLock;
    use tempfile::TempDir;

    // Redirect state_store writes to a tempdir for the whole test binary.
    // Without this, `recents::push` → `state_store::set_state_field` spawns a
    // debounced thread (DEBOUNCE_MS=250) that outlives the test and writes to
    // the developer's real ~/Library/Application Support/Vlerv/state.json.
    // Set once per process — `std::env::set_var` is process-global, and cargo
    // runs tests in parallel threads, so racing per-test set_vars would point
    // the debounced writer at TempDirs that have already been dropped.
    fn ensure_isolated_state_dir() {
        static STATE_DIR: OnceLock<TempDir> = OnceLock::new();
        let dir = STATE_DIR.get_or_init(|| TempDir::new().expect("state tempdir"));
        std::env::set_var("VLERV_STATE_DIR", dir.path());
    }

    fn setup_root_with_file(name: &str) -> (TempDir, std::path::PathBuf, security::RootSet) {
        ensure_isolated_state_dir();
        let dir = TempDir::new().expect("tempdir");
        let file_path = dir.path().join(name);
        fs::write(&file_path, "content").expect("write");
        let roots = security::RootSet::new(vec![dir.path().to_path_buf()]);
        (dir, file_path, roots)
    }

    #[test]
    fn open_intent_within_root_returns_open_event() {
        let (_dir, file_path, roots) = setup_root_with_file("hello.html");
        let url = format!("vlerv://open?path={}", file_path.display());

        let event = dispatch_deep_link(&url, &roots).expect("ok");

        assert_eq!(event.intent, DeepLinkIntentKind::Open);
        assert_eq!(event.path, file_path.canonicalize().unwrap());
        assert!(!event.out_of_root);
    }

    #[test]
    fn reveal_intent_within_root_returns_reveal_event() {
        let (_dir, file_path, roots) = setup_root_with_file("revealme.txt");
        let url = format!("vlerv://reveal?path={}", file_path.display());

        let event = dispatch_deep_link(&url, &roots).expect("ok");

        assert_eq!(event.intent, DeepLinkIntentKind::Reveal);
        assert_eq!(event.path, file_path.canonicalize().unwrap());
        assert!(!event.out_of_root);
    }

    #[test]
    fn path_outside_roots_returns_ad_hoc_open_event() {
        ensure_isolated_state_dir();
        let dir = TempDir::new().expect("tempdir");
        let outside = TempDir::new().expect("tempdir-outside");
        let outside_file = outside.path().join("a.html");
        fs::write(&outside_file, "x").expect("write");
        let roots = security::RootSet::new(vec![dir.path().to_path_buf()]);
        let url = format!("vlerv://open?path={}", outside_file.display());

        let event = dispatch_deep_link(&url, &roots).expect("ok");

        assert_eq!(event.intent, DeepLinkIntentKind::Open);
        assert_eq!(event.path, outside_file.canonicalize().unwrap());
        assert!(event.out_of_root);
    }

    #[test]
    fn nonexistent_path_still_returns_error() {
        ensure_isolated_state_dir();
        let dir = TempDir::new().expect("tempdir");
        let roots = security::RootSet::new(vec![dir.path().to_path_buf()]);
        let url = "vlerv://open?path=/no/such/path/exists/here.html";

        let err = dispatch_deep_link(url, &roots).expect_err("expected error");

        assert_eq!(err.url, url);
        assert!(!err.reason.is_empty());
    }

    #[test]
    fn malformed_url_returns_parse_error() {
        let roots = security::RootSet::new(vec![std::env::temp_dir()]);
        let err = dispatch_deep_link("notaurl://garbage", &roots).expect_err("err");
        assert_eq!(err.url, "notaurl://garbage");
        assert!(!err.reason.is_empty());
    }
}
