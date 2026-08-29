// iroh Endpoint + Router lifecycle and identity persistence. All iroh types
// stay behind this module (and beam.rs) — the version pin in Cargo.toml is
// deliberate, and upgrades are migrations, not `cargo update` accidents.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use iroh::endpoint::presets;
use iroh::protocol::Router;
use iroh::{Endpoint, SecretKey};
use iroh_blobs::store::fs::FsStore;
use iroh_blobs::BlobsProtocol;

use crate::beam::{OfferInfo, Offers};
use crate::paths::Dirs;
use crate::{proto, scope};

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
    /// Where this node keeps its files. Carried on the node so the fetch
    /// paths derive `received/` and `cache/` from the consumer's own base
    /// instead of a hardcoded application-support directory.
    pub dirs: Dirs,
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
            write_secret_file(&key_path, &key.to_bytes())?;
            Ok(key)
        }
        Err(e) => Err(format!("cannot read {key_path:?}: {e}")),
    }
}

#[cfg(unix)]
fn write_secret_file(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("cannot create {path:?}: {e}"))?;
    f.write_all(bytes).map_err(|e| e.to_string())
}

#[cfg(not(unix))]
fn write_secret_file(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes).map_err(|e| e.to_string())
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
        dirs: dirs.clone(),
    })
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
    let id: iroh::EndpointId = peer.parse().map_err(|_| "malformed peer id".to_string())?;
    Ok(iroh::EndpointAddr::from_parts(
        id,
        [iroh::TransportAddr::Ip(socket)],
    ))
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
}
