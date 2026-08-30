// RemotePairDialog — the pairing confirm UI, mounted at the app root so it
// pops up regardless of whether Settings is open (design §6: a `vlerv://pair`
// link can arrive at any time, and the fingerprint step happens on BOTH
// screens the moment either side reaches it). Two faces, mutually exclusive:
//  - pairLinkArrival: a deep link arrived; nothing has been dialed yet.
//  - pendingConfirm: the fingerprint step, on whichever side reached it
//    (`role` tells host and guest apart) — the six words must match on both
//    screens before either persists the peer.
import * as React from "react";
import { Check, ShieldCheck, X } from "lucide-react";
import { useRemoteActions, useRemoteState } from "../state/remote";
import type { RemotePairLinkArrival } from "../state/remote";
import { useEscape } from "../hooks/useEscape";
import type { RemotePendingPair, RemoteScope } from "../ipc";

// Both faces take their subject as a prop. The host below has already
// decided which one is up, and re-reading the context here would only buy a
// null the parent has ruled out.
function PairLinkFace({
  arrival,
}: {
  arrival: RemotePairLinkArrival;
}): React.ReactElement {
  const { pairingBusy, pairingError } = useRemoteState();
  const { completePairing, dismissPairLink } = useRemoteActions();
  useEscape(dismissPairLink);

  return (
    <div
      className="beam-dialog"
      role="dialog"
      aria-label="Pairing request"
      data-testid="remote-pair-link-dialog"
    >
      <div className="beam-dialog-header">
        <ShieldCheck size={14} strokeWidth={2} />
        <span>Pairing request</span>
        <button
          type="button"
          className="beam-dialog-close"
          title="Decline"
          aria-label="Decline"
          onClick={dismissPairLink}
        >
          <X size={13} strokeWidth={2} />
        </button>
      </div>
      <div className="beam-dialog-body">
        <div className="beam-file-row">
          <span className="beam-file-name">{arrival.device}</span>
        </div>
        <p className="beam-hint">
          This link asks to pair with{" "}
          <code className="beam-fingerprint" title={arrival.peer}>
            {arrival.peer_short}
          </code>
          . Nothing is dialed until you proceed, and nothing is trusted until
          you compare the six words on both screens.
        </p>
        {pairingError ? <p className="beam-error" role="alert">{pairingError}</p> : null}
        <div className="beam-actions">
          <button
            type="button"
            className="button"
            disabled={pairingBusy}
            onClick={() => completePairing(arrival.ticket)}
          >
            {pairingBusy ? "Connecting…" : "Continue"}
          </button>
          <button type="button" className="button button-secondary" autoFocus onClick={dismissPairLink}>
            Decline
          </button>
        </div>
      </div>
    </div>
  );
}

function FingerprintFace({
  pending,
}: {
  pending: RemotePendingPair;
}): React.ReactElement {
  const { pairingError } = useRemoteState();
  const { confirmPairing } = useRemoteActions();
  const [scope, setScope] = React.useState<RemoteScope>("view-open");
  useEscape(() => confirmPairing(false));

  return (
    <div
      className="beam-dialog"
      role="dialog"
      aria-label="Confirm pairing"
      data-testid="remote-fingerprint-dialog"
    >
      <div className="beam-dialog-header">
        <ShieldCheck size={14} strokeWidth={2} />
        <span>Confirm pairing</span>
        <button
          type="button"
          className="beam-dialog-close"
          title="Decline"
          aria-label="Decline"
          onClick={() => confirmPairing(false)}
        >
          <X size={13} strokeWidth={2} />
        </button>
      </div>
      <div className="beam-dialog-body">
        <div className="beam-file-row">
          <span className="beam-file-name">{pending.device}</span>
          <span className="beam-file-meta">
            {pending.role === "host" ? "wants to pair" : "you are pairing with"}
          </span>
        </div>
        <p className="beam-hint">
          Read these six words out loud and confirm they match on the OTHER
          screen too — that is what rules out a machine-in-the-middle.
        </p>
        <div className="remote-fingerprint-words" data-testid="remote-fingerprint-words">
          {pending.fingerprint.map((word, i) => (
            <span key={`${word}-${i}`}>{word}</span>
          ))}
        </div>
        <label className="beam-hint" style={{ display: "block", marginTop: "var(--space-3)" }}>
          Grant this device
          <select
            aria-label="Scope to grant"
            value={scope}
            onChange={(e) => setScope(e.target.value as RemoteScope)}
            style={{ marginLeft: "8px" }}
          >
            <option value="view-open">View open (tabs, bookmarks, recents)</option>
            <option value="browse">Browse (+ the whole workspace)</option>
            <option value="control">Control (+ open artifacts on this Mac)</option>
          </select>
        </label>
        {pairingError ? <p className="beam-error" role="alert">{pairingError}</p> : null}
        <div className="beam-actions">
          <button type="button" className="button" onClick={() => confirmPairing(true, scope)}>
            <Check size={13} strokeWidth={2.5} /> The words match — pair
          </button>
          <button
            type="button"
            className="button button-secondary"
            autoFocus
            onClick={() => confirmPairing(false)}
          >
            Decline
          </button>
        </div>
      </div>
    </div>
  );
}

/** Fingerprint confirmation wins over a fresh link arrival: it means a dial
 * already happened and is the more time-sensitive of the two. */
export default function RemotePairDialog(): React.ReactElement | null {
  const { pendingConfirm, pairLinkArrival } = useRemoteState();
  let face: React.ReactElement;
  if (pendingConfirm) face = <FingerprintFace pending={pendingConfirm} />;
  else if (pairLinkArrival) face = <PairLinkFace arrival={pairLinkArrival} />;
  else return null;
  return (
    <div className="beam-backdrop" data-testid="remote-pair-backdrop">
      {face}
    </div>
  );
}
