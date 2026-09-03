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

use iroh::endpoint::{Connection, ConnectionError, RecvStream, SendStream, VarInt};
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh::{Endpoint, EndpointAddr, EndpointId};
use iroh_blobs::api::Tag;
use iroh_blobs::store::fs::FsStore;
use iroh_blobs::{BlobFormat, Hash, HashAndFormat};
use tokio::sync::{mpsc, oneshot};

use crate::beam::{delete_tags, parse_hash};
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

/// Display cap on a close reason a peer wrote. Every reason this crate sends
/// is three or four words; the bound is here because the bytes belong to
/// another machine, and they end up in a sentence a human reads.
const MAX_CLOSE_REASON_CHARS: usize = 100;

/// Subject of every failed dial this module makes — a session, a pairing
/// handshake, a cache fetch. `endpoint::dial` appends the cause, and a
/// timeout reads the same as a refusal because both mean the same thing to
/// the person looking at the screen.
const PEER_OFFLINE: &str = "peer offline — could not reach it";

// ── Grants: peer-locked blob capabilities ──────────────────────────────────

struct Grant {
    /// Peers allowed to fetch this hash, each with the moment ITS OWN
    /// capability lapses. A grant is NOT a beam ticket: possession of the
    /// hash is not enough, the fetching NodeId must be one that asked for the
    /// artifact through a scoped session.
    ///
    /// The clock is per peer because unrelated callers share one hash. A
    /// queued send re-grants the same bytes to its receiving device once per
    /// attempt that reaches a push: `grant_pinned` runs inside
    /// `push_staged_via`, so a device that is asleep re-grants nothing — its
    /// `dial_session` fails first — while a device that answers and then
    /// refuses the bytes re-grants at the drain's own cadence, the retry
    /// ladder from 60 s up to 10 min, for as long as that record lives.
    /// Another peer may hold a browse grant on the same file the whole time.
    /// One grant-wide clock lets those re-grants renew the browse capability
    /// for the record's week, so bytes a peer asked for once stay fetchable
    /// long after the hour it was given.
    peers: HashMap<EndpointId, u64>,
    /// The staging tag this grant OWNS, or `None` when the bytes are pinned
    /// by somebody with a longer memory than a grant has. `None` means
    /// "expiring me must unpin nothing", and the case that needs it is a send
    /// accepted for a peer that is asleep: those bytes are pinned by a durable
    /// tag the sender keeps for up to a week, while a grant lives one hour and
    /// is re-minted per delivery attempt. Let such a grant own that tag and
    /// the next `stage_for_peer` sweep unpins a file the user was promised,
    /// leaving a delivery that pushes a hash with no bytes behind it.
    ///
    /// `None` is not permanent: a grant that owns nothing ADOPTS the next
    /// staging tag minted for the same content — see `insert`. That tag is a
    /// fresh one this registry made, never the spool's `outbox/<id>`, so
    /// releasing it can never unpin a pending send.
    tag: Option<Tag>,
}

/// A grant for one peer, its hour already started. The two minting call
/// sites — `Grants::insert` and `Grants::grant_pinned` — differ only in
/// whether they own a tag, and the peer's expiry has to be the SAME
/// expression at both: a grant minted with a clock the gate does not use is a
/// capability that lapses at the wrong moment.
fn new_grant(peer: EndpointId, tag: Option<Tag>) -> Grant {
    Grant { peers: HashMap::from([(peer, now_unix() + GRANT_TTL_SECS)]), tag }
}

impl Grant {
    /// Let one more peer fetch these bytes, and restart that peer's clock — a
    /// peer that just asked is about to fetch. Every other peer on this hash
    /// keeps the expiry it earned, so one peer's traffic cannot extend
    /// another's capability.
    fn admit(&mut self, peer: EndpointId) {
        self.peers.insert(peer, now_unix() + GRANT_TTL_SECS);
    }

    /// May this peer still fetch? Unknown and lapsed answer the same.
    fn admits(&self, peer: &EndpointId, now: u64) -> bool {
        self.peers.get(peer).is_some_and(|expires_at| *expires_at > now)
    }

    /// Drop the peers whose hour ran out, and report whether anybody is left.
    /// A grant survives while ONE peer still holds it: dropping the whole
    /// entry on the first lapse would take the pin with it and free bytes a
    /// live peer is still allowed to fetch.
    fn retain_live(&mut self, now: u64) -> bool {
        self.peers.retain(|_, expires_at| *expires_at > now);
        !self.peers.is_empty()
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
    /// redundant staging tag when this content was already staged AND the
    /// grant already owns a pin of its own — leaving it would pin a second
    /// copy of the bytes that nothing can reach.
    ///
    /// A grant that owns NO tag ADOPTS the fresh one instead, and that is the
    /// whole reason this is not one arm. A tag-less grant defers to another
    /// owner, and that owner is the spool record, which is unpinned the
    /// moment the delivery lands. Hand the fresh tag back after that and the
    /// content has no root left at all: the collector is free to take the
    /// bytes while the peer this grant was just minted for is fetching them.
    /// Adoption takes nothing the spool needs — the adopted tag is the fresh
    /// staging one, never `outbox/<id>` — so `take_expired` and `revoke_peer`
    /// release it on the grant's own clock and a pending send keeps its pin.
    fn insert(&self, hash: Hash, peer: EndpointId, tag: Tag) -> Option<Tag> {
        let mut map = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        match map.get_mut(&hash) {
            Some(existing) => {
                existing.admit(peer);
                match existing.tag {
                    Some(_) => Some(tag),
                    None => {
                        existing.tag = Some(tag);
                        None
                    }
                }
            }
            None => {
                map.insert(hash, new_grant(peer, Some(tag)));
                None
            }
        }
    }

    /// `insert` for bytes this grant does not own: admit an existing grant, or
    /// mint one that pins nothing. `push_staged_via` calls it once per
    /// delivery attempt — a grant is in-memory and lives `GRANT_TTL_SECS`,
    /// so nothing durable may ever store one and replay it later.
    ///
    /// The concrete failure the absent tag prevents: `stage_for_peer` runs
    /// `take_expired` on EVERY invocation and deletes every tag it returns.
    /// Let this grant own the durable tag that holds a pending send, and the
    /// first unrelated staging an hour later unpins the user's file, after
    /// the send was reported as accepted.
    ///
    /// `admit` touches ONE peer's clock, which is what makes this safe to
    /// call on a retry loop: a record whose device answers the dial and then
    /// refuses the bytes re-grants the same hash once per drain pass for up
    /// to a week, and it must not carry a browse capability another peer
    /// earned along with it.
    fn grant_pinned(&self, hash: Hash, peer: EndpointId) {
        let mut map = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        match map.get_mut(&hash) {
            Some(existing) => existing.admit(peer),
            None => {
                map.insert(hash, new_grant(peer, None));
            }
        }
    }

    /// Widen an EXISTING grant to one more peer, refreshing that peer's hour.
    /// `false` when nothing is staged under `hash`: a grant with no bytes
    /// behind it is a dangling capability, so the caller must stage first.
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
            Some(grant) => grant.admits(&peer, now_unix()),
            None => false,
        }
    }

    /// Drop expired grants, returning the staging tags they OWNED for cleanup.
    ///
    /// Every lapsed peer leaves its grant, so `admit` answers false the moment
    /// that peer's TTL passes. Only the tag collection is conditional. A
    /// pinned grant that outlived its expiry would be an immortal fetch
    /// capability, which is the opposite of what the option is for — it holds
    /// back a DELETE, never a revocation.
    ///
    /// An entry goes only when its LAST peer lapses, because the tag it owns
    /// is what keeps the bytes on disk for the peers that are still inside
    /// their hour.
    fn take_expired(&self, now: u64) -> Vec<Tag> {
        let mut map = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let mut expired = Vec::new();
        map.retain(|_, grant| {
            if grant.retain_live(now) {
                return true;
            }
            if let Some(tag) = &grant.tag {
                expired.push(tag.clone());
            }
            false
        });
        expired
    }

    /// Revoke every grant held by one peer — what unpairing must do to bytes
    /// already staged for it. Returns the tags of grants nobody holds anymore.
    ///
    /// Same split as `take_expired`: the grant goes whatever its tag is, so
    /// an unpaired peer loses the capability at once, and only a grant that
    /// OWNED its staging tag hands one back to delete.
    pub fn revoke_peer(&self, peer: &EndpointId) -> Vec<Tag> {
        let mut map = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let mut orphaned = Vec::new();
        map.retain(|_, grant| {
            grant.peers.remove(peer);
            if grant.peers.is_empty() {
                if let Some(tag) = &grant.tag {
                    orphaned.push(tag.clone());
                }
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
        // The interest set only grows, so a view-open peer that CLOSED the tab
        // still `holds` the path — but the path has left its published set.
        // Re-gate each holder against its current scope so we stop re-staging
        // and pushing a new content address the peer may no longer fetch.
        let wire = path.to_string_lossy();
        let sessions = self.state.sessions();
        sessions
            .iter()
            .filter(|s| s.handles.is_subscribed() && s.handles.holds(path))
            .filter(|s| {
                self.state
                    .peers
                    .get(&s.peer)
                    .is_some_and(|peer| self.state.gate_for(&peer, &wire).is_ok())
            })
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
        //
        // This runs AFTER the allowlist check at the top, with an await in
        // between (the peer chooses when to send `Hello`, up to
        // HANDSHAKE_TIMEOUT). `refresh_device` never inserts, so a peer the
        // operator revoked inside that window cannot write itself back into
        // the store — and we refuse it here rather than registering a session
        // it would otherwise keep. A write failure is not a reason to drop a
        // connection the allowlist already admitted, so only `Ok(false)`,
        // "the peer is gone", closes it.
        match self.state.peers.refresh_device(&node_id, &device) {
            Ok(true) => {}
            Ok(false) => {
                connection.close(VarInt::from_u32(CLOSE_REFUSED), b"not a paired peer");
                return Err(refused("peer was revoked during the handshake"));
            }
            Err(e) => eprintln!("vlerv: remote: cannot persist peers.json: {e}"),
        }

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

        // Announced only once the ack is queued, and that placement is the
        // whole meaning of the signal: everything that can still refuse this
        // connection — the allowlist, the protocol version, the revocation
        // window around `refresh_device`, the session cap — has already run,
        // so a host acting on it is acting on a peer that HOLDS a session.
        //
        // The concrete consumer is the sender's queue: a device that dials in
        // is reachable now, and a send accepted for it while it was asleep
        // should go out on this connection rather than wait out a retry
        // ladder measured in minutes. The signal is fire-and-forget by
        // contract, so a sink that does work must not block this path.
        self.state.signal(HostSignal::PeerConnected {
            peer: node_id.clone(),
            device,
            scope: peer.scope.as_str().to_string(),
        });

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

/// Why a dial did not end in a session, split by what the caller may do next.
///
/// The classification is produced HERE, where the cause is known, and never
/// re-derived from the message: `McpCore::session` wraps a failed dial as
/// `"{label} is not reachable: {e}. …"`, which buries `PEER_OFFLINE` in the
/// middle of the sentence, so no `starts_with` sniff downstream can ever tell
/// a sleeping phone from a host that answered and refused.
///
/// `Display` yields the inner string verbatim on both variants, so every
/// sentence a user already sees stays byte-for-byte what it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectError {
    /// The peer was never spoken to: the dial itself failed (`endpoint::dial`
    /// already appends the cause to `PEER_OFFLINE`), or it accepted the
    /// connection and never answered the handshake. Only this means "asleep".
    Unreachable(String),
    /// The peer, or its stream, answered — a refusal, a version mismatch, a
    /// frame that is not an ack. A retry later cannot change any of these.
    Refused(String),
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectError::Unreachable(reason) | ConnectError::Refused(reason) => {
                f.write_str(reason)
            }
        }
    }
}

/// What the peer SAID when it closed, when it closed on purpose.
///
/// `ScopeServer::accept` turns a dial from a device that has unpaired this
/// node away by closing with `CLOSE_REFUSED` and the reason
/// `not a paired peer` — before `accept_bi`, so no application frame is ever
/// written and `classify_ack` never sees the `Res::Denied` that would carry
/// it. The close bytes DO arrive here. Without this, every stream call on the
/// dead connection reports only `connection lost`, and the sentence built
/// from that sends the user to check a network that is working, about a
/// device that is awake and refusing this node on purpose.
///
/// The text is written by another machine, so it passes the crate's one strip
/// set before it reaches a human, exactly like a device name. Nothing
/// branches on it: it is DISPLAYED, never matched. A refusal is already known
/// to be a refusal from the variant, which is decided here rather than read
/// back out of the string.
fn stated_refusal(connection: &Connection) -> Option<String> {
    // Every other variant is the transport's own account of a connection that
    // died, which `read_frame` already renders as well as this could. Only an
    // application close carries a sentence some peer chose to write.
    let ConnectionError::ApplicationClosed(close) = connection.close_reason()? else {
        return None;
    };
    let said = proto::strip_spoofing_chars(
        &String::from_utf8_lossy(&close.reason),
        MAX_CLOSE_REASON_CHARS,
    );
    let said = said.trim();
    (!said.is_empty()).then(|| said.to_string())
}

/// One post-dial failure, told in the peer's own words when it left any and
/// in the transport's when it did not.
fn refusal(connection: &Connection, transport: String) -> ConnectError {
    ConnectError::Refused(stated_refusal(connection).unwrap_or(transport))
}

/// Why a push did not land, split the same way and for the same reason.
///
/// `Local` and `Denied` are answers about THIS request and repeating them
/// changes nothing; only `Transport` can mean the session died under a peer
/// that is merely asleep, and even then only a fresh dial can say which it
/// was — `is_closed()` cannot, because `writer_closed` is stored after
/// `send.finish()` and `reader_closed` after the read loop breaks, so both
/// still read false while `request` is already returning `Err`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushFailure {
    /// This side refused before anything went out: the offer policy rejected
    /// the path, the peer id is malformed, or staging failed.
    Local(String),
    /// The session is gone — the request never reached the host, or its
    /// answer never came back.
    Transport(String),
    /// The host answered, and its answer was not `Res::Pushed`.
    Denied(String),
}

impl std::fmt::Display for PushFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PushFailure::Local(reason)
            | PushFailure::Transport(reason)
            | PushFailure::Denied(reason) => f.write_str(reason),
        }
    }
}

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
    ) -> Result<Arc<Self>, ConnectError> {
        let peer = addr.id.to_string();
        // The two causes that mean the peer is asleep, and the only two: the
        // dial never reached it, or it took the connection and said nothing.
        let connection = endpoint::dial(&node.endpoint, addr, proto::SCOPE_ALPN, PEER_OFFLINE)
            .await
            .map_err(ConnectError::Unreachable)?;

        // Everything from here on happens on a connection the peer accepted,
        // so it is a refusal even when it reads like a network fault. Calling
        // any of it unreachable would let a broken stream look like a nap.
        //
        // Each of the three asks the connection for a stated reason first: a
        // peer that closed on purpose left one, and it is the only text on
        // this path that says WHY. The transport's own wording is the
        // fallback, so a connection that merely broke reads as it always did.
        let (mut send, mut recv) = connection
            .open_bi()
            .await
            .map_err(|e| refusal(&connection, format!("cannot open the session stream: {e}")))?;

        write_frame(
            &mut send,
            &Req::Hello { proto: proto::PROTO_VERSION, device },
        )
        .await
        .map_err(|e| refusal(&connection, e))?;

        // The timeout arm stays UNREACHABLE and asks nothing: a peer that
        // took the connection and then said nothing has closed nothing, so
        // there is no reason to read, and it is the one shape here that
        // really does mean asleep.
        let ack = tokio::time::timeout(HANDSHAKE_TIMEOUT, read_frame::<Frame>(&mut recv))
            .await
            .map_err(|_| {
                ConnectError::Unreachable("the peer did not answer the handshake".to_string())
            })?
            .map_err(|e| refusal(&connection, e))?;
        let (host_device, scope) = classify_ack(ack)?;

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
    ) -> Result<PushedArtifact, PushFailure> {
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
    ) -> Result<PushedArtifact, PushFailure> {
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
    ) -> Result<PushedArtifact, PushFailure> {
        // Everything up to the frame is this side's own refusal: nothing left
        // the machine, so nothing about the peer is known yet.
        let cand = crate::beam::resolve_offerable(path, roots).map_err(PushFailure::Local)?;
        let host: EndpointId = self
            .peer
            .parse()
            .map_err(|_| PushFailure::Local("malformed peer id".to_string()))?;

        // Peer-locked, through the same pairing the host uses: possession of
        // the ticket is NOT enough on this side either, because the local
        // request gate admits the hash only for the host we are pushing to.
        let staged = stage_for_peer(&self.store, &self.grants, &cand.canonical, host)
            .await
            .map_err(PushFailure::Local)?;

        let hash = staged.hash.to_string();
        let ticket =
            iroh_blobs::ticket::BlobTicket::new(addr, staged.hash, staged.format).to_string();
        // `request_as` is not used here: it collapses a dead session and a
        // refusal from the host into one string, and those two are exactly
        // what the caller must tell apart.
        let res = self
            .request(Req::PushArtifact {
                name: cand.name,
                size: cand.size,
                hash: hash.clone(),
                ticket,
            })
            .await
            .map_err(PushFailure::Transport)?;
        let (name, size) = match res {
            Res::Pushed { name, size } => (name, size),
            // A `Denied` and a frame that makes no sense are both the host's
            // answer to THIS request, and neither improves by being asked
            // again later; `unexpected` keeps the wording each one had.
            other => return Err(PushFailure::Denied(unexpected(other))),
        };
        Ok(PushedArtifact { name, size, hash })
    }

    /// Push bytes that are ALREADY in this instance's store, named by content
    /// address instead of by path — the replay half of `push_artifact`.
    ///
    /// No file is read here, deliberately: the bytes were captured when the
    /// user accepted the send, and by the time a sleeping peer answers, the
    /// source may have been rewritten or deleted. Re-reading it would deliver
    /// something the user never accepted, or nothing at all.
    pub async fn push_staged(
        &self,
        hash_hex: &str,
        name: &str,
        size: u64,
    ) -> Result<PushedArtifact, PushFailure> {
        // Before the wait, not after it: a malformed address is this side's
        // own error and must not cost ten seconds of discovery first.
        staged_hash(hash_hex)?;
        // The bounded wait `push_artifact` makes, for the same reason. Skip
        // it and the ticket carries direct addresses only, so a replayed item
        // lands on the same LAN and fails everywhere else with "sender
        // offline" — the exact failure the queue exists to remove.
        let _ = tokio::time::timeout(Duration::from_secs(10), self.endpoint.online()).await;
        self.push_staged_via(hash_hex, name, size, self.endpoint.addr()).await
    }

    /// `push_staged` with the call-back address pinned to one socket, named
    /// in plain `std` types — the same test seam `push_artifact_at` is.
    pub async fn push_staged_at(
        &self,
        hash_hex: &str,
        name: &str,
        size: u64,
        socket: std::net::SocketAddr,
    ) -> Result<PushedArtifact, PushFailure> {
        let addr = endpoint::addr_at_id(self.endpoint.id(), socket);
        self.push_staged_via(hash_hex, name, size, addr).await
    }

    /// `push_staged` with an explicit call-back address.
    ///
    /// The grant and the ticket are minted FRESH on every attempt, and
    /// neither is ever stored: a grant lives `GRANT_TTL_SECS` in memory while
    /// a queued record lives up to a week across restarts, and a ticket names
    /// addresses this process stops holding the moment it is restarted.
    pub async fn push_staged_via(
        &self,
        hash_hex: &str,
        name: &str,
        size: u64,
        addr: EndpointAddr,
    ) -> Result<PushedArtifact, PushFailure> {
        let hash = staged_hash(hash_hex)?;
        let host: EndpointId = self
            .peer
            .parse()
            .map_err(|_| PushFailure::Local("malformed peer id".to_string()))?;

        // Peer-locked exactly as a fresh push is, but pinning nothing: the
        // caller owns the tag that keeps these bytes alive, so this grant
        // must not hand one to the next `stage_for_peer` sweep to delete.
        self.grants.grant_pinned(hash, host);

        let ticket = iroh_blobs::ticket::BlobTicket::new(addr, hash, BlobFormat::Raw).to_string();
        // Same reason `push_artifact_via` avoids `request_as`: a dead session
        // and a refusal from the host are the two answers the caller must
        // tell apart, and that helper writes both as one string.
        let res = self
            .request(Req::PushArtifact {
                name: name.to_string(),
                size,
                hash: hash_hex.to_string(),
                ticket,
            })
            .await
            .map_err(PushFailure::Transport)?;
        let (name, size) = match res {
            Res::Pushed { name, size } => (name, size),
            other => return Err(PushFailure::Denied(unexpected(other))),
        };
        Ok(PushedArtifact { name, size, hash: hash_hex.to_string() })
    }
}

/// A content address that arrived from the spool, as the push path's own
/// error. A record file is editable by hand and survives a build change, so
/// this string is no more trusted than one off the wire — `parse_hash` holds
/// the reason the length is guarded before the parse.
///
/// Free function so the refusal is reachable without two endpoints and a
/// socket: a `ClientSession` cannot be built without both.
fn staged_hash(hash_hex: &str) -> Result<Hash, PushFailure> {
    parse_hash(hash_hex).ok_or_else(|| PushFailure::Local("malformed content address".to_string()))
}

/// The handshake answer, classified: the host's device and the scope it
/// granted, or the reason there is no session.
///
/// Split out of `connect` because this is the whole classification table for
/// an answering peer, and a table nobody can reach without two endpoints and
/// a socket is a table nobody checks. Every arm is `Refused`: the peer spoke,
/// so waiting for it to wake up is not the answer to any of them.
fn classify_ack(ack: Frame) -> Result<(String, String), ConnectError> {
    match ack {
        Frame::Res(Res::Hello(ack)) if ack.proto == proto::PROTO_VERSION => {
            Ok((ack.device, ack.scope))
        }
        Frame::Res(Res::Hello(_)) => {
            Err(ConnectError::Refused("unsupported protocol version".to_string()))
        }
        Frame::Res(Res::Denied(reason)) => Err(ConnectError::Refused(reason)),
        _ => Err(ConnectError::Refused(
            "the peer answered with an unexpected frame".to_string(),
        )),
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
    // The string arrives over the wire, so it is parsed the guarded way —
    // see `parse_hash`.
    let hash = parse_hash(hash_hex).ok_or_else(|| "malformed content address".to_string())?;
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
    use n0_future::StreamExt;

    fn id(seed: u8) -> EndpointId {
        SecretKey::from_bytes(&[seed; 32]).public()
    }

    fn tab(path: &str, active: bool) -> TabEntry {
        TabEntry { path: path.to_string(), active }
    }

    /// Is any tag still rooting these bytes? A blob no tag names is the
    /// collector's to take, whoever is fetching it.
    async fn rooted(node: &RemoteNode, hash: Hash) -> bool {
        let mut tags = node.store.tags().list().await.expect("list the tags");
        while let Some(info) = tags.next().await {
            if info.expect("read a tag").hash == hash {
                return true;
            }
        }
        false
    }

    /// Age one peer's capability past its hour. Every grant reads `now_unix()`
    /// itself, so a test that needs a lapsed capability writes the expiry.
    fn expire_peer(grants: &Grants, hash: &Hash, peer: EndpointId) {
        let mut map = grants.inner.lock().unwrap();
        let grant = map.get_mut(hash).expect("a grant on these bytes");
        *grant.peers.get_mut(&peer).expect("this peer holds it") = now_unix() - 1;
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
            last_ack_scope: None,
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
            last_ack_scope: None,
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
            last_ack_scope: None,
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

    #[test]
    fn an_expired_pinned_grant_is_refused_and_unpins_nothing() {
        let grants = Grants::new();
        let hash = Hash::new(b"a queued artifact");
        grants.grant_pinned(hash, id(1));
        assert!(grants.admit(&hash, Some(id(1)), true));

        // This is the sweep `stage_for_peer` runs on every single call, with
        // a clock past the TTL. Two things must be true at once, and they
        // pull in opposite directions: the CAPABILITY goes, because a fetch
        // nobody can revoke is worse than a lost delivery; and NO tag comes
        // back, because the spool record still names the tag that keeps these
        // bytes on disk for a phone that has not woken up yet.
        let tags = grants.take_expired(now_unix() + GRANT_TTL_SECS + 1);
        assert!(tags.is_empty(), "the spool owns this pin, so there is nothing here to delete");
        assert!(!grants.admit(&hash, Some(id(1)), true), "the capability expires all the same");
        assert_eq!(grants.len(), 0);
    }

    #[test]
    fn a_revoked_peers_pinned_grant_is_dropped_and_unpins_nothing() {
        let grants = Grants::new();
        let queued = Hash::new(b"a queued artifact");
        let asked_for = Hash::new(b"an artifact a peer asked for");
        grants.grant_pinned(queued, id(1));
        grants.insert(asked_for, id(1), Tag::from("t1"));

        let orphaned = grants.revoke_peer(&id(1));
        assert_eq!(orphaned, vec![Tag::from("t1")], "only a grant that owns its tag yields one");
        assert!(!grants.admit(&queued, Some(id(1)), true), "unpairing takes both capabilities");
        assert_eq!(grants.len(), 0, "and leaves neither entry behind");
    }

    #[test]
    fn staging_content_a_pinned_grant_already_covers_makes_that_grant_adopt_the_fresh_tag() {
        let grants = Grants::new();
        let hash = Hash::new(b"a queued artifact");
        grants.grant_pinned(hash, id(1));

        // The user queues a file for a sleeping phone, then a second peer
        // asks for the same file: `stage_for_peer` re-adds identical bytes
        // and gets a fresh tag for them. The grant owns no pin, because the
        // spool record does — and that record is unpinned the moment the
        // delivery lands. Hand the fresh tag back to be deleted after that
        // and the bytes have no root at all, so the collector may take them
        // while this second peer is fetching. The grant adopts it instead.
        assert_eq!(grants.insert(hash, id(2), Tag::from("t2")), None, "nothing to delete");
        assert!(grants.admit(&hash, Some(id(2)), true));
        assert_eq!(
            grants.take_expired(now_unix() + GRANT_TTL_SECS + 1),
            vec![Tag::from("t2")],
            "the adopted tag is released on the grant's own clock, and only it"
        );
    }

    #[test]
    fn a_retried_delivery_grant_never_postpones_the_hour_another_peer_earned() {
        let grants = Grants::new();
        let hash = Hash::new(b"one file, a browse peer and a queued send");
        // A peer asked for this artifact through a session an hour ago.
        grants.insert(hash, id(1), Tag::from("t1"));
        expire_peer(&grants, &hash, id(1));

        // A queued send of the SAME bytes to a second device re-grants the
        // fetch on every delivery attempt, once a minute for as long as that
        // device sleeps — up to a week. One clock for the whole grant renews
        // the first peer's capability with every one of those attempts, so a
        // fetch nobody asked to keep outlives its hour by days.
        grants.grant_pinned(hash, id(2));

        assert!(!grants.admit(&hash, Some(id(1)), true), "the browse peer's hour still ran out");
        assert!(grants.admit(&hash, Some(id(2)), true), "the device being retried can fetch");
        assert!(
            grants.take_expired(now_unix()).is_empty(),
            "and the pin stays while one live peer still holds the grant"
        );
        assert_eq!(grants.len(), 1);
    }

    #[tokio::test]
    async fn the_grant_sweep_never_deletes_a_tag_the_spool_still_needs() {
        // The two halves of the `Option<Tag>` change, proved against a real
        // store: an accepted send keeps its bytes for the week the record
        // lives, while the grant that lets the host fetch them lives an hour
        // and is re-minted per attempt.
        let state = tempfile::TempDir::new().unwrap();
        let work = tempfile::TempDir::new().unwrap();
        let queued = work.path().join("report.html");
        std::fs::write(&queued, "a file a sleeping phone was promised").unwrap();
        let asked_for = work.path().join("other.html");
        std::fs::write(&asked_for, "an artifact another peer asked for").unwrap();
        let node = endpoint::boot(&Dirs::new(state.path()), None, |_| {})
            .await
            .expect("boot");

        let record_id = "1700000000001-0000";
        let hash = crate::beam::stage_outbox(&node, &queued, record_id).await.expect("stage").hash;
        let staged: Hash = hash.parse().unwrap();
        node.grants.grant_pinned(staged, id(1));

        // The phone stays asleep past `GRANT_TTL_SECS`. The clock cannot be
        // moved for `stage_for_peer`, which sweeps with `now_unix()` itself,
        // so the grant is aged directly instead.
        expire_peer(&node.grants, &staged, id(1));

        // Any unrelated staging runs that sweep — this one is the host
        // handing a different artifact to a different peer.
        stage_for_peer(&node.store, &node.grants, &asked_for, id(2))
            .await
            .expect("an ordinary grant is unaffected");

        assert!(
            !node.grants.admit(&staged, Some(id(1)), true),
            "the expired capability is gone, exactly as an owned one would be"
        );
        assert!(
            node.store
                .tags()
                .get(crate::outbox::tag_name(record_id))
                .await
                .unwrap()
                .is_some(),
            "but the spool's pin survives the sweep that took the grant"
        );
        assert!(
            crate::beam::outbox_bytes_present(&node, &hash).await,
            "and so do the bytes the user was told would be delivered"
        );
    }

    #[tokio::test]
    async fn re_staging_a_delivered_file_leaves_its_bytes_a_root_of_their_own() {
        // The same two owners against a real store, one step later in the
        // life of a send. The record owns the pin only until the delivery
        // lands, and completing it unpins there and then, while the grant the
        // delivery minted lives an hour more. The next peer to ask for that
        // file re-stages identical bytes, so the fresh tag is the ONLY root
        // they have left: hand it back to be deleted and the collector this
        // session runs is free to take the file while that peer fetches it.
        let state = tempfile::TempDir::new().unwrap();
        let work = tempfile::TempDir::new().unwrap();
        let sent = work.path().join("report.html");
        std::fs::write(&sent, "delivered to a phone, then browsed from a laptop").unwrap();
        let node = endpoint::boot(&Dirs::new(state.path()), None, |_| {})
            .await
            .expect("boot");

        let record_id = "1700000000003-0000";
        let hash = crate::beam::stage_outbox(&node, &sent, record_id).await.expect("stage").hash;
        let staged: Hash = hash.parse().unwrap();
        node.grants.grant_pinned(staged, id(1));
        // The push landed, so `complete` releases the record's pin.
        crate::beam::unpin_outbox(&node, &crate::outbox::tag_name(record_id)).await;

        // A browse peer now asks the host for the same file.
        stage_for_peer(&node.store, &node.grants, &sent, id(2)).await.expect("stage");

        assert!(
            node.grants.admit(&staged, Some(id(2)), true),
            "the peer that asked may fetch these bytes"
        );
        assert!(
            rooted(&node, staged).await,
            "and something must still pin them, or there is nothing to fetch"
        );
    }

    // ── Failure classification ─────────────────────────────────────────────

    #[test]
    fn every_connect_failure_reports_its_own_cause_not_one_string() {
        // The two causes that mean the peer is asleep, spelled exactly as the
        // call sites print them today: `endpoint::dial` appends its cause to
        // `PEER_OFFLINE`, and the timeout is a peer that took the connection
        // and then said nothing. `McpCore::session` wraps both as
        // "{label} is not reachable: {e}. …", so the cause sits in the MIDDLE
        // of the sentence and no prefix test downstream could ever find it —
        // which is the whole reason the variant is made here instead.
        let dial = ConnectError::Unreachable(format!("{PEER_OFFLINE} (timed out)"));
        assert_eq!(dial.to_string(), "peer offline — could not reach it (timed out)");
        let silent = ConnectError::Unreachable("the peer did not answer the handshake".to_string());
        assert_eq!(silent.to_string(), "the peer did not answer the handshake");

        // Every answer from a peer that DID speak is a refusal, each keeping
        // the wording the client printed before any of this was typed.
        assert_eq!(
            classify_ack(Frame::Res(Res::Denied("not a paired peer".to_string()))),
            Err(ConnectError::Refused("not a paired peer".to_string())),
            "the host's own wording survives, verbatim"
        );
        assert_eq!(
            classify_ack(Frame::Res(Res::Hello(HelloAck {
                proto: proto::PROTO_VERSION + 1,
                device: "Mac Studio".to_string(),
                scope: "control".to_string(),
            }))),
            Err(ConnectError::Refused("unsupported protocol version".to_string()))
        );
        assert_eq!(
            classify_ack(Frame::Event(Event::TabOpened { path: "/a.html".to_string() })),
            Err(ConnectError::Refused(
                "the peer answered with an unexpected frame".to_string()
            ))
        );

        // …and the one answer that is a session.
        assert_eq!(
            classify_ack(Frame::Res(Res::Hello(HelloAck {
                proto: proto::PROTO_VERSION,
                device: "Mac Studio".to_string(),
                scope: "control".to_string(),
            }))),
            Ok(("Mac Studio".to_string(), "control".to_string()))
        );

        // A push fails on this side, in transit, or at the host, and each
        // still says only what it said before. WHICH real cause lands in
        // which variant is pinned where the causes are real — the
        // two-endpoint proof in tests/push_artifact.rs.
        assert_eq!(
            PushFailure::Local("only files can be beamed".to_string()).to_string(),
            "only files can be beamed"
        );
        assert_eq!(
            PushFailure::Transport("the session is closed".to_string()).to_string(),
            "the session is closed"
        );
        assert_eq!(
            PushFailure::Denied("not permitted for this peer".to_string()).to_string(),
            "not permitted for this peer"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_device_that_unpaired_this_node_says_so_instead_of_reading_as_a_dead_link() {
        // The proof that gives this fix its whole reason: a REAL dial at a
        // real host that does not list the dialer. The test above drives
        // `classify_ack` with a synthetic `Res::Denied`, which this path
        // never produces — `accept` closes the connection before `accept_bi`,
        // so no application frame is ever written and the only account of the
        // refusal is the QUIC close reason. Before this, every call on the
        // dead connection answered `stream ended: connection lost`, and the
        // user was told to check a device that was awake and refusing on
        // purpose.
        let host_dir = tempfile::TempDir::new().unwrap();
        let dialer_dir = tempfile::TempDir::new().unwrap();
        // An EMPTY peer store is the unpair: `accept` looks the dialer up,
        // misses, and refuses. Seeding nothing is the same state the device
        // is in a moment after its owner removes this server.
        let host = endpoint::boot(
            &Dirs::new(host_dir.path()),
            Some(state(&host_dir, RootSet::empty())),
            |_| {},
        )
        .await
        .expect("the host is up");
        let dialer = endpoint::boot(&Dirs::new(dialer_dir.path()), None, |_| {})
            .await
            .expect("the dialer is up");

        // Loopback, so the proof depends on no relay and no discovery.
        let addr = endpoint::addr_at(
            &host.endpoint.id().to_string(),
            endpoint::loopback_socket(&host).await.expect("the host bound a port"),
        )
        .unwrap();
        let failed = ClientSession::connect(
            &dialer,
            addr,
            "Claude Code".to_string(),
            |_| {},
            || {},
        )
        .await
        .expect_err("a host that does not list this node refuses it");

        assert_eq!(
            failed,
            ConnectError::Refused("not a paired peer".to_string()),
            "the receiver's own word for it, recovered from the close it already sent"
        );
        // The variant matters as much as the text: `Unreachable` is the queue
        // door, and staging a private copy of the user's file for a device
        // that is refusing this server is what that door must never open for.
        assert!(matches!(failed, ConnectError::Refused(_)), "and it is not read as a nap");

        host.router.shutdown().await.ok();
        dialer.router.shutdown().await.ok();
    }

    #[test]
    fn a_close_reason_from_another_machine_is_bounded_before_a_human_reads_it() {
        // `stated_refusal` needs a live connection, so the treatment its text
        // gets is pinned on the function that applies it. The reason is
        // written by the peer, and it lands in a sentence a human reads: the
        // same strip set a device name goes through, and a cap, because
        // neither the length nor the characters are this machine's to trust.
        let spoof = format!("not a paired peer\u{202E}{}", "x".repeat(500));
        let shown = proto::strip_spoofing_chars(&spoof, MAX_CLOSE_REASON_CHARS);
        assert!(!shown.contains('\u{202E}'), "no bidi override survives into the message");
        assert_eq!(shown.chars().count(), MAX_CLOSE_REASON_CHARS, "and the length is bounded");
    }

    #[test]
    fn a_content_address_of_the_wrong_length_is_refused_instead_of_parsed() {
        let real = Hash::new(b"a queued artifact");
        let hex = real.to_string();
        assert_eq!(staged_hash(&hex), Ok(real));

        // Without the length guard every one of these reaches
        // `Hash::from_str` as base32, which can PANIC — and a replayed
        // address does not arrive over the wire but out of a plain JSON file
        // on the user's own disk, which a hand edit or a half-written record
        // can leave in any shape at all. A panic there kills the drain for
        // every OTHER pending delivery too.
        let one_too_long = format!("{hex}0");
        let right_length_wrong_alphabet = "z".repeat(64);
        for bad in [
            "",
            "abc",
            &hex[..63],
            one_too_long.as_str(),
            right_length_wrong_alphabet.as_str(),
        ] {
            assert_eq!(
                staged_hash(bad),
                Err(PushFailure::Local("malformed content address".to_string())),
                "refused {bad:?}"
            );
        }
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
