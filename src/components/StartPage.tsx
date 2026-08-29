// StartPage — shown in a tab with no file: recents + bookmarks as quick links.
import * as React from "react";
import { Clock, FolderOpen, FileUp, Star } from "lucide-react";
import { useRecentsContext } from "../state/recents-context";
import { useBookmarksContext } from "../state/bookmarks-context";
import { usePlatform } from "../state/platform";
import IosStartPage from "./IosStartPage";
import { FileGlyph } from "./FileIcon";
import type { OpenFileOptions } from "../state/TabsProvider";
import { openOptsFromClick } from "../state/TabsProvider";
import { basename, displayDir } from "../utils/path";

export interface StartPageProps {
  onOpenFile: (path: string, opts?: OpenFileOptions) => void;
  onPickFile: () => void;
  onPickWorkspace: () => void;
  workspaceRoot: string | null;
  /** Opens the Settings modal — only used by the iOS variant's "Pair with a
   * Mac" call to action. */
  onOpenSettings?: () => void;
}


function FileList({
  paths,
  onOpenFile,
  workspaceRoot,
}: {
  paths: string[];
  onOpenFile: StartPageProps["onOpenFile"];
  workspaceRoot: string | null;
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
            <span className="start-row-icon">
              <FileGlyph name={basename(path)} size={15} />
            </span>
            <span className="start-row-name">{basename(path)}</span>
            {/* <bdi> keeps the path reading left-to-right inside the
                right-to-left box that puts the ellipsis at the head, so the
                deepest segments survive on a long absolute path. */}
            <span className="start-row-dir">
              <bdi>{displayDir(path, workspaceRoot)}</bdi>
            </span>
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
  onOpenSettings,
}: StartPageProps): React.ReactElement {
  const { isIos } = usePlatform();
  const { recents, refresh } = useRecentsContext();
  const { bookmarks } = useBookmarksContext();

  // Recents have no backend update event; refresh when a start page appears.
  React.useEffect(() => {
    refresh();
  }, [refresh]);

  // iOS owns no local files (PRODUCT.md: read-only companion) — the phone's
  // empty-tab surface is pairing + what Beam/PushArtifact already delivered,
  // not a workspace/file picker built for a local tree that doesn't exist.
  if (isIos) {
    return <IosStartPage onOpenSettings={onOpenSettings} />;
  }

  return (
    <div className="start-page" data-testid="start-page">
      <div className="start-page-inner">
        {/* The brand mark — a lavender tile carrying the monogram. It is one
            of the four jobs the accent is allowed to do (DESIGN.md, The One
            Lavender Rule); the wordmark beside it stays in ink. */}
        <div className="start-brand">
          <span className="start-mark" aria-hidden>V</span>
          <h1 className="start-title">Vlervtifacts</h1>
        </div>
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
            <FileList
              paths={bookmarks.map((b) => b.path)}
              onOpenFile={onOpenFile}
              workspaceRoot={workspaceRoot}
            />
          </section>
        ) : null}

        {recents.length > 0 ? (
          <section className="start-section">
            <h2>
              <Clock size={13} strokeWidth={2} /> Recent
            </h2>
            <FileList
              paths={recents.map((r) => r.path)}
              onOpenFile={onOpenFile}
              workspaceRoot={workspaceRoot}
            />
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
