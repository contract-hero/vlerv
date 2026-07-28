// TabsProvider — hosts the tabs reducer, the single file-loader effect, the
// watcher-driven auto-reload, and the external-file watch registration.
import * as React from "react";
import type { IpcSurface } from "../ipc";
import { isUnderRoot } from "../utils/path";
import { useWatcherBus } from "./watcher-bus";
import {
  activeTab,
  currentEntry,
  initialTabsState,
  isLoading,
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

export interface OpenFileOptions {
  newTab?: boolean;
  background?: boolean;
  /** Override the computed in-root/external classification. */
  external?: boolean;
}

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

/**
 * Standard modifier interpretation for row/link clicks:
 * ⌘-click → background new tab, ⌘⇧-click → foreground new tab,
 * middle-click → background new tab.
 */
export function openOptsFromClick(e: {
  metaKey?: boolean;
  ctrlKey?: boolean;
  shiftKey?: boolean;
  button?: number;
}): OpenFileOptions | undefined {
  const meta = Boolean(e.metaKey || e.ctrlKey);
  const middle = e.button === 1;
  if (!meta && !middle) return undefined;
  return { newTab: true, background: !e.shiftKey };
}
