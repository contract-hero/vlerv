// TabsProvider — hosts the tabs reducer, the single file-loader effect, the
// watcher-driven auto-reload, and the external-file watch registration.
import * as React from "react";
import type { IpcSurface } from "../ipc";
import { isUnderRoot } from "../utils/path";
import type { OpenFileOptions } from "./open-opts";
import { useWatcherBus } from "./watcher-bus";
import {
  activeTab,
  currentEntry,
  hydrateTabsState,
  initialTabsState,
  isLoading,
  serializeTabs,
  tabsReducer,
} from "./tabs";
import type { TabsAction, TabsState } from "./tabs";

const TabsStateContext = React.createContext<TabsState>(initialTabsState());
const TabsDispatchContext = React.createContext<React.Dispatch<TabsAction>>(() => {});

export function useTabs(): TabsState {
  return React.useContext(TabsStateContext);
}

export function useTabsDispatch(): React.Dispatch<TabsAction> {
  return React.useContext(TabsDispatchContext);
}

export function useActiveTab() {
  const state = useTabs();
  return activeTab(state);
}

/** Debounce window for watcher-driven reloads (backend already debounces
 *  250 ms; this collapses bursts arriving across separate events). */
const RELOAD_DEBOUNCE_MS = 150;

/** Coalesce bursts of tab churn into one state.json write. */
const SESSION_PERSIST_MS = 400;

export interface TabsProviderProps {
  ipc: IpcSurface;
  children: React.ReactNode;
}

export function TabsProvider({ ipc, children }: TabsProviderProps): React.ReactElement {
  const [state, dispatch] = React.useReducer(tabsReducer, undefined, initialTabsState);
  const bus = useWatcherBus();

  // ── Loader: read the active tab's current entry whenever it changes ──────
  const active = activeTab(state);
  const entry = currentEntry(active);
  const loadSeq = React.useRef(0);

  React.useEffect(() => {
    if (!entry) return;
    if (!isLoading(active)) return;
    const seq = ++loadSeq.current;
    const { id: tabId, reloadNonce } = active;
    const path = entry.path;
    ipc
      .readFile(path)
      .then((payload) => {
        if (loadSeq.current !== seq) return;
        dispatch({ type: "LOAD_SUCCESS", tabId, path, nonce: reloadNonce, payload });
      })
      .catch((e: unknown) => {
        if (loadSeq.current !== seq) return;
        // Tauri rejects with the command's `Err` value, and `read_file` is
        // declared `Result<FilePayload, String>` — so this is usually a plain
        // string, not an Error. Reading `.message` off it gave `undefined`
        // and left the error screen with no detail at all.
        const reason = e instanceof Error ? e.message : String(e);
        dispatch({
          type: "LOAD_ERROR",
          tabId,
          path,
          nonce: reloadNonce,
          error: { kind: "Io", path, reason },
        });
      });
    // active/entry are derived from state; the identity that matters is
    // (tab id, path, nonce) — reruns on any of those.
  }, [ipc, active.id, entry?.path, active.reloadNonce, active.loadedFor, active.loadedNonce]); // eslint-disable-line react-hooks/exhaustive-deps

  // ── Session persistence ──────────────────────────────────────────────────
  // Window geometry already survived quit; the reading session did not. For a
  // custody-first tool that made quitting a data-loss event. Only WHERE each
  // tab points is stored — never its payload — so a restored tab re-reads
  // from disk and a file that changed or vanished meanwhile shows the truth.
  // Writing is gated on a SUCCESSFUL read. A failed read used to arm the
  // writer anyway, so one rejected `getState()` at launch could overwrite a
  // nine-tab session with the single empty tab the app fell back to. State,
  // not a ref, because the write effect has to re-run when the gate opens.
  const [writable, setWritable] = React.useState(false);

  React.useEffect(() => {
    if (!ipc.getState) {
      // No backend to read from and none to write to; nothing to protect.
      setWritable(true);
      return;
    }
    let cancelled = false;
    void ipc
      .getState()
      .then((s) => {
        if (cancelled) return;
        const persisted = s?.panes?.tabs;
        if (persisted) dispatch({ type: "HYDRATE", persisted });
        // A document a newer build wrote stays untouched: restore nothing and
        // never overwrite it.
        setWritable(hydrateTabsState(persisted).kind !== "unreadable");
      })
      .catch(() => {
        // Read failed. The document may still be intact, so leave `writable`
        // false: this run runs without session persistence rather than
        // destroying what it could not read.
      });
    return () => {
      cancelled = true;
    };
  }, [ipc]);

  // Serialize eagerly and carry the JSON alongside the value: `key` is what
  // the effect depends on, so a write is scheduled only when the session
  // genuinely changed. Payload loads and reload nonces churn `state` on every
  // read and must not each schedule a state.json write.
  const session = React.useMemo(() => {
    const value = serializeTabs(state);
    return { value, key: JSON.stringify(value) };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state.tabs, state.activeTabId]);

  React.useEffect(() => {
    if (!writable || !ipc.setStateField) return;
    const timer = window.setTimeout(() => {
      void ipc.setStateField?.("panes.tabs", session.value).catch(() => {});
    }, SESSION_PERSIST_MS);
    return () => window.clearTimeout(timer);
    // `session.key` is the stable identity of `session.value`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ipc, writable, session.key]);

  // Quitting inside the debounce window would drop the last change — close a
  // tab, press ⌘Q, and it comes back next launch. `pagehide` fires on webview
  // teardown, before the Rust side flushes its own debounce on exit.
  const pendingRef = React.useRef(session.value);
  pendingRef.current = session.value;
  React.useEffect(() => {
    if (!writable || !ipc.setStateField) return;
    const flush = () => {
      void ipc.setStateField?.("panes.tabs", pendingRef.current).catch(() => {});
    };
    window.addEventListener("pagehide", flush);
    return () => window.removeEventListener("pagehide", flush);
  }, [ipc, writable]);

  // ── Watcher-driven auto-reload ───────────────────────────────────────────
  const openPathsRef = React.useRef<Set<string>>(new Set());
  openPathsRef.current = new Set(
    state.tabs.map((t) => currentEntry(t)?.path).filter((p): p is string => Boolean(p)),
  );

  React.useEffect(() => {
    const timers = new Map<string, number>();
    const unsubscribe = bus.subscribe((change) => {
      if (!openPathsRef.current.has(change.path)) return;
      if (change.kind === "remove") {
        dispatch({ type: "FILE_REMOVED", path: change.path });
        return;
      }
      const existing = timers.get(change.path);
      if (existing) window.clearTimeout(existing);
      timers.set(
        change.path,
        window.setTimeout(() => {
          timers.delete(change.path);
          dispatch({ type: "FILE_CHANGED", path: change.path });
        }, RELOAD_DEBOUNCE_MS),
      );
    });
    return () => {
      unsubscribe();
      timers.forEach((t) => window.clearTimeout(t));
    };
  }, [bus]);

  // ── External-file watch registration ─────────────────────────────────────
  // The workspace watcher covers in-root files; every open out-of-root file
  // needs an individual watch. Re-register the full set whenever it changes.
  const externalPaths = React.useMemo(() => {
    const set = new Set<string>();
    for (const tab of state.tabs) {
      const e = currentEntry(tab);
      if (e?.external) set.add(e.path);
    }
    return Array.from(set).sort();
  }, [state.tabs]);
  const externalKey = externalPaths.join("\n");

  React.useEffect(() => {
    if (!ipc.watchExternalPaths) return;
    void ipc.watchExternalPaths(externalPaths).catch((e: unknown) => {
      console.warn("vlerv: failed to update external file watches", e);
    });
    // externalKey is the stable identity of externalPaths.
  }, [ipc, externalKey]); // eslint-disable-line react-hooks/exhaustive-deps

  return (
    <TabsStateContext.Provider value={state}>
      <TabsDispatchContext.Provider value={dispatch}>{children}</TabsDispatchContext.Provider>
    </TabsStateContext.Provider>
  );
}

export { openOptsFromClick } from "./open-opts";
export type { OpenFileOptions } from "./open-opts";

/**
 * The single producer funnel: tree clicks, bookmarks, address bar, ⌘O,
 * in-preview links, start page, and quick-open all route through this.
 */
export function useOpenFile(
  ipc: IpcSurface,
  workspaceRoot: string | null,
): (path: string, opts?: OpenFileOptions) => void {
  const dispatch = useTabsDispatch();
  return React.useCallback(
    (path: string, opts?: OpenFileOptions) => {
      const external = opts?.external ?? !isUnderRoot(path, workspaceRoot);
      if (opts?.newTab) {
        dispatch({ type: "OPEN_NEW_TAB", path, external, background: opts.background });
      } else {
        dispatch({ type: "NAVIGATE", path, external });
      }
      void ipc.pushRecent?.(path).catch(() => {});
    },
    [dispatch, ipc, workspaceRoot],
  );
}
