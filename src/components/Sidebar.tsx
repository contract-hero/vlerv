// Sidebar — workspace header + bookmarks list + recursive Explorer. The
// address bar lives in the Toolbar above the preview; the resize handle is
// rendered by App as a flex sibling so it isn't clipped by pane overflow.
import * as React from "react";
import { FilePlus2, FolderOpen, RotateCw } from "lucide-react";
import type { IpcSurface } from "../ipc";
import Explorer from "./Explorer";
import Bookmarks from "./Bookmarks";
import RecentFiles from "./RecentFiles";
import SidebarSection from "./SidebarSection";
import { useWorkspace } from "../state/workspace";
import { basename } from "../utils/path";
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
      <div className="sidebar-empty" data-tauri-drag-region>
        <p className="sidebar-empty-text">Pick a folder to use as your workspace.</p>
        {/* One primary on first run. The second action used to carry the same
            filled treatment, so two buttons competed for the same first
            click. */}
        <button className="button" onClick={onPickWorkspace}>
          Choose workspace folder…
        </button>
        <button
          className="button button-secondary"
          onClick={onPickFile}
          title="Open a single file (⌘O)"
        >
          Open a single file…
        </button>
      </div>
    );
  }

  const folderName = basename(workspaceRoot);

  return (
    <div className="sidebar">
      {/* Overlay title bar duty: the sidebar owns the window's top-left
          corner, so this header keeps the macOS traffic lights clear
          (--titlebar-inset in CSS) and drags the window. */}
      <div className="sidebar-header" data-tauri-drag-region>
        <span className="sidebar-header-title" title={workspaceRoot}>{folderName}</span>
        <button
          className="sidebar-header-change"
          onClick={onRefresh}
          title="Refresh the workspace tree"
          aria-label="Refresh the workspace tree"
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
      {/* Only the drawers scroll — the header above owns the traffic-light
          inset and the drag region, so it must stay pinned. */}
      <div className="sidebar-drawers">
        <Bookmarks onOpenFile={onOpenFile} />
        <RecentFiles onOpenFile={onOpenFile} />
        {/* The whole tree is one drawer, folded by default: a project root
            lists directories, not artifacts. Bookmarks, Recent and ⌘P are
            the primary paths to a file; the tree is for the
            walk-the-project case. */}
        <SidebarSection id="files" title="Files" defaultCollapsed grow>
          <Explorer
            ipc={ipc}
            root={workspaceRoot}
            onOpenFile={onOpenFile}
            selectedFile={selectedFile}
            refreshNonce={refreshNonce}
          />
        </SidebarSection>
      </div>
    </div>
  );
}
