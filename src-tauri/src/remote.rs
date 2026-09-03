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
// until the user invokes a remote action — or, at launch, when the peer store
// is non-empty AND `preferences.remote_listen` is on. Both platforms read
// that one rule (see `listens_at_launch`). The phone is the receiving end,
// and a phone that opens no socket cannot be handed a file another machine
// already accepted for it, so its first paired launch turns the preference
// ON once (see `adopts_listen_pref`) — the switch still exists, and a user
// who turns it off is obeyed. `RemoteState.node` starts empty and is
// populated on the first action that truly needs it; listing peers,
// publishing tabs and revoking a peer never boot it.

pub use vlerv_remote::{beam, endpoint, peers, proto, scope};

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use futures_util::future::join_all;
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
    /// One dial gate per peer id, so `session()` can serialize the dials for
    /// ONE peer without serializing the dials for all of them. Entries are
    /// never removed: it is one empty mutex per device this process has
    /// dialed, and a device that is unpaired and paired again comes back
    /// under the same node id.
    dials: tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Which round of sessions this app is on. Bumped when it abandons every
    /// session it holds — the iOS foreground hop — and read by each session's
    /// `on_closed` before that callback reports presence. Dropping a session
    /// does not wake its reader task: the task sits on a connection nobody
    /// uses until the peer's idle timer kills it, up to half a minute later,
    /// and by then a live replacement has been dialed for the same peer. The
    /// generation is what tells the late callback that it speaks for a
    /// session the app no longer has.
    session_generation: AtomicU64,
    /// Whether a foreground fan-out is running, and whether a resume arrived
    /// while it did. See `ResumeFlight`.
    resume: std::sync::Mutex<ResumeFlight>,
}

/// The in-flight state of the foreground fan-out: is a round running, and did
/// another resume arrive while it ran?
///
/// iOS raises `Resumed` on every foreground transition, and each round dials
/// every paired peer. A dial to a device that is asleep costs the whole
/// connect timeout, so a few hops in a minute stack a few dials on one peer's
/// gate — and `tokio::sync::Mutex` is fair, so the `remote_subscribe` the user
/// starts by opening a drawer waits behind every one of them. Any number of
/// resumes that arrive during one round therefore become exactly ONE more
/// round, which is the in-flight-plus-redrive shape `Drainer::drain_peer`
/// already uses for the send queue.
///
/// Redo rather than drop, because a resume carries information: the sessions
/// this app holds went stale again while the round was dialing, and the round
/// that is running cleared the ones it knew about before that happened.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct ResumeFlight {
    running: bool,
    redo: bool,
}

impl ResumeFlight {
    /// A resume arrived. `true` = this caller owns the fan-out and runs it;
    /// `false` = a round is already running and will run one more.
    fn arrive(&mut self) -> bool {
        if self.running {
            self.redo = true;
            return false;
        }
        self.running = true;
        self.redo = false;
        true
    }

    /// A round finished. `true` = a resume arrived while it ran, so run one
    /// more; `false` = the fan-out is over and the next resume owns it.
    fn finish(&mut self) -> bool {
        if self.redo {
            self.redo = false;
            return true;
        }
        self.running = false;
        false
    }
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
            dials: tokio::sync::Mutex::new(HashMap::new()),
            session_generation: AtomicU64::new(0),
            resume: std::sync::Mutex::new(ResumeFlight::default()),
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

    /// This peer's dial gate, created on first use. The map lock is taken and
    /// released without an await in between, so a peer whose gate is held for
    /// a whole thirty-second dial blocks nobody but the next dial to itself.
    async fn dial_gate(&self, peer: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.dials
            .lock()
            .await
            .entry(peer.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    /// The session this app holds for `peer`, read only once any dial in
    /// flight for that peer has landed.
    ///
    /// The gate is what makes this different from reading the map, and it is
    /// what a caller that acts on a peer's session needs. A dial takes up to
    /// the connect timeout, the drawer that started it can be closed inside
    /// that window, and a plain map read in that window answers `None` for a
    /// peer this app is about to hold a live session with. Waiting here costs
    /// the rest of one dial and gives the caller the session that dial made.
    async fn settled_session(&self, peer: &str) -> Option<Arc<scope::ClientSession>> {
        let gate = self.dial_gate(peer).await;
        let _dialing = gate.lock().await;
        self.sessions.lock().await.get(peer).cloned()
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
    /// iOS brought the app back to the foreground. Every cached session was
    /// dropped immediately before this went out, so the webview must treat
    /// its own subscriptions as gone and re-establish the ones it still
    /// wants — it is the only side that knows which drawers are open.
    Resumed,
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
    // The scope is parsed BEFORE the pairing is taken, the way
    // `McpCore::confirm_pairing` does it: `take` consumes the parked entry,
    // and nothing can re-park it, so a refusal after the take would strand the
    // dialog — every one of its exits calls this command again and would then
    // fail with "no pairing is waiting for confirmation".
    let granted = Scope::parse_optional(scope.as_deref())?;
    let Some(pending) = state.pairing.take(&node_id) else {
        return Err("no pairing is waiting for confirmation".to_string());
    };
    if !accept {
        return Ok(None);
    }
    // A named scope is the human's explicit grant, so it must land on disk
    // even when it NARROWS a device already in the store; naming none leaves
    // an existing grant where it is.
    let peer = match state.peers.confirm(&pending.node_id, &pending.device, granted) {
        Ok(peer) => peer,
        Err(e) => {
            // Put it back. The human is still standing in front of the six
            // words, and a store that could not be written is a reason to
            // retry, not a reason to lose the pairing.
            state.pairing.park(pending);
            return Err(e);
        }
    };
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
    // Behind this peer's dial gate, so an unsubscribe that arrives while the
    // subscribe it undoes is still dialing waits for that dial. The user can
    // close the drawer inside the dial window, and a map read in that window
    // finds nothing, returns Ok, and leaves the landed session subscribed —
    // the host then re-hashes every changed file and pushes `FileChanged` for
    // a drawer nobody is looking at, which is the cost this command exists to
    // prevent.
    match state.settled_session(&peer).await {
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
        HostSignal::PeerConnected { peer, device, scope } => {
            // Logged and nothing else, deliberately. The signal exists for a
            // host that holds a SEND QUEUE — the outbox lives in vlerv-mcp,
            // beside the blob-store claim that makes one process its only
            // drainer — and this app holds none, so a peer becoming reachable
            // gives it nothing to act on. No `vlerv://*` name is added
            // either: that namespace is the frontend's contract, presence in
            // the UI is driven by the sessions this app DIALS, and a second
            // source for it would be a second truth to keep in step. The line
            // stays because "did the phone ever reach this Mac" is the first
            // question a human debugging a missed delivery asks.
            eprintln!(
                "vlerv: scope: {peer} ({device}) connected; this machine grants it {scope:?}"
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
    // Held across the whole dial, so ONE peer is dialed once. The drawer opens
    // several remote commands for one peer in the same tick (subscribe,
    // list-tabs, list-bookmarks, list-recents); without this gate, each misses
    // the map and dials its own ClientSession, and every loser's `on_closed`
    // then reports a live peer offline.
    //
    // The gate is per peer and not per process because a dial to a device that
    // does not answer costs the whole connect timeout. `on_foreground` dials
    // every paired device at once, so a single lock here would make two
    // sleeping phones cost two timeouts one after the other — and hold every
    // UI-initiated command behind them for the same minute.
    let gate = state.dial_gate(peer).await;
    let _dialing = gate.lock().await;

    // Re-read now that this call owns the gate: the dial it queued behind may
    // have landed the very session it was about to make.
    //
    // The generation is read under the same lock the foreground hop clears the
    // map with, so a dial already in flight when the app resumes cannot land a
    // pre-resume session in the fresh generation and keep reporting presence
    // for it.
    let generation = {
        let sessions = state.sessions.lock().await;
        if let Some(existing) = sessions.get(peer) {
            if !existing.is_closed() {
                return Ok(existing.clone());
            }
        }
        state.session_generation.load(Ordering::Acquire)
    };

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
            // The generation this callback speaks for is fixed at dial time;
            // the one it is compared against is read now, because a resume
            // may have happened at any point in between.
            //
            // A session the foreground hop dropped keeps its reader task,
            // because dropping the session never wakes it: the task sits on a
            // connection nobody uses until the peer's idle timer kills it, up
            // to half a minute after the resume. Its `on_closed` then fires
            // for a session the app abandoned, and the replacement dial has
            // already reported the same peer online — so this emit would show
            // a working device as offline, with an empty tab list. Equality,
            // not "at least as new": every generation but the current one is
            // a round this app has finished with.
            let current = closed_app
                .state::<RemoteState>()
                .session_generation
                .load(Ordering::Acquire);
            if generation != current {
                return;
            }
            presence(&closed_app, &closed_peer, "offline", None, None, None);
        },
    )
    .await
    // The app is a viewer, not a sender: it has nothing to retry later, so
    // both dial failures collapse to the one sentence the drawer already
    // shows. `Display` is the inner string, so the header wording is
    // unchanged.
    .map_err(|e| e.to_string())
    .inspect_err(|e| presence(app, peer, "offline", None, None, Some(e.clone())))?;

    presence(
        app,
        peer,
        "online",
        Some(session.device.clone()),
        Some(session.scope.clone()),
        None,
    );
    // The map is for REUSE, and two things can have made this session
    // unreusable while the dial was in flight — both of them impossible back
    // when the dial held this lock from end to end.
    //
    // A resume empties the map and bumps the generation under this same lock,
    // so a session dialed in the round before it is one the app has finished
    // with; its `on_closed` is already silenced by the generation it captured,
    // and caching it would leave the drawer reporting a dead peer as online.
    // `remote_unpair` deletes the peer and then drops its entry under this
    // lock, so a peer revoked mid-dial must not get a cached session back.
    // The caller still receives the session it asked for in both cases.
    {
        let mut sessions = state.sessions.lock().await;
        let same_round = state.session_generation.load(Ordering::Acquire) == generation;
        if same_round && state.peers.get(peer).is_some() {
            sessions.insert(peer.to_string(), session.clone());
        }
    }
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

/// Does launch open a listening socket? Pure, because it decides whether an
/// install binds a socket before the user has asked for anything, and both
/// wrong answers cost something: an undeliverable send, or a socket nobody
/// wanted.
///
/// Both inputs are read the same way on both platforms. `paired` gates it, so
/// a fresh install with no peers binds nothing at all and the zero-sockets
/// promise survives everywhere. `listen_pref` is the user's own switch, and
/// it is OBEYED on the phone exactly as on the Mac — the phone is made to
/// listen by `adopts_listen_pref` turning that switch on once, not by this
/// predicate ignoring it. A platform test here would make an off switch a
/// lie: `state.json` would say the app listens to nobody while it binds a
/// socket at every launch, and the Settings row would decide nothing.
fn listens_at_launch(paired: bool, listen_pref: bool) -> bool {
    paired && listen_pref
}

/// Is this build the machine that exists to be reached? PRODUCT.md makes the
/// phone a read-only companion: it browses another machine's files and it is
/// where artifacts land, so it is the end that must be listening.
const RECEIVER_BUILD: bool = cfg!(target_os = "ios");

/// Does this launch have to turn `preferences.remote_listen` on for good?
///
/// The one-way migration behind `listens_at_launch` reading the preference on
/// both platforms. `remote_listen` defaults to false (state_store.rs), and a
/// phone that listens to nobody cannot be handed a file another machine
/// already accepted and spooled for it — the send succeeds, waits, and
/// expires undelivered. So the first launch of a receiver build that trusts
/// anybody flips the switch ON and records that it did.
///
/// `adopted` is what makes it one-way, and it is the whole point: a user who
/// then turns the switch back off stays off, because every later launch reads
/// the marker and leaves the preference alone. `paired` keeps a fresh,
/// unpaired install untouched — it binds nothing and it is asked nothing.
fn adopts_listen_pref(receiver: bool, paired: bool, adopted: bool) -> bool {
    receiver && paired && !adopted
}

/// The switch the launch decision reads, and the marker that stops the
/// adoption happening a second time. Named here because the ORDER they are
/// written in is the whole rule (`adopt_listen_pref_with`).
const LISTEN_KEY: &str = "preferences.remote_listen";
const ADOPTED_KEY: &str = "preferences.remote_listen_adopted";

/// Persist the adoption.
///
/// `set_state_field` moves the in-memory document at once and debounces the
/// disk write, like every other settings write, so the Settings row this
/// launch renders already shows the switch on.
fn adopt_listen_pref() {
    adopt_listen_pref_with(|key| {
        crate::state_store::set_state_field(key, serde_json::Value::Bool(true))
    });
}

/// The switch first, then the marker, and the marker only if the switch
/// landed. A marker that lands alone is read by every later launch and is
/// one-way, so it would leave the phone not listening with nothing left that
/// would ever turn it on again — and every send another machine accepts for
/// it then waits in that machine's queue until it expires.
///
/// `write` is a parameter so a test can fail the first call and watch the
/// second one never happen; `adopt_listen_pref` passes the state store.
fn adopt_listen_pref_with(mut write: impl FnMut(&str) -> Result<(), String>) {
    if let Err(e) = write(LISTEN_KEY) {
        eprintln!(
            "vlerv: remote: cannot turn listening on for this device's peers: {e} — the \
             next launch tries again"
        );
        return;
    }
    if let Err(e) = write(ADOPTED_KEY) {
        eprintln!(
            "vlerv: remote: this device listens for its peers, but recording that failed: \
             {e} — the next launch turns the switch on once more"
        );
    }
}

/// The launch-time half of the lazy-boot rule (design §4): boot the endpoint
/// at startup only when `listens_at_launch` says so. Every other path stays
/// zero-sockets until an action needs one.
pub fn listen_at_launch(app: &tauri::AppHandle) {
    // Both inputs are read here, before anything is spawned, so an install
    // that will not listen costs one map lookup instead of a tokio task.
    let paired = !app.state::<RemoteState>().peers.is_empty();
    let prefs = crate::state_store::current_state().preferences;
    // Adopted BEFORE the preference is read for the decision below, so the
    // phone's first paired launch already listens instead of waiting for the
    // one after it.
    let listen = if adopts_listen_pref(RECEIVER_BUILD, paired, prefs.remote_listen_adopted) {
        adopt_listen_pref();
        true
    } else {
        prefs.remote_listen
    };
    if !listens_at_launch(paired, listen) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<RemoteState>();
        if let Err(e) = boot_node(&app, &state).await {
            eprintln!("vlerv: remote: cannot start listening: {e}");
        }
    });
}

/// iOS brought the app back to the foreground. Everything here is recovery
/// from a suspension the process itself never saw: iOS freezes the app, the
/// peer's idle timer tears the QUIC connections down, and NOTHING in this
/// process learns of it — `ClientSession::is_closed` reads a cached
/// `AtomicBool` that the reader task can only set once it is running again.
/// So `session()` keeps handing out sessions whose next request answers "the
/// session is closed", which is the failure a sender then has to guess at.
///
/// Called from the `RunEvent::WindowEvent { event: Resumed }` arm in app.rs,
/// which is mobile-only. macOS never suspends the process, so there is no
/// stale session to drop there and nothing calls this.
pub fn on_foreground(app: &tauri::AppHandle) {
    let state = app.state::<RemoteState>();
    // No peers ⇒ nobody may dial this app and it has nobody to dial, so
    // resuming is not a reason to bind a socket.
    if state.peers.is_empty() {
        return;
    }
    // Answered before anything is spawned: a resume that arrives while a
    // round is running adds a redo flag, never a second fan-out. See
    // `ResumeFlight`.
    if !state.resume.lock().unwrap_or_else(|p| p.into_inner()).arrive() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<RemoteState>();
        loop {
            resume_round(&app, &state).await;
            if !state.resume.lock().unwrap_or_else(|p| p.into_inner()).finish() {
                return;
            }
        }
    });
}

/// One foreground round: drop every session this app holds, tell the webview,
/// then re-dial every paired peer at once. Split out from `on_foreground` so
/// the in-flight guard has exactly one body to repeat.
async fn resume_round(app: &tauri::AppHandle, state: &tauri::State<'_, RemoteState>) {
    // Dropped FIRST, before the event and before any re-dial: `session()`
    // returns a cached session whenever `is_closed()` reads false, so one
    // left in the map is handed to the reconnect this very event triggers,
    // and the reconnect then rides the dead connection.
    {
        let mut sessions = state.sessions.lock().await;
        sessions.clear();
        // Under the same lock, so the bump and the clear are one act to
        // every dial. What it silences is explained on the field.
        //
        // ADD, never assign: a counter that came back around would let a
        // session dropped two resumes ago pass the equality check in
        // `on_closed` and report a peer this app is actively talking to
        // as offline. Two suspensions in a row is the ordinary iOS case.
        state.session_generation.fetch_add(1, Ordering::Release);
    }
    let _ = app.emit("vlerv://remote-event", RemoteEvent::Resumed);
    // Re-dialing every paired peer is what makes the phone reachable
    // again: the dial boots the endpoint if the launch did not, and
    // `session()` emits the presence transitions itself, so the drawer
    // header reports the truth for devices whose `on_closed` never fired
    // because the reader task was frozen with the rest of the app.
    //
    // TOGETHER, not one after another. A dial to a device that is itself
    // asleep costs the whole connect timeout, so dialed in turn, four
    // paired devices with two asleep make one foreground hop take about a
    // minute — and every `remote_subscribe` the user starts in that
    // minute waits behind the same dials. Foregrounding is the most
    // frequent lifecycle event iOS has. `session()` gates per peer, so
    // the fan-out still dials each peer exactly once.
    let peers = state.peers.list();
    join_all(peers.iter().map(|peer| async move {
        if let Err(e) = session(app, state, &peer.node_id).await {
            eprintln!("vlerv: remote: {} is not reachable after resume: {e}", peer.device);
        }
    }))
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_listens_for_exactly_one_of_the_four_states_it_can_be_in() {
        // All four, on either platform: the predicate reads no platform at
        // all. A peer to listen for AND the switch the user left on is the
        // only combination that binds a socket before the user asks for
        // anything.
        assert!(listens_at_launch(true, true));
        // The switch is obeyed. On the phone this is a user who turned
        // listening off AFTER the adoption below ran; overriding them here
        // would make `state.json` disagree with what the app does.
        assert!(!listens_at_launch(true, false));
        // The zero-sockets promise a fresh install makes (design §2). Nothing
        // may dial an app that trusts nobody, so listening buys nothing.
        assert!(!listens_at_launch(false, true));
        assert!(!listens_at_launch(false, false));
    }

    #[test]
    fn the_phone_adopts_the_listen_switch_once_and_never_overrules_a_user_who_turns_it_off() {
        // The migration that lets `listens_at_launch` read the preference on
        // both platforms. Without it the phone keeps the default false, and
        // every send to it is accepted, spooled, and expires undelivered.
        assert!(adopts_listen_pref(true, true, false));
        // Once adopted, never again — this is what makes the Settings row on
        // the phone a control that actually decides something.
        assert!(!adopts_listen_pref(true, true, true));
        // A fresh, unpaired install is asked nothing and left at the default,
        // so it still binds nothing.
        assert!(!adopts_listen_pref(true, false, false));
        // The Mac has always had the switch and has never needed the phone's
        // reason for it: it is the end that reaches out, not the end that is
        // reached.
        assert!(!adopts_listen_pref(false, true, false));
    }

    #[test]
    fn adopting_the_switch_also_records_that_it_happened() {
        // Two writes, and the second is the one that matters: a marker that
        // never lands turns a one-time migration into a launch-time override
        // that re-enables listening every time the user turns it off.
        crate::state_store::ensure_shared_test_state_dir();
        crate::state_store::set_state_field(
            "preferences.remote_listen",
            serde_json::Value::Bool(false),
        )
        .expect("seed the default");
        crate::state_store::set_state_field(
            "preferences.remote_listen_adopted",
            serde_json::Value::Bool(false),
        )
        .expect("seed the default");

        adopt_listen_pref();

        let prefs = crate::state_store::current_state().preferences;
        assert!(prefs.remote_listen, "the phone now listens for its peers");
        assert!(prefs.remote_listen_adopted, "and will not be asked again");
        assert!(!adopts_listen_pref(RECEIVER_BUILD, true, prefs.remote_listen_adopted));
    }

    #[test]
    fn the_resumed_kind_is_the_name_the_webview_switches_on() {
        // `RemoteEvent` is serialized straight onto `vlerv://remote-event`,
        // and the frontend dispatches on `kind`. A rename here is a silent
        // no-op there, not a compile error.
        assert_eq!(
            serde_json::to_value(RemoteEvent::Resumed).expect("serialize"),
            serde_json::json!({ "kind": "resumed" })
        );
    }

    #[test]
    fn a_switch_write_that_fails_never_lets_the_adopted_marker_land() {
        // The marker is one-way: every later launch reads it and leaves the
        // preference alone. Landing it after a failed switch write is the one
        // outcome nothing recovers from — the phone listens to nobody, the
        // adoption never runs again, and every send another machine accepts
        // for it sits in that machine's queue until it expires.
        let mut attempted: Vec<String> = Vec::new();
        adopt_listen_pref_with(|key| {
            attempted.push(key.to_string());
            Err("state.json is read-only".to_string())
        });
        assert_eq!(attempted, vec![LISTEN_KEY], "the marker is not even attempted");

        // And in the order the doc states when both writes take.
        let mut written: Vec<String> = Vec::new();
        adopt_listen_pref_with(|key| {
            written.push(key.to_string());
            Ok(())
        });
        assert_eq!(written, vec![LISTEN_KEY, ADOPTED_KEY], "the switch, then the marker");
    }

    #[test]
    fn a_resume_during_a_fan_out_adds_one_more_round_and_never_a_second_fan_out() {
        let mut flight = ResumeFlight::default();
        // The first resume owns the fan-out and runs it.
        assert!(flight.arrive());
        // Three more foreground hops while that round dials. Each one used to
        // spawn its own fan-out over every paired peer, and an unreachable
        // peer costs a whole connect timeout per dial — stacked on one fair
        // mutex, the user's own subscribe waits behind all of them.
        assert!(!flight.arrive());
        assert!(!flight.arrive());
        assert!(!flight.arrive());
        assert!(flight.finish(), "the running round redoes for the resumes it did not see");
        assert!(!flight.finish(), "and ONE redo answers for all three");
        // The fan-out is over, so the next resume owns a fresh one.
        assert!(flight.arrive());
        assert!(!flight.finish());
    }

    #[tokio::test]
    async fn one_peers_dial_gate_never_blocks_a_dial_to_another_device() {
        crate::state_store::ensure_shared_test_state_dir();
        let state = RemoteState::new(RootSet::empty());

        // One device, one gate — two dials to the same peer must serialize,
        // or each misses the sessions map, makes its own session, and every
        // loser's `on_closed` reports a live peer offline.
        let phone = state.dial_gate("device-a").await;
        let phone_again = state.dial_gate("device-a").await;
        assert!(Arc::ptr_eq(&phone, &phone_again), "the same peer id names the same gate");

        let laptop = state.dial_gate("device-b").await;
        assert!(!Arc::ptr_eq(&phone, &laptop), "another device gets its own gate");

        // The reason the gate is per peer: a dial to a device that is asleep
        // costs the whole connect timeout, and the fan-out after a resume
        // dials every paired device at once. One process-wide lock would make
        // a sleeping phone hold every other dial for that timeout.
        let dialing_phone = phone.lock().await;
        assert!(laptop.try_lock().is_ok(), "the laptop is dialed while the phone is dialing");
        assert!(phone.try_lock().is_err(), "and the phone is still dialed once");
        drop(dialing_phone);
    }

    #[tokio::test]
    async fn an_unsubscribe_arriving_during_a_dial_waits_for_that_dial_instead_of_finding_nothing()
    {
        // `remote_unsubscribe` reads the sessions map, and the session it has
        // to find is inserted at the END of a dial that takes up to the
        // connect timeout. The user closes the drawer inside that window, so
        // a read that does not wait answers "nothing open", returns Ok, and
        // leaves the landed session subscribed — the host then re-hashes
        // every changed file and pushes `FileChanged` for a drawer nobody is
        // looking at.
        crate::state_store::ensure_shared_test_state_dir();
        let state = Arc::new(RemoteState::new(RootSet::empty()));
        let peer = "device-being-dialed";

        // Stands in for the dial in flight: `session()` holds this exact gate
        // from before it reads the map until after it inserts.
        let gate = state.dial_gate(peer).await;
        let dialing = gate.lock().await;

        let unsubscribing = tokio::spawn({
            let state = state.clone();
            async move {
                state.settled_session(peer).await;
            }
        });

        let mut pending = unsubscribing;
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(150), &mut pending)
                .await
                .is_err(),
            "the unsubscribe has to wait for the dial, not read the map behind it"
        );

        // The dial lands, and the caller gets the map the dial left.
        drop(dialing);
        tokio::time::timeout(std::time::Duration::from_secs(5), pending)
            .await
            .expect("the unsubscribe stops waiting as soon as the dial is done")
            .expect("the waiting task did not panic");
    }
}
