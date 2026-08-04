// TabView — renders the ACTIVE tab's content: start page, deleted-file
// notice, load error, or the file preview.
import * as React from "react";
import { FileX, FileWarning, Folder, RotateCw } from "lucide-react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { useActiveTab, useTabsDispatch } from "../state/TabsProvider";
import { currentEntry } from "../state/tabs";
import type { OpenFileOptions } from "../state/TabsProvider";
import Preview from "./Preview";
import StartPage from "./StartPage";

/**
 * The backend hands back a machine `kind` ("Io") and an OS reason string.
 * Neither is a headline. Name the problem in the reader's language and keep
 * the raw reason as secondary detail.
 */
export function readErrorTitle(kind: string, reason: string): string {
  const text = `${kind} ${reason}`.toLowerCase();
  if (text.includes("not found") || text.includes("notfound") || text.includes("no such file")) {
    return "File not found";
  }
  if (text.includes("permission") || text.includes("denied")) return "Permission denied";
  if (text.includes("out of root") || text.includes("outofroot")) {
    return "Outside the workspace";
  }
  if (text.includes("too large") || text.includes("oversized")) return "File too large to display";
  return "Could not read this file";
}

export interface TabViewProps {
  onOpenFile: (path: string, opts?: OpenFileOptions) => void;
  onPickFile: () => void;
  onPickWorkspace: () => void;
  workspaceRoot: string | null;
}

function isErrorPayload(p: unknown): p is { error: { kind: string; path: string; reason: string } } {
  return !!p && typeof p === "object" && "error" in (p as Record<string, unknown>);
}

export default function TabView({
  onOpenFile,
  onPickFile,
  onPickWorkspace,
  workspaceRoot,
}: TabViewProps): React.ReactElement {
  const tab = useActiveTab();
  const dispatch = useTabsDispatch();
  const entry = currentEntry(tab);

  if (!entry) {
    return (
      <StartPage
        onOpenFile={onOpenFile}
        onPickFile={onPickFile}
        onPickWorkspace={onPickWorkspace}
        workspaceRoot={workspaceRoot}
      />
    );
  }

  if (tab.deleted) {
    return (
      <div className="file-notice" data-testid="deleted-notice" role="alert">
        <FileX size={28} strokeWidth={1.5} aria-hidden />
        <p className="file-notice-title">File deleted</p>
        <p className="file-notice-path">{entry.path}</p>
        <p className="file-notice-hint">
          The file was removed from disk. It will reload automatically if it
          comes back.
        </p>
        <div className="file-notice-actions">
          <button
            type="button"
            className="button"
            onClick={() => dispatch({ type: "CLOSE_TAB", tabId: tab.id })}
          >
            Close tab
          </button>
        </div>
      </div>
    );
  }

  // Same shape as the deleted-file notice on purpose: a failed read is the
  // same class of event and deserves the same care — name it, show the path,
  // and offer the two things worth doing about it.
  if (isErrorPayload(tab.payload)) {
    const { kind, path, reason } = tab.payload.error;
    return (
      <div role="alert" data-testid="preview-error" className="file-notice">
        <FileWarning size={28} strokeWidth={1.5} aria-hidden />
        <p className="file-notice-title">{readErrorTitle(kind, reason)}</p>
        <p className="file-notice-path">{path}</p>
        <p className="file-notice-hint">{reason}</p>
        <div className="file-notice-actions">
          <button
            type="button"
            className="button"
            data-testid="preview-error-retry"
            onClick={() => dispatch({ type: "RELOAD" })}
          >
            <RotateCw size={13} strokeWidth={2} aria-hidden /> Try again
          </button>
          <button
            type="button"
            className="button"
            onClick={() => {
              void revealItemInDir(path).catch(() => {
                // Parent directory may be gone too — nothing useful to add.
              });
            }}
          >
            <Folder size={13} strokeWidth={2} aria-hidden /> Reveal in Finder
          </button>
        </div>
      </div>
    );
  }

  if (tab.payload === null) {
    // First load of this tab still in flight.
    return <div className="preview-empty">Loading…</div>;
  }

  // Keyed per (tab, path): forces a fresh renderer (and iframe) per document,
  // so late scroll messages from an outgoing document fail the e.source check
  // instead of stamping their offsets onto the incoming document's key.
  return <Preview key={`${tab.id}::${entry.path}`} payload={tab.payload} tabId={tab.id} zoom={tab.zoom} />;
}
