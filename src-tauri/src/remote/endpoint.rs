// iroh Endpoint + Router lifecycle and identity persistence. All iroh types
// stay behind this module (and beam.rs) — the version pin in Cargo.toml is
// deliberate, and upgrades are migrations, not `cargo update` accidents.

use std::path::PathBuf;
use std::sync::Arc;

use iroh::endpoint::presets;
use iroh::protocol::Router;
use iroh::{Endpoint, SecretKey};
use iroh_blobs::store::fs::FsStore;
use iroh_blobs::BlobsProtocol;

use super::beam::{OfferInfo, Offers};
use crate::state_store;

/// Backing state for one booted remote node: identity, endpoint, protocol
/// router, blob store, and the offers registry the request gate enforces.
pub struct RemoteNode {
    pub endpoint: Endpoint,
    pub router: Router,
    pub store: FsStore,
    pub offers: Arc<Offers>,
}

/// `~/Library/Application Support/Vlerv/remote/` — identity + blob store.
pub fn remote_dir() -> PathBuf {
    state_store::state_dir().join("remote")
}

/// `~/Library/Application Support/Vlerv/received/` — landed beams. Inside
/// the app's own state dir: the read-only principle holds (the app never
/// writes into the user's tree).
pub fn received_dir() -> PathBuf {
    state_store::state_dir().join("received")
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
/// `on_offers_change` receives the fresh offers list whenever the request
/// gate mutates the registry (fetch counts).
pub async fn boot(
    on_offers_change: impl Fn(Vec<OfferInfo>) + Send + Sync + 'static,
) -> Result<RemoteNode, String> {
    boot_in(&remote_dir(), on_offers_change).await
}

/// `boot` with an explicit state directory — the two-endpoint transfer test
/// runs a sender and a receiver node side by side in one process.
pub async fn boot_in(
    dir: &std::path::Path,
    on_offers_change: impl Fn(Vec<OfferInfo>) + Send + Sync + 'static,
) -> Result<RemoteNode, String> {
    let secret = load_or_create_identity(dir)?;

    let endpoint = Endpoint::builder(presets::N0)
        .secret_key(secret)
        .bind()
        .await
        .map_err(|e| format!("cannot bind iroh endpoint: {e}"))?;

    let store = FsStore::load(dir.join("blobs"))
        .await
        .map_err(|e| format!("cannot open blob store: {e}"))?;

    let offers = Arc::new(Offers::new());
    let events = offers.clone().gate(on_offers_change);
    let blobs = BlobsProtocol::new(&store, Some(events));
    let router = Router::builder(endpoint.clone())
        .accept(iroh_blobs::ALPN, blobs)
        .spawn();

    Ok(RemoteNode { endpoint, router, store, offers })
}

#[cfg(test)]
mod tests {
    use super::*;

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
