// RemoteProvider — Scope v2 (remote-control-design.html §6): trusted peers,
// presence, the pairing flow (both faces), and the live tab cache each
// online peer's drawer reads. Networking stays in the Rust core; this
// provider only sequences IPC commands and `vlerv://remote-*` events.
//
// Mounted INSIDE both TabsProvider and WatcherProvider (see App.tsx): it
// reads the tabs reducer to publish the open-tab list whenever that list
// changes, and it writes into the watcher bus to turn a wire FileChanged /
// FileRemoved into the same shape a local filesystem event has, so the
// tab-reload / scroll-restore machinery downstream never has to know a file
// lives on another Mac (Fig. 3 of the design).
import * as React from "react";
import type {
  IpcSurface,
  RemoteEvent,
  RemotePairInvite,
  RemotePeer,
  RemotePendingPair,
  RemotePresenceEvent,
  RemoteScope,
  RemoteTabEntry,
} from "../ipc";
import { useTauriEvent } from "../hooks/useTauriEvent";
import { nowSecs } from "../utils/beam-format";
import { formatRemoteAddress } from "../utils/remote-address";
import { dialsEveryPeer } from "./remote-reconnect";
import type { ReconnectReason } from "./remote-reconnect";
import { applyRemoteTabEvent, toPublishableTabs } from "./remote-tabs";
import { useWatcherBus } from "./watcher-bus";
import { useTabs } from "./TabsProvider";

/** A `vlerv://pair` deep link that arrived on this machine. Nothing is
 * dialed until the user proceeds — see `completePairing`. */
export interface RemotePairLinkArrival {
  peer: string;
  peer_short: string;
  device: string;
  ticket: string;
}

export interface RemoteStateValue {
  peers: RemotePeer[];
  /** Keyed by NodeId. Absent = never connected this session (not "offline"
   * — a peer can be perfectly reachable but simply not dialed yet). */
  presence: Record<string, RemotePresenceEvent>;
  /** Keyed by NodeId; only populated for a peer with an active subscription. */
  tabsByPeer: Record<string, RemoteTabEntry[]>;
  /** The peer whose active tab this instance mirrors, or null. */
  followingPeer: string | null;
  /** Host face: the link + this instance's identity, shown until dismissed. */
  invite: RemotePairInvite | null;
  /** Fingerprint step, on EITHER side (`role` tells them apart). */
  pendingConfirm: RemotePendingPair | null;
  /** A `vlerv://pair` link arrived; shows an inviting device before dialing. */
  pairLinkArrival: RemotePairLinkArrival | null;
  pairingBusy: boolean;
  pairingError: string | null;
}

export interface RemoteActionsValue {
  refreshPeers(): void;
  beginPairing(): void;
  dismissInvite(): void;
  /** Dial a ticket (from the address bar, a pasted link, or a `pair-link`
   * arrival) and park at the fingerprint step. */
  completePairing(ticket: string): void;
  dismissPairLink(): void;
  /** Resolve the parked pairing after the human compares the six words. */
  confirmPairing(accept: boolean, scope?: RemoteScope): void;
  unpair(nodeId: string): void;
  setScope(nodeId: string, scope: RemoteScope): void;
  toggleFollow(peer: string): void;
  /** Push an open intent onto a peer that granted US control scope (design
   * §6: "the literal remote control"). The peer is dialed lazily if there is
   * no live session yet. */
  openOnHost(peer: string, path: string, readerMode: boolean): void;
  /** Idempotent: safe to call from every drawer mount. */
  subscribePeer(peer: string): void;
  unsubscribePeer(peer: string): void;
}

const RemoteStateContext = React.createContext<RemoteStateValue | null>(null);
const RemoteActionsContext = React.createContext<RemoteActionsValue | null>(null);

export function useRemoteState(): RemoteStateValue {
  const ctx = React.useContext(RemoteStateContext);
  if (!ctx) throw new Error("useRemoteState must be used within RemoteProvider");
  return ctx;
}

export function useRemoteActions(): RemoteActionsValue {
  const ctx = React.useContext(RemoteActionsContext);
  if (!ctx) throw new Error("useRemoteActions must be used within RemoteProvider");
  return ctx;
}

export function RemoteProvider({
  ipc,
  children,
}: {
  ipc: IpcSurface;
  children: React.ReactNode;
}): React.ReactElement {
  const bus = useWatcherBus();
  const tabsState = useTabs();

  const [peers, setPeers] = React.useState<RemotePeer[]>([]);
  const [presence, setPresence] = React.useState<Record<string, RemotePresenceEvent>>({});
  const [tabsByPeer, setTabsByPeer] = React.useState<Record<string, RemoteTabEntry[]>>({});
  const [followingPeer, setFollowingPeer] = React.useState<string | null>(null);
  const [invite, setInvite] = React.useState<RemotePairInvite | null>(null);
  const [pendingConfirm, setPendingConfirm] = React.useState<RemotePendingPair | null>(null);
  const [pairLinkArrival, setPairLinkArrival] = React.useState<RemotePairLinkArrival | null>(null);
  const [pairingBusy, setPairingBusy] = React.useState(false);
  const [pairingError, setPairingError] = React.useState<string | null>(null);

  // ── Peers ──────────────────────────────────────────────────────────────
  // Every LATER refresh: a `peers-updated` event, a confirmed pairing, an
  // unpair that failed and has to be rolled back.
  const refreshPeers = React.useCallback(() => {
    ipc.remoteListPeers?.().then(setPeers).catch((e: unknown) => {
      console.error("vlerv: failed to list peers", e);
    });
  }, [ipc]);

  // False once the provider is gone, so a reconnect still in flight stops
  // writing state into a tree that unmounted under it. The mount effect's own
  // `cancelled` flag could not cover `reconnectAll`, which now also runs from
  // an event, long after that effect settled. Re-armed on mount because a
  // StrictMode double-mount runs the cleanup before the second mount.
  const mountedRef = React.useRef(true);
  React.useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  // Mount: ONE peer fetch feeds both the list and the launch-time reconnect.
  React.useEffect(() => {
    reconnectAll("launch");
    // This EFFECT is mount-only, which is not the same as the reconnect
    // being one-time: the `resumed` arm calls `reconnectAll` again on every
    // iOS foreground hop. What stays one-time is reading the preference —
    // flipping it mid-session doesn't retroactively dial the peers this
    // launch missed.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ipc]);

  useTauriEvent<RemotePresenceEvent>("vlerv://remote-presence", (p) => {
    setPresence((prev) => ({ ...prev, [p.peer]: p }));
    if (p.state === "offline") {
      // A dead session is not subscribed. Forgetting it here is what makes
      // "Connect" in Settings work a second time: `subscribePeer` short-
      // circuits on this set, and Settings only offers Connect while the
      // peer is offline — the exact state the stale entry would make dead.
      subscribedRef.current.delete(p.peer);
      // A dead session can't keep its drawer's tab list current — stale rows
      // are worse than an empty drawer.
      setTabsByPeer((prev) => {
        if (!(p.peer in prev)) return prev;
        const next = { ...prev };
        delete next[p.peer];
        return next;
      });
    }
  });

  useTauriEvent<RemoteEvent>("vlerv://remote-event", (event) => {
    switch (event.kind) {
      case "peers-updated":
        refreshPeers();
        break;
      case "resumed":
        // iOS froze this app, and the backend has just dropped every session
        // it held. The subscription set must go with them: `subscribePeer`
        // returns early for any peer still in it, so a single stale entry
        // turns the whole reconnect below into a silent no-op and that peer's
        // drawer never fills again.
        subscribedRef.current.clear();
        // "resume", not "launch": the line above threw the subscriptions
        // away, so a reconnect that consults `remote_listen` and finds it off
        // would re-establish nothing and leave every drawer empty for the
        // rest of the session.
        reconnectAll("resume");
        break;
      case "pair-pending":
        setPairingBusy(false);
        // A new pairing starts clean. `pairingError` is shared by both faces
        // of the dialog, so a leftover from an earlier failed dial would
        // otherwise render above this fingerprint as if it belonged to it.
        setPairingError(null);
        setPendingConfirm({
          node_id: event.peer,
          device: event.device,
          fingerprint: event.fingerprint,
          role: event.role,
          created_at: nowSecs(),
        });
        break;
      case "pair-link":
        setPairLinkArrival({
          peer: event.peer,
          peer_short: event.peer_short,
          device: event.device,
          ticket: event.ticket,
        });
        break;
      case "tab-opened":
      case "tab-closed":
      case "tab-activated":
        setTabsByPeer((prev) => ({
          ...prev,
          [event.peer]: applyRemoteTabEvent(prev[event.peer] ?? [], event),
        }));
        break;
      case "file-changed":
        // Live reload across the wire (design §6, Fig. 3): the downstream
        // debounce/reload/scroll-restore machinery in TabsProvider never
        // learns this event came off the network instead of `fsevents`.
        bus.publish({
          kind: "modify",
          path: formatRemoteAddress(event.peer, event.path),
          source: "remote",
        });
        break;
      case "file-removed":
        bus.publish({
          kind: "remove",
          path: formatRemoteAddress(event.peer, event.path),
          source: "remote",
        });
        break;
      default:
        break;
    }
  });

  // ── Pairing ────────────────────────────────────────────────────────────
  const beginPairing = React.useCallback(() => {
    setPairingError(null);
    setPairingBusy(true);
    const pending = ipc.remotePairBegin?.();
    if (!pending) {
      setPairingBusy(false);
      setPairingError("Remote pairing is not available in this build.");
      return;
    }
    pending
      .then((inv) => {
        setInvite(inv);
        setPairingBusy(false);
      })
      .catch((e: unknown) => {
        setPairingBusy(false);
        setPairingError(String(e));
      });
  }, [ipc]);

  const dismissInvite = React.useCallback(() => setInvite(null), []);

  const completePairing = React.useCallback(
    (ticket: string) => {
      setPairingError(null);
      setPairingBusy(true);
      const pending = ipc.remotePairComplete?.(ticket);
      if (!pending) {
        setPairingBusy(false);
        setPairingError("Remote pairing is not available in this build.");
        return;
      }
      pending
        .then((p) => {
          setPendingConfirm(p);
          setPairingBusy(false);
          setPairLinkArrival(null);
        })
        .catch((e: unknown) => {
          setPairingBusy(false);
          setPairingError(String(e));
        });
    },
    [ipc],
  );

  const dismissPairLink = React.useCallback(() => {
    setPairLinkArrival(null);
    // Clear the error with the dialog that owns it, or it reappears on the
    // next pairing surface.
    setPairingError(null);
  }, []);

  const confirmPairing = React.useCallback(
    (accept: boolean, scope?: RemoteScope) => {
      if (!pendingConfirm) return;
      const call = ipc.remotePairConfirm?.(pendingConfirm.node_id, accept, scope);
      if (!call) {
        setPairingError("Remote pairing is not available in this build.");
        return;
      }
      // Keep the dialog mounted until the call settles — it is the only
      // surface that renders `pairingError`, so clearing it up front would
      // hide a failed confirm and read as success while nothing was paired.
      setPairingBusy(true);
      call
        .then(() => {
          setPendingConfirm(null);
          setPairingBusy(false);
          refreshPeers();
        })
        .catch((e: unknown) => {
          setPairingBusy(false);
          setPairingError(`Pairing did not complete: ${String(e)}. The device is NOT paired.`);
        });
    },
    [ipc, pendingConfirm, refreshPeers],
  );

  const unpair = React.useCallback(
    (nodeId: string) => {
      // Optimistic: the design treats revocation as immediate.
      setPeers((list) => list.filter((p) => p.node_id !== nodeId));
      // The backend drops the session; drop our own subscription bookkeeping
      // too, so re-pairing the same device can subscribe again.
      subscribedRef.current.delete(nodeId);
      ipc.remoteUnpair?.(nodeId).catch((e: unknown) => {
        console.error("vlerv: failed to unpair", nodeId, e);
        refreshPeers();
      });
    },
    [ipc, refreshPeers],
  );

  const setScope = React.useCallback(
    (nodeId: string, scope: RemoteScope) => {
      setPeers((list) => list.map((p) => (p.node_id === nodeId ? { ...p, scope } : p)));
      ipc.remoteSetScope?.(nodeId, scope).catch((e: unknown) => {
        console.error("vlerv: failed to set scope", nodeId, e);
        refreshPeers();
      });
    },
    [ipc, refreshPeers],
  );

  const toggleFollow = React.useCallback((peer: string) => {
    setFollowingPeer((cur) => (cur === peer ? null : peer));
  }, []);

  const openOnHost = React.useCallback(
    (peer: string, path: string, readerMode: boolean) => {
      ipc.remoteOpenOnHost?.(peer, path, readerMode).catch((e: unknown) => {
        console.error("vlerv: failed to open on host", peer, path, e);
      });
    },
    [ipc],
  );

  // ── Live tab subscriptions ─────────────────────────────────────────────
  const subscribedRef = React.useRef<Set<string>>(new Set());

  const subscribePeer = React.useCallback(
    (peer: string) => {
      if (subscribedRef.current.has(peer)) return;
      subscribedRef.current.add(peer);
      const pending = ipc.remoteSubscribe?.(peer);
      if (!pending) {
        subscribedRef.current.delete(peer);
      } else {
        pending.catch((e: unknown) => {
          // Drop the optimistic entry so a later click can retry, instead of
          // short-circuiting on the poisoned set and leaving Connect dead.
          subscribedRef.current.delete(peer);
          console.error("vlerv: failed to subscribe to peer", peer, e);
        });
      }
      ipc.remoteListTabs?.(peer)
        .then((tabs) => setTabsByPeer((prev) => ({ ...prev, [peer]: tabs })))
        .catch((e: unknown) => console.error("vlerv: failed to list peer tabs", peer, e));
    },
    [ipc],
  );

  const unsubscribePeer = React.useCallback(
    (peer: string) => {
      if (!subscribedRef.current.has(peer)) return;
      subscribedRef.current.delete(peer);
      ipc.remoteUnsubscribe?.(peer).catch(() => {});
    },
    [ipc],
  );

  // Refresh the peer list and, when `dialsEveryPeer` allows it, dial every
  // paired peer. Two callers, and `reason` is what tells them apart: the
  // mount effect above ("launch"), and the `resumed` arm for the iOS
  // foreground hop ("resume") — the app is suspended and resumed far more
  // often than it is launched, and a resume is exactly when every session
  // this side holds has already died.
  //
  // At launch the dial follows design §4's lazy-boot rule, mirrored
  // client-side: ONLY when `preferences.remote_listen` is on do we
  // proactively dial every paired peer to learn who's online — otherwise a
  // paired-but-unvisited peer would never boot the endpoint on its own, and
  // the drawer for it would just never appear. With listening off, the
  // explicit "Connect" in Settings (or opening anything that calls
  // `subscribePeer`) is the action that boots it, same as any other remote
  // command. On the phone the backend re-dials its peers itself on resume, so
  // presence is restored either way — but presence is not a subscription, and
  // the tab list a drawer shows is.
  const reconnectAll = React.useCallback(
    (reason: ReconnectReason) => {
      if (!ipc.remoteListPeers) return;
      // A resume dials every peer whatever the preference says, so
      // `dialsEveryPeer` never reads it and this round trip would decide
      // nothing — while the peer list, and every drawer waiting on it, waits
      // for the answer. A resume is the frequent one, as the note above says.
      const settingsRead =
        reason === "resume"
          ? undefined
          : // A settings read that fails costs us the reconnect, never the
            // peer list — the preference only decides whether we dial.
            ipc.getState?.().catch(() => undefined);
      Promise.all([ipc.remoteListPeers(), settingsRead])
        .then(([list, settings]) => {
          if (!mountedRef.current) return;
          setPeers(list);
          if (!dialsEveryPeer(reason, settings?.preferences?.remote_listen ?? false)) return;
          list.forEach((p) => subscribePeer(p.node_id));
        })
        .catch((e: unknown) => {
          console.error("vlerv: failed to list peers", e);
        });
    },
    [ipc, subscribePeer],
  );

  // ── Host bridge: publish the open-tab list when that list changes ───────
  // Remote-address tabs are excluded: they aren't real filesystem paths, and
  // the backend's own canonicalize gate would silently drop them anyway
  // (scope.rs `canonical_tabs`) — filtering here just skips the round trip.
  //
  // Derived from the two fields that can move the list, not from the whole
  // reducer state: a reload, a zoom step and a scroll commit each produce a
  // new `tabsState` carrying the SAME open tabs.
  const publishableTabs = React.useMemo(
    () => toPublishableTabs(tabsState.tabs, tabsState.activeTabId),
    [tabsState.tabs, tabsState.activeTabId],
  );
  const publishKey = React.useMemo(() => JSON.stringify(publishableTabs), [publishableTabs]);

  React.useEffect(() => {
    if (!ipc.remotePublishTabs) return;
    void ipc.remotePublishTabs(publishableTabs).catch((e: unknown) => {
      console.error("vlerv: failed to publish tabs", e);
    });
    // Keyed on the serialized payload, not the array: the array is rebuilt
    // whenever the tabs array identity changes, and an unchanged key means a
    // byte-identical payload the host already has.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ipc, publishKey]);

  const stateValue = React.useMemo(
    (): RemoteStateValue => ({
      peers,
      presence,
      tabsByPeer,
      followingPeer,
      invite,
      pendingConfirm,
      pairLinkArrival,
      pairingBusy,
      pairingError,
    }),
    [
      peers, presence, tabsByPeer, followingPeer,
      invite, pendingConfirm, pairLinkArrival, pairingBusy, pairingError,
    ],
  );

  const actionsValue = React.useMemo(
    (): RemoteActionsValue => ({
      refreshPeers,
      beginPairing,
      dismissInvite,
      completePairing,
      dismissPairLink,
      confirmPairing,
      unpair,
      setScope,
      toggleFollow,
      openOnHost,
      subscribePeer,
      unsubscribePeer,
    }),
    [
      refreshPeers, beginPairing, dismissInvite, completePairing, dismissPairLink,
      confirmPairing, unpair, setScope, toggleFollow, openOnHost, subscribePeer, unsubscribePeer,
    ],
  );

  return (
    <RemoteActionsContext.Provider value={actionsValue}>
      <RemoteStateContext.Provider value={stateValue}>{children}</RemoteStateContext.Provider>
    </RemoteActionsContext.Provider>
  );
}
