// Where the remote subsystem keeps its files, the one walk policy the tree
// listing shares with the local quick-open walker, and the small file
// primitives every module here reads and writes through.
//
// The crate hardcodes NO absolute directory. The desktop app passes
// `~/Library/Application Support/Vlerv`; a headless consumer (the MCP server)
// passes its own base. Everything below is derived from that one base, so a
// second consumer cannot land bytes in the app's tree by accident.

use std::fs::Metadata;
use std::path::{Path, PathBuf};

/// Default-ignored directory names, filtered out of every listing — the local
/// scanner's list and the remote `ListTree` policy are the same list on
/// purpose (design §6: "same ignore/hidden/symlink policy as ⌘P's walker").
pub const DEFAULT_IGNORED: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".venv",
];

/// The base-directory seam. One state directory in, five derived paths out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dirs {
    base: PathBuf,
}

impl Dirs {
    /// `base` is the consumer's state directory — the app's
    /// `~/Library/Application Support/Vlerv`, a test's tempdir, or whatever a
    /// headless host owns.
    pub fn new(base: impl Into<PathBuf>) -> Self {
        Self { base: base.into() }
    }

    pub fn base(&self) -> &Path {
        &self.base
    }

    /// `<base>/remote/` — identity key, peer store, blob store.
    pub fn remote(&self) -> PathBuf {
        self.base.join("remote")
    }

    /// `<base>/remote/blobs/` — the content-addressed store both Beam and
    /// Scope stage into.
    pub fn blobs(&self) -> PathBuf {
        self.remote().join("blobs")
    }

    /// `<base>/remote/blobs.lock` — the file whose exclusive lock says who
    /// owns `blobs()`. A sibling, not a child: it stays outside the tree
    /// `FsStore` creates and manages, so resetting or deleting `blobs/`
    /// cannot take the claim with it.
    pub fn blobs_lock(&self) -> PathBuf {
        self.remote().join("blobs.lock")
    }

    /// `<base>/remote/cache/` — artifacts fetched from a scoped peer, named by
    /// content address.
    pub fn cache(&self) -> PathBuf {
        self.remote().join("cache")
    }

    /// `<base>/received/` — landed beams and pushes. Inside the consumer's own
    /// state dir: the read-only principle holds (nothing is ever written into
    /// the user's tree).
    pub fn received(&self) -> PathBuf {
        self.base.join("received")
    }

    /// `<base>/remote/outbox/` — one file per accepted-but-undelivered send.
    /// A record names a peer, a local path and a pinned hash, so it is a
    /// capability document and lives in the 0600 class with identity.key.
    ///
    /// Under `remote/`, beside the blob store it pins bytes in: the process
    /// holding `blobs_lock()` is the only one that may open that store, so it
    /// is also the only one that may write here.
    pub fn outbox(&self) -> PathBuf {
        self.remote().join("outbox")
    }
}

/// The last component of a path, as owned display text. `None` for a root or
/// a path ending in `..`, so each caller keeps its own fallback: the wire
/// wants an empty name, an offer wants "artifact", a landed push wants the
/// name the sender announced.
pub fn base_name(path: &Path) -> Option<String> {
    path.file_name().map(|n| n.to_string_lossy().into_owned())
}

/// A file's modification time in unix seconds. `0` when the platform cannot
/// report one — the wire (`ArtifactMeta::mtime`) and the Received list both
/// document 0 as "unknown", so the fallback lives here rather than at each
/// call site.
pub fn mtime_secs(meta: &Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Write `bytes` to `path` with 0600 permissions, truncating an existing file.
/// The one private-file writer in the crate: the identity key and the peer
/// store are both "this file IS a capability" documents, and a second writer
/// is how one of them silently loses its mode.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    write_private_inner(path, bytes, false)
}

/// `write_private`, refusing to touch a file that already exists
/// (`create_new`). What `identity.key` needs: clobbering it would silently
/// change this instance's NodeId.
pub fn create_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    write_private_inner(path, bytes, true)
}

/// `write_private`, made crash-safe: stage the bytes beside `path` under the
/// same name plus `.tmp`, then rename over the target. The one atomic writer
/// for this crate's 0600 documents — peers.json and every outbox record —
/// because a half-written capability document is a peer list missing the
/// machine that was just paired, or a queued send naming no peer at all.
///
/// THE STAGING NAME IS LOAD-BEARING. `outbox::Spool::read` skips what a crash
/// leaves behind by testing the LAST extension of every name in the
/// directory, so a `<id>.json.tmp` leftover is passed over while a name
/// ending in `.json` would be read back as a record.
///
/// The parent directory is created here rather than at each call site, which
/// is where both writers did it before. It is not redundant for the spool:
/// the outbox directory is made by `outbox::Outbox::enqueue` and by nothing
/// else, so an attempt recorded after the state directory was cleared out
/// from under a running server restores the file instead of failing.
pub fn write_private_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("cannot create {parent:?}: {e}"))?;
    }
    // Appended, not `with_extension`: that would REPLACE `.json`, and both
    // documents are read back by a rule that keys off the extension.
    let mut staging = path.as_os_str().to_os_string();
    staging.push(".tmp");
    let tmp = PathBuf::from(staging);
    write_private(&tmp, bytes)?;
    std::fs::rename(&tmp, path).map_err(|e| format!("cannot write {path:?}: {e}"))
}

#[cfg(unix)]
fn write_private_inner(path: &Path, bytes: &[u8], exclusive: bool) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut options = std::fs::OpenOptions::new();
    options.write(true).mode(0o600);
    if exclusive {
        options.create_new(true);
    } else {
        options.create(true).truncate(true);
    }
    let mut f = options
        .open(path)
        .map_err(|e| format!("cannot create {path:?}: {e}"))?;
    // `mode` applies at CREATION only. The non-exclusive path is the `.tmp`
    // staging file `write_private_atomic` writes, which a crash between write
    // and rename can leave behind: reopening it would keep whatever mode it
    // already had. Set the mode on the open handle so the renamed file is 0600
    // either way — it names the machines this install trusts, or a file one of
    // them may fetch.
    f.set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("cannot restrict {path:?}: {e}"))?;
    f.write_all(bytes).map_err(|e| e.to_string())
}

#[cfg(not(unix))]
fn write_private_inner(path: &Path, bytes: &[u8], exclusive: bool) -> Result<(), String> {
    use std::io::Write;
    if !exclusive {
        return std::fs::write(path, bytes).map_err(|e| e.to_string());
    }
    let mut f = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|e| format!("cannot create {path:?}: {e}"))?;
    f.write_all(bytes).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_path_derives_from_the_one_base() {
        let dirs = Dirs::new("/tmp/state");
        assert_eq!(dirs.remote(), Path::new("/tmp/state/remote"));
        assert_eq!(dirs.blobs(), Path::new("/tmp/state/remote/blobs"));
        assert_eq!(dirs.blobs_lock(), Path::new("/tmp/state/remote/blobs.lock"));
        assert_eq!(dirs.cache(), Path::new("/tmp/state/remote/cache"));
        assert_eq!(dirs.outbox(), Path::new("/tmp/state/remote/outbox"));
        assert_eq!(dirs.received(), Path::new("/tmp/state/received"));
        assert_eq!(dirs.base(), Path::new("/tmp/state"));
    }

    #[test]
    fn two_consumers_never_share_a_directory() {
        let app = Dirs::new("/a");
        let headless = Dirs::new("/b");
        assert_ne!(app.received(), headless.received());
        assert_ne!(app.remote(), headless.remote());
    }

    #[test]
    fn base_name_leaves_the_fallback_to_the_caller() {
        assert_eq!(base_name(Path::new("/w/report.html")).as_deref(), Some("report.html"));
        assert_eq!(base_name(Path::new("/")), None);
        assert_eq!(base_name(Path::new("/w/..")), None);
    }

    #[test]
    fn mtime_of_a_real_file_is_a_plausible_unix_second() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("a.html");
        std::fs::write(&file, "x").unwrap();
        let meta = std::fs::metadata(&file).unwrap();
        // 2020-01-01: any real clock is past it, and 0 would mean "unknown".
        assert!(mtime_secs(&meta) > 1_577_836_800);
    }

    #[test]
    fn the_private_writer_truncates_or_refuses_by_entry_point() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("secret");

        create_private(&path, b"first").unwrap();
        // The exclusive entry point is what stops a second boot from
        // clobbering an identity key it just failed to read.
        assert!(create_private(&path, b"second").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"first");

        // The staging entry point overwrites, and re-restricts a file a crash
        // left behind with a wider mode.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        write_private(&path, b"third").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"third");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
