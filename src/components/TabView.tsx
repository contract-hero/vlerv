// TabView — renders the ACTIVE tab's content: start page, deleted-file
// notice, load error, or the file preview.
import * as React from "react";
import { FileX } from "lucide-react";
import { useActiveTab, useTabsDispatch } from "../state/TabsProvider";
import { currentEntry } from "../state/tabs";
import type { OpenFileOptions } from "../state/TabsProvider";
import Preview from "./Preview";
import StartPage from "./StartPage";

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
      <div className="deleted-notice" data-testid="deleted-notice" role="alert">
        <FileX size={28} strokeWidth={1.5} aria-hidden />
        <p className="deleted-notice-title">File deleted</p>
        <p className="deleted-notice-path">{entry.path}</p>
        <p className="deleted-notice-hint">
          The file was removed from disk. It will reload automatically if it
          comes back.
        </p>
        <button
          type="button"
          className="sidebar-button"
          onClick={() => dispatch({ type: "CLOSE_TAB", tabId: tab.id })}
        >
          Close tab
        </button>
      </div>
    );
  }

  if (isErrorPayload(tab.payload)) {
    const { kind, path, reason } = tab.payload.error;
    return (
      <div role="alert" data-testid="preview-error" className="preview-error">
        <p className="preview-error-kind">{kind}</p>
        <p className="preview-error-path">{path}</p>
        <p className="preview-error-reason">{reason}</p>
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
