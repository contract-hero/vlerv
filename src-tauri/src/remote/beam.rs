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
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bao_tree::io::BaoContentItem;
use iroh_blobs::get::request::{get_blob, GetBlobItem};
use iroh_blobs::provider::events::{
    AbortReason, EventMask, EventSender, ProviderMessage, RequestMode,
};
use iroh_blobs::ticket::BlobTicket;
use iroh_blobs::{BlobFormat, Hash};
use n0_future::StreamExt;
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};

use super::endpoint::RemoteNode;
use crate::state_store;

/// Soft warn threshold (frontend dialog copy) and hard cap, per design §5.
pub const WARN_BYTES: u64 = 20 * 1024 * 1024;
pub const HARD_CAP_BYTES: u64 = 256 * 1024 * 1024;
const DEFAULT_TTL_HOURS: u32 = 24;

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
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProgressEvent {
    pub hash: String,
    pub received: u64,
    pub total: Option<u64>,
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
    tag: iroh_blobs::api::Tag,
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

    fn insert(&self, hash: Hash, entry: OfferEntry) {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).insert(hash, entry);
    }

    fn remove(&self, id: &str) -> Option<iroh_blobs::api::Tag> {
        let mut map = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let hash = map
            .iter()
            .find(|(_, e)| e.info.id == id)
            .map(|(h, _)| *h)?;
        map.remove(&hash).map(|e| e.tag)
    }

    /// Drop every expired offer, returning their store tags for cleanup.
    fn take_expired(&self, now: u64) -> Vec<iroh_blobs::api::Tag> {
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

    /// Build the provider-event gate for BlobsProtocol. Every get request is
    /// intercepted and admitted against the registry — this is what makes
    /// Stop an *instant* revocation instead of a GC eventually.
    pub fn gate(self: Arc<Self>, on_change: impl Fn() + Send + Sync + 'static) -> EventSender {
        let mask = EventMask { get: RequestMode::Intercept, ..EventMask::DEFAULT };
        let (tx, mut rx) = EventSender::channel(32, mask);
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if let ProviderMessage::GetRequestReceived(msg) = msg {
                    let res = self.admit(&msg.request.hash, msg.request.ranges.is_blob());
                    let admitted = res.is_ok();
                    msg.tx.send(res).await.ok();
                    if admitted {
                        on_change();
                    }
                }
            }
        });
        tx
    }
}

fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn ttl_hours() -> u32 {
    state_store::current_state()
        .preferences
        .beam_ttl_hours
        .unwrap_or(DEFAULT_TTL_HOURS)
        .max(1)
}

/// Build the `vlerv://receive?…` link. `name`/`size` are display hints for
/// the receiver's confirm dialog — the ticket's hash is the truth.
pub fn build_link(ticket: &str, name: &str, size: u64) -> String {
    let name_enc = utf8_percent_encode(name, QUERY_VALUE_SET);
    format!("vlerv://receive?ticket={ticket}&name={name_enc}&size={size}")
}

/// Parse a ticket string into the facts the confirm dialog shows. Rejects
/// malformed tickets and (v1) anything that isn't a single raw blob.
pub fn ticket_info(ticket_str: &str) -> Result<TicketInfo, String> {
    let ticket: BlobTicket = ticket_str
        .parse()
        .map_err(|e| format!("invalid beam ticket: {e}"))?;
    if ticket.format() != BlobFormat::Raw {
        return Err("beam v1 supports single-file tickets only".to_string());
    }
    let id = ticket.addr().id;
    Ok(TicketInfo {
        node_id: id.to_string(),
        node_id_short: id.fmt_short().to_string(),
        hash: ticket.hash().to_string(),
    })
}

/// Stage `path` into the store and register an offer. Re-beaming the same
/// content refreshes the offer (same hash ⇒ same id) with a fresh TTL.
pub async fn offer(node: &RemoteNode, canonical: &Path) -> Result<OfferInfo, String> {
    let meta = std::fs::metadata(canonical).map_err(|e| e.to_string())?;
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

    // Housekeeping: drop expired offers and their tags while we have the
    // store at hand.
    for tag in node.offers.take_expired(now_unix()) {
        let _ = node.store.tags().delete(tag).await;
    }

    let tag = node
        .store
        .blobs()
        .add_path(canonical)
        .await
        .map_err(|e| format!("cannot stage file: {e}"))?;

    // Wait (bounded) for relay + discovery so the ticket dials from other
    // networks; on timeout the ticket still carries direct addresses, which
    // covers the same-LAN case.
    let _ = tokio::time::timeout(Duration::from_secs(10), node.endpoint.online()).await;

    let ticket = BlobTicket::new(node.endpoint.addr(), tag.hash, tag.format).to_string();
    let name = canonical
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "artifact".to_string());
    let created_at = now_unix();
    let info = OfferInfo {
        id: tag.hash.to_string(),
        path: canonical.to_path_buf(),
        name: name.clone(),
        size,
        link: build_link(&ticket, &name, size),
        ticket,
        created_at,
        expires_at: created_at + u64::from(ttl_hours()) * 3600,
        fetches: 0,
    };
    node.offers.insert(tag.hash, OfferEntry { info: info.clone(), tag: tag.name });
    Ok(info)
}

/// Revoke an offer. The gate stops admitting the hash immediately; the store
/// tag is deleted so the staged bytes become garbage-collectable.
pub async fn stop(node: &RemoteNode, offer_id: &str) {
    if let Some(tag) = node.offers.remove(offer_id) {
        let _ = node.store.tags().delete(tag).await;
    }
    for tag in node.offers.take_expired(now_unix()) {
        let _ = node.store.tags().delete(tag).await;
    }
}

/// Dial a ticket and stream the blob into `received_root`, verified chunk by
/// chunk. `on_progress(hash, received, total_hint)` fires about once per MiB.
pub async fn receive(
    node: &RemoteNode,
    ticket_str: &str,
    name_hint: Option<&str>,
    size_hint: Option<u64>,
    received_root: &Path,
    mut on_progress: impl FnMut(&Hash, u64, Option<u64>),
) -> Result<ReceivedFile, String> {
    let ticket: BlobTicket = ticket_str
        .parse()
        .map_err(|e| format!("invalid beam ticket: {e}"))?;
    if ticket.format() != BlobFormat::Raw {
        return Err("beam v1 supports single-file tickets only".to_string());
    }
    let hash = ticket.hash();

    let connection = tokio::time::timeout(
        Duration::from_secs(30),
        node.endpoint.connect(ticket.addr().clone(), iroh_blobs::ALPN),
    )
    .await
    .map_err(|_| "sender offline — could not reach the sender (timed out)".to_string())?
    .map_err(|e| format!("sender offline — could not reach the sender ({e})"))?;

    // Stream into a partial file next to the final location (same volume ⇒
    // atomic rename). The bao stream yields leaves in ascending offset order
    // for a full-blob request; the offset check guards that invariant.
    let partials_dir = received_root.join(".partial");
    std::fs::create_dir_all(&partials_dir).map_err(|e| e.to_string())?;
    // Unique per attempt: two concurrent receives of the same hash must not
    // interleave writes into one partial file.
    static ATTEMPT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let attempt = ATTEMPT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let partial_path = partials_dir.join(format!("{hash}.{attempt}"));
    let mut file = std::io::BufWriter::new(
        std::fs::File::create(&partial_path).map_err(|e| e.to_string())?,
    );

    let mut written: u64 = 0;
    let mut last_emitted: u64 = 0;
    let mut progress = get_blob(connection, hash);
    let finish = |file: std::io::BufWriter<std::fs::File>| -> Result<(), String> {
        file.into_inner().map_err(|e| e.to_string())?.sync_all().map_err(|e| e.to_string())
    };
    loop {
        match progress.next().await {
            Some(GetBlobItem::Item(BaoContentItem::Leaf(leaf))) => {
                if leaf.offset != written {
                    drop(progress);
                    let _ = std::fs::remove_file(&partial_path);
                    return Err("transfer aborted: non-contiguous stream".to_string());
                }
                written += leaf.data.len() as u64;
                if written > HARD_CAP_BYTES {
                    drop(progress);
                    let _ = std::fs::remove_file(&partial_path);
                    return Err(format!(
                        "transfer aborted: content exceeds the {} cap",
                        human_bytes(HARD_CAP_BYTES)
                    ));
                }
                file.write_all(&leaf.data).map_err(|e| e.to_string())?;
                if written - last_emitted >= PROGRESS_STRIDE_BYTES {
                    last_emitted = written;
                    on_progress(&hash, written, size_hint);
                }
            }
            Some(GetBlobItem::Item(BaoContentItem::Parent(_))) => {}
            Some(GetBlobItem::Done(_stats)) => break,
            Some(GetBlobItem::Error(e)) => {
                let _ = std::fs::remove_file(&partial_path);
                return Err(format!("transfer failed: {e}"));
            }
            None => {
                let _ = std::fs::remove_file(&partial_path);
                return Err("transfer failed: stream ended unexpectedly".to_string());
            }
        }
    }
    finish(file)?;
    on_progress(&hash, written, size_hint);

    let name = sanitize_beam_name(name_hint, &hash.to_string());
    let day_dir = received_root.join(civil_date_string(now_unix()));
    std::fs::create_dir_all(&day_dir).map_err(|e| e.to_string())?;
    let target = unique_target_path(&day_dir, &name);
    std::fs::rename(&partial_path, &target).map_err(|e| e.to_string())?;

    Ok(ReceivedFile { path: target, name, size: written, hash: hash.to_string() })
}

/// Reduce an attacker-controlled name hint to a safe bare filename. The hint
/// travels in the deep link, so it gets the same distrust as any URL input:
/// no path components, no control chars, no dotfiles, bounded length.
pub fn sanitize_beam_name(hint: Option<&str>, hash_hex: &str) -> String {
    let fallback = || format!("beam-{}", &hash_hex[..8.min(hash_hex.len())]);
    let Some(hint) = hint else { return fallback() };

    // Last path segment only — either separator convention.
    let base = hint.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base
        .chars()
        .filter(|c| !c.is_control() && *c != '\0')
        .take(128)
        .collect();
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

/// Past beams, newest first, capped at 100 entries.
pub fn list_received(received_root: &Path) -> Vec<ReceivedEntry> {
    let mut entries = Vec::new();
    let Ok(days) = std::fs::read_dir(received_root) else { return entries };
    for day in days.flatten() {
        if day.file_name().to_string_lossy().starts_with('.') {
            continue; // .partial
        }
        let Ok(files) = std::fs::read_dir(day.path()) else { continue };
        for f in files.flatten() {
            let Ok(meta) = f.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }
            let received_at = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            entries.push(ReceivedEntry {
                path: f.path(),
                name: f.file_name().to_string_lossy().into_owned(),
                size: meta.len(),
                received_at,
            });
        }
    }
    entries.sort_by(|a, b| b.received_at.cmp(&a.received_at));
    entries.truncate(100);
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

fn human_bytes(n: u64) -> String {
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

    // ── offers registry: admit / expiry / revocation ────────────────────────

    fn dummy_offer(expires_at: u64) -> OfferEntry {
        OfferEntry {
            info: OfferInfo {
                id: "id".into(),
                path: PathBuf::from("/tmp/x"),
                name: "x".into(),
                size: 1,
                ticket: "t".into(),
                link: "l".into(),
                created_at: 0,
                expires_at,
                fetches: 0,
            },
            tag: iroh_blobs::api::Tag::from("test-tag"),
        }
    }

    #[test]
    fn admit_counts_active_offers_and_denies_unknown() {
        let offers = Offers::new();
        let hash = Hash::new(b"content");
        offers.insert(hash, dummy_offer(now_unix() + 3600));

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
        offers.insert(hash, dummy_offer(now_unix() + 3600));
        assert!(offers.admit(&hash, false).is_err());
    }

    #[test]
    fn expired_offers_deny_and_disappear_from_list() {
        let offers = Offers::new();
        let hash = Hash::new(b"content");
        offers.insert(hash, dummy_offer(now_unix().saturating_sub(10)));
        assert!(offers.admit(&hash, true).is_err(), "stale link must be useless");
        assert!(offers.list().is_empty());
        assert_eq!(offers.take_expired(now_unix()).len(), 1);
    }

    #[test]
    fn remove_by_id_revokes() {
        let offers = Offers::new();
        let hash = Hash::new(b"content");
        offers.insert(hash, dummy_offer(now_unix() + 3600));
        assert!(offers.remove("id").is_some());
        assert!(offers.admit(&hash, true).is_err());
        assert!(offers.remove("id").is_none());
    }
}
