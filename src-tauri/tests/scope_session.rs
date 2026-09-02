// M3–M5 proof: an in-process two-endpoint Scope session. A host boots with a
// peer store, a workspace root and a published tab list; a guest pairs with
// it over `vlerv/pair/0`, then browses, fetches and drives it over
// `vlerv/scope/0` — with every refusal the design promises actually refused.
//
// The DATA PATH is loopback, exactly like the Beam transfer test: the guest
// dials `127.0.0.1:<host port>` instead of resolving the host's NodeId
// through n0 DNS, so no relay and no discovery lookup carry the session.
// Cross-network traversal is the M0 spike's territory, on real machines.

use std::sync::{Arc, Mutex};

use src_tauri::remote::peers::{PairTicket, PeerStore, Pairing, Scope};
use src_tauri::remote::proto::{Event, TabEntry};
use src_tauri::remote::scope::{self, ClientSession, HostSignal, ScopeState, TabsCache, DENIED};
use src_tauri::remote::{endpoint, peers, AppCatalog, Dirs, EmptyCatalog};
use src_tauri::security::RootSet;

/// Point every state_store read (bookmarks, recents) at a tempdir: the host's
/// published set consults them, and a test must never touch the developer's
/// real `~/Library/Application Support/Vlerv/state.json`.
fn isolate_state_dir() {
    use std::sync::OnceLock;
    static DIR: OnceLock<tempfile::TempDir> = OnceLock::new();
    static SET: OnceLock<()> = OnceLock::new();
    let dir = DIR.get_or_init(|| tempfile::TempDir::new().expect("state tempdir"));
    SET.get_or_init(|| std::env::set_var("VLERV_STATE_DIR", dir.path()));
}

/// The host's address with its transport reduced to loopback — the endpoint
/// binds 0.0.0.0, so 127.0.0.1 always reaches it. Both halves are the crate's
/// own helpers, so this test cannot drift from how the app names a peer.
async fn loopback_addr(node: &endpoint::RemoteNode) -> iroh::EndpointAddr {
    let socket = endpoint::loopback_socket(node)
        .await
        .expect("the endpoint publishes an IPv4 direct address");
    endpoint::addr_at_id(node.endpoint.id(), socket)
}

#[tokio::test(flavor = "multi_thread")]
async fn scope_pairs_then_serves_under_the_granted_scope() {
    isolate_state_dir();
    let host_dir = tempfile::TempDir::new().unwrap();
    let guest_dir = tempfile::TempDir::new().unwrap();
    let workspace = tempfile::TempDir::new().unwrap();
    let cache = tempfile::TempDir::new().unwrap();

    // The host's workspace: one artifact it will publish as an open tab, one
    // in-root file it never mentions, and one file outside every root.
    let artifact = workspace.path().join("report.html");
    let unmentioned = workspace.path().join("private.md");
    std::fs::write(&artifact, "<!doctype html><h1>scoped</h1>").unwrap();
    std::fs::write(&unmentioned, "# not published").unwrap();
    std::fs::create_dir(workspace.path().join("sub")).unwrap();
    let outside = tempfile::TempDir::new().unwrap();
    let secret = outside.path().join("secret.txt");
    std::fs::write(&secret, "out of root").unwrap();

    // ── Host ────────────────────────────────────────────────────────────────
    let signals: Arc<Mutex<Vec<HostSignal>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = signals.clone();
    let host_peers = Arc::new(PeerStore::load(host_dir.path()));
    let host_pairing = Arc::new(Pairing::new());
    let host_state = Arc::new(ScopeState::new(
        host_peers.clone(),
        host_pairing.clone(),
        Arc::new(TabsCache::new()),
        RootSet::new(vec![workspace.path().to_path_buf()]),
        "Mac Studio".to_string(),
        // The app's own catalog seam: bookmarks and recents from state.json,
        // which `isolate_state_dir` pointed at a tempdir.
        Arc::new(AppCatalog),
        move |signal| sink.lock().unwrap().push(signal),
    ));
    let host = endpoint::boot(&Dirs::new(host_dir.path()), Some(host_state.clone()), |_| {})
        .await
        .expect("host boot");
    let host_addr = loopback_addr(&host).await;
    let host_id = host.endpoint.id();

    // ── Guest ───────────────────────────────────────────────────────────────
    let guest_peers = Arc::new(PeerStore::load(guest_dir.path()));
    let guest_state = Arc::new(ScopeState::new(
        guest_peers.clone(),
        Arc::new(Pairing::new()),
        Arc::new(TabsCache::new()),
        RootSet::empty(),
        "MacBook".to_string(),
        Arc::new(EmptyCatalog),
        |_| {},
    ));
    let guest = endpoint::boot(&Dirs::new(guest_dir.path()), Some(guest_state), |_| {})
        .await
        .expect("guest boot");
    let guest_id = guest.endpoint.id();

    // ── A stranger is refused at the handshake ──────────────────────────────
    let err = ClientSession::connect(
        &guest,
        host_addr.clone(),
        "MacBook".to_string(),
        |_| {},
        || {},
    )
    .await
    .expect_err("an unpaired NodeId must not get a session");
    assert!(!err.to_string().is_empty(), "refusal carries a reason: {err}");

    // ── Pairing ─────────────────────────────────────────────────────────────
    let token = host_pairing.mint();
    let ticket = PairTicket {
        addr: host_addr.clone(),
        token,
        device: "Mac Studio".to_string(),
    };
    // The link the user carries between machines survives its own parser.
    let link = peers::build_pair_link(&ticket.to_string());
    match src_tauri::deeplink::parse(&link).expect("own pair link re-parses") {
        src_tauri::deeplink::DeepLinkIntent::Pair { ticket: parsed } => {
            assert_eq!(parsed, ticket.to_string())
        }
        other => panic!("expected Pair, got {other:?}"),
    }

    let pending = scope::pair_dial(&guest, &ticket, "MacBook".to_string())
        .await
        .expect("pairing handshake");
    assert_eq!(pending.node_id, host_id.to_string());
    assert_eq!(pending.device, "Mac Studio");
    assert_eq!(pending.role, "guest");

    // Both screens derived the same six words.
    let host_signal = signals.lock().unwrap().first().cloned().expect("host saw the pairing");
    let HostSignal::PairPending(host_pending) = host_signal else {
        panic!("expected a pairing signal");
    };
    assert_eq!(host_pending.node_id, guest_id.to_string());
    assert_eq!(host_pending.device, "MacBook");
    assert_eq!(host_pending.role, "host");
    assert_eq!(
        host_pending.fingerprint, pending.fingerprint,
        "the two machines must show identical words"
    );
    assert_eq!(pending.fingerprint.len(), peers::FINGERPRINT_WORDS);

    // The token is spent: a forwarded link pairs nobody a second time.
    assert!(
        scope::pair_dial(&guest, &ticket, "MacBook".to_string()).await.is_err(),
        "a replayed pairing link must find nothing"
    );

    // Each side persists the other only after its own user confirmed.
    host_peers
        .seed(&guest_id.to_string(), "MacBook", Scope::ViewOpen)
        .unwrap();
    guest_peers
        .seed(&host_id.to_string(), "Mac Studio", Scope::Browse)
        .unwrap();

    // ── The session, under view-open ────────────────────────────────────────
    host_state.publish_tabs(vec![TabEntry {
        path: artifact.to_string_lossy().into_owned(),
        active: true,
    }]);

    let events: Arc<Mutex<Vec<Event>>> = Arc::new(Mutex::new(Vec::new()));
    let event_sink = events.clone();
    let session = ClientSession::connect(
        &guest,
        host_addr.clone(),
        "MacBook".to_string(),
        move |event| event_sink.lock().unwrap().push(event),
        || {},
    )
    .await
    .expect("a paired peer gets a session");
    assert_eq!(session.device, "Mac Studio");
    assert_eq!(session.scope, "view-open", "the host tells the client what it may do");

    let tabs = session.list_tabs().await.expect("list tabs");
    assert_eq!(tabs.len(), 1);
    assert_eq!(tabs[0].path, artifact.canonicalize().unwrap().to_string_lossy());

    // view-open cannot walk the tree…
    assert_eq!(
        session.list_tree(workspace.path().to_string_lossy().into_owned()).await.unwrap_err(),
        "not permitted for this peer"
    );
    // …nor drive the host…
    assert_eq!(
        session
            .open_on_host(artifact.to_string_lossy().into_owned(), true)
            .await
            .unwrap_err(),
        "not permitted for this peer"
    );
    // …nor fetch an in-root file it was never told about, or anything out of
    // root, or a traversal — all with the SAME wording.
    for path in [
        unmentioned.to_string_lossy().into_owned(),
        secret.to_string_lossy().into_owned(),
        format!("{}/../{}", workspace.path().display(), "etc/passwd"),
        "relative/path.html".to_string(),
    ] {
        assert_eq!(
            session.get_artifact(path.clone()).await.unwrap_err(),
            DENIED,
            "refusal must not leak existence for {path}"
        );
    }

    // The open tab IS fetchable, and the bytes arrive verified.
    let meta = session
        .get_artifact(artifact.to_string_lossy().into_owned())
        .await
        .expect("the published artifact is fetchable");
    assert_eq!(meta.size, std::fs::metadata(&artifact).unwrap().len());
    assert!(!meta.warn, "a small artifact does not trip the soft cap");
    let cached = scope::fetch_into_cache(&guest, host_addr.clone(), &meta.hash, "", cache.path())
        .await
        .expect("blob fetch");
    assert_eq!(
        std::fs::read_to_string(&cached).unwrap(),
        std::fs::read_to_string(&artifact).unwrap()
    );
    assert_eq!(
        cached.file_name().unwrap().to_string_lossy(),
        meta.hash,
        "the cache is content-addressed"
    );
    // A second fetch is a cache hit, not a transfer.
    assert_eq!(
        scope::fetch_into_cache(&guest, host_addr.clone(), &meta.hash, "", cache.path())
            .await
            .unwrap(),
        cached
    );

    // ── Widening the scope takes effect on the NEXT request ─────────────────
    host_peers.set_scope(&guest_id.to_string(), Scope::Control).unwrap();
    let tree = session
        .list_tree(workspace.path().to_string_lossy().into_owned())
        .await
        .expect("browse walks the tree");
    let names: Vec<&str> = tree.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["sub", "private.md", "report.html"]);

    session
        .open_on_host(artifact.to_string_lossy().into_owned(), true)
        .await
        .expect("control drives the host");
    let opened = signals
        .lock()
        .unwrap()
        .iter()
        .find_map(|s| match s {
            HostSignal::OpenOnHost { path, reader_mode, .. } => Some((path.clone(), *reader_mode)),
            _ => None,
        })
        .expect("the host was asked to open the artifact");
    assert_eq!(opened.0, artifact.canonicalize().unwrap());
    assert!(opened.1, "reader mode crosses the wire");

    // ── Live: subscriptions carry tab and file events ───────────────────────
    session.subscribe().await.expect("subscribe");
    host_state.publish_tabs(vec![
        TabEntry { path: artifact.to_string_lossy().into_owned(), active: false },
        TabEntry { path: unmentioned.to_string_lossy().into_owned(), active: true },
    ]);
    // The watcher's job, done by hand: the file changed on disk.
    std::fs::write(&artifact, "<!doctype html><h1>rewritten</h1>").unwrap();
    host.scope
        .as_ref()
        .expect("the host booted with a scope server")
        .note_change(&artifact, false)
        .await;

    let canonical = artifact.canonicalize().unwrap().to_string_lossy().into_owned();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let seen = events.lock().unwrap().clone();
        let has_change = seen.iter().any(|e| matches!(e, Event::FileChanged { path, hash } if *path == canonical && *hash != meta.hash));
        let has_tab = seen.iter().any(|e| matches!(e, Event::TabOpened { path } if path.contains("private.md")));
        if has_change && has_tab {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "expected live events, saw {seen:?}");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    // ── Revocation is immediate ─────────────────────────────────────────────
    host_peers.remove(&guest_id.to_string()).unwrap();
    assert!(
        session.list_tabs().await.is_err(),
        "a revoked peer is refused on its very next request"
    );
}
