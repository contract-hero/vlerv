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

use vlerv_mcp::core::{Delivery, McpCore};
use vlerv_mcp::devices::ResolveError;
use vlerv_remote::host::{EmptyCatalog, HostSignal};
use vlerv_remote::outbox::Outbox;
use vlerv_remote::peers::{Pairing, PeerStore, Scope};
use vlerv_remote::scope::{ClientSession, ScopeState, TabsCache};
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

/// The node id a device will present, created without booting anything: an
/// identity key is a file write, and both peer stores have to name the device
/// before it is up.
fn device_identity(dir: &std::path::Path) -> String {
    endpoint::load_or_create_identity(&Dirs::new(dir).remote())
        .expect("an identity key")
        .public()
        .to_string()
}

/// The same device, ASLEEP: its own identity at another address, answering the
/// transport and speaking no scope ALPN, so a dial to it fails the way a dial
/// to a suspended phone does.
///
/// A device that is really suspended costs `DIAL_TIMEOUT` — thirty seconds —
/// to give up on, which is half a minute of test time per queued send. This
/// fails the same `endpoint::dial` in milliseconds and produces the same
/// `ConnectError::Unreachable`, which is the one input the decision to queue
/// reads.
///
/// It boots over a state directory of its OWN, and that is not a detail:
/// `StoreLock::acquire` refuses a second claim inside ONE process, and
/// `router.shutdown()` does not release it, so a sleeping twin sharing the
/// host's directory would make the host's later boot fail with "another Vlerv
/// process is already using the blob store" and read like a queue bug.
async fn boot_asleep(
    dir: &std::path::Path,
    identity_from: &std::path::Path,
) -> endpoint::RemoteNode {
    let remote = Dirs::new(dir).remote();
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::copy(
        Dirs::new(identity_from).remote().join("identity.key"),
        remote.join("identity.key"),
    )
    .expect("the device's identity, so the queued record still names it when it wakes");
    endpoint::boot(&Dirs::new(dir), None, |_| {}).await.expect("the sleeping device")
}

/// Accept one send for a device that does not answer, and hand back the
/// record it became. Every proof about the spool starts here: a file the user
/// asked for, a device that said nothing, and a promise on disk.
async fn queue_while_asleep(
    core: &McpCore,
    asleep: &endpoint::RemoteNode,
    path: &std::path::Path,
) -> (String, String) {
    core.use_loopback(endpoint::loopback_socket(asleep).await.unwrap());
    match core.send_to_device(path.to_str().unwrap(), "iPhone").await.unwrap() {
        Delivery::Queued { id, hash, .. } => (id, hash),
        landed => {
            panic!("a device that did not answer must never be reported delivered: {landed:?}")
        }
    }
}

/// The four temporary directories the stage below stands on, in ONE field so
/// that a test which reads only some of them still keeps all of them. A
/// `TempDir` deletes its tree when it drops, and the sleeping node, the spool
/// and the artifact are all sitting inside these.
struct StageDirs {
    /// The receiving device's: its identity key, its peer store, and where an
    /// artifact lands once it wakes.
    host: tempfile::TempDir,
    /// The server's: the spool, the blob store and the identity. The proofs
    /// that survive a process reopen this one with a SECOND core.
    mcp: tempfile::TempDir,
    /// The one root the server may send from, holding the artifact.
    workspace: tempfile::TempDir,
    /// The sleeping twin's own, which `boot_asleep` explains may not be the
    /// host's. No test names it; it is here to outlive the node.
    _asleep: tempfile::TempDir,
}

/// The stage every queue proof opens on, because none of them can say a word
/// until all of it is standing: a workspace holding one artifact, a device
/// that does not answer, and both peer stores paired at control.
struct Asleep {
    dirs: StageDirs,
    artifact: std::path::PathBuf,
    core: McpCore,
    asleep: endpoint::RemoteNode,
    phone_id: String,
}

/// Build that stage, with `body` as the artifact's contents — the one part
/// each proof wants its own copy of, because the bytes and the size are what
/// it goes on to assert landed.
async fn asleep_fixture(body: &str) -> Asleep {
    let host = tempfile::TempDir::new().unwrap();
    let asleep_dir = tempfile::TempDir::new().unwrap();
    let mcp = tempfile::TempDir::new().unwrap();
    let workspace = tempfile::TempDir::new().unwrap();

    let artifact = workspace.path().join("report.html");
    std::fs::write(&artifact, body).unwrap();

    // The identity is minted before the twin boots, because both peer stores
    // have to name the device while it is still down.
    let phone_id = device_identity(host.path());
    let asleep = boot_asleep(asleep_dir.path(), host.path()).await;

    let core = McpCore::new(
        mcp.path().to_path_buf(),
        vec![workspace.path().to_path_buf()],
        workspace.path().to_path_buf(),
        None,
    );
    // Both peer stores at control, the way `confirm_pairing` writes them.
    let phone_peers = PeerStore::load(host.path());
    core.peer_store().seed(&phone_id, "Val's iPhone", Scope::Control).unwrap();
    phone_peers.seed(&core.node_id().unwrap(), core.device(), Scope::Control).unwrap();

    let dirs = StageDirs { host, mcp, workspace, _asleep: asleep_dir };
    Asleep { dirs, artifact, core, asleep, phone_id }
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
    core.peer_store().seed(&host.node_id(), "Val's iPhone", Scope::ViewOpen).unwrap();
    host.peers.seed(&mcp_id, core.device(), Scope::Browse).unwrap();

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
    // WHICH answer came back is itself an assertion, here and at both sites
    // below: a queued send that read as a delivered one is the exact failure
    // the tagged answer exists to make impossible, and a test that shrugged
    // at it would let it back in.
    let Delivery::Delivered { device, node_id, name, size, hash } = delivery else {
        panic!("expected a delivery, got a queued send: {delivery:?}")
    };
    assert_eq!(name, "report.html");
    assert_eq!(size, body.len() as u64, "the receiver reports the bytes it measured");
    assert_eq!(device, "Val's iPhone");
    assert_eq!(node_id, host.node_id());

    // The session this first successful send established. Every later send
    // must reuse it — checked once the sends are done.
    let first_session = core.cached_session_id(&host.node_id()).await;
    assert!(first_session.is_some(), "the send cached its session");

    let landed = host.received();
    assert_eq!(landed.len(), 1, "the host surfaced exactly one artifact");
    let (from, path, landed_name, landed_size, landed_hash) = &landed[0];
    assert_eq!(from, &mcp_id, "the signal names the MCP server as the sender");
    assert_eq!(landed_name, "report.html");
    assert_eq!(*landed_size, body.len() as u64);
    assert_eq!(landed_hash, &hash, "both sides verified the same content address");
    assert_eq!(std::fs::read_to_string(path).unwrap(), body, "verified bytes landed");
    assert!(path.starts_with(host.dirs.received()), "it landed in the host's own state dir");

    // A second send of the same file gets a fresh name on the receiving side —
    // the Beam collision rule, because it IS the Beam landing path.
    let again = core.send_to_device(artifact.to_str().unwrap(), "iPhone").await.unwrap();
    let Delivery::Delivered { name: again_name, .. } = again else {
        panic!("expected a delivery, got a queued send: {again:?}")
    };
    assert_eq!(again_name, "report-2.html");

    // ── A node-id prefix names the same device ─────────────────────────────
    let by_prefix =
        core.send_to_device(artifact.to_str().unwrap(), &host.node_id()[..8]).await.unwrap();
    let Delivery::Delivered { node_id: by_prefix_node, .. } = by_prefix else {
        panic!("expected a delivery, got a queued send: {by_prefix:?}")
    };
    assert_eq!(by_prefix_node, host.node_id());

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

    // ── Four sends over one device reuse ONE session ──────────────────────
    // Identity, not count: the map is keyed by node id, so its length is 1
    // whether or not the cache-hit path works. Only a stable `Arc` across the
    // sends shows the connection was reused instead of re-dialled each time.
    assert_eq!(core.cached_sessions().await, 1);
    let reused = core.cached_session_id(&host.node_id()).await;
    assert!(reused.is_some(), "the successful sends left a cached session");
    assert_eq!(
        reused, first_session,
        "every send after the control grant reused one session, and did not re-handshake"
    );

    // ── Revocation on the device takes effect on the next call ─────────────
    host.peers.remove(&mcp_id).unwrap();
    assert!(
        core.send_to_device(artifact.to_str().unwrap(), "iPhone").await.is_err(),
        "a revoked server is refused on its very next request"
    );

    // ── A session that dies evicts its own cache entry ────────────────────
    // The MCP server is long-lived and probes every paired device, so a
    // session that is never dropped is one dead connection per device for the
    // life of the process.
    // The revocation closed the session from the host side. Nothing calls
    // `forget_session` on this path, so the cache draining is the eviction
    // callback doing its job.
    let drained = wait_until(|| async { core.cached_sessions().await == 0 }).await;
    assert!(drained, "a closed session must not stay in the cache");

    // Nothing above was queued. Every one of those refusals is an answer
    // about this file and this peer, and a week of retries would collect the
    // same answer every time.
    assert_eq!(core.server_status().await.unwrap().queued_total, 0);

    host.node.router.shutdown().await.ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_send_to_a_sleeping_device_is_accepted_and_lands_when_it_wakes() {
    // The proof the whole queue exists for. A file is sent to a device that
    // does not answer; the tool says so instead of failing, and when that
    // same device — same identity key, same node id, new address — comes
    // back, the bytes captured at the moment of the send land there
    // verified, with nobody asking again.
    let body = "<!doctype html><h1>written while the phone was asleep</h1>".repeat(16);
    let Asleep { dirs, artifact, core, asleep, phone_id } = asleep_fixture(&body).await;

    // ── Accepted, and nothing has arrived anywhere ─────────────────────────
    let (id, hash) = queue_while_asleep(&core, &asleep, &artifact).await;
    assert!(
        !Dirs::new(dirs.host.path()).received().exists(),
        "a queued send has landed nowhere — that is what queued MEANS"
    );
    assert_eq!(core.server_status().await.unwrap().queued_total, 1);

    // ── The phone wakes ────────────────────────────────────────────────────
    // The sleeping twin goes first: it holds the same identity key, and the
    // device that comes up next must be the only endpoint answering for that
    // node id.
    asleep.router.shutdown().await.ok();
    drop(asleep);
    let phone = boot_host(dirs.host.path(), "Val's iPhone").await;
    assert_eq!(phone.node_id(), phone_id, "the same device, so the record still names it");
    // Read LIVE by the drain, which has been running since the send booted
    // this server: a device that comes back at another address is the case.
    core.use_loopback(endpoint::loopback_socket(&phone.node).await.unwrap());
    core.wake_drain(&phone_id);

    assert!(
        wait_until(|| async { phone.received().len() == 1 }).await,
        "the queued send never went out"
    );
    let landed = phone.received();
    let (from, path, name, size, landed_hash) = &landed[0];
    assert_eq!(from, &core.node_id().unwrap(), "the device names this server as the sender");
    assert_eq!(name, "report.html");
    assert_eq!(*size, body.len() as u64, "the device reports the bytes it measured");
    assert_eq!(landed_hash, &hash, "the copy taken at enqueue is the copy that was verified");
    assert_eq!(std::fs::read_to_string(path).unwrap(), body);
    assert!(path.starts_with(Dirs::new(dirs.host.path()).received()));

    // ── And the promise is retired, record and pin ─────────────────────────
    assert!(
        wait_until(|| async { core.server_status().await.unwrap().queued_total == 0 }).await,
        "a delivered record must leave the spool, or it is sent again at the next tick"
    );
    let status = core.server_status().await.unwrap();
    assert_eq!(status.queued_bytes, 0);
    assert_eq!(status.retained_bytes, 0, "the private copy is no longer owed to anybody");
    assert!(
        !Dirs::new(dirs.mcp.path()).outbox().join(format!("{id}.json")).exists(),
        "the record file goes with the delivery, or the next boot replays it"
    );

    phone.node.router.shutdown().await.ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_that_dials_in_delivers_its_own_queue_without_being_asked() {
    // The wake signal, end to end. Nothing here calls `wake_drain` and
    // nothing waits for a timer: the device comes back, dials THIS server the
    // way the app does when it returns to the foreground, and the file it was
    // owed goes out because of that connection alone.
    //
    // The clock is what makes it a proof rather than a coincidence. The send
    // that found the device asleep armed the first rung of the retry ladder,
    // so no timer pass would dial this peer for sixty seconds, and the tick
    // is sixty seconds away as well — while `wait_until` gives up after five.
    let body = "<h1>written while the phone was locked</h1>";
    let Asleep { dirs, artifact, core, asleep, phone_id: _ } = asleep_fixture(body).await;

    let (_, hash) = queue_while_asleep(&core, &asleep, &artifact).await;
    assert_eq!(core.server_status().await.unwrap().queued_total, 1);

    // ── The device comes back and connects, as it does on resume ───────────
    asleep.router.shutdown().await.ok();
    drop(asleep);
    let phone = boot_host(dirs.host.path(), "Val's iPhone").await;
    core.use_loopback(endpoint::loopback_socket(&phone.node).await.unwrap());
    assert!(phone.received().is_empty(), "an address alone delivers nothing");

    let server = endpoint::addr_at(
        &core.node_id().unwrap(),
        core.loopback_socket().await.expect("the server is up, so it has an address"),
    )
    .unwrap();
    let inbound =
        ClientSession::connect(&phone.node, server, "Val's iPhone".to_string(), |_| {}, || {})
            .await
            .expect("the device dials the server it paired with");
    assert_eq!(inbound.scope, "control", "the ack reports what THIS server grants the device");

    assert!(
        wait_until(|| async { phone.received().len() == 1 }).await,
        "the inbound connection is what delivers, and nothing else could have"
    );
    let landed = phone.received();
    assert_eq!(landed[0].0, core.node_id().unwrap(), "the device names this server as the sender");
    assert_eq!(landed[0].2, "report.html");
    assert_eq!(landed[0].4, hash, "the copy taken at enqueue is the copy that was verified");
    assert_eq!(std::fs::read_to_string(&landed[0].1).unwrap(), body);
    assert!(
        wait_until(|| async { core.server_status().await.unwrap().queued_total == 0 }).await,
        "and the promise is retired, so the next pass does not send it again"
    );

    drop(inbound);
    phone.node.router.shutdown().await.ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_spooled_delivery_survives_the_process_that_accepted_it() {
    // The queued answer promises the user that the file goes out at the
    // first network-touching tool call of a LATER session over the same
    // state directory. This is that sentence, tested.
    let body = "<h1>accepted by a session that is now gone</h1>";
    let Asleep { dirs, artifact, core: accepted, asleep, phone_id } = asleep_fixture(body).await;

    let (id, hash) = queue_while_asleep(&accepted, &asleep, &artifact).await;
    let promised = accepted.server_status().await.unwrap().queued[0].clone();
    drop(accepted);

    // ── A SECOND server over the same state directory ──────────────────────
    let next = McpCore::new(
        dirs.mcp.path().to_path_buf(),
        vec![dirs.workspace.path().to_path_buf()],
        dirs.workspace.path().to_path_buf(),
        None,
    );
    let status = next.server_status().await.unwrap();
    assert_eq!(status.queued_total, 1, "reading the spool is a file read, not a boot");
    assert_eq!(status.queued[0], promised, "every reported field, byte for byte");
    assert!(!status.draining, "nothing has booted here, so nothing is moving it");
    // The pin the record names has to survive with it: without the tag, the
    // bytes are the next sweep's orphan and the delivery cannot happen.
    let on_disk = Outbox::load(&Dirs::new(dirs.mcp.path()).outbox()).list();
    assert_eq!(on_disk.len(), 1);
    assert_eq!(on_disk[0].id, id);
    assert_eq!(on_disk[0].hash, hash);
    assert_eq!(on_disk[0].tag, format!("outbox/{id}"));
    assert_eq!(on_disk[0].peer, phone_id);
    assert_eq!(on_disk[0].source, artifact.canonicalize().unwrap());
    assert_eq!(on_disk[0].enqueued_at, promised.enqueued_at);

    // ── The phone wakes, and the next tool call delivers ───────────────────
    asleep.router.shutdown().await.ok();
    drop(asleep);
    let phone = boot_host(dirs.host.path(), "Val's iPhone").await;
    next.use_loopback(endpoint::loopback_socket(&phone.node).await.unwrap());
    // The first server's drain task holds the node — and with it the blob
    // store claim — until its wake channel closes, which happens a scheduler
    // turn after the core is dropped rather than instantly. `StoreLock`
    // refuses rather than queues, so the boot is retried, not raced.
    assert!(
        wait_until(|| async { next.list_devices(true).await.is_ok() }).await,
        "the dropped server must hand the blob store over"
    );
    assert!(
        wait_until(|| async { phone.received().len() == 1 }).await,
        "the boot pass delivers what a dead process accepted"
    );
    let landed = phone.received();
    assert_eq!(landed[0].2, "report.html");
    assert_eq!(std::fs::read_to_string(&landed[0].1).unwrap(), body);
    assert!(
        wait_until(|| async { next.server_status().await.unwrap().queued_total == 0 }).await,
        "and retires the record it delivered"
    );

    phone.node.router.shutdown().await.ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_queued_record_outside_the_current_roots_is_held_and_says_so() {
    // `VLERV_MCP_ROOTS` differs between projects, so the session that picks
    // a record up may not be the one that accepted it. Sending it anyway
    // would push a file the operator has since put out of reach; dropping it
    // would break a promise another session can still keep. It is HELD, and
    // the reason is on the status surface, because a count that never moves
    // and says nothing is the silent failure this queue exists to remove.
    // The fixture's own workspace is the project this send is accepted in; it
    // needs no name here, because every assertion below is about the other one.
    let Asleep { dirs, artifact, core: accepted, asleep, phone_id: _ } =
        asleep_fixture("<h1>report</h1>").await;
    let other_project = tempfile::TempDir::new().unwrap();
    queue_while_asleep(&accepted, &asleep, &artifact).await;
    drop(accepted);

    // The next session over this state directory was started in ANOTHER
    // project, so the file the record names is outside everything it may
    // send.
    let elsewhere = McpCore::new(
        dirs.mcp.path().to_path_buf(),
        vec![other_project.path().to_path_buf()],
        other_project.path().to_path_buf(),
        None,
    );
    asleep.router.shutdown().await.ok();
    drop(asleep);
    let phone = boot_host(dirs.host.path(), "Val's iPhone").await;
    elsewhere.use_loopback(endpoint::loopback_socket(&phone.node).await.unwrap());
    assert!(
        wait_until(|| async { elsewhere.list_devices(true).await.is_ok() }).await,
        "the dropped server must hand the blob store over"
    );

    assert!(
        wait_until(|| async {
            elsewhere
                .server_status()
                .await
                .unwrap()
                .queued
                .first()
                .is_some_and(|q| q.last_error.is_some())
        })
        .await,
        "a record that is not moving must say why"
    );
    let status = elsewhere.server_status().await.unwrap();
    assert_eq!(status.queued_total, 1, "held: not sent, and not dropped either");
    let held = &status.queued[0];
    let why = held.last_error.clone().unwrap();
    assert!(why.contains("outside this server's send roots"), "{why}");
    assert!(why.contains("report.html"), "the file is named: {why}");
    assert!(
        why.contains(&format!("{:?}", other_project.path().canonicalize().unwrap())),
        "and so are the roots that refused it: {why}"
    );
    assert!(
        phone.received().is_empty(),
        "the device is reachable and granted control — the ROOTS are what held it"
    );

    phone.node.router.shutdown().await.ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_caller_error_is_refused_outright_and_never_becomes_a_pending_delivery() {
    // The queue's one dangerous failure mode: a mistake the caller could fix
    // in a second, accepted as a promise instead and repeated for a week
    // while a private copy of the file sits in the state directory. Every
    // refusal that existed before the queue must still be a refusal, with the
    // wording it had, and must leave the spool at zero.
    let host_dir = tempfile::TempDir::new().unwrap();
    let mcp_dir = tempfile::TempDir::new().unwrap();
    let workspace = tempfile::TempDir::new().unwrap();
    let artifact = workspace.path().join("report.html");
    std::fs::write(&artifact, "<h1>report</h1>").unwrap();
    let path = artifact.to_str().unwrap();

    let host = boot_host(host_dir.path(), "Val's iPhone").await;
    let core = McpCore::new(
        mcp_dir.path().to_path_buf(),
        vec![workspace.path().to_path_buf()],
        workspace.path().to_path_buf(),
        None,
    );
    core.use_loopback(endpoint::loopback_socket(&host.node).await.unwrap());

    // Nothing paired at all.
    assert_eq!(
        core.send_to_device(path, "iPhone").await.unwrap_err(),
        ResolveError::NotPaired.to_string()
    );
    assert_eq!(spool(&core).await, 0, "the refusal above queued nothing");

    // Paired, but the name matches nothing.
    let mcp_id = core.node_id().unwrap();
    core.peer_store().seed(&host.node_id(), "Val's iPhone", Scope::ViewOpen).unwrap();
    host.peers.seed(&mcp_id, core.device(), Scope::Browse).unwrap();
    assert!(core.send_to_device(path, "Mac Studio").await.unwrap_err().contains("Val's iPhone"));
    assert_eq!(spool(&core).await, 0, "the refusal above queued nothing");

    // An empty device argument is an argument error, not a lookup failure.
    assert!(core.send_to_device(path, "  ").await.unwrap_err().contains("list_devices"));
    assert_eq!(spool(&core).await, 0, "the refusal above queued nothing");

    // The path gate, both of its refusals.
    assert_eq!(
        core.send_to_device(workspace.path().join("nope.html").to_str().unwrap(), "iPhone")
            .await
            .unwrap_err(),
        "path not found or out of root"
    );
    assert_eq!(
        core.send_to_device(workspace.path().to_str().unwrap(), "iPhone").await.unwrap_err(),
        "only files can be beamed"
    );
    assert_eq!(spool(&core).await, 0, "the refusal above queued nothing");

    // A device that answers and has not granted control. This one is the
    // reason the pre-check exists at all: the device is REACHABLE, so the
    // refusal comes from its own handshake, and it must not be softened into
    // a pending delivery either.
    let err = core.send_to_device(path, "iPhone").await.unwrap_err();
    assert!(err.contains("has not granted this server control"), "{err}");
    assert_eq!(spool(&core).await, 0, "the refusal above queued nothing");
    assert!(
        !Dirs::new(mcp_dir.path()).outbox().exists(),
        "not one of those refusals may leave a copy of the file behind"
    );

    host.node.router.shutdown().await.ok();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_claimed_store_refuses_the_send_instead_of_promising_it() {
    // Two Claude Code sessions over one state directory. The second one may
    // not open the blob store, so it can neither copy a file into it nor
    // replay one out of it — and a queue it will never move is the worst
    // possible thing for it to report.
    let mcp_dir = tempfile::TempDir::new().unwrap();
    let workspace = tempfile::TempDir::new().unwrap();
    let artifact = workspace.path().join("report.html");
    std::fs::write(&artifact, "<h1>report</h1>").unwrap();

    // Stand in for the other process without any `vlerv-remote` type: the
    // claim is an ordinary exclusive file lock, and that IS the contract.
    let lock_path = mcp_dir.path().join("remote").join("blobs.lock");
    std::fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
    let holder = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .unwrap();
    holder.try_lock().expect("the test must be the holder");

    let core = McpCore::new(
        mcp_dir.path().to_path_buf(),
        vec![workspace.path().to_path_buf()],
        workspace.path().to_path_buf(),
        None,
    );
    core.peer_store().seed(&"ab".repeat(32), "Val's iPhone", Scope::Control).unwrap();

    let err = core.send_to_device(artifact.to_str().unwrap(), "iPhone").await.unwrap_err();
    assert!(err.contains("already using the blob store"), "{err}");
    assert_eq!(
        core.server_status().await.unwrap().queued_total,
        0,
        "a send that could not be staged must not be reported as accepted"
    );
    assert!(!Dirs::new(mcp_dir.path()).outbox().exists(), "nothing was staged, so nothing wrote");

    let status = core.server_status().await.unwrap();
    assert!(!status.draining, "this process moves nothing");
    assert!(
        status.queue_blocked_reason.as_deref().unwrap_or_default().contains("already using"),
        "the queue must name the claim rather than answer blandly: {:?}",
        status.queue_blocked_reason
    );
}

/// How many sends are waiting in the spool. Asserted after every refusal
/// below, because "it was refused" and "it was refused and forgotten" are
/// different outcomes and only one of them is right for a caller error.
async fn spool(core: &McpCore) -> usize {
    core.server_status().await.unwrap().queued_total
}

/// Poll a condition for up to five seconds. The eviction runs on the session's
/// own reader task, so a test cannot observe it synchronously.
async fn wait_until<F, Fut>(mut cond: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..100 {
        if cond().await {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    false
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
    assert!(core.list_devices(false).await.unwrap().is_empty());
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
    core.peer_store().seed(&host.node_id(), "Mac Studio", Scope::ViewOpen).unwrap();
    host.peers.seed(&core.node_id().unwrap(), core.device(), Scope::Control).unwrap();
    let quiet = core.list_devices(false).await.unwrap();
    assert_eq!(quiet.len(), 1);
    assert_eq!(quiet[0].device, "Mac Studio");
    assert_eq!(quiet[0].scope, "view-open", "the scope shown is the one granted HERE");
    assert_eq!(quiet[0].presence, "unknown", "nothing dialed it yet");
    // 10 characters — iroh's own `fmt_short` width, so a short id in a log
    // line and one in this list are the same string.
    assert_eq!(quiet[0].node_id_short, host.node_id()[..10]);

    let probed = core.list_devices(true).await.unwrap();
    assert_eq!(probed[0].presence, "online", "the probe reached the host");
    // The session is now cached, so presence is live without a second dial.
    assert_eq!(core.list_devices(false).await.unwrap()[0].presence, "online");

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
    core.peer_store().seed(&"ab".repeat(32), "Mac Studio", Scope::Control).unwrap();
    let err = core.send_to_device(artifact.to_str().unwrap(), "iPhone").await.unwrap_err();
    assert!(err.contains("Mac Studio (ababababab)"), "{err}");

    // An empty device name is an argument error, not a lookup failure.
    assert!(core
        .send_to_device(artifact.to_str().unwrap(), "  ")
        .await
        .unwrap_err()
        .contains("list_devices"));

    // None of the above needed the endpoint.
    assert!(!core.server_status().await.unwrap().booted);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_claimed_store_is_reported_as_the_reason_not_as_an_empty_answer() {
    // The point of the claim is that a second server SAYS why it can do
    // nothing. Answering fast and blandly — "no links to revoke", "every
    // device offline" — is the original silent failure wearing a new face.
    let mcp_dir = tempfile::TempDir::new().unwrap();
    let workspace = tempfile::TempDir::new().unwrap();
    let artifact = workspace.path().join("a.html");
    std::fs::write(&artifact, "<p>a</p>").unwrap();

    // Stand in for the other process without any `vlerv-remote` type: the
    // claim is an ordinary exclusive file lock, and that IS the contract.
    let lock_path = mcp_dir.path().join("remote").join("blobs.lock");
    std::fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
    let holder = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .unwrap();
    holder.try_lock().expect("the test must be the holder");

    let core = McpCore::new(
        mcp_dir.path().to_path_buf(),
        vec![workspace.path().to_path_buf()],
        workspace.path().to_path_buf(),
        None,
    );

    let err = core.beam_artifact(artifact.to_str().unwrap(), None).await.unwrap_err();
    assert!(err.contains("already using the blob store"), "{err}");
    assert!(err.contains("its own state directory"), "{err}");

    // Status names the refusal instead of reading like a healthy idle server.
    let status = core.server_status().await.unwrap();
    assert!(!status.booted);
    assert!(
        status.boot_error.as_deref().unwrap_or_default().contains("already using"),
        "server_status must say WHY it has no node: {:?}",
        status.boot_error
    );

    // A revocation tool that could not look must not answer "nothing to
    // revoke": the caller's link may be live in the process holding the store.
    let err = core.stop_beam(None).await.unwrap_err();
    assert!(err.contains("cannot check beam links"), "{err}");

    // A probe reaches nothing either, and has to say so rather than blame
    // every paired device for being offline.
    core.peer_store().seed(&"ab".repeat(32), "Mac Studio", Scope::Control).unwrap();
    let err = core.list_devices(true).await.unwrap_err();
    assert!(err.contains("already using the blob store"), "{err}");
    // Without a probe nothing needs the network, so the list still answers.
    assert_eq!(core.list_devices(false).await.unwrap()[0].device, "Mac Studio");

    // Releasing hands the store over — the refusal was never cached.
    drop(holder);
    core.beam_artifact(artifact.to_str().unwrap(), None)
        .await
        .expect("a released store must let the next process boot");
    assert!(core.server_status().await.unwrap().booted);
}
