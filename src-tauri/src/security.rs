// Root-anchored security boundary. `canonicalize_and_check_root` is the
// load-bearing gate every filesystem read in the IPC layer flows through.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

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

/// The set of canonical absolute paths deep links are classified against.
/// Interior-shared (`Arc<RwLock>`): every `Clone` sees later `add_root`
/// calls, so the boot-time clone captured by the deep-link callback stays in
/// sync with the workspace the user actually picks in the UI.
#[derive(Debug, Clone, Default)]
pub struct RootSet {
    roots: Arc<RwLock<Vec<PathBuf>>>,
}

impl RootSet {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        let canonical_roots = roots
            .into_iter()
            .filter_map(|p| p.canonicalize().ok())
            .collect();
        Self {
            roots: Arc::new(RwLock::new(canonical_roots)),
        }
    }

    pub fn empty() -> Self {
        Self::default()
    }

    /// Canonicalize and append a root, deduped. Non-resolvable paths are
    /// silently ignored (same policy as `new`).
    pub fn add_root(&self, root: &Path) {
        if let Ok(canonical) = root.canonicalize() {
            let mut roots = self.roots.write().unwrap_or_else(|p| p.into_inner());
            if !roots.contains(&canonical) {
                roots.push(canonical);
            }
        }
    }

    /// Snapshot of the current roots.
    pub fn roots(&self) -> Vec<PathBuf> {
        self.roots
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub fn is_empty(&self) -> bool {
        self.roots
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .is_empty()
    }

    pub fn contains(&self, path: &Path) -> bool {
        self.roots
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .any(|r| path.starts_with(r))
    }
}

/// Resolve `path` to its canonical form and verify it lives under one of
/// the configured roots.
pub fn canonicalize_and_check_root(
    path: &Path,
    roots: &RootSet,
) -> Result<PathBuf, OutOfRootError> {
    if roots.is_empty() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn add_root_visible_through_clone() {
        let dir = TempDir::new().unwrap();
        let set = RootSet::empty();
        let clone = set.clone(); // e.g. the deep-link callback's copy

        assert!(clone.is_empty());
        set.add_root(dir.path());
        assert!(!clone.is_empty());
        assert!(clone.contains(&dir.path().canonicalize().unwrap()));
    }

    #[test]
    fn add_root_dedupes() {
        let dir = TempDir::new().unwrap();
        let set = RootSet::empty();
        set.add_root(dir.path());
        set.add_root(dir.path());
        assert_eq!(set.roots().len(), 1);
    }
}
