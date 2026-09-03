// Tool arguments: the JSON Schema an LLM caller sees, and the validation that
// runs before any of them reaches the network.
//
// Every check here is a pure function over strings. The rule is the same one
// the app's IPC layer follows: reject a malformed argument with a message that
// says what to send instead, and never let a defaulted value silently widen
// what the call does.

use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::Deserialize;
// TTL bounds for a beam link, in hours: the crate's own, so the range this
// server refuses outside of is exactly the range `beam::offer` serves. The
// floor keeps a caller from minting an already-dead link; the ceiling is the
// 30 days `beam::offer` would otherwise clamp to silently.
use vlerv_remote::beam::{DEFAULT_TTL_HOURS, MAX_TTL_HOURS, MIN_TTL_HOURS};
use vlerv_remote::peers::Scope;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct BeamArtifactArgs {
    /// Absolute path of the file to send. A relative path resolves against the
    /// server's working directory.
    pub path: String,
    /// How long the link stays fetchable, in hours (1-720). Defaults to 24.
    #[serde(default)]
    pub ttl_hours: Option<u32>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct StopBeamArgs {
    /// Which link to revoke: the content hash reported by beam_artifact or
    /// server_status, or a prefix of it (8 characters or more). Omit it to
    /// revoke every link this server is still serving.
    #[serde(default)]
    pub hash: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct ListDevicesArgs {
    /// Dial every paired device to report live presence. Costs a network
    /// round trip per device; without it presence comes from the last
    /// handshake this server saw.
    #[serde(default)]
    pub probe: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SendToDeviceArgs {
    /// Absolute path of the file to send. A relative path resolves against the
    /// server's working directory.
    pub path: String,
    /// Which paired device to send to: its name, or a prefix of its node id.
    pub device: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ForgetDeviceArgs {
    /// Which paired device to forget: its name, or a prefix of its node id.
    pub device: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ConfirmPairingArgs {
    /// True completes the pairing, false discards it. A discarded pairing
    /// writes nothing to disk.
    pub accept: bool,
    /// Which pending pairing to resolve. Optional while exactly one is
    /// waiting; required when more than one is.
    #[serde(default)]
    pub node_id: Option<String>,
    /// What the new device may do on THIS server: "view-open", "browse" or
    /// "control". A new device defaults to "view-open", the narrowest grant.
    /// Naming a scope for a device that is already paired REPLACES its grant,
    /// including narrowing it; omitting it leaves that grant unchanged.
    #[serde(default)]
    pub scope: Option<String>,
}

/// Resolve a caller-supplied path argument. `cwd` and `home` are parameters,
/// not process lookups, so the rule is testable without touching the
/// environment.
///
/// This is argument hygiene only. Whether the file may actually be sent is
/// decided later by `beam::resolve_offerable` against the RootSet — the same
/// gate the desktop share sheet uses.
pub fn resolve_arg_path(raw: &str, cwd: &Path, home: Option<&Path>) -> Result<PathBuf, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("path must not be empty".to_string());
    }
    if trimmed.contains('\0') {
        return Err("path must not contain NUL bytes".to_string());
    }
    if trimmed == "~" || trimmed.starts_with("~/") {
        let home = home.ok_or_else(|| "cannot expand ~: no home directory".to_string())?;
        let rest = trimmed.trim_start_matches('~').trim_start_matches('/');
        return Ok(if rest.is_empty() { home.to_path_buf() } else { home.join(rest) });
    }
    let path = Path::new(trimmed);
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(cwd.join(path))
    }
}

/// Clamp-free TTL validation: an out-of-range number is an error, because a
/// caller that asked for 0 hours or 10 years wanted something this server
/// cannot give and should be told so.
pub fn validate_ttl(hours: Option<u32>) -> Result<u32, String> {
    match hours {
        None => Ok(DEFAULT_TTL_HOURS),
        Some(h) if (MIN_TTL_HOURS..=MAX_TTL_HOURS).contains(&h) => Ok(h),
        Some(h) => Err(format!(
            "ttl_hours must be between {MIN_TTL_HOURS} and {MAX_TTL_HOURS}, got {h}"
        )),
    }
}

/// A device argument is free text matched against the peer list later; the
/// only thing to reject here is nothing at all.
pub fn validate_device_query(raw: &str) -> Result<&str, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("device must not be empty — call list_devices to see the paired names".to_string());
    }
    Ok(trimmed)
}

/// The crate's optional-scope rule, plus the list of valid values. The
/// PARSING lives in `Scope::parse_optional`, so this server and the app's
/// command layer cannot drift on trimming or on what an omitted argument
/// means; only the wording of the refusal belongs to this server.
pub fn validate_optional_scope(raw: Option<&str>) -> Result<Option<Scope>, String> {
    Scope::parse_optional(raw)
        .map_err(|e| format!("{e} — use \"view-open\", \"browse\" or \"control\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cwd() -> PathBuf {
        PathBuf::from("/work/project")
    }

    #[test]
    fn an_absolute_path_passes_through_unchanged() {
        let got = resolve_arg_path("/w/report.html", &cwd(), None).unwrap();
        assert_eq!(got, PathBuf::from("/w/report.html"));
    }

    #[test]
    fn a_relative_path_resolves_against_the_working_directory() {
        let got = resolve_arg_path("dist/report.html", &cwd(), None).unwrap();
        assert_eq!(got, PathBuf::from("/work/project/dist/report.html"));
        // Surrounding whitespace is a copy-paste artifact, not part of a name.
        assert_eq!(
            resolve_arg_path("  dist/report.html  ", &cwd(), None).unwrap(),
            PathBuf::from("/work/project/dist/report.html")
        );
    }

    #[test]
    fn a_tilde_expands_only_when_a_home_directory_is_known() {
        let home = PathBuf::from("/Users/x");
        assert_eq!(
            resolve_arg_path("~/notes/a.md", &cwd(), Some(&home)).unwrap(),
            PathBuf::from("/Users/x/notes/a.md")
        );
        assert_eq!(resolve_arg_path("~", &cwd(), Some(&home)).unwrap(), home);
        assert!(resolve_arg_path("~/a.md", &cwd(), None).is_err());
        // A bare `~name` is NOT a home directory reference — leave it alone
        // rather than guessing another user's directory.
        assert_eq!(
            resolve_arg_path("~other/a.md", &cwd(), Some(&home)).unwrap(),
            PathBuf::from("/work/project/~other/a.md")
        );
    }

    #[test]
    fn empty_and_nul_paths_are_refused() {
        assert_eq!(resolve_arg_path("", &cwd(), None).unwrap_err(), "path must not be empty");
        assert_eq!(resolve_arg_path("   ", &cwd(), None).unwrap_err(), "path must not be empty");
        assert_eq!(
            resolve_arg_path("/w/a\0b.html", &cwd(), None).unwrap_err(),
            "path must not contain NUL bytes"
        );
    }

    #[test]
    fn ttl_defaults_to_a_day_and_refuses_out_of_range_values() {
        assert_eq!(validate_ttl(None).unwrap(), DEFAULT_TTL_HOURS);
        assert_eq!(validate_ttl(Some(1)).unwrap(), 1);
        assert_eq!(validate_ttl(Some(MAX_TTL_HOURS)).unwrap(), MAX_TTL_HOURS);
        // Zero would mint a link that is already dead; the ceiling is the same
        // one `beam::offer` clamps to, surfaced as an error instead.
        assert!(validate_ttl(Some(0)).is_err());
        assert!(validate_ttl(Some(MAX_TTL_HOURS + 1)).is_err());
        assert!(validate_ttl(Some(u32::MAX)).is_err());
    }

    #[test]
    fn a_device_query_must_carry_something_to_match_on() {
        assert_eq!(validate_device_query("  MacBook  ").unwrap(), "MacBook");
        assert!(validate_device_query("").is_err());
        assert!(validate_device_query("\t \n").is_err());
    }

    #[test]
    fn an_optional_scope_keeps_the_difference_between_none_and_a_named_grant() {
        // `None` must stay `None` all the way to the peer store: it is what
        // tells `confirm` to leave an existing grant where it is.
        assert_eq!(validate_optional_scope(None).unwrap(), None);
        assert_eq!(validate_optional_scope(Some(" control ")).unwrap(), Some(Scope::Control));
        assert_eq!(validate_optional_scope(Some("view-open")).unwrap(), Some(Scope::ViewOpen));
        assert_eq!(validate_optional_scope(Some("browse")).unwrap(), Some(Scope::Browse));
        // A typo is a refusal, not a default — it must never widen or narrow
        // a grant silently. Case matters: the wire spelling is lower-kebab.
        assert!(validate_optional_scope(Some("")).is_err());
        assert!(validate_optional_scope(Some("Control")).is_err());
        assert!(validate_optional_scope(Some("admin")).is_err());
    }

}
