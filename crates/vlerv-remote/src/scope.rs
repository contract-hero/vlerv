// Scope — the v2 session protocol: server (this machine is the host) and
// client (this machine views another host). Design §6.
//
// Host order of checks, per request, never reordered:
//   1. allowlist — the connection's NodeId must be in peers.json, or the
//      handshake is refused before a single request byte is parsed;
//   2. scope filter — the peer's granted scope must admit the request kind;
//   3. `security::canonicalize_and_check_root` — the same gate local IPC
//      uses, so a remote peer can never see more than the local UI could;
//   4. for a view-open peer, the canonical path must be one it was already
//      told about (open tabs, bookmarks, recents).
//
// Refusals all carry the share module's no-existence-leak wording: a peer
// cannot tell a missing file from a forbidden one.
//
// Metadata rides this stream; artifact BYTES ride the existing iroh-blobs
// protocol, content-addressed and verified there. `GetArtifact` stages the
// file into the same store Beam uses and records a peer-locked grant that the
// blobs request gate consults.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iroh::endpoint::{Connection, RecvStream, SendStream, VarInt};
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh::{Endpoint, EndpointAddr, EndpointId};
use iroh_blobs::api::Tag;
use iroh_blobs::store::fs::FsStore;
use iroh_blobs::{Hash, HashAndFormat};
use tokio::sync::{mpsc, oneshot};

use crate::beam::delete_tags;
use crate::endpoint::{self, RemoteNode};
use crate::paths::{base_name, mtime_secs, Dirs, DEFAULT_IGNORED};
use crate::peers::{self, now_unix, Peer, PeerStore, Pairing, PendingPair, Scope};
use crate::proto::{
    self, ArtifactMeta, Event, Frame, HelloAck, PairAck, PairHello, PathEntry, Req, Res, TabEntry,
    TreeEntry,
};
use crate::security::{self, RootSet};

/// The host seams live in `host.rs` — re-exported here because they are part
/// of this module's contract: a host implements them to serve a session.
pub use crate::host::{EventSink, HostCatalog, HostSignal};

/// The one refusal string. Same wording as the share module: a peer learns
/// nothing about what exists from a denial.
pub const DENIED: &str = "path not found or out of root";

/// Concurrent sessions one peer may hold. The subscription fan-out and the
/// per-request work are both per session, so this is the cap that bounds a
/// paired-but-misbehaving machine (design §7, "resource exhaustion").
pub const MAX_SESSIONS_PER_PEER: usize = 4;

/// Frames buffered per session before the host starts dropping EVENTS (never
/// responses). A client that stops reading slows itself, not the host.
const SESSION_QUEUE: usize = 64;

/// How long a staged artifact stays fetchable by the peer that asked for it.
/// Long enough to fetch and refetch after a live-reload event, short enough
/// that a closed session's bytes stop being reachable.
pub const GRANT_TTL_SECS: u64 = 3600;

/// Directory children returned by one `ListTree`. Bounds one hostile request.
pub const MAX_TREE_ENTRIES: usize = 2_000;

/// Handshake timeout. A peer that opens a connection and says nothing must
/// not hold a session slot forever. The dial half lives in
/// `endpoint::DIAL_TIMEOUT`, shared with Beam.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// QUIC close code for a refused session. The value is arbitrary; the peer
/// only needs to see that it was refused, not why.
const CLOSE_REFUSED: u32 = 1;

/// Subject of every failed dial this module makes — a session, a pairing
/// handshake, a cache fetch. `endpoint::dial` appends the cause, and a
/// timeout reads the same as a refusal because both mean the same thing to
/// the person looking at the screen.
const PEER_OFFLINE: &str = "peer offline — could not reach it";

// ── Grants: peer-locked blob capabilities ──────────────────────────────────

struct Grant {
    /// Peers allowed to fetch this hash. A grant is NOT a beam ticket:
    /// possession of the hash is not enough, the fetching NodeId must be one
    /// that asked for the artifact through a scoped session.
    peers: HashSet<EndpointId>,
    expires_at: u64,
    tag: Tag,
}

impl Grant {
    /// Let one more peer fetch these bytes, and restart the clock — a peer
    /// that just asked is about to fetch.
    fn admit(&mut self, peer: EndpointId) {
        self.peers.insert(peer);
        self.expires_at = now_unix() + GRANT_TTL_SECS;
    }
}

/// Blob capabilities minted by `GetArtifact`, consulted by the blobs request
/// gate beside the Beam offers registry.
#[derive(Default)]
pub struct Grants {
    inner: Mutex<HashMap<Hash, Grant>>,
}

// Hand-written: the router requires `Debug` on a protocol handler, and a
// grant's contents (which peer may fetch what) are not log material.
impl std::fmt::Debug for Grants {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Grants").finish_non_exhaustive()
    }
}

impl Grants {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record (or refresh) a grant. Returns a tag the caller must delete: the
    /// redundant staging tag when this content was already staged — leaving
    /// it would pin a second copy of the bytes that nothing can reach.
    fn insert(&self, hash: Hash, peer: EndpointId, tag: Tag) -> Option<Tag> {
        let mut map = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        match map.get_mut(&hash) {
            Some(existing) => {
                existing.admit(peer);
                Some(tag)
            }
            None => {
                map.insert(
                    hash,
                    Grant {
                        peers: HashSet::from([peer]),
                        expires_at: now_unix() + GRANT_TTL_SECS,
                        tag,
                    },
                );
                None
            }
        }
    }

    /// Widen an EXISTING grant to one more peer, refreshing its TTL. `false`
    /// when nothing is staged under `hash`: a grant with no bytes behind it is
    /// a dangling capability, so the caller must stage first.
    ///
    /// This is the cheap half of `stage_for_peer`. Re-staging a file the
    /// watcher touched costs a read plus a BLAKE3 pass and produces the same
    /// hash for every interested peer — only the admission is per peer.
    fn add_peer(&self, hash: Hash, peer: EndpointId) -> bool {
        let mut map = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        match map.get_mut(&hash) {
            Some(grant) => {
                grant.admit(peer);
                true
            }
            None => false,
        }
    }

    /// The gate's question: may this connection fetch this hash? Unknown,
    /// expired and wrong-peer all answer the same `false`.
    pub fn admit(&self, hash: &Hash, peer: Option<EndpointId>, is_blob_request: bool) -> bool {
        if !is_blob_request {
            return false;
        }
        let Some(peer) = peer else { return false };
        let map = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        match map.get(hash) {
            Some(grant) => grant.expires_at > now_unix() && grant.peers.contains(&peer),
            None => false,
        }
    }

    /// Drop expired grants, returning their staging tags for cleanup.
    fn take_expired(&self, now: u64) -> Vec<Tag> {
        let mut map = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let mut expired = Vec::new();
        map.retain(|_, grant| {
            if grant.expires_at > now {
                return true;
            }
            expired.push(grant.tag.clone());
            false
        });
        expired
    }

    /// Revoke every grant held by one peer — what unpairing must do to bytes
    /// already staged for it. Returns the tags of grants nobody holds anymore.
    pub fn revoke_peer(&self, peer: &EndpointId) -> Vec<Tag> {
        let mut map = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let mut orphaned = Vec::new();
        map.retain(|_, grant| {
            grant.peers.remove(peer);
            if grant.peers.is_empty() {
                orphaned.push(grant.tag.clone());
                false
            } else {
                true
            }
        });
        orphaned
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).len()
    }
}

// ── Tabs bridge: React session state cached for the wire ───────────────────

/// The host's open-tab list, published from the tabs reducer on every commit
/// (`remote_publish_tabs`) and diffed into events for subscribers.
#[derive(Default)]
pub struct TabsCache {
    inner: Mutex<Vec<TabEntry>>,
}

impl TabsCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn list(&self) -> Vec<TabEntry> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Replace the cached list, returning the events the change implies.
    pub fn publish(&self, next: Vec<TabEntry>) -> Vec<Event> {
        let mut guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let events = diff_tabs(&guard, &next);
        *guard = next;
        events
    }
}

/// Derive `TabOpened` / `TabClosed` / `TabActivated` from two published tab
/// lists. Pure — the whole tab-event surface is testable without a webview.
///
/// Paths, not tab ids, are the wire identity: the client renders a list of
/// artifacts, and two tabs on the same file are one entry to it. Order is
/// fixed (closed, then opened, then activated) so a client applying the batch
/// never sees a path briefly absent from a list it is about to re-add.
pub fn diff_tabs(prev: &[TabEntry], next: &[TabEntry]) -> Vec<Event> {
    let (prev_paths, prev_set) = unique_paths(prev);
    let (next_paths, next_set) = unique_paths(next);

    let mut events = Vec::new();
    for path in prev_paths {
        if !next_set.contains(path) {
            events.push(Event::TabClosed { path: path.to_string() });
        }
    }
    for path in next_paths {
        if !prev_set.contains(path) {
            events.push(Event::TabOpened { path: path.to_string() });
        }
    }
    if let Some(active) = active_of(next) {
        if active_of(prev) != Some(active) {
            events.push(Event::TabActivated { path: active.to_string() });
        }
    }
    events
}

/// Normalize a published tab list for the wire: gate every path and keep it
/// in CANONICAL form. Two reasons, both load-bearing — an out-of-root tab
/// must never be announced to a peer, and the watcher emits canonical paths,
/// so tab identity and `FileChanged` identity have to be the same string.
pub fn canonical_tabs(tabs: Vec<TabEntry>, roots: &RootSet) -> Vec<TabEntry> {
    tabs.into_iter()
        .filter_map(|t| {
            let canonical = gate_raw(&t.path, roots).ok()?;
            Some(TabEntry {
                path: canonical.to_string_lossy().into_owned(),
                active: t.active,
            })
        })
        .collect()
}

/// The security gate with the wire's input hardening in front of it: absolute
/// path, no NUL bytes (the deep-link parser's rules), then
/// `canonicalize_and_check_root`. One refusal string for every failure, so a
/// peer cannot tell a missing file from a forbidden one.
///
/// Free-standing because BOTH directions need it and only one of them has a
/// `ScopeState`: an inbound request gates the path a peer named, and
/// `canonical_tabs` gates every path this host is about to announce.
fn gate_raw(raw: &str, roots: &RootSet) -> Result<PathBuf, String> {
    if raw.is_empty() || !raw.starts_with('/') || raw.contains('\0') {
        return Err(DENIED.to_string());
    }
    security::canonicalize_and_check_root(Path::new(raw), roots).map_err(|_| DENIED.to_string())
}

/// The distinct non-empty paths of a tab list, in strip order, plus the same
/// paths as a set. Both come out of ONE pass: the diff needs the order to emit
/// stable events and the set to answer "was this open before".
fn unique_paths(tabs: &[TabEntry]) -> (Vec<&str>, HashSet<&str>) {
    let mut seen = HashSet::new();
    let ordered: Vec<&str> = tabs
        .iter()
        .map(|t| t.path.as_str())
        .filter(|p| !p.is_empty() && seen.insert(*p))
        .collect();
    (ordered, seen)
}

/// The active tab's path, borrowed. An empty path is the start page, not an
/// artifact: it is neither listed nor activated on the wire.
fn active_of(tabs: &[TabEntry]) -> Option<&str> {
    tabs.iter()
        .find(|t| t.active && !t.path.is_empty())
        .map(|t| t.path.as_str())
}

// ── Host state ─────────────────────────────────────────────────────────────

/// Everything the host side needs that exists before the endpoint boots. Held
/// by `RemoteState` so `remote_list_peers` / `remote_publish_tabs` work with
/// zero sockets, and handed to the router at boot.
pub struct ScopeState {
    pub peers: Arc<PeerStore>,
    pub pairing: Arc<Pairing>,
    pub tabs: Arc<TabsCache>,
    pub roots: RootSet,
    pub device: String,
    /// What this host considers starred and recently opened. The app passes
    /// its own stores; a headless host passes `EmptyCatalog`.
    pub catalog: Arc<dyn HostCatalog>,
    sessions: Mutex<Vec<Session>>,
    next_session_id: AtomicU64,
    signal: Box<dyn EventSink>,
}

impl std::fmt::Debug for ScopeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScopeState").field("device", &self.device).finish_non_exhaustive()
    }
}

struct Session {
    id: u64,
    peer: String,
    tx: mpsc::Sender<Frame>,
    /// Shared with the accept loop serving this session — ONE `Arc`, so the
    /// registry and the request handler cannot end up looking at different
    /// flags. Pairing two `Arc` fields by hand was how they could.
    handles: Arc<SessionHandles>,
}

/// The per-session state the request handler mutates and the fan-out reads.
struct SessionHandles {
    subscribed: AtomicBool,
    /// Canonical paths this session fetched. A file change only crosses the
    /// wire for artifacts the client actually holds — that bounds re-hashing
    /// to what someone is looking at.
    interest: Mutex<HashSet<PathBuf>>,
}

impl SessionHandles {
    fn new() -> Self {
        Self {
            subscribed: AtomicBool::new(false),
            interest: Mutex::new(HashSet::new()),
        }
    }

    fn is_subscribed(&self) -> bool {
        self.subscribed.load(Ordering::SeqCst)
    }

    fn holds(&self, path: &Path) -> bool {
        self.interest.lock().unwrap_or_else(|p| p.into_inner()).contains(path)
    }
}

impl ScopeState {
    pub fn new(
        peers: Arc<PeerStore>,
        pairing: Arc<Pairing>,
        tabs: Arc<TabsCache>,
        roots: RootSet,
        device: String,
        catalog: Arc<dyn HostCatalog>,
        signal: impl EventSink,
    ) -> Self {
        Self {
            peers,
            pairing,
            tabs,
            roots,
            device,
            catalog,
            sessions: Mutex::new(Vec::new()),
            next_session_id: AtomicU64::new(1),
            signal: Box::new(signal),
        }
    }

    fn sessions(&self) -> std::sync::MutexGuard<'_, Vec<Session>> {
        self.sessions.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Register a session, refusing the peer past its concurrency cap.
    fn register(&self, session: Session) -> Result<u64, String> {
        let mut sessions = self.sessions();
        if sessions.iter().filter(|s| s.peer == session.peer).count() >= MAX_SESSIONS_PER_PEER {
            return Err("too many open sessions for this peer".to_string());
        }
        let id = session.id;
        sessions.push(session);
        Ok(id)
    }

    fn unregister(&self, id: u64) {
        self.sessions().retain(|s| s.id != id);
    }

    /// Drop every session held by a peer — what revocation does to sessions
    /// that were already open when the user unpaired.
    pub fn drop_sessions_for(&self, node_id: &str) {
        self.sessions().retain(|s| s.peer != node_id);
    }

    /// Push a tab list from the reducer and fan the derived events out to
    /// subscribers. Returns the events for the caller to log or test.
    pub fn publish_tabs(&self, tabs: Vec<TabEntry>) -> Vec<Event> {
        let events = self.tabs.publish(canonical_tabs(tabs, &self.roots));
        for event in &events {
            self.broadcast(event.clone(), None);
        }
        events
    }

    /// Send an event to subscribed sessions. `only_interested_in` restricts
    /// the fan-out to sessions that hold that path.
    fn broadcast(&self, event: Event, only_interested_in: Option<&Path>) {
        let sessions = self.sessions();
        for session in sessions.iter() {
            if !session.handles.is_subscribed() {
                continue;
            }
            if only_interested_in.is_some_and(|path| !session.handles.holds(path)) {
                continue;
            }
            // Events are droppable by design: a client that stopped reading
            // must not stall the host's watcher bridge.
            let _ = session.tx.try_send(Frame::Event(event.clone()));
        }
    }

    /// Did any subscribed session fetch this path? The one question the
    /// watcher bridge asks per event, and the answer that decides whether the
    /// file is worth re-reading and re-hashing at all — so it short-circuits
    /// on the first holder instead of collecting every session's interest.
    fn any_session_holds(&self, path: &Path) -> bool {
        self.sessions()
            .iter()
            .any(|s| s.handles.is_subscribed() && s.handles.holds(path))
    }

    /// Is this canonical path one a view-open peer was told about — an open
    /// tab, a bookmark or a recent? Every candidate is gated on the way past,
    /// so this can only ever narrow what the RootSet already admitted.
    ///
    /// Short-circuiting on purpose: one `.contains` does not justify
    /// canonicalizing every tab, bookmark and recent the host holds, and the
    /// answer must stay live (a tab opened a moment ago is fetchable).
    fn is_published(&self, canonical: &Path) -> bool {
        let published = |raw: &str| self.gate_path(raw).is_ok_and(|p| p == canonical);
        self.tabs.list().iter().any(|t| published(&t.path))
            || self.catalog.bookmarks().iter().any(|p| published(&p.to_string_lossy()))
            || self.catalog.recents().iter().any(|p| published(&p.to_string_lossy()))
    }

    /// This host's roots applied to one raw wire path. See `gate_raw`.
    fn gate_path(&self, raw: &str) -> Result<PathBuf, String> {
        gate_raw(raw, &self.roots)
    }

    /// The full path check for one request: security gate, then the
    /// view-open peer's narrowing to the artifacts it was told about.
    fn gate_for(&self, peer: &Peer, raw: &str) -> Result<PathBuf, String> {
        let canonical = self.gate_path(raw)?;
        if peer.scope == Scope::ViewOpen && !self.is_published(&canonical) {
            return Err(DENIED.to_string());
        }
        Ok(canonical)
    }

    fn signal(&self, signal: HostSignal) {
        self.signal.emit(signal);
    }
}

// ── Host: the scope protocol server ────────────────────────────────────────

/// The `vlerv/scope/0` handler. Registered on the same Router as iroh-blobs,
/// so one endpoint serves both protocols (design §4).
#[derive(Debug, Clone)]
pub struct ScopeServer {
    pub state: Arc<ScopeState>,
    store: FsStore,
    grants: Arc<Grants>,
    /// This instance's endpoint. The host DIALS with it to pull a pushed
    /// artifact — the one request shape where bytes travel toward the host.
    endpoint: Endpoint,
    /// Where a pushed artifact lands. Derived from the consumer's base dir,
    /// never hardcoded.
    dirs: Dirs,
}

impl ScopeServer {
    pub fn new(
        state: Arc<ScopeState>,
        store: FsStore,
        grants: Arc<Grants>,
        endpoint: Endpoint,
        dirs: Dirs,
    ) -> Self {
        Self { state, store, grants, endpoint, dirs }
    }

    /// Bridge one watcher event to subscribers. A changed file is re-hashed
    /// so the event carries the NEW content address the client refetches by
    /// (design §6); a removed file needs no hash.
    pub async fn note_change(&self, path: &Path, removed: bool) {
        let canonical = match path.canonicalize() {
            Ok(canonical) => canonical,
            // A removed file cannot canonicalize; fall back to the raw path,
            // which the interest set stores in canonical form anyway.
            Err(_) if removed => path.to_path_buf(),
            Err(_) => return,
        };
        if !self.state.any_session_holds(&canonical) {
            return;
        }
        let wire_path = canonical.to_string_lossy().into_owned();
        if removed {
            self.state
                .broadcast(Event::FileRemoved { path: wire_path }, Some(&canonical));
            return;
        }

        // Re-stage under the same peer grants that already hold this path, so
        // the refetch of the new hash is admitted without a second round trip.
        // ONE staging pass for all of them: `add_path` re-reads the file and
        // re-runs BLAKE3 over it, and neither depends on who is asking — the
        // rest of the interested peers only need the admission.
        let peers = self.peers_holding(&canonical);
        let Some((first, rest)) = peers.split_first() else { return };
        let staged = match stage_for_peer(&self.store, &self.grants, &canonical, *first).await {
            Ok(staged) => staged,
            Err(e) => {
                eprintln!("vlerv: scope: cannot re-stage {canonical:?}: {e}");
                return;
            }
        };
        for peer in rest {
            if !self.grants.add_peer(staged.hash, *peer) {
                // The grant `stage_for_peer` just made was revoked underneath
                // us. Say so rather than drop it silently: that peer's refetch
                // of the new hash will be refused at the gate.
                eprintln!("vlerv: scope: no live grant to widen for {canonical:?}");
            }
        }
        self.state.broadcast(
            Event::FileChanged { path: wire_path, hash: staged.hash.to_string() },
            Some(&canonical),
        );
    }

    fn peers_holding(&self, path: &Path) -> Vec<EndpointId> {
        let sessions = self.state.sessions();
        sessions
            .iter()
            .filter(|s| s.handles.is_subscribed() && s.handles.holds(path))
            .filter_map(|s| s.peer.parse::<EndpointId>().ok())
            .collect()
    }

    /// Revoke a peer's grants and drop its sessions. Called on unpair.
    pub async fn revoke(&self, node_id: &str) {
        self.state.drop_sessions_for(node_id);
        if let Ok(id) = node_id.parse::<EndpointId>() {
            let orphaned = self.grants.revoke_peer(&id);
            delete_tags(&self.store, orphaned).await;
        }
    }

    async fn handle(
        &self,
        peer: &Peer,
        peer_id: EndpointId,
        req: Req,
        session: &SessionHandles,
    ) -> Res {
        // 2 — verb-level scope filter, before any path touches the disk.
        if !peer.scope.allows(&req) {
            return Res::Denied("not permitted for this peer".to_string());
        }
        // Every refusal below — scope, path, size — is an `Err`, and this is
        // the ONE place it becomes the no-existence-leak `Denied` frame.
        self.dispatch(peer, peer_id, req, session).await.unwrap_or_else(Res::Denied)
    }

    async fn dispatch(
        &self,
        peer: &Peer,
        peer_id: EndpointId,
        req: Req,
        session: &SessionHandles,
    ) -> Result<Res, String> {
        match req {
            // A second Hello on a live session is a protocol error, not a
            // request; answer it as a refusal rather than re-handshaking.
            Req::Hello { .. } => Err("session already established".to_string()),
            // The cached list is already gated AND canonicalized: nothing
            // reaches it except through `canonical_tabs` at publish time, so
            // re-canonicalizing every entry per request would only re-derive
            // an answer the host already holds.
            Req::ListTabs => Ok(Res::Tabs(self.state.tabs.list())),
            Req::ListBookmarks => Ok(Res::Paths(
                self.state
                    .catalog
                    .bookmarks()
                    .into_iter()
                    .filter_map(|p| self.path_entry(&p))
                    .collect(),
            )),
            Req::ListRecents => Ok(Res::Paths(
                self.state
                    .catalog
                    .recents()
                    .into_iter()
                    .filter_map(|p| self.path_entry(&p))
                    .collect(),
            )),
            Req::ListTree { path } => {
                let canonical = self.state.gate_for(peer, &path)?;
                Ok(Res::Tree(list_tree(&canonical)?))
            }
            Req::GetArtifact { path } => {
                let (meta, canonical) = self.artifact(peer, peer_id, &path).await?;
                session
                    .interest
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .insert(canonical);
                Ok(Res::Artifact(meta))
            }
            Req::Subscribe => {
                session.subscribed.store(true, Ordering::SeqCst);
                Ok(Res::Subscribed)
            }
            Req::Unsubscribe => {
                session.subscribed.store(false, Ordering::SeqCst);
                Ok(Res::Subscribed)
            }
            Req::OpenOnHost { path, reader_mode } => {
                let canonical = self.state.gate_for(peer, &path)?;
                self.state.signal(HostSignal::OpenOnHost {
                    peer: peer.node_id.clone(),
                    path: canonical,
                    reader_mode,
                });
                Ok(Res::Opened)
            }
            Req::PushArtifact { name, size, hash, ticket } => {
                let file = self.accept_push(peer, peer_id, &name, size, &hash, &ticket).await?;
                Ok(Res::Pushed { name: file.name, size: file.size })
            }
        }
    }

    /// Land an artifact a control peer pushed. No path of the host's is read
    /// here — the direction is reversed, so the RootSet gate does its work on
    /// the CLIENT side (`ClientSession::push_artifact` resolves the file
    /// through the same offer policy Beam uses). What the host enforces is
    /// who may push, from where, and how much:
    ///
    ///   1. control scope — already refused in `handle` before we get here;
    ///   2. the ticket is peer-locked to the pushing NodeId and names the
    ///      announced hash (`beam::verify_push_ticket`);
    ///   3. the announced size is under the same hard cap Beam enforces, and
    ///      the REAL cap is re-enforced on the actual stream inside
    ///      `beam::receive_via`, which is byte-for-byte the Beam receive path
    ///      — BLAKE3 verification, `.partial` staging, collision naming, and
    ///      `received/<date>/` as the only place the app ever writes.
    ///
    /// The landing is announced as `HostSignal::ArtifactReceived`, so the
    /// desktop opens it in a tab exactly like an accepted beam and a headless
    /// host handles it its own way.
    async fn accept_push(
        &self,
        peer: &Peer,
        peer_id: EndpointId,
        name: &str,
        size: u64,
        hash: &str,
        ticket: &str,
    ) -> Result<crate::beam::ReceivedFile, String> {
        // The announced size is a hint, refused early so an oversized push
        // costs no dial at all; the stream itself is capped regardless.
        if size > crate::beam::HARD_CAP_BYTES {
            return Err("artifact exceeds the transfer size cap".to_string());
        }
        crate::beam::verify_push_ticket(ticket, peer_id, hash)?;

        let file = crate::beam::receive_via(
            &self.endpoint,
            ticket,
            Some(name),
            &self.dirs.received(),
            |_, _| {},
        )
        .await?;

        // The name that matters downstream is the one ON DISK: collision
        // handling may have landed `report.html` as `report-2.html`, and both
        // the signal and the response describe the file the host now holds.
        let landed = base_name(&file.path).unwrap_or_else(|| file.name.clone());
        self.state.signal(HostSignal::ArtifactReceived {
            peer: peer.node_id.clone(),
            path: file.path.clone(),
            name: landed.clone(),
            size: file.size,
            hash: file.hash.clone(),
        });
        Ok(crate::beam::ReceivedFile { name: landed, ..file })
    }

    fn path_entry(&self, path: &Path) -> Option<PathEntry> {
        let canonical = self.state.gate_path(&path.to_string_lossy()).ok()?;
        Some(PathEntry {
            name: base_name(&canonical).unwrap_or_default(),
            path: canonical.to_string_lossy().into_owned(),
        })
    }

    async fn artifact(
        &self,
        peer: &Peer,
        peer_id: EndpointId,
        raw: &str,
    ) -> Result<(ArtifactMeta, PathBuf), String> {
        let canonical = self.state.gate_for(peer, raw)?;
        let meta = std::fs::metadata(&canonical).map_err(|_| DENIED.to_string())?;
        if !meta.is_file() {
            return Err(DENIED.to_string());
        }
        // Same caps as Beam: the transfer path is the same blob protocol.
        if meta.len() > crate::beam::HARD_CAP_BYTES {
            return Err("artifact exceeds the transfer size cap".to_string());
        }
        let staged = stage_for_peer(&self.store, &self.grants, &canonical, peer_id).await?;
        Ok((
            ArtifactMeta {
                hash: staged.hash.to_string(),
                size: meta.len(),
                mtime: mtime_secs(&meta),
                warn: meta.len() > crate::beam::WARN_BYTES,
            },
            canonical,
        ))
    }
}

/// Stage a file into the blob store and grant EXACTLY ONE peer a fetch of the
/// resulting hash — plus the housekeeping sweep that unpins whatever expired
/// while the store was at hand.
///
/// The `add_path` and the `grants.insert` are one operation on purpose, in one
/// place, used by both directions: the host staging an artifact a peer asked
/// for, and a client staging one it is about to push. Bytes in the store with
/// no grant behind them are unreachable, and a grant is what makes the fetch
/// peer-locked — splitting the pair is how a hash becomes fetchable by anyone
/// who learns it.
async fn stage_for_peer(
    store: &FsStore,
    grants: &Grants,
    path: &Path,
    peer: EndpointId,
) -> Result<HashAndFormat, String> {
    delete_tags(store, grants.take_expired(now_unix())).await;
    let tag = store
        .blobs()
        .add_path(path)
        .await
        .map_err(|e| format!("cannot stage artifact: {e}"))?;
    let staged = HashAndFormat { hash: tag.hash, format: tag.format };
    if let Some(redundant) = grants.insert(tag.hash, peer, tag.name) {
        // This content was already staged: the fresh tag would pin a second
        // copy of the same bytes that nothing can reach.
        delete_tags(store, vec![redundant]).await;
    }
    Ok(staged)
}

impl ProtocolHandler for ScopeServer {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let peer_id = connection.remote_id();
        let node_id = peer_id.to_string();

        // 1 — the allowlist, before a single request byte is parsed. QUIC
        // already authenticated this NodeId, so there is nothing to spoof.
        let Some(peer) = self.state.peers.get(&node_id) else {
            connection.close(VarInt::from_u32(CLOSE_REFUSED), b"not a paired peer");
            return Err(refused("not a paired peer"));
        };
        self.state.peers.touch(&node_id);

        let (mut send, mut recv) = connection.accept_bi().await?;

        // Handshake: version first, everything else after.
        let hello = tokio::time::timeout(HANDSHAKE_TIMEOUT, read_frame::<Req>(&mut recv))
            .await
            .map_err(|_| refused("handshake timed out"))?
            .map_err(refused_owned)?;
        let device = match hello {
            Req::Hello { proto, device } if proto == proto::PROTO_VERSION => {
                proto::sanitize_device(&device)
            }
            Req::Hello { .. } => {
                write_frame(&mut send, &Frame::Res(Res::Denied("unsupported protocol version".into())))
                    .await
                    .ok();
                connection.close(VarInt::from_u32(CLOSE_REFUSED), b"protocol version");
                return Err(refused("unsupported protocol version"));
            }
            _ => {
                connection.close(VarInt::from_u32(CLOSE_REFUSED), b"expected hello");
                return Err(refused("expected hello"));
            }
        };
        // The device name travels in every handshake (design §4): keep the
        // stored name current without touching the granted scope.
        let _ = self.state.peers.upsert(&node_id, &device, peer.scope);

        let (tx, mut rx) = mpsc::channel::<Frame>(SESSION_QUEUE);
        let handles = Arc::new(SessionHandles::new());
        let id = self.state.next_session_id.fetch_add(1, Ordering::SeqCst);
        self.state
            .register(Session {
                id,
                peer: node_id.clone(),
                tx: tx.clone(),
                handles: handles.clone(),
            })
            .map_err(|e| {
                connection.close(VarInt::from_u32(CLOSE_REFUSED), b"session cap");
                refused_owned(e)
            })?;

        // One writer owns the send half: responses and pushed events share
        // the stream without interleaving mid-frame.
        let writer = tokio::spawn(async move {
            while let Some(frame) = rx.recv().await {
                if write_frame(&mut send, &frame).await.is_err() {
                    break;
                }
            }
            let _ = send.finish();
        });

        tx.send(Frame::Res(Res::Hello(HelloAck {
            proto: proto::PROTO_VERSION,
            device: self.state.device.clone(),
            scope: peer.scope.as_str().to_string(),
        })))
        .await
        .ok();

        let result = self.serve(&node_id, peer_id, &mut recv, &tx, &handles).await;
        self.state.unregister(id);
        drop(tx);
        writer.abort();
        if let Err(e) = &result {
            eprintln!("vlerv: scope: session with {node_id} ended: {e}");
        }
        connection.close(VarInt::from_u32(0), b"bye");
        Ok(())
    }
}

impl ScopeServer {
    async fn serve(
        &self,
        node_id: &str,
        peer_id: EndpointId,
        recv: &mut RecvStream,
        tx: &mpsc::Sender<Frame>,
        handles: &SessionHandles,
    ) -> Result<(), String> {
        loop {
            let req: Req = match read_frame(recv).await {
                Ok(req) => req,
                // A closed stream is the normal end of a session.
                Err(e) => return Err(e),
            };
            // Re-read the peer per request: a revocation mid-session must
            // take effect on the next request, not on the next connection.
            let Some(peer) = self.state.peers.get(node_id) else {
                return Err("peer was revoked".to_string());
            };
            let res = self.handle(&peer, peer_id, req, handles).await;
            if tx.send(Frame::Res(res)).await.is_err() {
                return Err("client stopped reading".to_string());
            }
        }
    }
}

/// Depth-1 directory listing with the walker's policy (design §6: "same
/// ignore/hidden/symlink policy as ⌘P's list_files_recursive"): default-
/// ignored names out, symlinks out, hidden directories out, hidden files in.
fn list_tree(dir: &Path) -> Result<Vec<TreeEntry>, String> {
    let read_iter = std::fs::read_dir(dir).map_err(|_| DENIED.to_string())?;
    let mut entries = Vec::new();
    for raw in read_iter.flatten() {
        if entries.len() >= MAX_TREE_ENTRIES {
            break;
        }
        let name = raw.file_name().to_string_lossy().into_owned();
        if DEFAULT_IGNORED.contains(&name.as_str()) {
            continue;
        }
        let Ok(file_type) = raw.file_type() else { continue };
        if file_type.is_symlink() {
            continue;
        }
        let is_dir = file_type.is_dir();
        if is_dir && name.starts_with('.') {
            continue;
        }
        entries.push(TreeEntry {
            name,
            path: raw.path().to_string_lossy().into_owned(),
            is_dir,
        });
    }
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
    Ok(entries)
}

// ── Host: the pairing server (its own ALPN) ────────────────────────────────

/// `vlerv/pair/0`. The only door an unpaired NodeId may knock on, and only
/// with a live one-time token. Nothing is persisted here — the handshake
/// parks a pending pairing and the local human confirms the fingerprint.
#[derive(Debug, Clone)]
pub struct PairServer {
    pub state: Arc<ScopeState>,
    /// This endpoint's own NodeId, threaded in at construction: the
    /// fingerprint is derived from BOTH ids, and a `Connection` only exposes
    /// the remote one.
    local: EndpointId,
}

impl PairServer {
    pub fn new(state: Arc<ScopeState>, local: EndpointId) -> Self {
        Self { state, local }
    }
}

impl ProtocolHandler for PairServer {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let peer_id = connection.remote_id();
        let (mut send, mut recv) = connection.accept_bi().await?;
        let hello = tokio::time::timeout(HANDSHAKE_TIMEOUT, read_frame::<PairHello>(&mut recv))
            .await
            .map_err(|_| refused("pairing handshake timed out"))?
            .map_err(refused_owned)?;

        let ack = if hello.proto != proto::PROTO_VERSION {
            PairAck::Denied("unsupported protocol version".to_string())
        } else if !self.state.pairing.consume(&hello.token) {
            // Unknown, expired and already-used tokens are one refusal.
            PairAck::Denied("pairing is not open on this machine".to_string())
        } else {
            let device = proto::sanitize_device(&hello.device);
            self.state.signal(HostSignal::PairPending(PendingPair {
                node_id: peer_id.to_string(),
                device: device.clone(),
                fingerprint: peers::fingerprint(&self.local, &peer_id),
                role: "host".to_string(),
                created_at: now_unix(),
            }));
            PairAck::Ok {
                proto: proto::PROTO_VERSION,
                device: self.state.device.clone(),
            }
        };
        write_frame(&mut send, &ack).await.map_err(refused_owned)?;
        let _ = send.finish();
        connection.closed().await;
        Ok(())
    }
}

// ── Client: dialing a host ─────────────────────────────────────────────────

/// What a completed `push_artifact` reports back: the name the HOST gave the
/// landed file (collision handling may have renamed it) and the size it
/// actually measured — never the numbers this side announced.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PushedArtifact {
    pub name: String,
    pub size: u64,
    /// BLAKE3 content address, hex — the same string both sides verified.
    pub hash: String,
}

/// A live session with a paired host. Owns a reader task (responses and
/// events) and a writer task (requests), so the caller just awaits futures.
pub struct ClientSession {
    pub peer: String,
    pub device: String,
    pub scope: String,
    reqs: mpsc::Sender<(Req, oneshot::Sender<Res>)>,
    closed: Arc<AtomicBool>,
    /// The staging half of `push_artifact`: this instance's endpoint (the
    /// address the host dials back on), its blob store, and the grant
    /// registry the local request gate consults. Cheap clones of the node's,
    /// held here so a push needs nothing but the session.
    endpoint: Endpoint,
    store: FsStore,
    grants: Arc<Grants>,
}

// Hand-written: the blob store is not `Debug`, and a session's interesting
// facts are the three strings.
impl std::fmt::Debug for ClientSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientSession")
            .field("peer", &self.peer)
            .field("device", &self.device)
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}


impl ClientSession {
    /// Dial `addr` on the scope ALPN, handshake, and start the pumps.
    /// `on_event` receives every pushed event; `on_closed` fires once when
    /// the session ends, which is what drives presence back to offline.
    pub async fn connect(
        node: &RemoteNode,
        addr: EndpointAddr,
        device: String,
        on_event: impl Fn(Event) + Send + Sync + 'static,
        on_closed: impl FnOnce() + Send + 'static,
    ) -> Result<Arc<Self>, String> {
        let peer = addr.id.to_string();
        let connection = endpoint::dial(&node.endpoint, addr, proto::SCOPE_ALPN, PEER_OFFLINE).await?;

        let (mut send, mut recv) = connection
            .open_bi()
            .await
            .map_err(|e| format!("cannot open the session stream: {e}"))?;

        write_frame(
            &mut send,
            &Req::Hello { proto: proto::PROTO_VERSION, device },
        )
        .await?;

        let ack = tokio::time::timeout(HANDSHAKE_TIMEOUT, read_frame::<Frame>(&mut recv))
            .await
            .map_err(|_| "the peer did not answer the handshake".to_string())??;
        let (host_device, scope) = match ack {
            Frame::Res(Res::Hello(ack)) if ack.proto == proto::PROTO_VERSION => {
                (ack.device, ack.scope)
            }
            Frame::Res(Res::Hello(_)) => return Err("unsupported protocol version".to_string()),
            Frame::Res(Res::Denied(reason)) => return Err(reason),
            _ => return Err("the peer answered with an unexpected frame".to_string()),
        };

        let (req_tx, mut req_rx) = mpsc::channel::<(Req, oneshot::Sender<Res>)>(32);
        // Responses arrive in request order; the reader pops the matching
        // waiter for each `Res` frame and routes every `Event` to the sink.
        let (waiter_tx, mut waiter_rx) = mpsc::unbounded_channel::<oneshot::Sender<Res>>();
        let closed = Arc::new(AtomicBool::new(false));

        let writer_closed = closed.clone();
        tokio::spawn(async move {
            while let Some((req, reply)) = req_rx.recv().await {
                // Register the waiter BEFORE the frame goes out. The reader
                // task runs concurrently, and a nearby host can answer while
                // this task is still inside `write_frame` — registering
                // afterwards let that response find an empty waiter queue,
                // which the reader reads as a desync and tears the session
                // down. A waiter left behind by a failed write is released
                // when this task drops `waiter_tx`.
                if waiter_tx.send(reply).is_err() {
                    break;
                }
                if write_frame(&mut send, &req).await.is_err() {
                    break;
                }
            }
            let _ = send.finish();
            writer_closed.store(true, Ordering::SeqCst);
        });

        let reader_closed = closed.clone();
        tokio::spawn(async move {
            loop {
                match read_frame::<Frame>(&mut recv).await {
                    Ok(Frame::Res(res)) => match waiter_rx.try_recv() {
                        Ok(reply) => {
                            let _ = reply.send(res);
                        }
                        // A response with nobody waiting means the two sides
                        // disagree about the stream; stop rather than guess.
                        Err(_) => break,
                    },
                    Ok(Frame::Event(event)) => on_event(event),
                    Err(_) => break,
                }
            }
            reader_closed.store(true, Ordering::SeqCst);
            on_closed();
        });

        Ok(Arc::new(Self {
            peer,
            device: host_device,
            scope,
            reqs: req_tx,
            closed,
            endpoint: node.endpoint.clone(),
            store: node.store.clone(),
            grants: node.grants.clone(),
        }))
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Issue one request and await its response.
    pub async fn request(&self, req: Req) -> Result<Res, String> {
        let (tx, rx) = oneshot::channel();
        self.reqs
            .send((req, tx))
            .await
            .map_err(|_| "the session is closed".to_string())?;
        rx.await.map_err(|_| "the session is closed".to_string())
    }

    /// Issue one request and pull the expected shape out of the answer.
    /// `extract` hands back any other variant untouched, which `unexpected`
    /// turns into the host's own wording — a `Denied` frame is the host's
    /// answer, not a protocol failure, and every accessor must surface it the
    /// same way.
    async fn request_as<T>(
        &self,
        req: Req,
        extract: fn(Res) -> Result<T, Res>,
    ) -> Result<T, String> {
        self.request(req).await.and_then(|res| extract(res).map_err(unexpected))
    }

    pub async fn list_tabs(&self) -> Result<Vec<TabEntry>, String> {
        self.request_as(Req::ListTabs, |res| match res {
            Res::Tabs(tabs) => Ok(tabs),
            other => Err(other),
        })
        .await
    }

    pub async fn list_bookmarks(&self) -> Result<Vec<PathEntry>, String> {
        self.request_as(Req::ListBookmarks, |res| match res {
            Res::Paths(paths) => Ok(paths),
            other => Err(other),
        })
        .await
    }

    pub async fn list_recents(&self) -> Result<Vec<PathEntry>, String> {
        self.request_as(Req::ListRecents, |res| match res {
            Res::Paths(paths) => Ok(paths),
            other => Err(other),
        })
        .await
    }

    pub async fn list_tree(&self, path: String) -> Result<Vec<TreeEntry>, String> {
        self.request_as(Req::ListTree { path }, |res| match res {
            Res::Tree(entries) => Ok(entries),
            other => Err(other),
        })
        .await
    }

    pub async fn get_artifact(&self, path: String) -> Result<ArtifactMeta, String> {
        self.request_as(Req::GetArtifact { path }, |res| match res {
            Res::Artifact(meta) => Ok(meta),
            other => Err(other),
        })
        .await
    }

    pub async fn open_on_host(&self, path: String, reader_mode: bool) -> Result<(), String> {
        self.request_as(Req::OpenOnHost { path, reader_mode }, |res| match res {
            Res::Opened => Ok(()),
            other => Err(other),
        })
        .await
    }

    pub async fn subscribe(&self) -> Result<(), String> {
        self.request_as(Req::Subscribe, |res| match res {
            Res::Subscribed => Ok(()),
            other => Err(other),
        })
        .await
    }

    pub async fn unsubscribe(&self) -> Result<(), String> {
        self.request_as(Req::Unsubscribe, |res| match res {
            Res::Subscribed => Ok(()),
            other => Err(other),
        })
        .await
    }

    /// Push a local artifact onto the host — the reverse of `get_artifact`,
    /// and control scope on the host's side (a lesser peer gets the same
    /// single refusal string every other over-reach gets).
    ///
    /// The bytes never ride the control stream. `path` goes through the SAME
    /// offer policy Beam uses (`beam::resolve_offerable`: the conservative
    /// share gate over the RootSet, files only, hard cap), is staged into this
    /// instance's blob store, and is granted to exactly one peer — the host —
    /// so the ticket in the frame is useless to anybody else who sees it.
    pub async fn push_artifact(
        &self,
        path: &Path,
        roots: &RootSet,
    ) -> Result<PushedArtifact, String> {
        // Bounded wait for relay + discovery so the host can dial back from
        // another network; on timeout the address still carries direct addrs,
        // which covers the same-LAN case. Same policy as minting a beam
        // ticket.
        let _ = tokio::time::timeout(Duration::from_secs(10), self.endpoint.online()).await;
        self.push_artifact_via(path, roots, self.endpoint.addr()).await
    }

    /// `push_artifact` with the call-back address pinned to one socket, named
    /// in plain `std` types. The two-endpoint tests hand this `127.0.0.1:<own
    /// port>` so the host dials back over loopback — the same reason the Beam
    /// test re-mints its ticket — without a consumer ever naming an iroh type.
    pub async fn push_artifact_at(
        &self,
        path: &Path,
        roots: &RootSet,
        socket: std::net::SocketAddr,
    ) -> Result<PushedArtifact, String> {
        let addr = endpoint::addr_at_id(self.endpoint.id(), socket);
        self.push_artifact_via(path, roots, addr).await
    }

    /// `push_artifact` with an explicit call-back address. The app uses the
    /// endpoint's own address; the two-endpoint transfer test passes loopback,
    /// the same way the Beam test re-mints its ticket, so the proof never
    /// depends on relays or discovery.
    pub async fn push_artifact_via(
        &self,
        path: &Path,
        roots: &RootSet,
        addr: EndpointAddr,
    ) -> Result<PushedArtifact, String> {
        let cand = crate::beam::resolve_offerable(path, roots)?;
        let host: EndpointId = self
            .peer
            .parse()
            .map_err(|_| "malformed peer id".to_string())?;

        // Peer-locked, through the same pairing the host uses: possession of
        // the ticket is NOT enough on this side either, because the local
        // request gate admits the hash only for the host we are pushing to.
        let staged = stage_for_peer(&self.store, &self.grants, &cand.canonical, host).await?;

        let hash = staged.hash.to_string();
        let ticket =
            iroh_blobs::ticket::BlobTicket::new(addr, staged.hash, staged.format).to_string();
        let (name, size) = self
            .request_as(
                Req::PushArtifact {
                    name: cand.name,
                    size: cand.size,
                    hash: hash.clone(),
                    ticket,
                },
                |res| match res {
                    Res::Pushed { name, size } => Ok((name, size)),
                    other => Err(other),
                },
            )
            .await?;
        Ok(PushedArtifact { name, size, hash })
    }
}

/// A `Denied` frame is the host's answer, not a bug — surface its wording
/// verbatim so the UI shows the same no-existence-leak string.
fn unexpected(res: Res) -> String {
    match res {
        Res::Denied(reason) => reason,
        other => format!("unexpected response from the peer: {other:?}"),
    }
}

/// Dial a host's pairing ALPN with a ticket's token. Returns the host's
/// device name and the six-word fingerprint both screens must show.
pub async fn pair_dial(
    node: &RemoteNode,
    ticket: &crate::peers::PairTicket,
    local_device: String,
) -> Result<PendingPair, String> {
    let host_id = ticket.addr.id;
    let connection = endpoint::dial(
        &node.endpoint,
        ticket.addr.clone(),
        proto::PAIR_ALPN,
        PEER_OFFLINE,
    )
    .await?;

    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .map_err(|e| format!("cannot open the pairing stream: {e}"))?;
    write_frame(
        &mut send,
        &PairHello {
            proto: proto::PROTO_VERSION,
            token: ticket.token,
            device: local_device,
        },
    )
    .await?;
    let _ = send.finish();

    let ack = tokio::time::timeout(HANDSHAKE_TIMEOUT, read_frame::<PairAck>(&mut recv))
        .await
        .map_err(|_| "the peer did not answer the pairing handshake".to_string())??;
    match ack {
        PairAck::Ok { proto: version, device } if version == proto::PROTO_VERSION => {
            Ok(PendingPair {
                node_id: host_id.to_string(),
                device: proto::sanitize_device(&device),
                fingerprint: peers::fingerprint(&node.endpoint.id(), &host_id),
                role: "guest".to_string(),
                created_at: now_unix(),
            })
        }
        PairAck::Ok { .. } => Err("unsupported protocol version".to_string()),
        PairAck::Denied(reason) => Err(reason),
    }
}

/// Fetch an artifact by content address into `remote/cache/<hash><ext>`,
/// verified by the blob protocol. Content-addressed: a hash (+ extension)
/// already in the cache is a hit and costs no network at all.
///
/// `ext` is the source path's extension, WITH its leading dot (e.g. `.png`),
/// or empty. It rides along purely so the local reader can dispatch on it —
/// `reader.rs`'s raster-image detection (and nothing else in the read path)
/// keys off the file's extension, and a bare content hash has none. Same
/// hash + same extension still hits the cache; only a hash fetched under two
/// different extensions (a legitimate but rare rename-with-same-bytes case)
/// costs a second copy.
pub async fn fetch_into_cache(
    node: &RemoteNode,
    addr: EndpointAddr,
    hash_hex: &str,
    ext: &str,
    cache_dir: &Path,
) -> Result<PathBuf, String> {
    // Length guard first: `Hash::from_str` treats other lengths as base32 and
    // can panic on malformed input, and this string arrives over the wire.
    if hash_hex.len() != 64 {
        return Err("malformed content address".to_string());
    }
    let hash: Hash = hash_hex.parse().map_err(|_| "malformed content address".to_string())?;
    let target = cache_dir.join(format!("{hash_hex}{ext}"));
    if target.is_file() {
        return Ok(target);
    }
    std::fs::create_dir_all(cache_dir)
        .map_err(|e| format!("cannot prepare the cache folder {}: {e}", cache_dir.display()))?;

    let connection =
        endpoint::dial(&node.endpoint, addr, iroh_blobs::ALPN, PEER_OFFLINE).await?;

    // Same staging, verification and one-cleanup discipline as a Beam
    // receive — it IS that routine. Only the final move differs: the cache
    // name is the content address, so the target is known before the bytes are.
    let partial = cache_dir.join(crate::beam::partial_name(hash_hex, ext));
    let (_written, landed) = crate::beam::download_to(
        connection,
        hash,
        hash_hex,
        &partial,
        |partial| {
            std::fs::rename(partial, &target)
                .map_err(|e| format!("cannot move the fetched artifact into place: {e}"))?;
            Ok(target.clone())
        },
        &mut |_, _| {},
    )
    .await?;
    Ok(landed)
}

// ── Framing over a QUIC stream ─────────────────────────────────────────────

async fn write_frame<T: serde::Serialize>(send: &mut SendStream, value: &T) -> Result<(), String> {
    let bytes = proto::encode_frame(value)?;
    send.write_all(&bytes).await.map_err(|e| format!("cannot write frame: {e}"))
}

async fn read_frame<T: serde::de::DeserializeOwned>(recv: &mut RecvStream) -> Result<T, String> {
    let mut prefix = [0u8; 4];
    recv.read_exact(&mut prefix)
        .await
        .map_err(|e| format!("stream ended: {e}"))?;
    let len = proto::frame_len(prefix)?;
    let mut body = vec![0u8; len];
    recv.read_exact(&mut body)
        .await
        .map_err(|e| format!("stream ended: {e}"))?;
    proto::decode_frame(&body)
}

fn refused(reason: &'static str) -> AcceptError {
    AcceptError::from_err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, reason))
}

fn refused_owned(reason: String) -> AcceptError {
    AcceptError::from_err(std::io::Error::other(reason))
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::SecretKey;

    fn id(seed: u8) -> EndpointId {
        SecretKey::from_bytes(&[seed; 32]).public()
    }

    fn tab(path: &str, active: bool) -> TabEntry {
        TabEntry { path: path.to_string(), active }
    }

    fn state(dir: &tempfile::TempDir, roots: RootSet) -> Arc<ScopeState> {
        state_with(dir, roots, Arc::new(crate::host::EmptyCatalog))
    }

    fn state_with(
        dir: &tempfile::TempDir,
        roots: RootSet,
        catalog: Arc<dyn HostCatalog>,
    ) -> Arc<ScopeState> {
        Arc::new(ScopeState::new(
            Arc::new(PeerStore::load(dir.path())),
            Arc::new(Pairing::new()),
            Arc::new(TabsCache::new()),
            roots,
            "Test Mac".to_string(),
            catalog,
            |_| {},
        ))
    }

    /// A catalog with fixed contents — the seam the app fills with its
    /// bookmarks and recents stores.
    struct FixedCatalog {
        bookmarks: Vec<PathBuf>,
        recents: Vec<PathBuf>,
    }

    impl HostCatalog for FixedCatalog {
        fn bookmarks(&self) -> Vec<PathBuf> {
            self.bookmarks.clone()
        }
        fn recents(&self) -> Vec<PathBuf> {
            self.recents.clone()
        }
    }

    // ── Tab diffing ────────────────────────────────────────────────────────

    #[test]
    fn opening_a_tab_derives_one_open_and_one_activation() {
        let events = diff_tabs(&[], &[tab("/a.html", true)]);
        assert_eq!(
            events,
            vec![
                Event::TabOpened { path: "/a.html".into() },
                Event::TabActivated { path: "/a.html".into() },
            ]
        );
    }

    #[test]
    fn closing_a_tab_derives_a_close() {
        let events = diff_tabs(&[tab("/a.html", true), tab("/b.html", false)], &[tab("/b.html", true)]);
        assert_eq!(
            events,
            vec![
                Event::TabClosed { path: "/a.html".into() },
                Event::TabActivated { path: "/b.html".into() },
            ]
        );
    }

    #[test]
    fn switching_tabs_derives_only_an_activation() {
        let prev = vec![tab("/a.html", true), tab("/b.html", false)];
        let next = vec![tab("/a.html", false), tab("/b.html", true)];
        assert_eq!(diff_tabs(&prev, &next), vec![Event::TabActivated { path: "/b.html".into() }]);
    }

    #[test]
    fn an_unchanged_list_derives_nothing() {
        let tabs = vec![tab("/a.html", true), tab("/b.html", false)];
        assert!(diff_tabs(&tabs, &tabs).is_empty());
    }

    #[test]
    fn the_same_file_in_two_tabs_is_one_wire_entry() {
        let prev = vec![tab("/a.html", true)];
        let next = vec![tab("/a.html", true), tab("/a.html", false)];
        assert!(diff_tabs(&prev, &next).is_empty(), "a duplicate path is not a new artifact");
        // Closing one of the two leaves the path open.
        assert!(diff_tabs(&next, &prev).is_empty());
    }

    #[test]
    fn empty_paths_and_the_start_page_are_not_artifacts() {
        assert!(diff_tabs(&[], &[tab("", true)]).is_empty());
    }

    #[test]
    fn the_cache_publishes_and_diffs_in_one_step() {
        let cache = TabsCache::new();
        assert_eq!(cache.publish(vec![tab("/a.html", true)]).len(), 2);
        assert_eq!(cache.list(), vec![tab("/a.html", true)]);
        assert!(cache.publish(vec![tab("/a.html", true)]).is_empty());
    }

    #[test]
    fn publishing_gates_and_canonicalizes_before_anything_reaches_the_wire() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        let inside = root.join("a.html");
        std::fs::write(&inside, "x").unwrap();
        let outside = tempfile::TempDir::new().unwrap();
        let out_file = outside.path().join("b.html");
        std::fs::write(&out_file, "x").unwrap();
        let roots = RootSet::new(vec![root.clone()]);

        let published = canonical_tabs(
            vec![
                tab(&inside.to_string_lossy(), true),
                // An out-of-root tab is legitimate locally and must never be
                // announced to a peer.
                tab(&out_file.to_string_lossy(), false),
                // The start page and hostile shapes drop out here.
                tab("", false),
                tab("relative.html", false),
                tab("/tmp/a\0b", false),
            ],
            &roots,
        );
        assert_eq!(published.len(), 1);
        assert_eq!(published[0].path, inside.canonicalize().unwrap().to_string_lossy());
        assert!(published[0].active);
    }

    #[test]
    fn published_paths_are_canonical_so_watcher_events_match_them() {
        let dir = tempfile::TempDir::new().unwrap();
        let st = state(&dir, RootSet::new(vec![dir.path().to_path_buf()]));
        let file = dir.path().join("a.html");
        std::fs::write(&file, "x").unwrap();
        let events = st.publish_tabs(vec![tab(&file.to_string_lossy(), true)]);
        let canonical = file.canonicalize().unwrap().to_string_lossy().into_owned();
        assert_eq!(events[0], Event::TabOpened { path: canonical.clone() });
        assert_eq!(st.tabs.list()[0].path, canonical);
    }

    // ── The path gate ──────────────────────────────────────────────────────

    #[test]
    fn traversal_and_out_of_root_paths_are_refused_identically() {
        let dir = tempfile::TempDir::new().unwrap();
        let outside = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("a.html"), "x").unwrap();
        let secret = outside.path().join("secret.txt");
        std::fs::write(&secret, "x").unwrap();

        let st = state(&dir, RootSet::new(vec![root.clone()]));

        // In-root file passes.
        assert!(st.gate_path(&root.join("a.html").to_string_lossy()).is_ok());

        // A traversal that climbs out of the root is refused AFTER
        // canonicalization — the string tricks do not survive the gate.
        let traversal = format!("{}/../{}", root.display(), secret.strip_prefix(outside.path()).unwrap().display());
        assert_eq!(st.gate_path(&traversal).unwrap_err(), DENIED);
        assert_eq!(st.gate_path(&secret.to_string_lossy()).unwrap_err(), DENIED);
        // A missing file gets the SAME wording as a forbidden one.
        assert_eq!(st.gate_path(&format!("{}/nope.html", root.display())).unwrap_err(), DENIED);
    }

    #[test]
    fn relative_paths_and_nul_bytes_never_reach_the_filesystem() {
        let dir = tempfile::TempDir::new().unwrap();
        let st = state(&dir, RootSet::new(vec![dir.path().to_path_buf()]));
        assert_eq!(st.gate_path("relative/path.html").unwrap_err(), DENIED);
        assert_eq!(st.gate_path("").unwrap_err(), DENIED);
        assert_eq!(st.gate_path("/tmp/a\0b").unwrap_err(), DENIED);
    }

    #[test]
    fn an_empty_root_set_admits_nothing() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("a.html");
        std::fs::write(&file, "x").unwrap();
        let st = state(&dir, RootSet::empty());
        assert_eq!(st.gate_path(&file.to_string_lossy()).unwrap_err(), DENIED);
    }

    #[test]
    fn a_view_open_peer_only_reaches_the_artifacts_it_was_told_about() {
        let dir = tempfile::TempDir::new().unwrap();
        let open = dir.path().join("open.html");
        let other = dir.path().join("other.html");
        std::fs::write(&open, "x").unwrap();
        std::fs::write(&other, "x").unwrap();
        let st = state(&dir, RootSet::new(vec![dir.path().to_path_buf()]));
        st.tabs.publish(vec![tab(&open.to_string_lossy(), true)]);

        let viewer = Peer {
            node_id: id(1).to_string(),
            device: "d".into(),
            scope: Scope::ViewOpen,
            paired_at: 0,
            last_seen: 0,
        };
        let browser = Peer { scope: Scope::Browse, ..viewer.clone() };

        assert!(st.gate_for(&viewer, &open.to_string_lossy()).is_ok(), "the open tab is fetchable");
        assert_eq!(
            st.gate_for(&viewer, &other.to_string_lossy()).unwrap_err(),
            DENIED,
            "an in-root file it was never told about stays invisible"
        );
        // Browse widens to everything the RootSet admits.
        assert!(st.gate_for(&browser, &other.to_string_lossy()).is_ok());
    }

    #[test]
    fn the_host_catalog_is_what_widens_a_view_open_peer_and_nothing_else() {
        let dir = tempfile::TempDir::new().unwrap();
        let starred = dir.path().join("starred.html");
        let opened = dir.path().join("opened.html");
        let never = dir.path().join("never.html");
        for f in [&starred, &opened, &never] {
            std::fs::write(f, "x").unwrap();
        }
        let st = state_with(
            &dir,
            RootSet::new(vec![dir.path().to_path_buf()]),
            Arc::new(FixedCatalog {
                bookmarks: vec![starred.clone()],
                recents: vec![opened.clone()],
            }),
        );
        let viewer = Peer {
            node_id: id(1).to_string(),
            device: "d".into(),
            scope: Scope::ViewOpen,
            paired_at: 0,
            last_seen: 0,
        };

        assert!(st.gate_for(&viewer, &starred.to_string_lossy()).is_ok());
        assert!(st.gate_for(&viewer, &opened.to_string_lossy()).is_ok());
        // A catalog can only ever narrow further: an in-root file it does not
        // report stays invisible.
        assert_eq!(st.gate_for(&viewer, &never.to_string_lossy()).unwrap_err(), DENIED);
    }

    #[test]
    fn a_catalog_entry_out_of_root_never_reaches_the_wire() {
        // The catalog is app state, not a capability: the RootSet gate runs
        // on every entry it reports.
        let dir = tempfile::TempDir::new().unwrap();
        let outside = tempfile::TempDir::new().unwrap();
        let leaked = outside.path().join("secret.txt");
        std::fs::write(&leaked, "x").unwrap();
        let st = state_with(
            &dir,
            RootSet::new(vec![dir.path().to_path_buf()]),
            Arc::new(FixedCatalog { bookmarks: vec![leaked.clone()], recents: Vec::new() }),
        );
        let viewer = Peer {
            node_id: id(1).to_string(),
            device: "d".into(),
            scope: Scope::ViewOpen,
            paired_at: 0,
            last_seen: 0,
        };
        assert_eq!(st.gate_for(&viewer, &leaked.to_string_lossy()).unwrap_err(), DENIED);
    }

    // ── Session caps ───────────────────────────────────────────────────────

    /// A registrable session with a live channel. The receiver is returned so
    /// the caller keeps it alive — a dropped one closes the sender.
    fn session(id: u64, peer: &str) -> (Session, mpsc::Receiver<Frame>) {
        let (tx, rx) = mpsc::channel(1);
        let session = Session {
            id,
            peer: peer.to_string(),
            tx,
            handles: Arc::new(SessionHandles::new()),
        };
        (session, rx)
    }

    #[test]
    fn a_peer_cannot_hold_more_than_the_session_cap() {
        let dir = tempfile::TempDir::new().unwrap();
        let st = state(&dir, RootSet::empty());
        let mut ids = Vec::new();
        let mut keep_alive = Vec::new();
        for n in 0..MAX_SESSIONS_PER_PEER {
            let (session, rx) = session(n as u64, "nodeA");
            keep_alive.push(rx);
            ids.push(st.register(session).expect("under the cap"));
        }
        let (over_cap, _rx) = session(99, "nodeA");
        assert!(st.register(over_cap).is_err(), "the cap refuses the next session");

        // Another peer is unaffected — the cap is per peer.
        let (other_peer, _rx) = session(100, "nodeB");
        assert!(st.register(other_peer).is_ok());

        st.unregister(ids[0]);
        let (replacement, _rx) = session(101, "nodeA");
        assert!(st.register(replacement).is_ok());
    }

    #[test]
    fn only_a_subscribed_session_that_fetched_a_path_makes_it_worth_re_hashing() {
        let dir = tempfile::TempDir::new().unwrap();
        let st = state(&dir, RootSet::empty());
        let watched = PathBuf::from("/w/a.html");

        let (session, _rx) = session(1, "nodeA");
        let handles = session.handles.clone();
        st.register(session).unwrap();

        // Fetched but not subscribed: the client is not listening for events.
        handles.interest.lock().unwrap().insert(watched.clone());
        assert!(!st.any_session_holds(&watched));

        // Subscribed and holding it: this is the one case worth the re-read.
        handles.subscribed.store(true, Ordering::SeqCst);
        assert!(st.any_session_holds(&watched));
        assert!(!st.any_session_holds(Path::new("/w/never-fetched.html")));
    }

    // ── Grants ─────────────────────────────────────────────────────────────

    #[test]
    fn a_grant_is_peer_locked_and_expires() {
        let grants = Grants::new();
        let hash = Hash::new(b"artifact");
        assert!(grants.insert(hash, id(1), Tag::from("t1")).is_none());

        assert!(grants.admit(&hash, Some(id(1)), true));
        assert!(!grants.admit(&hash, Some(id(2)), true), "another peer holds no grant");
        assert!(!grants.admit(&hash, None, true), "an anonymous fetch holds no grant");
        assert!(!grants.admit(&hash, Some(id(1)), false), "only plain blob requests");
        assert!(!grants.admit(&Hash::new(b"other"), Some(id(1)), true));

        // A second peer asking for the same artifact shares the entry, and
        // the redundant staging tag comes back for deletion.
        assert!(grants.insert(hash, id(2), Tag::from("t2")).is_some());
        assert!(grants.admit(&hash, Some(id(2)), true));
        assert_eq!(grants.len(), 1);
    }

    #[test]
    fn expired_grants_are_swept_with_their_tags() {
        let grants = Grants::new();
        let hash = Hash::new(b"artifact");
        grants.insert(hash, id(1), Tag::from("t1"));
        // Sweep with a clock far past the TTL.
        let tags = grants.take_expired(now_unix() + GRANT_TTL_SECS + 1);
        assert_eq!(tags.len(), 1);
        assert!(!grants.admit(&hash, Some(id(1)), true));
    }

    #[test]
    fn revoking_a_peer_drops_the_grants_only_it_held() {
        let grants = Grants::new();
        let solo = Hash::new(b"solo");
        let shared = Hash::new(b"shared");
        grants.insert(solo, id(1), Tag::from("t1"));
        grants.insert(shared, id(1), Tag::from("t2"));
        grants.insert(shared, id(2), Tag::from("t3"));

        let orphaned = grants.revoke_peer(&id(1));
        assert_eq!(orphaned.len(), 1, "only the grant nobody else holds is collectable");
        assert!(!grants.admit(&solo, Some(id(1)), true));
        assert!(!grants.admit(&shared, Some(id(1)), true), "the revoked peer loses the shared grant");
        assert!(grants.admit(&shared, Some(id(2)), true), "the other peer keeps it");
    }

    // ── Tree listing policy ────────────────────────────────────────────────

    #[test]
    fn tree_listing_matches_the_quick_open_walker_policy() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join("node_modules")).unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("a.html"), "x").unwrap();
        std::fs::write(dir.path().join(".env"), "x").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.path().join("a.html"), dir.path().join("link.html")).unwrap();

        let names: Vec<String> = list_tree(dir.path()).unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, vec!["src", ".env", "a.html"], "dirs first, ignored/hidden dirs and symlinks out");
    }

    #[test]
    fn listing_a_file_instead_of_a_directory_is_refused() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("a.html");
        std::fs::write(&file, "x").unwrap();
        assert_eq!(list_tree(&file).unwrap_err(), DENIED);
    }

}
