// Sidebar — workspace root selection + recursive Explorer.
import * as React from "react";
import type { IpcSurface } from "../ipc";
import Explorer from "./Explorer";

const WORKSPACE_ROOT_KEY = "vlerv.workspaceRoot";

export interface SidebarProps {
  ipc: IpcSurface;
  onSelectFile?: (path: string, external?: boolean) => void;
  selectedFile?: string | null;
  /**
   * Imperative trigger for the "Open File…" picker. The parent assigns this
   * ref so a keyboard shortcut (e.g. ⌘O in App) can invoke the same path as
   * clicking the button. Returns nothing — the picked path is delivered via
   * `onSelectFile(path, external)`.
   */
  openFileTrigger?: React.MutableRefObject<(() => void) | null>;
}

function readSavedRoot(): string | null {
  try {
    return globalThis.localStorage?.getItem(WORKSPACE_ROOT_KEY) ?? null;
  } catch {
    return null;
  }
}

function isUnderRoot(path: string, root: string | null): boolean {
  if (!root) return false;
  const normalized = root.endsWith("/") ? root : `${root}/`;
  return path === root || path.startsWith(normalized);
}

export default function Sidebar({
  ipc,
  onSelectFile,
  selectedFile,
  openFileTrigger,
}: SidebarProps): React.ReactElement {
  const [workspaceRoot, setWorkspaceRoot] = React.useState<string | null>(readSavedRoot());

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
          onClick={() => void handleOpenFile()}
          title="Open file… (⌘O)"
        >
          📄
        </button>
        <button className="sidebar-header-change" onClick={handleChange} title="Change workspace folder">
          ⤴
        </button>
      </div>
      <Explorer
        ipc={ipc}
        root={workspaceRoot}
        onSelectFile={(p) => onSelectFile?.(p, false)}
        selectedFile={selectedFile ?? null}
      />
    </div>
  );
}
