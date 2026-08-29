import { describe, expect, it } from "vitest";
import {
  activeTab,
  canGoBack,
  canGoForward,
  CLOSED_CAP,
  currentEntry,
  HISTORY_CAP,
  hydrateTabsState,
  TABS_SCHEMA,
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
  const restored = (o: ReturnType<typeof hydrateTabsState>) => {
    if (o.kind !== "restored") throw new Error(`expected restored, got ${o.kind}`);
    return o.state;
  };

  it("serializes only tabs that point somewhere, and marks the active one", () => {
    let s = nav(initialTabsState(), "/a");
    s = tabsReducer(s, { type: "OPEN_NEW_TAB", path: "/b" });
    s = tabsReducer(s, { type: "SET_ZOOM", tabId: s.activeTabId, zoom: 1.5 });
    s = tabsReducer(s, { type: "OPEN_NEW_TAB", background: true }); // empty tab

    const persisted = serializeTabs(s);
    expect(persisted.v).toBe(TABS_SCHEMA);
    expect(persisted.tabs.map((t) => t.history[t.historyIndex].path)).toEqual(["/a", "/b"]);
    expect(persisted.tabs[persisted.activeIndex].zoom).toBe(1.5);
  });

  it("activeIndex is the index into the serialized set, not the tab list", () => {
    // The app always starts with an empty start-page tab at index 0, so this
    // is the shape of nearly every real session: the two indices differ.
    let s = tabsReducer(initialTabsState(), { type: "OPEN_NEW_TAB", path: "/a" });
    s = tabsReducer(s, { type: "OPEN_NEW_TAB", path: "/b" });
    const aId = s.tabs[1].id;
    s = tabsReducer(s, { type: "ACTIVATE_TAB", tabId: aId });

    const persisted = serializeTabs(s);
    expect(persisted.activeIndex).toBe(0);
    expect(currentEntry(activeTab(restored(hydrateTabsState(persisted))))?.path).toBe("/a");
  });

  it("round-trips history, position, zoom and the external flag", () => {
    let s = nav(initialTabsState(), "/a", true);
    s = nav(s, "/b", true);
    s = tabsReducer(s, { type: "GO_BACK" });
    s = tabsReducer(s, { type: "SET_ZOOM", tabId: s.activeTabId, zoom: 1.2 });

    const r = restored(hydrateTabsState(serializeTabs(s)));
    expect(currentEntry(activeTab(r))?.path).toBe("/a");
    expect(canGoForward(activeTab(r))).toBe(true);
    expect(activeTab(r).zoom).toBe(1.2);
    // `external` drives out-of-root watch registration: lose it and live
    // reload dies silently for every restored external tab.
    expect(currentEntry(activeTab(r))?.external).toBe(true);
    // Payloads are deliberately NOT persisted: a restored tab re-reads disk.
    expect(activeTab(r).payload).toBeNull();
  });

  it("reports nothing to restore for junk and empty sessions", () => {
    expect(hydrateTabsState(null).kind).toBe("none");
    expect(hydrateTabsState({}).kind).toBe("none");
    expect(hydrateTabsState({ tabs: [] }).kind).toBe("none");
    expect(hydrateTabsState({ tabs: [{ history: [], historyIndex: -1, zoom: 1 }] }).kind).toBe("none");
    expect(hydrateTabsState({ tabs: [{ history: [{ nope: 1 }], historyIndex: 0, zoom: 1 }] }).kind).toBe("none");
  });

  it("refuses to read — and so to overwrite — a document from a newer build", () => {
    const outcome = hydrateTabsState({
      v: TABS_SCHEMA + 1,
      tabs: [{ history: [{ path: "/a", external: false }], historyIndex: 0, zoom: 1 }],
      activeIndex: 0,
    });
    expect(outcome.kind).toBe("unreadable");
  });

  it("clamps an out-of-range activeIndex, historyIndex and zoom", () => {
    const r = restored(hydrateTabsState({
      tabs: [{ history: [{ path: "/a", external: false }], historyIndex: 99, zoom: 99 }],
      activeIndex: 99,
    }));
    expect(currentEntry(activeTab(r))?.path).toBe("/a");
    expect(activeTab(r).zoom).toBe(3);
  });

  it("never restores a non-empty history parked on no entry", () => {
    // -1, missing, and non-numeric all used to survive, producing a tab that
    // holds history but shows the start page with Forward live.
    for (const historyIndex of [-1, undefined, "abc"]) {
      const r = restored(hydrateTabsState({
        tabs: [{ history: [{ path: "/a", external: false }], historyIndex, zoom: 1 }],
        activeIndex: 0,
      }));
      expect(currentEntry(activeTab(r))?.path).toBe("/a");
      expect(canGoForward(activeTab(r))).toBe(false);
    }
  });

  it("survives a fractional activeIndex instead of throwing on every launch", () => {
    const r = restored(hydrateTabsState({
      tabs: [
        { history: [{ path: "/a", external: false }], historyIndex: 0, zoom: 1 },
        { history: [{ path: "/b", external: false }], historyIndex: 0, zoom: 1 },
      ],
      activeIndex: 0.5,
    }));
    expect(currentEntry(activeTab(r))?.path).toBe("/a");
  });

  it("defaults a missing external flag to false rather than undefined", () => {
    const r = restored(hydrateTabsState({
      tabs: [{ history: [{ path: "/a" }], historyIndex: 0, zoom: 1 }],
      activeIndex: 0,
    }));
    expect(currentEntry(activeTab(r))?.external).toBe(false);
  });

  it("HYDRATE restores into a pristine session and keeps the closed stack", () => {
    let s = tabsReducer(initialTabsState(), { type: "OPEN_NEW_TAB", path: "/gone" });
    s = tabsReducer(s, { type: "CLOSE_TAB", tabId: s.activeTabId });
    const closedBefore = s.closed;

    s = tabsReducer(s, {
      type: "HYDRATE",
      persisted: {
        v: TABS_SCHEMA,
        tabs: [{ history: [{ path: "/restored", external: false }], historyIndex: 0, zoom: 1 }],
        activeIndex: 0,
      },
    });
    expect(s.tabs).toHaveLength(1);
    expect(currentEntry(activeTab(s))?.path).toBe("/restored");
    expect(s.closed).toEqual(closedBefore);
  });

  it("HYDRATE never discards a session the user already opened", () => {
    // A cold start from `vlerv open <path>` dispatches FOCUS_OR_OPEN before
    // the async getState() read lands. The user's file must win.
    let s = nav(initialTabsState(), "/deeplinked.md");
    s = tabsReducer(s, {
      type: "HYDRATE",
      persisted: {
        v: TABS_SCHEMA,
        tabs: [{ history: [{ path: "/old", external: false }], historyIndex: 0, zoom: 1 }],
        activeIndex: 0,
      },
    });
    expect(s.tabs.map((t) => currentEntry(t)?.path)).toEqual(["/deeplinked.md"]);
  });

  it("HYDRATE with a corrupt payload keeps the live session", () => {
    let s = nav(initialTabsState(), "/live");
    s = tabsReducer(s, { type: "HYDRATE", persisted: { tabs: "corrupt" } });
    expect(s.tabs).toHaveLength(1);
    expect(currentEntry(activeTab(s))?.path).toBe("/live");
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

describe("FOLLOW_NAVIGATE — follow mode's dedicated per-peer tab", () => {
  const PREFIX = "vlerv-remote://peer1";
  const A = `${PREFIX}/w/a.md`;
  const B = `${PREFIX}/w/b.md`;

  it("opens a new tab when no tab shows this peer yet", () => {
    let s = initialTabsState();
    s = nav(s, "/local.md");
    s = tabsReducer(s, { type: "FOLLOW_NAVIGATE", prefix: PREFIX, path: A });
    expect(s.tabs).toHaveLength(2);
    expect(currentEntry(activeTab(s))?.path).toBe(A);
    expect(currentEntry(activeTab(s))?.external).toBe(true);
  });

  it("reuses that tab on every later switch, leaving the local tab alone", () => {
    let s = initialTabsState();
    s = nav(s, "/local.md");
    const localId = s.activeTabId;
    s = tabsReducer(s, { type: "FOLLOW_NAVIGATE", prefix: PREFIX, path: A });
    const followId = s.activeTabId;
    // The user goes back to reading something local...
    s = tabsReducer(s, { type: "ACTIVATE_TAB", tabId: localId });
    // ...and the host switches tabs again.
    s = tabsReducer(s, { type: "FOLLOW_NAVIGATE", prefix: PREFIX, path: B });
    expect(s.tabs).toHaveLength(2);
    expect(s.activeTabId).toBe(followId);
    expect(currentEntry(s.tabs.find((t) => t.id === followId)!)?.path).toBe(B);
    // The local tab kept its own document.
    expect(currentEntry(s.tabs.find((t) => t.id === localId)!)?.path).toBe("/local.md");
  });

  it("keeps one tab per peer — another peer gets its own", () => {
    let s = initialTabsState();
    s = tabsReducer(s, { type: "FOLLOW_NAVIGATE", prefix: PREFIX, path: A });
    s = tabsReducer(s, {
      type: "FOLLOW_NAVIGATE",
      prefix: "vlerv-remote://peer2",
      path: "vlerv-remote://peer2/w/c.md",
    });
    expect(s.tabs.filter((t) => currentEntry(t) !== null)).toHaveLength(2);
  });

  it("pushes history in the followed tab, so back walks the host's trail", () => {
    let s = initialTabsState();
    s = tabsReducer(s, { type: "FOLLOW_NAVIGATE", prefix: PREFIX, path: A });
    s = tabsReducer(s, { type: "FOLLOW_NAVIGATE", prefix: PREFIX, path: B });
    expect(canGoBack(activeTab(s))).toBe(true);
  });
});
