// Beam state — offers, the send/receive dialog faces, and the received-dir
// prefix for the "beamed" badge. All networking lives in the Rust core; this
// provider only sequences IPC commands and `vlerv://beam-*` events.
//
// Two contexts, deliberately: actions (identity-stable callbacks plus the
// set-once receivedDir) and state (offers/dialog/progress). Chrome-wide
// consumers — Toolbar, every context menu via useFileMenu — subscribe only
// to actions, so per-MiB progress events re-render just the dialog and the
// indicator, not the whole app.
//
// Two entry points create a send dialog: the toolbar button (user already
// acted — offer immediately) and the `vlerv://beam` deep link (a link is
// hostile until proven otherwise — show a confirm face first; nothing is
// staged until the user clicks through). The receive dialog is always
// confirm-first: nothing transfers before an explicit accept.
import * as React from "react";
import type { BeamOffer, BeamReceivedEntry, IpcSurface } from "../ipc";
import { useTauriEvent } from "../hooks/useTauriEvent";
import { basename } from "../utils/path";
import { nextBurst } from "./beam-burst";
import { useTabsDispatch } from "./TabsProvider";

/** Payload of `vlerv://beam-send-request` (CLI / deep-link initiated). */
interface BeamSendRequestPayload {
  path: string;
  name: string;
  size: number;
}

/** Payload of `vlerv://beam-receive-request`. `name` arrives pre-sanitized
 * by the backend; `warn` is the backend's large-file judgment (the size
 * limits live in one place, in Rust); `sender_id_short` is the fingerprint
 * the user verifies. */
export interface BeamReceiveRequestPayload {
  ticket: string;
  name: string;
  size: number | null;
  warn: boolean;
  sender_id: string;
  sender_id_short: string;
  hash: string;
}

interface BeamProgressPayload {
  hash: string;
  received: number;
}

/** Payload of `vlerv://beam-received`: an artifact that landed WITHOUT this
 * side driving a fetch — today, a control-scoped peer's push (Scope v2). The
 * bytes went through the same verified receive path an accepted beam takes,
 * so they are surfaced the same way: a tab, with the beamed badge. `peer` is
 * absent for anything the local user accepted itself. */
interface BeamReceivedPayload {
  peer?: string;
  path: string;
  name: string;
  size: number;
  hash: string;
}

// Discriminated unions: the fields a phase needs travel WITH the phase, so
// "ready without an offer" or "error without a message" cannot be
// constructed — the dialog's `phase === "ready" && send.offer` guards used to
// paper over exactly those illegal states.
export type BeamSendState = { path: string; name: string } & (
  | { phase: "confirm" | "creating"; size: number | null }
  | { phase: "ready"; size: number; offer: BeamOffer }
  | { phase: "error"; size: number | null; error: string }
);

export type BeamReceiveState = { req: BeamReceiveRequestPayload } & (
  | { phase: "confirm" }
  | { phase: "fetching"; received: number }
  | { phase: "error"; error: string }
);

export interface BeamStateValue {
  offers: BeamOffer[];
  received: BeamReceivedEntry[];
  send: BeamSendState | null;
  receive: BeamReceiveState | null;
  /** Set when a revocation failed — the offer may still be served. */
  stopError: string | null;
  /** Set when the Received list could not be refreshed. An arrival inside a
   *  delivery run opens no tab, so the list is its only visible sign: a
   *  refresh that fails silently loses the whole arrival. */
  receivedError: string | null;
}

export interface BeamActionsValue {
  receivedDir: string | null;
  /** Toolbar path: stage + mint immediately. */
  beginSend(path: string): void;
  /** Deep-link confirm face, and "Try again" after a send error. */
  confirmSend(): void;
  closeSend(): void;
  /** Receive confirm, and "Try again" after sender-offline. */
  acceptReceive(): void;
  declineReceive(): void;
  stopOffer(id: string): void;
  openReceived(path: string): void;
  refreshReceived(): void;
}

const BeamStateContext = React.createContext<BeamStateValue | null>(null);
const BeamActionsContext = React.createContext<BeamActionsValue | null>(null);

export function useBeamState(): BeamStateValue {
  const ctx = React.useContext(BeamStateContext);
  if (!ctx) throw new Error("useBeamState must be used within BeamProvider");
  return ctx;
}

export function useBeamActions(): BeamActionsValue {
  const ctx = React.useContext(BeamActionsContext);
  if (!ctx) throw new Error("useBeamActions must be used within BeamProvider");
  return ctx;
}

export function BeamProvider({
  ipc,
  children,
}: {
  ipc: IpcSurface;
  children: React.ReactNode;
}): React.ReactElement {
  const dispatch = useTabsDispatch();
  const [offers, setOffers] = React.useState<BeamOffer[]>([]);
  const [received, setReceived] = React.useState<BeamReceivedEntry[]>([]);
  const [receivedDir, setReceivedDir] = React.useState<string | null>(null);
  const [send, setSend] = React.useState<BeamSendState | null>(null);
  const [receive, setReceive] = React.useState<BeamReceiveState | null>(null);
  const [stopError, setStopError] = React.useState<string | null>(null);
  const [receivedError, setReceivedError] = React.useState<string | null>(null);

  // Latest dialog state for the stable action callbacks — reading it via a
  // ref keeps the actions context identity-stable across progress events.
  const sendRef = React.useRef(send);
  sendRef.current = send;
  const receiveRef = React.useRef(receive);
  receiveRef.current = receive;

  // Mount: only the badge prefix and any pre-existing offers. The received
  // list is fetched lazily (popover open / after a receive) — scanning the
  // whole received/ tree on every app launch paid for a list that cannot
  // be visible yet.
  React.useEffect(() => {
    let cancelled = false;
    ipc.beamReceivedDir?.().then((dir) => {
      if (!cancelled) setReceivedDir(dir);
    }).catch(() => {});
    ipc.beamListOffers?.().then((list) => {
      if (!cancelled) setOffers(list);
    }).catch(() => {});
    return () => { cancelled = true; };
  }, [ipc]);

  useTauriEvent<BeamSendRequestPayload>("vlerv://beam-send-request", (req) => {
    setSend({ path: req.path, name: req.name, size: req.size, phase: "confirm" });
  });

  useTauriEvent<BeamReceiveRequestPayload>("vlerv://beam-receive-request", (req) => {
    // A transfer in flight owns the dialog — a second link must not replace
    // the fetching face out from under the user (and orphan its progress).
    if (receiveRef.current?.phase === "fetching") return;
    setReceive({ req, phase: "confirm" });
  });

  useTauriEvent<BeamProgressPayload>("vlerv://beam-progress", (p) => {
    setReceive((r) =>
      r && r.phase === "fetching" && r.req.hash === p.hash
        ? { ...r, received: p.received }
        : r,
    );
  });

  useTauriEvent<BeamOffer[]>("vlerv://beam-offers-updated", (list) => {
    setOffers(list);
  });

  // The one path to the Received list, so an arrival and the popover that
  // lists it cannot report the refresh differently.
  const refreshReceived = React.useCallback(() => {
    ipc.beamListReceived?.()
      .then((list) => {
        setReceived(list);
        setReceivedError(null);
      })
      .catch((e: unknown) => {
        console.error("vlerv: beam received list failed", e);
        setReceivedError("Could not list what has arrived — a file landed and is not shown.");
      });
  }, [ipc]);

  // When the previous artifact landed, or null while none has. A sender that
  // queued files while this device was unreachable delivers them in one run
  // as soon as it can reach it, so arrivals come in bursts and one tab each
  // would bury whatever the user is reading under a stack they never asked
  // for. This ref is all the provider holds: `nextBurst` judges the arrival
  // AND says what to remember, so no half of the rule lives here, and
  // beam-burst.test.ts holds its cases.
  const lastReceivedAtRef = React.useRef<number | null>(null);

  // A pushed artifact arrives already on disk and already verified — there is
  // no dialog to gate (the host's control-scope grant IS the consent), so an
  // arrival that opens a tab opens it exactly like an accepted receive does.
  // The rest of a burst only refresh the Received list, which is where a run
  // of files belongs.
  useTauriEvent<BeamReceivedPayload>("vlerv://beam-received", (file) => {
    const burst = nextBurst(lastReceivedAtRef.current, Date.now());
    lastReceivedAtRef.current = burst.previous;
    if (burst.opens) {
      dispatch({ type: "FOCUS_OR_OPEN", path: file.path, external: true });
    }
    // Surfaced, not swallowed: for every arrival of a run after the first
    // this list is the ONLY place the artifact shows up, so a refresh that
    // fails leaves a file that landed on this disk invisible everywhere.
    refreshReceived();
  });

  const runOffer = React.useCallback(
    (path: string, name: string, size: number | null) => {
      setSend({ path, name, size, phase: "creating" });
      // Optional-chaining a missing method yields undefined and would strand
      // the dialog on "creating" forever with no .catch — surface it instead.
      const pending = ipc.beamOffer?.(path);
      if (!pending) {
        setSend({ path, name, size, phase: "error", error: "Beam is not available in this build." });
        return;
      }
      pending
        .then((offer) => {
          setSend((s) =>
            s && s.path === path ? { path, name, size: offer.size, phase: "ready", offer } : s,
          );
        })
        .catch((e: unknown) => {
          setSend((s) =>
            s && s.path === path ? { path, name, size, phase: "error", error: String(e) } : s,
          );
        });
    },
    [ipc],
  );

  const beginSend = React.useCallback(
    (path: string) => runOffer(path, basename(path), null),
    [runOffer],
  );

  const confirmSend = React.useCallback(() => {
    const s = sendRef.current;
    if (s && (s.phase === "confirm" || s.phase === "error")) {
      runOffer(s.path, s.name, s.size);
    }
  }, [runOffer]);

  const closeSend = React.useCallback(() => setSend(null), []);

  const acceptReceive = React.useCallback(() => {
    const r = receiveRef.current;
    if (!r || r.phase === "fetching") return;
    const { req } = r;
    setReceive({ req, phase: "fetching", received: 0 });
    const pending = ipc.beamReceive?.(req.ticket, req.name);
    if (!pending) {
      setReceive({ req, phase: "error", error: "Beam is not available in this build." });
      return;
    }
    pending
      .then((file) => {
        // Only steal focus / open the tab if THIS transfer still owns the
        // dialog — a declined or superseded fetch must not surface a file the
        // user dismissed (the receive has no backend cancel, so it still
        // lands on disk; refreshReceived surfaces it in the list instead).
        const owns = receiveRef.current?.req.ticket === req.ticket;
        setReceive((cur) => (cur && cur.req.ticket === req.ticket ? null : cur));
        if (owns) {
          dispatch({ type: "FOCUS_OR_OPEN", path: file.path, external: true });
        }
        refreshReceived();
      })
      .catch((e: unknown) => {
        setReceive((cur) =>
          cur && cur.req.ticket === req.ticket
            ? { req: cur.req, phase: "error", error: String(e) }
            : cur,
        );
      });
  }, [dispatch, ipc, refreshReceived]);

  const declineReceive = React.useCallback(() => setReceive(null), []);

  const stopOffer = React.useCallback(
    (id: string) => {
      // Optimistically hide the row, but a FAILED stop leaves the offer still
      // served (revocation is a security action). Only success re-emits
      // beam-offers-updated, so on failure re-fetch the true list ourselves
      // and surface it — otherwise the offer is live but invisible until the
      // app quits.
      setOffers((list) => list.filter((o) => o.id !== id));
      setStopError(null);
      ipc.beamStop?.(id).catch((e: unknown) => {
        console.error("vlerv: beam stop failed", id, e);
        ipc.beamListOffers?.().then(setOffers).catch(() => {});
        setStopError("Could not stop this beam — it may still be shared.");
      });
    },
    [ipc],
  );

  const openReceived = React.useCallback(
    (path: string) => {
      dispatch({ type: "FOCUS_OR_OPEN", path, external: true });
    },
    [dispatch],
  );

  const stateValue = React.useMemo(
    (): BeamStateValue => ({ offers, received, send, receive, stopError, receivedError }),
    [offers, received, send, receive, stopError, receivedError],
  );

  const actionsValue = React.useMemo(
    (): BeamActionsValue => ({
      receivedDir,
      beginSend,
      confirmSend,
      closeSend,
      acceptReceive,
      declineReceive,
      stopOffer,
      openReceived,
      refreshReceived,
    }),
    [
      receivedDir,
      beginSend, confirmSend, closeSend, acceptReceive, declineReceive,
      stopOffer, openReceived, refreshReceived,
    ],
  );

  return (
    <BeamActionsContext.Provider value={actionsValue}>
      <BeamStateContext.Provider value={stateValue}>
        {children}
      </BeamStateContext.Provider>
    </BeamActionsContext.Provider>
  );
}
