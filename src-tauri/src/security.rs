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

/// The "ad-hoc external file" policy: resolve `path`, allowing paths that
/// exist but lie outside every root (Preview legitimately displays those —
/// user-picked files and out-of-root deep links). Returns the canonical path
/// plus `out_of_root`. Unresolvable paths and an empty root set stay hard
/// errors. Shared by the deep-link dispatcher and the share command so the
/// external-file policy lives in exactly one place.
///
/// Note the deliberate asymmetry with `canonicalize_allow_rootless` below:
/// outbound actions (share) stay conservative when no workspace has been
/// picked; merely displaying a file does not.
pub fn canonicalize_allow_external(
    path: &Path,
    roots: &RootSet,
) -> Result<(PathBuf, bool), OutOfRootError> {
    match canonicalize_and_check_root(path, roots) {
        Ok(canonical) => Ok((canonical, false)),
        Err(OutOfRootError::OutOfRoot(canonical)) => Ok((canonical, true)),
        Err(e) => Err(e),
    }
}

/// Like `canonicalize_allow_external`, but an EMPTY root set (fresh install,
/// no workspace picked yet) also resolves as out-of-root instead of erroring.
/// Used by the deep-link dispatcher: rejecting every deep link on a rootless
/// install was a bug, and opening a file only displays it locally. Do NOT use
/// this for actions that send data off the machine.
pub fn canonicalize_allow_rootless(
    path: &Path,
    roots: &RootSet,
) -> Result<(PathBuf, bool), OutOfRootError> {
    match canonicalize_allow_external(path, roots) {
        Err(OutOfRootError::EmptyRoots) => match path.canonicalize() {
            Ok(canonical) => Ok((canonical, true)),
            Err(e) => Err(OutOfRootError::CanonicalizeFailed {
                path: path.to_path_buf(),
                reason: e.to_string(),
            }),
        },
        other => other,
    }
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

    #[test]
    fn allow_rootless_falls_through_on_empty_roots() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("adhoc.html");
        std::fs::write(&file, "x").unwrap();
        let (canonical, out_of_root) =
            canonicalize_allow_rootless(&file, &RootSet::empty()).expect("resolvable path");
        assert!(out_of_root);
        assert_eq!(canonical, file.canonicalize().unwrap());

        // Outbound actions stay conservative on a rootless install.
        assert!(canonicalize_allow_external(&file, &RootSet::empty()).is_err());
    }

    #[test]
    fn allow_rootless_still_rejects_unresolvable_paths() {
        assert!(canonicalize_allow_rootless(
            Path::new("/no/such/file/anywhere.html"),
            &RootSet::empty()
        )
        .is_err());
    }
}
