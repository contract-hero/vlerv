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

/** The durable part of a tab: what it points at, not what it has loaded. */
export interface PersistedTab {
  history: HistoryEntry[];
  historyIndex: number;
  zoom: number;
}

/**
 * Wire shape of `panes.tabs` in state.json. One build writes this document and
 * another one reads it, so it carries a version: a reader that does not
 * recognise `v` restores nothing AND refuses to overwrite, rather than
 * silently rewriting a newer document in an older shape.
 */
export const TABS_SCHEMA = 1;

export interface PersistedTabs {
  v: typeof TABS_SCHEMA;
  tabs: PersistedTab[];
  activeIndex: number;
}

/**
 * What a stored session turned out to be.
 * - `restored`: use it.
 * - `none`: nothing worth restoring; safe to overwrite.
 * - `unreadable`: written by a build we do not understand; never overwrite.
 */
export type HydrateOutcome =
  | { kind: "restored"; state: TabsState }
  | { kind: "none" }
  | { kind: "unreadable" };

export interface TabsState {
  tabs: Tab[];
  /** Always references an existing tab; the state always has ≥ 1 tab. */
  activeTabId: string;
  /** Most-recently-closed first; the ⌘⇧T stack. Not persisted. */
  closed: PersistedTab[];
}

export const HISTORY_CAP = 50;
export const CLOSED_CAP = 10;
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
  | { type: "SET_ZOOM"; tabId: string; zoom: number }
  | { type: "REOPEN_CLOSED_TAB" }
  // Carries a document read off disk, not a validated value: the reducer
  // hands it to hydrateTabsState. Typing it as PersistedTabs would be a
  // promise the caller has to lie about.
  | { type: "HYDRATE"; persisted: unknown };

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
  return { tabs: [tab], activeTabId: tab.id, closed: [] };
}

export function toPersistedTab(tab: Tab): PersistedTab {
  return { history: tab.history, historyIndex: tab.historyIndex, zoom: tab.zoom };
}

function fromPersistedTab(p: PersistedTab): Tab {
  const history = Array.isArray(p.history)
    ? p.history.map(toHistoryEntry).filter((e): e is HistoryEntry => e !== null)
    : [];
  // A non-empty history must land on a real entry. A stored -1, a missing
  // field, or a non-number would otherwise restore a tab that holds history
  // but shows the start page, with Forward live and Back dead.
  const wanted = Number.isInteger(p.historyIndex) ? p.historyIndex : history.length - 1;
  const historyIndex =
    history.length === 0 ? -1 : Math.max(0, Math.min(history.length - 1, wanted));
  return {
    ...makeTab(),
    history,
    historyIndex,
    zoom: clampZoom(typeof p.zoom === "number" ? p.zoom : 1),
  };
}

/**
 * Normalize one stored history entry. A predicate would only *assert* that
 * `external` is a boolean; this returns a fresh entry where it actually is
 * one. That flag drives out-of-root watch registration, so an entry missing
 * it would restore a tab that never live-reloads.
 */
function toHistoryEntry(e: unknown): HistoryEntry | null {
  if (!e || typeof e !== "object") return null;
  const { path, external } = e as Partial<HistoryEntry>;
  return typeof path === "string" ? { path, external: external === true } : null;
}

/**
 * The reading session as it goes to disk. Payloads are deliberately dropped:
 * the file on disk is the truth, so a restored tab re-reads rather than
 * showing a stale snapshot.
 */
// `Pick` rather than the whole state: `closed` must never reach disk, and the
// compiler now enforces that instead of a comment. It also states the exact
// dependency set the provider's memo relies on.
export function serializeTabs(state: Pick<TabsState, "tabs" | "activeTabId">): PersistedTabs {
  const withHistory = state.tabs.filter((t) => t.history.length > 0);
  const activeIndex = withHistory.findIndex((t) => t.id === state.activeTabId);
  return {
    v: TABS_SCHEMA,
    tabs: withHistory.map(toPersistedTab),
    activeIndex: activeIndex < 0 ? 0 : activeIndex,
  };
}

/**
 * Rebuild a session from disk. The three outcomes are distinct on purpose:
 * "nothing to restore" is safe to overwrite, "unreadable" is not.
 */
export function hydrateTabsState(persisted: unknown): HydrateOutcome {
  if (!persisted || typeof persisted !== "object") return { kind: "none" };
  const raw = persisted as Partial<PersistedTabs>;
  // An absent `v` is the pre-versioning writer, which wrote v1.
  const version = raw.v ?? TABS_SCHEMA;
  if (version !== TABS_SCHEMA) return { kind: "unreadable" };
  if (!Array.isArray(raw.tabs)) return { kind: "none" };
  const tabs = raw.tabs
    .filter((t): t is PersistedTab => !!t && typeof t === "object")
    .map(fromPersistedTab)
    .filter((t) => t.history.length > 0);
  if (tabs.length === 0) return { kind: "none" };
  // Clamp AND floor: a fractional index would index past the array and make
  // `active` undefined, throwing inside the reducer on every launch.
  const rawIndex = typeof raw.activeIndex === "number" ? Math.floor(raw.activeIndex) : 0;
  const index = Number.isFinite(rawIndex) ? rawIndex : 0;
  const active = tabs[Math.max(0, Math.min(tabs.length - 1, index))] ?? tabs[0];
  return { kind: "restored", state: { tabs, activeTabId: active.id, closed: [] } };
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
        ...state,
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
      const victim = state.tabs[idx];
      // Closing is not a destroy: an empty tab has nothing to remember, but a
      // tab with history goes on the ⌘⇧T stack.
      const closed =
        victim.history.length > 0
          ? [toPersistedTab(victim), ...state.closed].slice(0, CLOSED_CAP)
          : state.closed;
      if (state.tabs.length === 1) {
        // Never zero tabs: closing the last tab replaces it with an empty one.
        const fresh = makeTab();
        return { tabs: [fresh], activeTabId: fresh.id, closed };
      }
      const tabs = state.tabs.filter((t) => t.id !== action.tabId);
      let activeTabId = state.activeTabId;
      if (action.tabId === state.activeTabId) {
        // Activate the right neighbour, else the new last tab.
        const next = tabs[Math.min(idx, tabs.length - 1)];
        activeTabId = next.id;
      }
      return { tabs, activeTabId, closed };
    }

    case "REOPEN_CLOSED_TAB": {
      const [restored, ...rest] = state.closed;
      if (!restored) return state;
      const tab = fromPersistedTab(restored);
      const activeIdx = state.tabs.findIndex((t) => t.id === state.activeTabId);
      const tabs = [...state.tabs];
      tabs.splice(activeIdx + 1, 0, tab);
      return { tabs, activeTabId: tab.id, closed: rest };
    }

    case "HYDRATE": {
      // Anything the user (or a deep link) already opened outranks the disk.
      // `getState()` is an async read and nothing blocks dispatches while it
      // is in flight, so a cold start from `vlerv open <path>` would otherwise
      // show the requested file and then replace it with the old session.
      if (state.tabs.some((t) => t.history.length > 0)) return state;
      const outcome = hydrateTabsState(action.persisted);
      return outcome.kind === "restored"
        ? { ...outcome.state, closed: state.closed }
        : state;
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
