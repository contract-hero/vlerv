// StartPage — shown in a tab with no file: recents + bookmarks as quick links.
import * as React from "react";
import { Clock, FolderOpen, FileUp, Star } from "lucide-react";
import { useRecentsContext } from "../state/recents-context";
import { useBookmarksContext } from "../state/bookmarks-context";
import { FileGlyph } from "./FileIcon";
import type { OpenFileOptions } from "../state/TabsProvider";
import { openOptsFromClick } from "../state/TabsProvider";
import { basename, dirname } from "../utils/path";

export interface StartPageProps {
  onOpenFile: (path: string, opts?: OpenFileOptions) => void;
  onPickFile: () => void;
  onPickWorkspace: () => void;
  workspaceRoot: string | null;
}

function FileList({
  paths,
  onOpenFile,
}: {
  paths: string[];
  onOpenFile: StartPageProps["onOpenFile"];
}): React.ReactElement {
  return (
    <ul className="start-list">
      {paths.map((path) => (
        <li key={path}>
          <button
            type="button"
            className="start-row"
            title={path}
            onClick={(e) => onOpenFile(path, openOptsFromClick(e))}
            onAuxClick={(e) => {
              if (e.button === 1) onOpenFile(path, { newTab: true, background: true });
            }}
          >
            <span className="start-row-icon" aria-hidden>
              <FileGlyph name={basename(path)} size={15} />
            </span>
            <span className="start-row-name">{basename(path)}</span>
            <span className="start-row-dir">{dirname(path)}</span>
          </button>
        </li>
      ))}
    </ul>
  );
}

export default function StartPage({
  onOpenFile,
  onPickFile,
  onPickWorkspace,
  workspaceRoot,
}: StartPageProps): React.ReactElement {
  const { recents, refresh } = useRecentsContext();
  const { bookmarks } = useBookmarksContext();

  // Recents have no backend update event; refresh when a start page appears.
  React.useEffect(() => {
    refresh();
  }, [refresh]);

  return (
    <div className="start-page" data-testid="start-page">
      <div className="start-page-inner">
        <h1 className="start-title">Vlervtifacts</h1>
        <p className="start-subtitle">
          {workspaceRoot ?? "No workspace selected"}
        </p>

        <div className="start-actions">
          <button type="button" className="start-action" onClick={onPickFile}>
            <FileUp size={15} strokeWidth={2} /> Open file… <kbd>⌘O</kbd>
          </button>
          <button type="button" className="start-action" onClick={onPickWorkspace}>
            <FolderOpen size={15} strokeWidth={2} />
            {workspaceRoot ? "Change workspace…" : "Choose workspace…"}
          </button>
        </div>

        {bookmarks.length > 0 ? (
          <section className="start-section">
            <h2>
              <Star size={13} strokeWidth={2} /> Bookmarks
            </h2>
            <FileList paths={bookmarks.map((b) => b.path)} onOpenFile={onOpenFile} />
          </section>
        ) : null}

        {recents.length > 0 ? (
          <section className="start-section">
            <h2>
              <Clock size={13} strokeWidth={2} /> Recent
            </h2>
            <FileList paths={recents.map((r) => r.path)} onOpenFile={onOpenFile} />
          </section>
        ) : null}

        {bookmarks.length === 0 && recents.length === 0 ? (
          <p className="start-empty">
            Files you open and bookmark will show up here. Press <kbd>⌘P</kbd> to
            quick-open a workspace file, or drop a path into the address bar
            (<kbd>⌘L</kbd>).
          </p>
        ) : null}
      </div>
    </div>
  );
}
