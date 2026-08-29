// PhoneShell — the iOS layout. One column: a slim title band, the artifact
// full-bleed, and a bottom bar at the thumb. The desktop's sidebar / tab
// strip / toolbar stack never mounts here; Library (Remote + Received) and
// the open-tab list are bottom sheets summoned from the bar and dismissed
// by the scrim. Same tokens, same components inside the sheets — the phone
// changes the architecture, not the language.
import * as React from "react";
import {
  ChevronLeft,
  ChevronRight,
  LibraryBig,
  Plus,
  Settings as SettingsIcon,
  X,
} from "lucide-react";
import type { IpcSurface } from "../ipc";
import TabView from "./TabView";
import RemoteDrawerList from "./RemoteDrawer";
import ReceivedDrawer from "./ReceivedDrawer";
import { useActiveTab, useTabs, useTabsDispatch } from "../state/TabsProvider";
import type { OpenFileOptions } from "../state/TabsProvider";
import { canGoBack, canGoForward, currentEntry } from "../state/tabs";
import { useWatcherBus } from "../state/watcher-bus";
import { basename } from "../utils/path";

/** How long the title-band live dot glows after a cross-wire reload. */
const LIVE_PULSE_MS = 2400;

export interface PhoneShellProps {
  ipc: IpcSurface;
  onOpenFile: (path: string, opts?: OpenFileOptions) => void;
  onOpenSettings: () => void;
  onPickFile: () => void;
  onPickWorkspace: () => void;
  workspaceRoot: string | null;
}

type Sheet = null | "library" | "tabs";

export default function PhoneShell({
  ipc,
  onOpenFile,
  onOpenSettings,
  onPickFile,
  onPickWorkspace,
  workspaceRoot,
}: PhoneShellProps): React.ReactElement {
  const { tabs, activeTabId } = useTabs();
  const dispatch = useTabsDispatch();
  const active = useActiveTab();
  const entry = currentEntry(active);
  const [sheet, setSheet] = React.useState<Sheet>(null);
  const closeSheet = React.useCallback(() => setSheet(null), []);

  // The signature moment: the open artifact was just rewritten — locally or
  // on the paired Mac (remote FileChanged rides the same bus). The title
  // band's dot pulses once in the accent while the reload lands.
  const bus = useWatcherBus();
  const activePath = entry?.path ?? null;
  const [pulsing, setPulsing] = React.useState(false);
  const pulseTimer = React.useRef<number | null>(null);
  React.useEffect(() => {
    if (!activePath) return;
    const unsubscribe = bus.subscribe((change) => {
      if (change.path !== activePath || change.kind === "remove") return;
      setPulsing(true);
      if (pulseTimer.current) window.clearTimeout(pulseTimer.current);
      pulseTimer.current = window.setTimeout(() => setPulsing(false), LIVE_PULSE_MS);
    });
    return () => {
      unsubscribe();
      if (pulseTimer.current) window.clearTimeout(pulseTimer.current);
      // A path change inside the pulse window clears the timer before it can
      // fire, so reset here or the dot stays lit on the next artifact.
      setPulsing(false);
    };
  }, [bus, activePath]);

  // `basename` also does the right thing for a `vlerv-remote://<peer>/abs/path`
  // address: its strip is greedy, so what survives is the host-side filename.
  const title = activePath ? basename(activePath) : null;

  const openFromSheet = React.useCallback(
    (path: string, opts?: OpenFileOptions) => {
      onOpenFile(path, opts);
      closeSheet();
    },
    [onOpenFile, closeSheet],
  );

  return (
    <div className="phone-shell">
      <header className="phone-titlebar">
        <span
          className={"phone-live-dot" + (pulsing ? " is-live" : "")}
          title="Updated live"
          aria-hidden={!pulsing}
        />
        <span className={"phone-title" + (title ? "" : " phone-title-brand")}>
          {title ?? "Vlervtifacts"}
        </span>
      </header>

      <div
        className="phone-content"
        id="tab-panel"
        role="tabpanel"
        aria-labelledby={`tab-${active.id}`}
      >
        <TabView
          onOpenFile={onOpenFile}
          onPickFile={onPickFile}
          onPickWorkspace={onPickWorkspace}
          workspaceRoot={workspaceRoot}
          onOpenSettings={onOpenSettings}
        />
      </div>

      <nav className="phone-bar" aria-label="Reader controls">
        <button
          type="button"
          className="phone-bar-button"
          aria-label="Library"
          onClick={() => setSheet(sheet === "library" ? null : "library")}
        >
          <LibraryBig size={20} strokeWidth={1.8} />
        </button>
        <button
          type="button"
          className="phone-bar-button"
          aria-label="Back"
          disabled={!canGoBack(active)}
          onClick={() => dispatch({ type: "GO_BACK" })}
        >
          <ChevronLeft size={22} strokeWidth={1.8} />
        </button>
        <button
          type="button"
          className="phone-bar-button"
          aria-label="Forward"
          disabled={!canGoForward(active)}
          onClick={() => dispatch({ type: "GO_FORWARD" })}
        >
          <ChevronRight size={22} strokeWidth={1.8} />
        </button>
        <button
          type="button"
          className="phone-bar-button"
          aria-label={`Open tabs (${tabs.length})`}
          onClick={() => setSheet(sheet === "tabs" ? null : "tabs")}
        >
          <span className="phone-tab-count">{tabs.length}</span>
        </button>
      </nav>

      {sheet ? (
        <button
          type="button"
          className="phone-scrim"
          aria-label="Dismiss"
          onClick={closeSheet}
        />
      ) : null}

      {sheet === "library" ? (
        <PhoneSheet
          label="Library"
          title="Library"
          onClose={closeSheet}
          actions={
            <button
              type="button"
              className="phone-sheet-action"
              aria-label="Settings"
              onClick={() => {
                closeSheet();
                onOpenSettings();
              }}
            >
              <SettingsIcon size={17} strokeWidth={1.8} />
            </button>
          }
        >
          <RemoteDrawerList ipc={ipc} onOpenFile={openFromSheet} />
          <ReceivedDrawer onOpen={closeSheet} />
        </PhoneSheet>
      ) : null}

      {sheet === "tabs" ? (
        <PhoneSheet
          label="Open tabs"
          title="Tabs"
          onClose={closeSheet}
          actions={
            <button
              type="button"
              className="phone-sheet-action"
              aria-label="New tab"
              onClick={() => {
                dispatch({ type: "OPEN_NEW_TAB" });
                closeSheet();
              }}
            >
              <Plus size={17} strokeWidth={1.8} />
            </button>
          }
        >
          <ul className="phone-tab-list">
            {tabs.map((tab) => {
              const tabEntry = currentEntry(tab);
              const label = tabEntry ? basename(tabEntry.path) : "New tab";
              return (
                <li
                  key={tab.id}
                  className={
                    "phone-tab-row" + (tab.id === activeTabId ? " is-active" : "")
                  }
                >
                  <button
                    type="button"
                    className="phone-tab-row-label"
                    onClick={() => {
                      dispatch({ type: "ACTIVATE_TAB", tabId: tab.id });
                      closeSheet();
                    }}
                  >
                    <span className="phone-tab-row-name">{label}</span>
                    {tabEntry ? (
                      <span className="phone-tab-row-path">{tabEntry.path}</span>
                    ) : null}
                  </button>
                  <button
                    type="button"
                    className="phone-sheet-action"
                    aria-label={`Close ${label}`}
                    onClick={() => dispatch({ type: "CLOSE_TAB", tabId: tab.id })}
                  >
                    <X size={15} strokeWidth={1.8} />
                  </button>
                </li>
              );
            })}
          </ul>
        </PhoneSheet>
      ) : null}
    </div>
  );
}

/** The chrome both bottom sheets share: grab handle, head, and the scrolling
 *  body. Close is always present and always last, so the dismiss target sits
 *  in the same place in both sheets; `actions` is whatever that sheet adds
 *  before it. `label` is the accessible name, which can say more than the
 *  visible title has room for — the tabs sheet reads "Open tabs". */
function PhoneSheet({
  label,
  title,
  actions,
  onClose,
  children,
}: {
  label: string;
  title: string;
  actions?: React.ReactNode;
  onClose: () => void;
  children: React.ReactNode;
}): React.ReactElement {
  return (
    <div className="phone-sheet" role="dialog" aria-label={label}>
      <div className="phone-sheet-grab" aria-hidden />
      <div className="phone-sheet-head">
        <span className="phone-sheet-title">{title}</span>
        {actions}
        <button
          type="button"
          className="phone-sheet-action"
          aria-label="Close"
          onClick={onClose}
        >
          <X size={17} strokeWidth={1.8} />
        </button>
      </div>
      <div className="phone-sheet-body">{children}</div>
    </div>
  );
}
