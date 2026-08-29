// The proof that this MCP server really delivers: an in-process two-endpoint
// run where a `vlerv-remote` host receives a `PushArtifact` sent by the tool
// handler `send_to_device` calls.
//
// The stdio transport is deliberately out of the picture — it carries JSON and
// nothing else, and rmcp owns it. What is worth proving is the half this crate
// wrote: argument to peer to session to verified bytes on the other machine.
//
// The DATA PATH is loopback, like every other two-endpoint test in this
// workspace: the core dials `127.0.0.1:<host port>` and mints its push ticket
// on `127.0.0.1:<own port>`, so no relay and no discovery lookup can make the
// proof flaky.

use std::sync::{Arc, Mutex};

use vlerv_mcp::core::McpCore;
use vlerv_mcp::devices::ResolveError;
use vlerv_remote::host::{EmptyCatalog, HostSignal};
use vlerv_remote::peers::{Pairing, PeerStore, Scope};
use vlerv_remote::scope::{ScopeState, TabsCache};
use vlerv_remote::security::RootSet;
use vlerv_remote::{endpoint, Dirs};

/// One receiving device: a headless `vlerv-remote` host with no catalog, no
/// roots and no application shell — the same shape a Vlervcode iOS instance
/// presents to the wire.
struct Host {
    node: Arc<endpoint::RemoteNode>,
    peers: Arc<PeerStore>,
    dirs: Dirs,
    signals: Arc<Mutex<Vec<HostSignal>>>,
}

async fn boot_host(dir: &std::path::Path, name: &str) -> Host {
    let signals: Arc<Mutex<Vec<HostSignal>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = signals.clone();
    let peers = Arc::new(PeerStore::load(dir));
    let state = Arc::new(ScopeState::new(
        peers.clone(),
        Arc::new(Pairing::new()),
        Arc::new(TabsCache::new()),
        // A push reads nothing on the receiving side, so the receiver needs no
        // workspace at all.
        RootSet::empty(),
        name.to_string(),
        Arc::new(EmptyCatalog),
        move |signal| sink.lock().unwrap().push(signal),
    ));
    let dirs = Dirs::new(dir);
    let node = Arc::new(
        endpoint::boot(&dirs, Some(state), |_| {}).await.expect("host boot"),
    );
    Host { node, peers, dirs, signals }
}

impl Host {
    fn node_id(&self) -> String {
        self.node.endpoint.id().to_string()
    }

    fn received(&self) -> Vec<(String, std::path::PathBuf, String, u64, String)> {
        self.signals
            .lock()
            .unwrap()
            .iter()
            .filter_map(|s| match s {
                HostSignal::ArtifactReceived { peer, path, name, size, hash } => {
                    Some((peer.clone(), path.clone(), name.clone(), *size, hash.clone()))
                }
                _ => None,
            })
            .collect()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn send_to_device_lands_a_verified_artifact_on_a_paired_host() {
    let host_dir = tempfile::TempDir::new().unwrap();
    let mcp_dir = tempfile::TempDir::new().unwrap();
    let workspace = tempfile::TempDir::new().unwrap();

    let artifact = workspace.path().join("report.html");
    let body = "<!doctype html><h1>from the agent</h1>".repeat(32);
    std::fs::write(&artifact, &body).unwrap();

    let host = boot_host(host_dir.path(), "Val's iPhone").await;
    let host_socket = endpoint::loopback_socket(&host.node)
        .await
        .expect("the host publishes a direct IPv4 address");

    let core = McpCore::new(
        mcp_dir.path().to_path_buf(),
        vec![workspace.path().to_path_buf()],
        workspace.path().to_path_buf(),
        None,
    );
    core.use_loopback(host_socket);

    // Both peer stores, written the way `confirm_pairing` writes them. The
    // scope on each side governs what the OTHER machine may do there.
    let mcp_id = core.node_id().unwrap();
    core.peer_store().upsert(&host.node_id(), "Val's iPhone", Scope::ViewOpen).unwrap();
    host.peers.upsert(&mcp_id, core.device(), Scope::Browse).unwrap();

    // ── Without the control grant, the send is refused with an instruction ──
    let err = core.send_to_device(artifact.to_str().unwrap(), "iPhone").await.unwrap_err();
    assert!(err.contains("has not granted this server control"), "{err}");
    assert!(err.contains("\"browse\""), "the message must name the scope it found: {err}");
    assert!(err.contains(core.device()), "the human must know which peer to widen: {err}");
    assert!(
        !host.dirs.received().exists(),
        "a refused send must not create the receiving folder"
    );

    // ── The device grants control; the very next call re-handshakes ─────────
    host.peers.set_scope(&mcp_id, Scope::Control).unwrap();
    let delivery = core
        .send_to_device(artifact.to_str().unwrap(), "iPhone")
        .await
        .expect("a control grant lets the push land");
    assert_eq!(delivery.name, "report.html");
    assert_eq!(delivery.size, body.len() as u64, "the receiver reports the bytes it measured");
    assert_eq!(delivery.device, "Val's iPhone");
    assert_eq!(delivery.node_id, host.node_id());

    let landed = host.received();
    assert_eq!(landed.len(), 1, "the host surfaced exactly one artifact");
    let (from, path, name, size, hash) = &landed[0];
    assert_eq!(from, &mcp_id, "the signal names the MCP server as the sender");
    assert_eq!(name, "report.html");
    assert_eq!(*size, body.len() as u64);
    assert_eq!(hash, &delivery.hash, "both sides verified the same content address");
    assert_eq!(std::fs::read_to_string(path).unwrap(), body, "verified bytes landed");
    assert!(path.starts_with(host.dirs.received()), "it landed in the host's own state dir");

    // A second send of the same file gets a fresh name on the receiving side —
    // the Beam collision rule, because it IS the Beam landing path.
    let again = core.send_to_device(artifact.to_str().unwrap(), "iPhone").await.unwrap();
    assert_eq!(again.name, "report-2.html");

    // ── A node-id prefix names the same device ─────────────────────────────
    let by_prefix = core.send_to_device(artifact.to_str().unwrap(), &host.node_id()[..8]).await;
    assert_eq!(by_prefix.unwrap().node_id, host.node_id());

    // ── The path policy is the share sheet's, not a wider one ──────────────
    let missing = workspace.path().join("nope.html");
    assert_eq!(
        core.send_to_device(missing.to_str().unwrap(), "iPhone").await.unwrap_err(),
        "path not found or out of root"
    );
    assert_eq!(
        core.send_to_device(workspace.path().to_str().unwrap(), "iPhone").await.unwrap_err(),
        "only files can be beamed"
    );

    // ── Revocation on the device takes effect on the next call ─────────────
    host.peers.remove(&mcp_id).unwrap();
    assert!(
        core.send_to_device(artifact.to_str().unwrap(), "iPhone").await.is_err(),
        "a revoked server is refused on its very next request"
    );

    host.node.router.shutdown().await.ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn list_devices_and_server_status_report_what_the_tools_did() {
    let host_dir = tempfile::TempDir::new().unwrap();
    let mcp_dir = tempfile::TempDir::new().unwrap();
    let workspace = tempfile::TempDir::new().unwrap();
    let artifact = workspace.path().join("chart.html");
    std::fs::write(&artifact, "<svg/>").unwrap();

    let host = boot_host(host_dir.path(), "Mac Studio").await;
    let host_socket = endpoint::loopback_socket(&host.node).await.unwrap();

    let core = McpCore::new(
        mcp_dir.path().to_path_buf(),
        vec![workspace.path().to_path_buf()],
        workspace.path().to_path_buf(),
        None,
    );
    core.use_loopback(host_socket);

    // ── Nothing paired, nothing booted ─────────────────────────────────────
    assert!(core.list_devices(false).await.is_empty());
    let before = core.server_status().await.unwrap();
    assert!(!before.booted, "listing must not boot the network stack");
    assert_eq!(before.node_id, core.node_id().unwrap());
    assert!(before.identity_dir.ends_with("remote"));
    assert!(before.active_offers.is_empty());
    assert_eq!(before.roots, vec![workspace.path().canonicalize().unwrap()]);

    // ── A beam link is minted, served, and reported ────────────────────────
    let link = core.beam_artifact(artifact.to_str().unwrap(), Some(2)).await.unwrap();
    assert!(link.link.starts_with("vlerv://receive?ticket="), "{}", link.link);
    assert!(link.link.contains("name=chart.html"), "{}", link.link);
    assert_eq!(link.name, "chart.html");
    assert_eq!(link.size, 6);
    let ttl = link.expires_at - vlerv_remote::peers::now_unix();
    assert!((2 * 3600 - 60..=2 * 3600).contains(&ttl), "a 2 hour ttl, got {ttl}s");

    let after = core.server_status().await.unwrap();
    assert!(after.booted, "minting a link needs the endpoint");
    assert_eq!(after.active_offers.len(), 1);
    assert_eq!(after.active_offers[0].name, "chart.html");
    assert_eq!(after.active_offers[0].fetches, 0);
    assert_eq!(after.node_id, before.node_id, "the identity survives the boot");

    // An out-of-range ttl is refused rather than clamped.
    assert!(core.beam_artifact(artifact.to_str().unwrap(), Some(0)).await.is_err());

    // ── A link is revocable before its TTL, which is the only answer to one
    //    that went to the wrong place ──────────────────────────────────────
    assert_eq!(after.active_offers[0].hash, link.hash, "status reports the id to revoke by");
    assert!(core.stop_beam(Some("ab")).await.is_err(), "too short a prefix names nothing");
    assert!(core.stop_beam(Some(&"f".repeat(64))).await.is_err(), "no such link");
    assert_eq!(
        core.server_status().await.unwrap().active_offers.len(),
        1,
        "a refused stop revokes nothing"
    );
    let stopped = core.stop_beam(Some(&link.hash[..8])).await.unwrap();
    assert_eq!(stopped.len(), 1);
    assert_eq!(stopped[0].name, "chart.html");
    assert!(
        core.server_status().await.unwrap().active_offers.is_empty(),
        "the request gate reads this registry, so the next fetch is refused"
    );
    // Nothing live left: revoking everything is a no-op, not an error.
    assert!(core.stop_beam(None).await.unwrap().is_empty());

    // ── Presence: unknown until something dials ────────────────────────────
    core.peer_store().upsert(&host.node_id(), "Mac Studio", Scope::ViewOpen).unwrap();
    host.peers.upsert(&core.node_id().unwrap(), core.device(), Scope::Control).unwrap();
    let quiet = core.list_devices(false).await;
    assert_eq!(quiet.len(), 1);
    assert_eq!(quiet[0].device, "Mac Studio");
    assert_eq!(quiet[0].scope, "view-open", "the scope shown is the one granted HERE");
    assert_eq!(quiet[0].presence, "unknown", "nothing dialed it yet");
    assert_eq!(quiet[0].node_id_short, host.node_id()[..8]);

    let probed = core.list_devices(true).await;
    assert_eq!(probed[0].presence, "online", "the probe reached the host");
    // The session is now cached, so presence is live without a second dial.
    assert_eq!(core.list_devices(false).await[0].presence, "online");

    let status = core.server_status().await.unwrap();
    assert_eq!(status.paired_devices, 1);
    assert!(status.received_artifacts.is_empty(), "this server received nothing");

    host.node.router.shutdown().await.ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_send_to_a_device_that_is_not_there_never_opens_a_socket() {
    let mcp_dir = tempfile::TempDir::new().unwrap();
    let workspace = tempfile::TempDir::new().unwrap();
    let artifact = workspace.path().join("a.html");
    std::fs::write(&artifact, "<p>a</p>").unwrap();

    let core = McpCore::new(
        mcp_dir.path().to_path_buf(),
        vec![workspace.path().to_path_buf()],
        workspace.path().to_path_buf(),
        None,
    );

    // Nothing paired: the caller is told how to fix that, not "offline".
    let err = core.send_to_device(artifact.to_str().unwrap(), "iPhone").await.unwrap_err();
    assert_eq!(err, ResolveError::NotPaired.to_string());

    // Paired but misnamed: the error lists the names that ARE valid.
    core.peer_store().upsert(&"ab".repeat(32), "Mac Studio", Scope::Control).unwrap();
    let err = core.send_to_device(artifact.to_str().unwrap(), "iPhone").await.unwrap_err();
    assert!(err.contains("Mac Studio (abababab)"), "{err}");

    // An empty device name is an argument error, not a lookup failure.
    assert!(core
        .send_to_device(artifact.to_str().unwrap(), "  ")
        .await
        .unwrap_err()
        .contains("list_devices"));

    // None of the above needed the endpoint.
    assert!(!core.server_status().await.unwrap().booted);
}
