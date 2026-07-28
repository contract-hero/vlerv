// Workspace scanner — enumerates directory entries with lazy expansion and an
// in-session cache.
//
// Public API has two enumeration functions:
//   - `list_workspace_roots`: dirs-only (sidebar's project list view).
//   - `list_dir`: directories and files (project-tree view).
// Both share the same ordering / hidden-grouping / default-ignore semantics
// and the same in-session readdir cache.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// One directory entry returned by the scanner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// File or directory name (final path component, not the full path).
    pub name: String,
    /// Absolute, canonical path to the entry.
    pub path: PathBuf,
    /// True iff the entry resolves to a directory (after following symlinks).
    pub is_dir: bool,
    /// True iff the entry name starts with '.'.
    pub is_hidden: bool,
}

#[derive(Default)]
struct ScannerInner {
    readdir_counts: HashMap<PathBuf, usize>,
    /// C2 (CF3): number of per-entry errors logged-and-skipped during the
    /// most recent `list_dir` against each canonical directory.
    skip_counts: HashMap<PathBuf, usize>,
}

/// Stateful scanner that caches readdir results by canonical absolute path.
pub struct Scanner {
    inner: Mutex<ScannerInner>,
}

impl Scanner {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(ScannerInner::default()),
        }
    }

    /// Enumerate the workspace root's *directory* children only. Files at the
    /// workspace root are skipped (the sidebar surfaces projects only).
    pub fn list_workspace_roots(&self, dir: &Path) -> Result<Vec<Entry>, ScanError> {
        let all = self.list_dir(dir)?;
        Ok(all.into_iter().filter(|e| e.is_dir).collect())
    }

    /// List the immediate (depth-1) children of `dir`.
    pub fn list_dir(&self, dir: &Path) -> Result<Vec<Entry>, ScanError> {
        let canonical = canonicalize(dir)?;

        let meta = std::fs::metadata(&canonical).map_err(|source| ScanError::Io {
            path: canonical.clone(),
            reason: source.to_string(),
        })?;
        if !meta.is_dir() {
            return Err(ScanError::NotADirectory(canonical));
        }

        let read_iter = std::fs::read_dir(&canonical).map_err(|source| ScanError::Io {
            path: canonical.clone(),
            reason: source.to_string(),
        })?;

        let mut entries = Vec::new();
        let mut skip_count = 0usize;

        for raw in read_iter {
            // CF3: log and skip per-entry I/O errors instead of aborting.
            let raw = match raw {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("vlerv: list_dir skip error in {:?}: {}", canonical, e);
                    skip_count += 1;
                    continue;
                }
            };

            let name = raw.file_name().to_string_lossy().into_owned();

            if DEFAULT_IGNORED.contains(&name.as_str()) {
                continue;
            }

            let entry_path = raw.path();
            let is_hidden = name.starts_with('.');

            let resolved_meta = std::fs::metadata(&entry_path);

            // CF3: skip entries whose metadata is unreadable (e.g. chmod 000 dirs).
            let is_dir = match &resolved_meta {
                Ok(m) => m.is_dir(),
                Err(e) => {
                    eprintln!("vlerv: list_dir skip entry {:?}: {}", entry_path, e);
                    skip_count += 1;
                    continue;
                }
            };

            let canonical_path = entry_path
                .canonicalize()
                .unwrap_or_else(|_| entry_path.clone());

            entries.push(Entry {
                name,
                path: canonical_path,
                is_dir,
                is_hidden,
            });
        }

        entries.sort_by(|a, b| match (a.is_hidden, b.is_hidden) {
            (false, true) => std::cmp::Ordering::Less,
            (true, false) => std::cmp::Ordering::Greater,
            _ => a
                .name
                .to_lowercase()
                .cmp(&b.name.to_lowercase()),
        });

        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        *inner.readdir_counts.entry(canonical.clone()).or_insert(0) += 1;
        inner.skip_counts.insert(canonical, skip_count);

        Ok(entries)
    }

    /// Number of `readdir` calls issued for `dir` so far in this session.
    pub fn readdir_count_for(&self, dir: &Path) -> usize {
        let canonical = canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
        *self
            .inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .readdir_counts
            .get(&canonical)
            .unwrap_or(&0)
    }

    /// C2 (CF3): number of per-entry errors logged-and-skipped during the
    /// most recent `list_dir` against `dir`. Used by T-013.
    pub fn last_skip_count_for(&self, dir: &Path) -> usize {
        let canonical = canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
        *self
            .inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .skip_counts
            .get(&canonical)
            .unwrap_or(&0)
    }
}

impl Default for Scanner {
    fn default() -> Self {
        Self::new()
    }
}

fn canonicalize(path: &Path) -> Result<PathBuf, ScanError> {
    path.canonicalize().map_err(|source| ScanError::Io {
        path: path.to_path_buf(),
        reason: source.to_string(),
    })
}

#[derive(Debug, thiserror::Error, Serialize, Deserialize)]
pub enum ScanError {
    /// C2 (CF6): Io variant carries a `String` so the type is `Serialize`
    /// — `std::io::Error` is not `Serialize`.
    #[error("io error at {path:?}: {reason}")]
    Io {
        path: PathBuf,
        reason: String,
    },
    #[error("not a directory: {0:?}")]
    NotADirectory(PathBuf),
}

/// Default-ignored directory names filtered out during expansion.
pub const DEFAULT_IGNORED: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".venv",
];

/// Cap on entries returned by `list_files_recursive`. BFS order means the
/// shallowest paths survive truncation — better quick-open hits.
pub const MAX_INDEX_ENTRIES: usize = 20_000;

/// Flat recursive file index for the quick-open (⌘P) palette.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileIndex {
    /// Canonical root the entries are relative to.
    pub root: PathBuf,
    /// '/'-separated paths relative to `root`, in BFS (shallow-first) order.
    pub files: Vec<String>,
    /// True iff the walk stopped at the entry cap.
    pub truncated: bool,
}

/// Walk `root` breadth-first and return every file as a root-relative path.
/// Skips `DEFAULT_IGNORED` names, hidden *directories* (dot-dirs are
/// machinery), and symlinks (loop safety); keeps hidden *files* (dotfiles
/// are often artifacts). Per-entry I/O errors are logged and skipped.
pub fn list_files_recursive(root: &Path) -> Result<FileIndex, ScanError> {
    walk_files(root, MAX_INDEX_ENTRIES)
}

fn walk_files(root: &Path, cap: usize) -> Result<FileIndex, ScanError> {
    use std::collections::VecDeque;

    let canonical = canonicalize(root)?;
    let mut files = Vec::new();
    let mut truncated = false;
    let mut queue: VecDeque<PathBuf> = VecDeque::from([canonical.clone()]);

    'walk: while let Some(dir) = queue.pop_front() {
        let read_iter = match std::fs::read_dir(&dir) {
            Ok(it) => it,
            Err(e) => {
                eprintln!("vlerv: walk skip dir {dir:?}: {e}");
                continue;
            }
        };
        for raw in read_iter {
            let raw = match raw {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("vlerv: walk skip entry in {dir:?}: {e}");
                    continue;
                }
            };
            let name = raw.file_name().to_string_lossy().into_owned();
            if DEFAULT_IGNORED.contains(&name.as_str()) {
                continue;
            }
            // file_type() does NOT follow symlinks — skip them entirely for
            // loop safety (a symlinked artifact can still be opened via ⌘O).
            let file_type = match raw.file_type() {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("vlerv: walk skip entry {:?}: {e}", raw.path());
                    continue;
                }
            };
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                if !name.starts_with('.') {
                    queue.push_back(raw.path());
                }
                continue;
            }
            if files.len() >= cap {
                truncated = true;
                break 'walk;
            }
            let rel = raw
                .path()
                .strip_prefix(&canonical)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| raw.path().to_string_lossy().into_owned());
            files.push(rel);
        }
    }

    Ok(FileIndex {
        root: canonical,
        files,
        truncated,
    })
}

#[cfg(test)]
mod walk_tests {
    use super::*;
    use tempfile::TempDir;

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "x").unwrap();
    }

    #[test]
    fn returns_relative_paths() {
        let dir = TempDir::new().unwrap();
        touch(&dir.path().join("a.html"));
        touch(&dir.path().join("sub/b.md"));

        let idx = list_files_recursive(dir.path()).unwrap();
        let mut files = idx.files.clone();
        files.sort();
        assert_eq!(files, vec!["a.html", "sub/b.md"]);
        assert!(!idx.truncated);
    }

    #[test]
    fn ignored_dirs_are_excluded() {
        let dir = TempDir::new().unwrap();
        touch(&dir.path().join("keep.ts"));
        touch(&dir.path().join("node_modules/dep/index.js"));
        touch(&dir.path().join(".git/HEAD"));

        let idx = list_files_recursive(dir.path()).unwrap();
        assert_eq!(idx.files, vec!["keep.ts"]);
    }

    #[test]
    fn truncates_at_cap() {
        let dir = TempDir::new().unwrap();
        for i in 0..5 {
            touch(&dir.path().join(format!("f{i}.txt")));
        }

        let idx = walk_files(dir.path(), 3).unwrap();
        assert_eq!(idx.files.len(), 3);
        assert!(idx.truncated);
    }

    #[test]
    fn hidden_dirs_skipped_hidden_files_kept() {
        let dir = TempDir::new().unwrap();
        touch(&dir.path().join(".claude/settings.json"));
        touch(&dir.path().join(".env"));
        touch(&dir.path().join("visible.md"));

        let idx = list_files_recursive(dir.path()).unwrap();
        let mut files = idx.files.clone();
        files.sort();
        assert_eq!(files, vec![".env", "visible.md"]);
    }

    #[test]
    fn symlinked_dirs_not_followed() {
        let dir = TempDir::new().unwrap();
        touch(&dir.path().join("real/file.txt"));
        std::os::unix::fs::symlink(dir.path().join("real"), dir.path().join("loop")).unwrap();

        let idx = list_files_recursive(dir.path()).unwrap();
        assert_eq!(idx.files, vec!["real/file.txt"]);
    }
}
