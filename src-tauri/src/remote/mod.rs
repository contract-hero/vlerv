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

use tauri::{Emitter, Manager};

use crate::security::{self, RootSet};

/// Managed Tauri state holding the lazily booted remote node.
#[derive(Default)]
pub struct RemoteState {
    node: tokio::sync::Mutex<Option<Arc<endpoint::RemoteNode>>>,
}

impl RemoteState {
    /// Get the booted node, booting it on first use. The `on_offers_change`
    /// callback is installed once, at boot, and fires whenever the request
    /// gate mutates the offers registry (fetch counts, expiry denials).
    async fn node(
        &self,
        on_offers_change: impl Fn() + Send + Sync + 'static,
    ) -> Result<Arc<endpoint::RemoteNode>, String> {
        let mut guard = self.node.lock().await;
        if let Some(node) = guard.as_ref() {
            return Ok(node.clone());
        }
        let node = Arc::new(endpoint::boot(on_offers_change).await?);
        *guard = Some(node.clone());
        Ok(node)
    }
}

fn offers_changed(app: &tauri::AppHandle, node: &endpoint::RemoteNode) {
    let _ = app.emit("vlerv://beam-offers-updated", node.offers.list());
}

/// Stage a file into the blob store, mint a ticket, and register the offer.
/// Path policy is the share module's: out-of-root files that resolve are
/// beamable on purpose; an empty root set stays conservative (beam sends
/// data off the machine).
#[tauri::command]
pub async fn beam_offer(
    app: tauri::AppHandle,
    state: tauri::State<'_, RemoteState>,
    roots: tauri::State<'_, RootSet>,
    path: String,
) -> Result<beam::OfferInfo, String> {
    let (canonical, _out_of_root) =
        security::canonicalize_allow_external(std::path::Path::new(&path), &roots)
            // Same no-existence-leak wording as the share module.
            .map_err(|_| "path not found or out of root".to_string())?;

    let node = boot_node(&app, &state).await?;
    let info = beam::offer(&node, &canonical).await?;
    offers_changed(&app, &node);
    Ok(info)
}

/// Revoke an active offer. The ticket dies with the offer: the request gate
/// consults the registry per request, so the next fetch is denied even if
/// the blob bytes are still in the store.
#[tauri::command]
pub async fn beam_stop(
    app: tauri::AppHandle,
    state: tauri::State<'_, RemoteState>,
    offer_id: String,
) -> Result<(), String> {
    let node = boot_node(&app, &state).await?;
    beam::stop(&node, &offer_id).await;
    offers_changed(&app, &node);
    Ok(())
}

/// Active (unexpired) offers for the "beaming" indicator.
#[tauri::command]
pub async fn beam_list_offers(
    state: tauri::State<'_, RemoteState>,
) -> Result<Vec<beam::OfferInfo>, String> {
    let guard = state.node.lock().await;
    // No node yet → no offers, and listing must NOT boot the endpoint.
    Ok(guard.as_ref().map(|n| n.offers.list()).unwrap_or_default())
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
        move |hash, received, total| {
            let _ = progress_app.emit(
                "vlerv://beam-progress",
                beam::ProgressEvent { hash: hash.to_string(), received, total },
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
    state
        .node(move || {
            let app = app.clone();
            // Fire-and-forget: hop onto the async runtime to read the
            // registry without blocking the gate loop.
            tauri::async_runtime::spawn(async move {
                let state = app.state::<RemoteState>();
                let guard = state.node.lock().await;
                if let Some(node) = guard.as_ref() {
                    let _ = app.emit("vlerv://beam-offers-updated", node.offers.list());
                }
            });
        })
        .await
}
