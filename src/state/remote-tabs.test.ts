import { describe, expect, it } from "vitest";
import {
  activeTabPath,
  applyRemoteTabEvent,
  shouldFollowNavigate,
  toPublishableTabs,
} from "./remote-tabs";
import { makeTab, tabsReducer } from "./tabs";
import type { Tab } from "./tabs";
import type { RemoteTabEntry } from "../ipc";

function tabAt(path: string): Tab {
  const tab = makeTab();
  const state = tabsReducer(
    { tabs: [tab], activeTabId: tab.id, closed: [] },
    { type: "NAVIGATE", path, external: false },
  );
  return state.tabs[0];
}

describe("applyRemoteTabEvent — remote state reducer basics", () => {
  it("tab-opened adds a new, inactive entry", () => {
    const next = applyRemoteTabEvent([], { kind: "tab-opened", peer: "p", path: "/a.md" });
    expect(next).toEqual([{ path: "/a.md", active: false }]);
  });

  it("tab-opened is idempotent for an already-known path", () => {
    const tabs: RemoteTabEntry[] = [{ path: "/a.md", active: true }];
    expect(applyRemoteTabEvent(tabs, { kind: "tab-opened", peer: "p", path: "/a.md" })).toBe(tabs);
  });

  it("tab-closed removes the matching entry only", () => {
    const tabs: RemoteTabEntry[] = [
      { path: "/a.md", active: false },
      { path: "/b.md", active: true },
    ];
    expect(applyRemoteTabEvent(tabs, { kind: "tab-closed", peer: "p", path: "/a.md" })).toEqual([
      { path: "/b.md", active: true },
    ]);
  });

  it("tab-activated flips the active flag and clears every other one", () => {
    const tabs: RemoteTabEntry[] = [
      { path: "/a.md", active: true },
      { path: "/b.md", active: false },
    ];
    expect(applyRemoteTabEvent(tabs, { kind: "tab-activated", peer: "p", path: "/b.md" })).toEqual([
      { path: "/a.md", active: false },
      { path: "/b.md", active: true },
    ]);
  });

  it("tab-activated for an unseen path adds it as active (race tolerance)", () => {
    const next = applyRemoteTabEvent([{ path: "/a.md", active: true }], {
      kind: "tab-activated",
      peer: "p",
      path: "/b.md",
    });
    expect(next).toEqual([
      { path: "/a.md", active: false },
      { path: "/b.md", active: true },
    ]);
  });
});

describe("activeTabPath", () => {
  it("finds the active entry", () => {
    expect(
      activeTabPath([
        { path: "/a.md", active: false },
        { path: "/b.md", active: true },
      ]),
    ).toBe("/b.md");
  });

  it("is null when nothing is active", () => {
    expect(activeTabPath([{ path: "/a.md", active: false }])).toBeNull();
    expect(activeTabPath([])).toBeNull();
  });
});

describe("shouldFollowNavigate — follow-mode activation logic", () => {
  it("fires only when following, a target exists, and it actually changed", () => {
    expect(shouldFollowNavigate(null, "/a.md", true)).toBe(true);
    expect(shouldFollowNavigate("/a.md", "/b.md", true)).toBe(true);
  });

  it("does not fire when not following", () => {
    expect(shouldFollowNavigate(null, "/a.md", false)).toBe(false);
  });

  it("does not fire when the active path is unchanged", () => {
    expect(shouldFollowNavigate("/a.md", "/a.md", true)).toBe(false);
  });

  it("does not fire when there is no active path to follow", () => {
    expect(shouldFollowNavigate("/a.md", null, true)).toBe(false);
  });
});

describe("toPublishableTabs — tab-publish effect shape", () => {
  it("publishes each tab's current entry with exactly one active flag", () => {
    const tabs = [tabAt("/a.md"), tabAt("/b.md")];
    const result = toPublishableTabs(tabs, tabs[1].id);
    expect(result).toEqual([
      { path: "/a.md", active: false },
      { path: "/b.md", active: true },
    ]);
  });

  it("drops tabs with no history (the start page)", () => {
    const empty = makeTab();
    const open = tabAt("/a.md");
    const result = toPublishableTabs([empty, open], open.id);
    expect(result).toEqual([{ path: "/a.md", active: true }]);
  });

  it("excludes vlerv-remote:// tabs — they aren't real filesystem paths", () => {
    const remote = tabAt("vlerv-remote://peer/w/report.html");
    const local = tabAt("/a.md");
    const result = toPublishableTabs([remote, local], local.id);
    expect(result).toEqual([{ path: "/a.md", active: true }]);
  });
});
