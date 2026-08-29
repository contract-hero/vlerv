// Turning "send it to my phone" into exactly one paired peer.
//
// The caller is a language model with a free-text name, so the match is
// deliberately forgiving — full node id, device name, a prefix of either — and
// deliberately refuses to guess: two candidates is an error that lists both,
// never the first one sorted. Sending a file to the wrong machine is not
// undoable.

use vlerv_remote::peers::Peer;

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
const MIN_PREFIX: usize = 4;

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
    let needle = query.trim().to_lowercase();

    if let Some(peer) = peers.iter().find(|p| p.node_id.eq_ignore_ascii_case(&needle)) {
        return Ok(peer.clone());
    }

    let exact_name: Vec<&Peer> =
        peers.iter().filter(|p| p.device.to_lowercase() == needle).collect();
    if !exact_name.is_empty() {
        return decide(query, exact_name);
    }

    let mut fuzzy: Vec<&Peer> = Vec::new();
    if needle.len() >= MIN_PREFIX {
        fuzzy.extend(peers.iter().filter(|p| p.node_id.to_lowercase().starts_with(&needle)));
    }
    for peer in peers.iter().filter(|p| p.device.to_lowercase().contains(&needle)) {
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

/// How a peer is named back to the caller: the device name plus a short node
/// id, which is what disambiguates two machines with the same name.
pub fn label(peer: &Peer) -> String {
    format!("{} ({})", peer.device, short_id(&peer.node_id))
}

/// First 8 characters of a node id. Node ids are hex, so slicing is
/// char-safe; the length guard covers a truncated entry read off disk.
pub fn short_id(node_id: &str) -> String {
    if node_id.len() <= 8 {
        node_id.to_string()
    } else {
        node_id[..8].to_string()
    }
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
        assert!(text.contains("Mac Studio (bbbbbbbb)"), "{text}");
        assert!(text.contains("MacBook Pro (cccccccc)"), "{text}");
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
        for expected in ["Val's iPhone (aaaaaaaa)", "Mac Studio (bbbbbbbb)", "MacBook Pro (cccccccc)"] {
            assert!(text.contains(expected), "{text} is missing {expected}");
        }
    }

    #[test]
    fn short_id_never_panics_on_a_truncated_entry() {
        assert_eq!(short_id(&"ab".repeat(32)), "abababab");
        assert_eq!(short_id("abc"), "abc");
        assert_eq!(short_id(""), "");
    }
}
