// Pure diff logic for a peer's live tab list (design §6: the client mirrors
// the host's TabOpened / TabClosed / TabActivated events). Kept dependency-
// free so the drawer's live-tab behaviour is unit-testable without a
// provider, an IPC mock, or an event listener.
import type { RemoteEvent, RemoteTabEntry } from "../ipc";
import { isRemoteAddress } from "../utils/remote-address";
import { currentEntry } from "./tabs";
import type { Tab } from "./tabs";

type TabEvent = Extract<RemoteEvent, { kind: "tab-opened" | "tab-closed" | "tab-activated" }>;

/** Apply one tab event to a peer's cached tab list. Unknown paths are
 * tolerant: a `tab-activated` for a path the client hasn't seen `tab-opened`
 * for yet (a race between the initial `ListTabs` fetch and the subscription
 * starting) still lands as a new, active entry rather than being dropped. */
export function applyRemoteTabEvent(tabs: RemoteTabEntry[], event: TabEvent): RemoteTabEntry[] {
  switch (event.kind) {
    case "tab-opened":
      return tabs.some((t) => t.path === event.path)
        ? tabs
        : [...tabs, { path: event.path, active: false }];
    case "tab-closed":
      return tabs.filter((t) => t.path !== event.path);
    case "tab-activated": {
      const has = tabs.some((t) => t.path === event.path);
      const withEntry = has ? tabs : [...tabs, { path: event.path, active: false }];
      return withEntry.map((t) => ({ ...t, active: t.path === event.path }));
    }
    default:
      return tabs;
  }
}

/** The host's currently active tab, if any — what the drawer highlights and
 * what follow mode mirrors. */
export function activeTabPath(tabs: RemoteTabEntry[]): string | null {
  return tabs.find((t) => t.active)?.path ?? null;
}

/**
 * Follow mode's whole decision: open/focus the new active path only when
 * following THIS peer and the active path actually changed. Isolated as a
 * pure predicate so "does follow mode fire" is testable without mounting
 * the drawer or faking a live session.
 */
/**
 * The host bridge's whole payload shape (design §6: "a thin effect publishes
 * the open-tab list to the backend on every commit"). Pure so the shape is
 * testable without a provider or a fake IPC surface: remote-address tabs are
 * excluded — they aren't real filesystem paths, and the backend's own
 * `canonical_tabs` gate would silently drop them anyway (scope.rs) — and
 * exactly one entry is `active`, the current entry of the active tab.
 */
export function toPublishableTabs(tabs: Tab[], activeTabId: string): RemoteTabEntry[] {
  return tabs
    .map((t) => ({ entry: currentEntry(t), active: t.id === activeTabId }))
    .filter(
      (x): x is { entry: NonNullable<ReturnType<typeof currentEntry>>; active: boolean } =>
        x.entry !== null && !isRemoteAddress(x.entry.path),
    )
    .map((x) => ({ path: x.entry.path, active: x.active }));
}

export function shouldFollowNavigate(
  previousActivePath: string | null,
  nextActivePath: string | null,
  isFollowing: boolean,
): boolean {
  return isFollowing && nextActivePath !== null && nextActivePath !== previousActivePath;
}
