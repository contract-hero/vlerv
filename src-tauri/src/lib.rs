// Vlerv Tauri core library. Modules are declared here so integration tests
// under tests/ can import them as `src_tauri::workspace::...` etc.

pub mod workspace;
pub mod reader;
pub mod deeplink;
pub mod drag_spike;
pub mod security;
pub mod share;
pub mod state_store;
pub mod recents;
pub mod bookmarks;
pub mod watcher;

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
    /// Optional line number from `vlerv://open?path=…&line=N`.
    #[serde(default)]
    pub line: Option<u32>,
}

/// Typed payload emitted on `vlerv://deep-link-error` when the URL is
/// unparseable, the path is rejected by the root check, or the path does
/// not exist.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeepLinkErrorEvent {
    pub url: String,
    pub reason: String,
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

    let (path, kind, line) = match intent {
        deeplink::DeepLinkIntent::Open { path, line } => (path, DeepLinkIntentKind::Open, line),
        deeplink::DeepLinkIntent::Reveal { path } => (path, DeepLinkIntentKind::Reveal, None),
    };

    // Ad-hoc external-open policy: paths that exist but lie outside every
    // configured root — or any path at all when no workspace has been picked
    // yet — are allowed with `out_of_root: true`; anything the OS can't
    // resolve is a hard error. Opening only displays the file locally, so the
    // rootless variant is safe here (share_file stays conservative).
    let (canonical, out_of_root) =
        security::canonicalize_allow_rootless(&path, roots).map_err(|e| make_err(e.to_string()))?;

    if matches!(kind, DeepLinkIntentKind::Open) && !out_of_root {
        // Recents is scoped to in-root files; ad-hoc external opens stay
        // ephemeral until the user adopts a multi-root model.
        let _ = recents::push(&canonical);
    }

    Ok(OpenFileEvent {
        path: canonical,
        intent: kind,
        out_of_root,
        line,
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

#[cfg(test)]
mod dispatch_deep_link_tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // Redirect state_store writes to the crate-shared test tempdir. Without
    // this, `recents::push` → `state_store::set_state_field` spawns a
    // debounced thread (DEBOUNCE_MS=250) that outlives the test and writes to
    // the developer's real ~/Library/Application Support/Vlerv/state.json.
    // The tempdir + set_var live in state_store::ensure_shared_test_state_dir
    // so this module and bookmarks::tests stop racing each other's env value.
    fn ensure_isolated_state_dir() {
        crate::state_store::ensure_shared_test_state_dir();
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

    #[test]
    fn empty_roots_existing_path_falls_through_out_of_root() {
        ensure_isolated_state_dir();
        let dir = TempDir::new().expect("tempdir");
        let file_path = dir.path().join("adhoc.html");
        fs::write(&file_path, "x").expect("write");
        let roots = security::RootSet::empty();
        let url = format!("vlerv://open?path={}", file_path.display());

        let event = dispatch_deep_link(&url, &roots).expect("ok");

        assert_eq!(event.path, file_path.canonicalize().unwrap());
        assert!(event.out_of_root);
    }

    #[test]
    fn empty_roots_missing_path_still_errors() {
        ensure_isolated_state_dir();
        let roots = security::RootSet::empty();
        let url = "vlerv://open?path=/no/such/path/anywhere.html";

        let err = dispatch_deep_link(url, &roots).expect_err("err");
        assert!(!err.reason.is_empty());
    }

    #[test]
    fn open_line_param_survives_dispatch() {
        let (_dir, file_path, roots) = setup_root_with_file("lined.md");
        let url = format!("vlerv://open?path={}&line=42", file_path.display());

        let event = dispatch_deep_link(&url, &roots).expect("ok");
        assert_eq!(event.line, Some(42));
    }

    #[test]
    fn reveal_rejects_relative_path() {
        let err = deeplink::parse("vlerv://reveal?path=relative/path.md").expect_err("err");
        assert!(err.to_string().contains("absolute"));
    }

    #[test]
    fn reveal_rejects_nul_bytes() {
        let err = deeplink::parse("vlerv://reveal?path=/tmp/a%00b").expect_err("err");
        assert!(err.to_string().contains("NUL"));
    }

    // ── Recents side effect (the `!out_of_root` guard's actual purpose) ──

    fn recents_contains(path: &std::path::Path) -> bool {
        recents::list().iter().any(|r| r.path == path)
    }

    #[test]
    fn in_root_open_pushes_recents() {
        let (_dir, file_path, roots) = setup_root_with_file("recorded.md");
        let url = format!("vlerv://open?path={}", file_path.display());

        dispatch_deep_link(&url, &roots).expect("ok");

        assert!(recents_contains(&file_path.canonicalize().unwrap()));
    }

    #[test]
    fn out_of_root_open_stays_out_of_recents() {
        ensure_isolated_state_dir();
        let dir = TempDir::new().expect("tempdir");
        let outside = TempDir::new().expect("tempdir-outside");
        let outside_file = outside.path().join("ephemeral.md");
        fs::write(&outside_file, "x").expect("write");
        let roots = security::RootSet::new(vec![dir.path().to_path_buf()]);
        let url = format!("vlerv://open?path={}", outside_file.display());

        dispatch_deep_link(&url, &roots).expect("ok");

        assert!(!recents_contains(&outside_file.canonicalize().unwrap()));
    }

    #[test]
    fn reveal_does_not_push_recents() {
        let (_dir, file_path, roots) = setup_root_with_file("revealed-not-recorded.md");
        let url = format!("vlerv://reveal?path={}", file_path.display());

        dispatch_deep_link(&url, &roots).expect("ok");

        assert!(!recents_contains(&file_path.canonicalize().unwrap()));
    }
}
