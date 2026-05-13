// File reader — returns raw text content for small text files, metadata-only
// for binary or oversized files. C1 implementation; C2 amendments below.

use crate::security::{canonicalize_and_check_root, RootSet};
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// 10 MiB cap. Files strictly larger than this return metadata-only.
pub const MAX_TEXT_BYTES: u64 = 10 * 1024 * 1024;

/// Size of the binary-detection window (first 8 KiB).
const BINARY_PROBE_SIZE: usize = 8 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilePayload {
    pub path: PathBuf,
    /// File size in bytes.
    pub size: u64,
    /// mtime in seconds since Unix epoch.
    pub mtime: i64,
    /// True iff the first 8 KiB contains a NUL byte.
    pub is_binary: bool,
    /// True iff the file exceeded the size cap.
    pub oversized: bool,
    /// Raw bytes as a UTF-8 string when text and within size cap. None for
    /// binary or oversized.
    pub content: Option<String>,
}

#[derive(Debug, thiserror::Error, Serialize, Deserialize)]
pub enum ReadError {
    /// Legacy C1 variant — kept so C1 reader tests continue to compile.
    #[error("not found: {0:?}")]
    NotFound(PathBuf),
    /// C2: missing file at the canonical path.
    #[error("missing: {0:?}")]
    Missing(PathBuf),
    /// C2: EACCES on the target.
    #[error("permission denied: {0:?}")]
    PermissionDenied(PathBuf),
    /// C2: file exceeds the 10 MB cap.
    #[error("oversize: {size} bytes")]
    Oversize { size: u64 },
    /// C2: NUL-byte detected in the first 8 KB.
    #[error("binary content")]
    Binary,
    /// C2: out-of-root rejection by the security gate.
    #[error("out of root")]
    OutOfRoot,
    /// Generic IO failure. C2: carries a String so the variant is Serialize
    /// (CF6) — io::Error is not Serialize.
    #[error("io error at {path:?}: {reason}")]
    Io {
        path: PathBuf,
        reason: String,
    },
}

/// C2: file payload returned by `read_file_with_roots`. Distinct from the
/// legacy `FilePayload` so C1 callers keep their shape. The frontend can
/// short-circuit on `kind: Binary | OversizeMetadata` without a second read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecuredFilePayload {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    pub kind: PayloadKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PayloadKind {
    Text,
    Binary,
    OversizeMetadata { size: u64, mtime: i64 },
}

/// Serialize a `ReadError` to a JSON string for logging or IPC transport.
/// This is the primary reason `serde_json` is declared as a dependency.
pub fn read_error_to_json(e: &ReadError) -> String {
    serde_json::to_string(e).unwrap_or_else(|_| format!("\"unknown error\""))
}

/// C2: gated read. Every call passes through `canonicalize_and_check_root`
/// before any file I/O on the target path.
pub fn read_file_with_roots(
    path: &Path,
    roots: &RootSet,
) -> Result<SecuredFilePayload, ReadError> {
    // Security gate: resolve symlinks and check root membership FIRST.
    // If canonicalize fails (missing path), return appropriate error.
    let canonical = match canonicalize_and_check_root(path, roots) {
        Ok(c) => c,
        Err(crate::security::OutOfRootError::OutOfRoot(_)) => {
            return Err(ReadError::OutOfRoot);
        }
        Err(crate::security::OutOfRootError::CanonicalizeFailed { path: p, .. }) => {
            // Missing file inside root (canonicalize fails because path doesn't exist)
            return Err(ReadError::Missing(p));
        }
        Err(crate::security::OutOfRootError::EmptyRoots) => {
            return Err(ReadError::OutOfRoot);
        }
    };

    // Check metadata.
    let metadata = std::fs::metadata(&canonical).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            ReadError::Missing(canonical.clone())
        } else if e.kind() == std::io::ErrorKind::PermissionDenied {
            ReadError::PermissionDenied(canonical.clone())
        } else {
            ReadError::Io { path: canonical.clone(), reason: e.to_string() }
        }
    })?;

    let size = metadata.len();
    let mtime = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    // Oversize: return metadata-only without reading body.
    if size > MAX_TEXT_BYTES {
        return Ok(SecuredFilePayload {
            path: canonical,
            bytes: Vec::new(),
            kind: PayloadKind::OversizeMetadata { size, mtime },
        });
    }

    // Open file for binary detection.
    let mut file = std::fs::File::open(&canonical).map_err(|e| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            ReadError::PermissionDenied(canonical.clone())
        } else {
            ReadError::Io { path: canonical.clone(), reason: e.to_string() }
        }
    })?;

    let probe_size = (size as usize).min(BINARY_PROBE_SIZE);
    let mut probe = vec![0u8; probe_size];
    if probe_size > 0 {
        file.read_exact(&mut probe).map_err(|e| ReadError::Io { path: canonical.clone(), reason: e.to_string() })?;
    }

    let is_binary = probe.contains(&0u8);

    if is_binary {
        return Ok(SecuredFilePayload {
            path: canonical,
            bytes: Vec::new(),
            kind: PayloadKind::Binary,
        });
    }

    // Read remaining bytes.
    let mut rest = Vec::new();
    file.read_to_end(&mut rest).map_err(|e| ReadError::Io { path: canonical.clone(), reason: e.to_string() })?;

    let mut all = probe;
    all.extend_from_slice(&rest);

    Ok(SecuredFilePayload {
        path: canonical,
        bytes: all,
        kind: PayloadKind::Text,
    })
}

/// Read a file and return its payload. Returns metadata-only for binary or
/// oversized files. Returns a typed error for missing paths.
pub fn read_file(path: &Path) -> Result<FilePayload, ReadError> {
    // Check existence and gather metadata.
    let metadata = std::fs::metadata(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            ReadError::NotFound(path.to_path_buf())
        } else {
            ReadError::Io { path: path.to_path_buf(), reason: e.to_string() }
        }
    })?;

    let size = metadata.len();

    let mtime = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    // Oversized check — return metadata only, no content.
    if size > MAX_TEXT_BYTES {
        return Ok(FilePayload {
            path: path.to_path_buf(),
            size,
            mtime,
            is_binary: false,
            oversized: true,
            content: None,
        });
    }

    // Open and read up to BINARY_PROBE_SIZE bytes to detect binary.
    let mut file = std::fs::File::open(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            ReadError::NotFound(path.to_path_buf())
        } else {
            ReadError::Io { path: path.to_path_buf(), reason: e.to_string() }
        }
    })?;

    let probe_size = (size as usize).min(BINARY_PROBE_SIZE);
    let mut probe = vec![0u8; probe_size];
    if probe_size > 0 {
        file.read_exact(&mut probe).map_err(|e| ReadError::Io { path: path.to_path_buf(), reason: e.to_string() })?;
    }

    let is_binary = probe.contains(&0u8);

    if is_binary {
        return Ok(FilePayload {
            path: path.to_path_buf(),
            size,
            mtime,
            is_binary: true,
            oversized: false,
            content: None,
        });
    }

    // Read the full file as text.
    let full_content = if size == 0 {
        String::new()
    } else {
        // Read remaining bytes after the probe.
        let mut rest = Vec::new();
        file.read_to_end(&mut rest).map_err(|e| ReadError::Io { path: path.to_path_buf(), reason: e.to_string() })?;

        // Combine probe + rest.
        let mut all = probe;
        all.extend_from_slice(&rest);

        String::from_utf8_lossy(&all).into_owned()
    };

    Ok(FilePayload {
        path: path.to_path_buf(),
        size,
        mtime,
        is_binary: false,
        oversized: false,
        content: Some(full_content),
    })
}
