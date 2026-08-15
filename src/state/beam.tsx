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
  total: number | null;
}

export interface BeamSendState {
  path: string;
  name: string;
  /** Null when initiated from the toolbar (size shown once the offer exists). */
  size: number | null;
  phase: "confirm" | "creating" | "ready" | "error";
  offer?: BeamOffer;
  error?: string;
}

export interface BeamReceiveState {
  req: BeamReceiveRequestPayload;
  phase: "confirm" | "fetching" | "error";
  received: number;
  error?: string;
}

export interface BeamStateValue {
  offers: BeamOffer[];
  received: BeamReceivedEntry[];
  send: BeamSendState | null;
  receive: BeamReceiveState | null;
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
    setReceive({ req, phase: "confirm", received: 0 });
  });

  useTauriEvent<BeamProgressPayload>("vlerv://beam-progress", (p) => {
    setReceive((r) =>
      r && r.req.hash === p.hash && r.phase === "fetching"
        ? { ...r, received: p.received }
        : r,
    );
  });

  useTauriEvent<BeamOffer[]>("vlerv://beam-offers-updated", (list) => {
    setOffers(list);
  });

  const runOffer = React.useCallback(
    (path: string, name: string, size: number | null) => {
      setSend({ path, name, size, phase: "creating" });
      ipc.beamOffer?.(path)
        .then((offer) => {
          setSend((s) =>
            s && s.path === path
              ? { ...s, size: offer.size, phase: "ready", offer }
              : s,
          );
        })
        .catch((e: unknown) => {
          setSend((s) =>
            s && s.path === path ? { ...s, phase: "error", error: String(e) } : s,
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

  const refreshReceived = React.useCallback(() => {
    ipc.beamListReceived?.().then(setReceived).catch(() => {});
  }, [ipc]);

  const acceptReceive = React.useCallback(() => {
    const r = receiveRef.current;
    if (!r || r.phase === "fetching") return;
    const { req } = r;
    setReceive({ req, phase: "fetching", received: 0 });
    ipc.beamReceive?.(req.ticket, req.name, req.size ?? undefined)
      .then((file) => {
        setReceive(null);
        dispatch({ type: "FOCUS_OR_OPEN", path: file.path, external: true });
        refreshReceived();
      })
      .catch((e: unknown) => {
        setReceive((cur) =>
          cur && cur.req.ticket === req.ticket
            ? { ...cur, phase: "error", error: String(e) }
            : cur,
        );
      });
  }, [dispatch, ipc, refreshReceived]);

  const declineReceive = React.useCallback(() => setReceive(null), []);

  const stopOffer = React.useCallback(
    (id: string) => {
      // The offers-updated event refreshes the list; the local filter just
      // makes the row vanish without waiting for the round-trip.
      setOffers((list) => list.filter((o) => o.id !== id));
      ipc.beamStop?.(id).catch(() => {});
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
    (): BeamStateValue => ({ offers, received, send, receive }),
    [offers, received, send, receive],
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
