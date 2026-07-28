import { describe, expect, it } from "vitest";
import {
  activeTab,
  canGoBack,
  canGoForward,
  currentEntry,
  HISTORY_CAP,
  initialTabsState,
  isLoading,
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

  it("SET_ZOOM clamps", () => {
    let s = nav(initialTabsState(), "/a");
    s = tabsReducer(s, { type: "SET_ZOOM", tabId: s.activeTabId, zoom: 99 });
    expect(activeTab(s).zoom).toBe(3);
    s = tabsReducer(s, { type: "SET_ZOOM", tabId: s.activeTabId, zoom: 0.01 });
    expect(activeTab(s).zoom).toBe(0.25);
  });
});
