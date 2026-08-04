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
      .catch((e: Error) => {
        if (loadSeq.current !== seq) return;
        dispatch({
          type: "LOAD_ERROR",
          tabId,
          path,
          nonce: reloadNonce,
          error: { kind: "Io", path, reason: e.message },
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
  const hydratedRef = React.useRef(false);

  React.useEffect(() => {
    if (!ipc.getState) {
      hydratedRef.current = true;
      return;
    }
    let cancelled = false;
    void ipc
      .getState()
      .then((s) => {
        if (cancelled) return;
        const persisted = s?.panes?.tabs;
        if (persisted) dispatch({ type: "HYDRATE", persisted: persisted as never });
      })
      .catch(() => {
        // No backend or no state.json — start with the fresh empty tab.
      })
      .finally(() => {
        if (!cancelled) hydratedRef.current = true;
      });
    return () => {
      cancelled = true;
    };
  }, [ipc]);

  // Serialize eagerly so the effect below only fires on a REAL session change;
  // payload loads and reload nonces churn the state constantly and must not
  // schedule a write.
  const sessionJson = React.useMemo(
    () => JSON.stringify(serializeTabs(state)),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [state.tabs, state.activeTabId],
  );

  React.useEffect(() => {
    if (!hydratedRef.current || !ipc.setStateField) return;
    const timer = window.setTimeout(() => {
      void ipc.setStateField?.("panes.tabs", JSON.parse(sessionJson)).catch(() => {});
    }, SESSION_PERSIST_MS);
    return () => window.clearTimeout(timer);
  }, [ipc, sessionJson]);

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
