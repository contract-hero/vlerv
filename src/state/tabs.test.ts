import { describe, expect, it } from "vitest";
import {
  activeTab,
  canGoBack,
  canGoForward,
  CLOSED_CAP,
  currentEntry,
  HISTORY_CAP,
  hydrateTabsState,
  initialTabsState,
  isLoading,
  serializeTabs,
  tabsReducer,
} from "./tabs";
import type { TabsState } from "./tabs";

function nav(state: TabsState, path: string, external = false): TabsState {
  return tabsReducer(state, { type: "NAVIGATE", path, external });
}

describe("tabsReducer", () => {
  it("starts with one empty tab (start page)", () => {
    const s = initialTabsState();
    expect(s.tabs).toHaveLength(1);
    expect(currentEntry(activeTab(s))).toBeNull();
  });

  it("NAVIGATE pushes history and truncates the forward stack", () => {
    let s = nav(initialTabsState(), "/a");
    s = nav(s, "/b");
    s = nav(s, "/c");
    s = tabsReducer(s, { type: "GO_BACK" });
    s = tabsReducer(s, { type: "GO_BACK" });
    expect(currentEntry(activeTab(s))?.path).toBe("/a");
    expect(canGoForward(activeTab(s))).toBe(true);

    s = nav(s, "/d");
    const tab = activeTab(s);
    expect(tab.history.map((h) => h.path)).toEqual(["/a", "/d"]);
    expect(canGoForward(tab)).toBe(false);
  });

  it("NAVIGATE to the same path is a no-op for history", () => {
    let s = nav(initialTabsState(), "/a");
    s = nav(s, "/a");
    expect(activeTab(s).history).toHaveLength(1);
  });

  it("NAVIGATE mode:replace overwrites the current entry, keeping history around it", () => {
    let s = nav(initialTabsState(), "/a");
    s = nav(s, "/b");
    s = tabsReducer(s, { type: "NAVIGATE", path: "/c", external: false, mode: "replace" });
    const tab = activeTab(s);
    expect(tab.history.map((h) => h.path)).toEqual(["/a", "/c"]);
    expect(tab.historyIndex).toBe(1);
    expect(canGoBack(tab)).toBe(true);
  });

  it("NAVIGATE mode:replace on an empty tab pushes instead", () => {
    const s = tabsReducer(initialTabsState(), {
      type: "NAVIGATE",
      path: "/a",
      external: false,
      mode: "replace",
    });
    const tab = activeTab(s);
    expect(tab.history.map((h) => h.path)).toEqual(["/a"]);
    expect(tab.historyIndex).toBe(0);
  });

  it("history is capped", () => {
    let s = initialTabsState();
    for (let i = 0; i < HISTORY_CAP + 10; i++) {
      s = nav(s, `/f${i}`);
    }
    const tab = activeTab(s);
    expect(tab.history).toHaveLength(HISTORY_CAP);
    expect(currentEntry(tab)?.path).toBe(`/f${HISTORY_CAP + 9}`);
  });

  it("GO_BACK / GO_FORWARD move the index and no-op at the edges", () => {
    let s = nav(initialTabsState(), "/a");
    s = nav(s, "/b");
    expect(canGoBack(activeTab(s))).toBe(true);
    s = tabsReducer(s, { type: "GO_BACK" });
    expect(currentEntry(activeTab(s))?.path).toBe("/a");
    s = tabsReducer(s, { type: "GO_BACK" });
    expect(currentEntry(activeTab(s))?.path).toBe("/a");
    s = tabsReducer(s, { type: "GO_FORWARD" });
    expect(currentEntry(activeTab(s))?.path).toBe("/b");
    s = tabsReducer(s, { type: "GO_FORWARD" });
    expect(currentEntry(activeTab(s))?.path).toBe("/b");
  });

  it("OPEN_NEW_TAB inserts after the active tab; background keeps focus", () => {
    let s = nav(initialTabsState(), "/a");
    const firstId = s.activeTabId;
    s = tabsReducer(s, { type: "OPEN_NEW_TAB", path: "/b", background: true });
    expect(s.activeTabId).toBe(firstId);
    expect(s.tabs).toHaveLength(2);
    s = tabsReducer(s, { type: "OPEN_NEW_TAB", path: "/c" });
    expect(s.activeTabId).not.toBe(firstId);
    // /c inserted directly after the active (first) tab, before /b.
    expect(s.tabs.map((t) => currentEntry(t)?.path)).toEqual(["/a", "/c", "/b"]);
  });

  it("FOCUS_OR_OPEN focuses an existing tab with the same path", () => {
    let s = nav(initialTabsState(), "/a");
    s = tabsReducer(s, { type: "OPEN_NEW_TAB", path: "/b" });
    s = tabsReducer(s, { type: "FOCUS_OR_OPEN", path: "/a", external: false });
    expect(currentEntry(activeTab(s))?.path).toBe("/a");
    expect(s.tabs).toHaveLength(2);
    s = tabsReducer(s, { type: "FOCUS_OR_OPEN", path: "/new", external: false });
    expect(s.tabs).toHaveLength(3);
    expect(currentEntry(activeTab(s))?.path).toBe("/new");
  });

  it("CLOSE_TAB keeps ≥1 tab and activates the right neighbour", () => {
    let s = nav(initialTabsState(), "/a");
    s = tabsReducer(s, { type: "OPEN_NEW_TAB", path: "/b" });
    s = tabsReducer(s, { type: "OPEN_NEW_TAB", path: "/c" });
    // tabs: a, b, c — active c. Activate b, close it → c should activate.
    const bId = s.tabs[1].id;
    s = tabsReducer(s, { type: "ACTIVATE_TAB", tabId: bId });
    s = tabsReducer(s, { type: "CLOSE_TAB", tabId: bId });
    expect(currentEntry(activeTab(s))?.path).toBe("/c");

    // Close everything: last close replaces with an empty tab.
    s = tabsReducer(s, { type: "CLOSE_TAB", tabId: s.tabs[0].id });
    s = tabsReducer(s, { type: "CLOSE_TAB", tabId: s.tabs[0].id });
    expect(s.tabs).toHaveLength(1);
    expect(currentEntry(activeTab(s))).toBeNull();
  });

  it("ACTIVATE_DELTA wraps around; ACTIVATE_INDEX -1 selects the last tab", () => {
    let s = nav(initialTabsState(), "/a");
    s = tabsReducer(s, { type: "OPEN_NEW_TAB", path: "/b" });
    s = tabsReducer(s, { type: "ACTIVATE_DELTA", delta: 1 });
    expect(currentEntry(activeTab(s))?.path).toBe("/a");
    s = tabsReducer(s, { type: "ACTIVATE_DELTA", delta: -1 });
    expect(currentEntry(activeTab(s))?.path).toBe("/b");
    s = tabsReducer(s, { type: "ACTIVATE_INDEX", index: -1 });
    expect(s.activeTabId).toBe(s.tabs[s.tabs.length - 1].id);
  });

  it("MOVE_TAB reorders", () => {
    let s = nav(initialTabsState(), "/a");
    s = tabsReducer(s, { type: "OPEN_NEW_TAB", path: "/b" });
    s = tabsReducer(s, { type: "OPEN_NEW_TAB", path: "/c" });
    const cId = s.tabs[2].id;
    s = tabsReducer(s, { type: "MOVE_TAB", tabId: cId, toIndex: 0 });
    expect(s.tabs.map((t) => currentEntry(t)?.path)).toEqual(["/c", "/a", "/b"]);
  });

  it("RELOAD bumps the nonce only when there is a file", () => {
    let s = initialTabsState();
    s = tabsReducer(s, { type: "RELOAD" });
    expect(activeTab(s).reloadNonce).toBe(0);
    s = nav(s, "/a");
    s = tabsReducer(s, { type: "RELOAD" });
    expect(activeTab(s).reloadNonce).toBe(1);
  });

  it("LOAD_SUCCESS is dropped when the tab navigated away", () => {
    let s = nav(initialTabsState(), "/a");
    const tabId = s.activeTabId;
    s = nav(s, "/b");
    s = tabsReducer(s, {
      type: "LOAD_SUCCESS",
      tabId,
      path: "/a",
      nonce: 0,
      payload: { path: "/a", size: 1, mtime: 0, is_binary: false, oversized: false, content: "x" },
    });
    expect(activeTab(s).payload).toBeNull();
    expect(isLoading(activeTab(s))).toBe(true);
  });

  it("LOAD_ERROR lands for the current path and is dropped when stale", () => {
    let s = nav(initialTabsState(), "/a");
    const tabId = s.activeTabId;
    s = tabsReducer(s, {
      type: "LOAD_ERROR",
      tabId,
      path: "/a",
      nonce: 0,
      error: { kind: "Io", path: "/a", reason: "boom" },
    });
    expect(activeTab(s).payload).toEqual({ error: { kind: "Io", path: "/a", reason: "boom" } });
    expect(isLoading(activeTab(s))).toBe(false);

    // Stale: navigated away before the error arrived.
    s = nav(s, "/b");
    s = tabsReducer(s, {
      type: "LOAD_ERROR",
      tabId,
      path: "/a",
      nonce: 0,
      error: { kind: "Io", path: "/a", reason: "stale" },
    });
    expect(isLoading(activeTab(s))).toBe(true);
    expect(activeTab(s).payload).toEqual({ error: { kind: "Io", path: "/a", reason: "boom" } });
  });

  it("FILE_CHANGED reloads matching tabs; FILE_REMOVED marks deleted; re-add clears", () => {
    let s = nav(initialTabsState(), "/a");
    s = tabsReducer(s, { type: "OPEN_NEW_TAB", path: "/b", background: true });
    s = tabsReducer(s, { type: "FILE_CHANGED", path: "/b" });
    const bTab = s.tabs.find((t) => currentEntry(t)?.path === "/b")!;
    expect(bTab.reloadNonce).toBe(1);
    expect(activeTab(s).reloadNonce).toBe(0);

    s = tabsReducer(s, { type: "FILE_REMOVED", path: "/b" });
    expect(s.tabs.find((t) => currentEntry(t)?.path === "/b")!.deleted).toBe(true);
    s = tabsReducer(s, { type: "FILE_CHANGED", path: "/b" });
    expect(s.tabs.find((t) => currentEntry(t)?.path === "/b")!.deleted).toBe(false);
  });

  it("SET_ZOOM clamps, quantizes float drift, and is per-tab", () => {
    let s = nav(initialTabsState(), "/a");
    s = tabsReducer(s, { type: "SET_ZOOM", tabId: s.activeTabId, zoom: 99 });
    expect(activeTab(s).zoom).toBe(3);
    s = tabsReducer(s, { type: "SET_ZOOM", tabId: s.activeTabId, zoom: 0.01 });
    expect(activeTab(s).zoom).toBe(0.25);
    // ⌘+ steps accumulate float error (0.1 + 0.2 === 0.30000000000000004);
    // the reducer must store exactly two decimals or the % indicator drifts.
    s = tabsReducer(s, { type: "SET_ZOOM", tabId: s.activeTabId, zoom: 0.1 + 0.2 });
    expect(activeTab(s).zoom).toBe(0.3);

    // Per-tab: zooming the active tab leaves others untouched, and an
    // unknown tabId is a no-op.
    s = tabsReducer(s, { type: "OPEN_NEW_TAB", path: "/b", background: true });
    const other = s.tabs.find((t) => t.id !== s.activeTabId)!;
    s = tabsReducer(s, { type: "SET_ZOOM", tabId: s.activeTabId, zoom: 2 });
    expect(s.tabs.find((t) => t.id === other.id)!.zoom).toBe(1);
    expect(tabsReducer(s, { type: "SET_ZOOM", tabId: "nope", zoom: 2 })).toEqual(s);
  });
});

describe("session persistence", () => {
  it("serializes only tabs that point somewhere, and marks the active one", () => {
    let s = nav(initialTabsState(), "/a");
    s = tabsReducer(s, { type: "OPEN_NEW_TAB", path: "/b" });
    s = tabsReducer(s, { type: "SET_ZOOM", tabId: s.activeTabId, zoom: 1.5 });
    s = tabsReducer(s, { type: "OPEN_NEW_TAB", background: true }); // empty tab

    const persisted = serializeTabs(s);
    expect(persisted.tabs.map((t) => t.history[t.historyIndex].path)).toEqual(["/a", "/b"]);
    expect(persisted.tabs[persisted.activeIndex].zoom).toBe(1.5);
  });

  it("round-trips history, position and zoom", () => {
    let s = nav(initialTabsState(), "/a");
    s = nav(s, "/b");
    s = tabsReducer(s, { type: "GO_BACK" });
    s = tabsReducer(s, { type: "SET_ZOOM", tabId: s.activeTabId, zoom: 1.2 });

    const restored = hydrateTabsState(serializeTabs(s))!;
    expect(currentEntry(activeTab(restored))?.path).toBe("/a");
    expect(canGoForward(activeTab(restored))).toBe(true);
    expect(activeTab(restored).zoom).toBe(1.2);
    // Payloads are deliberately NOT persisted: a restored tab re-reads disk.
    expect(activeTab(restored).payload).toBeNull();
  });

  it("refuses junk and empty sessions so the app falls back to a fresh tab", () => {
    expect(hydrateTabsState(null)).toBeNull();
    expect(hydrateTabsState({})).toBeNull();
    expect(hydrateTabsState({ tabs: [] })).toBeNull();
    expect(hydrateTabsState({ tabs: [{ history: [], historyIndex: -1, zoom: 1 }] })).toBeNull();
    expect(hydrateTabsState({ tabs: [{ history: [{ nope: 1 }], historyIndex: 0, zoom: 1 }] })).toBeNull();
  });

  it("clamps an out-of-range activeIndex and historyIndex", () => {
    const restored = hydrateTabsState({
      tabs: [{ history: [{ path: "/a", external: false }], historyIndex: 99, zoom: 99 }],
      activeIndex: 99,
    })!;
    expect(currentEntry(activeTab(restored))?.path).toBe("/a");
    expect(activeTab(restored).zoom).toBe(3);
  });

  it("HYDRATE replaces the session and keeps the closed stack", () => {
    let s = nav(initialTabsState(), "/old");
    s = tabsReducer(s, { type: "OPEN_NEW_TAB", path: "/gone" });
    s = tabsReducer(s, { type: "CLOSE_TAB", tabId: s.activeTabId });
    const closedBefore = s.closed;

    s = tabsReducer(s, {
      type: "HYDRATE",
      persisted: {
        tabs: [{ history: [{ path: "/restored", external: false }], historyIndex: 0, zoom: 1 }],
        activeIndex: 0,
      },
    });
    expect(s.tabs).toHaveLength(1);
    expect(currentEntry(activeTab(s))?.path).toBe("/restored");
    expect(s.closed).toEqual(closedBefore);
  });
});

describe("reopen closed tab", () => {
  it("restores the last closed tab with its history and zoom", () => {
    let s = nav(initialTabsState(), "/a");
    s = tabsReducer(s, { type: "OPEN_NEW_TAB", path: "/b" });
    s = nav(s, "/c");
    s = tabsReducer(s, { type: "SET_ZOOM", tabId: s.activeTabId, zoom: 2 });
    s = tabsReducer(s, { type: "CLOSE_TAB", tabId: s.activeTabId });
    expect(s.tabs).toHaveLength(1);

    s = tabsReducer(s, { type: "REOPEN_CLOSED_TAB" });
    expect(s.tabs).toHaveLength(2);
    const reopened = activeTab(s);
    expect(currentEntry(reopened)?.path).toBe("/c");
    expect(canGoBack(reopened)).toBe(true);
    expect(reopened.zoom).toBe(2);
    // The stack is consumed, so a second press does nothing.
    const after = tabsReducer(s, { type: "REOPEN_CLOSED_TAB" });
    expect(after.tabs).toHaveLength(2);
  });

  it("ignores empty tabs and caps the stack", () => {
    let s = initialTabsState();
    s = tabsReducer(s, { type: "OPEN_NEW_TAB" }); // empty — nothing to remember
    s = tabsReducer(s, { type: "CLOSE_TAB", tabId: s.activeTabId });
    expect(s.closed).toHaveLength(0);

    for (let i = 0; i < CLOSED_CAP + 3; i += 1) {
      s = tabsReducer(s, { type: "OPEN_NEW_TAB", path: `/f${i}` });
      s = tabsReducer(s, { type: "CLOSE_TAB", tabId: s.activeTabId });
    }
    expect(s.closed).toHaveLength(CLOSED_CAP);
    // Most-recently-closed first.
    expect(s.closed[0].history[0].path).toBe(`/f${CLOSED_CAP + 2}`);
  });
});
