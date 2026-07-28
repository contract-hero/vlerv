// Tabs state — pure reducer, no React. Chrome-style tabs, each with its own
// back/forward history. Unit-testable in isolation (see tabs.test.ts).
import type { FilePayload } from "../ipc";

export interface HistoryEntry {
  path: string;
  /** True when the path lies outside the workspace root at navigation time. */
  external: boolean;
}

export interface ErrorPayload {
  error: { kind: string; path: string; reason: string };
}

export type TabContent = FilePayload | ErrorPayload | null;

export interface Tab {
  id: string;
  /** Empty history ⇒ start page. */
  history: HistoryEntry[];
  /** Index into `history`; -1 when history is empty. */
  historyIndex: number;
  /** Bumped to force a re-read of the current entry without a history push. */
  reloadNonce: number;
  /** Content zoom factor; 1 = 100%. */
  zoom: number;
  /** The watcher reported the current file removed from disk. */
  deleted: boolean;
  /** Last successfully loaded payload — kept while the next one loads so
   *  navigation doesn't flash a blank pane (browser-like). */
  payload: TabContent;
  /** Path `payload` corresponds to (null before first load). */
  loadedFor: string | null;
  /** reloadNonce `payload` corresponds to. */
  loadedNonce: number;
}

export interface TabsState {
  tabs: Tab[];
  /** Always references an existing tab; the state always has ≥ 1 tab. */
  activeTabId: string;
}

export const HISTORY_CAP = 50;
export const MIN_ZOOM = 0.25;
export const MAX_ZOOM = 3;
export const ZOOM_STEP = 0.1;

export type TabsAction =
  | { type: "NAVIGATE"; path: string; external: boolean; mode?: "push" | "replace" }
  | { type: "OPEN_NEW_TAB"; path?: string; external?: boolean; background?: boolean }
  | { type: "FOCUS_OR_OPEN"; path: string; external: boolean }
  | { type: "GO_BACK" }
  | { type: "GO_FORWARD" }
  | { type: "RELOAD"; tabId?: string }
  | { type: "CLOSE_TAB"; tabId: string }
  | { type: "ACTIVATE_TAB"; tabId: string }
  | { type: "ACTIVATE_INDEX"; index: number }
  | { type: "ACTIVATE_DELTA"; delta: 1 | -1 }
  | { type: "MOVE_TAB"; tabId: string; toIndex: number }
  | { type: "LOAD_SUCCESS"; tabId: string; path: string; nonce: number; payload: FilePayload }
  | { type: "LOAD_ERROR"; tabId: string; path: string; nonce: number; error: ErrorPayload["error"] }
  | { type: "FILE_CHANGED"; path: string }
  | { type: "FILE_REMOVED"; path: string }
  | { type: "SET_ZOOM"; tabId: string; zoom: number };

let nextTabSeq = 0;
export function makeTabId(): string {
  // crypto.randomUUID isn't available in every test environment; a
  // process-monotonic id is enough (ids never persist).
  nextTabSeq += 1;
  return `tab-${nextTabSeq}-${Math.random().toString(36).slice(2, 8)}`;
}

export function makeTab(entry?: HistoryEntry): Tab {
  return {
    id: makeTabId(),
    history: entry ? [entry] : [],
    historyIndex: entry ? 0 : -1,
    reloadNonce: 0,
    zoom: 1,
    deleted: false,
    payload: null,
    loadedFor: null,
    loadedNonce: -1,
  };
}

export function initialTabsState(): TabsState {
  const tab = makeTab();
  return { tabs: [tab], activeTabId: tab.id };
}

export function currentEntry(tab: Tab): HistoryEntry | null {
  return tab.history[tab.historyIndex] ?? null;
}

export function canGoBack(tab: Tab): boolean {
  return tab.historyIndex > 0;
}

export function canGoForward(tab: Tab): boolean {
  return tab.historyIndex < tab.history.length - 1;
}

export function activeTab(state: TabsState): Tab {
  const tab = state.tabs.find((t) => t.id === state.activeTabId);
  // The reducer maintains the ≥1-tab / valid-active invariants; the fallback
  // exists only to keep the type non-nullable.
  return tab ?? state.tabs[0];
}

function clampZoom(zoom: number): number {
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, Math.round(zoom * 100) / 100));
}

function updateTab(state: TabsState, tabId: string, update: (tab: Tab) => Tab): TabsState {
  return {
    ...state,
    tabs: state.tabs.map((t) => (t.id === tabId ? update(t) : t)),
  };
}

function pushEntry(tab: Tab, entry: HistoryEntry): Tab {
  const current = currentEntry(tab);
  if (current && current.path === entry.path) {
    // Same file: no history push, just clear a stale deleted flag and pick up
    // a possibly-changed external classification.
    return {
      ...tab,
      deleted: false,
      history: tab.history.map((h, i) => (i === tab.historyIndex ? entry : h)),
    };
  }
  let history = [...tab.history.slice(0, tab.historyIndex + 1), entry];
  let historyIndex = history.length - 1;
  if (history.length > HISTORY_CAP) {
    history = history.slice(history.length - HISTORY_CAP);
    historyIndex = history.length - 1;
  }
  return { ...tab, history, historyIndex, deleted: false };
}

function replaceEntry(tab: Tab, entry: HistoryEntry): Tab {
  if (tab.historyIndex < 0) return pushEntry(tab, entry);
  return {
    ...tab,
    deleted: false,
    history: tab.history.map((h, i) => (i === tab.historyIndex ? entry : h)),
  };
}

export function tabsReducer(state: TabsState, action: TabsAction): TabsState {
  switch (action.type) {
    case "NAVIGATE": {
      const entry: HistoryEntry = { path: action.path, external: action.external };
      return updateTab(state, state.activeTabId, (tab) =>
        action.mode === "replace" ? replaceEntry(tab, entry) : pushEntry(tab, entry),
      );
    }

    case "OPEN_NEW_TAB": {
      const entry = action.path
        ? { path: action.path, external: action.external ?? false }
        : undefined;
      const tab = makeTab(entry);
      const activeIdx = state.tabs.findIndex((t) => t.id === state.activeTabId);
      const tabs = [...state.tabs];
      // Insert after the active tab (browser convention).
      tabs.splice(activeIdx + 1, 0, tab);
      return {
        tabs,
        activeTabId: action.background ? state.activeTabId : tab.id,
      };
    }

    case "FOCUS_OR_OPEN": {
      const existing = state.tabs.find((t) => currentEntry(t)?.path === action.path);
      if (existing) {
        return { ...state, activeTabId: existing.id };
      }
      return tabsReducer(state, {
        type: "OPEN_NEW_TAB",
        path: action.path,
        external: action.external,
      });
    }

    case "GO_BACK":
      return updateTab(state, state.activeTabId, (tab) =>
        canGoBack(tab) ? { ...tab, historyIndex: tab.historyIndex - 1, deleted: false } : tab,
      );

    case "GO_FORWARD":
      return updateTab(state, state.activeTabId, (tab) =>
        canGoForward(tab) ? { ...tab, historyIndex: tab.historyIndex + 1, deleted: false } : tab,
      );

    case "RELOAD":
      return updateTab(state, action.tabId ?? state.activeTabId, (tab) =>
        currentEntry(tab) ? { ...tab, reloadNonce: tab.reloadNonce + 1 } : tab,
      );

    case "CLOSE_TAB": {
      const idx = state.tabs.findIndex((t) => t.id === action.tabId);
      if (idx < 0) return state;
      if (state.tabs.length === 1) {
        // Never zero tabs: closing the last tab replaces it with an empty one.
        const fresh = makeTab();
        return { tabs: [fresh], activeTabId: fresh.id };
      }
      const tabs = state.tabs.filter((t) => t.id !== action.tabId);
      let activeTabId = state.activeTabId;
      if (action.tabId === state.activeTabId) {
        // Activate the right neighbour, else the new last tab.
        const next = tabs[Math.min(idx, tabs.length - 1)];
        activeTabId = next.id;
      }
      return { tabs, activeTabId };
    }

    case "ACTIVATE_TAB":
      return state.tabs.some((t) => t.id === action.tabId)
        ? { ...state, activeTabId: action.tabId }
        : state;

    case "ACTIVATE_INDEX": {
      // -1 (⌘9 convention) means "last tab".
      const idx = action.index === -1 ? state.tabs.length - 1 : action.index;
      const tab = state.tabs[idx];
      return tab ? { ...state, activeTabId: tab.id } : state;
    }

    case "ACTIVATE_DELTA": {
      const idx = state.tabs.findIndex((t) => t.id === state.activeTabId);
      const next = (idx + action.delta + state.tabs.length) % state.tabs.length;
      return { ...state, activeTabId: state.tabs[next].id };
    }

    case "MOVE_TAB": {
      const from = state.tabs.findIndex((t) => t.id === action.tabId);
      if (from < 0) return state;
      const to = Math.max(0, Math.min(state.tabs.length - 1, action.toIndex));
      if (from === to) return state;
      const tabs = [...state.tabs];
      const [moved] = tabs.splice(from, 1);
      tabs.splice(to, 0, moved);
      return { ...state, tabs };
    }

    case "LOAD_SUCCESS":
      return updateTab(state, action.tabId, (tab) => {
        // Stale load (user already navigated elsewhere) — drop it.
        if (currentEntry(tab)?.path !== action.path) return tab;
        return {
          ...tab,
          payload: action.payload,
          loadedFor: action.path,
          loadedNonce: action.nonce,
          deleted: false,
        };
      });

    case "LOAD_ERROR":
      return updateTab(state, action.tabId, (tab) => {
        if (currentEntry(tab)?.path !== action.path) return tab;
        return {
          ...tab,
          payload: { error: action.error },
          loadedFor: action.path,
          loadedNonce: action.nonce,
        };
      });

    case "FILE_CHANGED":
      return {
        ...state,
        tabs: state.tabs.map((tab) =>
          currentEntry(tab)?.path === action.path
            ? { ...tab, reloadNonce: tab.reloadNonce + 1, deleted: false }
            : tab,
        ),
      };

    case "FILE_REMOVED":
      return {
        ...state,
        tabs: state.tabs.map((tab) =>
          currentEntry(tab)?.path === action.path ? { ...tab, deleted: true } : tab,
        ),
      };

    case "SET_ZOOM":
      return updateTab(state, action.tabId, (tab) => ({
        ...tab,
        zoom: clampZoom(action.zoom),
      }));

    default:
      return state;
  }
}

/** True when the active tab is waiting on a read for its current entry. */
export function isLoading(tab: Tab): boolean {
  const entry = currentEntry(tab);
  if (!entry) return false;
  return tab.loadedFor !== entry.path || tab.loadedNonce !== tab.reloadNonce;
}
