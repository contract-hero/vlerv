// M1 proof: in-process two-endpoint Beam transfer. A sender node offers a
// file, a receiver node dials the minted ticket and lands the verified blob
// under received/<date>/; Stop revokes the offer for subsequent fetches.
//
// Hermetic by construction: the receiver dials a loopback re-mint of the
// offer's ticket (same node id, same hash, 127.0.0.1 + the sender's real
// UDP port). The endpoint binds 0.0.0.0, so loopback always reaches it —
// no relay, no discovery, no external network in the loop. Cross-network
// traversal is the M0 spike's territory, on real machines.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use iroh::{EndpointAddr, TransportAddr};
use iroh_blobs::ticket::BlobTicket;
use src_tauri::remote::{beam, endpoint};

/// Re-mint `ticket` so its only transport addr is `127.0.0.1:<sender port>`.
fn loopback_ticket(ticket: &str) -> String {
    let ticket: BlobTicket = ticket.parse().expect("offer mints a valid ticket");
    let port = ticket
        .addr()
        .ip_addrs()
        .find(|a| a.is_ipv4())
        .expect("offer ticket carries an IPv4 direct addr")
        .port();
    let addr = EndpointAddr::from_parts(
        ticket.addr().id,
        [TransportAddr::Ip((std::net::Ipv4Addr::LOCALHOST, port).into())],
    );
    BlobTicket::new(addr, ticket.hash(), ticket.format()).to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn beam_round_trip_then_stop_revokes() {
    let sender_dir = tempfile::TempDir::new().unwrap();
    let receiver_dir = tempfile::TempDir::new().unwrap();
    let received_root = tempfile::TempDir::new().unwrap();

    let sender = endpoint::boot_in(sender_dir.path(), || {}).await.expect("sender boot");
    let receiver = endpoint::boot_in(receiver_dir.path(), || {}).await.expect("receiver boot");

    // Stage + offer on the sender.
    let artifact = sender_dir.path().join("report.html");
    let body = "<!doctype html><h1>beamed</h1>".repeat(64);
    std::fs::write(&artifact, &body).unwrap();
    let offer = beam::offer(&sender, &artifact.canonicalize().unwrap())
        .await
        .expect("offer");
    assert_eq!(offer.name, "report.html");
    assert_eq!(offer.size, body.len() as u64);
    assert!(offer.link.starts_with("vlerv://receive?ticket="));
    assert_eq!(sender.offers.list().len(), 1);

    let dial_ticket = loopback_ticket(&offer.ticket);

    // Fetch on the receiver, counting progress callbacks.
    let progress_calls = Arc::new(AtomicU64::new(0));
    let calls = progress_calls.clone();
    let received = beam::receive(
        &receiver,
        &dial_ticket,
        Some(&offer.name),
        Some(offer.size),
        received_root.path(),
        move |_hash, _received, _total| {
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
        Some(offer.size),
        received_root.path(),
        |_, _, _| {},
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
        None,
        received_root.path(),
        |_, _, _| {},
    )
    .await;
    assert!(denied.is_err(), "revoked offer must not be fetchable");

    receiver.router.shutdown().await.ok();
    sender.router.shutdown().await.ok();
}
