// Deep-link parser — `vlerv://open?path=<abs>`, `vlerv://reveal?path=<abs>`,
// `vlerv://receive?ticket=<t>`, `vlerv://beam?path=<abs>`.
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
    /// Beam v1: an incoming artifact offer. `name`/`size` are untrusted
    /// display hints; the ticket's hash is the truth. Parsing performs NO
    /// network action — the confirm dialog stands between this intent and
    /// any fetch.
    Receive {
        ticket: String,
        name: Option<String>,
        size: Option<u64>,
    },
    /// Beam v1: ask the app to offer a file (CLI `vlerv beam <path>`).
    /// Opens the send dialog — a hostile link cannot mint an offer without
    /// the user clicking through it.
    Beam {
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

// ── Per-verb building blocks ────────────────────────────────────────────────
// Every verb arm is composed from these, so a new verb cannot silently lose
// a validation step — exactly what happened once when `reveal` was copied
// from `open` without the absolute-path/NUL checks.

/// Parse the verb's query string, treating a missing query as a missing
/// `param` (the parameter the verb cannot exist without).
fn parse_query_for(
    input: &str,
    query_opt: Option<&str>,
    param: &str,
) -> Result<Vec<(String, String)>, DeepLinkError> {
    let query = query_opt.ok_or_else(|| DeepLinkError::MissingParameter {
        input: input.to_string(),
        param: param.to_string(),
    })?;
    parse_query_strict(query).map_err(|reason| DeepLinkError::Malformed {
        input: input.to_string(),
        reason,
    })
}

/// Find a required, non-empty parameter value.
fn require_param<'a>(
    input: &str,
    params: &'a [(String, String)],
    param: &str,
) -> Result<&'a str, DeepLinkError> {
    params
        .iter()
        .find(|(k, _)| k == param)
        .map(|(_, v)| v.as_str())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| DeepLinkError::MissingParameter {
            input: input.to_string(),
            param: param.to_string(),
        })
}

/// Find an optional, non-empty parameter value.
fn optional_param(params: &[(String, String)], param: &str) -> Option<String> {
    params
        .iter()
        .find(|(k, _)| k == param)
        .map(|(_, v)| v.clone())
        .filter(|v| !v.is_empty())
}

/// The required `path` parameter with the shared validation every path verb
/// gets: absolute, no NUL bytes (T-017).
fn require_abs_path(
    input: &str,
    params: &[(String, String)],
) -> Result<PathBuf, DeepLinkError> {
    let path_val = require_param(input, params, "path")?;

    if !path_val.starts_with('/') {
        return Err(DeepLinkError::Malformed {
            input: input.to_string(),
            reason: "path must be absolute (start with '/')".to_string(),
        });
    }

    if path_val.contains('\0') {
        return Err(DeepLinkError::Malformed {
            input: input.to_string(),
            reason: "path must not contain NUL bytes".to_string(),
        });
    }

    Ok(PathBuf::from(path_val))
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
            let params = parse_query_for(&input, query_opt, "path")?;
            let path = require_abs_path(&input, &params)?;
            let line = params
                .iter()
                .find(|(k, _)| k == "line")
                .and_then(|(_, v)| v.parse::<u32>().ok());
            Ok(DeepLinkIntent::Open { path, line })
        }
        "reveal" => {
            let params = parse_query_for(&input, query_opt, "path")?;
            let path = require_abs_path(&input, &params)?;
            Ok(DeepLinkIntent::Reveal { path })
        }
        "receive" => {
            let params = parse_query_for(&input, query_opt, "ticket")?;
            let ticket = require_param(&input, &params, "ticket")?;

            // Tickets are base32 strings. Alphanumeric-only is a strict
            // superset that rejects NUL, separators, and quoting tricks
            // before the ticket parser ever sees the value.
            if !ticket.chars().all(|c| c.is_ascii_alphanumeric()) {
                return Err(DeepLinkError::Malformed {
                    input: input.clone(),
                    reason: "ticket must be alphanumeric".to_string(),
                });
            }

            Ok(DeepLinkIntent::Receive {
                ticket: ticket.to_string(),
                name: optional_param(&params, "name"),
                size: optional_param(&params, "size").and_then(|v| v.parse::<u64>().ok()),
            })
        }
        "beam" => {
            let params = parse_query_for(&input, query_opt, "path")?;
            let path = require_abs_path(&input, &params)?;
            Ok(DeepLinkIntent::Beam { path })
        }
        other => Err(DeepLinkError::UnknownVerb {
            input: input.clone(),
            verb: other.to_string(),
        }),
    }
}
