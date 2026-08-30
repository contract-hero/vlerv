// Remote subsystem — the Tauri half of Beam (v1) and Scope (v2) of
// remote-control-design.html.
//
// The networked core lives in the `vlerv-remote` crate: identity, endpoint,
// wire protocol, peer store, request gate and the RootSet path gate. THIS
// file is everything that crate deliberately does not know — the
// `#[tauri::command]` layer, the `vlerv://*` event glue, the watcher bridge,
// and the two seams the crate asks a host to fill:
//
//   * `EventSink`   — `host_signal_sink` turns each `HostSignal` into the
//                     webview event it belongs to;
//   * `HostCatalog` — `AppCatalog` reports this app's bookmarks and recents;
//   * base dirs     — `dirs()` names `~/Library/Application Support/Vlerv`,
//                     which the crate itself never mentions.
//
// The crate's modules are re-exported so `remote::beam::…` and
// `remote::peers::…` keep working for the deep-link layer and the tests.
//
// Lazy-boot contract (design §2): the app makes ZERO network connections
// until the user invokes a remote action — or, at launch, only when the peer
// store is non-empty AND `preferences.remote_listen` is on. `RemoteState.node`
// starts empty and is populated on the first action that truly needs it;
// listing peers, publishing tabs and revoking a peer never boot it.

pub use vlerv_remote::{beam, endpoint, peers, proto, scope};

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::{Emitter, Manager};

use crate::security::RootSet;
use peers::{Peer, PendingPair, Scope};
use proto::{ArtifactMeta, PathEntry, TabEntry, TreeEntry};
// Re-exported for the app's own consumers (deep-link layer, tests) and for
// symmetry with the module re-exports above.
pub use vlerv_remote::{Dirs, EmptyCatalog, HostCatalog, HostSignal};

/// Where this app keeps the remote subsystem's files. The crate hardcodes no
/// directory: every path below derives from the app's own state dir, so a
/// headless consumer of the same crate lands its blobs somewhere else.
pub(crate) fn dirs() -> Dirs {
    Dirs::new(crate::state_store::state_dir())
}

/// The app's answer to `HostCatalog`: what a remote peer may be told is
/// starred and recently opened here. Every entry still passes the RootSet
/// gate inside the crate before it reaches the wire — this only ever narrows.
#[derive(Debug, Clone, Copy, Default)]
pub struct AppCatalog;

impl HostCatalog for AppCatalog {
    fn bookmarks(&self) -> Vec<PathBuf> {
        crate::bookmarks::list().into_iter().map(|b| b.path).collect()
    }

    fn recents(&self) -> Vec<PathBuf> {
        crate::recents::list().into_iter().map(|r| r.path).collect()
    }
}

/// Managed Tauri state: the lazily booted node, plus the host-side state that
/// exists with or without an endpoint (peers, published tabs, pending
/// pairings) and the client-side sessions this instance holds.
pub struct RemoteState {
    node: tokio::sync::Mutex<Option<Arc<endpoint::RemoteNode>>>,
    /// `node.is_some()`, readable without the async mutex. The watcher bridge
    /// asks this on its own thread for every filesystem event, and an app that
    /// never booted an endpoint must not spawn a task per event to find that
    /// out. Only ever goes false → true, under the node mutex.
    booted: AtomicBool,
    peers: Arc<peers::PeerStore>,
    pairing: Arc<peers::Pairing>,
    tabs: Arc<scope::TabsCache>,
    roots: RootSet,
    device: String,
    sessions: tokio::sync::Mutex<HashMap<String, Arc<scope::ClientSession>>>,
}

impl RemoteState {
    pub fn new(roots: RootSet) -> Self {
        Self {
            node: tokio::sync::Mutex::new(None),
            booted: AtomicBool::new(false),
            peers: Arc::new(peers::PeerStore::load(&dirs().remote())),
            pairing: Arc::new(peers::Pairing::new()),
            tabs: Arc::new(scope::TabsCache::new()),
            roots,
            device: vlerv_remote::device_name(),
            sessions: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    /// Get the booted node, booting it on first use. `scope_state` is a
    /// FACTORY, not a value: every call after the first returns the node
    /// already in hand, and building a `ScopeState` to drop it again is what
    /// each of those calls used to do. The `on_offers_change` callback is
    /// installed once, at boot, and fires with the fresh offers list whenever
    /// the request gate mutates the registry (fetch counts).
    async fn node(
        &self,
        scope_state: impl FnOnce() -> Arc<scope::ScopeState> + Send,
        on_offers_change: impl Fn(Vec<beam::OfferInfo>) + Send + Sync + 'static,
    ) -> Result<Arc<endpoint::RemoteNode>, String> {
        let mut guard = self.node.lock().await;
        if let Some(node) = guard.as_ref() {
            return Ok(node.clone());
        }
        let node = Arc::new(endpoint::boot(&dirs(), Some(scope_state()), on_offers_change).await?);
        *guard = Some(node.clone());
        // Published while the mutex is still held, so nothing can observe the
        // flag before the node it advertises is in place.
        self.booted.store(true, Ordering::Release);
        Ok(node)
    }

    /// Peek the node without booting — for commands where "no node yet"
    /// means "nothing to do" (listing, revoking). Booting sockets to answer
    /// a guaranteed no-op would break the lazy-boot contract.
    async fn existing(&self) -> Option<Arc<endpoint::RemoteNode>> {
        self.node.lock().await.clone()
    }

    /// Has an endpoint ever booted? The lock-free half of `existing`, for
    /// callers that only need "is there anything at all to talk to".
    fn is_booted(&self) -> bool {
        self.booted.load(Ordering::Acquire)
    }
}

fn offers_changed(app: &tauri::AppHandle, node: &endpoint::RemoteNode) {
    let _ = app.emit("vlerv://beam-offers-updated", node.offers.list());
}

// ═══ Beam (v1) ══════════════════════════════════════════════════════════════

/// Stage a file into the blob store, mint a ticket, and register the offer.
/// Path policy lives in `beam::resolve_offerable`, shared with the
/// `vlerv://beam` dispatch arm: conservative share gate, files only, hard
/// cap — rechecked here at confirm time.
#[tauri::command]
pub async fn beam_offer(
    app: tauri::AppHandle,
    state: tauri::State<'_, RemoteState>,
    roots: tauri::State<'_, RootSet>,
    path: String,
) -> Result<beam::OfferInfo, String> {
    let cand = beam::resolve_offerable(std::path::Path::new(&path), &roots)?;
    let ttl_hours = crate::state_store::current_state()
        .preferences
        .beam_ttl_hours
        .unwrap_or(beam::DEFAULT_TTL_HOURS);

    let node = boot_node(&app, &state).await?;
    let info = beam::offer(&node, &cand, ttl_hours).await?;
    offers_changed(&app, &node);
    Ok(info)
}

/// Revoke an active offer. The ticket dies with the offer: the request gate
/// consults the registry per request, so the next fetch is denied even if
/// the blob bytes are still in the store. Never boots — with no node there
/// is nothing to revoke.
#[tauri::command]
pub async fn beam_stop(
    app: tauri::AppHandle,
    state: tauri::State<'_, RemoteState>,
    offer_id: String,
) -> Result<(), String> {
    let Some(node) = state.existing().await else {
        return Ok(());
    };
    beam::stop(&node, &offer_id).await;
    offers_changed(&app, &node);
    Ok(())
}

/// Active (unexpired) offers for the "beaming" indicator. Never boots.
#[tauri::command]
pub async fn beam_list_offers(
    state: tauri::State<'_, RemoteState>,
) -> Result<Vec<beam::OfferInfo>, String> {
    Ok(state.existing().await.map(|n| n.offers.list()).unwrap_or_default())
}

/// Post-confirm fetch: dial the ticket, stream the BLAKE3-verified blob, and
/// land it under `received/<date>/`. Progress goes out as
/// `vlerv://beam-progress` events keyed by the ticket's hash.
#[tauri::command]
pub async fn beam_receive(
    app: tauri::AppHandle,
    state: tauri::State<'_, RemoteState>,
    ticket: String,
    name: Option<String>,
) -> Result<beam::ReceivedFile, String> {
    let node = boot_node(&app, &state).await?;
    let progress_app = app.clone();
    beam::receive(
        &node,
        &ticket,
        name.as_deref(),
        &dirs().received(),
        move |hash_hex, received| {
            let _ = progress_app.emit(
                "vlerv://beam-progress",
                beam::ProgressEvent { hash: hash_hex.to_string(), received },
            );
        },
    )
    .await
}

/// Where received artifacts land — the frontend uses this prefix to swap the
/// "external" badge for a "beamed" one.
#[tauri::command]
pub fn beam_received_dir() -> String {
    dirs().received().to_string_lossy().into_owned()
}

/// Past beams, newest first, for the "Received" list.
#[tauri::command]
pub fn beam_list_received() -> Vec<beam::ReceivedEntry> {
    beam::list_received(&dirs().received())
}

// ═══ Scope (v2) ═════════════════════════════════════════════════════════════

/// What `vlerv://remote-presence` carries. One payload for all three states,
/// so the drawer header renders from a single subscription.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PresenceEvent {
    pub peer: String,
    /// "connecting" | "online" | "offline".
    pub state: &'static str,
    /// The host's announced device name, once the handshake produced one.
    pub device: Option<String>,
    /// The scope the host granted us, once the handshake produced one.
    pub scope: Option<String>,
    /// Why the session ended or failed to start. Absent on success.
    pub reason: Option<String>,
}

/// What `vlerv://remote-event` carries: the re-emitted session events plus
/// the pairing prompt. One event name, discriminated by `kind`, so the
/// frozen `vlerv://*` namespace grows by exactly the two names the design
/// lists.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum RemoteEvent {
    TabOpened { peer: String, path: String },
    TabClosed { peer: String, path: String },
    TabActivated { peer: String, path: String },
    FileChanged { peer: String, path: String, hash: String },
    FileRemoved { peer: String, path: String },
    /// A pairing reached the fingerprint step on THIS machine. The UI shows
    /// the six words and calls `remote_pair_confirm`.
    PairPending {
        peer: String,
        device: String,
        fingerprint: Vec<String>,
        /// "host" (this machine minted the ticket) or "guest".
        role: String,
    },
    /// A `vlerv://pair?ticket=…` deep link arrived. The UI shows the inviting
    /// device and calls `remote_pair_complete(ticket)` when the user proceeds
    /// — the link alone never dials anything.
    PairLink {
        peer: String,
        peer_short: String,
        device: String,
        ticket: String,
    },
    /// The peer list changed (paired, unpaired, scope edited).
    PeersUpdated,
}

impl RemoteEvent {
    fn from_session(peer: &str, event: proto::Event) -> Self {
        let peer = peer.to_string();
        match event {
            proto::Event::TabOpened { path } => RemoteEvent::TabOpened { peer, path },
            proto::Event::TabClosed { path } => RemoteEvent::TabClosed { peer, path },
            proto::Event::TabActivated { path } => RemoteEvent::TabActivated { peer, path },
            proto::Event::FileChanged { path, hash } => {
                RemoteEvent::FileChanged { peer, path, hash }
            }
            proto::Event::FileRemoved { path } => RemoteEvent::FileRemoved { peer, path },
        }
    }
}

/// A verified artifact fetched from a peer, in the local content-addressed
/// cache. `path` is what the render pipeline opens.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RemoteArtifact {
    pub peer: String,
    /// The path ON THE HOST — the identity the drawer and the events use.
    pub remote_path: String,
    /// The local cache file: `remote/cache/<hash><ext>`.
    pub path: String,
    pub hash: String,
    pub size: u64,
    pub mtime: u64,
    pub warn: bool,
}

/// Trusted peers, newest pairing first. Never boots the endpoint.
#[tauri::command]
pub fn remote_list_peers(state: tauri::State<'_, RemoteState>) -> Vec<Peer> {
    state.peers.list()
}

/// Mint a one-time pairing token and the `vlerv://pair?ticket=…` link that
/// carries it. Boots the endpoint: the ticket must contain reachable
/// addresses, which only a bound endpoint knows.
///
/// What the Settings pane gets back is `peers::PairInvite` — the link to carry
/// to the other machine plus this instance's own identity to show beside it.
/// The crate mints it, so the ticket cannot be described here with a TTL, a
/// link shape or a NodeId that differs from the one handed out.
#[tauri::command]
pub async fn remote_pair_begin(
    app: tauri::AppHandle,
    state: tauri::State<'_, RemoteState>,
) -> Result<peers::PairInvite, String> {
    let node = boot_node(&app, &state).await?;
    // Bounded wait for relay + discovery so the ticket dials from another
    // network; on timeout it still carries direct addresses (same policy as
    // minting a beam ticket). The wait is the caller's, not the minter's:
    // it is endpoint lifecycle, not pairing policy.
    let _ = tokio::time::timeout(std::time::Duration::from_secs(10), node.endpoint.online()).await;
    Ok(peers::mint_invite(node.endpoint.addr(), &state.pairing, &state.device))
}

/// Open a pairing ticket: dial the host's pairing ALPN, present the token,
/// and park the pairing at the fingerprint step. NOTHING is persisted here —
/// `remote_pair_confirm` is the step the human authorizes.
#[tauri::command]
pub async fn remote_pair_complete(
    app: tauri::AppHandle,
    state: tauri::State<'_, RemoteState>,
    ticket: String,
) -> Result<PendingPair, String> {
    let ticket: peers::PairTicket = ticket.parse()?;
    let node = boot_node(&app, &state).await?;
    let pending = scope::pair_dial(&node, &ticket, state.device.clone()).await?;
    park_and_announce(&app, pending.clone());
    Ok(pending)
}

/// Resolve a parked pairing after the human compared the six words. `accept:
/// false` discards it — the peer is never written to disk, so a mismatched
/// fingerprint leaves no trace to clean up. Returns the persisted peer on
/// acceptance.
#[tauri::command]
pub async fn remote_pair_confirm(
    app: tauri::AppHandle,
    state: tauri::State<'_, RemoteState>,
    node_id: String,
    accept: bool,
    scope: Option<String>,
) -> Result<Option<Peer>, String> {
    let Some(pending) = state.pairing.take(&node_id) else {
        return Err("no pairing is waiting for confirmation".to_string());
    };
    if !accept {
        return Ok(None);
    }
    // A named scope is the human's explicit grant, so it must land on disk
    // even when it NARROWS a device already in the store; naming none leaves
    // an existing grant where it is. `confirm` expresses that difference —
    // `upsert`, the passive handshake path, never moves a grant at all.
    let granted = Scope::parse_optional(scope.as_deref())?;
    let peer = state.peers.confirm(&pending.node_id, &pending.device, granted)?;
    let _ = app.emit("vlerv://remote-event", RemoteEvent::PeersUpdated);
    Ok(Some(peer))
}

/// Revoke a peer: delete the entry, drop its open sessions, and un-grant the
/// artifacts staged for it. The scope server rejects it at the next
/// handshake, so revocation is immediate. Never boots.
#[tauri::command]
pub async fn remote_unpair(
    app: tauri::AppHandle,
    state: tauri::State<'_, RemoteState>,
    node_id: String,
) -> Result<(), String> {
    state.peers.remove(&node_id)?;
    state.sessions.lock().await.remove(&node_id);
    if let Some(node) = state.existing().await {
        if let Some(server) = &node.scope {
            server.revoke(&node_id).await;
        }
    }
    let _ = app.emit("vlerv://remote-event", RemoteEvent::PeersUpdated);
    Ok(())
}

/// Change what a peer may do. Takes effect on its next request — the server
/// re-reads the peer per request, not per connection. Never boots.
#[tauri::command]
pub fn remote_set_scope(
    app: tauri::AppHandle,
    state: tauri::State<'_, RemoteState>,
    node_id: String,
    scope: String,
) -> Result<(), String> {
    state.peers.set_scope(&node_id, Scope::parse(&scope)?)?;
    let _ = app.emit("vlerv://remote-event", RemoteEvent::PeersUpdated);
    Ok(())
}

/// The host bridge from the tabs reducer (design §6). Called on every tabs
/// commit; the backend caches the list for `ListTabs` and diffs it into
/// TabOpened / TabClosed / TabActivated events for subscribed peers. Never
/// boots: with no endpoint there is nobody to notify, and the cache still
/// needs to be current for the moment one connects.
#[tauri::command]
pub async fn remote_publish_tabs(
    state: tauri::State<'_, RemoteState>,
    tabs: Vec<TabEntry>,
) -> Result<(), String> {
    // ONE cache either way: `boot_node` hands the server `state.tabs`, so
    // `server.state.tabs` IS this `state.tabs` and both arms leave the same
    // gated, canonical list behind for the next `ListTabs`. The only thing a
    // live server adds is fanning the derived events out to its subscribers.
    match state.existing().await.and_then(|n| n.scope.clone()) {
        Some(server) => server.state.publish_tabs(tabs),
        None => state.tabs.publish(scope::canonical_tabs(tabs, &state.roots)),
    };
    Ok(())
}

/// The peer's open tabs, live.
#[tauri::command]
pub async fn remote_list_tabs(
    app: tauri::AppHandle,
    state: tauri::State<'_, RemoteState>,
    peer: String,
) -> Result<Vec<TabEntry>, String> {
    session(&app, &state, &peer).await?.list_tabs().await
}

/// The peer's bookmarks.
#[tauri::command]
pub async fn remote_list_bookmarks(
    app: tauri::AppHandle,
    state: tauri::State<'_, RemoteState>,
    peer: String,
) -> Result<Vec<PathEntry>, String> {
    session(&app, &state, &peer).await?.list_bookmarks().await
}

/// The peer's recents.
#[tauri::command]
pub async fn remote_list_recents(
    app: tauri::AppHandle,
    state: tauri::State<'_, RemoteState>,
    peer: String,
) -> Result<Vec<PathEntry>, String> {
    session(&app, &state, &peer).await?.list_recents().await
}

/// One directory level of the peer's workspace. Browse scope and up; a
/// view-open peer gets the host's single refusal string.
#[tauri::command]
pub async fn remote_list_tree(
    app: tauri::AppHandle,
    state: tauri::State<'_, RemoteState>,
    peer: String,
    path: String,
) -> Result<Vec<TreeEntry>, String> {
    session(&app, &state, &peer).await?.list_tree(path).await
}

/// Fetch an artifact from a peer: ask for its content address on the session
/// stream, then pull the bytes over the verified blob protocol into
/// `remote/cache/<hash><ext>`. Returns the LOCAL cache path the render
/// pipeline opens.
#[tauri::command]
pub async fn remote_get(
    app: tauri::AppHandle,
    state: tauri::State<'_, RemoteState>,
    peer: String,
    path: String,
) -> Result<RemoteArtifact, String> {
    let session = session(&app, &state, &peer).await?;
    let meta: ArtifactMeta = session.get_artifact(path.clone()).await?;
    // A live session means the endpoint is already up — `session` booted it
    // (or reused the boot that opened the cached one), and the node is never
    // torn down. So this reads the node instead of asking to boot a second.
    let node = state.existing().await.ok_or("the remote endpoint is gone")?;
    let addr = endpoint::addr_for(&peer)?;
    // The cache filename carries the source extension (design intent: the
    // local reader dispatches raster images by extension — reader.rs never
    // sniffs image formats from bytes) — a bare hash would read back as
    // generic binary and lose image rendering entirely.
    let ext = std::path::Path::new(&path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    let cached =
        scope::fetch_into_cache(&node, addr, &meta.hash, &ext, &dirs().cache()).await?;
    Ok(RemoteArtifact {
        peer,
        remote_path: path,
        path: cached.to_string_lossy().into_owned(),
        hash: meta.hash,
        size: meta.size,
        mtime: meta.mtime,
        warn: meta.warn,
    })
}

/// Drive the peer: open an artifact on ITS screen. Control scope only, and
/// on the host side it can invoke nothing a deep link could not.
#[tauri::command]
pub async fn remote_open_on_host(
    app: tauri::AppHandle,
    state: tauri::State<'_, RemoteState>,
    peer: String,
    path: String,
    reader_mode: bool,
) -> Result<(), String> {
    session(&app, &state, &peer).await?.open_on_host(path, reader_mode).await
}

/// Start receiving the peer's events (`vlerv://remote-event`).
#[tauri::command]
pub async fn remote_subscribe(
    app: tauri::AppHandle,
    state: tauri::State<'_, RemoteState>,
    peer: String,
) -> Result<(), String> {
    session(&app, &state, &peer).await?.subscribe().await
}

/// Stop receiving the peer's events. Tells the HOST to stop, so it also stops
/// re-hashing changed files for a drawer nobody is looking at.
#[tauri::command]
pub async fn remote_unsubscribe(
    state: tauri::State<'_, RemoteState>,
    peer: String,
) -> Result<(), String> {
    let existing = state.sessions.lock().await.get(&peer).cloned();
    match existing {
        Some(session) if !session.is_closed() => session.unsubscribe().await,
        // Nothing open ⇒ nothing to unsubscribe. Dialing the peer to say
        // "stop" would boot sockets to achieve nothing.
        _ => Ok(()),
    }
}

// ── Plumbing shared by the commands ────────────────────────────────────────

async fn boot_node(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, RemoteState>,
) -> Result<Arc<endpoint::RemoteNode>, String> {
    let offers_app = app.clone();
    state
        .node(
            // Cold path only: `node` calls this exactly when it is about to
            // boot, so a command that finds the endpoint already up never
            // builds a `ScopeState` it would immediately drop.
            || {
                Arc::new(scope::ScopeState::new(
                    state.peers.clone(),
                    state.pairing.clone(),
                    state.tabs.clone(),
                    state.roots.clone(),
                    state.device.clone(),
                    Arc::new(AppCatalog),
                    host_signal_sink(app.clone()),
                ))
            },
            // The gate hands the fresh offers list straight to the callback —
            // one emit path that never re-locks the RemoteState.node tokio
            // mutex (the gate loop still takes the Offers mutex to admit +
            // list; that's fine).
            move |offers| {
                let _ = offers_app.emit("vlerv://beam-offers-updated", offers);
            },
        )
        .await
}

/// Turn host-side signals into the webview events they belong to. This is the
/// app's `EventSink` — the only place Tauri meets the scope server, and the
/// seam a headless host replaces with its own handling.
fn host_signal_sink(app: tauri::AppHandle) -> impl Fn(HostSignal) + Send + Sync + 'static {
    move |signal| match signal {
        HostSignal::PairPending(pending) => park_and_announce(&app, pending),
        HostSignal::OpenOnHost { peer, path, reader_mode } => {
            // A control peer inherits the deep-link posture exactly: the SAME
            // event the `vlerv://open` verb emits, on an already-gated path.
            eprintln!("vlerv: scope: {peer} opened {} on this machine", path.display());
            let _ = app.emit(
                "vlerv://open-file",
                crate::OpenFileEvent {
                    path,
                    intent: crate::DeepLinkIntentKind::Open,
                    out_of_root: false,
                    line: None,
                    reader_mode: Some(reader_mode),
                },
            );
        }
        HostSignal::ArtifactReceived { peer, path, name, size, hash } => {
            // A control peer's push landed through the SAME verified path an
            // accepted beam takes, in the same `received/` folder — so it is
            // surfaced the same way, and the frontend opens it in a tab with
            // the beamed badge.
            eprintln!("vlerv: scope: {peer} pushed {name} to this machine");
            let _ = app.emit(
                "vlerv://beam-received",
                BeamReceivedEvent { peer: Some(peer), path, name, size, hash },
            );
        }
    }
}

/// Park a pairing at the fingerprint step and tell the UI about it. BOTH faces
/// of pairing land here — the outbound `remote_pair_complete` and the inbound
/// `HostSignal::PairPending` — so the order is the same on either side: park
/// first, because `remote_pair_confirm` resolves against the parked entry, and
/// only then emit the prompt that makes a human call it.
fn park_and_announce(app: &tauri::AppHandle, pending: PendingPair) {
    app.state::<RemoteState>().pairing.park(pending.clone());
    let _ = app.emit(
        "vlerv://remote-event",
        RemoteEvent::PairPending {
            peer: pending.node_id.clone(),
            device: pending.device.clone(),
            fingerprint: pending.fingerprint.clone(),
            role: pending.role.clone(),
        },
    );
    // Debug-only E2E hook: a simulator has no screen to tap "Confirm" on, so
    // both faces honor VLERV_TEST_AUTOPAIR. Absent from release builds — see
    // the banner below.
    #[cfg(debug_assertions)]
    test_autopair(app, &pending);
}

// ═════════════════════════════════════════════════════════════════════════════
// ██  TEST-ONLY AUTOPAIR HOOK — NOT PRESENT IN RELEASE BUILDS  ██
// ═════════════════════════════════════════════════════════════════════════════
//
// WHAT THIS DOES: it answers the six-word fingerprint prompt FOR the human.
// A stranger who dials this instance becomes a trusted peer with no screen
// to compare and nobody to compare it. That is the whole point of the
// pairing step (design §6/§7), so this MUST NOT reach a shipped binary.
//
// WHY IT EXISTS: the iOS simulator E2E runs headless. The simulator instance
// mints a pairing ticket (`remote_pair_begin`), the driver machine dials it,
// and the inbound `HostSignal::PairPending` lands on the simulator — where
// no test driver can tap "Confirm". This hook resolves that one parked
// pairing.
//
// TWO INDEPENDENT GATES, BOTH REQUIRED:
//   1. `#[cfg(debug_assertions)]` — a compile-time gate, so a release build
//      does not contain this function, its call site, or the env-var read.
//      A runtime check alone would ship the code path.
//   2. `VLERV_TEST_AUTOPAIR` — the env var must be present in the process
//      environment. Unset (the default for every `cargo tauri dev` run) and
//      the hook does nothing at all.
//
// ENV CONTRACT — `VLERV_TEST_AUTOPAIR`:
//   unset                          → inert; normal human confirmation.
//   set, empty  (`VLERV_TEST_AUTOPAIR=`)   → grant scope `control`.
//   set to `view-open`|`browse`|`control`  → grant that scope.
//   set to anything else           → refuse and log; the pairing stays
//                                    parked for a human, so a typo cannot
//                                    silently downgrade or widen the grant.
//
// The default is `control` on purpose: the E2E drives push/open-on-host,
// which is the only scope that admits them. `peers::DEFAULT_SCOPE` stays
// `view-open` for real humans and is deliberately not reused here.

/// The env var name, declared ONCE. Both hooks below and the deep-link gate in
/// `app.rs` (through `autopair_enabled`) read this const — three separate
/// copies of the string were three chances for one of them to drift.
#[cfg(debug_assertions)]
const AUTOPAIR_ENV: &str = "VLERV_TEST_AUTOPAIR";

/// Is the hook armed? The deep-link arm in `app.rs` asks before it dials an
/// arriving `vlerv://pair` link with no human in the loop. Same `env::var`
/// read the hook itself performs, so the gate and the hook agree.
#[cfg(debug_assertions)]
pub fn autopair_enabled() -> bool {
    std::env::var(AUTOPAIR_ENV).is_ok()
}

#[cfg(debug_assertions)]
fn test_autopair(app: &tauri::AppHandle, pending: &PendingPair) {
    let Ok(raw) = std::env::var(AUTOPAIR_ENV) else { return };

    let requested = raw.trim();
    let scope = if requested.is_empty() {
        Scope::Control
    } else {
        match Scope::parse(requested) {
            Ok(scope) => scope,
            Err(e) => {
                eprintln!("vlerv: {AUTOPAIR_ENV}: {e}; leaving the pairing for a human");
                return;
            }
        }
    };

    let state = app.state::<RemoteState>();
    // Take the entry the caller just parked. `take` is what
    // `remote_pair_confirm` does, so the two paths cannot both resolve the
    // same pairing.
    let Some(parked) = state.pairing.take(&pending.node_id) else {
        return;
    };
    match state.peers.confirm(&parked.node_id, &parked.device, Some(scope)) {
        Ok(_) => {
            eprintln!(
                "vlerv: {AUTOPAIR_ENV}: AUTO-CONFIRMED {} ({}) as {} — TEST BUILD ONLY",
                parked.device,
                parked.node_id,
                scope.as_str()
            );
            let _ = app.emit("vlerv://remote-event", RemoteEvent::PeersUpdated);
        }
        Err(e) => eprintln!("vlerv: {AUTOPAIR_ENV}: cannot persist the peer: {e}"),
    }
}

// Debug-only E2E hook, third arm: a `vlerv://pair` link arrived and no human
// can tap the confirm UI (simulator). Dial + park + `test_autopair` — the same
// steps `remote_pair_complete` then a confirm tap would take. Only called when
// `autopair_enabled` says so (checked at the deep-link site); absent from
// release builds.
#[cfg(debug_assertions)]
pub fn test_autopair_dial(app: tauri::AppHandle, ticket: String) {
    tauri::async_runtime::spawn(async move {
        let parsed: peers::PairTicket = match ticket.parse() {
            Ok(t) => t,
            Err(e) => return eprintln!("vlerv: {AUTOPAIR_ENV}: bad ticket: {e}"),
        };
        let state = app.state::<RemoteState>();
        let node = match boot_node(&app, &state).await {
            Ok(n) => n,
            Err(e) => return eprintln!("vlerv: {AUTOPAIR_ENV}: boot failed: {e}"),
        };
        let pending = match scope::pair_dial(&node, &parsed, state.device.clone()).await {
            Ok(p) => p,
            Err(e) => return eprintln!("vlerv: {AUTOPAIR_ENV}: dial failed: {e}"),
        };
        state.pairing.park(pending.clone());
        test_autopair(&app, &pending);
    });
}

/// What `vlerv://beam-received` carries: an artifact that landed on this
/// machine without the local user driving a fetch. `peer` names the pushing
/// peer (Scope v2); it is `None` for anything the local user accepted itself.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BeamReceivedEvent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer: Option<String>,
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub hash: String,
}

/// Get a live session with a paired peer, dialing on first use. Presence
/// transitions are emitted here, so every entry point drives the drawer
/// header the same way.
async fn session(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, RemoteState>,
    peer: &str,
) -> Result<Arc<scope::ClientSession>, String> {
    // Only paired peers are dialable — the allowlist is symmetric.
    if state.peers.get(peer).is_none() {
        return Err("unknown peer".to_string());
    }
    // Hold the sessions lock across the whole dial. The drawer opens several
    // remote commands for one peer in the same tick (subscribe, list-tabs,
    // list-bookmarks, list-recents); without this, each misses the map and
    // dials its own ClientSession, and every loser's `on_closed` then reports
    // a live peer offline. A dial is bounded by the connect timeout, and this
    // app pairs a handful of peers, so serializing dials is the right cost.
    let mut sessions = state.sessions.lock().await;
    if let Some(existing) = sessions.get(peer) {
        if !existing.is_closed() {
            return Ok(existing.clone());
        }
    }

    let node = boot_node(app, state).await?;
    let addr = endpoint::addr_for(peer)?;
    presence(app, peer, "connecting", None, None, None);

    let event_app = app.clone();
    let event_peer = peer.to_string();
    let closed_app = app.clone();
    let closed_peer = peer.to_string();
    let session = scope::ClientSession::connect(
        &node,
        addr,
        state.device.clone(),
        move |event| {
            let _ = event_app.emit(
                "vlerv://remote-event",
                RemoteEvent::from_session(&event_peer, event),
            );
        },
        move || {
            presence(&closed_app, &closed_peer, "offline", None, None, None);
        },
    )
    .await
    .inspect_err(|e| presence(app, peer, "offline", None, None, Some(e.clone())))?;

    presence(
        app,
        peer,
        "online",
        Some(session.device.clone()),
        Some(session.scope.clone()),
        None,
    );
    sessions.insert(peer.to_string(), session.clone());
    Ok(session)
}

fn presence(
    app: &tauri::AppHandle,
    peer: &str,
    state: &'static str,
    device: Option<String>,
    scope: Option<String>,
    reason: Option<String>,
) {
    let _ = app.emit(
        "vlerv://remote-presence",
        PresenceEvent { peer: peer.to_string(), state, device, scope, reason },
    );
}

/// Bridge one watcher event to subscribed peers (design §6: the watcher's
/// events fan out as `FileChanged`, re-hashed so the event carries the new
/// content address). Called from the watcher bridge threads in `main.rs`.
/// Does nothing — and boots nothing — when no scope session exists.
pub fn note_file_change(app: &tauri::AppHandle, change: crate::watcher::FileChange) {
    // Answered on the watcher's own thread, before anything is spawned or
    // cloned: an install that never booted an endpoint gets one atomic load
    // per filesystem event instead of a tokio task that takes the node mutex
    // only to find `None`. Editing a file in the workspace fires this
    // hundreds of times.
    if !app.state::<RemoteState>().is_booted() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<RemoteState>();
        let Some(node) = state.existing().await else { return };
        let Some(server) = node.scope.clone() else { return };
        let removed = matches!(change.kind, crate::watcher::TreeChangeKind::Remove);
        server.note_change(&change.path, removed).await;
    });
}

/// The launch-time half of the lazy-boot rule (design §4): boot the endpoint
/// at startup ONLY when this install has peers AND the user turned listening
/// on. Every other path stays zero-sockets until an action needs one.
pub fn listen_at_launch(app: &tauri::AppHandle) {
    let listen = crate::state_store::current_state().preferences.remote_listen;
    if !listen {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<RemoteState>();
        if state.peers.is_empty() {
            return;
        }
        if let Err(e) = boot_node(&app, &state).await {
            eprintln!("vlerv: remote: cannot start listening: {e}");
        }
    });
}
