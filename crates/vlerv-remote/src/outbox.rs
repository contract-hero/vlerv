// The outbox — one file per accepted-but-undelivered send.
//
// A send to a paired device that is asleep used to fail with "the session is
// closed", and that was the end of it: nothing remembered the request. This
// module is the durable half of the answer — the bytes are copied into the
// blob store under a persistent tag at the moment the send is accepted, and a
// JSON record names the peer, that tag and the content address until the
// delivery lands or the record expires.
//
// NOTHING HERE IS ON THE WIRE, and a reviewer will look, so the reason lives
// here. No `proto::Req`, `proto::Res`, `proto::Event` or `HelloAck` variant is
// added, no ALPN is registered and `PROTO_VERSION` stays 1: a queued delivery
// replays the EXISTING `Req::PushArtifact`, with a fresh grant and a fresh
// ticket minted per attempt. An appended request variant would not be ignored
// by an older peer — `decode_frame` returns `Err`, `ScopeServer::serve`
// propagates it and the connection is closed, so the client reports "the
// session is closed", which is the exact failure this module exists to remove
// and is indistinguishable from a sleeping phone. Appending a field to
// `HelloAck` breaks the other direction, because postcard hands a new reader
// of an old three-field payload a `DeserializeUnexpectedEnd`. If a verb is
// ever genuinely needed, it belongs on its own ALPN beside `SCOPE_ALPN`, so
// an older peer refuses the dial locally instead of losing a live session.
//
// A record is a CAPABILITY DOCUMENT: it names a file on this disk, a peer that
// may fetch it, and the pin that keeps a private copy of the user's bytes in
// the store. So it lives in the 0600 class with identity.key and peers.json,
// and every mutation reaches disk BEFORE memory. `PeerStore::remove` is the
// one place in this codebase that inverts that, because a revocation whose
// write fails is still right to take effect; nothing here has that shape. A
// record this process holds in memory but never wrote is a promise no restart
// can keep, and the boot sweep unpins its bytes as an orphan; a record dropped
// from memory whose file stays comes back at the next boot with its pin
// already released, which is a delivery that can never succeed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::beam::human_bytes;
use crate::paths;
use crate::peers::now_unix;

/// On-disk schema version of ONE record. Versioned per file rather than per
/// spool: a document written by a newer build must be left alone, and a whole
/// list would make one such document hide every other pending send.
pub const OUTBOX_SCHEMA: u32 = 1;

/// The one prefix every pin this spool owns is named under, so a sweep can
/// find them all and can never touch a Beam offer's or a grant's tag.
pub const TAG_PREFIX: &str = "outbox/";

/// How many records the spool holds. A full spool REFUSES the send, naming
/// the cap and the device; it never drops the oldest record. That is the
/// opposite of the `MAX_RECEIVED` policy in the MCP server, and deliberately:
/// a dropped `Received` entry is a log line the caller can live without,
/// while the oldest queue record is a file the user was promised would be
/// delivered.
pub const MAX_RECORDS: usize = 64;

/// Total staged bytes the spool may hold. Every queued send is a private full
/// copy of a user file inside the state directory (and stays one until the
/// delivery lands or the record expires), so the ceiling is stated in bytes
/// as well as in records.
pub const MAX_SPOOL_BYTES: u64 = 1024 * 1024 * 1024;

/// How long an undelivered record is kept. A week covers a phone left in a
/// drawer over a holiday; past that, the honest answer is that the send is
/// not going to happen, and the copy of the user's file stops being kept.
pub const RECORD_TTL_SECS: u64 = 7 * 24 * 3600;

/// How often the drain wakes on its own. The precise trigger is a peer
/// connecting; this is the fallback that covers a peer that comes back
/// without ever dialing in. Naming a peer costs an n0 discovery lookup and a
/// possible relay traversal, so each avoided tick is one less third-party
/// observation of who talks to whom.
pub const DRAIN_TICK: Duration = Duration::from_secs(60);

/// Records one pass may push to ONE peer. A bound, not a batch size: a pass
/// that tried to empty a full spool down one session would hold that peer for
/// minutes and starve every other one.
pub const MAX_PER_PASS: usize = 8;

/// The tag that pins one record's bytes. THE one deriver: `stage_outbox`
/// names the pin with it before the record exists, `enqueue` stores what it
/// returns, and the sweep's keep-set is built from the stored names — so a
/// record and the tag holding its bytes cannot drift apart.
pub fn tag_name(id: &str) -> String {
    format!("{TAG_PREFIX}{id}")
}

/// One accepted-but-undelivered send.
///
/// No `BlobTicket` and no grant is stored, deliberately: a ticket names
/// addresses this process stops holding at its next restart, and a grant is
/// in-memory with an hour's TTL. Both are minted fresh per attempt. No iroh
/// type appears here either — `hash` and `tag` are plain strings, which is
/// what keeps this module readable by a consumer that never links iroh.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// Also the file stem and the tag suffix.
    pub id: String,
    /// 64-hex EndpointId — the peers.json key, re-checked at drain time.
    pub peer: String,
    /// The device name as it stood at enqueue. Display only: the peer record
    /// is the authority, and it may have been renamed since.
    pub device: String,
    /// The canonical path the gate resolved. Kept for the drain-time root
    /// check, never re-read for bytes.
    pub source: PathBuf,
    /// What the receiver is told the file is called.
    pub name: String,
    pub size: u64,
    /// BLAKE3 hex of the STAGED bytes — the snapshot, not whatever the source
    /// path holds by the time the phone wakes up.
    pub hash: String,
    /// `outbox/<id>`, the pin that keeps those bytes on disk.
    pub tag: String,
    pub enqueued_at: u64,
    pub expires_at: u64,
    pub attempts: u32,
    pub last_attempt_at: u64,
    pub last_error: Option<String>,
}

/// The versioned envelope one record file holds.
#[derive(Serialize, Deserialize)]
struct RecordDoc {
    v: u32,
    record: Record,
}

/// What the caller knows about a send it has already staged. The spool owns
/// every field the caller must NOT choose — the tag name, both timestamps and
/// the attempt counters — so a record cannot be written with a pin the sweep
/// will not keep or a TTL nothing agreed to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Staged {
    pub id: String,
    pub peer: String,
    pub device: String,
    pub source: PathBuf,
    pub name: String,
    pub size: u64,
    pub hash: String,
}

/// Everything one directory read produced, replaced as a unit by `reload`.
///
/// The report fields sit under the SAME lock as the records because they are
/// read together: a keep-set built from a fresh record list and a stale
/// quarantine list would unpin bytes belonging to a file this build could not
/// parse.
#[derive(Default)]
struct Spool {
    records: BTreeMap<String, Record>,
    /// Set when the outbox DIRECTORY itself could not be read. While it
    /// stands, `enqueue` refuses (so a caller reports a hard error instead of
    /// a false promise) and `live_tags` answers `None` (so no sweep runs).
    load_error: Option<String>,
    /// Stems this build could not parse, quarantined to `.broken.<unix>`.
    /// Rebuilt from the moved-aside files themselves on every read, so the
    /// list does not empty out on the second one.
    quarantined: Vec<String>,
    /// Stems whose schema version this build does not write.
    foreign: Vec<String>,
    /// Stems whose file would not READ this time — not corrupt, just not
    /// available to this process right now. Left untouched so a later boot
    /// can pick the delivery back up.
    io_faults: Vec<String>,
}

/// Why one record file did not become a live record. All three answers keep
/// the stem in the tag keep-set — bytes this build cannot account for are the
/// last thing it should delete — and they differ in what happens to the file.
enum Fault {
    /// The bytes were read and they are not a record this build can use.
    /// Moved aside, because no later boot changes that answer.
    Unreadable(String),
    /// Written by a schema version this build does not know.
    Foreign(u32),
    /// The bytes could not be READ AT ALL: EACCES after a permissions repair,
    /// a changed owner after a restore from backup, EIO on a bad sector. The
    /// file is left alone, because the record behind it may be perfectly
    /// valid — quarantining it would rename a promised delivery to
    /// `.broken.<unix>` and abandon it permanently over a condition that
    /// passes on its own.
    Io(String),
}

/// The spool, backed by `<base>/remote/outbox/`. Lock discipline matches the
/// peer store: short critical sections, never held across an await — there is
/// no await in this module at all, because staging and unpinning are the blob
/// store's job and live in `beam`.
pub struct Outbox {
    dir: PathBuf,
    inner: Mutex<Spool>,
    /// Breaks ties between two sends accepted inside one millisecond, and —
    /// because `load` SEEDS it past every sequence number the directory
    /// already holds — between two sends accepted in the same millisecond by
    /// two different runs.
    ///
    /// A counter that restarted at 0 left the millisecond stamp as the only
    /// thing separating a restart's first id from the first id of the run
    /// before it, and `now_millis` answers 0 for every call on a machine
    /// whose clock reports a time before 1970. There, every process minted
    /// `0000000000000-0000` first: the second one's `stage_outbox` retargeted
    /// the incumbent's `outbox/<id>` pin at its own bytes, and the cleanup
    /// that runs when `enqueue`'s claim then fails deleted that pin. The
    /// claim kept the incumbent's FILE and lost its BYTES.
    seq: AtomicU64,
}

impl Outbox {
    /// Read `<base>/remote/outbox/`, tolerating a missing directory (nothing
    /// was ever queued). `dir` IS the outbox directory: `Dirs::outbox()`
    /// names it, so no second place derives the layout.
    pub fn load(dir: &Path) -> Self {
        let spool = Spool::read(dir);
        let seq = AtomicU64::new(next_seq_after(&spool));
        Self { dir: dir.to_path_buf(), inner: Mutex::new(spool), seq }
    }

    /// Re-read the directory. The boot path runs this before it decides
    /// anything: a previous process may have completed or expired records
    /// after `load` read the directory, and acting on that stale list would
    /// re-push a file that already landed.
    pub fn reload(&self) {
        let next = Spool::read(&self.dir);
        // The counter only ever climbs. A reload that lowered it would hand
        // back a sequence number this run has already minted, and the id
        // carrying it names a pin that is already holding somebody's bytes.
        self.seq.fetch_max(next_seq_after(&next), Ordering::SeqCst);
        *self.guard() = next;
    }

    fn guard(&self) -> std::sync::MutexGuard<'_, Spool> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn path_of(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    /// Why the spool did not load, if it did not. A caller surfaces this
    /// rather than reporting an empty queue: the records are hidden, not
    /// gone, and nothing may be queued or swept while it stands.
    pub fn load_error(&self) -> Option<String> {
        self.guard().load_error.clone()
    }

    /// Stems moved aside as `<id>.json.broken.<unix>`. Counted and surfaced
    /// so an unparseable record is a visible fact rather than a delivery that
    /// silently never happens.
    pub fn quarantined(&self) -> Vec<String> {
        self.guard().quarantined.clone()
    }

    /// Stems written by a newer schema. Left byte-for-byte alone: the build
    /// that wrote them is the one that can finish them.
    pub fn foreign(&self) -> Vec<String> {
        self.guard().foreign.clone()
    }

    /// Stems whose file would not read this time. Surfaced beside `foreign`
    /// and for the same reason: the record is not on the pending list, so a
    /// caller that did not report it would show a delivery as simply gone.
    /// The file is untouched, so the next boot may well read it.
    pub fn io_faults(&self) -> Vec<String> {
        self.guard().io_faults.clone()
    }

    /// Every pending record, in id order — which is enqueue order, because
    /// the id starts with the millisecond it was accepted at.
    pub fn list(&self) -> Vec<Record> {
        self.guard().records.values().cloned().collect()
    }

    /// The records queued for one peer, in id order. The drain groups by peer
    /// because a dial to a sleeping phone costs `DIAL_TIMEOUT`, so five
    /// records for one device must cost one dial, not five.
    pub fn for_peer(&self, peer: &str) -> Vec<Record> {
        self.guard()
            .records
            .values()
            .filter(|r| r.peer == peer)
            .cloned()
            .collect()
    }

    /// The pending record for this exact (peer, content) pair, if there is
    /// one. The dedupe lookup: a language model retries a call that looked
    /// like it failed, and without this one user intent becomes several
    /// records — each pinning its own tag on the same bytes.
    pub fn find_pending(&self, peer: &str, hash: &str) -> Option<Record> {
        self.guard()
            .records
            .values()
            .find(|r| r.peer == peer && r.hash == hash)
            .cloned()
    }

    /// Mint the next record id: the millisecond it was accepted at, then a
    /// counter that breaks ties inside that millisecond. Sortable by enqueue
    /// time and safe as a filename.
    ///
    /// The id names no content, and it cannot: it is minted BEFORE the file
    /// is staged, because `beam::stage_outbox` must know the tag name before
    /// the single await that both copies the bytes and pins them, and the
    /// content address only exists once that await has returned.
    ///
    /// So the counter is the whole of what makes an id unique, which is why
    /// it is seeded off the directory rather than started at 0. It is only
    /// half the guard even so: `beam::stage_outbox` refuses a tag name the
    /// store already holds, so an id that repeated anyway — from a stem this
    /// module did not mint, or a tag no file names any more — still cannot
    /// take another record's pin.
    pub fn next_id(&self) -> String {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst);
        format!("{:013}-{:04}", now_millis(), seq)
    }

    /// Would one more send of `size` bytes fit? Called BEFORE the bytes are
    /// staged, so a refused send never costs a copy of the file, and again
    /// inside `enqueue`, so nothing can slip past by taking the other door.
    pub fn room_for(&self, device: &str, size: u64) -> Result<(), String> {
        room_in(&self.guard(), device, size)
    }

    /// Persist one accepted send. Disk before memory, and the id is CLAIMED
    /// with `create_private` (`create_new`) before anything is written to it:
    /// a backwards clock jump plus tmp+rename would otherwise overwrite a
    /// record that was already accepted, and the user would be told both
    /// sends are pending while one file is gone.
    ///
    /// Any failure here leaves nothing behind and returns `Err`. The caller
    /// must unpin the tag it staged under, because an unpersisted record can
    /// never be reported as accepted and its pin would be unreachable.
    pub fn enqueue(&self, item: Staged) -> Result<Record, String> {
        let mut spool = self.guard();
        if let Some(reason) = &spool.load_error {
            // The peers.json wording, for the same situation: the records are
            // hidden, not gone, so the way out is to move the unreadable
            // thing aside rather than to write over it.
            return Err(format!(
                "refusing to queue a send into {:?}: it did not load ({reason}). \
                 Nothing is queued or swept while that stands.",
                self.dir
            ));
        }
        room_in(&spool, &item.device, item.size)?;

        let now = now_unix();
        let record = Record {
            tag: tag_name(&item.id),
            id: item.id,
            peer: item.peer,
            device: item.device,
            source: item.source,
            name: item.name,
            size: item.size,
            hash: item.hash,
            enqueued_at: now,
            expires_at: now + RECORD_TTL_SECS,
            attempts: 0,
            last_attempt_at: 0,
            last_error: None,
        };

        std::fs::create_dir_all(&self.dir)
            .map_err(|e| format!("cannot create {:?}: {e}", self.dir))?;
        let final_path = self.path_of(&record.id);
        paths::create_private(&final_path, b"").map_err(|e| {
            format!(
                "refusing to queue a send: the record file {final_path:?} could not be \
                 claimed ({e}). A file already there is a send that was accepted, and \
                 writing over it would lose that delivery."
            )
        })?;
        if let Err(e) = self.save_record(&record) {
            // The claim is the only trace left, and an empty file would be
            // quarantined on the next load and keep a pin alive for a record
            // that never existed. Best effort: the write already failed, so
            // this one may too, and the caller is told either way.
            let _ = std::fs::remove_file(&final_path);
            return Err(e);
        }
        spool.records.insert(record.id.clone(), record.clone());
        Ok(record)
    }

    /// A delivery landed. Returns the record so the caller can unpin its tag
    /// AFTERWARDS: a crash between the two leaks a tag, which the next boot
    /// sweep collects, while the other order leaves a record whose bytes are
    /// gone and a delivery that can never succeed.
    pub fn complete(&self, id: &str) -> Result<Option<Record>, String> {
        self.remove(id)
    }

    /// Drop a record that must not be retried — a revoked peer, bytes that
    /// are no longer in the store, a source outside every root of every
    /// session that will ever run. The reason is printed rather than
    /// swallowed: a delivery the user was promised is ending here.
    pub fn drop_record(&self, id: &str, reason: &str) -> Result<Option<Record>, String> {
        let dropped = self.remove(id)?;
        if let Some(record) = &dropped {
            eprintln!(
                "vlerv: remote: dropping the queued send of {:?} to {} — {reason}",
                record.name, record.device
            );
        }
        Ok(dropped)
    }

    /// Every record past its TTL, removed and handed back so the caller can
    /// unpin them. Same order as `complete`, for the same reason.
    pub fn take_expired(&self, now: u64) -> Vec<Record> {
        let ids: Vec<String> = {
            let spool = self.guard();
            spool
                .records
                .values()
                .filter(|r| r.expires_at <= now)
                .map(|r| r.id.clone())
                .collect()
        };
        let mut expired = Vec::new();
        for id in ids {
            match self.remove(&id) {
                Ok(Some(record)) => {
                    eprintln!(
                        "vlerv: remote: the queued send of {:?} to {} expired undelivered",
                        record.name, record.device
                    );
                    expired.push(record);
                }
                Ok(None) => {}
                // A record whose file will not go must stay in memory: it is
                // coming back on the next load, and reporting it gone would
                // lose the pin that keeps its bytes.
                Err(e) => eprintln!("vlerv: remote: cannot expire a queued send: {e}"),
            }
        }
        expired
    }

    /// Note one delivery attempt: the counter the backoff ladder is derived
    /// from, when it happened, and why it did not land. `None` is the arm for
    /// an attempt that produced no error: it counts like every other one and
    /// it clears whatever reason the record was carrying.
    ///
    /// No drain path passes `None`, and that is the design rather than a gap.
    /// A record the running session can serve is pushed on the same visit,
    /// and it then either leaves the spool or takes that pass's own reason —
    /// so a pending record explaining a condition that has passed is not a
    /// state the drain produces. Clearing a reason to say a hold is over
    /// would also tell the user less, not more: `server_status` renders any
    /// record whose `last_error` is `None` as "not tried yet", however many
    /// attempts stand behind it.
    ///
    /// A REPEAT OF THE SAME REASON IS NOT AN ATTEMPT, and is not written —
    /// the rule `note_ack_scope` states for the peer store, for the same
    /// reason. A record held on a condition that does not change (its source
    /// is outside this session's roots) is visited by every pass, and
    /// counting each visit rewrote a 0600 file per pass and made
    /// `server_status` report "attempt 1440" after a day: 1440 delivery
    /// attempts a reader would look for in a log and never find.
    pub fn record_attempt(&self, id: &str, error: Option<String>) -> Result<(), String> {
        let mut spool = self.guard();
        let Some(record) = spool.records.get(id) else {
            return Ok(());
        };
        if record.last_error == error {
            return Ok(());
        }
        let mut next = record.clone();
        next.attempts = next.attempts.saturating_add(1);
        next.last_attempt_at = now_unix();
        next.last_error = error;
        self.save_record(&next)?;
        spool.records.insert(next.id.clone(), next);
        Ok(())
    }

    /// Whether the drain has anything to do. The one counting question this
    /// type answers, because it is the one the supervisor asks before it arms
    /// a timer; totals are summed from `list`, at the surface that prints
    /// them, so a number and the records beside it cannot disagree.
    pub fn is_empty(&self) -> bool {
        self.guard().records.is_empty()
    }

    /// The keep-set for `beam::sweep_outbox_tags`: every tag this spool still
    /// needs, including the ones belonging to quarantined, newer-schema and
    /// unreadable-this-time files — bytes this build cannot account for are
    /// the last it should unpin.
    ///
    /// `None` when the directory did not load, and the sweep MUST NOT run
    /// then. That is the whole reason this returns an option instead of a
    /// list: an unreadable spool has an empty record map, so a sweep that ran
    /// anyway would delete every pin and take every pending file with it,
    /// with no error anywhere.
    pub fn live_tags(&self) -> Option<Vec<String>> {
        let spool = self.guard();
        if spool.load_error.is_some() {
            return None;
        }
        let mut tags: Vec<String> = spool.records.values().map(|r| r.tag.clone()).collect();
        tags.extend(spool.quarantined.iter().map(|stem| tag_name(stem)));
        tags.extend(spool.foreign.iter().map(|stem| tag_name(stem)));
        tags.extend(spool.io_faults.iter().map(|stem| tag_name(stem)));
        Some(tags)
    }

    /// Take one record off disk and out of the map, disk first. `None` when
    /// the id names nothing — completing twice is not an error, and neither
    /// is expiring a record another pass already delivered.
    fn remove(&self, id: &str) -> Result<Option<Record>, String> {
        let mut spool = self.guard();
        if !spool.records.contains_key(id) {
            return Ok(None);
        }
        let path = self.path_of(id);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            // Already gone on disk: the memory copy is the stale one, so
            // dropping it is the correction, not a loss.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("cannot remove {path:?}: {e}")),
        }
        Ok(spool.records.remove(id))
    }

    /// Atomic write (tmp + rename), 0600 — the record names a file on this
    /// disk and a peer that may fetch it.
    ///
    /// Never locks: every caller already holds the guard, which is what keeps
    /// a cap check and the write it admitted from being interleaved.
    fn save_record(&self, record: &Record) -> Result<(), String> {
        let doc = RecordDoc { v: OUTBOX_SCHEMA, record: record.clone() };
        let json = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| format!("cannot create {:?}: {e}", self.dir))?;
        let path = self.path_of(&record.id);
        let tmp = path.with_extension("json.tmp");
        paths::write_private(&tmp, json.as_bytes())?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("cannot write {path:?}: {e}"))
    }
}

/// The cap check, over a spool the caller already holds. Both entry points
/// run THIS, so the answer a send gets before it is staged and the answer it
/// gets as it is written cannot disagree.
fn room_in(spool: &Spool, device: &str, size: u64) -> Result<(), String> {
    if spool.records.len() >= MAX_RECORDS {
        return Err(format!(
            "the send queue is full at {MAX_RECORDS} pending deliveries, so the send to \
             {device} was not queued. Nothing is dropped to make room — every record here \
             is a file somebody was told would arrive."
        ));
    }
    let staged: u64 = spool.records.values().map(|r| r.size).sum();
    if staged.saturating_add(size) > MAX_SPOOL_BYTES {
        return Err(format!(
            "the send queue already holds {} of its {} limit, so the {} send to {device} was \
             not queued. Nothing is dropped to make room — every record here is a file \
             somebody was told would arrive.",
            human_bytes(staged),
            human_bytes(MAX_SPOOL_BYTES),
            human_bytes(size)
        ));
    }
    Ok(())
}

impl Spool {
    /// One directory read. A missing directory is a fresh install, not a
    /// failure; a directory that cannot be READ is, because the difference
    /// between "nothing is queued" and "the queue is unreadable" decides
    /// whether the sweep may unpin anything.
    fn read(dir: &Path) -> Self {
        let mut spool = Spool::default();
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return spool,
            Err(e) => {
                let msg = format!("cannot read {dir:?}: {e}");
                eprintln!(
                    "vlerv: remote: {msg} — no send is queued and no staged copy is swept \
                     while that stands"
                );
                spool.load_error = Some(msg);
                return spool;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            // QUARANTINE IS STICKY, and this is the line that makes it so. A
            // read happens on every `load` and every `reload`, and only the
            // FIRST one sees `<id>.json` — after that the file is named
            // `<id>.json.broken.<unix>`. Counting it only on that first read
            // dropped the stem out of `live_tags` on the very next one, and
            // `Drainer::reconcile` reloads before it builds the keep-set, so
            // the boot sweep unpinned the staged bytes of the one record this
            // build could not account for. It also emptied the
            // `queue_unreadable` list, which is the only place a human hears
            // about the delivery at all.
            if let Some(stem) = broken_stem(name) {
                spool.quarantined.push(stem.to_string());
                continue;
            }
            // Records only. This also skips what a crash leaves behind — a
            // `.json.tmp` staging file — because `extension()` reads the LAST
            // component and that does not end in `json`.
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()).map(str::to_string) else {
                continue;
            };
            match read_record(&path, &stem) {
                Ok(record) => {
                    spool.records.insert(record.id.clone(), record);
                }
                Err(Fault::Foreign(v)) => {
                    eprintln!(
                        "vlerv: remote: outbox record {stem} is schema v{v} but this build \
                         reads v{OUTBOX_SCHEMA} — leaving it alone, and keeping its staged bytes"
                    );
                    spool.foreign.push(stem);
                }
                Err(Fault::Io(why)) => {
                    eprintln!(
                        "vlerv: remote: outbox record {stem} could not be read ({why}) — leaving \
                         it alone, and keeping its staged bytes; a later boot retries it"
                    );
                    spool.io_faults.push(stem);
                }
                Err(Fault::Unreadable(why)) => {
                    eprintln!(
                        "vlerv: remote: outbox record {stem} is unreadable ({why}) — moving it \
                         aside; it is never replayed and never rewritten"
                    );
                    quarantine(&path);
                    spool.quarantined.push(stem);
                }
            }
        }
        // read_dir yields whatever order the filesystem likes; the records
        // are sorted by the map, and these three are reported to a human.
        spool.quarantined.sort();
        // One stem, one entry. A clock that went backwards can hand a stem
        // whose file was already moved aside back to `enqueue`, so a second
        // `.broken.<unix>` for one stem is reachable, and naming the same
        // dead delivery twice would read as two.
        spool.quarantined.dedup();
        spool.foreign.sort();
        spool.io_faults.sort();
        spool
    }
}

/// The record stem behind a quarantined file, or `None` for anything else the
/// directory holds. `<id>.json.broken.<unix>` is the only name `quarantine`
/// writes, and the stamp must be digits: a file a user dropped in here called
/// `notes.json.broken.bak` names no record and must not enter the keep-set.
fn broken_stem(name: &str) -> Option<&str> {
    let (head, stamp) = name.rsplit_once(".broken.")?;
    if stamp.is_empty() || !stamp.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    head.strip_suffix(".json")
}

/// The version field, read on its own. Serde ignores what it was not asked
/// about, so this parses a document of any shape that carries a `v`.
#[derive(Deserialize)]
struct VersionOnly {
    v: u32,
}

/// Parse one record file. The id must match the file stem: `complete` derives
/// the path it deletes from the id, so a record whose two names disagree
/// would either delete a sibling's file or fail forever — and a hand edit is
/// exactly how they come to disagree.
fn read_record(path: &Path, stem: &str) -> Result<Record, Fault> {
    // AN I/O ERROR IS NOT CORRUPTION. A record this build never managed to
    // read may be a perfectly good delivery behind a condition that passes —
    // EACCES after a permissions repair, a changed owner after a restore from
    // backup, EIO on a bad sector. Calling that `Unreadable` renamed the file
    // to `.broken.<unix>` and abandoned the send for good, which is the one
    // thing quarantine is not for: the module scopes it to a file this build
    // CANNOT PARSE.
    let raw = std::fs::read_to_string(path).map_err(|e| Fault::Io(e.to_string()))?;
    // The version FIRST, in a pass of its own. A record written by a newer
    // build almost certainly carries fields this one does not know, so
    // parsing the payload first would call every foreign record corrupt and
    // MOVE IT ASIDE — the one thing the newer-schema rule forbids.
    let version: VersionOnly =
        serde_json::from_str(&raw).map_err(|e| Fault::Unreadable(e.to_string()))?;
    if version.v != OUTBOX_SCHEMA {
        return Err(Fault::Foreign(version.v));
    }
    let doc: RecordDoc =
        serde_json::from_str(&raw).map_err(|e| Fault::Unreadable(e.to_string()))?;
    if doc.record.id != stem {
        return Err(Fault::Unreadable(format!(
            "it calls itself {:?} but its file is named {stem:?}",
            doc.record.id
        )));
    }
    Ok(doc.record)
}

/// Move an unparseable record aside, the way the app moves a corrupt
/// state.json aside. It is never replayed and never rewritten — the pin it
/// names stays in the keep-set, because unpinning bytes this build cannot
/// account for is how a pending file disappears with no error anywhere.
fn quarantine(path: &Path) {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return;
    };
    let broken = path.with_file_name(format!("{name}.broken.{}", now_unix()));
    if let Err(e) = std::fs::rename(path, &broken) {
        eprintln!("vlerv: remote: cannot move {path:?} aside: {e}");
    }
}

/// The first sequence number no stem in the directory can already carry, so
/// no id this process mints can repeat one — two ids differ as soon as their
/// counters do, whatever the clock says about the millisecond half.
///
/// EVERY stem counts, not just the live records. A quarantined, foreign or
/// unreadable-this-time file keeps its `outbox/<id>` pin in the keep-set, and
/// re-minting one of those ids is the worse half of the collision: the file
/// is named `<id>.json.broken.<unix>` or holds a schema this build will not
/// touch, so `enqueue`'s `create_new` claim does NOT clash, the send is
/// accepted, and the staging silently repoints a pin at bytes its record
/// never named.
fn next_seq_after(spool: &Spool) -> u64 {
    spool
        .records
        .keys()
        .chain(spool.quarantined.iter())
        .chain(spool.foreign.iter())
        .chain(spool.io_faults.iter())
        .filter_map(|stem| seq_of(stem))
        .max()
        .map_or(0, |seq| seq.saturating_add(1))
}

/// The counter half of `{millis}-{seq}`, or `None` for a name this module did
/// not mint. Only the tail is read: the millisecond half is the part a wrong
/// clock corrupts, so the guard against a repeated id must not lean on it.
fn seq_of(stem: &str) -> Option<u64> {
    stem.rsplit_once('-')?.1.parse().ok()
}

/// Unix milliseconds, the resolution the record id is ordered by. Seconds are
/// not enough: a model that sends three files in one turn would put all three
/// in one second, and the tie-break counter would be the only ordering left.
fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spool(dir: &tempfile::TempDir) -> Outbox {
        Outbox::load(&dir.path().join("outbox"))
    }

    /// What the spool holds, asked the way every caller asks it: off `list`.
    /// There is no `len`/`bytes` pair to ask instead, deliberately — a second
    /// way to count is a second answer that can disagree with the records
    /// printed beside it.
    fn depth(out: &Outbox) -> usize {
        out.list().len()
    }

    fn staged_bytes(out: &Outbox) -> u64 {
        out.list().iter().map(|r| r.size).sum()
    }

    fn staged(id: &str, peer: &str, device: &str, size: u64) -> Staged {
        Staged {
            id: id.to_string(),
            peer: peer.to_string(),
            device: device.to_string(),
            source: PathBuf::from("/w/report.html"),
            name: "report.html".to_string(),
            size,
            hash: "a".repeat(64),
        }
    }

    // ── Record persistence ─────────────────────────────────────────────────

    #[test]
    fn a_record_round_trips_through_disk_and_a_fresh_load_sees_it() {
        let dir = tempfile::TempDir::new().unwrap();
        let out = spool(&dir);
        assert!(out.list().is_empty());
        assert!(out.is_empty());

        let id = out.next_id();
        let record = out.enqueue(staged(&id, "nodeA", "iPhone", 4096)).unwrap();
        assert_eq!(record.tag, format!("outbox/{id}"), "the pin is named after the record");
        assert_eq!(record.attempts, 0);
        assert_eq!(record.expires_at, record.enqueued_at + RECORD_TTL_SECS);
        assert_eq!(depth(&out), 1);
        assert_eq!(staged_bytes(&out), 4096);

        // A fresh load sees the persisted record — this is what survives the
        // process that accepted the send.
        let reloaded = spool(&dir);
        assert_eq!(reloaded.list(), vec![record.clone()], "every field, byte for byte");
        assert_eq!(reloaded.find_pending("nodeA", &record.hash), Some(record.clone()));
        assert_eq!(reloaded.find_pending("nodeB", &record.hash), None, "records are per peer");
        assert_eq!(reloaded.for_peer("nodeA"), vec![record]);
        assert!(reloaded.for_peer("nodeB").is_empty());
    }

    #[test]
    fn a_completed_delivery_stays_completed_after_a_reload() {
        let dir = tempfile::TempDir::new().unwrap();
        let out = spool(&dir);
        let kept = out.next_id();
        out.enqueue(staged(&kept, "nodeA", "iPhone", 10)).unwrap();
        let landed = out.next_id();
        out.enqueue(staged(&landed, "nodeB", "iPad", 20)).unwrap();

        let done = out.complete(&landed).unwrap().expect("the record was there");
        assert_eq!(done.tag, tag_name(&landed), "the caller is handed the pin to release");
        assert!(out.complete(&landed).unwrap().is_none(), "completing twice is not an error");

        // The other record is untouched, and the completion is on disk: a
        // delivery that came back after a restart would push the file twice.
        let reloaded = spool(&dir);
        assert_eq!(depth(&reloaded), 1);
        assert_eq!(reloaded.list()[0].id, kept);
        assert_eq!(staged_bytes(&reloaded), 10);
    }

    #[test]
    fn an_attempt_is_recorded_on_disk_so_the_reason_survives_a_restart() {
        let dir = tempfile::TempDir::new().unwrap();
        let out = spool(&dir);
        let id = out.next_id();
        out.enqueue(staged(&id, "nodeA", "iPhone", 10)).unwrap();

        out.record_attempt(&id, Some("the peer did not answer the handshake".to_string()))
            .unwrap();
        let after = &spool(&dir).list()[0];
        assert_eq!(after.attempts, 1);
        assert!(after.last_attempt_at > 0);
        assert_eq!(after.last_error.as_deref(), Some("the peer did not answer the handshake"));

        // The clearing arm, pinned because the reason has to go rather than
        // survive an attempt that reported none. No drain path passes `None`
        // today — `record_attempt` says why — so this is the only thing
        // holding that half of the signature to its meaning.
        out.record_attempt(&id, None).unwrap();
        let cleared = &spool(&dir).list()[0];
        assert_eq!(cleared.attempts, 2);
        assert_eq!(cleared.last_error, None);
        // A record that is not there is not an error — a pass can race a
        // completion, and a panicking drain would strand every other record.
        assert!(out.record_attempt("no-such-record", None).is_ok());
    }

    #[test]
    fn an_attempt_that_repeats_the_same_reason_changes_nothing() {
        // A record held on a condition that does not change — its source is
        // outside the roots of the session that is running — is visited by
        // every drain pass. Counting each visit rewrote this 0600 file once a
        // minute, and made the status surface say "attempt 1440" after a day:
        // 1440 delivery attempts a reader would go looking for and never
        // find, because only one was ever made.
        let dir = tempfile::TempDir::new().unwrap();
        let out = spool(&dir);
        let id = out.next_id();
        out.enqueue(staged(&id, "nodeA", "iPhone", 10)).unwrap();
        let why = "held, not sent: it is outside this server's send roots".to_string();

        out.record_attempt(&id, Some(why.clone())).unwrap();
        let first = spool(&dir).list()[0].clone();
        assert_eq!(first.attempts, 1);
        let on_disk = std::fs::read(dir.path().join("outbox").join(format!("{id}.json"))).unwrap();

        for _ in 0..3 {
            out.record_attempt(&id, Some(why.clone())).unwrap();
        }
        assert_eq!(out.list()[0], first, "the same answer twice is still one attempt");
        assert_eq!(
            std::fs::read(dir.path().join("outbox").join(format!("{id}.json"))).unwrap(),
            on_disk,
            "and the record file is not rewritten to say so"
        );

        // A reason that really did change is still written, or a record would
        // keep explaining a condition that has passed.
        out.record_attempt(&id, Some("the peer did not answer the handshake".to_string()))
            .unwrap();
        assert_eq!(spool(&dir).list()[0].attempts, 2);
    }

    #[test]
    fn an_expired_record_leaves_and_hands_its_pin_back() {
        let dir = tempfile::TempDir::new().unwrap();
        let out = spool(&dir);
        let id = out.next_id();
        let record = out.enqueue(staged(&id, "nodeA", "iPhone", 10)).unwrap();

        assert!(out.take_expired(record.enqueued_at).is_empty(), "not yet");
        let expired = out.take_expired(record.expires_at);
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].tag, tag_name(&id), "the bytes stop being kept with the record");
        assert!(out.is_empty());
        assert!(spool(&dir).is_empty(), "and it is gone from disk, not just from memory");
    }

    #[test]
    fn a_record_id_is_claimed_loudly_instead_of_overwritten() {
        // The clock can go backwards, and tmp+rename alone would then let a
        // second send silently replace a record the user was told was
        // pending — one file promised, one file gone.
        let dir = tempfile::TempDir::new().unwrap();
        let out = spool(&dir);
        let id = out.next_id();
        out.enqueue(staged(&id, "nodeA", "iPhone", 10)).unwrap();

        let err = out
            .enqueue(staged(&id, "nodeB", "iPad", 20))
            .expect_err("the id is taken");
        assert!(err.contains("refusing to queue"), "the refusal names the cause, got: {err}");
        assert_eq!(depth(&out), 1, "and nothing was added in memory either");
        let survivor = &spool(&dir).list()[0];
        assert_eq!(survivor.peer, "nodeA", "the first record is intact on disk");
        assert_eq!(survivor.size, 10);
    }

    #[test]
    fn two_sends_inside_one_millisecond_get_their_own_ids() {
        let dir = tempfile::TempDir::new().unwrap();
        let out = spool(&dir);
        let ids: Vec<String> = (0..64).map(|_| out.next_id()).collect();
        let unique: std::collections::HashSet<&String> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len(), "the tie-break counter is what makes this true");
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(sorted, ids, "ids sort into the order the sends were accepted in");
    }

    #[test]
    fn a_restart_mints_past_every_id_the_directory_already_holds() {
        // The collision this closes. A record id is `{millis}-{seq}`, and the
        // counter used to restart at 0, so the millisecond stamp was the only
        // thing keeping a restart's first id off the first id of the run
        // before it — and `now_millis` answers 0 for every call on a machine
        // whose clock reports a time before 1970. Every process there minted
        // `0000000000000-0000` first, and the second one's `stage_outbox`
        // took the incumbent's pin: `enqueue` refused the repeated id, and
        // the caller's cleanup then deleted the tag holding the incumbent's
        // bytes. The claim kept the record FILE and lost the copy of the
        // user's file behind it.
        let dir = tempfile::TempDir::new().unwrap();
        let outbox_dir = dir.path().join("outbox");

        // What one run leaves behind. `live_tags` keeps an `outbox/<id>` pin
        // for all three, so all three are ids no later run may mint again —
        // and the two that are not live records are the dangerous half, because
        // neither owns an `<id>.json` for `create_new` to refuse.
        let first = spool(&dir);
        first.enqueue(staged("0000000000000-0000", "nodeA", "iPhone", 10)).unwrap();
        std::fs::write(outbox_dir.join("0000000000000-0007.json"), "{not json").unwrap();
        std::fs::write(outbox_dir.join("0000000000000-0031.json"), r#"{"v":99}"#).unwrap();

        let next = spool(&dir);
        assert_eq!(next.quarantined(), vec!["0000000000000-0007".to_string()]);
        assert_eq!(next.foreign(), vec!["0000000000000-0031".to_string()]);
        for id in (0..4).map(|_| next.next_id()) {
            assert!(
                seq_of(&id).unwrap() > 31,
                "{id} has to sit past every counter already on disk: a clock reporting a \
                 time before 1970 makes the millisecond half identical for every run"
            );
        }

        // `Drainer::reconcile` RELOADS before it decides anything, so a
        // reload has to raise the counter too. A spool opened before another
        // record was written would otherwise mint straight into it.
        std::fs::write(outbox_dir.join("0000000000000-0099.json"), "{not json").unwrap();
        next.reload();
        assert!(seq_of(&next.next_id()).unwrap() > 99, "the reload raises the counter");

        // And never lowers it. A reload after a delivery landed reads a
        // directory that may hold nothing at all, and a counter that followed
        // it back down would re-mint an id whose pin this run has not
        // released yet.
        for entry in std::fs::read_dir(&outbox_dir).unwrap().flatten() {
            std::fs::remove_file(entry.path()).unwrap();
        }
        next.reload();
        assert!(next.is_empty(), "the directory really is empty now");
        assert!(
            seq_of(&next.next_id()).unwrap() > 99,
            "an emptied directory does not hand the counter back"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_record_file_is_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().unwrap();
        let out = spool(&dir);
        let id = out.next_id();
        out.enqueue(staged(&id, "nodeA", "iPhone", 10)).unwrap();
        // It names a file on this disk and a peer allowed to fetch it, so it
        // is in the identity.key class, not the world-readable one.
        let path = dir.path().join("outbox").join(format!("{id}.json"));
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    // ── Corruption policy, split by cause ──────────────────────────────────

    #[test]
    fn an_unreadable_record_is_quarantined_and_loses_no_sibling() {
        let dir = tempfile::TempDir::new().unwrap();
        let out = spool(&dir);
        let good = out.next_id();
        out.enqueue(staged(&good, "nodeA", "iPhone", 10)).unwrap();
        let outbox_dir = dir.path().join("outbox");
        std::fs::write(outbox_dir.join("0000000000001-0099.json"), "{not json").unwrap();

        let reloaded = spool(&dir);
        assert_eq!(reloaded.list().len(), 1, "one bad record poisons one delivery, not the spool");
        assert_eq!(reloaded.list()[0].id, good);
        assert_eq!(reloaded.quarantined(), vec!["0000000000001-0099".to_string()]);
        assert!(reloaded.load_error().is_none(), "one bad file is not an unreadable directory");

        // Moved aside, not deleted, and not re-read on the next load.
        assert!(!outbox_dir.join("0000000000001-0099.json").exists());
        let aside: Vec<PathBuf> = std::fs::read_dir(&outbox_dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.to_string_lossy().contains(".broken."))
            .collect();
        assert_eq!(aside.len(), 1, "quarantined to .broken.<unix>");
        assert_eq!(std::fs::read_to_string(&aside[0]).unwrap(), "{not json");

        // Read again: the file has already been moved, so it is counted from
        // its `.broken.<unix>` name and NOT moved a second time.
        let again = spool(&dir);
        assert_eq!(again.quarantined(), vec!["0000000000001-0099".to_string()]);
        let still: Vec<PathBuf> = std::fs::read_dir(&outbox_dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.to_string_lossy().contains(".broken."))
            .collect();
        assert_eq!(still, aside, "the same one file, under the same name");
    }

    #[test]
    fn a_quarantined_stem_survives_every_later_read_and_keeps_its_pin() {
        // THE STEM IS THE ONLY THING KEEPING THE BYTES. `Drainer::reconcile`
        // reloads the spool and then builds the sweep keep-set from
        // `live_tags`, so a quarantine that only counted on the read that
        // moved the file aside left the stem out of the keep-set on the very
        // next read — and the boot sweep unpinned the staged copy of the one
        // record this build could not account for. The same erasure emptied
        // the `queue_unreadable` list a human hears about it through.
        let dir = tempfile::TempDir::new().unwrap();
        let out = spool(&dir);
        let good = out.next_id();
        out.enqueue(staged(&good, "nodeA", "iPhone", 10)).unwrap();
        std::fs::write(dir.path().join("outbox").join("0000000000001-0099.json"), "{not json")
            .unwrap();

        // Read one moves it aside. Reads two and three see only the moved
        // file, which is exactly what a second process and a later boot see.
        let first = spool(&dir);
        assert_eq!(first.quarantined(), vec!["0000000000001-0099".to_string()]);
        for read in 2..=3 {
            let later = spool(&dir);
            assert_eq!(
                later.quarantined(),
                vec!["0000000000001-0099".to_string()],
                "read {read} must still name the dead delivery"
            );
            let mut keep = later.live_tags().expect("a clean load answers");
            keep.sort();
            let mut expected = vec![tag_name(&good), tag_name("0000000000001-0099")];
            expected.sort();
            assert_eq!(keep, expected, "read {read} must still keep its pin out of the sweep");
        }

        // And `reload` on a spool that is already open answers the same, which
        // is the call `reconcile` actually makes before it sweeps.
        out.reload();
        assert_eq!(out.quarantined(), vec!["0000000000001-0099".to_string()]);
        assert!(out
            .live_tags()
            .expect("a clean load answers")
            .contains(&tag_name("0000000000001-0099")));
    }

    #[test]
    fn a_file_that_only_looks_quarantined_is_not_taken_for_a_record() {
        // The keep-set is a delete list's opposite, so what goes into it must
        // be a name this module wrote. `quarantine` writes exactly one shape,
        // `<id>.json.broken.<unix>`, and anything else in this directory is
        // somebody else's file.
        let dir = tempfile::TempDir::new().unwrap();
        let outbox_dir = dir.path().join("outbox");
        std::fs::create_dir_all(&outbox_dir).unwrap();
        for name in ["notes.json.broken.bak", "notes.broken.12", "notes.json.broken."] {
            std::fs::write(outbox_dir.join(name), "x").unwrap();
        }

        let out = spool(&dir);
        assert!(out.quarantined().is_empty(), "none of those names a record");
        assert_eq!(out.live_tags(), Some(Vec::new()));
    }

    #[test]
    fn a_newer_schema_record_is_left_byte_for_byte_alone() {
        let dir = tempfile::TempDir::new().unwrap();
        let outbox_dir = dir.path().join("outbox");
        std::fs::create_dir_all(&outbox_dir).unwrap();
        let path = outbox_dir.join("0000000000002-0000.json");
        let future = r#"{"v":99,"record":{"id":"0000000000002-0000","peer":"nodeA"}}"#;
        std::fs::write(&path, future).unwrap();

        let out = spool(&dir);
        assert!(out.is_empty(), "a record this build cannot read is not a delivery it can make");
        assert_eq!(out.foreign(), vec!["0000000000002-0000".to_string()]);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), future, "never rewritten, never moved");
    }

    #[test]
    fn a_record_that_will_not_read_is_left_alone_for_a_later_boot() {
        // AN I/O ERROR IS NOT CORRUPTION, and the difference is a delivery.
        // A backup restore that changes the owner, a permissions repair that
        // leaves EACCES, a bad sector that answers EIO: the record behind the
        // file may be perfectly valid, and renaming it to `.broken.<unix>`
        // abandons a promised send over a condition that passes on its own.
        //
        // A DIRECTORY where a record file belongs produces exactly that
        // failure — `read_dir` lists it, `read_to_string` answers EISDIR —
        // without the test having to depend on the uid it runs as.
        let dir = tempfile::TempDir::new().unwrap();
        let out = spool(&dir);
        let good = out.next_id();
        out.enqueue(staged(&good, "nodeA", "iPhone", 10)).unwrap();
        let unreadable = dir.path().join("outbox").join("0000000000007-0000.json");
        std::fs::create_dir(&unreadable).unwrap();

        let reloaded = spool(&dir);
        assert_eq!(depth(&reloaded), 1, "one unreadable file poisons one delivery");
        assert_eq!(reloaded.list()[0].id, good);
        assert!(reloaded.quarantined().is_empty(), "it is not corrupt, so it is not moved aside");
        assert_eq!(reloaded.io_faults(), vec!["0000000000007-0000".to_string()]);
        assert!(reloaded.load_error().is_none(), "one bad file is not an unreadable directory");

        // Untouched, so the boot that can read it still finds it under the
        // name the record calls itself by.
        assert!(unreadable.is_dir(), "left exactly where it was");
        let aside: Vec<PathBuf> = std::fs::read_dir(dir.path().join("outbox"))
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.to_string_lossy().contains(".broken."))
            .collect();
        assert!(aside.is_empty(), "nothing was quarantined: {aside:?}");

        // And its pin stays out of the sweep, or the bytes would go while the
        // record file is still sitting there waiting to be read.
        let keep = reloaded.live_tags().expect("a clean load answers");
        assert!(keep.contains(&tag_name("0000000000007-0000")), "got: {keep:?}");
    }

    #[test]
    fn a_record_that_does_not_match_its_filename_is_quarantined() {
        // `complete` deletes `<id>.json`, so a hand-edited id would delete a
        // sibling's file or fail forever. Neither is a delivery.
        let dir = tempfile::TempDir::new().unwrap();
        let out = spool(&dir);
        let id = out.next_id();
        out.enqueue(staged(&id, "nodeA", "iPhone", 10)).unwrap();
        let path = dir.path().join("outbox").join(format!("{id}.json"));
        let raw = std::fs::read_to_string(&path).unwrap().replace(&id, "somebody-elses-id");
        std::fs::write(&path, raw).unwrap();

        let reloaded = spool(&dir);
        assert!(reloaded.is_empty());
        assert_eq!(reloaded.quarantined(), vec![id]);
    }

    #[test]
    fn an_unreadable_spool_never_lets_the_sweep_unpin_a_live_record() {
        // The directory itself will not read. Its records are HIDDEN, not
        // gone: a sweep run against the empty list it produces would delete
        // every `outbox/*` tag and take every pending file with it, with no
        // error anywhere.
        let dir = tempfile::TempDir::new().unwrap();
        let outbox_dir = dir.path().join("outbox");
        std::fs::create_dir_all(&outbox_dir).unwrap();
        std::fs::write(outbox_dir.join("0000000000003-0000.json"), "not read").unwrap();
        let blocked = Outbox::load(&outbox_dir.join("0000000000003-0000.json"));

        assert!(blocked.load_error().is_some(), "a file where a directory belongs cannot be read");
        assert_eq!(blocked.live_tags(), None, "no keep-set, so no sweep can run");
        let refused = blocked
            .enqueue(staged("0000000000004-0000", "nodeA", "iPhone", 10))
            .expect_err("a spool that did not load must not promise a delivery");
        assert!(refused.contains("did not load"), "the refusal names the cause, got: {refused}");

        // And the other half: a spool that DID load keeps a quarantined stem
        // and a newer-schema stem in the keep-set, because bytes this build
        // cannot account for are the last thing it should unpin.
        let live = tempfile::TempDir::new().unwrap();
        let out = spool(&live);
        let good = out.next_id();
        out.enqueue(staged(&good, "nodeA", "iPhone", 10)).unwrap();
        let live_dir = live.path().join("outbox");
        std::fs::write(live_dir.join("0000000000005-0000.json"), "{not json").unwrap();
        std::fs::write(live_dir.join("0000000000006-0000.json"), r#"{"v":99,"record":{}}"#).unwrap();

        let mut keep = spool(&live).live_tags().expect("a clean load answers");
        keep.sort();
        let mut expected = vec![
            tag_name(&good),
            tag_name("0000000000005-0000"),
            tag_name("0000000000006-0000"),
        ];
        expected.sort();
        assert_eq!(keep, expected);
    }

    // ── Caps ───────────────────────────────────────────────────────────────

    #[test]
    fn a_full_spool_refuses_the_send_instead_of_dropping_the_oldest_file() {
        // The chosen failure mode, stated: a refused send is an error the
        // caller can act on, while a dropped record is a file the user was
        // told would arrive and never does. `MAX_RECEIVED` in the MCP server
        // drops its oldest for the opposite reason — that list is a log.
        let dir = tempfile::TempDir::new().unwrap();
        let out = spool(&dir);
        let oldest = out.next_id();
        out.enqueue(staged(&oldest, "nodeA", "iPhone", 1)).unwrap();
        for _ in 1..MAX_RECORDS {
            let id = out.next_id();
            out.enqueue(staged(&id, "nodeA", "iPhone", 1)).unwrap();
        }

        let id = out.next_id();
        let full = out
            .enqueue(staged(&id, "nodeA", "iPhone", 1))
            .expect_err("the record cap holds");
        assert!(full.contains(&MAX_RECORDS.to_string()), "the refusal names the cap: {full}");
        assert!(full.contains("iPhone"), "and the device: {full}");
        assert_eq!(depth(&out), MAX_RECORDS);
        assert_eq!(out.list()[0].id, oldest, "the oldest record is still there");
        // The pre-staging gate answers the same, so a refused send never
        // costs a copy of the file first.
        assert!(out.room_for("iPhone", 1).is_err());

        // The byte cap is the second ceiling: a queued send is a private full
        // copy of a user file inside the state directory.
        let roomy = tempfile::TempDir::new().unwrap();
        let out = spool(&roomy);
        let big = out.next_id();
        out.enqueue(staged(&big, "nodeA", "iPhone", MAX_SPOOL_BYTES - 1)).unwrap();
        assert!(out.room_for("iPhone", 1).is_ok(), "exactly at the limit still fits");
        let over = out.next_id();
        let err = out
            .enqueue(staged(&over, "nodeA", "iPhone", 2))
            .expect_err("the byte cap holds");
        assert!(err.contains("1024 MiB"), "the refusal names the cap: {err}");
        assert!(err.contains("iPhone"), "and the device: {err}");
        assert_eq!(depth(&out), 1);
    }

    // ── Reload ─────────────────────────────────────────────────────────────

    #[test]
    fn a_reload_replaces_what_another_process_already_finished() {
        // The boot path reloads before it decides anything: a previous
        // process may have completed records after this one read the
        // directory, and re-pushing a delivered file is a duplicate on the
        // user's phone.
        let dir = tempfile::TempDir::new().unwrap();
        let first = spool(&dir);
        let id = first.next_id();
        first.enqueue(staged(&id, "nodeA", "iPhone", 10)).unwrap();

        let second = spool(&dir);
        assert_eq!(depth(&second), 1);
        first.complete(&id).unwrap();
        assert_eq!(depth(&second), 1, "the stale view is still stale");
        second.reload();
        assert!(second.is_empty(), "and the reload is what corrects it");
        assert_eq!(second.live_tags(), Some(Vec::new()));
    }
}
