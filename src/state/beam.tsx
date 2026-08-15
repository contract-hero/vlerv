// Beam state — offers, the send/receive dialog faces, and the received-dir
// prefix for the "beamed" badge. All networking lives in the Rust core; this
// provider only sequences IPC commands and `vlerv://beam-*` events.
//
// Two entry points create a send dialog: the toolbar button (user already
// acted — offer immediately) and the `vlerv://beam` deep link (a link is
// hostile until proven otherwise — show a confirm face first; nothing is
// staged until the user clicks through). The receive dialog is always
// confirm-first: nothing transfers before an explicit accept.
import * as React from "react";
import type { BeamOffer, BeamReceivedEntry, IpcSurface } from "../ipc";
import { useTauriEvent } from "../hooks/useTauriEvent";
import { useTabsDispatch } from "./TabsProvider";

/** Payload of `vlerv://beam-send-request` (CLI / deep-link initiated). */
interface BeamSendRequestPayload {
  path: string;
  name: string;
  size: number;
}

/** Payload of `vlerv://beam-receive-request`. `name` arrives pre-sanitized
 * by the backend; `sender_id_short` is the fingerprint the user verifies. */
export interface BeamReceiveRequestPayload {
  ticket: string;
  name: string;
  size: number | null;
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

export interface BeamContextValue {
  offers: BeamOffer[];
  received: BeamReceivedEntry[];
  receivedDir: string | null;
  send: BeamSendState | null;
  receive: BeamReceiveState | null;
  /** Toolbar path: stage + mint immediately. */
  beginSend(path: string): void;
  /** Deep-link path: the user confirmed the send dialog. */
  confirmSend(): void;
  closeSend(): void;
  acceptReceive(): void;
  declineReceive(): void;
  stopOffer(id: string): void;
  openReceived(path: string): void;
  refreshReceived(): void;
}

const BeamContext = React.createContext<BeamContextValue | null>(null);

export function useBeam(): BeamContextValue {
  const ctx = React.useContext(BeamContext);
  if (!ctx) throw new Error("useBeam must be used within BeamProvider");
  return ctx;
}

function basename(path: string): string {
  return path.split("/").pop() ?? path;
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

  React.useEffect(() => {
    let cancelled = false;
    ipc.beamReceivedDir?.().then((dir) => {
      if (!cancelled) setReceivedDir(dir);
    }).catch(() => {});
    ipc.beamListOffers?.().then((list) => {
      if (!cancelled) setOffers(list);
    }).catch(() => {});
    ipc.beamListReceived?.().then((list) => {
      if (!cancelled) setReceived(list);
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

  // Also the "Try again" action from the error face. Reads closure state —
  // side effects must stay out of setState updaters (StrictMode
  // double-invokes them, which would double-stage the offer).
  const confirmSend = React.useCallback(() => {
    if (send && (send.phase === "confirm" || send.phase === "error")) {
      runOffer(send.path, send.name, send.size);
    }
  }, [send, runOffer]);

  const closeSend = React.useCallback(() => setSend(null), []);

  const refreshReceived = React.useCallback(() => {
    ipc.beamListReceived?.().then(setReceived).catch(() => {});
  }, [ipc]);

  // Also the "Try again" action after a sender-offline error. Same closure-
  // state rule as confirmSend: one click, one fetch.
  const acceptReceive = React.useCallback(() => {
    if (!receive || receive.phase === "fetching") return;
    const { req } = receive;
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
  }, [receive, dispatch, ipc, refreshReceived]);

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

  const value = React.useMemo(
    (): BeamContextValue => ({
      offers,
      received,
      receivedDir,
      send,
      receive,
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
      offers, received, receivedDir, send, receive,
      beginSend, confirmSend, closeSend, acceptReceive, declineReceive,
      stopOffer, openReceived, refreshReceived,
    ],
  );

  return <BeamContext.Provider value={value}>{children}</BeamContext.Provider>;
}
