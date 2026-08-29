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

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::future::join_all;
use serde::Serialize;
use vlerv_remote::host::{EmptyCatalog, EventSink, HostSignal};
use vlerv_remote::peers::{self, short_id, Pairing, Peer, PeerStore, PendingPair, Scope};
use vlerv_remote::scope::{ClientSession, ScopeState, TabsCache};
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

/// Shortest content-hash prefix `stop_beam` accepts for ONE link. Revoking
/// the wrong link is recoverable (mint another); revoking by a one-character
/// prefix by accident is just noise. Omitting the argument revokes them all,
/// which is the deliberate, unambiguous form.
const MIN_HASH_CHARS: usize = 8;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Delivery {
    pub device: String,
    pub node_id: String,
    /// The name the RECEIVING device landed the file under — collision
    /// handling there may have renamed it.
    pub name: String,
    /// The size the receiver measured, never the one this side announced.
    pub size: u64,
    pub hash: String,
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
    pub uptime_secs: u64,
    pub paired_devices: usize,
    pub active_offers: Vec<OfferSummary>,
    /// Files other devices pushed to this server during this process.
    pub received_artifacts: Vec<ReceivedArtifact>,
    /// The directories a sent file is resolved against.
    pub roots: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReceivedArtifact {
    pub from: String,
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    pub hash: String,
}

/// This server's `EventSink`. A headless host has no window to raise, so the
/// three signals become: park the pairing for `pair_status`, record the
/// artifact for `server_status`, and a stderr line for a human tailing the
/// log. Stdout is the JSON-RPC channel and is never written to here.
struct McpSink {
    pairing: Arc<Pairing>,
    received: Arc<Mutex<Vec<ReceivedArtifact>>>,
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
                self.received
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .push(ReceivedArtifact { from: peer, name, path, size, hash });
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
    received: Arc<Mutex<Vec<ReceivedArtifact>>>,
    node: tokio::sync::Mutex<Option<Arc<endpoint::RemoteNode>>>,
    sessions: tokio::sync::Mutex<HashMap<String, Arc<ClientSession>>>,
    started: Instant,
    /// Test seam: when set, peers are dialed at this socket and the push
    /// ticket names this server's own loopback address, so a two-endpoint
    /// test never depends on relays or discovery. `None` in production.
    loopback: Mutex<Option<SocketAddr>>,
}

impl McpCore {
    /// Build a core over `state_dir`, with `roots` as the send policy's root
    /// set. Reads `peers.json` (a missing file is a fresh install) and binds
    /// nothing.
    pub fn new(state_dir: PathBuf, roots: Vec<PathBuf>, cwd: PathBuf, home: Option<PathBuf>) -> Self {
        let dirs = Dirs::new(state_dir);
        Self {
            device: device_name(),
            peers: Arc::new(PeerStore::load(&dirs.remote())),
            pairing: Arc::new(Pairing::new()),
            received: Arc::new(Mutex::new(Vec::new())),
            roots: RootSet::new(roots),
            cwd,
            home,
            dirs,
            node: tokio::sync::Mutex::new(None),
            sessions: tokio::sync::Mutex::new(HashMap::new()),
            started: Instant::now(),
            loopback: Mutex::new(None),
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
    #[doc(hidden)]
    pub fn use_loopback(&self, host: SocketAddr) {
        *self.loopback.lock().unwrap_or_else(|p| p.into_inner()) = Some(host);
    }

    fn loopback(&self) -> Option<SocketAddr> {
        *self.loopback.lock().unwrap_or_else(|p| p.into_inner())
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
        let mut guard = self.node.lock().await;
        if let Some(node) = guard.as_ref() {
            return Ok(node.clone());
        }
        let state = Arc::new(ScopeState::new(
            self.peers.clone(),
            self.pairing.clone(),
            Arc::new(TabsCache::new()),
            self.roots.clone(),
            self.device.clone(),
            // Headless: no bookmarks, no recents, no open tabs. A view-open
            // peer is therefore told about nothing and may fetch nothing.
            Arc::new(EmptyCatalog),
            McpSink { pairing: self.pairing.clone(), received: self.received.clone() },
        ));
        let node = Arc::new(endpoint::boot(&self.dirs, Some(state), |_| {}).await?);
        *guard = Some(node.clone());
        Ok(node)
    }

    async fn booted(&self) -> Option<Arc<endpoint::RemoteNode>> {
        self.node.lock().await.clone()
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
        let Some(node) = self.booted().await else {
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
    pub async fn list_devices(&self, probe: bool) -> Vec<DeviceInfo> {
        let peers = self.peers.list();
        // Pay the lazy endpoint boot ONCE, outside the per-device probe budget.
        // Otherwise the first probe on a cold server spends its whole timeout
        // on bind + relay + store load and reports a reachable device offline.
        if probe {
            let _ = self.node().await;
        }
        // The probes run TOGETHER. Dialed one after another, a fleet where
        // three devices are asleep costs three PROBE_TIMEOUTs before the list
        // comes back; concurrently the whole call bounds at about one, however
        // many devices are paired.
        let presence =
            join_all(peers.iter().map(|peer| self.presence(peer, probe))).await;
        peers
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
            .collect()
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

    /// Resolve `device` to one paired peer and push `raw_path` to it.
    pub async fn send_to_device(&self, raw_path: &str, device: &str) -> Result<Delivery, String> {
        let query = args::validate_device_query(device)?;
        // Same order as `beam_artifact`: one gate pass, and an unsendable file
        // is refused before any socket opens.
        let cand = self.gate_arg_path(raw_path)?;
        let peer = devices::resolve_device(&self.peers.list(), query).map_err(|e| e.to_string())?;

        // The scope in the handshake is what the DEVICE granted this server.
        // Checking it here turns the host's one deliberately vague refusal
        // ("not permitted for this peer") into an instruction the human can
        // act on.
        let mut session = self.session(&peer).await?;
        if session.scope != Scope::Control.as_str() {
            // A session reports the grant as it stood when it connected, so a
            // cached one can be stale: the human may have widened the scope
            // between two tool calls. Re-handshake once before refusing.
            self.forget_session(&peer.node_id).await;
            session = self.session(&peer).await?;
        }
        if session.scope != Scope::Control.as_str() {
            return Err(format!(
                "{} has not granted this server control. It paired this server at scope {:?}; \
                 pushing a file needs \"control\". On that device, open its Vlervtifacts peer \
                 settings, find \"{}\", and set its scope to \"control\".",
                label(&peer),
                session.scope,
                self.device
            ));
        }

        // The canonical path the gate resolved, not the caller's string: the
        // push re-applies the same policy, and it must see the same file.
        let pushed = match self.own_loopback().await {
            Some(own) => session.push_artifact_at(&cand.canonical, &self.roots, own).await?,
            None => session.push_artifact(&cand.canonical, &self.roots).await?,
        };
        Ok(Delivery {
            device: peer.device,
            node_id: peer.node_id,
            name: pushed.name,
            size: pushed.size,
            hash: pushed.hash,
        })
    }

    /// Drop a cached session so the next call re-handshakes.
    async fn forget_session(&self, node_id: &str) {
        self.sessions.lock().await.remove(node_id);
    }

    /// This server's own `127.0.0.1:<port>`, and only in loopback mode — the
    /// address a pushed ticket names so the peer dials back over loopback.
    async fn own_loopback(&self) -> Option<SocketAddr> {
        self.loopback()?;
        let node = self.node().await.ok()?;
        endpoint::loopback_socket(&node).await
    }

    /// A live session with a paired peer, dialed on first use and reused
    /// after. Dial failures are reported as the device being offline, which
    /// is what they almost always are.
    async fn session(&self, peer: &Peer) -> Result<Arc<ClientSession>, String> {
        if let Some(existing) = self.sessions.lock().await.get(&peer.node_id) {
            if !existing.is_closed() {
                return Ok(existing.clone());
            }
        }
        let node = self.node().await?;
        let addr = match self.loopback() {
            Some(host) => endpoint::addr_at(&peer.node_id, host)?,
            None => endpoint::addr_for(&peer.node_id)?,
        };
        let session =
            ClientSession::connect(&node, addr, self.device.clone(), |_| {}, || {})
                .await
                .map_err(|e| {
                    format!(
                        "{} is not reachable: {e}. Check that the device is awake, on a network, \
                         and that Vlervtifacts is running on it.",
                        label(peer)
                    )
                })?;
        self.sessions.lock().await.insert(peer.node_id.clone(), session.clone());
        Ok(session)
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
    pub fn confirm_pairing(
        &self,
        accept: bool,
        node_id: Option<&str>,
        scope: Option<&str>,
    ) -> Result<PairingOutcome, String> {
        // The scope is checked FIRST, so a typo is refused before a parked
        // pairing is consumed.
        let granted = args::validate_scope(scope)?;
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
        let peer = self.peers.upsert(&pending.node_id, &pending.device, granted)?;
        Ok(PairingOutcome {
            paired: true,
            device: peer.device,
            node_id: peer.node_id,
            scope: Some(peer.scope.as_str().to_string()),
        })
    }

    // ── server_status ──────────────────────────────────────────────────────

    pub async fn server_status(&self) -> Result<ServerStatus, String> {
        let node = self.booted().await;
        let active_offers = node
            .as_ref()
            .map(|n| n.offers.list().into_iter().map(OfferSummary::from).collect())
            .unwrap_or_default();
        let node_id = match node.as_ref() {
            Some(n) => n.endpoint.id().to_string(),
            None => self.node_id()?,
        };
        Ok(ServerStatus {
            node_id_short: short_id(&node_id),
            node_id,
            device: self.device.clone(),
            identity_dir: self.dirs.remote(),
            state_dir: self.dirs.base().to_path_buf(),
            booted: node.is_some(),
            uptime_secs: self.started.elapsed().as_secs(),
            paired_devices: self.peers.list().len(),
            active_offers,
            received_artifacts: self
                .received
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone(),
            roots: self.roots.roots(),
        })
    }
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

    #[test]
    fn the_pairing_instructions_name_the_grant_direction() {
        let text = pairing_instructions("Claude Code @ studio").join(" ");
        assert!(text.contains("control"), "{text}");
        assert!(text.contains("Claude Code @ studio"), "{text}");
    }
}
