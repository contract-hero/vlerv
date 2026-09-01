// iroh Endpoint + Router lifecycle and identity persistence. All iroh types
// stay behind this module (and beam.rs) — the version pin in Cargo.toml is
// deliberate, and upgrades are migrations, not `cargo update` accidents.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use iroh::endpoint::{presets, Connection};
use iroh::protocol::Router;
use iroh::{Endpoint, EndpointId, SecretKey};
use iroh_blobs::store::fs::FsStore;
use iroh_blobs::BlobsProtocol;

use crate::beam::{OfferInfo, Offers};
use crate::paths::{self, Dirs};
use crate::{proto, scope};

/// How long any outgoing dial may take before it is reported as the other
/// machine being offline. One constant for every ALPN: a beam fetch, a scope
/// session, a pairing handshake and a cache download all wait the same.
pub const DIAL_TIMEOUT: Duration = Duration::from_secs(30);

/// Backing state for one booted remote node: identity, endpoint, protocol
/// router, blob store, and the two registries the request gate enforces —
/// Beam's ticket offers and Scope's peer-locked grants.
pub struct RemoteNode {
    pub endpoint: Endpoint,
    pub router: Router,
    pub store: FsStore,
    pub offers: Arc<Offers>,
    pub grants: Arc<scope::Grants>,
    /// The scope server, when this boot was given host state. `None` in the
    /// Beam-only transfer test, which boots endpoints with no peer store.
    pub scope: Option<Arc<scope::ScopeServer>>,
    /// Held for the life of the node: the exclusive claim on the blob store
    /// directory. Dropping it (or exiting) lets the next process boot.
    _store_lock: StoreLock,
}

/// An exclusive advisory lock on one blob-store directory, held by an open
/// file descriptor for as long as the value lives.
///
/// `FsStore` is a redb database, so exactly one process may own it. Without
/// this claim a second process does not fail — it HANGS: `FsStore::load`
/// blocks, and unwinding it deadlocks inside `RtWrapper::drop`, which drops a
/// tokio `BlockingPool` from inside `block_in_place`. A `timeout` cannot
/// rescue that, because the stuck work is a synchronous drop inside a poll,
/// not an await that can be cancelled. So the contention has to be caught
/// BEFORE the store is opened, which is what this type does.
///
/// Every Claude Code session spawns its own `vlerv-mcp` against one state
/// directory, so this is the ordinary case, not a corner case.
#[derive(Debug)]
pub struct StoreLock {
    _file: std::fs::File,
}

impl StoreLock {
    /// Claim `<dir>/blobs.lock`, or report which directory is already taken.
    /// Non-blocking on purpose: a caller that waited would be back to hanging.
    fn acquire(dir: &std::path::Path) -> Result<Self, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {dir:?}: {e}"))?;
        let path = dir.join("blobs.lock");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|e| format!("cannot open {path:?}: {e}"))?;

        // SAFETY: `flock` needs only a valid fd, which `file` owns for the
        // whole call and after it. Closing that fd releases the lock, so the
        // fd lives in the returned value.
        let rc = unsafe {
            libc::flock(
                std::os::unix::io::AsRawFd::as_raw_fd(&file),
                libc::LOCK_EX | libc::LOCK_NB,
            )
        };
        if rc != 0 {
            let err = std::io::Error::last_os_error();
            return match err.kind() {
                std::io::ErrorKind::WouldBlock => Err(format!(
                    "another Vlerv process is already using the blob store at {dir:?}. \
                     Only one can serve beams at a time — close the other one, \
                     or point this one at its own VLERV_MCP_STATE_DIR."
                )),
                _ => Err(format!("cannot lock {path:?}: {err}")),
            };
        }
        Ok(Self { _file: file })
    }
}

/// Load the persisted ed25519 secret key, generating one on first use.
/// The file is written 0600: the secret IS the instance's identity.
pub fn load_or_create_identity(dir: &std::path::Path) -> Result<SecretKey, String> {
    let key_path = dir.join("identity.key");
    match std::fs::read(&key_path) {
        Ok(bytes) => {
            let bytes: [u8; 32] = bytes
                .try_into()
                .map_err(|_| format!("identity.key is corrupt (not 32 bytes): {key_path:?}"))?;
            Ok(SecretKey::from_bytes(&bytes))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            let key = SecretKey::generate();
            // `create_private`, not `write_private`: two boots racing on a
            // fresh install must not each write a key and leave one of them
            // holding a NodeId the file no longer names.
            paths::create_private(&key_path, &key.to_bytes())?;
            Ok(key)
        }
        Err(e) => Err(format!("cannot read {key_path:?}: {e}")),
    }
}

/// Boot the node: identity, endpoint (n0 preset: hole-punching + encrypted
/// relay fallback + address discovery), blob store, and the gated blobs
/// protocol behind an iroh Router. Called lazily — never at app launch.
///
/// `dirs` is the consumer's state base: the app passes its application-support
/// directory, a headless host passes its own, and the two-endpoint tests pass
/// tempdirs to run a sender and a receiver side by side in one process.
/// `scope_state` is the v2 host side: with it, the router also answers the
/// scope and pairing ALPNs. Without it the node is Beam-only.
/// `on_offers_change` receives the fresh offers list whenever the request
/// gate mutates the registry (fetch counts).
pub async fn boot(
    dirs: &Dirs,
    scope_state: Option<Arc<scope::ScopeState>>,
    on_offers_change: impl Fn(Vec<OfferInfo>) + Send + Sync + 'static,
) -> Result<RemoteNode, String> {
    let remote_dir = dirs.remote();
    let secret = load_or_create_identity(&remote_dir)?;

    // Before anything binds or opens: claim the blob store. A second process
    // that skipped this claim would hang forever inside `FsStore::load`
    // (see `StoreLock`), so the claim comes first and fails loudly.
    let store_lock = StoreLock::acquire(&remote_dir)?;

    let endpoint = Endpoint::builder(presets::N0)
        .secret_key(secret)
        .bind()
        .await
        .map_err(|e| format!("cannot bind iroh endpoint: {e}"))?;

    let store = FsStore::load(dirs.blobs())
        .await
        .map_err(|e| format!("cannot open blob store: {e}"))?;

    let offers = Arc::new(Offers::new());
    let grants = Arc::new(scope::Grants::new());
    let events = offers.clone().gate(grants.clone(), on_offers_change);
    let blobs = BlobsProtocol::new(&store, Some(events));

    // One endpoint, three protocols, multiplexed by ALPN (design §4): blobs
    // carries content, `vlerv/scope/0` carries the session, `vlerv/pair/0` is
    // the one door an unpaired NodeId may knock on.
    let mut builder = Router::builder(endpoint.clone()).accept(iroh_blobs::ALPN, blobs);
    let scope_server = scope_state.map(|state| {
        let server = Arc::new(scope::ScopeServer::new(
            state.clone(),
            store.clone(),
            grants.clone(),
            endpoint.clone(),
            dirs.clone(),
        ));
        (server, state)
    });
    if let Some((server, state)) = &scope_server {
        builder = builder
            .accept(proto::SCOPE_ALPN, server.clone())
            .accept(proto::PAIR_ALPN, scope::PairServer::new(state.clone(), endpoint.id()));
    }
    let router = builder.spawn();

    Ok(RemoteNode {
        endpoint,
        router,
        store,
        offers,
        grants,
        scope: scope_server.map(|(server, _)| server),
        _store_lock: store_lock,
    })
}

/// Dial one peer on one ALPN, bounded by `DIAL_TIMEOUT`. Every outgoing
/// connection in the crate opens here, so the timeout and the "who is
/// unreachable" wording cannot drift between the four call sites.
///
/// `unreachable` is the subject of the failure sentence — "peer offline —
/// could not reach it", "sender offline — could not reach the sender" — and
/// the cause is appended in parentheses. A timeout and a refusal read the
/// same on purpose: both mean the other machine did not answer.
pub(crate) async fn dial(
    endpoint: &Endpoint,
    addr: iroh::EndpointAddr,
    alpn: &[u8],
    unreachable: &str,
) -> Result<Connection, String> {
    tokio::time::timeout(DIAL_TIMEOUT, endpoint.connect(addr, alpn))
        .await
        .map_err(|_| format!("{unreachable} (timed out)"))?
        .map_err(|e| format!("{unreachable} ({e})"))
}

/// Turn a peer's NodeId string into a dialable address. The one place a
/// consumer needs to name a peer, so iroh's own types stay behind this crate:
/// the app holds hex strings and never an `EndpointId`.
pub fn addr_for(peer: &str) -> Result<iroh::EndpointAddr, String> {
    let id: iroh::EndpointId = peer.parse().map_err(|_| "malformed peer id".to_string())?;
    Ok(iroh::EndpointAddr::from(id))
}

/// `addr_for`, pinned to one transport address. `addr_for` names a peer and
/// leaves reaching it to discovery and the relays; this one says exactly
/// where to knock. The argument is a plain `SocketAddr`, so a consumer still
/// never names an iroh type: an in-process two-endpoint test dials loopback,
/// and a caller that already knows the socket skips discovery entirely.
pub fn addr_at(peer: &str, socket: SocketAddr) -> Result<iroh::EndpointAddr, String> {
    let id: EndpointId = peer.parse().map_err(|_| "malformed peer id".to_string())?;
    Ok(addr_at_id(id, socket))
}

/// `addr_at` for a caller that already holds the parsed id — its own
/// endpoint's, typically, which cannot be malformed and so has no error to
/// report. The one place `iroh::TransportAddr` is named outside a dial.
pub fn addr_at_id(id: EndpointId, socket: SocketAddr) -> iroh::EndpointAddr {
    iroh::EndpointAddr::from_parts(id, [iroh::TransportAddr::Ip(socket)])
}

/// This node's own `127.0.0.1:<bound port>`. The endpoint binds `0.0.0.0`, so
/// loopback always reaches it from the same machine — which is what the
/// two-endpoint tests dial instead of depending on relays or discovery.
///
/// Direct addresses appear a moment after bind on some machines, so a first
/// empty answer waits (bounded) on `online()` before giving up.
pub async fn loopback_socket(node: &RemoteNode) -> Option<SocketAddr> {
    let port = match ipv4_port(node) {
        Some(port) => port,
        None => {
            let _ = tokio::time::timeout(Duration::from_secs(10), node.endpoint.online()).await;
            ipv4_port(node)?
        }
    };
    Some(SocketAddr::from((Ipv4Addr::LOCALHOST, port)))
}

fn ipv4_port(node: &RemoteNode) -> Option<u16> {
    node.endpoint.addr().ip_addrs().find(|a| a.is_ipv4()).map(|a| a.port())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addr_for_round_trips_a_node_id_and_refuses_garbage() {
        let id = SecretKey::from_bytes(&[5u8; 32]).public();
        assert_eq!(addr_for(&id.to_string()).unwrap().id, id);
        assert_eq!(addr_for("nonsense").unwrap_err(), "malformed peer id");
        assert!(addr_for("").is_err());
    }

    #[test]
    fn addr_at_pins_the_peer_to_one_socket() {
        let id = SecretKey::from_bytes(&[7u8; 32]).public();
        let socket = SocketAddr::from((Ipv4Addr::LOCALHOST, 4321));
        let addr = addr_at(&id.to_string(), socket).unwrap();
        assert_eq!(addr.id, id);
        assert_eq!(addr.ip_addrs().copied().collect::<Vec<_>>(), vec![socket]);
        // Same refusal as `addr_for`: the id is parsed the same way.
        assert_eq!(addr_at("nonsense", socket).unwrap_err(), "malformed peer id");
    }

    #[test]
    fn identity_round_trips_and_is_0600() {
        let dir = tempfile::TempDir::new().unwrap();
        let key1 = load_or_create_identity(dir.path()).unwrap();
        let key2 = load_or_create_identity(dir.path()).unwrap();
        assert_eq!(key1.to_bytes(), key2.to_bytes(), "second load must reuse the key");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("identity.key"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn corrupt_identity_is_a_hard_error_not_a_silent_regen() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("identity.key"), b"short").unwrap();
        // Regenerating would silently change the instance's NodeId — every
        // previously shared ticket and (v2) pairing would dangle.
        assert!(load_or_create_identity(dir.path()).is_err());
    }

    #[test]
    fn second_claim_on_one_store_is_refused_not_queued() {
        let dir = tempfile::TempDir::new().unwrap();
        let first = StoreLock::acquire(dir.path()).expect("first claim");

        // Refused, and refused IMMEDIATELY. A claim that waited would restore
        // the hang this lock exists to prevent, so the test bounds the call.
        let started = std::time::Instant::now();
        let second = StoreLock::acquire(dir.path());
        assert!(second.is_err(), "a second process must not get the store");
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "the refusal must not block"
        );

        let msg = second.unwrap_err();
        assert!(
            msg.contains("already using the blob store"),
            "the error has to name the cause, got: {msg}"
        );
        assert!(
            msg.contains("VLERV_MCP_STATE_DIR"),
            "the error has to name the way out, got: {msg}"
        );

        // Releasing hands the store to the next process — this is what makes
        // "close the other one" an actual fix.
        drop(first);
        assert!(
            StoreLock::acquire(dir.path()).is_ok(),
            "a released store must re-open"
        );
    }

    #[test]
    fn each_state_dir_gets_its_own_claim() {
        // Two servers pointed at separate state dirs must not collide — that
        // is the escape hatch the refusal message names.
        let a = tempfile::TempDir::new().unwrap();
        let b = tempfile::TempDir::new().unwrap();
        let _held_a = StoreLock::acquire(a.path()).expect("first dir");
        assert!(
            StoreLock::acquire(b.path()).is_ok(),
            "a separate dir is independent"
        );
    }
}
