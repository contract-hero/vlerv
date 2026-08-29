// File reader — returns raw text content for small text files, base64 for
// raster images, metadata-only for other binary or oversized files.
//
// Reads are deliberately NOT root-gated: this is a single-user local viewer
// whose deep-link contract intentionally opens out-of-root files (the
// "external file" badge flow), and ⌘O / the address bar / external tabs all
// depend on ungated reads. RootSet's job is workspace anchoring for
// deep-link classification, not read control.

use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// 10 MiB cap. Files strictly larger than this return metadata-only.
pub const MAX_TEXT_BYTES: u64 = 10 * 1024 * 1024;

/// 20 MiB cap for raster images (base64 inflates 4/3 → ~27 MB IPC payload
/// max). Larger than MAX_TEXT_BYTES because viewing images is a primary use
/// case and camera JPEGs routinely exceed 10 MiB; 20 MiB still keeps the
/// webview responsive.
pub const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;

/// Size of the binary-detection window (first 8 KiB).
const BINARY_PROBE_SIZE: usize = 8 * 1024;

/// How `FilePayload.content` is encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Encoding {
    #[default]
    Text,
    Base64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilePayload {
    pub path: PathBuf,
    /// File size in bytes.
    pub size: u64,
    /// mtime in seconds since Unix epoch.
    pub mtime: i64,
    /// True iff the first 8 KiB contains a NUL byte (or the file is a
    /// recognized raster image, which is binary by definition).
    pub is_binary: bool,
    /// True iff the file exceeded the size cap.
    pub oversized: bool,
    /// How `content` is encoded: UTF-8 text, or base64 for raster images.
    #[serde(default)]
    pub encoding: Encoding,
    /// File content when within the size cap: UTF-8 text for text files,
    /// base64 bytes for raster images. None for other binary or oversized
    /// files.
    pub content: Option<String>,
    /// True when the file was authored by someone other than this machine's
    /// user — a beam-received or a Scope-fetched artifact under the app's own
    /// `received/` or `cache/` dirs. The renderer isolates it (opaque-origin
    /// iframe, no `file://` base). Provenance travels ON the payload so the
    /// decision never depends on an async lookup the frontend races.
    #[serde(default)]
    pub untrusted: bool,
}

/// Raster image extensions served as base64 (rendered via data: URI in the
/// frontend). SVG is deliberately absent — it's text and renders inline.
const RASTER_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "avif",
];

fn is_raster(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| RASTER_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

#[derive(Debug, thiserror::Error, Serialize, Deserialize)]
pub enum ReadError {
    #[error("not found: {0:?}")]
    NotFound(PathBuf),
    /// Generic IO failure. Carries a String so the variant is Serialize —
    /// io::Error is not Serialize.
    #[error("io error at {path:?}: {reason}")]
    Io {
        path: PathBuf,
        reason: String,
    },
}

/// Read a file and return its payload. Raster images within the image cap
/// return base64 content; text files within the text cap return UTF-8
/// content; everything else returns metadata-only. Returns a typed error
/// for missing paths.
pub fn read_file(path: &Path) -> Result<FilePayload, ReadError> {
    let mut payload = read_file_with_caps(path, MAX_TEXT_BYTES, MAX_IMAGE_BYTES)?;
    payload.untrusted = is_untrusted_origin(path);
    Ok(payload)
}

/// True when `path` lives under the app's `received/` or `cache/` dirs — i.e.
/// a beam-received or Scope-fetched artifact authored by another machine.
/// Compared on the canonical form so a `..` cannot dodge the prefix check.
fn is_untrusted_origin(path: &Path) -> bool {
    let dirs = crate::remote::dirs();
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    for base in [dirs.received(), dirs.cache()] {
        if let Ok(base) = base.canonicalize() {
            if canonical.starts_with(&base) {
                return true;
            }
        }
    }
    false
}

/// Implementation with injectable caps so tests don't need multi-MiB files.
fn read_file_with_caps(
    path: &Path,
    max_text_bytes: u64,
    max_image_bytes: u64,
) -> Result<FilePayload, ReadError> {
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

    // Raster images bypass the NUL probe entirely: read whole file, base64.
    if is_raster(path) {
        if size > max_image_bytes {
            return Ok(FilePayload {
                untrusted: false,
                path: path.to_path_buf(),
                size,
                mtime,
                is_binary: true,
                oversized: true,
                encoding: Encoding::Base64,
                content: None,
            });
        }
        let bytes = std::fs::read(path).map_err(|e| ReadError::Io {
            path: path.to_path_buf(),
            reason: e.to_string(),
        })?;
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        return Ok(FilePayload {
                untrusted: false,
            path: path.to_path_buf(),
            size,
            mtime,
            is_binary: true,
            oversized: false,
            encoding: Encoding::Base64,
            content: Some(b64),
        });
    }

    // Oversized check — return metadata only, no content.
    if size > max_text_bytes {
        return Ok(FilePayload {
                untrusted: false,
            path: path.to_path_buf(),
            size,
            mtime,
            is_binary: false,
            oversized: true,
            encoding: Encoding::Text,
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
                untrusted: false,
            path: path.to_path_buf(),
            size,
            mtime,
            is_binary: true,
            oversized: false,
            encoding: Encoding::Text,
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
                untrusted: false,
        path: path.to_path_buf(),
        size,
        mtime,
        is_binary: false,
        oversized: false,
        encoding: Encoding::Text,
        content: Some(full_content),
    })
}

#[cfg(test)]
mod image_tests {
    use super::*;
    use base64::Engine as _;
    use tempfile::TempDir;

    // Minimal valid PNG header bytes — includes NULs so it would previously
    // have tripped the binary probe into a metadata-only payload.
    const PNG_MAGIC: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00];

    #[test]
    fn png_returns_base64_content() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("shot.png");
        std::fs::write(&p, PNG_MAGIC).unwrap();

        let payload = read_file(&p).expect("read");
        assert!(payload.is_binary);
        assert!(!payload.oversized);
        assert_eq!(payload.encoding, Encoding::Base64);
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(payload.content.expect("content"))
            .expect("valid base64");
        assert_eq!(decoded, PNG_MAGIC);
    }

    #[test]
    fn uppercase_extension_recognized() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("SHOT.PNG");
        std::fs::write(&p, PNG_MAGIC).unwrap();

        let payload = read_file(&p).expect("read");
        assert_eq!(payload.encoding, Encoding::Base64);
        assert!(payload.content.is_some());
    }

    #[test]
    fn image_over_cap_is_metadata_only() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("big.png");
        std::fs::write(&p, PNG_MAGIC).unwrap();

        let payload = read_file_with_caps(&p, MAX_TEXT_BYTES, 4).expect("read");
        assert!(payload.is_binary);
        assert!(payload.oversized);
        assert_eq!(payload.content, None);
    }

    #[test]
    fn text_file_has_text_encoding() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("notes.md");
        std::fs::write(&p, "# hello").unwrap();

        let payload = read_file(&p).expect("read");
        assert_eq!(payload.encoding, Encoding::Text);
        assert_eq!(payload.content.as_deref(), Some("# hello"));
    }

    #[test]
    fn encoding_serializes_to_the_lowercase_wire_shape() {
        // The TypeScript side (src/ipc.ts) declares `encoding?: "text" |
        // "base64"` and the router branches on those literal strings — pin
        // the serde representation, not just the Rust enum.
        let dir = TempDir::new().unwrap();
        let png = dir.path().join("wire.png");
        std::fs::write(&png, PNG_MAGIC).unwrap();
        let v = serde_json::to_value(read_file(&png).unwrap()).unwrap();
        assert_eq!(v["encoding"], "base64");

        let txt = dir.path().join("wire.txt");
        std::fs::write(&txt, "hi").unwrap();
        let v = serde_json::to_value(read_file(&txt).unwrap()).unwrap();
        assert_eq!(v["encoding"], "text");
    }

    #[test]
    fn missing_encoding_field_deserializes_to_text() {
        // Payloads serialized by older builds have no `encoding` key; the
        // #[serde(default)] must keep them deserializable as Text.
        let json = r#"{"path":"/tmp/a.md","size":1,"mtime":0,"is_binary":false,"oversized":false,"content":"x"}"#;
        let payload: FilePayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.encoding, Encoding::Text);
    }

    #[test]
    fn svg_stays_on_text_path() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("icon.svg");
        std::fs::write(&p, "<svg></svg>").unwrap();

        let payload = read_file(&p).expect("read");
        assert_eq!(payload.encoding, Encoding::Text);
        assert!(!payload.is_binary);
        assert_eq!(payload.content.as_deref(), Some("<svg></svg>"));
    }
}
