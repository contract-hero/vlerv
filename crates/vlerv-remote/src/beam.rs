// Beam — stage / serve / fetch one artifact (design §5).
//
// Serving side: files are staged into the blob store (ImportMode::Copy — the
// ticket pins the content at mint time) and offered under a ticket. The
// BlobsProtocol request gate consults the offers registry per request, so
// Stop and TTL expiry revoke instantly regardless of what the store holds.
//
// Receiving side: the blob streams in BLAKE3-verified (a corrupted or
// substituted stream fails before a byte can render) and lands under the
// app's own state dir, never in the user's tree.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bao_tree::io::BaoContentItem;
use iroh_blobs::api::Tag;
use iroh_blobs::get::request::{get_blob, GetBlobItem};
use iroh_blobs::provider::events::{
    AbortReason, ConnectMode, EventMask, EventSender, ProviderMessage, RequestMode,
};
use iroh_blobs::store::fs::FsStore;
use iroh_blobs::ticket::BlobTicket;
use iroh_blobs::{BlobFormat, Hash};
use n0_future::StreamExt;
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};

use crate::endpoint::{self, RemoteNode};
use crate::paths::{base_name, mtime_secs};
use crate::peers::now_unix;
use crate::proto;
use crate::security::{self, RootSet};

/// Soft warn threshold (receive-dialog copy) and hard cap, per design §5.
pub const WARN_BYTES: u64 = 20 * 1024 * 1024;
pub const HARD_CAP_BYTES: u64 = 256 * 1024 * 1024;

/// Offer lifetime, and the range any caller-supplied lifetime is clamped into.
/// The bounds are the policy, not a validation detail: a `0` from a stale
/// state.json must not mint a dead ticket, and a `u32::MAX` from an MCP client
/// must not mint a ~490,000-year fetch grant. Consumers that validate a
/// ttl_hours argument reject against the SAME two numbers.
pub const DEFAULT_TTL_HOURS: u32 = 24;
pub const MIN_TTL_HOURS: u32 = 1;
pub const MAX_TTL_HOURS: u32 = 24 * 30;

/// Longest beam name kept from a link's name hint, in chars. Same bounding
/// idea as `proto::MAX_DEVICE_CHARS`, on a string that also becomes a filename.
const MAX_NAME_CHARS: usize = 128;

/// Emit a progress event roughly once per this many received bytes — leaves
/// arrive every 16 KiB and per-leaf events would flood the webview bridge.
const PROGRESS_STRIDE_BYTES: u64 = 1024 * 1024;

/// Percent-encode everything outside RFC 3986 unreserved chars, same policy
/// as the CLI's PATH_SET (name travels as a query value in the beam link).
const QUERY_VALUE_SET: &AsciiSet = &CONTROLS
    .add(b' ').add(b'!').add(b'"').add(b'#').add(b'$').add(b'%').add(b'&')
    .add(b'\'').add(b'(').add(b')').add(b'*').add(b'+').add(b',').add(b'/')
    .add(b':').add(b';').add(b'<').add(b'=').add(b'>').add(b'?').add(b'@')
    .add(b'[').add(b'\\').add(b']').add(b'^').add(b'`').add(b'{').add(b'|')
    .add(b'}');

/// One active offer, as surfaced to the webview and the offers indicator.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OfferInfo {
    /// Offer id == the blob's BLAKE3 hash (hex). One offer per content.
    pub id: String,
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub ticket: String,
    /// The full `vlerv://receive?…` deep link the user shares.
    pub link: String,
    pub created_at: u64,
    pub expires_at: u64,
    pub fetches: u64,
}

/// Payload for `vlerv://beam-progress`.
/// `received` is real measured bytes. No `total`: the only size we have is
/// the sender's unverified hint, and the dialog already holds it — echoing
/// it here beside real bytes only invites a bogus `received / total`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProgressEvent {
    pub hash: String,
    pub received: u64,
}

/// A completed receive, returned by `beam_receive`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ReceivedFile {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub hash: String,
}

/// One past beam in the `received/` tree, for the Received list.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ReceivedEntry {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub received_at: u64,
}

/// Sender-side ticket facts shown in the receiver's confirm dialog. Pure
/// parse — no endpoint, no network.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TicketInfo {
    pub node_id: String,
    pub node_id_short: String,
    pub hash: String,
}

struct OfferEntry {
    info: OfferInfo,
    tag: Tag,
}

/// The offers registry. The request gate reads it per incoming request;
/// commands mutate it. Lock discipline: short critical sections only, never
/// held across an await.
#[derive(Default)]
pub struct Offers {
    inner: Mutex<HashMap<Hash, OfferEntry>>,
}

impl Offers {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an offer, returning the store tag of any entry it displaced
    /// (same hash, re-beamed) so the caller can delete it — the displaced tag
    /// is a fresh `add_path` tag that would otherwise pin the blob forever,
    /// unreachable by `stop`/`take_expired`.
    fn insert(&self, hash: Hash, entry: OfferEntry) -> Option<Tag> {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(hash, entry)
            .map(|old| old.tag)
    }

    fn remove(&self, id: &str) -> Option<Tag> {
        // An offer id IS the blob hash in hex (see `offer`), so the key is
        // derivable — no scan. The length guard keeps the parse on the hex
        // path: `Hash::from_str` treats other lengths as base32 and can
        // PANIC on malformed input, and this id arrives over IPC.
        if id.len() != 64 {
            return None;
        }
        let hash: Hash = id.parse().ok()?;
        let mut map = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        map.remove(&hash).map(|e| e.tag)
    }

    /// Drop every expired offer, returning their store tags for cleanup.
    fn take_expired(&self, now: u64) -> Vec<Tag> {
        let mut map = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let expired: Vec<Hash> = map
            .iter()
            .filter(|(_, e)| e.info.expires_at <= now)
            .map(|(h, _)| *h)
            .collect();
        expired.into_iter().filter_map(|h| map.remove(&h)).map(|e| e.tag).collect()
    }

    /// Active offers, newest first. Expired entries are filtered (their tag
    /// cleanup happens on the next mutating command via `take_expired`).
    pub fn list(&self) -> Vec<OfferInfo> {
        let now = now_unix();
        let map = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let mut infos: Vec<OfferInfo> = map
            .values()
            .filter(|e| e.info.expires_at > now)
            .map(|e| e.info.clone())
            .collect();
        infos.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        infos
    }

    /// Check + count one incoming request. `Ok` only for a plain full-blob
    /// request whose hash is an active, unexpired offer.
    fn admit(&self, hash: &Hash, is_blob_request: bool) -> Result<(), AbortReason> {
        if !is_blob_request {
            return Err(AbortReason::Permission);
        }
        let mut map = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        match map.get_mut(hash) {
            Some(entry) if entry.info.expires_at > now_unix() => {
                entry.info.fetches += 1;
                Ok(())
            }
            // Expired or never offered: same refusal, no distinction leaked.
            _ => Err(AbortReason::Permission),
        }
    }

    /// Build the provider-event gate for BlobsProtocol. Beam v1 serves
    /// exactly one request shape — a plain full-blob GET whose hash is an
    /// active, unexpired offer. Everything else is refused *explicitly* here.
    ///
    /// In iroh-blobs 0.103 the provider's generic `request<Req>` consults
    /// `mask.get` for every request kind, so `get: Intercept` routes get /
    /// get-many / push / observe all through this loop; the deny arms below
    /// are what refuse the non-GET kinds, not the drop of an unmatched
    /// message. The `Disabled` mask fields are belt-and-suspenders: should a
    /// future (pinned, deliberate) upgrade start honoring per-kind masks,
    /// those kinds are rejected before they ever reach this loop. This is
    /// what makes Stop an *instant* revocation instead of a GC eventually.
    /// `on_change` receives the fresh offers list (fetch counts moved).
    /// `grants` is the v2 half of the same gate: Scope's `GetArtifact` stages
    /// an artifact and grants ONE peer a fetch of that hash, so the gate also
    /// needs to know which NodeId is asking. `connected: Notify` is what
    /// supplies that — the provider reports the endpoint id once per
    /// connection, and this loop keeps the connection→peer map for the life
    /// of the connection.
    pub fn gate(
        self: Arc<Self>,
        grants: Arc<crate::scope::Grants>,
        on_change: impl Fn(Vec<OfferInfo>) + Send + Sync + 'static,
    ) -> EventSender {
        let mask = EventMask {
            connected: ConnectMode::Notify,
            get: RequestMode::Intercept,
            get_many: RequestMode::Disabled,
            push: RequestMode::Disabled,
            ..EventMask::DEFAULT
        };
        let (tx, mut rx) = EventSender::channel(32, mask);
        tokio::spawn(async move {
            let mut conn_peers: HashMap<u64, iroh::EndpointId> = HashMap::new();
            while let Some(msg) = rx.recv().await {
                match msg {
                    ProviderMessage::ClientConnectedNotify(msg) => {
                        if let Some(id) = msg.endpoint_id {
                            conn_peers.insert(msg.connection_id, id);
                        }
                    }
                    ProviderMessage::ConnectionClosed(msg) => {
                        conn_peers.remove(&msg.connection_id);
                    }
                    ProviderMessage::GetRequestReceived(msg) => {
                        let is_blob = msg.request.ranges.is_blob();
                        let hash = msg.request.hash;
                        // A beam offer is a capability held by whoever has the
                        // ticket; a scope grant is peer-locked. Either admits.
                        let res = match self.admit(&hash, is_blob) {
                            Ok(()) => Ok(()),
                            Err(denied) => {
                                let peer = conn_peers.get(&msg.connection_id).copied();
                                if grants.admit(&hash, peer, is_blob) {
                                    Ok(())
                                } else {
                                    Err(denied)
                                }
                            }
                        };
                        let admitted = res.is_ok();
                        msg.tx.send(res).await.ok();
                        if admitted {
                            on_change(self.list());
                        }
                    }
                    ProviderMessage::GetManyRequestReceived(msg) => {
                        msg.tx.send(Err(AbortReason::Permission)).await.ok();
                    }
                    ProviderMessage::PushRequestReceived(msg) => {
                        msg.tx.send(Err(AbortReason::Permission)).await.ok();
                    }
                    ProviderMessage::ObserveRequestReceived(msg) => {
                        msg.tx.send(Err(AbortReason::Permission)).await.ok();
                    }
                    _ => {}
                }
            }
        });
        tx
    }
}

/// v1 ticket policy in one place: parse + single-raw-blob check. Shared by
/// `ticket_info` (dispatch-time validation) and `receive`.
fn parse_raw_ticket(ticket_str: &str) -> Result<BlobTicket, String> {
    let ticket: BlobTicket = ticket_str
        .parse()
        .map_err(|e| format!("invalid beam ticket: {e}"))?;
    if ticket.format() != BlobFormat::Raw {
        return Err("beam v1 supports single-file tickets only".to_string());
    }
    Ok(ticket)
}

/// Build the `vlerv://receive?…` link. `name`/`size` are display hints for
/// the receiver's confirm dialog — the ticket's hash is the truth.
pub fn build_link(ticket: &str, name: &str, size: u64) -> String {
    let name_enc = utf8_percent_encode(name, QUERY_VALUE_SET);
    format!("vlerv://receive?ticket={ticket}&name={name_enc}&size={size}")
}

/// Parse a ticket string into the facts the confirm dialog shows. Pure
/// parse — no endpoint, no network.
pub fn ticket_info(ticket_str: &str) -> Result<TicketInfo, String> {
    let ticket = parse_raw_ticket(ticket_str)?;
    let id = ticket.addr().id;
    Ok(TicketInfo {
        node_id: id.to_string(),
        node_id_short: id.fmt_short().to_string(),
        hash: ticket.hash().to_string(),
    })
}

/// The push-side ticket policy (Scope v2 `Req::PushArtifact`), in one place
/// beside the receive-side one it mirrors. A pushed ticket is NOT a beam
/// ticket: a beam ticket is a capability anyone holding it may use, while a
/// push tells the host to DIAL somebody, so the host admits it only when
///
///   * it is a single raw blob (same as beam v1),
///   * its NodeId is the peer that sent the frame — peer-locked, so a control
///     peer cannot make the host fetch from a third machine, and
///   * its hash equals the announced one, so the display metadata the host
///     shows describes the bytes it is about to pull.
///
/// Pure parse — no endpoint, no network. Returns the verified content address.
pub fn verify_push_ticket(
    ticket_str: &str,
    from: iroh::EndpointId,
    claimed_hash: &str,
) -> Result<Hash, String> {
    let ticket = parse_raw_ticket(ticket_str)?;
    if ticket.addr().id != from {
        return Err("a pushed ticket must name the pushing peer".to_string());
    }
    if ticket.hash().to_string() != claimed_hash {
        return Err("the pushed ticket does not match the announced content".to_string());
    }
    Ok(ticket.hash())
}

/// A path that passed the full offer policy: share-module root gate,
/// regular file, size under the hard cap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferCandidate {
    pub canonical: PathBuf,
    pub name: String,
    pub size: u64,
}

/// The single offer-path policy, shared by the `vlerv://beam` dispatch arm
/// (dialog metadata) and `beam_offer` (confirm-time recheck): resolve via
/// the conservative share gate, require a regular file, enforce the hard
/// cap. Keeping it in one place is what keeps the two callers in agreement
/// — the cap especially must reject at dispatch, not after the user clicks.
pub fn resolve_offerable(path: &Path, roots: &RootSet) -> Result<OfferCandidate, String> {
    let (canonical, _out_of_root) = security::canonicalize_allow_external(path, roots)
        // Same no-existence-leak wording as the share module.
        .map_err(|_| "path not found or out of root".to_string())?;
    let meta = std::fs::metadata(&canonical)
        .map_err(|_| "path not found or out of root".to_string())?;
    if !meta.is_file() {
        return Err("only files can be beamed".to_string());
    }
    let size = meta.len();
    if size > HARD_CAP_BYTES {
        return Err(format!(
            "file is {} — beam v1 caps at {}",
            human_bytes(size),
            human_bytes(HARD_CAP_BYTES)
        ));
    }
    let name = base_name(&canonical).unwrap_or_else(|| "artifact".to_string());
    Ok(OfferCandidate { canonical, name, size })
}

/// Stage a resolved candidate into the store and register an offer.
/// Re-beaming the same content refreshes the offer (same hash ⇒ same id)
/// with a fresh TTL.
pub async fn offer(
    node: &RemoteNode,
    cand: &OfferCandidate,
    ttl_hours: u32,
) -> Result<OfferInfo, String> {
    // Housekeeping: drop expired offers and their tags while we have the
    // store at hand.
    delete_tags(&node.store, node.offers.take_expired(now_unix())).await;

    let tag = node
        .store
        .blobs()
        .add_path(&cand.canonical)
        .await
        .map_err(|e| format!("cannot stage file: {e}"))?;

    // Wait (bounded) for relay + discovery so the ticket dials from other
    // networks; on timeout the ticket still carries direct addresses, which
    // covers the same-LAN case.
    let _ = tokio::time::timeout(Duration::from_secs(10), node.endpoint.online()).await;

    let ticket = BlobTicket::new(node.endpoint.addr(), tag.hash, tag.format).to_string();
    let created_at = now_unix();
    let info = OfferInfo {
        id: tag.hash.to_string(),
        path: cand.canonical.clone(),
        name: cand.name.clone(),
        size: cand.size,
        link: build_link(&ticket, &cand.name, cand.size),
        ticket,
        created_at,
        // Clamped both ends — see MIN_TTL_HOURS / MAX_TTL_HOURS.
        expires_at: created_at
            + u64::from(ttl_hours.clamp(MIN_TTL_HOURS, MAX_TTL_HOURS)) * 3600,
        fetches: 0,
    };
    if let Some(old_tag) = node.offers.insert(tag.hash, OfferEntry { info: info.clone(), tag: tag.name }) {
        // Re-beam of the same content: drop the previous staging tag so its
        // copy of the bytes becomes collectable.
        delete_tags(&node.store, vec![old_tag]).await;
    }
    Ok(info)
}

/// Revoke an offer. The gate stops admitting the hash immediately; the store
/// tag is deleted so the staged bytes become garbage-collectable.
pub async fn stop(node: &RemoteNode, offer_id: &str) {
    if let Some(tag) = node.offers.remove(offer_id) {
        delete_tags(&node.store, vec![tag]).await;
    }
    delete_tags(&node.store, node.offers.take_expired(now_unix())).await;
}

/// Delete staging tags, logging failures rather than discarding them — a
/// failed delete leaves a private copy of somebody's file pinned on disk after
/// the offer or grant that justified it is gone, and that should be
/// diagnosable. The one tag sweeper: Beam's offers and Scope's grants both
/// unpin bytes through it, on either side of a push.
pub(crate) async fn delete_tags(store: &FsStore, tags: Vec<Tag>) {
    for tag in tags {
        if let Err(e) = store.tags().delete(tag.clone()).await {
            eprintln!("vlerv: remote: could not delete blob tag {tag:?}: {e}");
        }
    }
}

/// Stream one blob into `file`, BLAKE3-verified chunk by chunk, enforcing the
/// hard size cap on the ACTUAL bytes (never on a claimed size). Shared by the
/// Beam receive path and Scope's content-addressed cache fetch — one loop, so
/// the cap and the contiguity invariant cannot drift apart.
pub(crate) async fn stream_blob_into(
    connection: iroh::endpoint::Connection,
    hash: Hash,
    file: &mut std::io::BufWriter<std::fs::File>,
    hash_hex: &str,
    on_progress: &mut impl FnMut(&str, u64),
) -> Result<u64, String> {
    let mut written: u64 = 0;
    let mut last_emitted: u64 = 0;
    // The bao stream yields leaves in ascending offset order for a full-blob
    // request; the offset check guards that invariant.
    let mut progress = get_blob(connection, hash);
    loop {
        match progress.next().await {
            Some(GetBlobItem::Item(BaoContentItem::Leaf(leaf))) => {
                if leaf.offset != written {
                    return Err("transfer aborted: non-contiguous stream".to_string());
                }
                written += leaf.data.len() as u64;
                if written > HARD_CAP_BYTES {
                    return Err(format!(
                        "transfer aborted: content exceeds the {} cap",
                        human_bytes(HARD_CAP_BYTES)
                    ));
                }
                file.write_all(&leaf.data)
                    .map_err(|e| format!("cannot write the incoming file: {e}"))?;
                if written - last_emitted >= PROGRESS_STRIDE_BYTES {
                    last_emitted = written;
                    on_progress(hash_hex, written);
                }
            }
            Some(GetBlobItem::Item(BaoContentItem::Parent(_))) => {}
            Some(GetBlobItem::Done(_stats)) => break,
            Some(GetBlobItem::Error(e)) => return Err(format!("transfer failed: {e}")),
            None => return Err("transfer failed: stream ended unexpectedly".to_string()),
        }
    }
    Ok(written)
}

/// Serial number for in-flight downloads — see `partial_name`.
static PARTIAL_SEQ: AtomicU64 = AtomicU64::new(0);

/// Temp file name for one in-flight download. Unique per CALL, not per content
/// address: two receives of the same blob (two tabs on one remote artifact, or
/// a live-reload refetch overlapping the first read) run concurrently, and a
/// shared `<hash><ext>.partial` would let them interleave writes into one file
/// and make the loser's rename fail. Both still land on the same final path,
/// which is atomic and idempotent — the bytes are identical, the hash says so.
///
/// The ONE partial-name convention: the Beam receive path and the Scope cache
/// fetch both name their temp file here, so `.partial` means the same thing in
/// `received/` and in `remote/cache/`.
pub(crate) fn partial_name(hash_hex: &str, ext: &str) -> String {
    format!(
        "{hash_hex}{ext}.{}.{}.partial",
        std::process::id(),
        PARTIAL_SEQ.fetch_add(1, Ordering::SeqCst)
    )
}

/// Stream one blob into `partial`, flush it, and hand it to `finish`, which
/// moves it onto its final path and returns that path.
///
/// The point of the shape is the SINGLE cleanup: a stream error, a failed
/// flush and a failed rename all funnel through one `remove_file`, so a
/// half-written — or even a fully downloaded — blob is never orphaned in a
/// `.partial` name nothing prunes. A per-arm cleanup ritual had already
/// drifted once here.
///
/// `finish` runs AFTER the bytes land because the Beam receive path only knows
/// its final name by then: the day directory comes from the current date and
/// the collision suffix from what is free at that moment. It owns the rename
/// so each caller keeps its own wording for a failed move.
pub(crate) async fn download_to(
    connection: iroh::endpoint::Connection,
    hash: Hash,
    hash_hex: &str,
    partial: &Path,
    finish: impl FnOnce(&Path) -> Result<PathBuf, String>,
    on_progress: &mut impl FnMut(&str, u64),
) -> Result<(u64, PathBuf), String> {
    let result = async {
        let mut file = std::io::BufWriter::new(
            std::fs::File::create(partial)
                .map_err(|e| format!("cannot open the incoming file {}: {e}", partial.display()))?,
        );
        let written = stream_blob_into(connection, hash, &mut file, hash_hex, on_progress).await?;
        file.into_inner()
            .map_err(|e| format!("cannot flush the incoming file: {e}"))?
            .sync_all()
            .map_err(|e| format!("cannot flush the incoming file: {e}"))?;
        Ok::<(u64, PathBuf), String>((written, finish(partial)?))
    }
    .await;

    if result.is_err() {
        let _ = std::fs::remove_file(partial);
    }
    result
}

/// Dial a ticket and stream the blob into `received_root`, verified chunk by
/// chunk. `on_progress(hash_hex, received)` fires about once per MiB with the
/// real measured byte count.
pub async fn receive(
    node: &RemoteNode,
    ticket_str: &str,
    name_hint: Option<&str>,
    received_root: &Path,
    on_progress: impl FnMut(&str, u64),
) -> Result<ReceivedFile, String> {
    receive_via(&node.endpoint, ticket_str, name_hint, received_root, on_progress).await
}

/// `receive` against a bare endpoint. The Scope host uses this to pull a
/// pushed artifact: it holds an endpoint and the peer's ticket, not a whole
/// node, and a push must land through EXACTLY this path — same verification,
/// same cap, same `received/` folder, same collision naming.
pub async fn receive_via(
    endpoint: &iroh::Endpoint,
    ticket_str: &str,
    name_hint: Option<&str>,
    received_root: &Path,
    mut on_progress: impl FnMut(&str, u64),
) -> Result<ReceivedFile, String> {
    let ticket = parse_raw_ticket(ticket_str)?;
    let hash = ticket.hash();
    // Rendered once — the progress callback fires per MiB and the hex form
    // never changes during a transfer.
    let hash_hex = hash.to_string();

    let connection = endpoint::dial(
        endpoint,
        ticket.addr().clone(),
        iroh_blobs::ALPN,
        "sender offline — could not reach the sender",
    )
    .await?;

    // Stream into a partial file next to the final location (same volume ⇒
    // atomic rename). Errors carry the operation + path, matching the
    // module's own "cannot …: {e}" style, so the receive dialog never shows
    // a bare "No such file or directory (os error 2)".
    let partials_dir = received_root.join(".partial");
    std::fs::create_dir_all(&partials_dir)
        .map_err(|e| format!("cannot prepare the received folder {}: {e}", partials_dir.display()))?;
    let partial_path = partials_dir.join(partial_name(&hash_hex, ""));

    // The final name is only knowable once the bytes are down: it depends on
    // today's date and on which collision suffix is free right then.
    let name = sanitize_beam_name(name_hint, &hash_hex);
    let (written, target) = download_to(
        connection,
        hash,
        &hash_hex,
        &partial_path,
        |partial| {
            let day_dir = received_root.join(civil_date_string(now_unix()));
            std::fs::create_dir_all(&day_dir)
                .map_err(|e| format!("cannot create {}: {e}", day_dir.display()))?;
            let target = unique_target_path(&day_dir, &name);
            std::fs::rename(partial, &target)
                .map_err(|e| format!("cannot move the received file into place: {e}"))?;
            Ok(target)
        },
        &mut on_progress,
    )
    .await?;
    on_progress(&hash_hex, written);

    Ok(ReceivedFile { path: target, name, size: written, hash: hash_hex })
}

/// Reduce an attacker-controlled name hint to a safe bare filename. The hint
/// travels in the deep link, so it gets the same distrust as any URL input:
/// no path components, no control chars, no dotfiles, bounded length.
pub fn sanitize_beam_name(hint: Option<&str>, hash_hex: &str) -> String {
    let fallback = || format!("beam-{}", &hash_hex[..8.min(hash_hex.len())]);
    let Some(hint) = hint else { return fallback() };

    // Last path segment only — either separator convention. Everything after
    // it is the shared strip set (the hint is both the display string in the
    // confirm dialog and the on-disk filename, so a U+202E extension-spoof is
    // the whole risk) plus this caller's own rule: no leading dot, because a
    // hint must never land a hidden file.
    let base = hint.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned = proto::strip_spoofing_chars(base, MAX_NAME_CHARS);
    let trimmed = cleaned.trim().trim_start_matches('.').trim();
    if trimmed.is_empty() {
        return fallback();
    }
    trimmed.to_string()
}

/// First free path for `name` in `dir`: `report.html`, `report-2.html`, …
fn unique_target_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), String::new()),
    };
    for n in 2.. {
        let candidate = dir.join(format!("{stem}-{n}{ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!("counter exhausted");
}

/// Past beams, newest first, capped at 100 entries. Day directories are
/// named `YYYY-MM-DD`, so lexicographic order IS chronological — walking
/// newest-day-first and stopping at the cap bounds the stat count no matter
/// how large the received/ tree grows.
pub fn list_received(received_root: &Path) -> Vec<ReceivedEntry> {
    const CAP: usize = 100;
    let mut entries = Vec::new();
    let Ok(days) = std::fs::read_dir(received_root) else { return entries };
    let mut day_dirs: Vec<PathBuf> = days
        .flatten()
        .filter(|d| !d.file_name().to_string_lossy().starts_with('.')) // .partial
        .map(|d| d.path())
        .collect();
    day_dirs.sort_unstable();
    for day in day_dirs.into_iter().rev() {
        let Ok(files) = std::fs::read_dir(day) else { continue };
        let mut day_entries = Vec::new();
        for f in files.flatten() {
            let Ok(meta) = f.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }
            let path = f.path();
            day_entries.push(ReceivedEntry {
                name: base_name(&path).unwrap_or_default(),
                path,
                size: meta.len(),
                received_at: mtime_secs(&meta),
            });
        }
        day_entries.sort_by(|a, b| b.received_at.cmp(&a.received_at));
        entries.extend(day_entries);
        if entries.len() >= CAP {
            break;
        }
    }
    entries.truncate(CAP);
    entries
}

/// `2026-08-15` from a unix timestamp (days-from-epoch → civil, Hinnant's
/// algorithm) — avoids a chrono/time dependency for one directory name.
pub fn civil_date_string(unix_secs: u64) -> String {
    let z = (unix_secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Byte counts as this subsystem states them to a human: MiB above a
/// megabyte, KiB rounded up below it. Public because the size in a "file is X
/// — beam v1 caps at Y" refusal and the size a consumer shows beside a landed
/// artifact must read the same; a second formatter is how they stop matching.
pub fn human_bytes(n: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    if n >= MIB {
        format!("{} MiB", n / MIB)
    } else {
        format!("{} KiB", n.div_ceil(1024))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── sanitize_beam_name — the hint is hostile input ──────────────────────

    const H: &str = "abcdef0123456789";

    #[test]
    fn plain_name_passes_through() {
        assert_eq!(sanitize_beam_name(Some("report.html"), H), "report.html");
    }

    #[test]
    fn path_components_are_stripped_both_separators() {
        assert_eq!(sanitize_beam_name(Some("../../etc/passwd"), H), "passwd");
        assert_eq!(sanitize_beam_name(Some("C:\\Users\\x\\evil.html"), H), "evil.html");
    }

    #[test]
    fn dotfiles_and_traversal_stubs_fall_back_or_unhide() {
        assert_eq!(sanitize_beam_name(Some(".."), H), "beam-abcdef01");
        assert_eq!(sanitize_beam_name(Some("."), H), "beam-abcdef01");
        // A dotfile hint becomes a visible name, never a hidden file.
        assert_eq!(sanitize_beam_name(Some(".bashrc"), H), "bashrc");
    }

    #[test]
    fn control_chars_and_empties_are_rejected() {
        assert_eq!(sanitize_beam_name(Some("a\0b\nc.html"), H), "abc.html");
        assert_eq!(sanitize_beam_name(Some("   "), H), "beam-abcdef01");
        assert_eq!(sanitize_beam_name(None, H), "beam-abcdef01");
    }

    #[test]
    fn length_is_bounded_at_128_chars() {
        let long = "x".repeat(500);
        assert_eq!(sanitize_beam_name(Some(&long), H).chars().count(), 128);
    }

    #[test]
    fn multibyte_names_survive() {
        assert_eq!(sanitize_beam_name(Some("informe-año.html"), H), "informe-año.html");
    }

    #[test]
    fn bidi_and_zero_width_format_chars_are_stripped() {
        // U+202E (RLO) is the classic extension-spoof: "report<RLO>gnp.html"
        // renders as "reporthtml.png". It must not survive to the dialog or
        // the on-disk name.
        assert_eq!(sanitize_beam_name(Some("report\u{202E}gnp.html"), H), "reportgnp.html");
        assert_eq!(sanitize_beam_name(Some("a\u{200B}b\u{FEFF}c.html"), H), "abc.html");
        assert_eq!(sanitize_beam_name(Some("\u{2066}\u{2069}"), H), "beam-abcdef01");
    }

    // ── unique_target_path ──────────────────────────────────────────────────

    #[test]
    fn collision_appends_counter_before_extension() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("report.html"), "x").unwrap();
        std::fs::write(dir.path().join("report-2.html"), "x").unwrap();
        let p = unique_target_path(dir.path(), "report.html");
        assert_eq!(p.file_name().unwrap().to_str().unwrap(), "report-3.html");
    }

    #[test]
    fn extensionless_collision_appends_counter_at_end() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("beam-abcdef01"), "x").unwrap();
        let p = unique_target_path(dir.path(), "beam-abcdef01");
        assert_eq!(p.file_name().unwrap().to_str().unwrap(), "beam-abcdef01-2");
    }

    // ── civil_date_string ───────────────────────────────────────────────────

    #[test]
    fn civil_date_known_values() {
        assert_eq!(civil_date_string(0), "1970-01-01");
        // 2026-08-15 00:00:00 UTC
        assert_eq!(civil_date_string(1_786_752_000), "2026-08-15");
        // Leap day 2024-02-29 12:00:00 UTC
        assert_eq!(civil_date_string(1_709_208_000), "2024-02-29");
    }

    // ── link building ───────────────────────────────────────────────────────

    #[test]
    fn link_percent_encodes_the_name() {
        let link = build_link("TICKET", "mi informe año.html", 42);
        assert_eq!(
            link,
            "vlerv://receive?ticket=TICKET&name=mi%20informe%20a%C3%B1o.html&size=42"
        );
    }

    // ── ticket_info rejects garbage before any UI shows ─────────────────────

    #[test]
    fn ticket_info_rejects_malformed_tickets() {
        assert!(ticket_info("not-a-ticket").is_err());
        assert!(ticket_info("").is_err());
    }

    // ── verify_push_ticket — a push is peer-locked, an offer is not ─────────

    #[test]
    fn a_pushed_ticket_must_name_the_pushing_peer_and_the_announced_hash() {
        let pusher = iroh::SecretKey::from_bytes(&[1u8; 32]).public();
        let stranger = iroh::SecretKey::from_bytes(&[2u8; 32]).public();
        let hash = Hash::new(b"pushed bytes");
        let ticket =
            BlobTicket::new(pusher.into(), hash, BlobFormat::Raw).to_string();

        assert_eq!(
            verify_push_ticket(&ticket, pusher, &hash.to_string()).unwrap(),
            hash
        );
        // A control peer must not be able to point the host at a third machine.
        assert_eq!(
            verify_push_ticket(&ticket, stranger, &hash.to_string()).unwrap_err(),
            "a pushed ticket must name the pushing peer"
        );
        // Nor announce one artifact and hand over a ticket for another.
        assert_eq!(
            verify_push_ticket(&ticket, pusher, &Hash::new(b"other").to_string()).unwrap_err(),
            "the pushed ticket does not match the announced content"
        );
    }

    #[test]
    fn a_pushed_ticket_is_single_file_only_like_a_beam_ticket() {
        let pusher = iroh::SecretKey::from_bytes(&[3u8; 32]).public();
        let hash = Hash::new(b"seq");
        let seq = BlobTicket::new(pusher.into(), hash, BlobFormat::HashSeq).to_string();
        assert!(verify_push_ticket(&seq, pusher, &hash.to_string())
            .unwrap_err()
            .contains("single-file"));
        assert!(verify_push_ticket("not-a-ticket", pusher, "x").is_err());
    }

    // ── offers registry: admit / expiry / revocation ────────────────────────

    fn dummy_offer(hash: Hash, expires_at: u64) -> OfferEntry {
        OfferEntry {
            info: OfferInfo {
                // Invariant: an offer id is the blob hash in hex (`remove`
                // relies on it).
                id: hash.to_string(),
                path: PathBuf::from("/tmp/x"),
                name: "x".into(),
                size: 1,
                ticket: "t".into(),
                link: "l".into(),
                created_at: 0,
                expires_at,
                fetches: 0,
            },
            tag: Tag::from("test-tag"),
        }
    }

    #[test]
    fn admit_counts_active_offers_and_denies_unknown() {
        let offers = Offers::new();
        let hash = Hash::new(b"content");
        offers.insert(hash, dummy_offer(hash, now_unix() + 3600));

        assert!(offers.admit(&hash, true).is_ok());
        assert!(offers.admit(&hash, true).is_ok());
        assert_eq!(offers.list()[0].fetches, 2);

        let unknown = Hash::new(b"other");
        assert!(offers.admit(&unknown, true).is_err());
    }

    #[test]
    fn admit_denies_hashseq_requests_even_for_active_offers() {
        let offers = Offers::new();
        let hash = Hash::new(b"content");
        offers.insert(hash, dummy_offer(hash, now_unix() + 3600));
        assert!(offers.admit(&hash, false).is_err());
    }

    #[test]
    fn expired_offers_deny_and_disappear_from_list() {
        let offers = Offers::new();
        let hash = Hash::new(b"content");
        offers.insert(hash, dummy_offer(hash, now_unix().saturating_sub(10)));
        assert!(offers.admit(&hash, true).is_err(), "stale link must be useless");
        assert!(offers.list().is_empty());
        assert_eq!(offers.take_expired(now_unix()).len(), 1);
    }

    #[test]
    fn remove_by_id_revokes() {
        let offers = Offers::new();
        let hash = Hash::new(b"content");
        offers.insert(hash, dummy_offer(hash, now_unix() + 3600));
        assert!(offers.remove(&hash.to_string()).is_some());
        assert!(offers.admit(&hash, true).is_err());
        assert!(offers.remove(&hash.to_string()).is_none());
        // Garbage ids (not a hash) can never remove anything.
        assert!(offers.remove("id").is_none());
    }

    // ── resolve_offerable — the shared offer-path policy ────────────────────

    #[test]
    fn resolve_offerable_enforces_gate_file_and_cap() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("a.html");
        std::fs::write(&file, "x").unwrap();
        let roots = RootSet::new(vec![dir.path().to_path_buf()]);

        let cand = resolve_offerable(&file, &roots).expect("in-root file");
        assert_eq!(cand.name, "a.html");
        assert_eq!(cand.size, 1);

        // Conservative on empty roots (beam exports data off the machine).
        assert_eq!(
            resolve_offerable(&file, &RootSet::empty()).unwrap_err(),
            "path not found or out of root"
        );
        // Directories are not beamable.
        assert_eq!(
            resolve_offerable(dir.path(), &roots).unwrap_err(),
            "only files can be beamed"
        );
    }

    #[test]
    fn resolve_offerable_rejects_over_the_hard_cap() {
        // Sparse file — set_len is instant and allocates no bytes, so this
        // costs nothing on disk yet exercises the dispatch-time cap that
        // stops add_path from copying an oversized file into the store.
        let dir = tempfile::TempDir::new().unwrap();
        let big = dir.path().join("big.bin");
        let f = std::fs::File::create(&big).unwrap();
        f.set_len(HARD_CAP_BYTES + 1).unwrap();
        drop(f);
        let roots = RootSet::new(vec![dir.path().to_path_buf()]);

        let err = resolve_offerable(&big, &roots).unwrap_err();
        assert!(err.starts_with("file is "), "cap error, got: {err}");
    }

    #[test]
    fn ttl_clamp_bounds_both_ends() {
        // The clamp lives inline in `offer` (needs a node), but the arithmetic
        // is what matters: 0 → 1 h, huge → 30 days.
        let clamp = |h: u32| u64::from(h.clamp(MIN_TTL_HOURS, MAX_TTL_HOURS)) * 3600;
        assert_eq!(clamp(0), 3600);
        assert_eq!(clamp(DEFAULT_TTL_HOURS), 24 * 3600);
        assert_eq!(clamp(u32::MAX), 24 * 30 * 3600);
        // The default must be inside the range consumers validate against.
        assert!((MIN_TTL_HOURS..=MAX_TTL_HOURS).contains(&DEFAULT_TTL_HOURS));
    }

    // ── Partial download names ──────────────────────────────────────────────

    #[test]
    fn every_in_flight_download_gets_its_own_temp_name() {
        let hash = "a".repeat(64);
        let first = partial_name(&hash, ".html");
        let second = partial_name(&hash, ".html");
        // Two fetches of the SAME content address must not share a temp file:
        // they would interleave writes and one rename would fail.
        assert_ne!(first, second);
        for name in [&first, &second] {
            assert!(name.starts_with(&format!("{hash}.html.")), "{name} keeps hash and ext");
            assert!(name.ends_with(".partial"), "{name} stays a partial");
        }
        // The Beam receive path passes no extension and still gets a partial
        // that `list_received` never mistakes for a landed artifact.
        assert!(partial_name(&hash, "").ends_with(".partial"));
    }
}
