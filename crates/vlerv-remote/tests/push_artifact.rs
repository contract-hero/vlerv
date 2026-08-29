// Push proof: an in-process two-endpoint session where the CLIENT sends the
// bytes. A guest pairs into a host, is refused a push under `browse`, is
// granted `control`, and then lands a verified artifact in the host's
// `received/` folder — the same folder, the same BLAKE3 verification and the
// same collision naming an accepted Beam produces.
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

use iroh::{EndpointAddr, TransportAddr};
use vlerv_remote::host::{EmptyCatalog, HostSignal};
use vlerv_remote::peers::{PeerStore, Pairing, Scope};
use vlerv_remote::proto::Req;
use vlerv_remote::scope::{ClientSession, ScopeState, TabsCache};
use vlerv_remote::security::RootSet;
use vlerv_remote::{endpoint, Dirs};

/// A node's address with its transport reduced to loopback — the endpoint
/// binds 0.0.0.0, so 127.0.0.1 always reaches it.
async fn loopback_addr(node: &endpoint::RemoteNode) -> EndpointAddr {
    let addr = node.endpoint.addr();
    let port = match addr.ip_addrs().find(|a| a.is_ipv4()) {
        Some(a) => a.port(),
        None => {
            // Direct addresses appear a moment after bind on some machines.
            let _ =
                tokio::time::timeout(std::time::Duration::from_secs(10), node.endpoint.online())
                    .await;
            node.endpoint
                .addr()
                .ip_addrs()
                .find(|a| a.is_ipv4())
                .expect("the endpoint publishes an IPv4 direct address")
                .port()
        }
    };
    EndpointAddr::from_parts(
        addr.id,
        [TransportAddr::Ip((std::net::Ipv4Addr::LOCALHOST, port).into())],
    )
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
    host_peers.upsert(&guest_id, "MacBook", Scope::Browse).unwrap();

    let session = ClientSession::connect(&guest, host_addr.clone(), "MacBook".to_string(), |_| {}, || {})
        .await
        .expect("a paired peer gets a session");
    assert_eq!(session.scope, "browse");

    // ── A non-control peer cannot push ──────────────────────────────────────
    assert_eq!(
        session
            .push_artifact_via(&artifact, &guest_roots, guest_addr.clone())
            .await
            .unwrap_err(),
        "not permitted for this peer",
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
        "path not found or out of root",
        "outbound actions stay conservative when no workspace is picked"
    );
    assert_eq!(
        session
            .push_artifact_via(&workspace.path().join("nope.html"), &guest_roots, guest_addr.clone())
            .await
            .unwrap_err(),
        "path not found or out of root"
    );
    // A directory is not an artifact.
    assert_eq!(
        session
            .push_artifact_via(workspace.path(), &guest_roots, guest_addr.clone())
            .await
            .unwrap_err(),
        "only files can be beamed"
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
