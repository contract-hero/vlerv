// Turning "send it to my phone" into exactly one paired peer — and, on the
// same machinery, "confirm THAT pairing" into exactly one parked pairing.
//
// The caller is a language model with a free-text name, so the match is
// deliberately forgiving — full node id, device name, a prefix of either — and
// deliberately refuses to guess: two candidates is an error that lists both,
// never the first one sorted. Sending a file to the wrong machine is not
// undoable, and confirming the wrong pairing writes trust to disk.
//
// The prefix pass itself (`needle` + `by_prefix`) is shared: every "which one
// did you mean?" argument in this crate — a device, a waiting pairing, a live
// beam link — trims it, refuses one too short to mean anything, and matches it
// case-insensitively. Only what one, none, or many matches MEAN differs, and
// that stays with each caller.

use vlerv_remote::peers::{short_id, Peer, PendingPair};

/// Why a device argument did not name exactly one peer. Every variant carries
/// the material for a message that tells the caller what IS valid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// The peer store is empty: nothing is paired yet.
    NotPaired,
    /// Nothing matched.
    Unknown { query: String, known: Vec<String> },
    /// More than one peer matched.
    Ambiguous { query: String, matches: Vec<String> },
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolveError::NotPaired => f.write_str(
                "no devices are paired with this server yet — call pair_device to start pairing",
            ),
            ResolveError::Unknown { query, known } => write!(
                f,
                "no paired device matches {query:?}. Paired devices: {}",
                known.join(", ")
            ),
            ResolveError::Ambiguous { query, matches } => write!(
                f,
                "{query:?} matches more than one paired device: {}. Use the full name or a longer node id prefix",
                matches.join(", ")
            ),
        }
    }
}

/// Shortest node-id prefix accepted. Four hex characters is 16 bits — enough
/// that a typo does not land on a real peer, short enough to type.
pub const MIN_PREFIX: usize = 4;

/// Normalize a prefix argument: trim it, then refuse one shorter than
/// `min_chars`. `starts_with` on an unchecked argument accepts the empty
/// string and matches everything, so the floor is what keeps a resolver from
/// picking whichever candidate happened to come first. Returns the lowercase
/// needle a prefix match compares against.
pub fn needle(query: &str, min_chars: usize) -> Option<String> {
    let query = query.trim();
    (query.len() >= min_chars).then(|| query.to_lowercase())
}

/// Every item whose id starts with `needle` — which must already be lowercase,
/// i.e. come from `needle()`.
///
/// It returns ALL matches on purpose: `resolve_device` and `resolve_pending`
/// treat two as an error, while revoking beam links revokes both. Deciding is
/// the caller's job.
pub fn by_prefix<'a, T>(
    items: &'a [T],
    needle: &str,
    id_of: impl Fn(&T) -> &str,
) -> Vec<&'a T> {
    items.iter().filter(|item| id_of(item).to_lowercase().starts_with(needle)).collect()
}

/// Something a prefix argument can name and that is named back to the caller
/// the same way: a paired peer, or a pairing still waiting for its fingerprint
/// check. Both carry a device name and a node id, and both must be shown with
/// the short id that tells two same-named machines apart.
pub trait Named {
    fn device(&self) -> &str;
    fn node_id(&self) -> &str;
}

impl Named for Peer {
    fn device(&self) -> &str {
        &self.device
    }
    fn node_id(&self) -> &str {
        &self.node_id
    }
}

impl Named for PendingPair {
    fn device(&self) -> &str {
        &self.device
    }
    fn node_id(&self) -> &str {
        &self.node_id
    }
}

/// Match `query` against the paired peers.
///
/// Order, most specific first: a full node id, then an exact device name, then
/// a node-id prefix, then a device-name substring. The first tier that
/// produces candidates decides — so an exact name always beats somebody else's
/// substring — and a tier that produces two candidates is an error.
pub fn resolve_device(peers: &[Peer], query: &str) -> Result<Peer, ResolveError> {
    if peers.is_empty() {
        return Err(ResolveError::NotPaired);
    }
    let lowered = query.trim().to_lowercase();

    if let Some(peer) = peers.iter().find(|p| p.node_id.eq_ignore_ascii_case(&lowered)) {
        return Ok(peer.clone());
    }

    let exact_name: Vec<&Peer> =
        peers.iter().filter(|p| p.device.to_lowercase() == lowered).collect();
    if !exact_name.is_empty() {
        return decide(query, exact_name);
    }

    let mut fuzzy: Vec<&Peer> = Vec::new();
    if let Some(prefix) = needle(&lowered, MIN_PREFIX) {
        fuzzy.extend(by_prefix(peers, &prefix, |p| p.node_id.as_str()));
    }
    for peer in peers.iter().filter(|p| p.device.to_lowercase().contains(&lowered)) {
        if !fuzzy.iter().any(|p| p.node_id == peer.node_id) {
            fuzzy.push(peer);
        }
    }
    if fuzzy.is_empty() {
        return Err(ResolveError::Unknown {
            query: query.trim().to_string(),
            known: peers.iter().map(label).collect(),
        });
    }
    decide(query, fuzzy)
}

fn decide(query: &str, candidates: Vec<&Peer>) -> Result<Peer, ResolveError> {
    match candidates.len() {
        1 => Ok(candidates[0].clone()),
        _ => Err(ResolveError::Ambiguous {
            query: query.trim().to_string(),
            matches: candidates.into_iter().map(label).collect(),
        }),
    }
}

/// Which parked pairing a `node_id` argument names, or the only one waiting
/// when the argument is absent.
///
/// Same prefix rule as `resolve_device` with a higher floor, and the same
/// refusal to guess — confirming is the one step that writes trust to disk, so
/// two candidates is an error that lists both, never the first one parked.
pub fn resolve_pending<'a>(
    parked: &'a [PendingPair],
    query: Option<&str>,
) -> Result<&'a PendingPair, String> {
    let Some(query) = query else {
        return match parked {
            [] => Err("no pairing is waiting for confirmation — call pair_device and open the \
                       link on the other device first"
                .to_string()),
            [only] => Ok(only),
            many => Err(format!(
                "{} pairings are waiting; pass node_id to say which one: {}",
                many.len(),
                names(many.iter())
            )),
        };
    };
    let query = query.trim();
    let prefix = needle(query, MIN_PREFIX).ok_or_else(|| {
        format!(
            "node_id must be at least {MIN_PREFIX} characters — pass the node id from pair_status"
        )
    })?;
    match by_prefix(parked, &prefix, |p| p.node_id.as_str()).as_slice() {
        [only] => Ok(only),
        [] => Err(format!("no pairing is waiting for {query:?}")),
        many => Err(format!(
            "{query:?} matches {} waiting pairings: {}. Pass the full node id",
            many.len(),
            names(many.iter().copied())
        )),
    }
}

/// How a peer or a pending pairing is named back to the caller: the device
/// name plus a short node id, which is what disambiguates two machines with
/// the same name.
pub fn label<T: Named>(item: &T) -> String {
    format!("{} ({})", item.device(), short_id(item.node_id()))
}

/// A comma-separated list of labels, for a refusal that has to say which
/// candidates it could not choose between.
fn names<'a, T: Named + 'a>(items: impl Iterator<Item = &'a T>) -> String {
    items.map(label).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use vlerv_remote::peers::Scope;

    fn peer(node_id: &str, device: &str) -> Peer {
        Peer {
            node_id: node_id.to_string(),
            device: device.to_string(),
            scope: Scope::Control,
            paired_at: 1,
            last_seen: 2,
            last_ack_scope: None,
        }
    }

    fn fleet() -> Vec<Peer> {
        vec![
            peer(&"aa".repeat(32), "Val's iPhone"),
            peer(&"bb".repeat(32), "Mac Studio"),
            peer(&"cc".repeat(32), "MacBook Pro"),
        ]
    }

    #[test]
    fn an_empty_peer_store_says_so_instead_of_saying_not_found() {
        let err = resolve_device(&[], "anything").unwrap_err();
        assert_eq!(err, ResolveError::NotPaired);
        assert!(err.to_string().contains("pair_device"));
    }

    #[test]
    fn a_full_node_id_matches_regardless_of_case() {
        let peers = fleet();
        assert_eq!(resolve_device(&peers, &"AA".repeat(32)).unwrap().device, "Val's iPhone");
        assert_eq!(resolve_device(&peers, &"aa".repeat(32)).unwrap().device, "Val's iPhone");
    }

    #[test]
    fn an_exact_name_wins_over_another_peers_substring() {
        // "Mac Studio" is a substring of nothing else, but "MacBook Pro" also
        // contains "Mac" — the exact tier must settle it without ambiguity.
        let peers = fleet();
        assert_eq!(resolve_device(&peers, "Mac Studio").unwrap().node_id, "bb".repeat(32));
        assert_eq!(resolve_device(&peers, "mac studio").unwrap().node_id, "bb".repeat(32));
    }

    #[test]
    fn a_node_id_prefix_matches_from_four_characters() {
        let peers = fleet();
        assert_eq!(resolve_device(&peers, "bbbbbbbb").unwrap().device, "Mac Studio");
        assert_eq!(resolve_device(&peers, "bbbb").unwrap().device, "Mac Studio");
        // Three characters is below the floor, so it is not a prefix match —
        // and "bbb" is in no device name either.
        assert!(matches!(
            resolve_device(&peers, "bbb").unwrap_err(),
            ResolveError::Unknown { .. }
        ));
    }

    #[test]
    fn a_name_substring_matches_when_it_is_unique() {
        let peers = fleet();
        assert_eq!(resolve_device(&peers, "iphone").unwrap().device, "Val's iPhone");
        assert_eq!(resolve_device(&peers, "Book").unwrap().device, "MacBook Pro");
    }

    #[test]
    fn two_candidates_are_an_error_that_lists_both() {
        let peers = fleet();
        let err = resolve_device(&peers, "Mac").unwrap_err();
        let ResolveError::Ambiguous { matches, .. } = &err else {
            panic!("expected ambiguity, got {err:?}");
        };
        assert_eq!(matches.len(), 2);
        let text = err.to_string();
        assert!(text.contains("Mac Studio (bbbbbbbbbb)"), "{text}");
        assert!(text.contains("MacBook Pro (cccccccccc)"), "{text}");
    }

    #[test]
    fn two_machines_with_the_same_name_are_ambiguous_not_first_wins() {
        let peers = vec![peer(&"aa".repeat(32), "MacBook"), peer(&"bb".repeat(32), "MacBook")];
        let err = resolve_device(&peers, "MacBook").unwrap_err();
        assert!(matches!(err, ResolveError::Ambiguous { .. }));
        // The node id prefix is the way out of the tie.
        assert_eq!(resolve_device(&peers, "bbbb").unwrap().node_id, "bb".repeat(32));
    }

    #[test]
    fn an_unknown_name_lists_the_valid_ones() {
        let peers = fleet();
        let err = resolve_device(&peers, "Pixel").unwrap_err();
        let text = err.to_string();
        assert!(text.contains("\"Pixel\""), "{text}");
        for expected in [
            "Val's iPhone (aaaaaaaaaa)",
            "Mac Studio (bbbbbbbbbb)",
            "MacBook Pro (cccccccccc)",
        ] {
            assert!(text.contains(expected), "{text} is missing {expected}");
        }
    }

    #[test]
    fn a_label_shows_the_same_short_id_the_crate_logs() {
        // 10 characters, the width iroh's own `fmt_short` prints — a short id
        // copied from a log line and one shown in a list are one string.
        assert_eq!(label(&peer(&"ab".repeat(32), "Mac")), "Mac (ababababab)");
    }

    // ── the shared prefix pass ─────────────────────────────────────────────

    #[test]
    fn a_prefix_argument_is_trimmed_lowercased_and_floored() {
        assert_eq!(needle("  ABcd ", MIN_PREFIX).as_deref(), Some("abcd"));
        // Below the floor, and the empty string that `starts_with` would match
        // against everything.
        assert_eq!(needle("abc", MIN_PREFIX), None);
        assert_eq!(needle("  a ", MIN_PREFIX), None);
        assert_eq!(needle("", MIN_PREFIX), None);
    }

    #[test]
    fn a_prefix_match_returns_every_candidate_not_the_first() {
        let peers = fleet();
        let all = by_prefix(&peers, "aa", |p| p.node_id.as_str());
        assert_eq!(all.len(), 1);
        assert!(by_prefix(&peers, &"aa".repeat(33), |p| p.node_id.as_str()).is_empty());
    }

    fn pending(node_id: &str, device: &str) -> PendingPair {
        PendingPair {
            node_id: node_id.to_string(),
            device: device.to_string(),
            fingerprint: vec!["acid".to_string(); 6],
            role: "host".to_string(),
            created_at: 1,
        }
    }

    #[test]
    fn resolve_pending_needs_no_argument_only_while_one_pairing_waits() {
        let one = vec![pending(&"aa".repeat(32), "Phone")];
        assert_eq!(resolve_pending(&one, None).unwrap().device, "Phone");

        let err = resolve_pending(&[], None).unwrap_err();
        assert!(err.contains("pair_device"), "{err}");

        let two = vec![pending(&"aa".repeat(32), "Phone"), pending(&"bb".repeat(32), "Laptop")];
        let err = resolve_pending(&two, None).unwrap_err();
        assert!(err.contains("Phone (aaaaaaaaaa)"), "{err}");
        assert!(err.contains("Laptop (bbbbbbbbbb)"), "{err}");
    }

    #[test]
    fn resolve_pending_never_guesses_between_two_that_share_a_prefix() {
        let parked = vec![
            pending(&"ab".repeat(32), "Phone"),
            pending(&format!("{}cdcd", "ab".repeat(30)), "Laptop"),
        ];
        // Too short to mean anything — including the empty string.
        assert!(resolve_pending(&parked, Some("")).unwrap_err().contains("at least"));
        assert!(resolve_pending(&parked, Some("  a ")).unwrap_err().contains("at least"));
        // Shared by both.
        assert!(resolve_pending(&parked, Some("abab")).unwrap_err().contains("Phone"));
        // Names exactly one.
        let long = format!("{}cd", "ab".repeat(30));
        assert_eq!(resolve_pending(&parked, Some(&long)).unwrap().device, "Laptop");
        // Names none.
        assert!(resolve_pending(&parked, Some("ffff")).unwrap_err().contains("no pairing"));
    }
}
