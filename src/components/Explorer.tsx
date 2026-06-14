// Explorer — recursive file tree with inline folder expansion + native drag-out.
import * as React from "react";
import { ChevronRight, Star } from "lucide-react";
import { startDrag } from "@crabnebula/tauri-plugin-drag";
import { resolveResource } from "@tauri-apps/api/path";
import type { IpcSurface, TreeEntry } from "../ipc";
import { FileGlyph, FolderGlyph } from "./FileIcon";
import { useWatcher } from "../hooks/useWatcher";
import type { TreeChangedPayload } from "../hooks/useWatcher";
import { useBookmarks } from "../hooks/useBookmarks";

// Path-keyed cache-version map. FolderNode reads its own path's version from
// context; bumping it re-fires that node's listDir effect without touching
// siblings or its own expansion state.
const FolderCacheContext = React.createContext<ReadonlyMap<string, number>>(new Map());

// Monotonic manual-refresh counter. Every FolderNode's listDir effect depends
// on it, so bumping it (via the sidebar Refresh button) re-fetches every
// expanded folder at once while preserving each node's expansion state.
const RefreshContext = React.createContext<number>(0);

// POSIX dirname. macOS-only app, so `/` separator is safe.
function parentDir(absPath: string): string {
  const idx = absPath.lastIndexOf("/");
  return idx <= 0 ? "/" : absPath.slice(0, idx);
}

// Drag-preview icon: resolved from the bundled `resources` declaration in
// tauri.conf.json. Cached on first call so subsequent drags don't hit IPC.
let cachedDragIcon: string | null = null;
async function dragIcon(): Promise<string | null> {
  if (cachedDragIcon !== null) return cachedDragIcon;
  try {
    cachedDragIcon = await resolveResource("icons/drag-icon.png");
    return cachedDragIcon;
  } catch {
    return null;
  }
}

const DEFAULT_IGNORED = new Set([
  ".git", "node_modules", "target", "dist", "build", ".next", ".venv",
]);

export interface ExplorerProps {
  ipc: IpcSurface;
  root: string;
  onSelectFile?: (path: string) => void;
  selectedFile?: string | null;
  /** Bumped by the sidebar Refresh button to force a full tree re-fetch. */
  refreshNonce?: number;
}

function sortEntries(entries: TreeEntry[]): TreeEntry[] {
  return [...entries].sort((a, b) => {
    if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
    return a.name.toLowerCase().localeCompare(b.name.toLowerCase());
  });
}

interface NodeProps {
  ipc: IpcSurface;
  entry: TreeEntry;
  depth: number;
  onSelectFile?: (path: string) => void;
  selectedFile?: string | null;
}

function FolderNode({ ipc, entry, depth, onSelectFile, selectedFile }: NodeProps): React.ReactElement {
  const [expanded, setExpanded] = React.useState(false);
  const [entries, setEntries] = React.useState<TreeEntry[] | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const versions = React.useContext(FolderCacheContext);
  const version = versions.get(entry.path) ?? 0;
  const refreshNonce = React.useContext(RefreshContext);
  const { isBookmarked, toggle: toggleBookmark } = useBookmarks(ipc);

  React.useEffect(() => {
    if (!expanded) return;
    let cancelled = false;
    ipc.listDir(entry.path)
      .then((list) => {
        if (!cancelled) {
          setEntries(list.filter((e) => !DEFAULT_IGNORED.has(e.name)));
          setError(null);
        }
      })
      .catch((e: Error) => {
        if (!cancelled) setError(e.message);
      });
    return () => { cancelled = true; };
  }, [expanded, entry.path, ipc, version, refreshNonce]);

  const indentPx = 12 + depth * 16;
  const isSelected = selectedFile === entry.path;

  return (
    <>
      <div
        className={`row folder ${isSelected ? "selected" : ""}`}
        style={{ paddingLeft: `${indentPx}px` }}
        onClick={() => setExpanded((v) => !v)}
        data-path={entry.path}
        data-kind="dir"
      >
        <span className={`chevron ${expanded ? "open" : ""}`}>
          <ChevronRight size={14} strokeWidth={2} />
        </span>
        <span className="icon folder-icon" aria-hidden>
          <FolderGlyph open={expanded} />
        </span>
        <span className="label">{entry.name}</span>
      </div>
      {expanded && error !== null && (
        <div className="row error-row" style={{ paddingLeft: `${indentPx + 20}px` }}>
          {error}
        </div>
      )}
      {expanded && entries !== null && sortEntries(entries).map((child) =>
        child.is_dir ? (
          <FolderNode
            key={child.path}
            ipc={ipc}
            entry={child}
            depth={depth + 1}
            onSelectFile={onSelectFile}
            selectedFile={selectedFile}
          />
        ) : (
          <FileRow
            key={child.path}
            entry={child}
            depth={depth + 1}
            onSelectFile={onSelectFile}
            selected={selectedFile === child.path}
            bookmarked={isBookmarked(child.path)}
            onToggleBookmark={(p) => void toggleBookmark(p)}
          />
        )
      )}
    </>
  );
}

interface FileRowProps {
  entry: TreeEntry;
  depth: number;
  onSelectFile?: (path: string) => void;
  selected: boolean;
  bookmarked: boolean;
  onToggleBookmark: (path: string) => void;
}

function FileRow({ entry, depth, onSelectFile, selected, bookmarked, onToggleBookmark }: FileRowProps): React.ReactElement {
  const indentPx = 12 + depth * 16;
  return (
    <div
      className={`row file ${selected ? "selected" : ""}`}
      style={{ paddingLeft: `${indentPx}px` }}
      onClick={() => onSelectFile?.(entry.path)}
      draggable
      onDragStart={(e) => {
        e.preventDefault();
        void (async () => {
          const icon = await dragIcon();
          if (!icon) return;
          await startDrag({ item: [entry.path], icon }).catch(() => {
            // ignore — plugin may not be available in some environments
          });
        })();
      }}
      data-path={entry.path}
      data-kind="file"
    >
      <span className="chevron-placeholder" aria-hidden> </span>
      <span className="icon file-icon" aria-hidden>
        <FileGlyph name={entry.name} />
      </span>
      <span className="label">{entry.name}</span>
      <button
        className={`star-toggle${bookmarked ? " bookmarked" : ""}`}
        data-testid="bookmark-toggle"
        title={bookmarked ? "Remove bookmark" : "Bookmark"}
        aria-label={bookmarked ? "Remove bookmark" : "Bookmark"}
        aria-pressed={bookmarked}
        onClick={(e) => {
          e.stopPropagation();
          onToggleBookmark(entry.path);
        }}
      >
        <Star size={14} strokeWidth={2} fill={bookmarked ? "currentColor" : "none"} />
      </button>
    </div>
  );
}

export default function Explorer({ ipc, root, onSelectFile, selectedFile, refreshNonce = 0 }: ExplorerProps): React.ReactElement {
  const [entries, setEntries] = React.useState<TreeEntry[] | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [versions, setVersions] = React.useState<ReadonlyMap<string, number>>(() => new Map());
  const { isBookmarked, toggle: toggleBookmark } = useBookmarks(ipc);

  const invalidate = React.useCallback((path: string) => {
    setVersions((prev) => {
      const next = new Map(prev);
      next.set(path, (prev.get(path) ?? 0) + 1);
      return next;
    });
  }, []);

  const rootVersion = versions.get(root) ?? 0;

  // Start the backend filesystem watcher for the current root. Replaces any
  // previous watcher on the Rust side (see set_workspace_root in main.rs).
  React.useEffect(() => {
    if (!ipc.watchRoot) return;
    void ipc.watchRoot(root).catch((e: unknown) => {
      console.warn("vlerv: failed to start backend watcher for", root, e);
    });
  }, [ipc, root]);

  // Bridge backend events into the cache: invalidate the parent dir of the
  // changed path so just that folder re-fetches.
  const handleChange = React.useCallback((payload: TreeChangedPayload) => {
    invalidate(parentDir(payload.path));
  }, [invalidate]);
  useWatcher(handleChange);

  // Blank visible state when the user picks a different workspace folder, so
  // we don't briefly render stale entries under a new header.
  React.useEffect(() => {
    setEntries(null);
    setError(null);
  }, [root]);

  // Fetch root listing on mount, root change, or refresh. No setEntries(null)
  // here — that's the blank effect's job — so a watcher-driven refetch
  // updates in place without flashing "Loading…".
  React.useEffect(() => {
    let cancelled = false;
    ipc.listDir(root)
      .then((list) => {
        if (!cancelled) {
          setEntries(list.filter((e) => !DEFAULT_IGNORED.has(e.name)));
          setError(null);
        }
      })
      .catch((e: Error) => {
        if (!cancelled) setError(e.message);
      });
    return () => { cancelled = true; };
  }, [ipc, root, rootVersion, refreshNonce]);

  if (error !== null) {
    return <div role="alert" className="explorer-error">{error}</div>;
  }
  if (entries === null) {
    return <div className="explorer-loading">Loading…</div>;
  }

  return (
    <FolderCacheContext.Provider value={versions}>
      <RefreshContext.Provider value={refreshNonce}>
      <div className="explorer">
        {sortEntries(entries).map((entry) =>
          entry.is_dir ? (
            <FolderNode
              key={entry.path}
              ipc={ipc}
              entry={entry}
              depth={0}
              onSelectFile={onSelectFile}
              selectedFile={selectedFile}
            />
          ) : (
            <FileRow
              key={entry.path}
              entry={entry}
              depth={0}
              onSelectFile={onSelectFile}
              selected={selectedFile === entry.path}
              bookmarked={isBookmarked(entry.path)}
              onToggleBookmark={(p) => void toggleBookmark(p)}
            />
          )
        )}
      </div>
      </RefreshContext.Provider>
    </FolderCacheContext.Provider>
  );
}
