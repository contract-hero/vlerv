// Sidebar — workspace root selection + recursive Explorer.
import * as React from "react";
import type { IpcSurface } from "../ipc";
import Explorer from "./Explorer";

const WORKSPACE_ROOT_KEY = "vlerv.workspaceRoot";

export interface SidebarProps {
  ipc: IpcSurface;
  onSelectFile?: (path: string) => void;
  selectedFile?: string | null;
}

function readSavedRoot(): string | null {
  try {
    return globalThis.localStorage?.getItem(WORKSPACE_ROOT_KEY) ?? null;
  } catch {
    return null;
  }
}

export default function Sidebar({ ipc, onSelectFile, selectedFile }: SidebarProps): React.ReactElement {
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
      </div>
    );
  }

  const folderName = workspaceRoot.replace(/^.*\//, "") || workspaceRoot;

  return (
    <div className="sidebar">
      <div className="sidebar-header">
        <span className="sidebar-header-title" title={workspaceRoot}>{folderName}</span>
        <button className="sidebar-header-change" onClick={handleChange} title="Change workspace folder">
          ⤴
        </button>
      </div>
      <Explorer
        ipc={ipc}
        root={workspaceRoot}
        onSelectFile={onSelectFile}
        selectedFile={selectedFile ?? null}
      />
    </div>
  );
}
