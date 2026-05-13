// Root-anchored security boundary. `canonicalize_and_check_root` is the
// load-bearing gate every filesystem read in the IPC layer flows through.

use std::path::{Path, PathBuf};

/// Error returned by `canonicalize_and_check_root`. Reader code pattern-matches
/// on these variants — keep the shape stable.
#[derive(Debug, Clone, thiserror::Error, serde::Serialize, serde::Deserialize)]
pub enum OutOfRootError {
    /// The path was found and canonicalized but lies outside every root.
    #[error("path is out of root: {0:?}")]
    OutOfRoot(PathBuf),
    /// `path.canonicalize()` failed (NotFound, PermissionDenied, etc.).
    #[error("canonicalize failed at {path:?}: {reason}")]
    CanonicalizeFailed { path: PathBuf, reason: String },
    /// The configured root set is empty — no path can pass the gate.
    #[error("empty root set")]
    EmptyRoots,
}

/// The set of canonical absolute paths every read must be anchored within.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct RootSet {
    roots: Vec<PathBuf>,
}

impl RootSet {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        let canonical_roots = roots
            .into_iter()
            .filter_map(|p| p.canonicalize().ok())
            .collect();
        Self { roots: canonical_roots }
    }

    pub fn empty() -> Self {
        Self { roots: Vec::new() }
    }

    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    pub fn contains(&self, path: &Path) -> bool {
        self.roots.iter().any(|r| path.starts_with(r))
    }
}

/// Resolve `path` to its canonical form and verify it lives under one of
/// the configured roots.
pub fn canonicalize_and_check_root(
    path: &Path,
    roots: &RootSet,
) -> Result<PathBuf, OutOfRootError> {
    if roots.roots().is_empty() {
        return Err(OutOfRootError::EmptyRoots);
    }

    let canonical = path.canonicalize().map_err(|e| OutOfRootError::CanonicalizeFailed {
        path: path.to_path_buf(),
        reason: e.to_string(),
    })?;

    if !roots.contains(&canonical) {
        return Err(OutOfRootError::OutOfRoot(canonical));
    }
    Ok(canonical)
}
