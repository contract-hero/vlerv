// Deep-link parser — `vlerv://open?path=<abs>`, `vlerv://reveal?path=<abs>`.
// Pure URL parsing, no Tauri dependency.

use percent_encoding::percent_decode_str;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeepLinkIntent {
    Open {
        path: PathBuf,
        line: Option<u32>,
    },
    /// C3: the `reveal` verb selects + expands in the tree without switching the preview.
    Reveal {
        path: PathBuf,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum DeepLinkError {
    #[error("not a vlerv:// url: {input:?}")]
    NotVlervScheme { input: String },
    #[error("malformed url: {input:?} — {reason}")]
    Malformed { input: String, reason: String },
    #[error("missing required parameter {param:?} in {input:?}")]
    MissingParameter { input: String, param: String },
    #[error("unknown verb {verb:?} in {input:?}")]
    UnknownVerb { input: String, verb: String },
    #[error("verb {verb:?} is not yet implemented")]
    NotYetImplemented { verb: String },
}

/// Decode percent-encoded query-string value into a String. UTF-8-safe.
/// Returns Err if the percent-encoding is invalid (bad sequences like %ZZ).
fn pct_decode_value_strict(s: &str) -> Result<String, String> {
    // Pre-validate: check that every `%` is followed by exactly two valid hex digits.
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '%' {
            if i + 2 >= chars.len() {
                return Err(format!("truncated percent-sequence at position {i}"));
            }
            let h1 = chars[i + 1];
            let h2 = chars[i + 2];
            if !h1.is_ascii_hexdigit() || !h2.is_ascii_hexdigit() {
                return Err(format!("invalid percent-sequence %{h1}{h2} at position {i}"));
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    // Use percent_encoding crate directly (T-016: handles multibyte UTF-8 correctly).
    percent_decode_str(s)
        .decode_utf8()
        .map(|cow| cow.into_owned())
        .map_err(|e| e.to_string())
}

/// Parse a query string using strict UTF-8 decoding.
fn parse_query_strict(query: &str) -> Result<Vec<(String, String)>, String> {
    let mut result = Vec::new();
    for pair in query.split('&') {
        let mut parts = pair.splitn(2, '=');
        let key = match parts.next() {
            Some(k) if !k.is_empty() => k,
            _ => continue,
        };
        let val = parts.next().unwrap_or("");
        let decoded = pct_decode_value_strict(val)?;
        result.push((key.to_string(), decoded));
    }
    Ok(result)
}

/// Parse a `vlerv://` URL string into a typed intent.
pub fn parse(url: &str) -> Result<DeepLinkIntent, DeepLinkError> {
    let input = url.to_string();

    let rest = url.strip_prefix("vlerv://").ok_or_else(|| {
        DeepLinkError::NotVlervScheme {
            input: input.clone(),
        }
    })?;

    if rest.is_empty() {
        return Err(DeepLinkError::Malformed {
            input: input.clone(),
            reason: "missing verb".to_string(),
        });
    }

    let (verb_part, query_opt) = match rest.find('?') {
        Some(pos) => (&rest[..pos], Some(&rest[pos + 1..])),
        None => (rest, None),
    };

    if verb_part.is_empty() {
        return Err(DeepLinkError::Malformed {
            input: input.clone(),
            reason: "empty verb".to_string(),
        });
    }

    match verb_part {
        "open" => {
            let query = query_opt.ok_or_else(|| DeepLinkError::MissingParameter {
                input: input.clone(),
                param: "path".to_string(),
            })?;

            // Use strict decoding to catch invalid percent-encoding (e.g. %ZZ).
            let params = parse_query_strict(query).map_err(|reason| DeepLinkError::Malformed {
                input: input.clone(),
                reason,
            })?;

            let path_val = params
                .iter()
                .find(|(k, _)| k == "path")
                .map(|(_, v)| v.as_str())
                .ok_or_else(|| DeepLinkError::MissingParameter {
                    input: input.clone(),
                    param: "path".to_string(),
                })?;

            if path_val.is_empty() {
                return Err(DeepLinkError::MissingParameter {
                    input: input.clone(),
                    param: "path".to_string(),
                });
            }

            // Reject non-absolute paths (T-017).
            if !path_val.starts_with('/') {
                return Err(DeepLinkError::Malformed {
                    input: input.clone(),
                    reason: "path must be absolute (start with '/')".to_string(),
                });
            }

            // Reject paths containing NUL bytes (T-017).
            if path_val.contains('\0') {
                return Err(DeepLinkError::Malformed {
                    input: input.clone(),
                    reason: "path must not contain NUL bytes".to_string(),
                });
            }

            let line = params
                .iter()
                .find(|(k, _)| k == "line")
                .and_then(|(_, v)| v.parse::<u32>().ok());

            Ok(DeepLinkIntent::Open {
                path: PathBuf::from(path_val),
                line,
            })
        }
        "reveal" => {
            let query = query_opt.ok_or_else(|| DeepLinkError::MissingParameter {
                input: input.clone(),
                param: "path".to_string(),
            })?;

            let params = parse_query_strict(query).map_err(|reason| DeepLinkError::Malformed {
                input: input.clone(),
                reason,
            })?;

            let path_val = params
                .iter()
                .find(|(k, _)| k == "path")
                .map(|(_, v)| v.as_str())
                .ok_or_else(|| DeepLinkError::MissingParameter {
                    input: input.clone(),
                    param: "path".to_string(),
                })?;

            if path_val.is_empty() {
                return Err(DeepLinkError::MissingParameter {
                    input: input.clone(),
                    param: "path".to_string(),
                });
            }

            // Same validation as the `open` arm (previously asymmetric).
            if !path_val.starts_with('/') {
                return Err(DeepLinkError::Malformed {
                    input: input.clone(),
                    reason: "path must be absolute (start with '/')".to_string(),
                });
            }

            if path_val.contains('\0') {
                return Err(DeepLinkError::Malformed {
                    input: input.clone(),
                    reason: "path must not contain NUL bytes".to_string(),
                });
            }

            Ok(DeepLinkIntent::Reveal {
                path: PathBuf::from(path_val),
            })
        }
        other => Err(DeepLinkError::UnknownVerb {
            input: input.clone(),
            verb: other.to_string(),
        }),
    }
}
