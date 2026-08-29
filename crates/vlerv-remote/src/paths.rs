// Where the remote subsystem keeps its files, and the one walk policy the
// tree listing shares with the local quick-open walker.
//
// The crate hardcodes NO absolute directory. The desktop app passes
// `~/Library/Application Support/Vlerv`; a headless consumer (the MCP server)
// passes its own base. Everything below is derived from that one base, so a
// second consumer cannot land bytes in the app's tree by accident.

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

/// The base-directory seam. One state directory in, four derived paths out.
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_path_derives_from_the_one_base() {
        let dirs = Dirs::new("/tmp/state");
        assert_eq!(dirs.remote(), Path::new("/tmp/state/remote"));
        assert_eq!(dirs.blobs(), Path::new("/tmp/state/remote/blobs"));
        assert_eq!(dirs.cache(), Path::new("/tmp/state/remote/cache"));
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
}
