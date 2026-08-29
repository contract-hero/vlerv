// Scope wire protocol (design §6). Pure serde + postcard: no iroh types, no
// I/O — the framing helpers that touch a QUIC stream live in scope.rs, so
// every shape here is testable without an endpoint.
//
// Framing: one QUIC bi-stream per session carries length-prefixed postcard
// frames (u32 little-endian length, then the postcard body). The client
// writes `Req` frames; the server writes `Frame` frames, which multiplex
// request responses and subscription events on the same stream.
//
// Versioning: postcard identifies enum variants BY DECLARATION ORDER, so
// appending a variant is compatible and reordering is a break. `PROTO_VERSION`
// is the explicit break mechanism — `Hello` carries it and the server refuses
// a mismatch before answering anything else. The ALPN's trailing integer is
// the second, coarser one (design §4).

use serde::{de::DeserializeOwned, Deserialize, Serialize};

/// Session protocol ALPN. Bumping the trailing integer is the transport-level
/// protocol break; `PROTO_VERSION` is the fine-grained one.
pub const SCOPE_ALPN: &[u8] = b"vlerv/scope/0";

/// Pairing runs on its OWN ALPN. The scope server rejects every NodeId that
/// is not already in peers.json before it parses a byte (design §7), which is
/// exactly the check a first-time peer cannot pass — so the pairing handshake,
/// whose capability is the one-time token instead of the peer list, needs a
/// separate door.
pub const PAIR_ALPN: &[u8] = b"vlerv/pair/0";

/// Wire version carried in `Hello` / `PairHello`. A mismatch is refused.
pub const PROTO_VERSION: u32 = 1;

/// Frames carry metadata only — artifact bytes ride the iroh-blobs protocol.
/// The cap bounds what one hostile frame header can make us allocate.
pub const MAX_FRAME_BYTES: usize = 256 * 1024;

/// Longest device name accepted from the wire, in chars. Display-only text
/// from another machine: bounded and stripped like a beam name hint.
pub const MAX_DEVICE_CHARS: usize = 64;

// ── Requests (client → host) ────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Req {
    /// Always the first frame. Rejected on version mismatch.
    Hello { proto: u32, device: String },
    ListTabs,
    ListBookmarks,
    ListRecents,
    /// Browse scope and up.
    ListTree { path: String },
    /// Answers hash/size/mtime only; the bytes move over iroh-blobs.
    GetArtifact { path: String },
    /// Start pushing `Event` frames on this stream.
    Subscribe,
    /// Control scope only. Invokes the same intent a deep link could.
    OpenOnHost { path: String, reader_mode: bool },
    /// Stop pushing events. Appended last on purpose: postcard numbers enum
    /// variants by declaration order, so adding at the end is the compatible
    /// direction. Not in the design sketch — it exists so a client that
    /// closes its drawer stops the host from re-hashing files for nobody.
    Unsubscribe,
    /// Control scope only. The reverse direction of `GetArtifact`: the CLIENT
    /// offers bytes and the host pulls them. `ticket` is a blob ticket the
    /// client minted against ITS OWN endpoint, peer-locked — the host refuses
    /// it unless the ticket's NodeId is the peer that sent this frame, so a
    /// control peer cannot make the host fetch from a third machine.
    /// `name`/`size` are display hints, sanitized and re-derived from the
    /// actual stream exactly like a Beam receive; `hash` must equal the
    /// ticket's hash, which is the only content truth.
    ///
    /// Appended after `Unsubscribe`: postcard numbers variants by declaration
    /// order, so an older client and a newer host still agree on every
    /// variant that existed before this one.
    PushArtifact { name: String, size: u64, hash: String, ticket: String },
}

// ── Responses and events (host → client) ────────────────────────────────────

/// One frame written by the host. Responses arrive in request order; events
/// may be interleaved between them once the session subscribed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Frame {
    Res(Res),
    Event(Event),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Res {
    Hello(HelloAck),
    Tabs(Vec<TabEntry>),
    Paths(Vec<PathEntry>),
    Tree(Vec<TreeEntry>),
    Artifact(ArtifactMeta),
    Subscribed,
    Opened,
    /// Every refusal — scope, path, size — with the no-existence-leak wording
    /// of the share module. One variant so the client cannot tell a missing
    /// file from a forbidden one.
    Denied(String),
    /// A pushed artifact landed, verified, under the host's `received/`.
    /// Carries the host-side name because collision handling may have renamed
    /// it. Appended last for the same postcard reason as `Req::PushArtifact`.
    Pushed { name: String, size: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloAck {
    pub proto: u32,
    pub device: String,
    /// The scope this host granted the calling peer, as its wire string
    /// ("view-open" / "browse" / "control"). The client greys out affordances
    /// it may not use instead of discovering the refusal request by request.
    pub scope: String,
}

/// One open tab on the host, in strip order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabEntry {
    pub path: String,
    pub active: bool,
}

/// A bookmark or recent entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathEntry {
    pub path: String,
    pub name: String,
}

/// One child of a listed directory. Same shape as the local Explorer row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

/// The answer to `GetArtifact`: a content address plus display facts. The
/// bytes are fetched by `hash` over iroh-blobs, verified there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactMeta {
    /// BLAKE3 hash, hex.
    pub hash: String,
    pub size: u64,
    /// Unix seconds; 0 when the host cannot read a modification time.
    pub mtime: u64,
    /// The host crossed its soft size threshold. The host owns the limits;
    /// the client never mirrors the constant.
    pub warn: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Event {
    TabOpened { path: String },
    TabClosed { path: String },
    TabActivated { path: String },
    /// Live reload across the wire: the file was re-hashed, so the event
    /// carries the NEW content address the client refetches by.
    FileChanged { path: String, hash: String },
    FileRemoved { path: String },
}

// ── Pairing handshake (its own ALPN, its own tiny protocol) ─────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairHello {
    pub proto: u32,
    /// The one-time token from the pairing ticket. Possession of a live token
    /// is what admits an unknown NodeId here.
    pub token: [u8; 32],
    pub device: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PairAck {
    Ok { proto: u32, device: String },
    Denied(String),
}

// ── Framing ────────────────────────────────────────────────────────────────

/// Encode one frame: u32-LE length prefix + postcard body.
pub fn encode_frame<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    let body = postcard::to_stdvec(value).map_err(|e| format!("cannot encode frame: {e}"))?;
    if body.len() > MAX_FRAME_BYTES {
        return Err("frame exceeds the protocol size cap".to_string());
    }
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// Decode one frame body (the bytes AFTER the length prefix).
pub fn decode_frame<T: DeserializeOwned>(body: &[u8]) -> Result<T, String> {
    postcard::from_bytes(body).map_err(|e| format!("cannot decode frame: {e}"))
}

/// Read a length prefix and validate it against the cap. Returns the body
/// length the caller must read next.
pub fn frame_len(prefix: [u8; 4]) -> Result<usize, String> {
    let len = u32::from_le_bytes(prefix) as usize;
    if len == 0 || len > MAX_FRAME_BYTES {
        return Err("frame exceeds the protocol size cap".to_string());
    }
    Ok(len)
}

/// Reduce an attacker-controlled device name to safe display text: no control
/// characters, no bidi/zero-width spoofing, bounded length. Same distrust the
/// beam name hint gets — this string lands in the peer list and the drawer
/// header.
pub fn sanitize_device(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(c,
                    '\u{200B}'..='\u{200F}'
                    | '\u{202A}'..='\u{202E}'
                    | '\u{2066}'..='\u{2069}'
                    | '\u{FEFF}')
        })
        .take(MAX_DEVICE_CHARS)
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "unknown device".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn req_round_trips_through_postcard() {
        let cases = vec![
            Req::Hello { proto: PROTO_VERSION, device: "Mac Studio".into() },
            Req::ListTabs,
            Req::ListBookmarks,
            Req::ListRecents,
            Req::ListTree { path: "/w/project".into() },
            Req::GetArtifact { path: "/w/project/report.html".into() },
            Req::Subscribe,
            Req::OpenOnHost { path: "/w/project/report.html".into(), reader_mode: true },
            Req::Unsubscribe,
            Req::PushArtifact {
                name: "report.html".into(),
                size: 482_133,
                hash: "ef".repeat(32),
                ticket: "blobabcdef".into(),
            },
        ];
        for req in cases {
            let bytes = encode_frame(&req).expect("encode");
            let len = frame_len(bytes[..4].try_into().unwrap()).expect("length prefix");
            assert_eq!(len, bytes.len() - 4);
            let back: Req = decode_frame(&bytes[4..]).expect("decode");
            assert_eq!(back, req);
        }
    }

    #[test]
    fn appending_push_kept_every_earlier_variant_at_its_own_wire_number() {
        // postcard numbers variants BY DECLARATION ORDER, so appending is the
        // compatible direction only while nothing ahead of the new variant
        // moved. These are the numbers a peer built before `PushArtifact`
        // already speaks; a reorder would silently turn its `Subscribe` into
        // someone else's `GetArtifact`.
        let reqs: Vec<(Req, u8)> = vec![
            (Req::Hello { proto: PROTO_VERSION, device: String::new() }, 0),
            (Req::ListTabs, 1),
            (Req::ListBookmarks, 2),
            (Req::ListRecents, 3),
            (Req::ListTree { path: String::new() }, 4),
            (Req::GetArtifact { path: String::new() }, 5),
            (Req::Subscribe, 6),
            (Req::OpenOnHost { path: String::new(), reader_mode: false }, 7),
            (Req::Unsubscribe, 8),
            (
                Req::PushArtifact {
                    name: String::new(),
                    size: 0,
                    hash: String::new(),
                    ticket: String::new(),
                },
                9,
            ),
        ];
        for (req, tag) in reqs {
            let body = postcard::to_stdvec(&req).unwrap();
            assert_eq!(body[0], tag, "{req:?} moved on the wire");
        }

        let resps: Vec<(Res, u8)> = vec![
            (Res::Hello(HelloAck { proto: 1, device: String::new(), scope: String::new() }), 0),
            (Res::Tabs(Vec::new()), 1),
            (Res::Paths(Vec::new()), 2),
            (Res::Tree(Vec::new()), 3),
            (
                Res::Artifact(ArtifactMeta {
                    hash: String::new(),
                    size: 0,
                    mtime: 0,
                    warn: false,
                }),
                4,
            ),
            (Res::Subscribed, 5),
            (Res::Opened, 6),
            (Res::Denied(String::new()), 7),
            (Res::Pushed { name: String::new(), size: 0 }, 8),
        ];
        for (res, tag) in resps {
            let body = postcard::to_stdvec(&res).unwrap();
            assert_eq!(body[0], tag, "{res:?} moved on the wire");
        }
    }

    #[test]
    fn frame_round_trips_responses_and_events() {
        let frames = vec![
            Frame::Res(Res::Hello(HelloAck {
                proto: PROTO_VERSION,
                device: "MacBook".into(),
                scope: "browse".into(),
            })),
            Frame::Res(Res::Tabs(vec![TabEntry { path: "/a.html".into(), active: true }])),
            Frame::Res(Res::Paths(vec![PathEntry {
                path: "/a.html".into(),
                name: "a.html".into(),
            }])),
            Frame::Res(Res::Tree(vec![TreeEntry {
                name: "sub".into(),
                path: "/w/sub".into(),
                is_dir: true,
            }])),
            Frame::Res(Res::Artifact(ArtifactMeta {
                hash: "ab".repeat(32),
                size: 42,
                mtime: 1_786_752_000,
                warn: false,
            })),
            Frame::Res(Res::Subscribed),
            Frame::Res(Res::Opened),
            Frame::Res(Res::Denied("path not found or out of root".into())),
            Frame::Res(Res::Pushed { name: "report.html".into(), size: 482_133 }),
            Frame::Event(Event::TabOpened { path: "/a.html".into() }),
            Frame::Event(Event::TabClosed { path: "/a.html".into() }),
            Frame::Event(Event::TabActivated { path: "/a.html".into() }),
            Frame::Event(Event::FileChanged {
                path: "/a.html".into(),
                hash: "cd".repeat(32),
            }),
            Frame::Event(Event::FileRemoved { path: "/a.html".into() }),
        ];
        for frame in frames {
            let bytes = encode_frame(&frame).expect("encode");
            let back: Frame = decode_frame(&bytes[4..]).expect("decode");
            assert_eq!(back, frame);
        }
    }

    #[test]
    fn a_hello_from_another_protocol_version_is_distinguishable() {
        // The version rejection the server performs: same bytes decode, the
        // carried number is what refuses the session.
        let future = Req::Hello { proto: PROTO_VERSION + 1, device: "x".into() };
        let bytes = encode_frame(&future).unwrap();
        match decode_frame::<Req>(&bytes[4..]).unwrap() {
            Req::Hello { proto, .. } => assert_ne!(proto, PROTO_VERSION),
            other => panic!("expected Hello, got {other:?}"),
        }
    }

    #[test]
    fn oversized_and_zero_length_prefixes_are_refused() {
        assert!(frame_len((MAX_FRAME_BYTES as u32 + 1).to_le_bytes()).is_err());
        assert!(frame_len(u32::MAX.to_le_bytes()).is_err());
        assert!(frame_len(0u32.to_le_bytes()).is_err());
        assert!(frame_len(1u32.to_le_bytes()).is_ok());
    }

    #[test]
    fn garbage_bodies_fail_to_decode_instead_of_panicking() {
        assert!(decode_frame::<Req>(&[0xff, 0xff, 0xff]).is_err());
        assert!(decode_frame::<Frame>(&[]).is_err());
    }

    #[test]
    fn oversized_payloads_are_refused_at_encode_time() {
        let huge = Req::GetArtifact { path: "x".repeat(MAX_FRAME_BYTES + 1) };
        assert!(encode_frame(&huge).is_err());
    }

    #[test]
    fn device_names_are_sanitized_like_beam_name_hints() {
        assert_eq!(sanitize_device("Mac Studio"), "Mac Studio");
        assert_eq!(sanitize_device("Mac\u{202E}Studio"), "MacStudio");
        assert_eq!(sanitize_device("a\0b"), "ab");
        assert_eq!(sanitize_device("   "), "unknown device");
        assert_eq!(sanitize_device(&"x".repeat(500)).chars().count(), MAX_DEVICE_CHARS);
    }
}
