// Root app — provider stack + browser-style chrome: TabStrip over
// Toolbar over TabView, with the Explorer sidebar on the left.
import * as React from "react";
import { flushSync } from "react-dom";
import { X } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import Sidebar from "./components/Sidebar";
import SidebarResizer from "./components/SidebarResizer";
import TabStrip from "./components/TabStrip";
import Toolbar from "./components/Toolbar";
import TabView from "./components/TabView";
import PhoneShell from "./components/PhoneShell";
import QuickOpen from "./components/QuickOpen";
import BeamDialog from "./components/BeamDialog";
import RemotePairDialog from "./components/RemotePairDialog";
import SettingsModal from "./components/SettingsModal";
import { tauriIpc } from "./ipc";
import type { IpcSurface } from "./ipc";
import { useDeepLink } from "./hooks/useDeepLink";
import type { OpenFilePayload, DeepLinkErrorPayload } from "./hooks/useDeepLink";
import { useTheme } from "./hooks/useTheme";
import { WorkspaceProvider, useWorkspace } from "./state/workspace";
import { WatcherProvider } from "./state/watcher-bus";
import { BookmarksProvider } from "./state/bookmarks-context";
import { RecentsProvider } from "./state/recents-context";
import { ScrollMemoryProvider } from "./state/scroll-memory";
import { ExplorerUiProvider, useExplorerUi } from "./state/explorer-ui";
import { BeamProvider } from "./state/beam";
import { RemoteProvider } from "./state/remote";
import { PlatformProvider, usePlatform } from "./state/platform";
import { ContextMenuProvider } from "./components/ContextMenu";
import { isUnderRoot } from "./utils/path";
import {
  TabsProvider,
  useActiveTab,
  useTabsDispatch,
  useOpenFile,
} from "./state/TabsProvider";
import { currentEntry, ZOOM_STEP } from "./state/tabs";
import { dispatchChord, useShortcuts } from "./keyboard/shortcuts";
import type { Binding, ChordEvent } from "./keyboard/shortcuts";

interface AppProps {
  ipc?: IpcSurface;
}

const DEFAULT_SIDEBAR_PX = 280;
const MIN_SIDEBAR_PX = 200;
const MAX_SIDEBAR_PX = 480;

/** How long a transient notice stays up before it dismisses itself. */
const NOTICE_MS = 10000;

// Chords honored when forwarded from a preview iframe: tab, nav, zoom and
// view-mode chords, plus bare Escape (bound only while reader mode is on) —
// nothing that opens native dialogs or steals focus. Must stay in sync with
// the FORWARD map in the injected script in src/render/html.tsx.
const IFRAME_FORWARDABLE = new Set([
  "mod+t", "mod+w", "mod+shift+t", "ctrl+tab", "ctrl+shift+tab",
  "mod+shift+bracketright", "mod+shift+bracketleft",
  "mod+bracketleft", "mod+bracketright", "mod+r",
  "mod+equal", "mod+shift+equal", "mod+minus",
  "mod+b", "mod+shift+f", "escape",
  ...Array.from({ length: 10 }, (_, i) => `mod+digit${i}`),
]);

function clampSidebarPx(px: number): number {
  return Math.max(MIN_SIDEBAR_PX, Math.min(MAX_SIDEBAR_PX, px));
}

/** The transient notice toast (a rejected deep link, so far). Identical on
 *  both platforms — only where it mounts differs: over the phone shell, or
 *  inside the desktop preview column under the toolbar. */
function AppNotice({
  text,
  onDismiss,
}: {
  text: string;
  onDismiss: () => void;
}): React.ReactElement {
  return (
    <div className="app-notice" role="alert">
      <span className="app-notice-text">{text}</span>
      <button
        type="button"
        className="app-notice-dismiss"
        title="Dismiss"
        aria-label="Dismiss notice"
        onClick={onDismiss}
      >
        <X size={13} strokeWidth={2} />
      </button>
    </div>
  );
}

export default function App({ ipc: injectedIpc }: AppProps = {}): React.ReactElement {
  const ipc = injectedIpc ?? tauriIpc;
  // Drives <html data-theme="…"> via side-effect; must mount at the root.
  useTheme();

  return (
    <PlatformProvider ipc={ipc}>
      <WorkspaceProvider ipc={ipc}>
        <ProviderShell ipc={ipc} />
      </WorkspaceProvider>
    </PlatformProvider>
  );
}

function ProviderShell({ ipc }: { ipc: IpcSurface }): React.ReactElement {
  const { root } = useWorkspace();
  return (
    <WatcherProvider ipc={ipc} root={root}>
      <BookmarksProvider ipc={ipc}>
        <RecentsProvider ipc={ipc}>
          <TabsProvider ipc={ipc}>
            <RemoteProvider ipc={ipc}>
              <BeamProvider ipc={ipc}>
                <ScrollMemoryProvider>
                  <ExplorerUiProvider>
                    <ContextMenuProvider>
                      <AppShell ipc={ipc} />
                    </ContextMenuProvider>
                  </ExplorerUiProvider>
                </ScrollMemoryProvider>
              </BeamProvider>
            </RemoteProvider>
          </TabsProvider>
        </RecentsProvider>
      </BookmarksProvider>
    </WatcherProvider>
  );
}

function AppShell({ ipc }: { ipc: IpcSurface }): React.ReactElement {
  const { root, setRoot } = useWorkspace();
  const { isMacos } = usePlatform();
  const dispatch = useTabsDispatch();
  const active = useActiveTab();
  const entry = currentEntry(active);
  const openFile = useOpenFile(ipc, root);
  const { reveal } = useExplorerUi();

  // Auto-reveal: keep the tree pointing at the active tab's file.
  const activePath = entry?.path ?? null;
  React.useEffect(() => {
    if (activePath && root && isUnderRoot(activePath, root)) {
      reveal(activePath, root);
    }
  }, [activePath, root, reveal]);

  const [sidebarPx, setSidebarPx] = React.useState<number>(DEFAULT_SIDEBAR_PX);
  const [sidebarVisible, setSidebarVisible] = React.useState<boolean>(true);
  // Reader mode: chrome down to tabs + document. Deliberately transient —
  // a reading posture, not a workspace setting, so it never persists.
  const [readerMode, setReaderMode] = React.useState<boolean>(false);
  const [refreshNonce, setRefreshNonce] = React.useState<number>(0);
  const [quickOpenVisible, setQuickOpenVisible] = React.useState(false);
  const [settingsOpen, setSettingsOpen] = React.useState(false);
  const [notice, setNotice] = React.useState<string | null>(null);
  const addressBarRef = React.useRef<HTMLInputElement | null>(null);
  const noticeTimer = React.useRef<number | null>(null);

  const dismissNotice = React.useCallback(() => {
    if (noticeTimer.current) window.clearTimeout(noticeTimer.current);
    noticeTimer.current = null;
    setNotice(null);
  }, []);

  const showNotice = React.useCallback((text: string) => {
    setNotice(text);
    if (noticeTimer.current) window.clearTimeout(noticeTimer.current);
    noticeTimer.current = window.setTimeout(() => setNotice(null), NOTICE_MS);
  }, []);

  // ── Persisted sidebar width ────────────────────────────────────────────
  React.useEffect(() => {
    if (!ipc.getState) return;
    let cancelled = false;
    ipc.getState().then((s) => {
      if (cancelled) return;
      const px = s?.panes?.sidebar_px;
      if (typeof px === "number" && px > 0) {
        setSidebarPx(clampSidebarPx(px));
      }
      // Don't clobber a ⌘B the user pressed before this snapshot resolved.
      if (!userToggledSidebar.current && s?.panes?.sidebar_visible === false) {
        setSidebarVisible(false);
      }
    }).catch(() => {
      // Backend not wired or state.json missing — keep the default width
      // and visibility.
    });
    return () => { cancelled = true; };
  }, [ipc]);

  const handleSidebarResizeCommit = React.useCallback((finalPx: number) => {
    if (ipc.setStateField) {
      void ipc.setStateField("panes.sidebar_px", clampSidebarPx(finalPx));
    }
  }, [ipc]);

  // ── View modes ─────────────────────────────────────────────────────────
  // Set true on the first ⌘B so the async getState() hydration can't clobber
  // a toggle that raced it.
  const userToggledSidebar = React.useRef(false);

  const persistSidebarVisible = React.useCallback((visible: boolean) => {
    ipc.setStateField?.("panes.sidebar_visible", visible).catch((e: unknown) => {
      console.error("vlerv: failed to persist sidebar visibility", e);
    });
  }, [ipc]);

  // ⌘B. In reader mode the sidebar is already gone, so the intuitive result
  // of "show me the sidebar" is to leave reader mode with the sidebar on.
  // No functional setState here: updaters must stay pure (StrictMode
  // double-invokes them), and the IPC write is a side effect.
  const toggleSidebar = React.useCallback(() => {
    userToggledSidebar.current = true;
    if (readerMode) {
      setReaderMode(false);
      setSidebarVisible(true);
      persistSidebarVisible(true);
      return;
    }
    const next = !sidebarVisible;
    setSidebarVisible(next);
    persistSidebarVisible(next);
  }, [persistSidebarVisible, readerMode, sidebarVisible]);

  const toggleReaderMode = React.useCallback(() => {
    setReaderMode((v) => !v);
  }, []);

  // ── Pickers ────────────────────────────────────────────────────────────
  const handlePickFile = React.useCallback(() => {
    if (!ipc.pickFile) return;
    void ipc.pickFile().then((picked) => {
      if (picked) openFile(picked);
    }).catch(() => {});
  }, [ipc, openFile]);

  const handlePickWorkspace = React.useCallback(() => {
    if (!ipc.pickDirectory) return;
    void ipc.pickDirectory().then((picked) => {
      if (picked) setRoot(picked);
    }).catch(() => {});
  }, [ipc, setRoot]);

  // Manual refresh: re-fetch every expanded tree folder AND reload the
  // active tab, without collapsing expansion state.
  const handleRefresh = React.useCallback(() => {
    setRefreshNonce((n) => n + 1);
    dispatch({ type: "RELOAD" });
  }, [dispatch]);

  // ── Deep links ─────────────────────────────────────────────────────────
  const handleDeepLinkIntent = React.useCallback(
    ({ path, intent, out_of_root, reader_mode }: OpenFilePayload) => {
      if (intent === "open") {
        dispatch({ type: "FOCUS_OR_OPEN", path, external: Boolean(out_of_root) });
        // Only a control-scoped peer's `OpenOnHost` carries this field. It
        // enters reader mode but never leaves it: `reader_mode: false` is
        // "no opinion", so a remote open cannot yank a local reader session
        // out from under the person sitting at this machine.
        if (reader_mode) setReaderMode(true);
      } else {
        // Reveal: expand + scroll to the file in the tree WITHOUT switching
        // the preview (per the deeplink.rs contract).
        reveal(path, root);
      }
    },
    [dispatch, reveal, root],
  );
  const handleDeepLinkError = React.useCallback(
    ({ reason, url }: DeepLinkErrorPayload) => {
      showNotice(`Deep link rejected: ${reason} (${url})`);
    },
    [showNotice],
  );
  useDeepLink({ onIntent: handleDeepLinkIntent, onError: handleDeepLinkError });

  // ── Keyboard shortcuts ─────────────────────────────────────────────────
  const focusAddressBar = React.useCallback(() => {
    // ⌘L must work from reader mode: leave it first. flushSync, not a timer —
    // the chord can arrive via the iframe message path, where nothing
    // guarantees a macrotask lands after the commit that re-mounts the bar.
    flushSync(() => setReaderMode(false));
    const input = addressBarRef.current;
    if (input) {
      input.focus();
      input.select();
    }
  }, []);

  const bindings: Binding[] = [
    { combo: "mod+t", allowInInput: true, handler: () => dispatch({ type: "OPEN_NEW_TAB" }) },
    { combo: "mod+w", allowInInput: true, handler: () => dispatch({ type: "CLOSE_TAB", tabId: active.id }) },
    { combo: "mod+shift+t", allowInInput: true, handler: () => dispatch({ type: "REOPEN_CLOSED_TAB" }) },
    { combo: "ctrl+tab", allowInInput: true, handler: () => dispatch({ type: "ACTIVATE_DELTA", delta: 1 }) },
    { combo: "ctrl+shift+tab", allowInInput: true, handler: () => dispatch({ type: "ACTIVATE_DELTA", delta: -1 }) },
    { combo: "mod+shift+bracketright", allowInInput: true, handler: () => dispatch({ type: "ACTIVATE_DELTA", delta: 1 }) },
    { combo: "mod+shift+bracketleft", allowInInput: true, handler: () => dispatch({ type: "ACTIVATE_DELTA", delta: -1 }) },
    ...Array.from({ length: 8 }, (_, i): Binding => ({
      combo: `mod+digit${i + 1}`,
      allowInInput: true,
      handler: () => dispatch({ type: "ACTIVATE_INDEX", index: i }),
    })),
    { combo: "mod+digit9", allowInInput: true, handler: () => dispatch({ type: "ACTIVATE_INDEX", index: -1 }) },
    { combo: "mod+bracketleft", handler: () => dispatch({ type: "GO_BACK" }) },
    { combo: "mod+bracketright", handler: () => dispatch({ type: "GO_FORWARD" }) },
    { combo: "mod+r", allowInInput: true, handler: handleRefreshActive },
    { combo: "mod+l", allowInInput: true, handler: focusAddressBar },
    // File picker (⌘O) and quick-open (⌘P) both act on a local workspace,
    // which doesn't exist on iOS (PRODUCT.md: the phone owns no files) — the
    // Sidebar and TabView hide their entry points there too.
    ...(isMacos
      ? [
          { combo: "mod+o", allowInInput: true, handler: handlePickFile } satisfies Binding,
          {
            combo: "mod+p",
            allowInInput: true,
            handler: () => setQuickOpenVisible((v) => !v),
          } satisfies Binding,
        ]
      : []),
    { combo: "mod+b", allowInInput: true, handler: toggleSidebar },
    { combo: "mod+shift+f", allowInInput: true, handler: toggleReaderMode },
    // Esc leaves reader mode. Registered only while the mode is on, so plain
    // Escape keeps its meaning everywhere else (QuickOpen, address bar). An
    // open context menu still owns Esc — it closes itself on the same window
    // event, and one keypress must not do both.
    ...(readerMode
      ? [{
          combo: "escape",
          handler: () => {
            if (document.querySelector(".context-menu")) return;
            setReaderMode(false);
          },
        } satisfies Binding]
      : []),
    { combo: "mod+equal", handler: () => zoomBy(ZOOM_STEP) },
    { combo: "mod+shift+equal", handler: () => zoomBy(ZOOM_STEP) },
    { combo: "mod+minus", handler: () => zoomBy(-ZOOM_STEP) },
    { combo: "mod+digit0", handler: () => dispatch({ type: "SET_ZOOM", tabId: active.id, zoom: 1 }) },
  ];

  function handleRefreshActive(): void {
    dispatch({ type: "RELOAD" });
  }

  function zoomBy(delta: number): void {
    dispatch({ type: "SET_ZOOM", tabId: active.id, zoom: active.zoom + delta });
  }

  useShortcuts(bindings);

  const showSidebar = sidebarVisible && !readerMode;

  // Keep a ref for the iframe-forwarded chord path.
  const bindingsRef = React.useRef<readonly Binding[]>(bindings);
  bindingsRef.current = bindings;

  // ── postMessage routing (iframe + markdown links, forwarded chords) ────
  React.useEffect(() => {
    const onMessage = (e: MessageEvent) => {
      const data = e.data as unknown;
      if (!data || typeof data !== "object") return;
      const d = data as {
        type?: unknown;
        path?: unknown;
        url?: unknown;
        meta?: unknown;
        shift?: unknown;
        middle?: unknown;
      };
      if (d.type === "vlerv:navigate" && typeof d.path === "string") {
        const newTab = Boolean(d.meta) || Boolean(d.middle);
        openFile(d.path, newTab ? { newTab: true, background: !d.shift } : undefined);
        return;
      }
      // External http(s) link from a rendered HTML/Markdown artifact: hand it
      // to the OS default browser. The opener plugin's capability is scoped to
      // http/https/mailto, so a malformed or unexpected scheme is rejected at
      // the Rust layer rather than silently navigating the host webview.
      if (d.type === "vlerv:openExternal" && typeof d.url === "string") {
        void openUrl(d.url).catch((err: unknown) => {
          console.error("vlerv: failed to open external URL", d.url, err);
        });
        return;
      }
      // Global chords forwarded from the focused HTML preview iframe. Only a
      // safe subset is honored — rendered artifact content is untrusted and
      // can postMessage this shape directly, so it must never synthesize a
      // chord that opens a native dialog (⌘O), seizes the address bar (⌘L),
      // or pops an overlay (⌘P).
      if (d.type === "vlerv:keydown") {
        dispatchChord(
          d as unknown as ChordEvent,
          bindingsRef.current.filter((b) => IFRAME_FORWARDABLE.has(b.combo)),
          false,
        );
        return;
      }
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, [openFile]);

  // Dialogs are summoned by events (a beam arriving, a pair link) or by a
  // gear button, never by the shell around them — so both architectures
  // mount the same set, at the same depth.
  const overlays = (
    <>
      <BeamDialog />
      <RemotePairDialog />
      {settingsOpen ? <SettingsModal ipc={ipc} onClose={() => setSettingsOpen(false)} /> : null}
    </>
  );

  // The phone gets its own architecture (PhoneShell): one column, bottom
  // bar, sheets. Overlays (dialogs, notice) mount the same either way.
  if (!isMacos) {
    return (
      <div className="app-shell">
        <PhoneShell
          ipc={ipc}
          onOpenFile={openFile}
          onOpenSettings={() => setSettingsOpen(true)}
          onPickFile={handlePickFile}
          onPickWorkspace={handlePickWorkspace}
          workspaceRoot={root}
        />
        {notice ? <AppNotice text={notice} onDismiss={dismissNotice} /> : null}
        {overlays}
      </div>
    );
  }

  return (
    // The sidebar is a full-height column; the tab strip lives inside the
    // preview pane, aligned with the reading field. On the left the sidebar
    // reserves the traffic-light space and drags the window — via its header
    // when a workspace is open, via the empty-state pane before that. The
    // strip keeps drag duty over the preview column, and takes the gutter
    // itself when the sidebar is hidden.
    <div className="app-shell">
      <div className="app">
        {showSidebar ? (
          <>
            <aside
              className="pane pane-sidebar"
              role="complementary"
              style={{ width: `${sidebarPx}px` }}
            >
              <Sidebar
                ipc={ipc}
                onOpenFile={openFile}
                selectedFile={entry?.path ?? null}
                onPickFile={handlePickFile}
                onPickWorkspace={handlePickWorkspace}
                onRefresh={handleRefresh}
                refreshNonce={refreshNonce}
                onOpenSettings={() => setSettingsOpen(true)}
              />
            </aside>
            <SidebarResizer
              width={sidebarPx}
              onResize={setSidebarPx}
              onCommit={handleSidebarResizeCommit}
            />
          </>
        ) : null}
        <main className="pane pane-preview" role="main">
          {/* With the sidebar gone the strip is the leftmost band, so it
              takes back the traffic-light gutter. */}
          <TabStrip onOpenFile={openFile} showGutter={!showSidebar} />
          {readerMode ? null : (
            <Toolbar
              addressBarRef={addressBarRef}
              onSubmitPath={(p) => openFile(p)}
              sidebarVisible={sidebarVisible}
              onToggleSidebar={toggleSidebar}
              onEnterReaderMode={toggleReaderMode}
            />
          )}
          {notice ? <AppNotice text={notice} onDismiss={dismissNotice} /> : null}
          <div
            className="tab-view"
            id="tab-panel"
            role="tabpanel"
            aria-labelledby={`tab-${active.id}`}
          >
            <TabView
              onOpenFile={openFile}
              onPickFile={handlePickFile}
              onPickWorkspace={handlePickWorkspace}
              workspaceRoot={root}
              onOpenSettings={() => setSettingsOpen(true)}
            />
          </div>
        </main>
      </div>
      {quickOpenVisible && root ? (
        <QuickOpen
          ipc={ipc}
          root={root}
          onOpenFile={openFile}
          onClose={() => setQuickOpenVisible(false)}
        />
      ) : null}
      {overlays}
    </div>
  );
}
