// The seams between this crate and whatever application hosts it.
//
// The remote stack owns identity, transport, the path gate and the wire
// protocol. It owns NO user interface and NO session truth: what is open,
// starred and recent lives in the app, and what a remote event should DO is
// the app's decision. Both cross this file.
//
// The desktop app implements `EventSink` with Tauri emits and `HostCatalog`
// with its bookmarks/recents stores; a headless host (the MCP server)
// implements the same two traits with its own handling — and gets the same
// gate, the same scope filter and the same peer-locked grants for free.

use std::path::PathBuf;

use crate::peers::PendingPair;

/// Something the host side needs the app shell to do. Keeps every UI toolkit
/// out of this crate: the app installs a sink that turns each signal into the
/// event (or log line, or IPC message) it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostSignal {
    /// A pairing reached the fingerprint step; the local user must confirm.
    PairPending(PendingPair),
    /// A control-scoped peer asked this machine to open an artifact. The
    /// sink emits the SAME open-file event a deep link produces — a control
    /// peer inherits the deep-link posture and can do nothing beyond it.
    OpenOnHost { peer: String, path: PathBuf, reader_mode: bool },
    /// A control-scoped peer pushed an artifact and the verified bytes landed
    /// under `received/`. Identical in every way to a Beam receive that the
    /// local user accepted — same folder, same verification, same cap — so
    /// the sink surfaces it the same way (the desktop opens it in a tab).
    ArtifactReceived {
        peer: String,
        path: PathBuf,
        name: String,
        size: u64,
        /// BLAKE3 content address, hex.
        hash: String,
    },
    /// A paired peer finished the handshake and holds a live session here.
    /// Emitted once the `HelloAck` is on its way, so it means "this peer is
    /// reachable RIGHT NOW" rather than "this peer knocked": the allowlist,
    /// the version check and the revocation window have all already run.
    ///
    /// A host with a send queue is what this exists for. A device that dials
    /// IN has just proved it is awake and on a network, which is the one fact
    /// a send waiting for that device needs, and learning it costs no wire
    /// byte and no extra dial. A host with nothing queued has nothing to do
    /// with it, which is why it is a signal and not a return value.
    PeerConnected {
        peer: String,
        device: String,
        /// What THIS machine grants that peer — the scope it was just told in
        /// the `HelloAck`. It is NOT what the peer grants this machine: that
        /// is the opposite direction and can only be read from an ack this
        /// side RECEIVES, so a sender must never cache this value as one.
        scope: String,
    },
}

/// Where host signals go. Implemented by the app.
///
/// A plain closure is a sink, so the app can pass `move |signal| …` and a test
/// can pass `|_| {}` — the trait exists so a headless host can hold state
/// (a channel, a queue, an MCP notification stream) instead of a closure.
pub trait EventSink: Send + Sync + 'static {
    fn emit(&self, signal: HostSignal);
}

impl<F> EventSink for F
where
    F: Fn(HostSignal) + Send + Sync + 'static,
{
    fn emit(&self, signal: HostSignal) {
        self(signal)
    }
}

/// The host's session catalog: the artifacts this machine considers starred
/// and recently opened. A view-open peer may fetch exactly what this reports
/// (plus the published tabs), so a host that reports nothing narrows the
/// remote surface to nothing — it can never widen it, because every entry
/// still passes the RootSet gate before it reaches the wire.
pub trait HostCatalog: Send + Sync + 'static {
    fn bookmarks(&self) -> Vec<PathBuf>;
    fn recents(&self) -> Vec<PathBuf>;
}

/// A catalog with nothing in it — the default for a headless host that has no
/// bookmarks or recents of its own.
#[derive(Debug, Clone, Copy, Default)]
pub struct EmptyCatalog;

impl HostCatalog for EmptyCatalog {
    fn bookmarks(&self) -> Vec<PathBuf> {
        Vec::new()
    }
    fn recents(&self) -> Vec<PathBuf> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn a_closure_is_an_event_sink() {
        let seen: Arc<Mutex<Vec<HostSignal>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = {
            let seen = seen.clone();
            move |signal: HostSignal| seen.lock().unwrap().push(signal)
        };
        let signal = HostSignal::OpenOnHost {
            peer: "nodeA".into(),
            path: PathBuf::from("/w/a.html"),
            reader_mode: true,
        };
        EventSink::emit(&sink, signal.clone());
        assert_eq!(seen.lock().unwrap().as_slice(), &[signal]);
    }

    #[test]
    fn a_struct_sink_can_hold_state_instead_of_capturing_it() {
        // The headless shape: an implementor with its own queue.
        struct Queue(Mutex<Vec<HostSignal>>);
        impl EventSink for Queue {
            fn emit(&self, signal: HostSignal) {
                self.0.lock().unwrap().push(signal);
            }
        }
        let q = Queue(Mutex::new(Vec::new()));
        q.emit(HostSignal::ArtifactReceived {
            peer: "nodeA".into(),
            path: PathBuf::from("/state/received/2026-08-29/a.html"),
            name: "a.html".into(),
            size: 3,
            hash: "ab".repeat(32),
        });
        assert_eq!(q.0.lock().unwrap().len(), 1);
    }

    #[test]
    fn the_empty_catalog_reports_nothing() {
        assert!(EmptyCatalog.bookmarks().is_empty());
        assert!(EmptyCatalog.recents().is_empty());
    }
}
