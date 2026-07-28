// Sidebar — workspace header + bookmarks list + recursive Explorer. The
// address bar lives in the Toolbar above the preview; the resize handle is
// rendered by App as a flex sibling so it isn't clipped by pane overflow.
import * as React from "react";
import { FilePlus2, FolderOpen, RotateCw } from "lucide-react";
import type { IpcSurface } from "../ipc";
import Explorer from "./Explorer";
import Bookmarks from "./Bookmarks";
import { useWorkspace } from "../state/workspace";
import type { OpenFileOptions } from "../state/TabsProvider";

export interface SidebarProps {
  ipc: IpcSurface;
  onOpenFile: (path: string, opts?: OpenFileOptions) => void;
  selectedFile: string | null;
  onPickFile: () => void;
  onPickWorkspace: () => void;
  /**
   * Manual refresh trigger — reloads the explorer tree and the current
   * preview. Invoked by the Refresh button in the sidebar header.
   */
  onRefresh: () => void;
  /**
   * Monotonic refresh counter forwarded to the Explorer so a manual refresh
   * re-fetches every visible folder without collapsing expansion state.
   */
  refreshNonce: number;
}

export default function Sidebar({
  ipc,
  onOpenFile,
  selectedFile,
  onPickFile,
  onPickWorkspace,
  onRefresh,
  refreshNonce,
}: SidebarProps): React.ReactElement {
  const { root: workspaceRoot, clearRoot } = useWorkspace();

  if (!workspaceRoot) {
    return (
      <div className="sidebar-empty">
        <p className="sidebar-empty-text">Pick a folder to use as your workspace.</p>
        <button className="sidebar-button" onClick={onPickWorkspace}>
          Choose workspace folder…
        </button>
        <button
          className="sidebar-button"
          onClick={onPickFile}
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
          onClick={onRefresh}
          title="Refresh files"
          aria-label="Refresh files"
        >
          <RotateCw size={13} strokeWidth={2} />
        </button>
        <button
          className="sidebar-header-change"
          onClick={onPickFile}
          title="Open file… (⌘O)"
          aria-label="Open file"
        >
          <FilePlus2 size={13} strokeWidth={2} />
        </button>
        <button
          className="sidebar-header-change"
          onClick={clearRoot}
          title="Change workspace folder"
          aria-label="Change workspace folder"
        >
          <FolderOpen size={13} strokeWidth={2} />
        </button>
      </div>
      <Bookmarks onOpenFile={onOpenFile} />
      <Explorer
        ipc={ipc}
        root={workspaceRoot}
        onOpenFile={onOpenFile}
        selectedFile={selectedFile}
        refreshNonce={refreshNonce}
      />
    </div>
  );
}
