// Beam UI — the send/receive dialog faces and the toolbar "beaming"
// indicator. State machine lives in src/state/beam.tsx; this file only
// renders it.
import * as React from "react";
import { Check, Copy, X, Zap } from "lucide-react";
import { useBeamActions, useBeamState } from "../state/beam";
import { useCopyFeedback } from "../hooks/useCopyFeedback";
import { expiresIn, humanBytes } from "../utils/beam-format";

function nowSecs(): number {
  return Math.floor(Date.now() / 1000);
}

function CopyLinkButton({ link }: { link: string }): React.ReactElement {
  const { copied, copy } = useCopyFeedback();
  return (
    <button
      type="button"
      className="button"
      data-testid="beam-copy-link"
      onClick={() => copy(link)}
    >
      {copied ? <Check size={13} strokeWidth={2.5} /> : <Copy size={13} strokeWidth={2} />}
      {copied ? "Copied" : "Copy link"}
    </button>
  );
}

/** Escape closes whichever dialog face is open (decline semantics). */
function useEscape(onEscape: () => void): void {
  React.useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onEscape();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onEscape]);
}

function SendFace(): React.ReactElement {
  const { send } = useBeamState();
  const { confirmSend, closeSend } = useBeamActions();
  useEscape(closeSend);
  if (!send) return <></>;

  return (
    <div className="beam-dialog" role="dialog" aria-label="Beam artifact" data-testid="beam-send-dialog">
      <div className="beam-dialog-header">
        <Zap size={14} strokeWidth={2} />
        <span>Beam artifact</span>
        <button type="button" className="beam-dialog-close" title="Close" aria-label="Close" onClick={closeSend}>
          <X size={13} strokeWidth={2} />
        </button>
      </div>

      <div className="beam-dialog-body">
        <div className="beam-file-row">
          <span className="beam-file-name">{send.name}</span>
          {send.size != null ? <span className="beam-file-meta">{humanBytes(send.size)}</span> : null}
        </div>
        <div className="beam-file-path" title={send.path}>{send.path}</div>

        {send.phase === "confirm" ? (
          <>
            <p className="beam-hint">
              Creates a one-shot link another Vlervcode instance can fetch this
              file with — peer-to-peer, end-to-end encrypted. Anyone holding
              the link can fetch while the offer is active.
            </p>
            <div className="beam-actions">
              <button type="button" className="button" autoFocus onClick={confirmSend}>
                Create beam link
              </button>
              <button type="button" className="button button-secondary" onClick={closeSend}>Cancel</button>
            </div>
          </>
        ) : null}

        {send.phase === "creating" ? (
          <p className="beam-hint" role="status">Staging file and minting ticket…</p>
        ) : null}

        {send.phase === "ready" && send.offer ? (
          <>
            <input
              className="beam-link-input"
              readOnly
              value={send.offer.link}
              onFocus={(e) => e.currentTarget.select()}
              data-testid="beam-link"
            />
            <p className="beam-hint">
              Serving while this app runs — expires in{" "}
              {expiresIn(send.offer.expires_at, nowSecs())}. Stop it anytime
              from the ⚡ indicator.
            </p>
            <div className="beam-actions">
              <CopyLinkButton link={send.offer.link} />
              <button type="button" className="button button-secondary" onClick={closeSend}>Done</button>
            </div>
          </>
        ) : null}

        {send.phase === "error" ? (
          <>
            <p className="beam-error" role="alert">{send.error}</p>
            <div className="beam-actions">
              <button type="button" className="button" onClick={confirmSend}>
                Try again
              </button>
              <button type="button" className="button button-secondary" onClick={closeSend}>Close</button>
            </div>
          </>
        ) : null}
      </div>
    </div>
  );
}

function ReceiveFace(): React.ReactElement {
  const { receive } = useBeamState();
  const { acceptReceive, declineReceive } = useBeamActions();
  useEscape(declineReceive);
  if (!receive) return <></>;
  const { req } = receive;
  const pct =
    receive.phase === "fetching" && req.size
      ? Math.min(100, Math.round((receive.received / req.size) * 100))
      : null;

  return (
    <div className="beam-dialog" role="dialog" aria-label="Incoming beam" data-testid="beam-receive-dialog">
      <div className="beam-dialog-header">
        <Zap size={14} strokeWidth={2} />
        <span>Incoming beam</span>
        <button type="button" className="beam-dialog-close" title="Decline" aria-label="Decline" onClick={declineReceive}>
          <X size={13} strokeWidth={2} />
        </button>
      </div>

      <div className="beam-dialog-body">
        <div className="beam-file-row">
          <span className="beam-file-name">{req.name}</span>
          <span className="beam-file-meta">
            {req.size != null ? humanBytes(req.size) : "unknown size"}
          </span>
        </div>
        <div className="beam-sender-row">
          from sender{" "}
          <code className="beam-fingerprint" title={req.sender_id}>{req.sender_id_short}</code>
        </div>
        {req.warn ? (
          <p className="beam-warn">Large file — the transfer may take a while.</p>
        ) : null}

        {receive.phase === "confirm" ? (
          <>
            <p className="beam-hint">
              Nothing has been fetched yet. Accepting streams the file
              (integrity-verified) into this app&apos;s received folder and
              opens it in a tab. The size shown is the sender&apos;s claim.
            </p>
            <div className="beam-actions">
              <button type="button" className="button" autoFocus onClick={acceptReceive}>
                Receive &amp; open
              </button>
              <button type="button" className="button button-secondary" onClick={declineReceive}>Decline</button>
            </div>
          </>
        ) : null}

        {receive.phase === "fetching" ? (
          <>
            <div className="beam-progress" role="progressbar" aria-valuenow={pct ?? undefined}>
              <div
                className={`beam-progress-fill${pct == null ? " indeterminate" : ""}`}
                style={
                  pct != null
                    ? ({ "--beam-progress": pct / 100 } as React.CSSProperties)
                    : undefined
                }
              />
            </div>
            <p className="beam-hint" role="status">
              {humanBytes(receive.received)}
              {req.size != null ? ` of ${humanBytes(req.size)}` : ""} received…
            </p>
          </>
        ) : null}

        {receive.phase === "error" ? (
          <>
            <p className="beam-error" role="alert">{receive.error}</p>
            <div className="beam-actions">
              {/* "Sender offline" is retryable from the same dialog — the
                  ticket is an offer, not an escrow. */}
              <button type="button" className="button" onClick={acceptReceive}>
                Try again
              </button>
              <button type="button" className="button button-secondary" onClick={declineReceive}>Close</button>
            </div>
          </>
        ) : null}
      </div>
    </div>
  );
}

/** Modal host: renders whichever face is active. Receive wins if both are
 * somehow up — an incoming request is the more time-sensitive of the two. */
export default function BeamDialog(): React.ReactElement | null {
  const { send, receive } = useBeamState();
  if (!send && !receive) return null;
  return (
    <div className="beam-backdrop" data-testid="beam-backdrop">
      {receive ? <ReceiveFace /> : <SendFace />}
    </div>
  );
}

/** Toolbar pill: visible only while offers are active. Click for the list
 * (fetch counts, expiry, Stop) plus recent received files (fetched lazily
 * on open — the popover is the only place the list renders). */
export function BeamIndicator(): React.ReactElement | null {
  const { offers, received } = useBeamState();
  const { stopOffer, openReceived, refreshReceived } = useBeamActions();
  const [open, setOpen] = React.useState(false);
  React.useEffect(() => {
    if (offers.length === 0) setOpen(false);
  }, [offers.length]);
  if (offers.length === 0) return null;

  return (
    <div className="beam-indicator-wrap">
      <button
        type="button"
        className="beam-indicator"
        data-testid="beam-indicator"
        title={`Beaming ${offers.length} ${offers.length === 1 ? "offer" : "offers"}`}
        aria-expanded={open}
        onClick={() => {
          setOpen((v) => {
            if (!v) refreshReceived();
            return !v;
          });
        }}
      >
        <Zap size={12} strokeWidth={2.25} />
        {offers.length}
      </button>
      {open ? (
        <>
          <div className="beam-popover-scrim" onMouseDown={() => setOpen(false)} />
          <div className="beam-popover" role="menu">
            <div className="beam-popover-title">Beaming</div>
            {offers.map((o) => (
              <div key={o.id} className="beam-popover-row">
                <div className="beam-popover-info">
                  <span className="beam-popover-name" title={o.path}>{o.name}</span>
                  <span className="beam-popover-meta">
                    {o.fetches} {o.fetches === 1 ? "fetch" : "fetches"} · expires in{" "}
                    {expiresIn(o.expires_at, nowSecs())}
                  </span>
                </div>
                <button
                  type="button"
                  className="button button-secondary beam-stop"
                  onClick={() => stopOffer(o.id)}
                >
                  Stop
                </button>
              </div>
            ))}
            {received.length > 0 ? (
              <>
                <div className="beam-popover-title">Received</div>
                {received.slice(0, 5).map((r) => (
                  <button
                    key={r.path}
                    type="button"
                    className="beam-popover-received"
                    title={r.path}
                    onClick={() => {
                      setOpen(false);
                      openReceived(r.path);
                    }}
                  >
                    <span className="beam-popover-name">{r.name}</span>
                    <span className="beam-popover-meta">{humanBytes(r.size)}</span>
                  </button>
                ))}
              </>
            ) : null}
          </div>
        </>
      ) : null}
    </div>
  );
}
