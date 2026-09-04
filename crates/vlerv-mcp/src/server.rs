// The MCP surface: nine tools over `McpCore`, described for a language model
// rather than for a person reading a manual.
//
// Every handler is the same three steps — validate, call the core, render —
// so the interesting behavior stays in `core.rs` where the integration test
// can reach it without a transport.
//
// Failure convention, per the MCP spec's two failure modes:
//   * a malformed ARGUMENT is a protocol error (`Err(ErrorData)`) — the caller
//     sent something this server cannot route;
//   * a device that is offline, unpaired, or has not granted control is a
//     TOOL error (`Ok(CallToolResult::error(…))`) — the call was valid, it
//     just did not work, and the model must read the reason to fix it.

use std::sync::Arc;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo,
};
use rmcp::{tool, tool_handler, tool_router, ErrorData, ServerHandler};
use serde::Serialize;

// One byte formatter for the whole subsystem: the size beside a link here and
// the size in the crate's own "file is X — beam v1 caps at Y" refusal must
// read the same, and a second formatter is how they stop matching.
use vlerv_remote::beam::human_bytes;

use crate::args::{
    BeamArtifactArgs, ConfirmPairingArgs, ForgetDeviceArgs, ListDevicesArgs, SendToDeviceArgs,
    StopBeamArgs,
};
use crate::core::{Delivery, Forgotten, McpCore, ServerStatus};

/// The rmcp handler. Holds the core behind an `Arc` because rmcp clones the
/// service per connection.
#[derive(Clone)]
pub struct VlervMcp {
    core: Arc<McpCore>,
}

impl VlervMcp {
    pub fn new(core: Arc<McpCore>) -> Self {
        Self { core }
    }
}

#[tool_router]
impl VlervMcp {
    #[tool(
        name = "beam_artifact",
        description = "Publish a local file as a one-time vlerv:// link that a person can open on \
                       any Mac running Vlervtifacts, or on Vlervcode for iOS. Use this when the \
                       user wants to SHARE a file (report, chart, HTML page) and no specific \
                       device is named, or when the receiving device is not paired with this \
                       server. The file is served peer-to-peer, end-to-end encrypted, straight \
                       from this machine — nothing is uploaded anywhere — so the link works only \
                       while this MCP server keeps running and only until it expires. Give the \
                       returned link to the user."
    )]
    async fn beam_artifact(
        &self,
        Parameters(args): Parameters<BeamArtifactArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        render(self.core.beam_artifact(&args.path, args.ttl_hours).await, |link| {
            format!(
                "{} ({}) is now beamable. Give the user this link:\n{}\nIt expires in {}, and it \
                 stops working when this MCP server exits.",
                link.name,
                human_bytes(link.size),
                link.link,
                human_hours(link.expires_at)
            )
        })
    }

    #[tool(
        name = "stop_beam",
        description = "Revoke a link minted by beam_artifact, immediately, before it expires. A \
                       beam link is a capability: anyone who holds the string can fetch the file \
                       until the link expires. Call this as soon as a link went to the wrong \
                       person, named the wrong file, or is simply finished with — the next fetch \
                       is then refused. Pass the hash of one link (beam_artifact and \
                       server_status both report it), or no argument at all to revoke every link \
                       this server is still serving."
    )]
    async fn stop_beam(
        &self,
        Parameters(args): Parameters<StopBeamArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let stopped = match self.core.stop_beam(args.hash.as_deref()).await {
            Ok(stopped) => stopped,
            Err(message) => return tool_failure(message),
        };
        let summary = if stopped.is_empty() {
            "No beam link was live, so nothing had to be revoked.".to_string()
        } else {
            let names: Vec<&str> = stopped.iter().map(|o| o.name.as_str()).collect();
            format!(
                "Revoked {} beam link(s): {}. Any further fetch is refused, even from somebody \
                 who still holds the link.",
                stopped.len(),
                names.join(", ")
            )
        };
        ok(summary, &serde_json::json!({ "stopped": stopped }))
    }

    #[tool(
        name = "list_devices",
        description = "List the devices paired with this MCP server, with their names, node ids \
                       and the scope each one granted. Call this before send_to_device to learn \
                       the exact device names, or when the user asks which devices are available. \
                       Presence is reported as \"unknown\" unless probe is true, which dials each \
                       device once. A probed device reads \"online\", \"offline\" (it did not \
                       answer), or \"refused\" — it answered and turned this server away, which \
                       means it no longer lists this server as a paired peer. Never tell the \
                       user to check the network of a device reported \"refused\"."
    )]
    async fn list_devices(
        &self,
        Parameters(args): Parameters<ListDevicesArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let devices = match self.core.list_devices(args.probe.unwrap_or(false)).await {
            Ok(devices) => devices,
            Err(e) => return tool_failure(e),
        };
        let summary = if devices.is_empty() {
            "No devices are paired with this server yet. Call pair_device to pair one.".to_string()
        } else {
            let rows: Vec<String> = devices
                .iter()
                .map(|d| {
                    format!(
                        "- {} ({}) — scope granted to this server's peer: {}, presence: {}",
                        d.device, d.node_id_short, d.scope, d.presence
                    )
                })
                .collect();
            format!("{} paired device(s):\n{}", devices.len(), rows.join("\n"))
        };
        ok(summary, &serde_json::json!({ "devices": devices }))
    }

    #[tool(
        name = "send_to_device",
        description = "Send a local file straight to one paired device, with no link and no \
                       action needed from the person holding it — the file lands on that device \
                       and opens there. Use this when the user names a destination (\"send it to \
                       my phone\", \"push this to the Mac Studio\"). The device argument matches a \
                       device name or a node-id prefix; call list_devices first if you are not \
                       sure of the name. A device that is asleep or off the network does not \
                       fail the call: the file is COPIED as it is now and queued, the result \
                       says status \"queued\" instead of \"delivered\", and it goes out when \
                       that device comes back — in this session if it is still running, \
                       otherwise at the first network-touching tool call of a later session \
                       over the same state directory. Read the status field and tell the user \
                       which of the two happened — a queued file is not on their device yet. \
                       This only works when the target device granted this server the \
                       \"control\" scope; the error text says so when it has not."
    )]
    async fn send_to_device(
        &self,
        Parameters(args): Parameters<SendToDeviceArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        render(self.core.send_to_device(&args.path, &args.device).await, |delivery| {
            delivery_summary(&args.path, delivery)
        })
    }

    #[tool(
        name = "pair_device",
        description = "Begin pairing a new device with this MCP server. Returns a vlerv://pair \
                       link for the human to open on the device they want to pair. Pairing is \
                       mutual and needs a person: after the device opens the link, six words \
                       appear on BOTH screens, the human compares them, and confirm_pairing \
                       finishes it. Show the link and the instructions to the user, then call \
                       pair_status."
    )]
    async fn pair_device(&self) -> Result<CallToolResult, ErrorData> {
        render(self.core.pair_device().await, |invite| {
            format!(
                "Pairing is open for 10 minutes. Ask the user to open this link on the device \
                 they want to pair:\n{}\n\n{}\n\nNext: {}",
                invite.link,
                invite.instructions.join("\n"),
                invite.fingerprint_hint
            )
        })
    }

    #[tool(
        name = "pair_status",
        description = "Show pairings waiting for confirmation, each with the six fingerprint \
                       words. ALWAYS show these words to the user verbatim and ask them to check \
                       that the same six words, in the same order, are on the other device's \
                       screen. If the words differ, a machine is in the middle: call \
                       confirm_pairing with accept false."
    )]
    async fn pair_status(&self) -> Result<CallToolResult, ErrorData> {
        let pending = self.core.pair_status();
        let summary = if pending.is_empty() {
            "No pairing is waiting for confirmation. Call pair_device, then have the user open \
             the link on the other device."
                .to_string()
        } else {
            let rows: Vec<String> = pending
                .iter()
                .map(|p| {
                    format!(
                        "- {} ({}) — fingerprint: {}",
                        p.device,
                        p.node_id_short,
                        p.fingerprint.join(" ")
                    )
                })
                .collect();
            format!(
                "{} pairing(s) waiting. Read the six words to the user and have them compare with \
                 the other screen:\n{}",
                pending.len(),
                rows.join("\n")
            )
        };
        ok(summary, &serde_json::json!({ "pending": pending }))
    }

    #[tool(
        name = "confirm_pairing",
        description = "Finish or reject a pending pairing after the human confirmed that the six \
                       fingerprint words match on both screens. Never call this with accept true \
                       before the user has actually compared the words. The optional scope \
                       argument says what the NEW DEVICE may do on this server (\"view-open\", \
                       \"browse\", \"control\"; a device new to this server defaults to \
                       \"view-open\"). For a device that is ALREADY paired, naming a scope \
                       replaces its grant, including narrowing it, and omitting the argument \
                       keeps the grant it already has. It does not decide \
                       what this server may do on the device — for send_to_device to work, the \
                       device must grant this server \"control\" on its own side."
    )]
    async fn confirm_pairing(
        &self,
        Parameters(args): Parameters<ConfirmPairingArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        render(
            self.core.confirm_pairing(args.accept, args.node_id.as_deref(), args.scope.as_deref()),
            |outcome| {
                if !outcome.paired {
                    return format!(
                        "Pairing with {} was rejected and nothing was written to disk.",
                        outcome.device
                    );
                }
                format!(
                    "Paired with {}. It may do \"{}\" on this server. For this server to push \
                     files TO it, that device must grant \"{}\" the \"control\" scope in its own \
                     peer settings — otherwise send_to_device is refused.",
                    outcome.device,
                    outcome.scope.clone().unwrap_or_default(),
                    self.core.device()
                )
            },
        )
    }

    #[tool(
        name = "forget_device",
        description = "Unpair one device from this server and delete everything the server was \
                       keeping for it. Use it when the user says a device is no longer theirs, \
                       when list_devices reports a device as \"refused\" (that device already \
                       removed this server, and this is how the two sides agree again), or when \
                       a queued send should simply stop being kept. It removes the pairing, so \
                       that device can no longer reach this server, and it DELETES the private \
                       copies of any files queued for it — those sends never arrive. Say both \
                       things to the user, and prefer asking before calling it: nothing here can \
                       be undone except by pairing again."
    )]
    async fn forget_device(
        &self,
        Parameters(args): Parameters<ForgetDeviceArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        render(self.core.forget_device(&args.device).await, forget_summary)
    }

    #[tool(
        name = "server_status",
        description = "Report this MCP server's own identity and state: its node id, where its \
                       identity and peer list live on disk, whether it has opened a network \
                       connection yet, how long it has been running, which beam links are still \
                       being served, which files other devices pushed to it, and which sends are \
                       queued for a device that was not reachable. Use it to diagnose a failed \
                       send, to tell the user which links are still live, or to answer \"did my \
                       file get there yet\" — a queued entry has not."
    )]
    async fn server_status(&self) -> Result<CallToolResult, ErrorData> {
        render(self.core.server_status().await, status_summary)
    }
}

/// The sentence `forget_device` returns. Both halves are stated because both
/// are irreversible and only one of them is the thing the user asked for: the
/// unpairing was requested, and the deleted copies are what it cost. A
/// summary that reported only the unpairing would let a model tell the user
/// their queued file is still on its way.
fn forget_summary(forgotten: &Forgotten) -> String {
    let mut summary = format!(
        "{} ({}) is no longer paired with this server. It can no longer reach this server, and \
         this server can no longer send to it.",
        forgotten.device, forgotten.node_id_short
    );
    if forgotten.dropped > 0 {
        summary.push_str(&format!(
            " {} queued send(s) were deleted with it, freeing {} — those files will NOT arrive on \
             that device. Tell the user.",
            forgotten.dropped,
            human_bytes(forgotten.dropped_bytes)
        ));
    }
    if let Some(note) = &forgotten.note {
        summary.push('\n');
        summary.push_str(note);
    }
    summary
}

/// The sentence `send_to_device` returns, one per outcome. A free function
/// beside `status_summary` and for the same reason: both sentences have to be
/// assertable without a device on the far end of a socket.
///
/// The VARIANT picks the verb. Nothing here may read "delivered" for a file
/// that is still on this machine — that is the original silent failure told
/// in a friendlier voice, and the type is what makes it impossible.
fn delivery_summary(path: &str, delivery: &Delivery) -> String {
    match delivery {
        Delivery::Delivered { device, name, size, .. } => format!(
            "Delivered {path} ({}) to {device}. It landed there as \"{name}\" and opened on \
             that device.",
            human_bytes(*size)
        ),
        Delivery::Queued { device, name, size, reason, notes, .. } => format!(
            "NOT delivered yet — {device} did not answer ({reason}). {path} ({}) is queued as \
             \"{name}\" and goes out the moment that device is reachable again. Tell the user \
             it has not arrived on their device.\n{}",
            human_bytes(*size),
            notes.join("\n")
        ),
    }
}

/// The sentence `server_status` returns. A free function, not a closure, so a
/// test can assert the truncation clause — the whole reason `received_total`
/// exists — without booting a server.
fn status_summary(status: &ServerStatus) -> String {
    // Say so when the list is shorter than the count, rather than letting the
    // reader take the listed entries for all of them.
    let listed = shortened_clause(status.received_total, status.received_artifacts.len());
    // "booted: false" reads as "idle" unless the refusal is named beside it.
    let booted = match &status.boot_error {
        Some(e) => format!("false — the last boot failed: {e}"),
        None => status.booted.to_string(),
    };
    format!(
        "{} — node {}\nidentity: {}\nnetwork booted: {}\nuptime: {}s\npaired devices: \
         {}\nactive beam links: {}\nreceived this session: {}{}\n{}{}",
        status.device,
        status.node_id_short,
        status.identity_dir.display(),
        booted,
        status.uptime_secs,
        status.paired_devices,
        status.active_offers.len(),
        status.received_total,
        listed,
        queue_line(status),
        abandoned_line(status)
    )
}

/// Deliveries that ENDED, and nothing at all when there are none.
///
/// Empty is the normal state, and a line saying "0 abandoned" on every status
/// call would be noise a reader learns to skip — which is the wrong habit for
/// the one line that reports a file somebody was promised and will not get.
/// Each entry names the file, the device and the reason, because "1 delivery
/// was abandoned" sends the reader looking for which one.
fn abandoned_line(status: &ServerStatus) -> String {
    if status.abandoned.is_empty() {
        return String::new();
    }
    let listed: Vec<String> = status
        .abandoned
        .iter()
        .map(|a| format!("- {} to {} ({}) — {}", a.name, a.device, human_bytes(a.size), a.reason))
        .collect();
    let shortened = shortened_clause(status.abandoned_total, status.abandoned.len());
    format!(
        "\nGIVEN UP ON in this session: {}{} — these files did NOT arrive and are no longer \
         queued. Tell the user:\n{}",
        status.abandoned_total,
        shortened,
        listed.join("\n")
    )
}

/// The queue's line in the status report.
///
/// A count on its own is the failure this whole surface exists to remove: a
/// reader who is told "3 queued" and nothing else cannot tell a queue that is
/// about to move from one that is stuck behind a store another process owns,
/// and would report the first while looking at the second. So the reason
/// comes first whenever there is one, and the devices are named — the person
/// asking already knows which one they were waiting for.
///
/// Three states stop the queue, and this line has to separate them: a stated
/// blocking reason, a record this build cannot read, and a server that has
/// not opened its network — where the records are real, the attempt counts
/// are real, and nothing at all is trying to deliver them.
fn queue_line(status: &ServerStatus) -> String {
    // A record this build cannot read is a promise nobody is keeping, so it
    // is named wherever the queue is named — including next to a count of 0,
    // which is exactly when it would otherwise be invisible.
    let unreadable = if status.queue_unreadable.is_empty() {
        String::new()
    } else {
        format!(
            "\n{} queued record(s) this build cannot read, so they are never sent and never \
             deleted: {}",
            status.queue_unreadable.len(),
            status.queue_unreadable.join(", ")
        )
    };
    if let Some(reason) = &status.queue_blocked_reason {
        return format!(
            "queued deliveries: {} — NONE of them can move: {reason}{unreadable}",
            status.queued_total
        );
    }
    if status.queued_total == 0 {
        return format!("queued deliveries: 0{unreadable}");
    }
    let waiting: Vec<String> = status
        .queued
        .iter()
        .map(|q| match &q.last_error {
            Some(e) => format!("- {} to {} (attempt {}: {e})", q.name, q.device, q.attempts),
            None => format!("- {} to {} (not tried yet)", q.name, q.device),
        })
        .collect();
    // The third blocking state, and the one a reader is least able to guess.
    // `server_status` never boots — it opens no socket, by design — so a
    // session that has called nothing else prints records with an attempt
    // count from a previous process. That reads as a retry loop that is
    // running right now, when this server has no socket, no drain and no way
    // to hear the device dial in. "network booted: false" ten lines up is not
    // the same sentence, and a reader who is waiting for a file will not read
    // it as one.
    if !status.booted {
        return format!(
            "queued deliveries: {} holding {} — none of them is moving: this server has not \
             opened its network yet, so there is no delivery pass and no device can dial in. \
             Any attempt count below was written by an earlier session. The first tool call \
             that needs the network — send_to_device, beam_artifact, pair_device, or \
             list_devices with probe — opens it and starts the queue; server_status never \
             does. Waiting:\n{}{unreadable}",
            status.queued_total,
            human_bytes(status.queued_bytes),
            waiting.join("\n")
        );
    }
    format!(
        "queued deliveries: {} holding {}, NOT delivered yet{}:\n{}{unreadable}",
        status.queued_total,
        human_bytes(status.queued_bytes),
        if status.draining { ", a drain pass is running" } else { "" },
        waiting.join("\n")
    )
}

#[tool_handler]
impl ServerHandler for VlervMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(
                Implementation::new("vlerv-mcp", env!("CARGO_PKG_VERSION"))
                    .with_title("Vlervtifacts remote"),
            )
            .with_instructions(
                "Sends local files to Vlervtifacts devices over a direct, end-to-end encrypted \
                 peer-to-peer link. Nothing is uploaded to a server.\n\n\
                 Pick the tool by what the user asked for:\n\
                 - a named device (\"send it to my phone\") -> send_to_device;\n\
                 - a shareable link, or an unpaired recipient -> beam_artifact;\n\
                 - \"which devices\" -> list_devices;\n\
                 - a new device -> pair_device, then pair_status, then confirm_pairing;\n\
                 - a device that is no longer theirs, or that list_devices reports as \
                 \"refused\" -> forget_device.\n\n\
                 Two rules that need a human:\n\
                 1. pairing is only safe when the person compares the six fingerprint words on \
                 both screens — always show them and wait;\n\
                 2. send_to_device works only after the receiving device grants this server the \
                 \"control\" scope on its own side.\n\n\
                 send_to_device answers with a status of either \"delivered\" or \"queued\". \
                 \"queued\" means the device was asleep, the file was copied as it stood and is \
                 waiting here — it is NOT on that device, and saying it is would be wrong. Say \
                 which one happened, and use server_status to report what is still waiting.\n\n\
                 Links from beam_artifact stay fetchable only while this server process runs. \
                 The queue outlives it: a queued send is written to the state directory, and \
                 it goes out at the first network-touching tool call of a later session over \
                 that same state directory.",
            )
    }
}

/// The failure convention this module documents, in one place: a core result
/// becomes either a sentence plus the same facts as structured JSON, or a
/// TOOL-level failure carrying the core's own message. Every handler that
/// calls a fallible core method goes through here, so a new tool cannot decide
/// to raise a protocol error for a device that is merely offline.
///
/// `summary` runs only on success, and only once — the model's sentence is
/// derived from the value that is about to be returned, never from a second
/// call to the core.
fn render<T: Serialize>(
    result: Result<T, String>,
    summary: impl FnOnce(&T) -> String,
) -> Result<CallToolResult, ErrorData> {
    match result {
        Ok(value) => ok(summary(&value), &value),
        Err(message) => tool_failure(message),
    }
}

/// A successful result: a sentence the model reads, plus the same facts as
/// structured JSON for a client that renders it.
///
/// MCP types `structuredContent` as a JSON object. A client that validates
/// the field rejects an array-shaped result before the model reads a word of
/// it, so a list-shaped tool must name its array under a key — three handlers
/// reached this with a bare `Vec` before the check below existed.
///
/// The check runs in RELEASE too, on purpose. A `debug_assert!` here would be
/// absent from the binary `README-MCP.md` tells people to build, and would
/// fire only for a handler some test happens to call; a wrong shape already
/// breaks the call at the client, so failing loudly here costs nothing and
/// turns an opaque protocol error into a message the model can report.
fn ok<T: Serialize>(summary: impl Into<String>, value: &T) -> Result<CallToolResult, ErrorData> {
    let json = serde_json::to_value(value)
        .map_err(|e| ErrorData::internal_error(format!("cannot serialize result: {e}"), None))?;
    if !json.is_object() {
        return tool_failure(format!(
            "internal: structuredContent must be a record, got: {json}"
        ));
    }
    let mut result = CallToolResult::structured(json);
    result.content = vec![ContentBlock::text(summary.into())];
    Ok(result)
}

/// A tool-level failure. `Ok` on purpose: the message is the useful part, and
/// a protocol error would be rendered opaquely instead of reaching the model.
fn tool_failure(message: String) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::error(vec![ContentBlock::text(message)]))
}

/// The clause a bounded list adds when the cap has dropped its oldest
/// entries, and nothing when it has not.
///
/// ONE producer, because both bounded lists print into the SAME status
/// message: two spellings of "there is more than this" in one response is a
/// reader asking which of the two lists is the shortened kind. A list that is
/// silently shortened reads as the whole account, which is the failure this
/// clause exists to prevent.
fn shortened_clause(total: u64, listed: usize) -> String {
    match total as usize > listed {
        true => format!(" (listing the last {listed})"),
        false => String::new(),
    }
}

/// "in 24 hours" style copy from an absolute expiry, so the model does not
/// have to do clock arithmetic to tell the user.
fn human_hours(expires_at: u64) -> String {
    let now = vlerv_remote::peers::now_unix();
    let left = expires_at.saturating_sub(now);
    match left {
        0 => "less than a minute".to_string(),
        s if s < 3600 => format!("{} minutes", s.div_ceil(60)),
        s if s < 2 * 3600 => "1 hour".to_string(),
        s => format!("{} hours", s.div_ceil(3600)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::handler::server::router::tool::ToolRouter;

    fn router() -> ToolRouter<VlervMcp> {
        VlervMcp::tool_router()
    }

    fn status_with(total: u64, listed: usize) -> ServerStatus {
        ServerStatus {
            node_id: "ab".repeat(32),
            node_id_short: "abababab".to_string(),
            device: "Claude Code @ test".to_string(),
            identity_dir: "/tmp/x/remote".into(),
            state_dir: "/tmp/x".into(),
            booted: true,
            boot_error: None,
            uptime_secs: 1,
            paired_devices: 0,
            active_offers: Vec::new(),
            received_artifacts: (0..listed)
                .map(|i| crate::core::ReceivedArtifact {
                    from: "cd".repeat(32),
                    name: format!("a{i}.html"),
                    path: format!("/tmp/a{i}.html").into(),
                    size: 1,
                    hash: format!("{i:064x}"),
                })
                .collect(),
            received_total: total,
            queued: Vec::new(),
            queued_total: 0,
            queued_bytes: 0,
            retained_bytes: 0,
            queue_unreadable: Vec::new(),
            abandoned: Vec::new(),
            abandoned_total: 0,
            draining: false,
            queue_blocked_reason: None,
            roots: Vec::new(),
        }
    }

    /// A status with one send waiting for a device that did not answer.
    fn status_with_queue(last_error: Option<&str>) -> ServerStatus {
        let mut status = status_with(0, 0);
        status.queued = vec![crate::core::QueuedDelivery {
            id: "0000000000001-0000".to_string(),
            device: "Val's iPhone".to_string(),
            node_id: "cd".repeat(32),
            node_id_short: "cdcdcdcdcd".to_string(),
            name: "report.html".to_string(),
            size: 4096,
            hash: "ab".repeat(32),
            source: "/w/report.html".into(),
            enqueued_at: 100,
            expires_at: 200,
            attempts: 2,
            last_attempt_at: 150,
            last_error: last_error.map(str::to_string),
        }];
        status.queued_total = 1;
        status.queued_bytes = 4096;
        status.retained_bytes = 4096;
        status
    }

    #[test]
    fn a_truncated_received_list_says_so_and_a_complete_one_does_not() {
        // The whole reason `received_total` exists: once the cap drops the
        // oldest arrivals, the count and the list disagree, and a reader who
        // is not told that takes the listed entries for all of them.
        let truncated = status_summary(&status_with(105, 100));
        assert!(truncated.contains("received this session: 105"), "{truncated}");
        assert!(truncated.contains("(listing the last 100)"), "{truncated}");

        // When they agree, the clause must not appear at all.
        let complete = status_summary(&status_with(3, 3));
        assert!(complete.contains("received this session: 3"), "{complete}");
        assert!(!complete.contains("listing"), "{complete}");

        // The boundary: exactly at the cap, nothing was dropped.
        let at_cap = status_summary(&status_with(100, 100));
        assert!(!at_cap.contains("listing"), "{at_cap}");
    }

    #[test]
    fn a_delivery_that_was_given_up_on_is_named_and_a_quiet_session_says_nothing() {
        // A count on its own would send the reader looking for which file,
        // and "0 abandoned" on every call is noise a reader learns to skip —
        // the wrong habit for the one line that reports a promise this server
        // broke.
        let quiet = status_summary(&status_with(0, 0));
        assert!(!quiet.contains("GIVEN UP ON"), "nothing died, so nothing is said: {quiet}");

        let mut status = status_with(0, 0);
        status.abandoned = vec![crate::core::AbandonedDelivery {
            id: "1700000000001-0000".to_string(),
            device: "Val's iPhone".to_string(),
            name: "report.html".to_string(),
            size: 2048,
            reason: "it was not delivered within the seven-day limit".to_string(),
            at: 1_700_000_000,
        }];
        status.abandoned_total = 1;
        let told = status_summary(&status);
        assert!(told.contains("GIVEN UP ON in this session: 1"), "{told}");
        assert!(told.contains("report.html to Val's iPhone (2 KiB)"), "{told}");
        assert!(told.contains("seven-day limit"), "{told}");
        assert!(told.contains("did NOT arrive"), "the reader is told what it means: {told}");
        assert!(!told.contains("listing the last"), "nothing was dropped: {told}");

        // Same claim `received_total` makes: a shortened list must say so, or
        // it reads as the whole account of what was lost.
        status.abandoned_total = 60;
        assert!(status_summary(&status).contains("(listing the last 1)"));
    }

    #[test]
    fn a_queued_send_never_reads_as_a_delivered_one() {
        // The failure this whole feature exists to remove is a send that
        // reads as done and is not. The tagged answer is what makes the two
        // sentences impossible to confuse, so both are pinned here.
        let delivered = delivery_summary(
            "/w/report.html",
            &Delivery::Delivered {
                device: "Val's iPhone".to_string(),
                node_id: "cd".repeat(32),
                name: "report.html".to_string(),
                size: 4096,
                hash: "ab".repeat(32),
            },
        );
        assert!(
            delivered.starts_with("Delivered /w/report.html (4 KiB) to Val's iPhone."),
            "{delivered}"
        );
        assert!(delivered.contains("landed there"), "{delivered}");

        let outcome = Delivery::Queued {
            device: "Val's iPhone".to_string(),
            node_id: "cd".repeat(32),
            name: "report.html".to_string(),
            size: 4096,
            hash: "ab".repeat(32),
            id: "0000000000001-0000".to_string(),
            expires_at: 200,
            reason: "peer offline — could not reach it (timed out)".to_string(),
            notes: vec!["The file was copied as it stands right now.".to_string()],
        };
        // The tag is what a client reads, and `structuredContent` is typed as
        // an OBJECT: an enum serialized any other way is rejected before the
        // model sees a word of it, which is what `ok` refuses at runtime.
        let json = serde_json::to_value(&outcome).unwrap();
        assert_eq!(json.get("status").and_then(|v| v.as_str()), Some("queued"), "{json}");
        assert!(json.is_object(), "{json}");

        let queued = delivery_summary("/w/report.html", &outcome);
        assert!(queued.starts_with("NOT delivered yet"), "{queued}");
        assert!(!queued.contains("Delivered"), "no reading of this may say it arrived: {queued}");
        assert!(queued.contains("peer offline"), "the cause is quoted, not paraphrased: {queued}");
        assert!(queued.contains("has not arrived"), "{queued}");
        // The notes are the honest part — snapshot semantics, the running
        // server, the deadline — and dropping them makes the sentence a lie
        // of omission.
        assert!(queued.contains("copied as it stands right now"), "{queued}");
    }

    #[test]
    fn a_queue_that_cannot_move_names_its_reason_instead_of_reporting_a_count() {
        // A second Claude Code session over one state directory can see the
        // queue and cannot touch it. Reporting "1 queued" there reads as "it
        // is on its way", which is the same silent failure in a new place.
        let mut blocked = status_with_queue(None);
        blocked.queue_blocked_reason =
            Some("another Vlerv process is already using the blob store".to_string());
        let text = status_summary(&blocked);
        assert!(text.contains("NONE of them can move"), "{text}");
        assert!(text.contains("already using the blob store"), "{text}");

        // With nothing blocking it, the queue names what is waiting and why
        // the last attempt did not land.
        let waiting = status_summary(&status_with_queue(Some("peer offline — could not reach it")));
        assert!(waiting.contains("queued deliveries: 1 holding 4 KiB"), "{waiting}");
        assert!(waiting.contains("NOT delivered yet"), "{waiting}");
        assert!(waiting.contains("report.html to Val's iPhone"), "{waiting}");
        assert!(waiting.contains("attempt 2: peer offline"), "{waiting}");
        assert!(
            !waiting.contains("a drain pass is running"),
            "a queue nothing is touching must not claim otherwise: {waiting}"
        );

        // And a queue something IS touching says so. The count alone reads
        // the same either way, and those are different situations for the
        // person deciding whether to wait.
        let mut moving = status_with_queue(Some("peer offline — could not reach it"));
        moving.draining = true;
        assert!(status_summary(&moving).contains("a drain pass is running"), "{moving:?}");

        // An empty queue says so in one line, and an unreadable record is
        // named even then — it is a delivery that is quietly not happening.
        let mut empty = status_with(0, 0);
        assert!(status_summary(&empty).contains("queued deliveries: 0"));
        empty.queue_unreadable = vec!["0000000000001-0000".to_string()];
        let broken = status_summary(&empty);
        assert!(broken.contains("cannot read"), "{broken}");
        assert!(broken.contains("0000000000001-0000"), "{broken}");
    }

    #[test]
    fn a_queue_on_a_server_that_never_opened_its_network_says_nothing_is_moving() {
        // The state a fresh session is in: `server_status` reads the spool off
        // disk and boots nothing, so it prints records another process queued,
        // with that process's attempt counts. "attempt 3: peer offline" beside
        // a count reads as a retry loop that is running now — there is no
        // socket, no drain and no way for the device to dial in, and the only
        // other clue is "network booted: false" ten lines up.
        let mut cold = status_with_queue(Some("peer offline — could not reach it"));
        cold.booted = false;
        let text = status_summary(&cold);
        assert!(text.contains("none of them is moving"), "{text}");
        assert!(text.contains("has not opened its network"), "{text}");
        // And what to call to start it, since this tool never will.
        assert!(text.contains("send_to_device"), "{text}");
        assert!(text.contains("server_status never does"), "{text}");
        // The records stay named: the reader is still owed which file, to
        // which device, and what the last session was told.
        assert!(text.contains("report.html to Val's iPhone"), "{text}");
        assert!(text.contains("attempt 2: peer offline"), "{text}");
        assert!(
            !text.contains("a drain pass is running"),
            "a server with no network cannot be draining: {text}"
        );

        // The same server with an empty spool says one plain line: there is
        // nothing to be wrong about, and a warning there would be noise on
        // every fresh session.
        let mut empty = status_with(0, 0);
        empty.booted = false;
        assert_eq!(status_summary(&empty).lines().last(), Some("queued deliveries: 0"));
    }

    #[test]
    fn every_documented_tool_is_registered_exactly_once() {
        let tools = router().list_all();
        let mut names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            [
                "beam_artifact",
                "confirm_pairing",
                "forget_device",
                "list_devices",
                "pair_device",
                "pair_status",
                "send_to_device",
                "server_status",
                "stop_beam",
            ]
        );
    }

    #[test]
    fn every_tool_carries_a_description_written_for_a_model() {
        for tool in router().list_all() {
            let description = tool.description.as_deref().unwrap_or_default();
            assert!(
                description.len() > 80,
                "{} needs a description that says WHEN to call it",
                tool.name
            );
        }
    }

    #[test]
    fn the_required_arguments_are_exactly_the_ones_without_a_default() {
        let tools = router().list_all();
        let required = |name: &str| -> Vec<String> {
            let tool = tools.iter().find(|t| t.name == name).expect(name);
            tool.input_schema
                .get("required")
                .and_then(|r| r.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                .unwrap_or_default()
        };
        assert_eq!(required("beam_artifact"), ["path"]);
        assert_eq!(required("send_to_device"), ["path", "device"]);
        assert_eq!(required("confirm_pairing"), ["accept"]);
        // The three no-argument tools take an empty object.
        for name in ["list_devices", "pair_device", "pair_status", "server_status", "stop_beam"] {
            assert!(required(name).is_empty(), "{name} must accept {{}}");
        }
    }

    #[test]
    fn the_schemas_name_the_properties_a_caller_must_send() {
        let tools = router().list_all();
        let props = |name: &str| -> Vec<String> {
            let tool = tools.iter().find(|t| t.name == name).expect(name);
            let mut keys: Vec<String> = tool
                .input_schema
                .get("properties")
                .and_then(|p| p.as_object())
                .map(|o| o.keys().cloned().collect())
                .unwrap_or_default();
            keys.sort();
            keys
        };
        assert_eq!(props("beam_artifact"), ["path", "ttl_hours"]);
        assert_eq!(props("list_devices"), ["probe"]);
        assert_eq!(props("send_to_device"), ["device", "path"]);
        assert_eq!(props("confirm_pairing"), ["accept", "node_id", "scope"]);
        assert_eq!(props("stop_beam"), ["hash"]);
    }

    #[test]
    fn the_server_info_tells_the_model_the_two_human_rules() {
        let core = McpCore::new("/tmp/vlerv-mcp-test".into(), vec![], "/tmp".into(), None);
        let info = VlervMcp::new(Arc::new(core)).get_info();
        let instructions = info.instructions.unwrap_or_default();
        assert!(instructions.contains("six fingerprint words"), "{instructions}");
        assert!(instructions.contains("control"), "{instructions}");
        assert_eq!(info.server_info.name, "vlerv-mcp");
        assert!(info.capabilities.tools.is_some(), "the server must advertise tools");
    }

    #[test]
    fn the_model_facing_text_never_says_a_queued_send_dies_with_this_process() {
        // Both strings said the queue ends when the process does, and
        // `a_spooled_delivery_survives_the_process_that_accepted_it` proves
        // the opposite: the record is on disk, and a later session over the
        // same state directory delivers it. A model reading the old sentence
        // tells the user a file that is still coming is lost.
        let core = McpCore::new("/tmp/vlerv-mcp-test".into(), vec![], "/tmp".into(), None);
        let info = VlervMcp::new(Arc::new(core)).get_info();
        let instructions = info.instructions.unwrap_or_default();
        assert!(instructions.contains("The queue outlives it"), "{instructions}");
        assert!(
            instructions.contains("later session over that same state directory"),
            "{instructions}"
        );

        let tools = router().list_all();
        let send = tools.iter().find(|t| t.name == "send_to_device").expect("send_to_device");
        let described = send.description.as_deref().unwrap_or_default();
        assert!(
            described.contains("first network-touching tool call of a later session"),
            "{described}"
        );
        assert!(
            !described.contains("while this server is running"),
            "a queued send does not need this process to survive: {described}"
        );
    }

    #[test]
    fn a_tool_failure_reaches_the_model_instead_of_becoming_a_protocol_error() {
        let result = tool_failure("device is offline".to_string()).unwrap();
        assert_eq!(result.is_error, Some(true));
        match &result.content[0] {
            ContentBlock::Text(text) => assert_eq!(text.text, "device is offline"),
            other => panic!("expected text content, got {other:?}"),
        }
    }

    #[test]
    fn a_success_carries_both_prose_and_structured_facts() {
        #[derive(Serialize)]
        struct Out {
            name: String,
        }
        let result = ok("sent it", &Out { name: "report.html".into() }).unwrap();
        assert_eq!(result.is_error, Some(false));
        assert_eq!(
            result.structured_content.unwrap().get("name").and_then(|v| v.as_str()),
            Some("report.html")
        );
        match &result.content[0] {
            ContentBlock::Text(text) => assert_eq!(text.text, "sent it"),
            other => panic!("expected text content, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn list_results_are_objects_because_mcp_rejects_a_bare_array() {
        // `structuredContent` must be a record. A handler that returns a
        // `Vec` directly makes the client reject the whole call, so both
        // list-shaped tools name their array under a key.
        // A real temp dir, like every other test here: a fixed path is shared
        // across runs, worktrees and parallel invocations.
        let dir = tempfile::TempDir::new().unwrap();
        let core = Arc::new(McpCore::new(
            dir.path().to_path_buf(),
            vec![],
            dir.path().to_path_buf(),
            None,
        ));
        let server = VlervMcp::new(core);

        let listed = server
            .list_devices(Parameters(ListDevicesArgs { probe: Some(false) }))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert!(listed.get("devices").is_some_and(|v| v.is_array()), "{listed}");

        let pending = server.pair_status().await.unwrap().structured_content.unwrap();
        assert!(pending.get("pending").is_some_and(|v| v.is_array()), "{pending}");

        // The third list-shaped tool. It reached `ok` with a bare `Vec` until
        // the assertion inside `ok` made every tool test catch that.
        let stopped = server
            .stop_beam(Parameters(StopBeamArgs { hash: None }))
            .await
            .unwrap()
            .structured_content
            .unwrap();
        assert!(stopped.get("stopped").is_some_and(|v| v.is_array()), "{stopped}");
    }

    #[test]
    fn sizes_and_expiries_are_rendered_for_a_person() {
        // The crate's formatter: KiB/MiB, the same units its own size
        // refusals use.
        assert_eq!(human_bytes(512), "1 KiB");
        assert_eq!(human_bytes(2048), "2 KiB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5 MiB");
        let now = vlerv_remote::peers::now_unix();
        assert_eq!(human_hours(now + 24 * 3600), "24 hours");
        assert_eq!(human_hours(now + 90 * 60), "1 hour");
        assert_eq!(human_hours(now + 600), "10 minutes");
        // An already-expired link must not underflow into a huge number.
        assert_eq!(human_hours(now.saturating_sub(10)), "less than a minute");
    }
}
