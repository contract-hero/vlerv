// Explorer — recursive file tree with inline folder expansion + native drag-out.
import * as React from "react";
import { ChevronRight, Star } from "lucide-react";
import { startDrag } from "@crabnebula/tauri-plugin-drag";
import { resolveResource } from "@tauri-apps/api/path";
import type { IpcSurface, TreeEntry } from "../ipc";
import { FileGlyph, FolderGlyph } from "./FileIcon";
import { useWatcherBus } from "../state/watcher-bus";
import { useBookmarksContext } from "../state/bookmarks-context";
import { useExplorerUi } from "../state/explorer-ui";
import { openOptsFromClick } from "../state/TabsProvider";
import { dirname } from "../utils/path";
import type { OpenFileOptions } from "../state/TabsProvider";
import { useContextMenu } from "./ContextMenu";
import { useFileMenu } from "../hooks/useFileMenu";

// Path-keyed cache-version map. FolderNode reads its own path's version from
// context; bumping it re-fires that node's listDir effect without touching
// siblings or its own expansion state.
const FolderCacheContext = React.createContext<ReadonlyMap<string, number>>(new Map());

// Monotonic manual-refresh counter. Every FolderNode's listDir effect depends
// on it, so bumping it (via the sidebar Refresh button) re-fetches every
// expanded folder at once while preserving each node's expansion state.
const RefreshContext = React.createContext<number>(0);

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
  onOpenFile?: (path: string, opts?: OpenFileOptions) => void;
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
  onOpenFile?: (path: string, opts?: OpenFileOptions) => void;
  selectedFile?: string | null;
}

function FolderNode({ ipc, entry, depth, onOpenFile, selectedFile }: NodeProps): React.ReactElement {
  // Expansion is hoisted into ExplorerUiContext so reveal() can expand whole
  // ancestor chains and expansion survives node remounts.
  const { expandedPaths, toggle, focusedPath } = useExplorerUi();
  const expanded = expandedPaths.has(entry.path);
  const [entries, setEntries] = React.useState<TreeEntry[] | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const versions = React.useContext(FolderCacheContext);
  const version = versions.get(entry.path) ?? 0;
  const refreshNonce = React.useContext(RefreshContext);
  const { isBookmarked, toggle: toggleBookmark } = useBookmarksContext();

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
        onClick={() => toggle(entry.path)}
        data-path={entry.path}
        data-kind="dir"
        role="treeitem"
        aria-expanded={expanded}
        aria-level={depth + 1}
        tabIndex={focusedPath === entry.path ? 0 : -1}
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
            onOpenFile={onOpenFile}
            selectedFile={selectedFile}
          />
        ) : (
          <FileRow
            key={child.path}
            entry={child}
            depth={depth + 1}
            onOpenFile={onOpenFile}
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
  onOpenFile?: (path: string, opts?: OpenFileOptions) => void;
  selected: boolean;
  bookmarked: boolean;
  onToggleBookmark: (path: string) => void;
}

function FileRow({ entry, depth, onOpenFile, selected, bookmarked, onToggleBookmark }: FileRowProps): React.ReactElement {
  const indentPx = 12 + depth * 16;
  const { revealTarget, focusedPath } = useExplorerUi();
  const rowRef = React.useRef<HTMLDivElement | null>(null);
  const { open: openMenu } = useContextMenu();
  const fileMenu = useFileMenu(onOpenFile);

  // Reveal: scroll into view when this row is the armed target. Runs on
  // mount and on nonce bumps, so it fires exactly when the async ancestor
  // expand-cascade finally renders the row.
  const isRevealTarget = revealTarget?.path === entry.path;
  React.useEffect(() => {
    if (isRevealTarget) {
      rowRef.current?.scrollIntoView({ block: "nearest" });
    }
  }, [isRevealTarget, revealTarget?.nonce]);

  return (
    <div
      ref={rowRef}
      className={`row file ${selected ? "selected" : ""}`}
      style={{ paddingLeft: `${indentPx}px` }}
      role="treeitem"
      aria-level={depth + 1}
      aria-selected={selected}
      tabIndex={focusedPath === entry.path ? 0 : -1}
      onClick={(e) => onOpenFile?.(entry.path, openOptsFromClick(e))}
      onAuxClick={(e) => {
        if (e.button === 1) onOpenFile?.(entry.path, { newTab: true, background: true });
      }}
      onContextMenu={(e) => openMenu(e, fileMenu(entry.path))}
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

export default function Explorer({ ipc, root, onOpenFile, selectedFile, refreshNonce = 0 }: ExplorerProps): React.ReactElement {
  const [entries, setEntries] = React.useState<TreeEntry[] | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [versions, setVersions] = React.useState<ReadonlyMap<string, number>>(() => new Map());
  const { isBookmarked, toggle: toggleBookmark } = useBookmarksContext();
  const { expandedPaths, expand, collapse, focusedPath, setFocusedPath } = useExplorerUi();
  const bus = useWatcherBus();
  const containerRef = React.useRef<HTMLDivElement | null>(null);

  // Tree keyboard navigation: roving tabindex over the rendered rows (the
  // recursive render produces flat DOM order, so querySelectorAll walks the
  // visible tree top-to-bottom).
  const onTreeKeyDown = React.useCallback(
    (e: React.KeyboardEvent) => {
      const container = containerRef.current;
      if (!container) return;
      const rows = Array.from(container.querySelectorAll<HTMLElement>("[data-path]"));
      if (rows.length === 0) return;
      const currentIdx = rows.findIndex((r) => r.dataset.path === focusedPath);

      const focusRow = (idx: number) => {
        const row = rows[Math.max(0, Math.min(rows.length - 1, idx))];
        if (!row) return;
        setFocusedPath(row.dataset.path ?? null);
        row.focus();
        row.scrollIntoView({ block: "nearest" });
      };

      const current = currentIdx >= 0 ? rows[currentIdx] : null;
      const currentPath = current?.dataset.path ?? null;
      const isDir = current?.dataset.kind === "dir";

      switch (e.key) {
        case "ArrowDown":
          e.preventDefault();
          focusRow(currentIdx < 0 ? 0 : currentIdx + 1);
          break;
        case "ArrowUp":
          e.preventDefault();
          focusRow(currentIdx < 0 ? 0 : currentIdx - 1);
          break;
        case "ArrowRight":
          e.preventDefault();
          if (isDir && currentPath) {
            if (!expandedPaths.has(currentPath)) expand(currentPath);
            else focusRow(currentIdx + 1); // first child is the next row
          }
          break;
        case "ArrowLeft":
          e.preventDefault();
          if (isDir && currentPath && expandedPaths.has(currentPath)) {
            collapse(currentPath);
          } else if (currentPath) {
            // Move to the parent row.
            const parent = dirname(currentPath);
            const parentIdx = rows.findIndex((r) => r.dataset.path === parent);
            if (parentIdx >= 0) focusRow(parentIdx);
          }
          break;
        case "Home":
          e.preventDefault();
          focusRow(0);
          break;
        case "End":
          e.preventDefault();
          focusRow(rows.length - 1);
          break;
        case "Enter":
        case " ":
          e.preventDefault();
          if (isDir && currentPath) {
            if (expandedPaths.has(currentPath)) collapse(currentPath);
            else expand(currentPath);
          } else if (currentPath) {
            onOpenFile?.(currentPath, openOptsFromClick(e));
          }
          break;
      }
    },
    [focusedPath, setFocusedPath, expandedPaths, expand, collapse, onOpenFile],
  );

  const invalidate = React.useCallback((path: string) => {
    setVersions((prev) => {
      const next = new Map(prev);
      next.set(path, (prev.get(path) ?? 0) + 1);
      return next;
    });
  }, []);

  const rootVersion = versions.get(root) ?? 0;

  // Bridge backend events into the cache: invalidate the parent dir of the
  // changed path so just that folder re-fetches. (The backend watcher itself
  // is started by WatcherProvider.)
  React.useEffect(() => {
    return bus.subscribe((change) => {
      if (change.source !== "tree") return;
      invalidate(dirname(change.path));
    });
  }, [bus, invalidate]);

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
      <div
        className="explorer"
        role="tree"
        aria-label="Workspace files"
        ref={containerRef}
        // Tab-reachable even before any row has been focused; the keydown
        // handler treats "no focused row" as "start at the first row".
        tabIndex={focusedPath ? -1 : 0}
        onKeyDown={onTreeKeyDown}
      >
        {sortEntries(entries).map((entry) =>
          entry.is_dir ? (
            <FolderNode
              key={entry.path}
              ipc={ipc}
              entry={entry}
              depth={0}
              onOpenFile={onOpenFile}
              selectedFile={selectedFile}
            />
          ) : (
            <FileRow
              key={entry.path}
              entry={entry}
              depth={0}
              onOpenFile={onOpenFile}
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
