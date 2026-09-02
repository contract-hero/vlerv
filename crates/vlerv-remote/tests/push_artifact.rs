// Push proof: an in-process two-endpoint session where the CLIENT sends the
// bytes. A guest pairs into a host, is refused a push under `browse`, is
// granted `control`, and then lands a verified artifact in the host's
// `received/` folder — the same folder, the same BLAKE3 verification and the
// same collision naming an accepted Beam produces.
//
// The second proof here is the REPLAY: a push that reads no file at all,
// because its bytes were captured when the send was accepted.
//
// The DATA PATH is loopback, exactly like the Beam and Scope tests: the ticket
// the guest mints names `127.0.0.1:<guest port>`, so no relay and no discovery
// lookup carry the push.
//
// This file lives in the crate, not in the Tauri app, on purpose: it is also
// the proof that the remote stack runs with NO application shell — the host
// here is an `EmptyCatalog` plus a closure sink, which is exactly what a
// headless (MCP) host provides.

use std::sync::{Arc, Mutex};

use vlerv_remote::host::{EmptyCatalog, HostSignal};
use vlerv_remote::peers::{PeerStore, Pairing, Scope};
use vlerv_remote::proto::Req;
use vlerv_remote::scope::{ClientSession, PushFailure, ScopeState, TabsCache};
use vlerv_remote::security::RootSet;
use vlerv_remote::{beam, endpoint, Dirs};

/// A node's address with its transport reduced to loopback — the endpoint
/// binds 0.0.0.0, so 127.0.0.1 always reaches it. Both halves are the crate's
/// own helpers, so this test cannot drift from how the app names a peer.
async fn loopback_addr(node: &endpoint::RemoteNode) -> iroh::EndpointAddr {
    let socket = endpoint::loopback_socket(node)
        .await
        .expect("the endpoint publishes an IPv4 direct address");
    endpoint::addr_at_id(node.endpoint.id(), socket)
}

#[tokio::test(flavor = "multi_thread")]
async fn control_peers_push_artifacts_and_lesser_peers_cannot() {
    let host_dir = tempfile::TempDir::new().unwrap();
    let guest_dir = tempfile::TempDir::new().unwrap();
    let workspace = tempfile::TempDir::new().unwrap();

    // The pushing side's workspace: one file it may send, one outside every
    // root that it may not.
    let artifact = workspace.path().join("report.html");
    let body = "<!doctype html><h1>pushed</h1>".repeat(64);
    std::fs::write(&artifact, &body).unwrap();
    let outside = tempfile::TempDir::new().unwrap();
    let secret = outside.path().join("secret.txt");
    std::fs::write(&secret, "out of root").unwrap();

    // ── Host: no catalog, no roots, no application shell ────────────────────
    // A push reads nothing on this machine, so the host needs no workspace at
    // all — which is exactly what an empty RootSet proves.
    let signals: Arc<Mutex<Vec<HostSignal>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = signals.clone();
    let host_peers = Arc::new(PeerStore::load(host_dir.path()));
    let host_state = Arc::new(ScopeState::new(
        host_peers.clone(),
        Arc::new(Pairing::new()),
        Arc::new(TabsCache::new()),
        RootSet::empty(),
        "Mac Studio".to_string(),
        Arc::new(EmptyCatalog),
        move |signal| sink.lock().unwrap().push(signal),
    ));
    let host_dirs = Dirs::new(host_dir.path());
    let host = endpoint::boot(&host_dirs, Some(host_state.clone()), |_| {})
        .await
        .expect("host boot");
    let host_addr = loopback_addr(&host).await;

    // ── Guest: a client only. Its router still answers the blobs ALPN, which
    // is what the host dials back on to pull the pushed bytes. ──────────────
    let guest = endpoint::boot(&Dirs::new(guest_dir.path()), None, |_| {})
        .await
        .expect("guest boot");
    let guest_addr = loopback_addr(&guest).await;
    let guest_id = guest.endpoint.id().to_string();
    let guest_roots = RootSet::new(vec![workspace.path().to_path_buf()]);

    // Paired, but only up to `browse` — the scope a colleague's machine gets.
    host_peers.seed(&guest_id, "MacBook", Scope::Browse).unwrap();

    let session = ClientSession::connect(&guest, host_addr.clone(), "MacBook".to_string(), |_| {}, || {})
        .await
        .expect("a paired peer gets a session");
    assert_eq!(session.scope, "browse");

    // ── A non-control peer cannot push ──────────────────────────────────────
    // The VARIANT is asserted beside the wording throughout this test: the
    // string is what the human reads, and the variant is what tells a caller
    // whether asking again later could ever change the answer.
    assert_eq!(
        session
            .push_artifact_via(&artifact, &guest_roots, guest_addr.clone())
            .await
            .unwrap_err(),
        PushFailure::Denied("not permitted for this peer".to_string()),
        "landing bytes on another machine is control-only"
    );
    assert!(
        !host_dirs.received().exists(),
        "a refused push must not create the received folder"
    );

    // ── Widened to control, the push lands ──────────────────────────────────
    host_peers.set_scope(&guest_id, Scope::Control).unwrap();
    let pushed = session
        .push_artifact_via(&artifact, &guest_roots, guest_addr.clone())
        .await
        .expect("a control peer pushes");
    assert_eq!(pushed.name, "report.html");
    assert_eq!(pushed.size, body.len() as u64, "the host reports the bytes it measured");

    let landed = signals
        .lock()
        .unwrap()
        .iter()
        .find_map(|s| match s {
            HostSignal::ArtifactReceived { peer, path, name, size, hash } => {
                Some((peer.clone(), path.clone(), name.clone(), *size, hash.clone()))
            }
            _ => None,
        })
        .expect("the host surfaced the push like a beam receive");
    assert_eq!(landed.0, guest_id, "the signal names the pushing peer");
    assert_eq!(landed.2, "report.html");
    assert_eq!(landed.3, body.len() as u64);
    assert_eq!(landed.4, pushed.hash);
    assert_eq!(std::fs::read_to_string(&landed.1).unwrap(), body, "verified bytes landed");

    // It landed in `received/<date>/`, inside the host's OWN base dir — the
    // read-only principle holds for a push too.
    assert!(landed.1.starts_with(host_dirs.received()));
    let day = landed.1.parent().unwrap().file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(day.len(), "2026-08-29".len(), "lands in a date directory: {day}");

    // A second push of the same file gets a fresh, non-colliding name — the
    // Beam collision rule, because it IS the Beam landing path.
    let again = session
        .push_artifact_via(&artifact, &guest_roots, guest_addr.clone())
        .await
        .expect("second push");
    assert_eq!(
        again.name, "report-2.html",
        "the host reports the name it actually landed the bytes under"
    );

    // ── The path policy on the pushing side is Beam's, exactly ──────────────
    // A push is an outbound action the local side chooses, so it reuses
    // `beam::resolve_offerable`: an existing out-of-root file is shareable on
    // purpose (the share-sheet rule)…
    let external = session
        .push_artifact_via(&secret, &guest_roots, guest_addr.clone())
        .await
        .expect("an external file is beamable, so it is pushable");
    assert_eq!(external.name, "secret.txt");
    // …but a rootless install sends nothing, and a path the OS cannot resolve
    // is refused with the same no-existence-leak wording.
    assert_eq!(
        session
            .push_artifact_via(&artifact, &RootSet::empty(), guest_addr.clone())
            .await
            .unwrap_err(),
        PushFailure::Local("path not found or out of root".to_string()),
        "outbound actions stay conservative when no workspace is picked"
    );
    assert_eq!(
        session
            .push_artifact_via(&workspace.path().join("nope.html"), &guest_roots, guest_addr.clone())
            .await
            .unwrap_err(),
        PushFailure::Local("path not found or out of root".to_string())
    );
    // A directory is not an artifact.
    assert_eq!(
        session
            .push_artifact_via(workspace.path(), &guest_roots, guest_addr.clone())
            .await
            .unwrap_err(),
        PushFailure::Local("only files can be beamed".to_string())
    );

    // ── The ticket is peer-locked ───────────────────────────────────────────
    // A control peer must not be able to point the host at a third machine:
    // hand-roll a frame whose ticket names somebody else's NodeId.
    let stranger = iroh::SecretKey::from_bytes(&[42u8; 32]).public();
    let hash = iroh_blobs::Hash::new(b"somebody else's bytes");
    let foreign = iroh_blobs::ticket::BlobTicket::new(
        stranger.into(),
        hash,
        iroh_blobs::BlobFormat::Raw,
    )
    .to_string();
    let res = session
        .request(Req::PushArtifact {
            name: "evil.html".to_string(),
            size: 10,
            hash: hash.to_string(),
            ticket: foreign,
        })
        .await
        .expect("the host answers");
    assert_eq!(
        res,
        vlerv_remote::proto::Res::Denied("a pushed ticket must name the pushing peer".to_string())
    );

    // ── An oversized announcement is refused before any dial ────────────────
    let honest = iroh_blobs::ticket::BlobTicket::new(
        guest.endpoint.id().into(),
        hash,
        iroh_blobs::BlobFormat::Raw,
    )
    .to_string();
    let res = session
        .request(Req::PushArtifact {
            name: "huge.bin".to_string(),
            size: u64::MAX,
            hash: hash.to_string(),
            ticket: honest,
        })
        .await
        .expect("the host answers");
    assert_eq!(
        res,
        vlerv_remote::proto::Res::Denied("artifact exceeds the transfer size cap".to_string())
    );

    // ── Revocation stops pushes too ─────────────────────────────────────────
    host_peers.remove(&guest_id).unwrap();
    assert!(
        session
            .push_artifact_via(&artifact, &guest_roots, guest_addr)
            .await
            .is_err(),
        "a revoked peer is refused on its very next request"
    );

    guest.router.shutdown().await.ok();
    host.router.shutdown().await.ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_staged_push_replays_from_the_store_without_reading_the_source_again() {
    // Why a queued send stages bytes instead of remembering a path: by the
    // time a sleeping phone answers, Claude Code has rewritten its own report
    // — or the user has deleted it. A delivery that re-read the source would
    // send something nobody accepted, or fail with nothing to send.
    let host_dir = tempfile::TempDir::new().unwrap();
    let guest_dir = tempfile::TempDir::new().unwrap();
    let workspace = tempfile::TempDir::new().unwrap();
    let artifact = workspace.path().join("report.html");
    let body = "<!doctype html><h1>captured when the send was accepted</h1>".repeat(32);
    std::fs::write(&artifact, &body).unwrap();

    let signals: Arc<Mutex<Vec<HostSignal>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = signals.clone();
    let host_peers = Arc::new(PeerStore::load(host_dir.path()));
    let host_state = Arc::new(ScopeState::new(
        host_peers.clone(),
        Arc::new(Pairing::new()),
        Arc::new(TabsCache::new()),
        RootSet::empty(),
        "Mac Studio".to_string(),
        Arc::new(EmptyCatalog),
        move |signal| sink.lock().unwrap().push(signal),
    ));
    let host = endpoint::boot(&Dirs::new(host_dir.path()), Some(host_state), |_| {})
        .await
        .expect("host boot");
    let host_addr = loopback_addr(&host).await;

    let guest = endpoint::boot(&Dirs::new(guest_dir.path()), None, |_| {})
        .await
        .expect("guest boot");
    let guest_addr = loopback_addr(&guest).await;
    host_peers
        .seed(&guest.endpoint.id().to_string(), "MacBook", Scope::Control)
        .unwrap();

    // The send is accepted while the phone is asleep: the bytes are copied
    // into the guest's own store and pinned under the record's tag.
    let hash = beam::stage_outbox(&guest, &artifact, "1700000000001-0000")
        .await
        .expect("stage");

    // Only now does the file go. From here nothing that reaches the host can
    // have been re-derived from the workspace.
    std::fs::remove_file(&artifact).unwrap();
    assert!(beam::outbox_bytes_present(&guest, &hash).await, "the snapshot outlives its source");

    let session = ClientSession::connect(&guest, host_addr, "MacBook".to_string(), |_| {}, || {})
        .await
        .expect("a control peer gets a session");
    let pushed = session
        .push_staged_via(&hash, "report.html", body.len() as u64, guest_addr)
        .await
        .expect("the replay lands");
    assert_eq!(pushed.hash, hash, "the address the record named is the one that travelled");
    assert_eq!(pushed.size, body.len() as u64);
    assert_eq!(pushed.name, "report.html");

    let landed = signals
        .lock()
        .unwrap()
        .iter()
        .find_map(|s| match s {
            HostSignal::ArtifactReceived { path, hash, .. } => Some((path.clone(), hash.clone())),
            _ => None,
        })
        .expect("the host landed the replayed bytes like any other push");
    assert_eq!(landed.1, hash, "BLAKE3-verified on the way in, as always");
    assert_eq!(
        std::fs::read_to_string(&landed.0).unwrap(),
        body,
        "the bytes the user accepted, from a file that no longer exists"
    );

    guest.router.shutdown().await.ok();
    host.router.shutdown().await.ok();
}
