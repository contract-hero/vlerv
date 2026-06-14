// Sidebar — workspace root selection + path input bar + bookmarks list +
// recursive Explorer. The resize handle is rendered by App as a flex sibling
// of the sidebar pane so it doesn't get clipped by the pane's overflow.
import * as React from "react";
import type { IpcSurface } from "../ipc";
import Explorer from "./Explorer";
import Bookmarks from "./Bookmarks";
import { isUnderRoot, normalizePathBarInput } from "../utils/path";

const WORKSPACE_ROOT_KEY = "vlerv.workspaceRoot";

export interface SidebarProps {
  ipc: IpcSurface;
  onSelectFile?: (path: string, external?: boolean) => void;
  selectedFile?: string | null;
  /**
   * Imperative trigger for the "Open File…" picker. The parent assigns this
   * ref so a keyboard shortcut (e.g. ⌘O in App) can invoke the same path as
   * clicking the button.
   */
  openFileTrigger?: React.MutableRefObject<(() => void) | null>;
  /**
   * Path-input bar focus ref. The parent assigns this ref to wire ⌘L.
   */
  pathBarRef?: React.MutableRefObject<HTMLInputElement | null>;
  /**
   * Manual refresh trigger — reloads the explorer tree and the current
   * preview. Invoked by the Refresh button in the sidebar header.
   */
  onRefresh?: () => void;
  /**
   * Monotonic refresh counter forwarded to the Explorer so a manual refresh
   * re-fetches every visible folder without collapsing expansion state.
   */
  refreshNonce?: number;
}

function readSavedRoot(): string | null {
  try {
    return globalThis.localStorage?.getItem(WORKSPACE_ROOT_KEY) ?? null;
  } catch {
    return null;
  }
}

export default function Sidebar({
  ipc,
  onSelectFile,
  selectedFile,
  openFileTrigger,
  pathBarRef,
  onRefresh,
  refreshNonce,
}: SidebarProps): React.ReactElement {
  const [workspaceRoot, setWorkspaceRoot] = React.useState<string | null>(readSavedRoot());
  const [pathBarValue, setPathBarValue] = React.useState<string>("");
  const [pathBarError, setPathBarError] = React.useState<string | null>(null);

  const handlePick = async () => {
    if (!ipc.pickDirectory) return;
    try {
      const picked = await ipc.pickDirectory();
      if (!picked) return;
      ipc.setWorkspaceRoot?.(picked);
      setWorkspaceRoot(picked);
    } catch {
      // ignore
    }
  };

  const handleOpenFile = React.useCallback(async () => {
    if (!ipc.pickFile) return;
    try {
      const picked = await ipc.pickFile();
      if (!picked) return;
      onSelectFile?.(picked, !isUnderRoot(picked, workspaceRoot));
    } catch {
      // ignore
    }
  }, [ipc, onSelectFile, workspaceRoot]);

  React.useEffect(() => {
    if (!openFileTrigger) return;
    openFileTrigger.current = () => void handleOpenFile();
    return () => {
      openFileTrigger.current = null;
    };
  }, [openFileTrigger, handleOpenFile]);

  const handleChange = () => {
    try {
      globalThis.localStorage?.removeItem(WORKSPACE_ROOT_KEY);
    } catch {
      // ignore
    }
    setWorkspaceRoot(null);
  };

  const submitPathBar = React.useCallback(() => {
    const result = normalizePathBarInput(pathBarValue);
    if (result.kind === "error") {
      setPathBarError(result.reason);
      return;
    }
    setPathBarError(null);
    onSelectFile?.(result.path, !isUnderRoot(result.path, workspaceRoot));
    setPathBarValue("");
  }, [pathBarValue, onSelectFile, workspaceRoot]);

  const onPathBarKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") {
      e.preventDefault();
      submitPathBar();
    } else if (e.key === "Escape") {
      setPathBarValue("");
      setPathBarError(null);
      (e.target as HTMLInputElement).blur();
    }
  };

  const pathBar = (
    <div className="sidebar-path-bar">
      <input
        ref={pathBarRef ?? null}
        type="text"
        spellCheck={false}
        autoCapitalize="off"
        autoCorrect="off"
        placeholder="Enter file path… (⌘L)"
        value={pathBarValue}
        onChange={(e) => {
          setPathBarValue(e.target.value);
          if (pathBarError) setPathBarError(null);
        }}
        onKeyDown={onPathBarKeyDown}
        data-testid="path-bar-input"
      />
      {pathBarError ? (
        <span className="path-bar-error" role="alert">{pathBarError}</span>
      ) : null}
    </div>
  );

  if (!workspaceRoot) {
    return (
      <div className="sidebar-empty">
        <p className="sidebar-empty-text">Pick a folder to use as your workspace.</p>
        <button className="sidebar-button" onClick={() => void handlePick()}>
          Choose workspace folder…
        </button>
        <button
          className="sidebar-button"
          onClick={() => void handleOpenFile()}
          title="Open a single file (⌘O)"
        >
          Open file…
        </button>
        {pathBar}
      </div>
    );
  }

  const folderName = workspaceRoot.replace(/^.*\//, "") || workspaceRoot;

  return (
    <div className="sidebar">
      <div className="sidebar-header">
        <span className="sidebar-header-title" title={workspaceRoot}>{folderName}</span>
        <button
          className="sidebar-header-change"
          onClick={() => onRefresh?.()}
          title="Refresh files"
          aria-label="Refresh files"
        >
          ⟳
        </button>
        <button
          className="sidebar-header-change"
          onClick={() => void handleOpenFile()}
          title="Open file… (⌘O)"
        >
          📄
        </button>
        <button className="sidebar-header-change" onClick={handleChange} title="Change workspace folder">
          ⤴
        </button>
      </div>
      {pathBar}
      <Bookmarks
        ipc={ipc}
        onSelectFile={onSelectFile}
        workspaceRoot={workspaceRoot}
      />
      <Explorer
        ipc={ipc}
        root={workspaceRoot}
        onSelectFile={(p) => onSelectFile?.(p, false)}
        selectedFile={selectedFile ?? null}
        refreshNonce={refreshNonce ?? 0}
      />
    </div>
  );
}
