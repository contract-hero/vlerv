// Remote subsystem — Beam (v1 of remote-control-design.html): one-shot,
// content-addressed artifact transfer between Vlervcode instances over iroh.
//
// Everything networked lives behind this module boundary. The webview gets
// Tauri commands, never sockets; iroh types stay quarantined in endpoint.rs
// and beam.rs so version upgrades are deliberate migrations.
//
// Lazy-boot contract (design §2): the app makes ZERO network connections
// until the user invokes a beam action. `RemoteState.node` starts empty and
// is populated on the first `beam_offer` / `beam_receive` call.

pub mod beam;
pub mod endpoint;

use std::sync::Arc;

use tauri::Emitter;

use crate::security::RootSet;

/// Managed Tauri state holding the lazily booted remote node.
#[derive(Default)]
pub struct RemoteState {
    node: tokio::sync::Mutex<Option<Arc<endpoint::RemoteNode>>>,
}

impl RemoteState {
    /// Get the booted node, booting it on first use. The `on_offers_change`
    /// callback is installed once, at boot, and fires with the fresh offers
    /// list whenever the request gate mutates the registry (fetch counts).
    async fn node(
        &self,
        on_offers_change: impl Fn(Vec<beam::OfferInfo>) + Send + Sync + 'static,
    ) -> Result<Arc<endpoint::RemoteNode>, String> {
        let mut guard = self.node.lock().await;
        if let Some(node) = guard.as_ref() {
            return Ok(node.clone());
        }
        let node = Arc::new(endpoint::boot(on_offers_change).await?);
        *guard = Some(node.clone());
        Ok(node)
    }

    /// Peek the node without booting — for commands where "no node yet"
    /// means "nothing to do" (listing, revoking). Booting sockets to answer
    /// a guaranteed no-op would break the lazy-boot contract.
    async fn existing(&self) -> Option<Arc<endpoint::RemoteNode>> {
        self.node.lock().await.clone()
    }
}

fn offers_changed(app: &tauri::AppHandle, node: &endpoint::RemoteNode) {
    let _ = app.emit("vlerv://beam-offers-updated", node.offers.list());
}

/// Stage a file into the blob store, mint a ticket, and register the offer.
/// Path policy lives in `beam::resolve_offerable`, shared with the
/// `vlerv://beam` dispatch arm: conservative share gate, files only, hard
/// cap — rechecked here at confirm time.
#[tauri::command]
pub async fn beam_offer(
    app: tauri::AppHandle,
    state: tauri::State<'_, RemoteState>,
    roots: tauri::State<'_, RootSet>,
    path: String,
) -> Result<beam::OfferInfo, String> {
    let cand = beam::resolve_offerable(std::path::Path::new(&path), &roots)?;
    let ttl_hours = crate::state_store::current_state()
        .preferences
        .beam_ttl_hours
        .unwrap_or(beam::DEFAULT_TTL_HOURS);

    let node = boot_node(&app, &state).await?;
    let info = beam::offer(&node, &cand, ttl_hours).await?;
    offers_changed(&app, &node);
    Ok(info)
}

/// Revoke an active offer. The ticket dies with the offer: the request gate
/// consults the registry per request, so the next fetch is denied even if
/// the blob bytes are still in the store. Never boots — with no node there
/// is nothing to revoke.
#[tauri::command]
pub async fn beam_stop(
    app: tauri::AppHandle,
    state: tauri::State<'_, RemoteState>,
    offer_id: String,
) -> Result<(), String> {
    let Some(node) = state.existing().await else {
        return Ok(());
    };
    beam::stop(&node, &offer_id).await;
    offers_changed(&app, &node);
    Ok(())
}

/// Active (unexpired) offers for the "beaming" indicator. Never boots.
#[tauri::command]
pub async fn beam_list_offers(
    state: tauri::State<'_, RemoteState>,
) -> Result<Vec<beam::OfferInfo>, String> {
    Ok(state.existing().await.map(|n| n.offers.list()).unwrap_or_default())
}

/// Post-confirm fetch: dial the ticket, stream the BLAKE3-verified blob, and
/// land it under `received/<date>/`. Progress goes out as
/// `vlerv://beam-progress` events keyed by the ticket's hash.
#[tauri::command]
pub async fn beam_receive(
    app: tauri::AppHandle,
    state: tauri::State<'_, RemoteState>,
    ticket: String,
    name: Option<String>,
    size: Option<u64>,
) -> Result<beam::ReceivedFile, String> {
    let node = boot_node(&app, &state).await?;
    let progress_app = app.clone();
    beam::receive(
        &node,
        &ticket,
        name.as_deref(),
        size,
        &endpoint::received_dir(),
        move |hash_hex, received, total| {
            let _ = progress_app.emit(
                "vlerv://beam-progress",
                beam::ProgressEvent { hash: hash_hex.to_string(), received, total },
            );
        },
    )
    .await
}

/// Where received artifacts land — the frontend uses this prefix to swap the
/// "external" badge for a "beamed" one.
#[tauri::command]
pub fn beam_received_dir() -> String {
    endpoint::received_dir().to_string_lossy().into_owned()
}

/// Past beams, newest first, for the "Received" list.
#[tauri::command]
pub fn beam_list_received() -> Vec<beam::ReceivedEntry> {
    beam::list_received(&endpoint::received_dir())
}

async fn boot_node(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, RemoteState>,
) -> Result<Arc<endpoint::RemoteNode>, String> {
    let app = app.clone();
    // The gate hands the fresh offers list straight to the callback — one
    // emit path, no locks touched from the gate loop.
    state
        .node(move |offers| {
            let _ = app.emit("vlerv://beam-offers-updated", offers);
        })
        .await
}
