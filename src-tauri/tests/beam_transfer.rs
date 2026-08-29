// M1 proof: in-process two-endpoint Beam transfer. A sender node offers a
// file, a receiver node dials the minted ticket and lands the verified blob
// under received/<date>/; Stop revokes the offer for subsequent fetches.
//
// The DATA PATH is loopback: the receiver dials a re-mint of the offer's
// ticket (same node id, same hash, sole transport addr 127.0.0.1 + the
// sender's real UDP port). The endpoint binds 0.0.0.0, so loopback always
// reaches it — no relay hop and no discovery lookup carry the bytes. The
// endpoints still boot the n0 preset, so bind publishes to n0 DNS and
// `offer()` waits up to 10 s on `online()` when relays are unreachable.
// Cross-network NAT traversal is the M0 spike's territory, on real machines.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use iroh_blobs::ticket::BlobTicket;
use src_tauri::remote::{beam, endpoint, Dirs};
use src_tauri::security::RootSet;

/// Re-mint `ticket` so its only transport addr is `127.0.0.1:<sender port>`.
fn loopback_ticket(ticket: &str) -> String {
    let ticket: BlobTicket = ticket.parse().expect("offer mints a valid ticket");
    let port = ticket
        .addr()
        .ip_addrs()
        .find(|a| a.is_ipv4())
        .expect("offer ticket carries an IPv4 direct addr")
        .port();
    let addr = endpoint::addr_at_id(
        ticket.addr().id,
        (std::net::Ipv4Addr::LOCALHOST, port).into(),
    );
    BlobTicket::new(addr, ticket.hash(), ticket.format()).to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn beam_round_trip_then_stop_revokes() {
    let sender_dir = tempfile::TempDir::new().unwrap();
    let receiver_dir = tempfile::TempDir::new().unwrap();
    let received_root = tempfile::TempDir::new().unwrap();

    // `None` scope state: these two nodes speak Beam only, so the router
    // answers the blobs ALPN and nothing else. Each node gets its own base
    // dir — the crate derives identity, blobs and received/ from it and
    // hardcodes nothing.
    let sender = endpoint::boot(&Dirs::new(sender_dir.path()), None, |_| {})
        .await
        .expect("sender boot");
    let receiver = endpoint::boot(&Dirs::new(receiver_dir.path()), None, |_| {})
        .await
        .expect("receiver boot");

    // Stage + offer on the sender, through the same path policy the
    // commands use.
    let artifact = sender_dir.path().join("report.html");
    let body = "<!doctype html><h1>beamed</h1>".repeat(64);
    std::fs::write(&artifact, &body).unwrap();
    let roots = RootSet::new(vec![sender_dir.path().to_path_buf()]);
    let cand = beam::resolve_offerable(&artifact, &roots).expect("offerable");
    let offer = beam::offer(&sender, &cand, beam::DEFAULT_TTL_HOURS)
        .await
        .expect("offer");
    assert_eq!(offer.name, "report.html");
    assert_eq!(offer.size, body.len() as u64);
    assert!(offer.link.starts_with("vlerv://receive?ticket="));
    assert_eq!(sender.offers.list().len(), 1);
    // Default TTL (24 h) is applied.
    assert_eq!(offer.expires_at - offer.created_at, 24 * 3600);

    // The product loop: the minted link must survive the app's OWN parser —
    // ticket, sanitized name, and size all round-trip. A charset drift
    // between build_link and the receive arm would break this before it broke
    // on a real two-machine paste.
    match src_tauri::deeplink::parse(&offer.link).expect("own link re-parses") {
        src_tauri::deeplink::DeepLinkIntent::Receive { ticket, name, size } => {
            assert_eq!(ticket, offer.ticket);
            assert_eq!(name.as_deref(), Some("report.html"));
            assert_eq!(size, Some(offer.size));
        }
        other => panic!("expected Receive, got {other:?}"),
    }

    let dial_ticket = loopback_ticket(&offer.ticket);

    // Fetch on the receiver, counting progress callbacks.
    let progress_calls = Arc::new(AtomicU64::new(0));
    let calls = progress_calls.clone();
    let received = beam::receive(
        &receiver,
        &dial_ticket,
        Some(&offer.name),
        received_root.path(),
        move |_hash, _received| {
            calls.fetch_add(1, Ordering::SeqCst);
        },
    )
    .await
    .expect("receive");

    // Verified content landed under received/<date>/report.html.
    assert_eq!(std::fs::read_to_string(&received.path).unwrap(), body);
    assert_eq!(received.name, "report.html");
    assert_eq!(received.size, body.len() as u64);
    let day_dir = received.path.parent().unwrap().file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(day_dir.len(), "2026-08-15".len(), "lands in a date directory: {day_dir}");
    assert!(progress_calls.load(Ordering::SeqCst) >= 1, "final progress callback fires");

    // The gate counted the fetch.
    assert_eq!(sender.offers.list()[0].fetches, 1);

    // A second fetch of the same content gets a fresh, non-colliding name.
    let again = beam::receive(
        &receiver,
        &dial_ticket,
        Some(&offer.name),
        received_root.path(),
        |_, _| {},
    )
    .await
    .expect("second receive");
    assert_eq!(again.name, "report.html");
    assert_eq!(
        again.path.file_name().unwrap().to_str().unwrap(),
        "report-2.html",
        "collision appends a counter"
    );

    // Stop revokes instantly: the connection may open, but the request dies
    // at the gate.
    beam::stop(&sender, &offer.id).await;
    assert!(sender.offers.list().is_empty());
    let denied = beam::receive(
        &receiver,
        &dial_ticket,
        None,
        received_root.path(),
        |_, _| {},
    )
    .await;
    assert!(denied.is_err(), "revoked offer must not be fetchable");

    // The revoked receive left no orphan in .partial/ (finding: post-abort
    // cleanup). The gate denies before any file is created, but assert the
    // dir is clean regardless.
    let partial = received_root.path().join(".partial");
    let leftover = std::fs::read_dir(&partial)
        .map(|d| d.flatten().count())
        .unwrap_or(0);
    assert_eq!(leftover, 0, "no partial files left after a denied receive");

    receiver.router.shutdown().await.ok();
    sender.router.shutdown().await.ok();
}
