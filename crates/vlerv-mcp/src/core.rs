// The tool logic, with no MCP framing anywhere in it.
//
// `McpCore` is a SECOND host for the `vlerv-remote` stack — a peer in its own
// right, with its own ed25519 identity, its own `peers.json` and its own blob
// store under `~/Library/Application Support/Vlerv/mcp/`. It is not a client
// of the desktop app and shares no state with it: pairing this server with a
// device is a separate act from pairing the app with that device.
//
// Two contracts carry over from the app unchanged:
//
//   * lazy boot — nothing binds a socket until a tool needs one, so a
//     registered-but-unused MCP server makes zero network connections;
//   * the path gate — every file this server sends passes
//     `beam::resolve_offerable` over a `RootSet`, exactly as the desktop
//     share sheet does.
//
// `server.rs` wraps each method below in an rmcp tool. Nothing in this file
// knows that MCP exists, which is what lets the integration test drive the
// same code path a tool call drives.

use std::collections::{BTreeSet, HashMap};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::future::join_all;
use serde::Serialize;
use tokio::sync::mpsc;
use vlerv_remote::host::{EmptyCatalog, EventSink, HostSignal};
use vlerv_remote::outbox::{self, Outbox, Record, Staged};
use vlerv_remote::peers::{self, short_id, Pairing, Peer, PeerStore, PendingPair, Scope};
use vlerv_remote::scope::{
    ClientSession, ConnectError, PushFailure, PushedArtifact, ScopeState, TabsCache,
};
use vlerv_remote::security::{self, RootSet};
use vlerv_remote::{beam, endpoint, Dirs};

use crate::args;
use crate::devices::{self, label};

/// How long `list_devices { probe: true }` waits on one device before calling
/// it offline. Short on purpose: presence is a hint, not a fact, and a caller
/// with six paired devices must not wait a minute for a list.
// A probe covers the dial plus the handshake once the endpoint is up. It must
// clear the handshake budget (HANDSHAKE_TIMEOUT = 10s in vlerv-remote); the
// cold-boot cost is paid once, before the probe loop, not inside this window.
const PROBE_TIMEOUT: Duration = Duration::from_secs(12);

/// How many pushed artifacts `server_status` lists. A server left running
/// beside a chatty control peer would otherwise grow this vector for the life
/// of the process. The newest entries are the ones a caller acts on, so the
/// oldest are dropped; `ServerStatus::received_total` still reports the true
/// count, because a silently shortened list reads as "that is all of them".
const MAX_RECEIVED: usize = 100;

/// Shortest content-hash prefix `stop_beam` accepts for ONE link. Revoking
/// the wrong link is recoverable (mint another); revoking by a one-character
/// prefix by accident is just noise. Omitting the argument revokes them all,
/// which is the deliberate, unambiguous form.
const MIN_HASH_CHARS: usize = 8;

/// How long a peer that would not take its queued sends is left alone, by how
/// many passes in a row have failed for it: one minute, two, five, then ten
/// forever.
///
/// The FLOOR is the deliberate part. Naming a peer costs an n0 discovery
/// lookup and a possible relay traversal, so a tighter ladder would spend the
/// week a record lives telling third parties who this machine talks to, to
/// learn what it already knows — the phone is asleep. The precise trigger is
/// a peer connecting; this is only the fallback for one that comes back
/// without ever dialing in.
const RETRY_LADDER: [u64; 4] = [60, 120, 300, 600];

/// How many wake requests may be pending before one is dropped. Lossy on
/// purpose: a wake is a HINT that a pass is worth running, and a pass that is
/// already queued covers every peer anyway. Blocking the caller — the host's
/// own event sink, on its accept path — to deliver a hint would be the wrong
/// trade in the other direction.
const WAKE_DEPTH: usize = 32;

/// Live sessions, one per peer, shared by the send path and the drain. One
/// map, deliberately: a session the drain dialed serves the next send, and a
/// send's session drains the queue without a second handshake.
type SessionCache = Arc<tokio::sync::Mutex<HashMap<String, Arc<ClientSession>>>>;

/// What the caller is told about a paired device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeviceInfo {
    pub device: String,
    pub node_id: String,
    pub node_id_short: String,
    /// What this device may do on THIS server ("view-open" / "browse" /
    /// "control"). It says nothing about what this server may do on the
    /// device — that grant lives in the device's own peer list.
    pub scope: String,
    pub paired_at: u64,
    pub last_seen: u64,
    /// "online", "offline", or "unknown" when nothing dialed it.
    pub presence: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BeamLink {
    /// The `vlerv://receive?…` link to hand to a person.
    pub link: String,
    pub ticket: String,
    pub name: String,
    pub size: u64,
    /// Unix seconds. After this the link is refused even if the process lives.
    pub expires_at: u64,
    pub hash: String,
}

/// What a `send_to_device` call actually did.
///
/// A TAGGED enum, and the tag is the whole point: "it is on your phone" and
/// "a copy of it is waiting here for your phone" are different events, and
/// the type is what stops the render code compiling until it has decided
/// which one it is printing. A wording convention could not — that is how the
/// original failure read as a success in the first place. Serde's internal
/// tagging keeps the JSON an OBJECT, which `structuredContent` requires.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Delivery {
    /// The bytes are on the other machine, which verified them itself.
    Delivered {
        device: String,
        node_id: String,
        /// The name the RECEIVING device landed the file under — collision
        /// handling there may have renamed it.
        name: String,
        /// The size the receiver measured, never the one this side announced.
        size: u64,
        hash: String,
    },
    /// The device did not answer, so the file was COPIED and the send was
    /// accepted for later. Nothing has arrived anywhere yet.
    Queued {
        device: String,
        node_id: String,
        /// What the receiver will be TOLD the file is called. It has agreed
        /// to nothing: a collision on its side may still rename it on
        /// arrival, the way a delivered one is renamed today.
        name: String,
        size: u64,
        /// BLAKE3 of the copy that was taken. The delivery replays THOSE
        /// bytes, whatever the source path holds by the time it goes out.
        hash: String,
        /// The spool record this send became — the id `server_status` reports
        /// it under.
        id: String,
        /// Unix seconds. After this the copy is given up on and deleted.
        expires_at: u64,
        /// The dial's own answer, verbatim: why the file is not there yet.
        reason: String,
        /// The facts the model has to pass on, because "queued" hides every
        /// one of them and none is obvious to the person who asked for a send.
        notes: Vec<String>,
    },
}

/// One accepted-but-undelivered send, as `server_status` reports it.
///
/// `last_error` is carried rather than summarized: a record that is not
/// moving has a reason, and a status surface that shows a count without it
/// is the silent failure this queue exists to remove.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QueuedDelivery {
    pub id: String,
    pub device: String,
    pub node_id: String,
    pub node_id_short: String,
    pub name: String,
    pub size: u64,
    pub hash: String,
    /// The file the copy was taken from. Reported because `VLERV_MCP_ROOTS`
    /// differs between projects: a record whose source has left this server's
    /// roots can only ever be served by a session whose roots still hold it.
    pub source: PathBuf,
    pub enqueued_at: u64,
    pub expires_at: u64,
    pub attempts: u32,
    pub last_attempt_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

impl From<Record> for QueuedDelivery {
    fn from(r: Record) -> Self {
        Self {
            node_id_short: short_id(&r.peer),
            id: r.id,
            device: r.device,
            node_id: r.peer,
            name: r.name,
            size: r.size,
            hash: r.hash,
            source: r.source,
            enqueued_at: r.enqueued_at,
            expires_at: r.expires_at,
            attempts: r.attempts,
            last_attempt_at: r.last_attempt_at,
            last_error: r.last_error,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PairingInvite {
    /// The `vlerv://pair?ticket=…` link the human opens on the other device.
    pub link: String,
    pub ticket: String,
    pub node_id: String,
    pub device: String,
    pub expires_at: u64,
    pub fingerprint_hint: String,
    pub instructions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingPairing {
    pub node_id: String,
    pub node_id_short: String,
    pub device: String,
    /// The six words that MUST also be on the other device's screen.
    pub fingerprint: Vec<String>,
    pub role: String,
    pub created_at: u64,
}

impl From<PendingPair> for PendingPairing {
    fn from(p: PendingPair) -> Self {
        Self {
            node_id_short: short_id(&p.node_id),
            node_id: p.node_id,
            device: p.device,
            fingerprint: p.fingerprint,
            role: p.role,
            created_at: p.created_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PairingOutcome {
    pub paired: bool,
    pub device: String,
    pub node_id: String,
    /// The scope granted to the new device on this server, when it was
    /// accepted.
    pub scope: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OfferSummary {
    pub name: String,
    pub size: u64,
    pub link: String,
    pub expires_at: u64,
    pub fetches: u64,
    /// BLAKE3 content address, hex — also the offer id `stop_beam` revokes by.
    pub hash: String,
}

impl From<beam::OfferInfo> for OfferSummary {
    fn from(o: beam::OfferInfo) -> Self {
        Self {
            name: o.name,
            size: o.size,
            link: o.link,
            expires_at: o.expires_at,
            fetches: o.fetches,
            hash: o.id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServerStatus {
    pub node_id: String,
    pub node_id_short: String,
    pub device: String,
    /// `<state dir>/remote`, which holds identity.key, peers.json and blobs.
    pub identity_dir: PathBuf,
    pub state_dir: PathBuf,
    /// False until a tool needed the network — the lazy-boot contract.
    pub booted: bool,
    /// Why the last boot failed, when one was tried and refused. `booted:
    /// false` alone cannot tell an idle server from one that is locked out
    /// for its whole life; this is the difference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boot_error: Option<String>,
    pub uptime_secs: u64,
    pub paired_devices: usize,
    pub active_offers: Vec<OfferSummary>,
    /// Files other devices pushed to this server during this process, newest
    /// last, capped at `MAX_RECEIVED` entries.
    pub received_artifacts: Vec<ReceivedArtifact>,
    /// How many arrived in total. Higher than `received_artifacts.len()` once
    /// the cap has dropped the oldest entries.
    pub received_total: u64,
    /// Sends accepted for a device that did not answer, oldest first. Not
    /// capped the way `received_artifacts` is: the spool itself is capped at
    /// `outbox::MAX_RECORDS`, and every entry is a file somebody was told
    /// would arrive, so none of them may be hidden from the list.
    pub queued: Vec<QueuedDelivery>,
    /// How many are waiting. It matches `queued.len()`, and that is the
    /// claim: unlike `received_total`, this count can never be higher than
    /// the list beside it, because a hidden pending delivery is the thing
    /// this surface exists to make impossible.
    pub queued_total: usize,
    /// What those records weigh — the number the spool's own cap counts.
    pub queued_bytes: u64,
    /// Private copies of the user's files this server is holding inside its
    /// state directory for the queue — one per distinct content address, not
    /// one per record, because two devices owed the same file are owed one
    /// copy of it. This is what the queue costs this disk, and `queued_bytes`
    /// beside it is what is still owed to devices; they differ exactly when a
    /// file is queued for more than one device.
    ///
    /// Still a floor for the state directory as a whole, and now only by a
    /// minute: a delivery that just landed has released its copy, and the
    /// store's collector takes it at its next pass (`endpoint::GC_INTERVAL`).
    pub retained_bytes: u64,
    /// Record files this build could not put on the list above: unparseable
    /// ones, moved aside as `<id>.json.broken.<unix>`; ones written by a
    /// newer schema and left byte-for-byte alone; and ones that would not
    /// read at all this boot, also left alone so a later boot can retry them.
    /// Named rather than counted, because each one is a delivery that is
    /// quietly not happening.
    ///
    /// A quarantined stem stays on this list for every later boot, not just
    /// the one that moved the file aside: the alternative is a delivery that
    /// is reported once and then silently forgotten.
    pub queue_unreadable: Vec<String>,
    /// Whether a drain pass owns this queue right now — true while any peer
    /// is being drained. A reader must be able to tell a queue that is moving
    /// from one that is merely stored: a count that is still 3 a minute later
    /// means something different when a pass has been trying the whole time.
    /// False on a server that never booted, which is the honest answer — the
    /// drain lives in the process holding the blob-store claim.
    pub draining: bool,
    /// Why this server can neither queue nor drain, when it cannot. Same
    /// register as `boot_error`, and for the same reason: a second Claude
    /// Code session over one state directory reports the store claim it lost
    /// rather than a queue it will never move.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue_blocked_reason: Option<String>,
    /// The directories a sent file is resolved against.
    pub roots: Vec<PathBuf>,
}

/// The pushed-artifact log: the last `MAX_RECEIVED` arrivals, and how many
/// arrived in all. One structure under one lock — a separate counter beside
/// the vector could be read a push out of step with it.
#[derive(Default)]
struct Received {
    items: Vec<ReceivedArtifact>,
    total: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReceivedArtifact {
    pub from: String,
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    pub hash: String,
}

/// What one delivery attempt settled on. Not a `Result`: two of the three
/// answers are neither a success nor a caller error, and only the type keeps
/// them from collapsing into the one string that made a sleeping phone and a
/// broken send read the same.
enum Attempt {
    /// The host answered `Res::Pushed` and reported what it landed.
    Landed(PushedArtifact),
    /// Nothing on the other side answered the dial. The queue-eligible cause.
    Asleep(String),
    /// The session died under the request. Queue-eligible only after a fresh
    /// dial has confirmed it, because a cached session's `is_closed()` flag
    /// cannot tell a sleeping peer from an awake one.
    SessionDied(String),
}

/// Why the drain was asked to wake up. The two reasons buy different things,
/// and collapsing them into "run a pass" would undo the retry ladder: a peer
/// that just proved it is reachable is worth dialing NOW, while a spool that
/// merely grew is not — the send that grew it has just finished failing to
/// reach that same device.
enum Wake {
    /// This peer holds a working session, or has just taken a file over one.
    /// It is dialed on this pass whatever its backoff says.
    Peer(String),
    /// A record was accepted. Nothing is forced; the point is only to arm the
    /// tick, which stands down while the spool is empty and would otherwise
    /// never learn that it stopped being empty.
    Spool,
}

/// When one peer may be dialed again, and how many passes in a row have
/// failed for it.
///
/// In memory only, and keyed by PEER rather than by record: a record's
/// persisted `attempts` is what the status surface reports to a human,
/// while this is the cadence, and a restart is a new decision — the phone may
/// well be awake now, so the boot pass tries every peer at once. `Instant`,
/// not `now_unix`, so a clock the user corrects backwards cannot park a
/// pending delivery for however long the correction was.
struct Backoff {
    failures: u32,
    next: Instant,
}

/// This server's `EventSink`. A headless host has no window to raise, so the
/// signals become: park the pairing for `pair_status`, record the artifact for
/// `server_status`, wake the drain for a device that just proved it is
/// reachable, and a stderr line for a human tailing the log. Stdout is the
/// JSON-RPC channel and is never written to here.
struct McpSink {
    pairing: Arc<Pairing>,
    received: Arc<Mutex<Received>>,
    /// The wake channel, held WEAKLY, and that is not a nicety. This sink is
    /// owned by the `ScopeState` inside the `RemoteNode` that the supervisor
    /// task holds, so a strong sender here would make the channel
    /// un-closable: `supervise` would never read `None`, never return, and
    /// never drop the node — and the blob-store claim would outlive the
    /// `McpCore` that took it, refusing every later process over the same
    /// state directory.
    wake: mpsc::WeakSender<Wake>,
}

impl EventSink for McpSink {
    fn emit(&self, signal: HostSignal) {
        match signal {
            HostSignal::PairPending(pending) => {
                // The pairing server SIGNALS the fingerprint step and leaves
                // holding it to the host. Parking it here is what makes
                // `pair_status` / `confirm_pairing` work for the side that
                // minted the ticket.
                eprintln!(
                    "vlerv-mcp: pairing with {} ({}) — fingerprint: {}",
                    pending.device,
                    short_id(&pending.node_id),
                    pending.fingerprint.join(" ")
                );
                self.pairing.park(pending);
            }
            HostSignal::OpenOnHost { peer, path, .. } => {
                // Nothing to open: this host has no tabs. Logged so a control
                // peer's attempt is at least visible.
                eprintln!(
                    "vlerv-mcp: {} asked to open {} — this server has no viewer",
                    short_id(&peer),
                    path.display()
                );
            }
            HostSignal::ArtifactReceived { peer, path, name, size, hash } => {
                eprintln!("vlerv-mcp: {} pushed {name} to {}", short_id(&peer), path.display());
                let mut received = self.received.lock().unwrap_or_else(|p| p.into_inner());
                received.total += 1;
                received.items.push(ReceivedArtifact { from: peer, name, path, size, hash });
                // Drop from the FRONT, so the list stays the last
                // MAX_RECEIVED arrivals rather than the first — an operator
                // asking what just landed wants the newest.
                if received.items.len() > MAX_RECEIVED {
                    received.items.remove(0);
                }
            }
            HostSignal::PeerConnected { peer, device, scope } => {
                // The precise wake trigger. This device holds a session right
                // now, so anything queued for it goes out on this pass
                // whatever its backoff says — the ladder exists to stop this
                // server naming a sleeping phone to n0 discovery every few
                // seconds, and a device that dialed in has already answered
                // the question the ladder was waiting on.
                //
                // `scope` is deliberately NOT cached as `last_ack_scope`. It
                // says what THIS server grants that device; the queue's
                // pre-check reads the opposite direction, what the device
                // grants this server, and only an ack this side RECEIVES
                // carries that — `Drainer::dial_session` writes those. Mixing
                // the two would stage private copies of the user's files for
                // a device that denies them on arrival, and refuse sends to
                // one that would take them.
                // The direction is spelled out because the log is the one
                // place a human reads this value, and both grants are called
                // "scope".
                eprintln!(
                    "vlerv-mcp: {device} ({}) connected; this server grants it {scope:?}",
                    short_id(&peer)
                );
                // This runs on the host's accept path, so it may not block or
                // fail it: a full channel already holds a pass that covers
                // this peer, and a gone channel means the drain this hint was
                // for has already ended with its process.
                if let Some(wake) = self.wake.upgrade() {
                    let _ = wake.try_send(Wake::Peer(peer));
                }
            }
        }
    }
}

/// The server's whole state. One instance per process.
pub struct McpCore {
    dirs: Dirs,
    device: String,
    roots: RootSet,
    cwd: PathBuf,
    home: Option<PathBuf>,
    peers: Arc<PeerStore>,
    pairing: Arc<Pairing>,
    received: Arc<Mutex<Received>>,
    /// Sends accepted for a device that was asleep. Read off disk in `new`,
    /// beside the peer store and for the same reason: both are files this
    /// server must be able to REPORT before it is allowed to open a socket,
    /// and reading a directory is not a network connection. `Arc` because the
    /// drain will hold it from a task of its own.
    outbox: Arc<Outbox>,
    /// The booted node, or nothing yet — the lazy-boot contract.
    ///
    /// A `OnceCell` rather than a `Mutex<Option<_>>` so that reading whether
    /// a node exists never waits on, or is defeated by, a boot in flight: a
    /// mutex held across `boot` also blocks the read side, and `try_lock`
    /// would answer "not booted" for a server that is serving.
    /// `get_or_try_init` does not cache a failure, so a refusal stays
    /// retryable once the other process exits.
    node: tokio::sync::OnceCell<Arc<endpoint::RemoteNode>>,
    /// Why the last boot attempt failed, if one did. `node` being empty says
    /// only "no node"; it does not say whether nobody asked yet or whether
    /// every attempt was refused. Without this, a server locked out for its
    /// whole life reports the same `booted: false` as a healthy idle one,
    /// and `stop_beam` answers "nothing to revoke" without having looked.
    /// Overwritten, not accumulated: a refusal is retryable, so only the
    /// most recent attempt is worth reporting.
    last_boot_error: Mutex<Option<String>>,
    /// Live sessions, one per peer. `Arc` because a session that closes
    /// evicts its own entry from here — see `dial_session`.
    sessions: SessionCache,
    started: Instant,
    /// Test seam: when set, peers are dialed at this socket and the push
    /// ticket names this server's own loopback address, so a two-endpoint
    /// test never depends on relays or discovery. `None` in production.
    ///
    /// Shared with the drain and read on every dial rather than copied when
    /// the drain starts: the two-endpoint proof re-points it at a second host
    /// while the drain task is already running, which is exactly the shape of
    /// a phone that went away and came back somewhere else.
    loopback: Arc<Mutex<Option<SocketAddr>>>,
    /// Tells the drain that a pass is worth running. Built in `new` because
    /// an mpsc pair binds nothing and every sender needs somewhere to send
    /// from the first tool call; the receiver is handed to the supervisor by
    /// the boot that spawns it.
    wake: mpsc::Sender<Wake>,
    wake_rx: Mutex<Option<mpsc::Receiver<Wake>>>,
    /// Peers with a drain pass in flight, and whether one more is owed. The
    /// single supervisor cannot overlap two passes by construction, but a
    /// wake arriving mid-pass must not be dropped either — it may be the one
    /// carrying the address the last attempt lacked.
    inflight: Arc<Mutex<HashMap<String, bool>>>,
    /// The retry ladder's state, one entry per peer that has refused to take
    /// its queued sends. Empty at startup, which is what makes a restart
    /// retry at once.
    retry: Arc<Mutex<HashMap<String, Backoff>>>,
    /// The size a staged copy may not pass, which is `beam::HARD_CAP_BYTES`
    /// in production and nothing else. It is a field only so a test can lower
    /// it — see `cap_staged_at`.
    staged_cap: Mutex<u64>,
}

impl McpCore {
    /// Build a core over `state_dir`, with `roots` as the send policy's root
    /// set. Reads `peers.json` (a missing file is a fresh install) and binds
    /// nothing.
    pub fn new(state_dir: PathBuf, roots: Vec<PathBuf>, cwd: PathBuf, home: Option<PathBuf>) -> Self {
        let dirs = Dirs::new(state_dir);
        let (wake, wake_rx) = mpsc::channel(WAKE_DEPTH);
        Self {
            device: device_name(),
            peers: Arc::new(PeerStore::load(&dirs.remote())),
            pairing: Arc::new(Pairing::new()),
            received: Arc::new(Mutex::new(Received::default())),
            outbox: Arc::new(Outbox::load(&dirs.outbox())),
            roots: RootSet::new(roots),
            cwd,
            home,
            dirs,
            node: tokio::sync::OnceCell::new(),
            last_boot_error: Mutex::new(None),
            sessions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            started: Instant::now(),
            loopback: Arc::new(Mutex::new(None)),
            wake,
            wake_rx: Mutex::new(Some(wake_rx)),
            inflight: Arc::new(Mutex::new(HashMap::new())),
            retry: Arc::new(Mutex::new(HashMap::new())),
            staged_cap: Mutex::new(beam::HARD_CAP_BYTES),
        }
    }

    /// Build a core from the process environment: state dir, roots and
    /// working directory as `main` sees them.
    pub fn from_env() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        let home = std::env::var_os("HOME").map(PathBuf::from);
        Self::new(state_dir(), configured_roots(&cwd), cwd, home)
    }

    /// Dial every peer at `host` instead of asking discovery where it is, and
    /// mint push tickets on this server's own loopback address. The
    /// integration test's seam, so a two-endpoint proof never depends on
    /// relays — production never calls it.
    ///
    /// Settable after the drain is running, and read by it on every dial: a
    /// device that went away and came back at another address is the case the
    /// queue exists for, so the proof of it has to be able to move.
    #[doc(hidden)]
    pub fn use_loopback(&self, host: SocketAddr) {
        *self.loopback.lock().unwrap_or_else(|p| p.into_inner()) = Some(host);
    }

    /// Whether the seam above is set, over this server's OWN handle. Reading
    /// it through a `Drainer` cost nine field clones — seven `Arc`s and two
    /// `String`s — to look at one `Mutex`, on every send.
    fn loopback(&self) -> Option<SocketAddr> {
        *self.loopback.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Refuse a staged copy over `bytes` instead of over the real
    /// `beam::HARD_CAP_BYTES`. The unit test's seam, so the proof that an
    /// oversized copy is refused — and unpinned — costs a few kilobytes on
    /// disk rather than the 256 MiB the production cap would need. Production
    /// never calls it, and the value it would pass is the one `new` already
    /// installs.
    #[cfg(test)]
    fn cap_staged_at(&self, bytes: u64) {
        *self.staged_cap.lock().unwrap_or_else(|p| p.into_inner()) = bytes;
    }

    /// The cap a staged copy is measured against, read through the seam.
    fn staged_cap(&self) -> u64 {
        *self.staged_cap.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn device(&self) -> &str {
        &self.device
    }

    /// Test seam: the integration test writes both peer stores by hand, the
    /// way `confirm_pairing` writes them, so a two-endpoint proof does not
    /// have to drive a human fingerprint comparison. No tool reads this.
    #[doc(hidden)]
    pub fn peer_store(&self) -> &Arc<PeerStore> {
        &self.peers
    }

    /// The node id this server presents, read from the identity key without
    /// binding a socket. First call creates the key file (0600) if it is
    /// missing — a file write, not a network connection.
    pub fn node_id(&self) -> Result<String, String> {
        let dir = self.dirs.remote();
        std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {dir:?}: {e}"))?;
        Ok(endpoint::load_or_create_identity(&dir)?.public().to_string())
    }

    /// Boot the endpoint on first use and keep it for the life of the
    /// process — which is what keeps a beam link fetchable after the tool call
    /// that minted it returned.
    async fn node(&self) -> Result<Arc<endpoint::RemoteNode>, String> {
        let booted = self.boot().await;
        // Remember a refusal so the read-only tools can say WHY they see no
        // node, instead of reporting the same thing an idle server reports.
        *self.last_boot_error.lock().unwrap_or_else(|p| p.into_inner()) =
            booted.as_ref().err().cloned();
        booted
    }

    async fn boot(&self) -> Result<Arc<endpoint::RemoteNode>, String> {
        self.node
            .get_or_try_init(|| async {
                let state = Arc::new(ScopeState::new(
                    self.peers.clone(),
                    self.pairing.clone(),
                    Arc::new(TabsCache::new()),
                    self.roots.clone(),
                    self.device.clone(),
                    // Headless: no bookmarks, no recents, no open tabs. A
                    // view-open peer is therefore told about nothing and may
                    // fetch nothing.
                    Arc::new(EmptyCatalog),
                    McpSink {
                        pairing: self.pairing.clone(),
                        received: self.received.clone(),
                        // Weak, so the supervisor spawned just below can
                        // still see the channel close when this core is
                        // dropped — see the field.
                        wake: self.wake.downgrade(),
                    },
                ));
                let node = Arc::new(endpoint::boot(&self.dirs, Some(state), |_| {}).await?);

                // The drain is spawned here and nowhere else, for two
                // reasons. The lazy-boot contract: a registered but unused
                // server must still bind nothing, and this initializer is the
                // one place that has already given that up. And the blob
                // store: the process holding `remote/blobs.lock` is the only
                // one that may open the store, so it is the only one that may
                // stage, re-grant or replay — the claim IS the outbox lock,
                // and no second lock is added. `get_or_try_init` does not
                // cache a failure and this sits after the `?`, so exactly one
                // supervisor is ever spawned.
                let drainer = self.drainer(&node);
                drainer.reconcile().await;
                match self.wake_rx.lock().unwrap_or_else(|p| p.into_inner()).take() {
                    Some(wake) => {
                        tokio::spawn(supervise(drainer, wake));
                    }
                    // Unreachable while the `OnceCell` holds: the receiver is
                    // taken exactly once, by the boot that succeeds. Reported
                    // rather than ignored, because a queue nothing drains is
                    // the failure this whole feature exists to remove.
                    None => eprintln!(
                        "vlerv-mcp: the queue supervisor was already started — this boot \
                         drains nothing"
                    ),
                }
                Ok(node)
            })
            .await
            .cloned()
    }

    /// The node IF one is already booted — never a wait, and never a false
    /// negative that matters. `stop_beam` reads this to decide there is
    /// nothing to revoke; an offer can only exist after `node()` has
    /// returned, so a `None` here never hides a live link. (It IS `None`
    /// while a boot is still running, which is the honest answer then.)
    fn booted(&self) -> Option<Arc<endpoint::RemoteNode>> {
        self.node.get().cloned()
    }

    /// Why the last boot failed, for the tools that find no node and have to
    /// tell the caller whether that means "idle" or "locked out".
    fn last_boot_error(&self) -> Option<String> {
        self.last_boot_error.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Resolve a caller-supplied path, confine it to this server's roots, and
    /// hand back the candidate the send path stages — the WHOLE gate, run
    /// once per tool call.
    ///
    /// The desktop share sheet may send a file that lies outside every root,
    /// because a human picked that file in a dialog. Here the caller is a
    /// language model, and its arguments can be steered by text it merely
    /// read — a repository file, a fetched page, another tool's output. So a
    /// headless host takes the STRICT half of the gate,
    /// `canonicalize_and_check_root`, the same check every remote read
    /// passes: `VLERV_MCP_ROOTS` (or the working directory) is a real
    /// boundary, not a hint, and a link minted here can only ever address a
    /// file the operator put inside it. `beam::resolve_offerable` then applies
    /// the share sheet's own policy — regular files only, under the hard cap.
    ///
    /// One refusal string for "does not exist", "outside the roots" and "no
    /// roots configured" — the share module's no-existence-leak convention,
    /// unchanged.
    fn gate_arg_path(&self, raw: &str) -> Result<beam::OfferCandidate, String> {
        let path = args::resolve_arg_path(raw, &self.cwd, self.home.as_deref())?;
        let confined = security::canonicalize_and_check_root(&path, &self.roots)
            .map_err(|_| "path not found or out of root".to_string())?;
        beam::resolve_offerable(&confined, &self.roots)
    }

    // ── beam_artifact ──────────────────────────────────────────────────────

    /// Stage a file and mint a `vlerv://receive?…` link for it. The offer
    /// stays served until it expires or the process ends.
    pub async fn beam_artifact(
        &self,
        raw_path: &str,
        ttl_hours: Option<u32>,
    ) -> Result<BeamLink, String> {
        let ttl = args::validate_ttl(ttl_hours)?;
        // Policy first, sockets second: a file that cannot be sent must not
        // boot the network stack to find that out.
        let cand = self.gate_arg_path(raw_path)?;
        let node = self.node().await?;
        let info = beam::offer(&node, &cand, ttl).await?;
        Ok(BeamLink {
            link: info.link,
            ticket: info.ticket,
            name: info.name,
            size: info.size,
            expires_at: info.expires_at,
            hash: info.id,
        })
    }

    // ── stop_beam ──────────────────────────────────────────────────────────

    /// Revoke a beam link before its TTL runs out. The blobs request gate
    /// consults the offers registry on EVERY request, so a stopped offer is
    /// refused on the next fetch even though the staged bytes are still in
    /// the store — the design's instant Stop, which is the only answer to a
    /// link that went to the wrong place. Boots nothing: with no endpoint
    /// there is no offer to revoke.
    pub async fn stop_beam(&self, hash: Option<&str>) -> Result<Vec<OfferSummary>, String> {
        let Some(node) = self.booted() else {
            // "No offers" and "I could not look" are different answers, and
            // only one of them means the caller's link is dead.
            if let Some(e) = self.last_boot_error() {
                return Err(format!("cannot check beam links: {e}"));
            }
            return Ok(Vec::new());
        };
        let live = node.offers.list();
        let doomed = match hash {
            None => live,
            Some(raw) => {
                let raw = raw.trim();
                // The same prefix pass a device or a pairing argument takes,
                // with this argument's own floor. Unlike those two, every
                // match is revoked: revoking one link too many is recoverable.
                let needle = devices::needle(raw, MIN_HASH_CHARS).ok_or_else(|| {
                    format!(
                        "hash must be at least {MIN_HASH_CHARS} characters — server_status \
                         reports the hash of every live link"
                    )
                })?;
                let matched: Vec<beam::OfferInfo> =
                    devices::by_prefix(&live, &needle, |o| o.id.as_str())
                        .into_iter()
                        .cloned()
                        .collect();
                if matched.is_empty() {
                    return Err(format!("no live beam link matches {raw:?}"));
                }
                matched
            }
        };
        let mut stopped = Vec::with_capacity(doomed.len());
        for offer in doomed {
            beam::stop(&node, &offer.id).await;
            stopped.push(OfferSummary::from(offer));
        }
        Ok(stopped)
    }

    // ── list_devices ───────────────────────────────────────────────────────

    /// Every paired device. With `probe`, each one is dialed once to report
    /// live presence; without it, presence is "online" only for a session
    /// this process already holds.
    pub async fn list_devices(&self, probe: bool) -> Result<Vec<DeviceInfo>, String> {
        let peers = self.peers.list();
        // Pay the lazy endpoint boot ONCE, outside the per-device probe budget.
        // Otherwise the first probe on a cold server spends its whole timeout
        // on bind + relay + store load and reports a reachable device offline.
        //
        // A boot that FAILS is the answer, not a detail to swallow: every
        // probe below would then dial through the same failure and report
        // "offline", sending the user to debug phones and networks over a
        // fault that is neither. Say the real reason once.
        if probe {
            self.node().await?;
        }
        // The probes run TOGETHER. Dialed one after another, a fleet where
        // three devices are asleep costs three PROBE_TIMEOUTs before the list
        // comes back; concurrently the whole call bounds at about one, however
        // many devices are paired.
        let presence =
            join_all(peers.iter().map(|peer| self.presence(peer, probe))).await;
        Ok(peers
            .into_iter()
            .zip(presence)
            .map(|(peer, presence)| DeviceInfo {
                node_id_short: short_id(&peer.node_id),
                device: peer.device,
                node_id: peer.node_id,
                scope: peer.scope.as_str().to_string(),
                paired_at: peer.paired_at,
                last_seen: peer.last_seen,
                presence,
            })
            .collect())
    }

    async fn presence(&self, peer: &Peer, probe: bool) -> &'static str {
        if let Some(session) = self.sessions.lock().await.get(&peer.node_id) {
            if !session.is_closed() {
                return "online";
            }
        }
        if !probe {
            return "unknown";
        }
        match tokio::time::timeout(PROBE_TIMEOUT, self.session(peer)).await {
            Ok(Ok(_)) => "online",
            _ => "offline",
        }
    }

    // ── send_to_device ─────────────────────────────────────────────────────

    /// Resolve `device` to one paired peer and push `raw_path` to it — or, if
    /// that device is merely asleep, copy the file now and promise it.
    ///
    /// Every CALLER error is still a hard error: an unsendable path, an
    /// unknown device, a device whose grant is KNOWN to be narrower than
    /// control — either from the handshake this call just completed, or from
    /// the scope the last completed one reported. A device this server has
    /// never handshaken with is the other case: nothing has refused anything
    /// yet, so a send that device does not answer is queued like any other,
    /// and the answer says its control grant is unverified. Only "the other
    /// machine did not answer" becomes a queued send, because that is the only
    /// cause a later attempt can change.
    pub async fn send_to_device(&self, raw_path: &str, device: &str) -> Result<Delivery, String> {
        let query = args::validate_device_query(device)?;
        // Same order as `beam_artifact`: one gate pass, and an unsendable file
        // is refused before any socket opens.
        let cand = self.gate_arg_path(raw_path)?;
        let peer = devices::resolve_device(&self.peers.list(), query).map_err(|e| e.to_string())?;

        // The store is opened BEFORE anything else is decided, and its refusal
        // is this call's refusal. Queuing copies the file INTO that store, so
        // a server that may not open it has to say so rather than promise a
        // delivery it can neither stage nor replay.
        let node = self.node().await?;

        match self.attempt_send(&node, &peer, &cand).await? {
            Attempt::Landed(pushed) => {
                // The most precise reachability fact this server can have:
                // that device just took a file over a live session. Anything
                // waiting for it goes out now instead of at the next tick.
                self.wake(Wake::Peer(peer.node_id.clone()));
                Ok(Delivery::Delivered {
                    device: peer.device,
                    node_id: peer.node_id,
                    name: pushed.name,
                    size: pushed.size,
                    hash: pushed.hash,
                })
            }
            Attempt::Asleep(why) | Attempt::SessionDied(why) => {
                // This call has just spent a full dial learning that the
                // device does not answer, so the drain starts one ladder step
                // in rather than repeating that dial a second later. The
                // RECORD still says nothing has been attempted, because
                // nothing has: this is the cadence, not the report.
                self.drainer(&node).back_off(&peer.node_id);
                let queued = self.queue_send(&node, &peer, &cand, why).await?;
                // The tick stands down while the spool is empty, so the pass
                // that would notice this record has to be told the spool
                // stopped being empty. Without this a send accepted on an
                // idle server waits for the phone to dial in, and nothing on
                // this side ever tries again.
                self.wake(Wake::Spool);
                Ok(queued)
            }
        }
    }

    /// One delivery attempt, and at most one retry.
    ///
    /// The retry is not politeness, it IS the reported bug. `dial_session`
    /// hands back a cached session whenever `is_closed()` reads false, and
    /// that flag is an `AtomicBool` the reader and writer tasks set — not a
    /// probe. After an iOS suspension it still reads false, so the failure
    /// arrives from `request` as `PushFailure::Transport` and never from a
    /// dial at all. A design that only classified dial failures would never
    /// queue the exact case the user reported. Only a fresh dial can tell a
    /// stale cache entry from a phone that has gone away.
    async fn attempt_send(
        &self,
        node: &Arc<endpoint::RemoteNode>,
        peer: &Peer,
        cand: &beam::OfferCandidate,
    ) -> Result<Attempt, String> {
        match self.try_send(node, peer, cand).await? {
            Attempt::SessionDied(_) => {
                forget_session(&self.sessions, &peer.node_id).await;
                self.try_send(node, peer, cand).await
            }
            settled => Ok(settled),
        }
    }

    /// Dial, check the grant, push. Every hard refusal on the send path is
    /// produced here, so a caller error and a sleeping device cannot end up
    /// wearing each other's answer.
    async fn try_send(
        &self,
        node: &Arc<endpoint::RemoteNode>,
        peer: &Peer,
        cand: &beam::OfferCandidate,
    ) -> Result<Attempt, String> {
        // `ConnectError::Unreachable` is THE queue door, and it is matched
        // here rather than behind a wrapper type so the rule reads where it
        // is enforced: every other dial failure leaves through `Err` and can
        // never reach the spool.
        let mut session = match self.drainer(node).dial_session(peer).await {
            Ok(session) => session,
            Err(ConnectError::Unreachable(why)) => return Ok(Attempt::Asleep(why)),
            Err(refused) => return Err(not_reachable(peer, &refused)),
        };
        // The scope in the handshake is what the DEVICE granted this server.
        // Checking it here turns the host's one deliberately vague refusal
        // ("not permitted for this peer") into an instruction the human can
        // act on. A scope this build cannot parse is treated as NOT granting
        // control (`unwrap_or(false)`), which is what a raw `!= "control"`
        // compare did: a live push is about to happen, so an unreadable grant
        // must refuse rather than guess.
        if !grants_control(&session.scope).unwrap_or(false) {
            // A session reports the grant as it stood when it connected, so a
            // cached one can be stale: the human may have widened the scope
            // between two tool calls. Re-handshake once before refusing.
            forget_session(&self.sessions, &peer.node_id).await;
            session = match self.drainer(node).dial_session(peer).await {
                Ok(session) => session,
                Err(ConnectError::Unreachable(why)) => return Ok(Attempt::Asleep(why)),
                Err(refused) => return Err(not_reachable(peer, &refused)),
            };
        }
        if !grants_control(&session.scope).unwrap_or(false) {
            return Err(needs_control(peer, &session.scope, &self.device));
        }

        // The canonical path the gate resolved, not the caller's string: the
        // push re-applies the same policy, and it must see the same file.
        let pushed = match own_loopback(self.loopback(), node).await {
            Some(own) => session.push_artifact_at(&cand.canonical, &self.roots, own).await,
            None => session.push_artifact(&cand.canonical, &self.roots).await,
        };
        match pushed {
            Ok(pushed) => Ok(Attempt::Landed(pushed)),
            Err(PushFailure::Transport(why)) => Ok(Attempt::SessionDied(why)),
            // This side refused, or the host did. Both are answers about
            // these bytes and this peer, and a week of retries would collect
            // the same answer every time.
            Err(local_or_denied) => Err(local_or_denied.to_string()),
        }
    }

    /// Accept a send for a device that did not answer: copy the bytes, write
    /// the record, and report exactly what was promised.
    ///
    /// Everything that can refuse runs before the copy is taken, with two
    /// exceptions, and both of them answer for the copy rather than the file:
    /// the hard size cap, which only the staged length can be measured
    /// against, and `Outbox::enqueue`, which writes the record after
    /// `stage_outbox` has staged the bytes and can still refuse — a record id
    /// it cannot claim, a cap, a directory that will not take the write.
    /// Both of those unpin the tag that staging minted. A `stage_outbox` that
    /// fails after writing its own pin is the one refusal that cannot, and the
    /// boot sweep collects that tag instead. A refusal that kept the pin would
    /// cost the
    /// user a private duplicate of their file inside this server's state
    /// directory for the life of the install.
    async fn queue_send(
        &self,
        node: &Arc<endpoint::RemoteNode>,
        peer: &Peer,
        cand: &beam::OfferCandidate,
        why: String,
    ) -> Result<Delivery, String> {
        // Re-read the peer instead of trusting the list this call started
        // from: the handshake it just tried may have refreshed the cached
        // grant, and the human may have unpaired the device in between.
        let Some(peer) = self.peers.get(&peer.node_id) else {
            return Err(format!(
                "{} is no longer paired with this server, so nothing was queued.",
                label(peer)
            ));
        };
        // The grant pre-check. A device that has not granted control refuses
        // these bytes the moment it wakes, so queuing them would keep a
        // private copy of the user's file for a week to deliver a refusal.
        // The same predicate the RECEIVER enforces answers it, so the two
        // cannot drift. Never handshaken (`None`) is not a no: that send is
        // queued, and its answer says the grant is unverified. A scope this
        // build cannot parse is not a no either (`unwrap_or(true)`): the
        // string comes off a hand-editable `peers.json`, and refusing an
        // accept on a cached hint nobody can read would lose a send that the
        // next real handshake may well permit.
        if let Some(granted) = peer.last_ack_scope.as_deref() {
            if !grants_control(granted).unwrap_or(true) {
                return Err(format!(
                    "{} Nothing was queued: a device that has not granted control refuses \
                     these bytes on arrival, and the file would sit here as a private copy \
                     until it expired. The scope above is what the last completed handshake \
                     reported; if the grant has been widened since, the send goes through as \
                     soon as that device is reachable again.",
                    needs_control(&peer, granted, &self.device)
                ));
            }
        }
        if let Some(reason) = self.queue_blocked_reason() {
            return Err(format!("the send to {} was not queued: {reason}", label(&peer)));
        }
        // The caps are checked before the copy, and again inside `enqueue`
        // over the same predicate — so a refusal never costs a duplicate of
        // the file, and the two answers cannot disagree.
        self.outbox.room_for(&label(&peer), cand.size)?;

        // ONE await copies the bytes and pins them under `outbox/<id>`. The
        // pin is the only thing keeping them: `stage_for_peer` sweeps expired
        // grants on every push, so an unpinned queued file would lose its
        // bytes an hour after it was accepted.
        let id = self.outbox.next_id();
        // BOTH halves of what the record announces come back from here, and
        // neither is taken from `cand`. `cand.size` is a `std::fs::metadata`
        // read taken before the dial this send has already spent — up to
        // `DIAL_TIMEOUT` earlier — so a file the user saved in between leaves
        // a record whose size names the file that was read and whose hash
        // names the copy that was taken. The receiver checks the announced
        // size against the transfer cap before it opens the stream and the
        // real cap on the stream itself, so an under-stated record is refused
        // mid-transfer; the drain holds a refusal, so that record retries once
        // a pass for the whole seven-day TTL and can never land.
        let staged = beam::stage_outbox(node, &cand.canonical, &id).await?;

        // The cap is asked again, of the copy this time: the gate asked it of
        // the file. `beam::resolve_offerable` reads the length with
        // `std::fs::metadata`, and the copy is taken here — after a whole dial
        // to a device that does not answer. A file that grew past the cap in
        // that window used to become a record the receiver can never take. It
        // answers `PushFailure::Denied`, the drain holds a denial rather than
        // dropping it, and that record then retries once a pass for the whole
        // seven-day TTL while holding one of this peer's `MAX_PER_PASS` push
        // slots — starving the records behind it. Refusing now costs the user
        // one answer; queuing it costs them a week of a queue that cannot move.
        //
        // One read of the cap, so the number this refusal enforces and the
        // number it prints cannot drift apart.
        let cap = self.staged_cap();
        if staged.size > cap {
            // Same cleanup the enqueue-failure arm below performs, and safe
            // for the same reason: `stage_outbox` refuses a tag the store
            // already holds, so this pin is this call's own.
            beam::unpin_outbox(node, &outbox::tag_name(&id)).await;
            return Err(format!(
                "{} is {} — beam v1 caps at {}, so the send to {} was not queued. The file \
                 was under the cap when it was read and over it when it was copied, so \
                 nothing was kept here.",
                cand.name,
                beam::human_bytes(staged.size),
                beam::human_bytes(cap),
                label(&peer)
            ));
        }

        // DEDUPE on (peer, content). The store is content-addressed, so
        // re-staging the same file costs a read and a hash pass, never a
        // second copy on disk. A model retries a call that looked like it
        // failed, and without this one user intent becomes several records.
        if let Some(pending) = self.outbox.find_pending(&peer.node_id, &staged.hash) {
            beam::unpin_outbox(node, &outbox::tag_name(&id)).await;
            return Ok(self.queued(&peer, pending, why));
        }

        let item = Staged {
            id: id.clone(),
            peer: peer.node_id.clone(),
            device: peer.device.clone(),
            source: cand.canonical.clone(),
            name: cand.name.clone(),
            size: staged.size,
            hash: staged.hash,
        };
        match self.outbox.enqueue(item) {
            Ok(record) => Ok(self.queued(&peer, record, why)),
            Err(e) => {
                // The record never reached disk, so nothing will ever name
                // this pin again. Releasing it here is what keeps a send that
                // was NOT accepted from leaving a copy of the user's file in
                // the store for the life of the install.
                //
                // THE PIN IS THIS CALL'S OWN, and only because `stage_outbox`
                // refuses a tag the store already holds. The commonest way to
                // reach this arm is `enqueue` refusing a REPEATED id, and an
                // unpin without that guard would release the incumbent
                // record's bytes — losing the very delivery the `create_new`
                // claim above is there to protect.
                beam::unpin_outbox(node, &outbox::tag_name(&id)).await;
                Err(e)
            }
        }
    }

    /// The queued answer, written for the model that has to relay it. Every
    /// note is a way this promise differs from the delivery the user asked
    /// for, and the word "queued" hides all of them.
    fn queued(&self, peer: &Peer, record: Record, why: String) -> Delivery {
        let mut notes = vec![
            format!(
                "The file was copied as it stands right now, so later edits to it do not \
                 change what {} receives.",
                peer.device
            ),
            "Delivery needs this server running. If this session ends first, the file goes \
             out at the first network-touching tool call of a later session over the same \
             state directory — not before."
                .to_string(),
            format!(
                "The copy is kept for {} days from when it was accepted, then deleted \
                 undelivered.",
                outbox::RECORD_TTL_SECS / (24 * 3600)
            ),
        ];
        if peer.last_ack_scope.is_none() {
            notes.push(format!(
                "This server has never completed a handshake with {}, so its \"control\" \
                 grant is unverified. Without that grant the device refuses the file when it \
                 wakes, and the send waits here until it expires.",
                peer.device
            ));
        }
        Delivery::Queued {
            device: peer.device.clone(),
            node_id: peer.node_id.clone(),
            name: record.name,
            size: record.size,
            hash: record.hash,
            id: record.id,
            expires_at: record.expires_at,
            reason: why,
            notes,
        }
    }

    /// Why this server can neither accept nor move a queued send, or `None`.
    ///
    /// Three causes, and each one stops the WHOLE queue rather than one send:
    /// another process holds the blob-store claim, so nothing may be copied
    /// in or replayed out; the peer list did not read, so no record can be
    /// told from a revoked one; or the spool itself is unusable — it did not
    /// read, so a record would be a promise this process could never find
    /// again, or it will not take a write, so a delivery attempt cannot even
    /// be noted. One producer, read by the send path and by `server_status`,
    /// so a refusal and the status that explains it cannot tell two different
    /// stories.
    fn queue_blocked_reason(&self) -> Option<String> {
        self.blocked_by(self.outbox.fault())
    }

    /// The half of the answer above that does not touch the spool, for a
    /// caller that already holds the spool's fault from a wider read. It
    /// exists so `server_status` can take the spool ONCE and still route the
    /// answer through the one producer, rather than spelling the `or` out a
    /// second time and letting the two orders drift.
    fn blocked_by(&self, spool_fault: Option<String>) -> Option<String> {
        self.last_boot_error()
            .or_else(|| self.peers.load_error().map(peer_list_blocked))
            .or(spool_fault)
    }

    /// Test seam: how many peer sessions this server is holding. A session
    /// evicts its own entry when it closes, so this is what tells a test the
    /// cache drains instead of growing one dead handle per probe.
    #[doc(hidden)]
    pub async fn cached_sessions(&self) -> usize {
        self.sessions.lock().await.len()
    }

    /// Test seam: the identity of the session cached for `node_id`, as the
    /// address of its `Arc`. A count cannot tell reuse from re-dialling —
    /// the map is keyed by node id, so it holds one entry per peer either
    /// way — but a stable identity across calls can.
    #[doc(hidden)]
    pub async fn cached_session_id(&self, node_id: &str) -> Option<usize> {
        self.sessions.lock().await.get(node_id).map(|s| Arc::as_ptr(s) as usize)
    }

    /// Test seam: the loopback address a device dials THIS server at. Every
    /// other two-endpoint proof drives the outbound leg, where this server
    /// dials; the inbound one — the phone coming back and connecting, which
    /// is what `HostSignal::PeerConnected` reports — needs an address to dial
    /// to. Booting is what creates it, so this boots. No tool reads it.
    #[doc(hidden)]
    pub async fn loopback_socket(&self) -> Option<SocketAddr> {
        let node = self.node().await.ok()?;
        endpoint::loopback_socket(&node).await
    }

    /// The drain's view of this server, over the booted node the caller
    /// already holds.
    ///
    /// Every field is an `Arc` clone or a cheap handle: the drain owns
    /// nothing of its own, so a `use_loopback` re-point, a session, a cached
    /// grant and the spool are the same ones the tools see. Built per call
    /// rather than stored because it can only exist once the store is open,
    /// and a type that cannot be built without a node is the cheapest way to
    /// say that a drain without the store claim is not a thing.
    fn drainer(&self, node: &Arc<endpoint::RemoteNode>) -> Drainer {
        Drainer {
            node: node.clone(),
            peers: self.peers.clone(),
            outbox: self.outbox.clone(),
            sessions: self.sessions.clone(),
            roots: self.roots.clone(),
            device: self.device.clone(),
            loopback: self.loopback.clone(),
            inflight: self.inflight.clone(),
            retry: self.retry.clone(),
        }
    }

    /// Tell the drain a pass is worth running. Never blocks and never fails
    /// the caller: a full channel already holds a pass this one would only
    /// repeat, and the tick covers whatever a dropped hint would have.
    fn wake(&self, reason: Wake) {
        let _ = self.wake.try_send(reason);
    }

    /// Test seam, and the shape the host's `PeerConnected` signal will use:
    /// this peer is reachable now, so its queued sends go out on the next
    /// pass whatever its backoff says.
    #[doc(hidden)]
    pub fn wake_drain(&self, node_id: &str) {
        self.wake(Wake::Peer(node_id.to_string()));
    }

    /// Whether a drain pass owns this queue right now — one entry per peer
    /// being drained. A pass that found nothing due is not "draining": it
    /// held the queue for no time at all, and a reader must be able to tell a
    /// queue that is moving from one that is merely stored.
    fn draining(&self) -> bool {
        !self.inflight.lock().unwrap_or_else(|p| p.into_inner()).is_empty()
    }

    /// A live session with a paired peer, with the dial's answer flattened
    /// into the one sentence every read-only caller wants. Dial failures are
    /// reported as the device being offline, which is what they almost always
    /// are.
    async fn session(&self, peer: &Peer) -> Result<Arc<ClientSession>, String> {
        let node = self.node().await?;
        self.drainer(&node).dial_session(peer).await.map_err(|e| not_reachable(peer, &e))
    }

    // ── pair_device / pair_status / confirm_pairing ────────────────────────

    /// Open pairing: mint a one-time token and the `vlerv://pair?ticket=…`
    /// link that carries it. The token dies in ten minutes.
    pub async fn pair_device(&self) -> Result<PairingInvite, String> {
        let node = self.node().await?;
        // Bounded wait for relay + discovery so the link works from another
        // network; on timeout it still carries direct addresses, which covers
        // the same-network case. Same policy the app's pairing uses.
        let _ = tokio::time::timeout(Duration::from_secs(10), node.endpoint.online()).await;

        // The crate mints it: one place decides the token, the ticket, the
        // link shape and the TTL, so this server cannot hand out a link and
        // then describe it with a different expiry or node id.
        let invite = peers::mint_invite(node.endpoint.addr(), &self.pairing, &self.device);
        let node_id = invite.node_id;
        Ok(PairingInvite {
            link: invite.link,
            ticket: invite.ticket,
            device: invite.device,
            expires_at: invite.expires_at,
            fingerprint_hint: format!(
                "Six words appear on BOTH screens once the device opens the link. Call \
                 pair_status to read this server's six words and compare them with the device's \
                 before calling confirm_pairing. This server is {} ({}).",
                self.device,
                short_id(&node_id)
            ),
            instructions: pairing_instructions(&self.device),
            node_id,
        })
    }

    /// Pairings waiting for a fingerprint comparison. Boots nothing.
    pub fn pair_status(&self) -> Vec<PendingPairing> {
        let mut pending: Vec<PendingPairing> =
            self.pairing.parked().into_iter().map(PendingPairing::from).collect();
        pending.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        pending
    }

    /// Complete or discard a pending pairing. Rejecting writes nothing to
    /// disk, so a fingerprint mismatch leaves no trace to clean up.
    ///
    /// A named `scope` is the operator's explicit grant and REPLACES whatever
    /// an already-trusted device holds, in either direction — re-pairing a
    /// control peer at "view-open" narrows it. Omitting `scope` names no
    /// grant: a new device gets the narrowest one, an existing device keeps
    /// the grant it already has.
    pub fn confirm_pairing(
        &self,
        accept: bool,
        node_id: Option<&str>,
        scope: Option<&str>,
    ) -> Result<PairingOutcome, String> {
        // The scope is checked FIRST, so a typo is refused before a parked
        // pairing is consumed.
        let granted = args::validate_optional_scope(scope)?;
        let parked = self.pairing.parked();
        let target = devices::resolve_pending(&parked, node_id)?.node_id.clone();
        drop(parked);

        let Some(pending) = self.pairing.take(&target) else {
            return Err("no pairing is waiting for confirmation".to_string());
        };
        if !accept {
            return Ok(PairingOutcome {
                paired: false,
                device: pending.device,
                node_id: pending.node_id,
                scope: None,
            });
        }
        // `confirm`, not `upsert`: this is the human's grant, so it must land
        // on disk even when it narrows an entry that is already there.
        let peer = self.peers.confirm(&pending.node_id, &pending.device, granted)?;
        Ok(PairingOutcome {
            paired: true,
            device: peer.device,
            node_id: peer.node_id,
            scope: Some(peer.scope.as_str().to_string()),
        })
    }

    // ── server_status ──────────────────────────────────────────────────────

    pub async fn server_status(&self) -> Result<ServerStatus, String> {
        let node = self.booted();
        let active_offers = node
            .as_ref()
            .map(|n| n.offers.list().into_iter().map(OfferSummary::from).collect())
            .unwrap_or_default();
        let node_id = match node.as_ref() {
            Some(n) => n.endpoint.id().to_string(),
            None => self.node_id()?,
        };
        let received = {
            let log = self.received.lock().unwrap_or_else(|p| p.into_inner());
            Received { items: log.items.clone(), total: log.total }
        };
        // ONE read of the spool for the three answers shown side by side. The
        // same argument the sum below makes: a pending list, an unaccounted
        // list and the spool's fault taken in three separate locks can
        // straddle a `reload`, and then the status names one delivery on two
        // lists, or reports an empty queue beside no reason for it.
        let spool = self.outbox.report();
        let queued: Vec<QueuedDelivery> =
            spool.records.into_iter().map(QueuedDelivery::from).collect();
        // Summed from the listed records, not read off the spool again: a
        // total that disagreed with the list beside it would send a reader
        // hunting for a record that is not there.
        let queued_bytes: u64 = queued.iter().map(|q| q.size).sum();
        // One copy backs every record that names the same content. The store
        // is content-addressed, so a file queued for the phone and the iPad
        // is two records, two pins and one blob — counting it twice would
        // report a disk cost that is not there.
        let mut counted: BTreeSet<&str> = BTreeSet::new();
        let retained_bytes: u64 =
            queued.iter().filter(|q| counted.insert(&q.hash)).map(|q| q.size).sum();
        Ok(ServerStatus {
            node_id_short: short_id(&node_id),
            node_id,
            device: self.device.clone(),
            identity_dir: self.dirs.remote(),
            state_dir: self.dirs.base().to_path_buf(),
            booted: node.is_some(),
            boot_error: node.is_none().then(|| self.last_boot_error()).flatten(),
            uptime_secs: self.started.elapsed().as_secs(),
            paired_devices: self.peers.list().len(),
            active_offers,
            received_artifacts: received.items,
            received_total: received.total,
            queued_total: queued.len(),
            queued,
            queued_bytes,
            retained_bytes,
            // Every cause in one list, including the only one that may fix
            // itself — a file that would not read this boot. All of them are
            // off the pending list, so a status that stayed silent would
            // report the delivery as simply gone.
            queue_unreadable: spool.unaccounted,
            draining: self.draining(),
            queue_blocked_reason: self.blocked_by(spool.fault),
            roots: self.roots.roots(),
        })
    }
}

// ── The drain ──────────────────────────────────────────────────────────────

/// What moves the queue: the booted node and the handles `McpCore` already
/// shares with its tools. It owns nothing, which is the point — the spool it
/// empties, the sessions it dials, the grants it caches and the loopback seam
/// it reads are the same ones a tool call sees, so a drain pass and a send
/// can never act on two different pictures of the same device.
///
/// It can only be built from an `Arc<endpoint::RemoteNode>`, and that is a
/// statement rather than a convenience: only the process holding
/// `remote/blobs.lock` may open the store, so only it may stage, re-grant or
/// replay. The store claim IS the outbox lock.
struct Drainer {
    node: Arc<endpoint::RemoteNode>,
    peers: Arc<PeerStore>,
    outbox: Arc<Outbox>,
    sessions: SessionCache,
    roots: RootSet,
    device: String,
    loopback: Arc<Mutex<Option<SocketAddr>>>,
    inflight: Arc<Mutex<HashMap<String, bool>>>,
    retry: Arc<Mutex<HashMap<String, Backoff>>>,
}

impl Drainer {
    /// The ONE dial path, with the classification `ClientSession::connect`
    /// produced still intact.
    ///
    /// `McpCore::session` flattens it into a sentence, and that sentence is
    /// where the cause stops being usable: it reads "{device} is not
    /// reachable: peer offline — …", so nothing downstream can tell a
    /// sleeping phone from a refusal without matching on prose. The send path
    /// and the drain take the variant instead, because "asleep" is the only
    /// cause a queued send may be built on, and the only one worth retrying.
    async fn dial_session(&self, peer: &Peer) -> Result<Arc<ClientSession>, ConnectError> {
        if let Some(existing) = self.sessions.lock().await.get(&peer.node_id) {
            if !existing.is_closed() {
                return Ok(existing.clone());
            }
        }
        // A peer record that cannot be turned into an address is a corrupt
        // entry in this server's own store, not a device that is asleep, so
        // it takes the refusal door and can never reach the spool.
        let addr = match self.loopback() {
            Some(host) => endpoint::addr_at(&peer.node_id, host),
            None => endpoint::addr_for(&peer.node_id),
        }
        .map_err(ConnectError::Refused)?;
        // A probe dials every paired device, and a device that later sleeps
        // leaves its session closed. Without this callback each one stays in
        // the map for the life of the process, holding a dead connection.
        let cache = self.sessions.clone();
        let evict = peer.node_id.clone();
        let on_closed = move || {
            tokio::spawn(async move {
                let mut sessions = cache.lock().await;
                // Only ever drop a CLOSED entry: a re-dial may already have
                // replaced this peer's session, and the old session's
                // callback must not evict the new one.
                if sessions.get(&evict).is_some_and(|s| s.is_closed()) {
                    sessions.remove(&evict);
                }
            });
        };
        let session =
            ClientSession::connect(&self.node, addr, self.device.clone(), |_| {}, on_closed)
                .await?;
        // What the DEVICE says it grants this server, cached the moment it
        // says it. It is read for exactly one decision — whether a send to
        // that device may be queued while it is ASLEEP, when no handshake can
        // be had — and it is a hint there too: the receiver's own filter
        // still decides what may land. A failed write costs the hint, never
        // the send, so it is reported and stepped over.
        if let Err(e) = self.peers.note_ack_scope(&peer.node_id, &session.scope) {
            eprintln!("vlerv-mcp: cannot cache the grant {} reported: {e}", label(peer));
        }
        self.sessions.lock().await.insert(peer.node_id.clone(), session.clone());
        Ok(session)
    }

    fn loopback(&self) -> Option<SocketAddr> {
        *self.loopback.lock().unwrap_or_else(|p| p.into_inner())
    }

    async fn own_loopback(&self) -> Option<SocketAddr> {
        own_loopback(self.loopback(), &self.node).await
    }

    /// What a boot owes the spool before anything else touches it.
    ///
    /// The order is load-bearing. The directory is re-read first because a
    /// previous process may have completed records after `McpCore::new` read
    /// it, and replaying those would push files that already landed. Records
    /// whose bytes are gone go next, each releasing its own pin, so the sweep
    /// that follows already has the final list of what is still owed. The
    /// sweep runs LAST and only behind `live_tags`, which answers `None` when
    /// the spool did not load: an unreadable spool has an EMPTY record map,
    /// so a sweep that ran anyway would unpin every pending send and take the
    /// files with it, with no error anywhere.
    async fn reconcile(&self) {
        self.outbox.reload();
        for record in self.outbox.list() {
            if !beam::outbox_bytes_present(&self.node, &record.hash).await {
                // Retrying this to the TTL would announce a fetch the
                // receiver can never complete, once per pass for a week.
                self.give_up(&record, "its staged copy is no longer in the blob store").await;
            }
        }
        let Some(keep) = self.outbox.live_tags() else {
            return;
        };
        let swept = beam::sweep_outbox_tags(&self.node, &keep).await;
        if swept > 0 {
            // What a process that died between claiming an id and writing its
            // record left pinned. Reported because the alternative reading —
            // that a delivery went missing — is one a reader must be able to
            // rule out.
            eprintln!("vlerv-mcp: released {swept} staged file(s) no queued send claims");
        }
    }

    /// One pass over the whole spool.
    ///
    /// `forced` is the peer a wake named, and it is dialed whatever its
    /// backoff says — a peer that just connected has answered the only
    /// question the ladder was guessing at.
    async fn pass(&self, forced: Option<&str>) {
        // Expiry first, so a record past its week is not dialed for one more
        // time on its way out. The copy of the user's file goes with it.
        for expired in self.outbox.take_expired(peers::now_unix()) {
            beam::unpin_outbox(&self.node, &expired.tag).await;
        }
        // Grouped by peer, and that is the whole shape of this pass: a dial
        // to a suspended phone costs DIAL_TIMEOUT, so five records for one
        // device must cost one dial, not five. The peers then run together,
        // the way `list_devices { probe: true }` probes, so one sleeping
        // device does not hold up a delivery to an awake one.
        let due: Vec<String> = self
            .outbox
            .pending_peers()
            .into_iter()
            .filter(|peer| forced == Some(peer.as_str()) || self.due(peer))
            .collect();
        join_all(due.iter().map(|peer| self.drain_peer(peer))).await;
    }

    /// One peer's pass, behind an in-flight + redrive guard.
    ///
    /// A second trigger for a peer already draining sets the redrive flag and
    /// returns, and the running pass loops once more. Two passes to one
    /// device would otherwise push the same record twice — the receiver lands
    /// both, under the Beam collision rule, and the user gets `report-2.html`
    /// they never asked for. The single supervisor cannot overlap passes
    /// today; this holds the moment anything else calls the drain.
    async fn drain_peer(&self, peer_id: &str) {
        {
            let mut flight = self.inflight.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(redrive) = flight.get_mut(peer_id) {
                *redrive = true;
                return;
            }
            flight.insert(peer_id.to_string(), false);
        }
        loop {
            self.peer_pass(peer_id).await;
            let mut flight = self.inflight.lock().unwrap_or_else(|p| p.into_inner());
            match flight.get_mut(peer_id) {
                // Somebody asked again while this pass was running, and the
                // answer they wanted may only be true now.
                Some(redrive) if *redrive => *redrive = false,
                _ => {
                    flight.remove(peer_id);
                    return;
                }
            }
        }
    }

    /// The steps one peer's records take, in the order the mechanism sets:
    /// still paired, reachable, granted control, then at most
    /// `MAX_PER_PASS` PUSHES — a bound, not a batch size, because a pass
    /// that tried to empty a full spool down one session would hold that
    /// device for minutes.
    ///
    /// Every one of those checks is made HERE rather than trusted from
    /// enqueue time: a record can be days old, and the pairing, the grant and
    /// the root set have all had that long to change.
    async fn peer_pass(&self, peer_id: &str) {
        let records = self.outbox.for_peer(peer_id);
        if records.is_empty() {
            return;
        }
        // A peer list that did not load answers `None` for every device on
        // this machine, and the revocation branch below would read that as an
        // unpairing: every record for every peer removed, every staged copy
        // unpinned, and the collector free to take the bytes within the
        // minute. One EACCES on peers.json — or one file written by a newer
        // build — would empty the whole queue with nothing on any surface
        // saying so. The same rule `live_tags` states for the spool and
        // `save_list` states for the peer file itself: what this build cannot
        // read is HIDDEN, not gone.
        if let Some(reason) = self.peers.load_error() {
            self.hold_peer(peer_id, &records, peer_list_blocked(reason));
            return;
        }
        // The device was unpaired since the send was accepted. Revocation
        // stays immediate: the bytes go now, not at the TTL.
        let Some(peer) = self.peers.get(peer_id) else {
            for record in &records {
                self.give_up(record, "the device is no longer paired with this server").await;
            }
            self.retry.lock().unwrap_or_else(|p| p.into_inner()).remove(peer_id);
            return;
        };
        let session = match self.dial_session(&peer).await {
            Ok(session) => session,
            // Asleep or refusing — for the queue these are the same move:
            // say why on every record and try the whole peer again later.
            // Only the wording differs, and it is the dial's own.
            Err(cause) => {
                self.hold_peer(peer_id, &records, not_reachable(&peer, &cause));
                return;
            }
        };
        // The grant as the DEVICE reports it, re-read on every pass. A record
        // queued for a device that has since narrowed its grant must say so
        // rather than collect "not permitted for this peer" forever. A scope
        // this build cannot parse HOLDS the record (`unwrap_or(false)`) — it
        // is neither pushed nor dropped, so a newer build that understands
        // the string still delivers it inside the TTL.
        if !grants_control(&session.scope).unwrap_or(false) {
            self.hold_peer(peer_id, &records, needs_control(&peer, &session.scope, &self.device));
            return;
        }

        let mut landed = false;
        // Both are INVARIANT for this pass, and both were rebuilt per record.
        // The loopback answer is this node's own bound port, which no record
        // can change; the roots sentence takes the `RootSet` lock, clones
        // every root and joins them, and `record_attempt` then usually
        // discards the string because it equals the reason already stored.
        // The record loop is bounded only for PUSHES, so a peer whose records
        // are all held paid both costs once per record, every pass, for the
        // seven days the records live.
        let own = self.own_loopback().await;
        let listed = listed_roots(&self.roots);
        // Pushes made, and it is COUNTED rather than taken off the front of
        // the list. `MAX_PER_PASS` exists to stop one peer holding a session
        // for minutes, and only a record that reaches `push_staged` costs
        // that session anything — a held or dropped record costs a string
        // compare. Spending the budget on skipped records starved every
        // deliverable record behind them: the state directory is shared by
        // every Claude Code session while VLERV_MCP_ROOTS is per project, so
        // eight records another project queued for the same phone sit at the
        // head of this peer's queue as a matter of course, and the ninth
        // waited out the whole seven-day TTL behind them.
        let mut pushes = 0usize;
        for record in &records {
            if pushes >= outbox::MAX_PER_PASS {
                break;
            }
            // A purely LEXICAL prefix test against the roots, never a fresh
            // canonicalize: the bytes were captured at enqueue, so a snapshot
            // stays deliverable after the source file is deleted. The record
            // is HELD — not sent, not dropped — because VLERV_MCP_ROOTS
            // differs between projects and a session started elsewhere can
            // still serve it.
            if !self.roots.contains(&record.source) {
                self.note(record, held_outside_roots(record, &listed));
                continue;
            }
            // `push_staged` mints a ticket for whatever hash it is handed, so
            // a record whose bytes went would announce a fetch the receiver
            // can never finish.
            if !beam::outbox_bytes_present(&self.node, &record.hash).await {
                self.give_up(record, "its staged copy is no longer in the blob store").await;
                continue;
            }
            // Past both skips, so this record is about to spend a session
            // round trip. That — not the loop iteration — is what the bound
            // counts.
            pushes += 1;
            // The STAGED bytes, named by content address: nothing re-reads
            // the source, which by now may have been rewritten or deleted.
            let pushed = match own {
                Some(own) => {
                    session.push_staged_at(&record.hash, &record.name, record.size, own).await
                }
                None => session.push_staged(&record.hash, &record.name, record.size).await,
            };
            match pushed {
                Ok(_) => {
                    // The RECORD FILE goes first, then the pin. A crash
                    // between the two leaks a tag, which the next boot sweep
                    // collects; the other order leaves a record whose bytes
                    // are gone and a delivery that can never succeed.
                    match self.outbox.complete(&record.id) {
                        Ok(_) => {
                            beam::unpin_outbox(&self.node, &record.tag).await;
                            landed = true;
                        }
                        // The bytes ARE on the other machine. Reporting the
                        // record as still pending is the honest answer, and
                        // the duplicate the next pass sends lands visibly as
                        // "report-2.html" rather than silently.
                        Err(e) => eprintln!(
                            "vlerv-mcp: {:?} reached {} but its record could not be \
                             removed ({e}) — it will be sent again",
                            record.name, record.device
                        ),
                    }
                }
                // The session died under the request, so nothing more is
                // going out over it. Dropping it from the cache is what makes
                // the next pass dial instead of reusing a handle whose
                // `is_closed()` flag has not caught up.
                Err(PushFailure::Transport(why)) => {
                    forget_session(&self.sessions, peer_id).await;
                    self.hold_peer(peer_id, &records, why);
                    return;
                }
                // This side refused these bytes, or the host did. Both are
                // answers about ONE record, so the next record still gets its
                // turn on this session.
                Err(refused) => self.note(record, refused.to_string()),
            }
        }
        if landed {
            // The device took a file, so whatever the ladder had learned
            // about it is stale. Anything left over goes at the next tick.
            self.retry.lock().unwrap_or_else(|p| p.into_inner()).remove(peer_id);
        } else {
            // NOTHING MOVED, so this peer is stalled like any other and the
            // ladder is what says so. Stepping it only on a failed dial or a
            // dead session left the reachable-but-held peer — every record
            // outside this session's roots, or refused one at a time — with
            // no ladder entry at all, so `due` answered true forever and the
            // pass ran again at every `DRAIN_TICK`: 1440 dials a day at a
            // device that had already given its answer.
            self.back_off(peer_id);
        }
    }

    /// End this peer's pass with one stated reason on every record it owns,
    /// and step the ladder. `record_attempt` on an id that is already gone is
    /// a no-op, so a record this pass delivered a moment ago is not
    /// resurrected by the failure that ended the pass.
    fn hold_peer(&self, peer_id: &str, records: &[Record], why: String) {
        for record in records {
            self.note(record, why.clone());
        }
        self.back_off(peer_id);
    }

    /// Write what happened to one record. The count and the sentence are what
    /// `server_status` shows a human: a record that is not moving has a
    /// reason, and a surface that shows the count without it is the silent
    /// failure this queue exists to remove.
    ///
    /// A write that does not land keeps that promise too, and not through
    /// this log line: the spool holds the reason and reports it as its own
    /// fault, so the status names the unwritable queue instead of showing a
    /// record that reads "not tried yet" after every pass has tried it.
    fn note(&self, record: &Record, why: String) {
        if let Err(e) = self.outbox.record_attempt(&record.id, Some(why)) {
            eprintln!("vlerv-mcp: cannot record an attempt on {}: {e}", record.id);
        }
    }

    /// Give up on one record: the file goes, then its pin. The reason is
    /// printed by the spool, because a delivery somebody was promised is
    /// ending here.
    async fn give_up(&self, record: &Record, reason: &str) {
        match self.outbox.drop_record(&record.id, reason) {
            Ok(Some(dropped)) => beam::unpin_outbox(&self.node, &dropped.tag).await,
            Ok(None) => {}
            Err(e) => eprintln!(
                "vlerv-mcp: cannot drop the queued send of {:?} to {}: {e}",
                record.name, record.device
            ),
        }
    }

    /// Step this peer one rung down the retry ladder.
    fn back_off(&self, peer_id: &str) {
        let mut ladder = self.retry.lock().unwrap_or_else(|p| p.into_inner());
        let entry = ladder
            .entry(peer_id.to_string())
            .or_insert(Backoff { failures: 0, next: Instant::now() });
        entry.failures = entry.failures.saturating_add(1);
        entry.next = Instant::now() + retry_delay(entry.failures);
    }

    /// May this peer be dialed on this pass? A peer nothing has failed for is
    /// always due, which is what makes the boot pass try everything at once.
    fn due(&self, peer_id: &str) -> bool {
        self.retry
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(peer_id)
            .is_none_or(|backoff| backoff.next <= Instant::now())
    }
}

/// The one drain task, spawned by the boot that claimed the blob store.
///
/// One serialized task means two passes cannot overlap by construction. The
/// tick is GUARDED by a non-empty spool, so an idle server costs zero
/// wakeups — the `Wake::Spool` hint is what tells it the spool stopped being
/// empty, and the sender lives on `McpCore`, so this task ends with the
/// server it serves rather than holding the store claim after it.
async fn supervise(drainer: Drainer, mut wake: mpsc::Receiver<Wake>) {
    // The boot pass, before anything asks: it delivers the work a dead
    // process left behind, which is the whole promise made to the user whose
    // send outlived the session that accepted it.
    drainer.pass(None).await;
    loop {
        let forced = tokio::select! {
            woken = wake.recv() => match woken {
                Some(Wake::Peer(peer)) => Some(peer),
                Some(Wake::Spool) => None,
                // Every sender is gone, so nothing will ever queue again and
                // this process is finished with the spool. This task holds
                // the last reference to the node, so handing the store over
                // is its last act — and it has to SAY so: dropping a node
                // whose store runs a collector releases the lock file but
                // leaves redb open, and the next opener inside this process
                // would hang on a store the lock says is free.
                None => return drainer.node.shutdown().await,
            },
            _ = tokio::time::sleep(outbox::DRAIN_TICK), if !drainer.outbox.is_empty() => None,
        };
        drainer.pass(forced.as_deref()).await;
    }
}

/// How long a peer that would not take its queued sends is left alone.
fn retry_delay(failures: u32) -> Duration {
    let rung = (failures.max(1) as usize - 1).min(RETRY_LADDER.len() - 1);
    Duration::from_secs(RETRY_LADDER[rung])
}

/// Drop a cached session so the next call re-handshakes. A free function over
/// the shared map: the send path and the drain both do this for the same
/// reason, and `is_closed()` is a cached flag rather than a probe, so neither
/// may leave a stale handle for the other to find.
async fn forget_session(sessions: &SessionCache, node_id: &str) {
    sessions.lock().await.remove(node_id);
}

/// This server's own `127.0.0.1:<port>`, and only in loopback mode — the
/// address a pushed ticket names so the peer dials back over loopback.
///
/// A free function over the two handles the answer needs — the `use_loopback`
/// pin and the booted node — so the send path and the drain share ONE
/// resolution while each reads the pin through the handle it already holds.
async fn own_loopback(pin: Option<SocketAddr>, node: &endpoint::RemoteNode) -> Option<SocketAddr> {
    pin?;
    endpoint::loopback_socket(node).await
}

/// What a record whose source has left this server's roots is told.
///
/// It is HELD, not dropped: `VLERV_MCP_ROOTS` differs between projects, so a
/// delivery accepted in one may only ever be served by a session started in
/// another. Saying which roots refused it is the difference between a user
/// who can start that session and one who watches a count that never moves.
fn held_outside_roots(record: &Record, listed: &str) -> String {
    format!(
        "held, not sent: {:?} is outside this server's send roots ({listed}), so it is neither \
         delivered nor dropped. A Claude Code session whose VLERV_MCP_ROOTS covers that file \
         delivers this record; otherwise it waits here until it expires.",
        record.source
    )
}

/// This server's send roots as the sentence above names them. Split out
/// because only `record.source` varies per record: taking the `RwLock`,
/// cloning every `PathBuf` and re-joining them once per HELD record was the
/// same string rebuilt for every record in the spool.
fn listed_roots(roots: &RootSet) -> String {
    roots.roots().iter().map(|root| format!("{root:?}")).collect::<Vec<_>>().join(", ")
}

/// Does this scope STRING let the device land artifacts? `None` means the
/// string parsed as nothing this build knows.
///
/// Both strings this answers about are outside this process's control — the
/// scope a peer reports in its `HelloAck`, and `last_ack_scope`, which is
/// plain text in a hand-editable `peers.json` — so "unparseable" is a real
/// input, not a theoretical one. It gets no default HERE on purpose: the
/// three callers want opposite ones (the accept site queues, the live push
/// and the drain refuse), and three inline spellings of the fallback are how
/// one of them silently stopped matching the other two. Each caller states
/// its own `unwrap_or` and says why.
fn grants_control(scope: &str) -> Option<bool> {
    Scope::parse(scope).ok().map(Scope::may_land_artifacts)
}

/// The refusal a device that has not granted this server "control" gets.
///
/// One producer, two callers: the live check against the scope in a
/// handshake, and the queue-time pre-check against the scope the last
/// handshake reported. The same human has to act on both, and two spellings
/// of one instruction are how one of them stops naming the peer to widen.
fn needs_control(peer: &Peer, scope: &str, this_device: &str) -> String {
    format!(
        "{} has not granted this server control. It paired this server at scope {scope:?}; \
         pushing a file needs \"control\". On that device, open its Vlervtifacts peer \
         settings, find \"{this_device}\", and set its scope to \"control\".",
        label(peer)
    )
}

/// What every record owned by a peer this server cannot look up is told, and
/// what `server_status` reports as the reason the whole queue is stopped.
///
/// One producer for both, because they describe one condition. A `PeerStore`
/// that did not load holds NO peers, so `get` answers `None` for every device
/// on the machine and nothing downstream can tell a device that was unpaired
/// from one that is merely hidden behind an unreadable file. The queue is HELD
/// on that answer: the peer list is the authority for revocation, and reading
/// its absence as a revocation deletes the records and unpins the copies of
/// the user's files behind them.
fn peer_list_blocked(reason: &str) -> String {
    format!(
        "held, not sent: this server cannot read its own peer list ({reason}), so it cannot \
         tell a device that was unpaired from one it simply cannot see. Repair that file or \
         move it aside; queued sends wait until it reads."
    )
}

/// The sentence a peer that would not talk produces, wherever the send path
/// gives up on reaching it. `ConnectError` prints its own cause verbatim, so
/// this wraps it without touching a word of it.
fn not_reachable(peer: &Peer, cause: &ConnectError) -> String {
    format!(
        "{} is not reachable: {cause}. Check that the device is awake, on a network, \
         and that Vlervtifacts is running on it.",
        label(peer)
    )
}

/// The name this server announces in every handshake and pairing ticket.
/// Names the tool, not the machine, so a human confirming a fingerprint can
/// tell it apart from the Vlervtifacts app on the same Mac.
pub fn device_name() -> String {
    vlerv_remote::proto::sanitize_device(&format!(
        "Claude Code @ {}",
        vlerv_remote::device_name()
    ))
}

/// Where this server keeps its identity, peer store and blobs. Its OWN
/// subdirectory: the desktop app's `remote/` must not be shared, because the
/// two are different peers with different keys.
///
/// Precedence, as README-MCP.md documents it: `VLERV_MCP_STATE_DIR` names the
/// directory outright; otherwise `VLERV_STATE_DIR` (or the platform config
/// directory plus `Vlerv`) names the app's base and this server takes its
/// `mcp/` subdirectory. The platform lookup is `dirs::config_dir`, the same
/// one `state_store::state_dir` uses on the app side, so the two agree about
/// where `~/Library/Application Support` is without either of them spelling
/// it out.
pub fn state_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("VLERV_MCP_STATE_DIR") {
        return PathBuf::from(dir);
    }
    let base = match std::env::var_os("VLERV_STATE_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("Vlerv"),
    };
    base.join("mcp")
}

/// The root set a sent file is resolved against: `VLERV_MCP_ROOTS` (a
/// colon-separated list) when set, otherwise the working directory Claude Code
/// launched this server in.
///
/// This IS the send boundary — `McpCore::gate_arg_path` confines every path
/// argument to it, so an operator who narrows the list narrows what a
/// prompt-injected caller can ever address. The set is never empty, because an
/// empty one refuses everything and a server that can send nothing is a
/// confusing failure rather than a safe default.
pub fn configured_roots(cwd: &Path) -> Vec<PathBuf> {
    match std::env::var("VLERV_MCP_ROOTS") {
        Ok(raw) => {
            let roots: Vec<PathBuf> =
                raw.split(':').map(str::trim).filter(|s| !s.is_empty()).map(PathBuf::from).collect();
            if roots.is_empty() {
                vec![cwd.to_path_buf()]
            } else {
                roots
            }
        }
        Err(_) => vec![cwd.to_path_buf()],
    }
}

/// The human half of pairing, written for the model to read out loud.
fn pairing_instructions(device: &str) -> Vec<String> {
    vec![
        "Give the link to the person at the other device — it is a capability, so send it over a channel you trust.".to_string(),
        "On that device, open the link. On iOS, tap it; on a Mac running Vlervtifacts, click it.".to_string(),
        "Both screens now show six words. They must match. If they differ, reject the pairing.".to_string(),
        "Call pair_status to read this server's six words, then confirm_pairing { accept: true } to finish.".to_string(),
        "The link expires 10 minutes after it is minted, and one link pairs one device.".to_string(),
        format!(
            "Grant direction: confirm_pairing decides what the DEVICE may do here. For this server \
             to push files to the device, the DEVICE must grant \"{device}\" the \"control\" scope \
             in its own peer settings. Without that, send_to_device is refused."
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_device_name_says_which_tool_it_is() {
        let name = device_name();
        assert!(name.starts_with("Claude Code @ "), "{name}");
        // Already sanitized, so it survives the wire's device-name check.
        assert_eq!(name, vlerv_remote::proto::sanitize_device(&name));
    }

    #[test]
    fn the_state_dir_is_never_the_apps_own_remote_directory() {
        // Both derive from the same base, but the MCP server is a separate
        // peer with a separate identity key.
        let mcp = Dirs::new(state_dir());
        assert!(mcp.base().ends_with("mcp"), "{:?}", mcp.base());
        assert_ne!(mcp.remote(), Dirs::new(mcp.base().parent().unwrap()).remote());
    }

    #[test]
    fn roots_default_to_the_working_directory() {
        assert_eq!(configured_roots(Path::new("/w/p")), vec![PathBuf::from("/w/p")]);
    }

    #[test]
    fn a_core_binds_nothing_until_a_tool_needs_it() {
        let dir = tempfile::TempDir::new().unwrap();
        let core = McpCore::new(
            dir.path().to_path_buf(),
            vec![dir.path().to_path_buf()],
            dir.path().to_path_buf(),
            None,
        );
        // Reading the identity is a file read, not a connection.
        let id = core.node_id().unwrap();
        assert_eq!(id.len(), 64, "a node id is 32 hex-encoded bytes");
        assert_eq!(core.node_id().unwrap(), id, "the key is reused, never regenerated");
        assert!(dir.path().join("remote/identity.key").is_file());
        assert!(core.pair_status().is_empty());
    }

    #[test]
    fn confirming_a_pairing_nobody_started_is_an_error_with_a_next_step() {
        let dir = tempfile::TempDir::new().unwrap();
        let core = McpCore::new(
            dir.path().to_path_buf(),
            vec![dir.path().to_path_buf()],
            dir.path().to_path_buf(),
            None,
        );
        let err = core.confirm_pairing(true, None, None).unwrap_err();
        assert!(err.contains("pair_device"), "{err}");
        // An invalid scope is refused before the pairing lookup, so a typo
        // never consumes a parked pairing.
        assert!(core.confirm_pairing(true, None, Some("admin")).unwrap_err().contains("admin"));
    }

    /// A parked pairing, the shape `PairServer` signals and `McpSink` stores.
    fn park(core: &McpCore, node_id: &str, device: &str) {
        core.pairing.park(PendingPair {
            node_id: node_id.to_string(),
            device: device.to_string(),
            fingerprint: vec!["acid".to_string(); 6],
            role: "host".to_string(),
            created_at: peers::now_unix(),
        });
    }

    fn core_in(dir: &tempfile::TempDir) -> McpCore {
        McpCore::new(
            dir.path().to_path_buf(),
            vec![dir.path().to_path_buf()],
            dir.path().to_path_buf(),
            None,
        )
    }

    #[test]
    fn re_pairing_at_a_narrower_scope_actually_narrows_the_stored_grant() {
        let dir = tempfile::TempDir::new().unwrap();
        let core = core_in(&dir);
        let node = "cd".repeat(32);

        park(&core, &node, "Val's iPhone");
        let wide = core.confirm_pairing(true, None, Some("control")).unwrap();
        assert_eq!(wide.scope.as_deref(), Some("control"));

        // The whole point: the operator re-pairs the SAME device and names a
        // narrower scope. Before `confirm`, this answered "control" AND left
        // "control" on disk — the report agreed with the store, and the
        // narrowing the human asked for was dropped with no error at all.
        // Both halves are asserted here: what was reported, and what landed.
        park(&core, &node, "Val's iPhone");
        let narrow = core.confirm_pairing(true, None, Some("view-open")).unwrap();
        assert_eq!(narrow.scope.as_deref(), Some("view-open"));
        assert_eq!(core.peer_store().get(&node).unwrap().scope, Scope::ViewOpen);

        // Naming NO scope says nothing about the grant, so it is left alone
        // rather than reset to the default.
        core.peer_store().set_scope(&node, Scope::Browse).unwrap();
        park(&core, &node, "Val's iPhone");
        let kept = core.confirm_pairing(true, None, None).unwrap();
        assert_eq!(kept.scope.as_deref(), Some("browse"));

        // A NEW device with no scope named still lands on the narrowest one.
        let other = "ef".repeat(32);
        park(&core, &other, "iPad");
        assert_eq!(core.confirm_pairing(true, None, None).unwrap().scope.as_deref(), Some("view-open"));
    }

    #[test]
    fn rejecting_a_pairing_leaves_an_existing_grant_untouched() {
        let dir = tempfile::TempDir::new().unwrap();
        let core = core_in(&dir);
        let node = "cd".repeat(32);
        park(&core, &node, "Val's iPhone");
        core.confirm_pairing(true, None, Some("control")).unwrap();

        park(&core, &node, "Val's iPhone");
        let outcome = core.confirm_pairing(false, None, Some("view-open")).unwrap();
        assert!(!outcome.paired);
        assert_eq!(outcome.scope, None);
        assert_eq!(
            core.peer_store().get(&node).unwrap().scope,
            Scope::Control,
            "a rejected pairing writes nothing — it must not narrow the peer either"
        );
    }

    #[test]
    fn the_received_list_is_bounded_and_still_reports_the_true_count() {
        let received = Arc::new(Mutex::new(Received::default()));
        let (wake, _drain) = mpsc::channel(WAKE_DEPTH);
        let sink = McpSink {
            pairing: Arc::new(Pairing::new()),
            received: received.clone(),
            wake: wake.downgrade(),
        };
        let pushes = MAX_RECEIVED + 5;
        for i in 0..pushes {
            sink.emit(HostSignal::ArtifactReceived {
                peer: "ab".repeat(32),
                path: PathBuf::from(format!("/tmp/a{i}.html")),
                name: format!("a{i}.html"),
                size: 1,
                hash: format!("{i:064x}"),
            });
        }
        let kept = received.lock().unwrap();
        assert_eq!(kept.items.len(), MAX_RECEIVED, "the vector never grows past the cap");
        // The OLDEST entries go, so what is listed is what just landed.
        assert_eq!(kept.items.first().unwrap().name, format!("a{}.html", pushes - MAX_RECEIVED));
        assert_eq!(kept.items.last().unwrap().name, format!("a{}.html", pushes - 1));
        assert_eq!(kept.total, pushes as u64, "the count is not capped");
    }

    #[test]
    fn a_peer_that_dials_in_wakes_the_drain_and_no_other_signal_does() {
        // The precise trigger, and the reason the retry ladder is allowed to
        // start at a whole minute: a device that dialed IN has answered the
        // question the ladder was waiting on, so its queued sends go out on
        // this pass instead of a later one. The other three signals say
        // nothing about whether anything is reachable, and a pass they
        // provoked would be one more n0 discovery lookup for the same answer.
        let (wake, mut passes) = mpsc::channel(WAKE_DEPTH);
        let sink = McpSink {
            pairing: Arc::new(Pairing::new()),
            received: Arc::new(Mutex::new(Received::default())),
            wake: wake.downgrade(),
        };
        let phone = "ab".repeat(32);

        sink.emit(HostSignal::PeerConnected {
            peer: phone.clone(),
            device: "Val's iPhone".into(),
            scope: "control".into(),
        });
        let Ok(Wake::Peer(woken)) = passes.try_recv() else {
            panic!("a peer holding a session must force a pass");
        };
        assert_eq!(woken, phone, "and force it for THAT peer, not for the whole spool");

        sink.emit(HostSignal::OpenOnHost {
            peer: phone.clone(),
            path: PathBuf::from("/tmp/a.html"),
            reader_mode: false,
        });
        sink.emit(HostSignal::ArtifactReceived {
            peer: phone.clone(),
            path: PathBuf::from("/tmp/b.html"),
            name: "b.html".into(),
            size: 1,
            hash: "cd".repeat(32),
        });
        assert!(passes.try_recv().is_err(), "nothing else claims a device is reachable");
    }

    #[tokio::test]
    async fn the_hosts_own_sink_never_holds_the_wake_channel_open() {
        // This sink is owned by the `ScopeState` inside the `RemoteNode` the
        // supervisor task holds. A STRONG sender in it would close the loop:
        // `supervise` would never read `None`, so it would never return,
        // never drop the node, and never release the blob-store claim — and
        // the next process over the same state directory would be refused for
        // as long as this one lived. The two restart tests in
        // tests/tool_handlers.rs fail on exactly that.
        let (wake, mut passes) = mpsc::channel::<Wake>(WAKE_DEPTH);
        let sink = McpSink {
            pairing: Arc::new(Pairing::new()),
            received: Arc::new(Mutex::new(Received::default())),
            wake: wake.downgrade(),
        };
        drop(wake);
        assert!(passes.recv().await.is_none(), "the last STRONG sender ends the supervisor");

        // And the host survives its drain: a peer connecting after the queue
        // is gone drops a hint, rather than panicking on the accept path.
        sink.emit(HostSignal::PeerConnected {
            peer: "ab".repeat(32),
            device: "Val's iPhone".into(),
            scope: "control".into(),
        });
    }

    #[test]
    fn confirm_pairing_never_guesses_which_pairing_a_node_id_argument_means() {
        let dir = tempfile::TempDir::new().unwrap();
        let core = McpCore::new(
            dir.path().to_path_buf(),
            vec![dir.path().to_path_buf()],
            dir.path().to_path_buf(),
            None,
        );
        // Two node ids that share a long prefix — the case a prefix argument
        // cannot decide between.
        park(&core, &"ab".repeat(32), "Phone");
        park(&core, &format!("{}cdcd", "ab".repeat(30)), "Laptop");

        // An empty argument used to match the first parked pairing through
        // `starts_with("")` and grant a scope to whichever one came first.
        let err = core.confirm_pairing(true, Some(""), None).unwrap_err();
        assert!(err.contains("at least"), "{err}");
        assert!(core.confirm_pairing(true, Some("  a "), None).unwrap_err().contains("at least"));
        assert_eq!(core.pair_status().len(), 2, "a refusal consumes nothing");

        // A prefix both pairings share names neither of them.
        let err = core.confirm_pairing(true, Some("abab"), None).unwrap_err();
        assert!(err.contains("Phone"), "{err}");

        // A prefix that names exactly one still works.
        let long = format!("{}cd", "ab".repeat(30));
        let outcome = core.confirm_pairing(true, Some(&long), Some("browse")).unwrap();
        assert!(outcome.paired);
        assert_eq!(outcome.device, "Laptop");
        assert_eq!(outcome.scope.as_deref(), Some("browse"));
        assert_eq!(core.pair_status().len(), 1, "only the named pairing was resolved");
    }

    #[tokio::test]
    async fn a_beam_link_can_only_ever_address_a_file_under_the_roots() {
        let state = tempfile::TempDir::new().unwrap();
        let workspace = tempfile::TempDir::new().unwrap();
        let elsewhere = tempfile::TempDir::new().unwrap();
        let secret = elsewhere.path().join("id_rsa");
        std::fs::write(&secret, "PRIVATE KEY").unwrap();

        let core = McpCore::new(
            state.path().to_path_buf(),
            vec![workspace.path().to_path_buf()],
            workspace.path().to_path_buf(),
            Some(elsewhere.path().to_path_buf()),
        );

        // The caller of an MCP tool is a language model, and its arguments can
        // be steered by text it merely read. An out-of-root file is refused
        // with the one no-existence-leak string, and no socket opens to say so.
        for arg in [
            secret.to_str().unwrap().to_string(),
            "~/id_rsa".to_string(),
            format!("../{}/id_rsa", elsewhere.path().file_name().unwrap().to_string_lossy()),
        ] {
            assert_eq!(
                core.beam_artifact(&arg, None).await.unwrap_err(),
                "path not found or out of root",
                "{arg} must not be beamable"
            );
        }
        assert!(!core.server_status().await.unwrap().booted, "a refusal binds nothing");
    }

    /// An endpoint that answers the transport and speaks no scope protocol.
    /// Why the proofs stand a deaf device up rather than a suspended one is
    /// written out at `boot_asleep` in tests/tool_handlers.rs: a real
    /// suspension costs `DIAL_TIMEOUT` — thirty seconds — and this fails the
    /// same `endpoint::dial` in milliseconds on the same
    /// `ConnectError::Unreachable`.
    ///
    /// This twin keeps the identity its own boot mints, and copies none in.
    /// No proof in this module ever brings the device back, so nothing has to
    /// answer for the same node id a second time.
    async fn deaf_device(dir: &tempfile::TempDir) -> Arc<endpoint::RemoteNode> {
        Arc::new(endpoint::boot(&Dirs::new(dir.path()), None, |_| {}).await.unwrap())
    }

    /// A core whose every dial goes to `device`, with that device already
    /// paired the way `confirm_pairing` writes it.
    async fn core_paired_with(
        state: &tempfile::TempDir,
        workspace: &tempfile::TempDir,
        device: &Arc<endpoint::RemoteNode>,
    ) -> McpCore {
        let core = McpCore::new(
            state.path().to_path_buf(),
            vec![workspace.path().to_path_buf()],
            workspace.path().to_path_buf(),
            None,
        );
        core.use_loopback(endpoint::loopback_socket(device).await.unwrap());
        core.peer_store()
            .seed(&device.endpoint.id().to_string(), "Val's iPhone", Scope::Control)
            .unwrap();
        core
    }

    /// The same core wired to an AWAKE device, and booted: what every drain
    /// proof needs before it can lay out a queue.
    ///
    /// A SECOND helper rather than an argument on the one above, because the
    /// order cannot be shared: `awake_device` seeds its grant for THIS core's
    /// node id, so the core has to exist before the device does, while
    /// `core_paired_with` takes the device as an argument.
    ///
    /// Hands back, in order: the core, its booted node (what `queue_staged`
    /// and `drainer` take), the phone, the phone's signal log (what landed on
    /// it), and the peer id the drain is driven by.
    async fn core_draining_to(
        state: &tempfile::TempDir,
        workspace: &tempfile::TempDir,
        phone_dir: &tempfile::TempDir,
    ) -> (
        McpCore,
        Arc<endpoint::RemoteNode>,
        Arc<endpoint::RemoteNode>,
        Arc<Mutex<Vec<HostSignal>>>,
        String,
    ) {
        let core = McpCore::new(
            state.path().to_path_buf(),
            vec![workspace.path().to_path_buf()],
            workspace.path().to_path_buf(),
            None,
        );
        let (phone, signals) = awake_device(phone_dir, &core.node_id().unwrap()).await;
        let phone_id = phone.endpoint.id().to_string();
        core.use_loopback(endpoint::loopback_socket(&phone).await.unwrap());
        core.peer_store().seed(&phone_id, "Val's iPhone", Scope::Control).unwrap();
        let node = core.node().await.unwrap();
        (core, node, phone, signals, phone_id)
    }

    /// The same device AWAKE: a headless `vlerv-remote` host that already
    /// grants `grants_to` control, which is the shape a Vlervcode instance
    /// presents to the wire. `deaf_device` is its sleeping twin, and the two
    /// exist for opposite proofs — that one is what the drain does when it
    /// cannot reach a peer, this one what it does when it CAN.
    async fn awake_device(
        dir: &tempfile::TempDir,
        grants_to: &str,
    ) -> (Arc<endpoint::RemoteNode>, Arc<Mutex<Vec<HostSignal>>>) {
        let peers = Arc::new(PeerStore::load(dir.path()));
        peers.seed(grants_to, "Claude Code", Scope::Control).unwrap();
        let signals: Arc<Mutex<Vec<HostSignal>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = signals.clone();
        let state = Arc::new(ScopeState::new(
            peers,
            Arc::new(Pairing::new()),
            Arc::new(TabsCache::new()),
            // A push reads nothing on the receiving side, so the receiver
            // needs no workspace at all.
            RootSet::empty(),
            "Val's iPhone".to_string(),
            Arc::new(EmptyCatalog),
            move |signal| sink.lock().unwrap_or_else(|p| p.into_inner()).push(signal),
        ));
        let node = endpoint::boot(&Dirs::new(dir.path()), Some(state), |_| {})
            .await
            .expect("the device is up");
        (Arc::new(node), signals)
    }

    /// The names a device landed, in arrival order.
    fn received(signals: &Arc<Mutex<Vec<HostSignal>>>) -> Vec<String> {
        signals
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter_map(|s| match s {
                HostSignal::ArtifactReceived { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect()
    }

    /// Stage one file into this server's own store and write the record that
    /// claims it: `send_to_device` without the dial, so a test can lay out an
    /// exact queue for one peer in an exact order. The id carries the
    /// millisecond it was minted at, so records enqueued here in sequence are
    /// the order the drain reads them in.
    async fn queue_staged(
        core: &McpCore,
        node: &Arc<endpoint::RemoteNode>,
        peer: &str,
        file: &Path,
    ) -> String {
        let id = core.outbox.next_id();
        // The record keeps the path the GATE resolved, and the roots are
        // canonical too, so an uncanonicalized source would be held here for
        // a reason the drain never meant.
        let source = file.canonicalize().unwrap();
        let staged = beam::stage_outbox(node, &source, &id).await.expect("staged bytes");
        core.outbox
            .enqueue(Staged {
                id: id.clone(),
                peer: peer.to_string(),
                device: "Val's iPhone".to_string(),
                name: source.file_name().unwrap().to_string_lossy().into_owned(),
                // Measured off the copy, exactly as `queue_send` does it.
                size: staged.size,
                hash: staged.hash,
                source,
            })
            .expect("the record is written");
        id
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_held_head_of_queue_never_starves_the_record_behind_it() {
        // One state directory serves every Claude Code session on this
        // machine, while VLERV_MCP_ROOTS is per project — so records another
        // project queued for the same phone sit at the head of this session's
        // view of that peer as a matter of course. They are HELD here, and a
        // per-pass budget spent on records that are only SKIPPED left the one
        // record this session can actually serve unreachable behind them for
        // the whole seven-day TTL, with nothing on any surface saying so.
        let state = tempfile::TempDir::new().unwrap();
        let phone_dir = tempfile::TempDir::new().unwrap();
        let workspace = tempfile::TempDir::new().unwrap();
        let other_project = tempfile::TempDir::new().unwrap();

        let (core, node, phone, signals, phone_id) =
            core_draining_to(&state, &workspace, &phone_dir).await;

        // One more than the bound, so a budget spent per record looked at
        // runs out before the queue does.
        for n in 0..=outbox::MAX_PER_PASS {
            let file = other_project.path().join(format!("held-{n}.html"));
            std::fs::write(&file, format!("<h1>queued in another project: {n}</h1>")).unwrap();
            queue_staged(&core, &node, &phone_id, &file).await;
        }
        // And the record this session queued, behind every one of them.
        let mine = workspace.path().join("report.html");
        let body = "<h1>queued right here</h1>";
        std::fs::write(&mine, body).unwrap();
        let deliverable = queue_staged(&core, &node, &phone_id, &mine).await;

        core.drainer(&node).drain_peer(&phone_id).await;

        assert_eq!(
            received(&signals),
            vec!["report.html".to_string()],
            "the one deliverable record has to go out in a SINGLE pass, whatever is ahead of it"
        );
        let status = core.server_status().await.unwrap();
        assert!(
            !status.queued.iter().any(|q| q.id == deliverable),
            "a delivered record leaves the spool, or the next pass sends it twice"
        );
        assert_eq!(
            status.queued_total,
            outbox::MAX_PER_PASS + 1,
            "and the held records stay held: not sent, and not dropped either"
        );
        assert!(
            status.queued.iter().all(|q| q
                .last_error
                .as_deref()
                .is_some_and(|why| why.contains("outside this server's send roots"))),
            "every one of them says why it is not moving: {:?}",
            status.queued
        );

        phone.router.shutdown().await.ok();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn one_pass_pushes_at_most_the_per_pass_bound_and_the_next_pass_takes_the_rest() {
        // What `MAX_PER_PASS` buys: one device with a full spool must not own
        // the drain until its last record is gone. The drain is one
        // serialized task, so every push a peer makes on one pass is time
        // every other paired device waits, and a spool holds up to
        // `MAX_RECORDS` of them.
        //
        // The starvation test above cannot show this and still passes with
        // the bound deleted: it queues exactly ONE deliverable record, so an
        // unbounded loop simply runs off the end of a queue whose other
        // records are all skipped. Here every record is deliverable, so a
        // pass that does not stop at the bound pushes the whole spool.
        let state = tempfile::TempDir::new().unwrap();
        let phone_dir = tempfile::TempDir::new().unwrap();
        let workspace = tempfile::TempDir::new().unwrap();

        let (core, node, phone, signals, phone_id) =
            core_draining_to(&state, &workspace, &phone_dir).await;

        // Two past the bound. All inside this session's roots, all with their
        // bytes in the store and all for one awake device that grants
        // control, so no record here can be held, dropped or refused: the
        // budget is the only thing left that can stop one.
        let mut names = Vec::new();
        let mut ids = Vec::new();
        for n in 0..outbox::MAX_PER_PASS + 2 {
            let name = format!("report-{n}.html");
            let file = workspace.path().join(&name);
            std::fs::write(&file, format!("<h1>deliverable {n}</h1>")).unwrap();
            ids.push(queue_staged(&core, &node, &phone_id, &file).await);
            names.push(name);
        }

        core.drainer(&node).drain_peer(&phone_id).await;

        assert_eq!(
            received(&signals),
            names[..outbox::MAX_PER_PASS],
            "one pass hands over the bound and stops, oldest record first"
        );
        let after_one = core.server_status().await.unwrap();
        assert_eq!(
            after_one.queued.iter().map(|q| q.id.as_str()).collect::<Vec<_>>(),
            ids[outbox::MAX_PER_PASS..],
            "the records past the bound are still pending, in the order they were queued"
        );
        assert!(
            after_one.queued.iter().all(|q| q.attempts == 0 && q.last_error.is_none()),
            "and the pass never reached them — this is the budget stopping, not a push \
             that was tried and failed: {:?}",
            after_one.queued
        );

        // A bound defers work, it never drops it: the next pass takes the
        // batch behind it, which is what makes a full spool drain at all.
        core.drainer(&node).drain_peer(&phone_id).await;

        assert_eq!(received(&signals), names, "the next pass takes the records behind the bound");
        assert_eq!(
            core.server_status().await.unwrap().queued_total,
            0,
            "and nothing is owed to this device any more"
        );

        phone.router.shutdown().await.ok();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn two_drain_passes_for_one_peer_deliver_each_file_exactly_once() {
        // The in-flight-plus-redrive guard on `drain_peer`, held to what it
        // promises. Both passes read the same record list before either has
        // completed a record, so without the guard both push every file on
        // it: the receiver keeps the second copy under the Beam collision
        // rule, and the user gets a `report-2.html` nobody sent. A device
        // that dials in while the 60 s tick is already draining it puts two
        // triggers in the same moment, which is the case this covers.
        let state = tempfile::TempDir::new().unwrap();
        let phone_dir = tempfile::TempDir::new().unwrap();
        let workspace = tempfile::TempDir::new().unwrap();

        let (core, node, phone, signals, phone_id) =
            core_draining_to(&state, &workspace, &phone_dir).await;

        let mut queued: Vec<String> = Vec::new();
        for name in ["report.html", "notes.html"] {
            let file = workspace.path().join(name);
            std::fs::write(&file, format!("<h1>{name}</h1>")).unwrap();
            queue_staged(&core, &node, &phone_id, &file).await;
            queued.push(name.to_string());
        }

        // One drainer, two passes at once — the same handles the supervisor
        // and a wake would use, so the guard under test is the real one.
        let drainer = core.drainer(&node);
        tokio::join!(drainer.drain_peer(&phone_id), drainer.drain_peer(&phone_id));

        let mut landed = received(&signals);
        landed.sort();
        queued.sort();
        assert_eq!(landed, queued, "each queued file lands once, under the name it was sent with");
        assert_eq!(
            core.server_status().await.unwrap().queued_total,
            0,
            "and the spool is empty, so nothing is left for a later pass to send again"
        );

        phone.router.shutdown().await.ok();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_reachable_peer_whose_records_are_all_held_still_steps_the_retry_ladder() {
        // The ladder used to be stepped only by a failed dial, a scope that
        // is too narrow or a dead session. A peer that ANSWERS, grants
        // control and then has nothing this session may send fell through all
        // three, so `due` said yes forever and the pass ran again at every
        // DRAIN_TICK — 1440 dials a day at a device that had already given
        // its answer, each one an n0 discovery lookup that tells a third
        // party who this machine talks to.
        let state = tempfile::TempDir::new().unwrap();
        let phone_dir = tempfile::TempDir::new().unwrap();
        let workspace = tempfile::TempDir::new().unwrap();
        let other_project = tempfile::TempDir::new().unwrap();
        let held = other_project.path().join("report.html");
        std::fs::write(&held, "<h1>queued in another project</h1>").unwrap();

        let (core, node, phone, signals, phone_id) =
            core_draining_to(&state, &workspace, &phone_dir).await;
        queue_staged(&core, &node, &phone_id, &held).await;
        assert!(core.drainer(&node).due(&phone_id), "nothing has failed for it yet");

        core.drainer(&node).drain_peer(&phone_id).await;

        assert!(received(&signals).is_empty(), "the ROOTS held it, so nothing was pushed");
        let why = core.server_status().await.unwrap().queued[0].last_error.clone().unwrap();
        assert!(
            why.contains("outside this server's send roots"),
            "the pass reached the roots test, so the dial and the grant both passed: {why}"
        );
        assert!(
            !core.drainer(&node).due(&phone_id),
            "a pass that moved nothing is a stalled peer, whatever stalled it"
        );

        phone.router.shutdown().await.ok();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn two_sends_of_one_file_to_one_sleeping_device_make_one_record() {
        let state = tempfile::TempDir::new().unwrap();
        let phone = tempfile::TempDir::new().unwrap();
        let workspace = tempfile::TempDir::new().unwrap();
        let artifact = workspace.path().join("report.html");
        let body = "<!doctype html><h1>report</h1>";
        std::fs::write(&artifact, body).unwrap();

        let asleep = deaf_device(&phone).await;
        let core = core_paired_with(&state, &workspace, &asleep).await;

        let first = core.send_to_device(artifact.to_str().unwrap(), "iPhone").await.unwrap();
        let Delivery::Queued { id, device, name, size, hash, expires_at, reason, notes, .. } = first
        else {
            panic!("a device that did not answer must be queued, never delivered: {first:?}");
        };
        assert_eq!(device, "Val's iPhone");
        assert_eq!(name, "report.html");
        assert_eq!(size, body.len() as u64);
        assert!(reason.contains("peer offline"), "the dial's own answer, verbatim: {reason}");
        assert!(expires_at > peers::now_unix());
        // No handshake has ever completed with this device, so the answer has
        // to say the control grant is unverified rather than imply a promise
        // the device may refuse when it wakes.
        assert!(notes.iter().any(|n| n.contains("unverified")), "{notes:?}");
        assert!(notes.iter().any(|n| n.contains("copied as it stands")), "{notes:?}");

        let record = Dirs::new(state.path()).outbox().join(format!("{id}.json"));
        assert!(record.is_file(), "the send is on disk before it is reported: {record:?}");

        // The retry a model makes when a call looked like it failed. The
        // store is content-addressed, so this stages the same bytes — and
        // without the dedupe one user intent becomes two records, each
        // pinning its own copy.
        let again = core.send_to_device(artifact.to_str().unwrap(), "iPhone").await.unwrap();
        let Delivery::Queued { id: repeated, .. } = again else {
            panic!("the second send must be queued too");
        };
        assert_eq!(repeated, id, "one intent, one record, however often it is asked for");

        let status = core.server_status().await.unwrap();
        assert_eq!(status.queued_total, 1);
        assert_eq!(status.queued_bytes, body.len() as u64);
        assert_eq!(status.retained_bytes, body.len() as u64, "the copy is real and is counted");
        // The drain IS running — `send_to_device` booted it — and it owns no
        // peer at this instant: each queued send stepped this device's retry
        // ladder, so the pass its wake started found nothing due.
        assert!(!status.draining, "no pass owns this peer, so the status must not claim one does");
        assert_eq!(status.queue_blocked_reason, None);
        assert!(status.queue_unreadable.is_empty());
        let queued = &status.queued[0];
        assert_eq!(queued.id, id);
        assert_eq!(queued.hash, hash);
        assert_eq!(queued.node_id, asleep.endpoint.id().to_string());
        assert_eq!(
            queued.source,
            artifact.canonicalize().unwrap(),
            "the record keeps the path the gate resolved, never the caller's argument"
        );
        assert_eq!(queued.attempts, 0, "nothing has tried to deliver it yet");

        asleep.router.shutdown().await.ok();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_replayed_record_id_leaves_the_incumbents_file_and_its_pin_alone() {
        // `queue_send` mints `{millis}-{seq}` one line before it stages the
        // bytes under `outbox/<id>`, so two runs that mint the same id put
        // the second run's staging on the first run's pin. That happens when
        // the clock goes backwards, and on any machine reporting a time
        // before 1970 it happens to every first send, because `now_millis`
        // answers 0 and the counter used to restart at 0 as well.
        // `Outbox::enqueue` refuses the repeated id — `create_new` is what
        // makes it fail loudly rather than overwrite — but the cleanup under
        // that refusal then unpins the tag, and the tag was the incumbent's.
        // The incumbent kept its FILE and lost its BYTES.
        //
        // The replay is handed to the staging call rather than provoked
        // through `next_id`: nothing in this process can move the machine's
        // clock, and this is the exact call `queue_send` makes with the id it
        // has just minted.
        let state = tempfile::TempDir::new().unwrap();
        let phone = tempfile::TempDir::new().unwrap();
        let workspace = tempfile::TempDir::new().unwrap();
        let artifact = workspace.path().join("report.html");
        let body = "<!doctype html><h1>accepted first, and still owed</h1>";
        std::fs::write(&artifact, body).unwrap();

        let asleep = deaf_device(&phone).await;
        let core = core_paired_with(&state, &workspace, &asleep).await;
        let node = core.node().await.unwrap();

        let accepted = core.send_to_device(artifact.to_str().unwrap(), "iPhone").await.unwrap();
        let Delivery::Queued { id, hash, .. } = accepted else {
            panic!("a device that did not answer must be queued: {accepted:?}");
        };
        let record = Dirs::new(state.path()).outbox().join(format!("{id}.json"));
        assert!(record.is_file(), "the incumbent is on disk before anything replays its id");

        // The second run, minting the id the first one already used.
        let intruder = workspace.path().join("other.html");
        std::fs::write(&intruder, "<h1>a second run minting the very same id</h1>").unwrap();
        let err = beam::stage_outbox(&node, &intruder.canonicalize().unwrap(), &id)
            .await
            .expect_err("a taken id has to fail before it can take a pin");
        assert!(err.contains(&outbox::tag_name(&id)), "the refusal names the pin, got: {err}");

        // BOTH halves of the incumbent survive, which is the whole claim.
        assert!(record.is_file(), "the record file the id claim protects");
        let pinned = node
            .store
            .tags()
            .get(outbox::tag_name(&id))
            .await
            .unwrap()
            .expect("the pin that keeps the bytes");
        assert_eq!(pinned.hash.to_string(), hash, "and it still names the bytes it was made for");
        assert!(beam::outbox_bytes_present(&node, &hash).await, "which are still in the store");

        let status = core.server_status().await.unwrap();
        assert_eq!(status.queued_total, 1, "one send was accepted, and it is still pending");
        assert_eq!(status.queued[0].hash, hash);
        assert_eq!(
            status.retained_bytes,
            body.len() as u64,
            "and the refused replay cost the user no second copy"
        );

        asleep.router.shutdown().await.ok();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_record_measures_the_copy_it_took_and_not_the_file_it_read() {
        // The gate reads the file's length with `std::fs::metadata`, and the
        // copy is taken later — after a whole dial to a device that does not
        // answer, which costs up to DIAL_TIMEOUT. A user who saves the file in
        // that window used to get a record whose size named the file that was
        // READ and whose hash named the bytes that were COPIED.
        //
        // Two things then go wrong with one record. The status reports a size
        // for bytes that are not those bytes; and the receiver checks the
        // ANNOUNCED size against the transfer cap before it opens the stream
        // and the real cap on the stream itself, so a record that under-states
        // a file which grew past that cap is refused mid-transfer — and the
        // drain HOLDS a refusal, so it retries once a pass for the whole
        // seven-day TTL and can never land.
        //
        // The two steps are driven directly here rather than raced against a
        // real dial: `send_to_device` runs exactly this pair, in this order,
        // and the window between them is the dial it has already spent.
        let state = tempfile::TempDir::new().unwrap();
        let phone = tempfile::TempDir::new().unwrap();
        let workspace = tempfile::TempDir::new().unwrap();
        let artifact = workspace.path().join("report.html");
        let as_read = "<h1>the file as the gate measured it</h1>";
        std::fs::write(&artifact, as_read).unwrap();

        let asleep = deaf_device(&phone).await;
        let core = core_paired_with(&state, &workspace, &asleep).await;
        let node = core.node().await.unwrap();
        let peer = core.peer_store().get(&asleep.endpoint.id().to_string()).unwrap();

        let cand = core.gate_arg_path(artifact.to_str().unwrap()).expect("inside the roots");
        assert_eq!(cand.size, as_read.len() as u64, "the candidate carries the file it read");

        // The save the user makes while the send is still waiting on the dial.
        let as_copied = "<h1>the file as it stood when the copy was actually taken, longer</h1>";
        assert_ne!(as_read.len(), as_copied.len(), "the proof needs the two lengths to differ");
        std::fs::write(&artifact, as_copied).unwrap();

        let accepted = core
            .queue_send(&node, &peer, &cand, "peer offline".to_string())
            .await
            .expect("a device that did not answer is queued");
        let Delivery::Queued { size, hash, .. } = accepted else {
            panic!("a device that did not answer must be queued, never delivered: {accepted:?}");
        };
        assert_eq!(size, as_copied.len() as u64, "the record announces the copy it took");

        // And the size and the hash describe the SAME bytes, which is the
        // whole invariant. The store is content-addressed, so staging those
        // bytes a second time is what names them independently.
        let twin = workspace.path().join("same-bytes.html");
        std::fs::write(&twin, as_copied).unwrap();
        let proof = beam::stage_outbox(&node, &twin.canonicalize().unwrap(), &core.outbox.next_id())
            .await
            .expect("staged bytes");
        assert_eq!(hash, proof.hash, "the address the record carries is the copy's own");
        assert_eq!(proof.size, size, "and the length beside it is that copy's length");

        // The surface a human reads says the same, because it reads the record.
        let status = core.server_status().await.unwrap();
        assert_eq!(status.queued_bytes, as_copied.len() as u64);
        assert_eq!(status.queued[0].size, as_copied.len() as u64);

        asleep.router.shutdown().await.ok();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_copy_that_grew_past_the_cap_is_refused_at_once_instead_of_queued_to_be_denied() {
        // The gate measures the file with `std::fs::metadata` and the copy is
        // taken later, after a whole dial to a device that does not answer.
        // A file that grows past the hard cap in that window used to become a
        // record nothing can ever deliver: the receiver refuses the announced
        // size against the same cap before it opens the stream, answers
        // `PushFailure::Denied`, and the drain holds a denial rather than
        // dropping it. The record then retries once
        // a pass for the whole seven-day TTL and holds one of that peer's
        // eight per-pass push slots, so the records behind it starve too.
        //
        // The cap is lowered for this proof — see `cap_staged_at`. The real
        // one is 256 MiB, and a file that size on disk is not what this claim
        // is about; the claim is that the staged length is what the cap is
        // asked of. The two steps are driven directly rather than raced
        // against a real dial: `send_to_device` runs exactly this pair, in
        // this order, and the window between them is the dial it has spent.
        let state = tempfile::TempDir::new().unwrap();
        let phone = tempfile::TempDir::new().unwrap();
        let workspace = tempfile::TempDir::new().unwrap();
        let artifact = workspace.path().join("report.html");
        let as_read = "<h1>small enough when the gate read it</h1>";
        std::fs::write(&artifact, as_read).unwrap();

        let asleep = deaf_device(&phone).await;
        let core = core_paired_with(&state, &workspace, &asleep).await;
        let node = core.node().await.unwrap();
        let peer = core.peer_store().get(&asleep.endpoint.id().to_string()).unwrap();
        assert_eq!(
            core.staged_cap(),
            beam::HARD_CAP_BYTES,
            "production measures the staged copy against the real cap"
        );
        core.cap_staged_at(4096);

        let cand = core.gate_arg_path(artifact.to_str().unwrap()).expect("inside the roots");
        assert_eq!(cand.size, as_read.len() as u64, "the candidate carries the file it read");

        // The save the user makes while the send is still waiting on the dial.
        // Eight KiB against a four KiB cap, so the two numbers in the refusal
        // are distinguishable once `human_bytes` has rounded them.
        let as_copied = vec![b'x'; 8192];
        std::fs::write(&artifact, &as_copied).unwrap();

        let err = core
            .queue_send(&node, &peer, &cand, "peer offline".to_string())
            .await
            .expect_err("a copy over the cap can never be delivered, so it is not accepted");
        assert!(err.contains("report.html"), "the refusal names the file: {err}");
        assert!(err.contains("is 8 KiB"), "and the size it actually measured: {err}");
        assert!(err.contains("caps at 4 KiB"), "and the cap it measured against: {err}");
        assert!(err.contains("was not queued"), "{err}");

        // Nothing was promised, so nothing is owed.
        assert_eq!(
            core.server_status().await.unwrap().queued_total,
            0,
            "a refused send leaves no record"
        );

        // And nothing was kept, which is the half a size check on its own
        // cannot cover: the copy is taken before the cap is asked, so a
        // refusal that skipped the unpin would leave a private duplicate of
        // the user's file pinned in this server's store for the life of the
        // install. The empty keep-set is the true keep-set here, because the
        // spool holds no record at all.
        assert_eq!(
            beam::sweep_outbox_tags(&node, &[]).await,
            0,
            "the refusal released the pin its own staging minted"
        );

        // The other side of the boundary: a copy of exactly the cap still
        // goes, because the receiver refuses only what is over the cap, and
        // a sender that refused one byte earlier would lose a deliverable
        // send. Same file, same bytes, one number moved.
        core.cap_staged_at(as_copied.len() as u64);
        let accepted = core
            .queue_send(&node, &peer, &cand, "peer offline".to_string())
            .await
            .expect("a copy that is exactly the cap is deliverable");
        let Delivery::Queued { size, .. } = accepted else {
            panic!("a device that did not answer must be queued, never delivered: {accepted:?}");
        };
        assert_eq!(size, as_copied.len() as u64, "the record announces the copy it took");
        assert_eq!(core.server_status().await.unwrap().queued_total, 1);

        asleep.router.shutdown().await.ok();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_sleeping_device_that_never_granted_control_is_refused_instead_of_hoarded_for() {
        // The privacy half of the queue. Every queued send is a full private
        // copy of the user's file kept for a week; making them for a device
        // that will refuse the bytes on arrival is the one way this feature
        // could quietly cost more than it is worth.
        let state = tempfile::TempDir::new().unwrap();
        let phone = tempfile::TempDir::new().unwrap();
        let workspace = tempfile::TempDir::new().unwrap();
        let artifact = workspace.path().join("report.html");
        std::fs::write(&artifact, "<h1>report</h1>").unwrap();

        let asleep = deaf_device(&phone).await;
        let core = core_paired_with(&state, &workspace, &asleep).await;
        let phone_id = asleep.endpoint.id().to_string();

        core.peer_store().note_ack_scope(&phone_id, "browse").unwrap();
        let err = core.send_to_device(artifact.to_str().unwrap(), "iPhone").await.unwrap_err();
        assert!(err.contains("has not granted this server control"), "{err}");
        assert!(err.contains("\"browse\""), "the message names the scope it knows about: {err}");
        assert!(err.contains("Nothing was queued"), "{err}");
        assert_eq!(core.server_status().await.unwrap().queued_total, 0);
        assert!(
            !Dirs::new(state.path()).outbox().exists(),
            "a refusal must not even create the spool, let alone stage a copy"
        );

        // The same device, once it has granted control: now the send waits,
        // and its answer no longer calls the grant unverified.
        core.peer_store().note_ack_scope(&phone_id, "control").unwrap();
        let queued = core.send_to_device(artifact.to_str().unwrap(), "iPhone").await.unwrap();
        let Delivery::Queued { notes, .. } = queued else {
            panic!("the device is still not answering");
        };
        assert!(!notes.iter().any(|n| n.contains("unverified")), "{notes:?}");
        assert_eq!(core.server_status().await.unwrap().queued_total, 1);

        asleep.router.shutdown().await.ok();
    }

    #[tokio::test]
    async fn two_devices_owed_one_file_are_owed_one_copy_of_it() {
        // What the status must not do is add a disk cost that is not there.
        // The store is content-addressed and a record pins a hash, so one
        // file queued for two devices is two records, two pins and one blob.
        // The spool is written straight to disk here: `McpCore::new` reads it
        // back, and neither number needs a socket, a device or a store.
        let state = tempfile::TempDir::new().unwrap();
        let workspace = tempfile::TempDir::new().unwrap();
        let spool = Outbox::load(&Dirs::new(state.path()).outbox());
        let hash = "b".repeat(64);
        for device in ["Val's iPhone", "Val's iPad"] {
            spool
                .enqueue(Staged {
                    id: spool.next_id(),
                    peer: "a".repeat(64),
                    device: device.to_string(),
                    source: workspace.path().join("report.html"),
                    name: "report.html".to_string(),
                    size: 4096,
                    hash: hash.clone(),
                })
                .unwrap();
        }

        let core = McpCore::new(
            state.path().to_path_buf(),
            vec![workspace.path().to_path_buf()],
            workspace.path().to_path_buf(),
            None,
        );
        let status = core.server_status().await.unwrap();
        assert_eq!(status.queued_total, 2, "both devices are still owed the file");
        assert_eq!(status.queued_bytes, 8192, "and that is what is owed, added up");
        assert_eq!(status.retained_bytes, 4096, "but one copy is what it costs this disk");
    }

    #[test]
    fn the_retry_ladder_never_dials_a_sleeping_device_more_than_once_a_minute() {
        // The floor is the privacy half of the cadence, not a performance
        // choice: `addr_for` names a peer by NodeId alone, so every retry is
        // an n0 discovery lookup and a possible relay traversal, and each one
        // is a third-party observation of who this machine talks to. A week
        // of a phone in a drawer is what this ladder is sized against.
        assert_eq!(retry_delay(1), Duration::from_secs(60));
        assert_eq!(retry_delay(2), Duration::from_secs(120));
        assert_eq!(retry_delay(3), Duration::from_secs(300));
        assert_eq!(retry_delay(4), Duration::from_secs(600));
        assert_eq!(retry_delay(u32::MAX), Duration::from_secs(600), "it never grows past ten");
        // Nothing calls this with zero — `back_off` increments first — but a
        // ladder that answered "retry immediately" to an off-by-one would
        // spin a dial as fast as the timeout allows.
        assert_eq!(retry_delay(0), Duration::from_secs(60));
        assert!(RETRY_LADDER.iter().all(|step| *step >= 60), "the floor holds at every rung");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_queued_send_does_not_re_dial_the_device_it_just_failed_to_reach() {
        // `send_to_device` has already spent a whole dial on this device
        // before it queued anything. A drain that then dialed again, because
        // a new record woke it, would learn the same thing twice and start
        // the week's retries at zero.
        let state = tempfile::TempDir::new().unwrap();
        let phone = tempfile::TempDir::new().unwrap();
        let workspace = tempfile::TempDir::new().unwrap();
        let artifact = workspace.path().join("report.html");
        std::fs::write(&artifact, "<h1>report</h1>").unwrap();

        let asleep = deaf_device(&phone).await;
        let core = core_paired_with(&state, &workspace, &asleep).await;
        let phone_id = asleep.endpoint.id().to_string();
        let node = core.node().await.unwrap();
        assert!(core.drainer(&node).due(&phone_id), "nothing has failed for it yet");

        core.send_to_device(artifact.to_str().unwrap(), "iPhone").await.unwrap();
        assert!(
            !core.drainer(&node).due(&phone_id),
            "the send's own failed dial is the first rung of the ladder"
        );

        asleep.router.shutdown().await.ok();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_peer_list_that_did_not_load_holds_the_queue_instead_of_destroying_it() {
        // `PeerStore::load` answers a parse failure, a newer `PEERS_SCHEMA`
        // and any I/O error that is not NotFound the same way: an EMPTY list
        // plus a `load_error`. `get` never consults that error, so after one
        // EACCES on peers.json every device on this machine looks unpaired —
        // and the drain read that as a revocation, removed every record and
        // unpinned every staged copy, which the collector then took within
        // the minute. `server_status` reported an empty queue with no reason
        // beside it.
        //
        // The rest of this stack already holds instead: `live_tags` answers
        // `None` on a spool that did not load so no sweep runs, and
        // `save_list` refuses to write over a peer file it could not read.
        let state = tempfile::TempDir::new().unwrap();
        let workspace = tempfile::TempDir::new().unwrap();
        let dirs = Dirs::new(state.path());
        std::fs::create_dir_all(dirs.remote()).unwrap();
        std::fs::write(dirs.remote().join("peers.json"), "{not json").unwrap();

        let core = McpCore::new(
            state.path().to_path_buf(),
            vec![workspace.path().to_path_buf()],
            workspace.path().to_path_buf(),
            None,
        );
        let node = core.node().await.expect("an unreadable peer list still boots an endpoint");
        let artifact = workspace.path().join("report.html");
        let body = "<h1>owed to a device this build cannot look up</h1>";
        std::fs::write(&artifact, body).unwrap();
        // The peer is named by the record alone, which is the situation: the
        // file that would confirm the pairing is the one that will not read.
        let phone_id = "ab".repeat(32);
        let id = queue_staged(&core, &node, &phone_id, &artifact).await;

        core.drainer(&node).drain_peer(&phone_id).await;

        // All three halves of the promise survive the pass.
        let record = dirs.outbox().join(format!("{id}.json"));
        assert!(record.is_file(), "the record file is still there: {record:?}");
        assert!(
            node.store.tags().get(outbox::tag_name(&id)).await.unwrap().is_some(),
            "and so is the pin, which is the only thing keeping the copy off the collector"
        );
        let status = core.server_status().await.unwrap();
        assert_eq!(status.queued_total, 1, "the delivery is hidden, not gone");
        assert_eq!(status.retained_bytes, body.len() as u64);
        assert!(beam::outbox_bytes_present(&node, &status.queued[0].hash).await);

        // And the human is told, on the record and on the server, that the
        // peer list is what stopped it — a queue that only stopped moving is
        // the silent failure this whole surface exists to remove.
        let blocked = status.queue_blocked_reason.as_deref().unwrap_or_default();
        assert!(blocked.contains("peer list"), "the status names the cause, got: {blocked:?}");
        let held = status.queued[0].last_error.as_deref().unwrap_or_default();
        assert!(held.contains("peer list"), "and so does the record, got: {held:?}");
    }

    /// Unix only: the proof is a directory mode, and it is the shape the
    /// failure takes in the wild — a state directory on a read-only mount, or
    /// one whose owner changed after a restore from backup.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_spool_that_cannot_be_written_says_so_instead_of_reporting_an_untried_record() {
        use std::os::unix::fs::PermissionsExt;

        // `record_attempt` saves the file before it updates the record in
        // memory, so a spool this process cannot write keeps handing back a
        // record that has never been attempted — after every pass has
        // attempted it. Only stderr knew, and the status surface showed a
        // count beside "not tried yet", which is what a queue that is about
        // to move looks like.
        let state = tempfile::TempDir::new().unwrap();
        let workspace = tempfile::TempDir::new().unwrap();
        let phone_dir = tempfile::TempDir::new().unwrap();
        let phone = deaf_device(&phone_dir).await;
        let core = core_paired_with(&state, &workspace, &phone).await;
        let node = core.node().await.unwrap();
        let phone_id = phone.endpoint.id().to_string();

        let artifact = workspace.path().join("report.html");
        std::fs::write(&artifact, "<h1>owed to a device that does not answer</h1>").unwrap();
        queue_staged(&core, &node, &phone_id, &artifact).await;

        // Read-only after the record is on disk: the pending list still reads,
        // and no attempt can be written beside it. Restored before the
        // assertions, so a failing one cannot leave a directory the temp dir
        // is unable to clean up.
        let outbox_dir = Dirs::new(state.path()).outbox();
        let writable = std::fs::metadata(&outbox_dir).unwrap().permissions();
        std::fs::set_permissions(&outbox_dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        core.drainer(&node).drain_peer(&phone_id).await;
        let stuck = core.server_status().await.unwrap();
        std::fs::set_permissions(&outbox_dir, writable).unwrap();

        assert_eq!(stuck.queued_total, 1, "the delivery is still owed");
        assert_eq!(stuck.queued[0].attempts, 0, "the pass had nowhere to write what it found");
        assert_eq!(stuck.queued[0].last_error, None, "so the record itself still reads untried");
        // Which is why the reason has to be on the server: the record cannot
        // carry it, and a status that stayed silent would report a queue
        // failing every pass as one waiting for a phone to wake up.
        let blocked = stuck.queue_blocked_reason.as_deref().unwrap_or_default();
        assert!(
            blocked.contains("send queue could not be written"),
            "the status names the write failure, got: {blocked:?}"
        );
        assert!(
            blocked.contains("Permission denied"),
            "and quotes what the filesystem said, got: {blocked:?}"
        );

        // The other half, and the reason this is not a latch: the fault ends
        // at the next write that lands. A reason kept after the volume came
        // back would refuse every later send over a failure that is over.
        core.drainer(&node).drain_peer(&phone_id).await;
        let healed = core.server_status().await.unwrap();
        assert_eq!(healed.queue_blocked_reason, None, "a spool that takes a write is not blocked");
        assert_eq!(healed.queued[0].attempts, 1, "and the attempt is on disk this time");
        assert!(
            healed.queued[0].last_error.is_some(),
            "with the reason the pass found: {:?}",
            healed.queued[0]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unpairing_a_device_releases_the_copies_it_was_owed_instead_of_keeping_them_to_the_ttl()
    {
        // The other side of the rule above, and the reason it has to be a
        // hold rather than a skip: a peer list that DID read and no longer
        // names the device is a revocation, and a revocation takes effect
        // now. Every record for that device is a private full copy of a user
        // file inside this server's state directory, and keeping one for a
        // week after the human unpaired the device it was for is the outcome
        // this queue must never produce.
        let state = tempfile::TempDir::new().unwrap();
        let workspace = tempfile::TempDir::new().unwrap();
        let core = McpCore::new(
            state.path().to_path_buf(),
            vec![workspace.path().to_path_buf()],
            workspace.path().to_path_buf(),
            None,
        );
        // No device is stood up: a pass for a peer the store no longer names
        // never dials, which is exactly what makes the revocation immediate.
        let phone_id = "cd".repeat(32);
        core.peer_store().seed(&phone_id, "Val's iPhone", Scope::Control).unwrap();
        let node = core.node().await.unwrap();

        let mut ids = Vec::new();
        let mut owed = 0u64;
        for n in 0..2 {
            let artifact = workspace.path().join(format!("report-{n}.html"));
            let body = format!("<h1>accepted while the device was still paired: {n}</h1>");
            std::fs::write(&artifact, &body).unwrap();
            owed += body.len() as u64;
            ids.push(queue_staged(&core, &node, &phone_id, &artifact).await);
        }
        let before = core.server_status().await.unwrap();
        assert_eq!(before.queued_total, 2);
        assert_eq!(before.retained_bytes, owed, "two copies, on this disk, right now");

        assert!(core.peer_store().remove(&phone_id).unwrap(), "the human unpairs the device");
        core.drainer(&node).drain_peer(&phone_id).await;

        let status = core.server_status().await.unwrap();
        assert_eq!(status.queued_total, 0, "nothing is owed to a device that is not paired");
        assert_eq!(status.retained_bytes, 0, "and nothing is kept on this disk for it");
        for id in &ids {
            assert!(
                !Dirs::new(state.path()).outbox().join(format!("{id}.json")).exists(),
                "the record file goes, or the next boot brings the delivery back"
            );
            assert!(
                node.store.tags().get(outbox::tag_name(id)).await.unwrap().is_none(),
                "and its pin goes with it, or the copy stays on disk for the life of the install"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_record_whose_bytes_are_gone_is_dropped_at_boot_instead_of_retried_forever() {
        // The record and the staged copy it names can come apart: a state
        // directory restored from a backup, a store rebuilt, a build that did
        // not write this pin. Replaying such a record announces a fetch the
        // receiver can never complete, and would do it once per pass for the
        // whole week the record lives.
        let state = tempfile::TempDir::new().unwrap();
        let workspace = tempfile::TempDir::new().unwrap();
        let phone_id = "ab".repeat(32);

        // Written before the core reads the directory, the way a previous
        // process would have left it.
        let spool = Outbox::load(&Dirs::new(state.path()).outbox());
        let id = spool.next_id();
        spool
            .enqueue(Staged {
                id: id.clone(),
                peer: phone_id.clone(),
                device: "Val's iPhone".to_string(),
                source: workspace.path().join("report.html"),
                name: "report.html".to_string(),
                size: 15,
                // No blob in this store has this address.
                hash: "b".repeat(64),
            })
            .unwrap();

        let core = McpCore::new(
            state.path().to_path_buf(),
            vec![workspace.path().to_path_buf()],
            workspace.path().to_path_buf(),
            None,
        );
        // Still paired, so a missing copy is the only thing that can explain
        // the record going away.
        core.peer_store().seed(&phone_id, "Val's iPhone", Scope::Control).unwrap();
        assert_eq!(core.server_status().await.unwrap().queued_total, 1, "it is on disk");

        // Reconciliation runs INSIDE the boot initializer, so a node in hand
        // means it has already finished.
        core.node().await.expect("this server's own store");
        let status = core.server_status().await.unwrap();
        assert_eq!(status.queued_total, 0, "a delivery that cannot happen is not kept");
        assert_eq!(status.queued_bytes, 0);
        assert!(
            !Dirs::new(state.path()).outbox().join(format!("{id}.json")).exists(),
            "the record file goes with it, or the next boot brings it back"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_quarantined_record_is_still_named_after_the_boot_reconciles() {
        // The boot RELOADS the spool before it builds the sweep keep-set, so
        // a quarantine counted only on the read that moved the file aside was
        // erased by the very next read. Two things went with it: this list,
        // which is the only place a human hears that a delivery is not
        // happening, and the stem's place in the keep-set, which is the only
        // thing holding the staged copy of the user's file back from the boot
        // sweep.
        let state = tempfile::TempDir::new().unwrap();
        let workspace = tempfile::TempDir::new().unwrap();
        let outbox_dir = Dirs::new(state.path()).outbox();
        std::fs::create_dir_all(&outbox_dir).unwrap();
        std::fs::write(outbox_dir.join("0000000000001-0099.json"), "{not json").unwrap();

        let core = McpCore::new(
            state.path().to_path_buf(),
            vec![workspace.path().to_path_buf()],
            workspace.path().to_path_buf(),
            None,
        );
        assert_eq!(
            core.server_status().await.unwrap().queue_unreadable,
            vec!["0000000000001-0099".to_string()],
            "the read that moved it aside names it"
        );

        // Reconciliation runs INSIDE the boot initializer, so a node in hand
        // means the reload and the sweep have both already happened.
        core.node().await.expect("this server's own store");
        assert_eq!(
            core.server_status().await.unwrap().queue_unreadable,
            vec!["0000000000001-0099".to_string()],
            "and so does every read after it"
        );
    }

    #[test]
    fn the_pairing_instructions_name_the_grant_direction() {
        let text = pairing_instructions("Claude Code @ studio").join(" ");
        assert!(text.contains("control"), "{text}");
        assert!(text.contains("Claude Code @ studio"), "{text}");
    }
}
