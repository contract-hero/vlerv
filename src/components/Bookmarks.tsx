// Bookmarks group — collapsible section in the Sidebar above the Explorer.
// Click → open the file; ✕ → remove; right-click → the shared file context
// menu (useFileMenu); drag rows to reorder. Collapse state persisted to
// localStorage so the choice survives reloads.
import * as React from "react";
import { ChevronRight, X } from "lucide-react";
import { useBookmarksContext } from "../state/bookmarks-context";
import { openOptsFromClick } from "../state/TabsProvider";
import type { OpenFileOptions } from "../state/TabsProvider";
import { useContextMenu } from "./ContextMenu";
import { basename } from "../utils/path";
import { useFileMenu } from "../hooks/useFileMenu";

const COLLAPSED_KEY = "vlerv.bookmarks.collapsed";

function readSavedCollapsed(): boolean {
  try {
    return globalThis.localStorage?.getItem(COLLAPSED_KEY) === "1";
  } catch {
    return false;
  }
}

function saveCollapsed(collapsed: boolean): void {
  try {
    globalThis.localStorage?.setItem(COLLAPSED_KEY, collapsed ? "1" : "0");
  } catch {
    // ignore
  }
}

export interface BookmarksProps {
  onOpenFile?: (path: string, opts?: OpenFileOptions) => void;
}

export default function Bookmarks({ onOpenFile }: BookmarksProps): React.ReactElement {
  const { bookmarks, remove, reorder } = useBookmarksContext();
  const { open: openMenu } = useContextMenu();
  const fileMenu = useFileMenu(onOpenFile);
  const [collapsed, setCollapsed] = React.useState<boolean>(readSavedCollapsed);
  // Drag-to-reorder state: the path being dragged and the path currently
  // hovered as a drop target (used to draw the insertion indicator).
  const [dragPath, setDragPath] = React.useState<string | null>(null);
  const [overPath, setOverPath] = React.useState<string | null>(null);

  const handleDrop = (targetPath: string) => {
    const from = dragPath;
    setDragPath(null);
    setOverPath(null);
    if (!from || from === targetPath) return;
    const order = bookmarks.map((b) => b.path);
    const fromIdx = order.indexOf(from);
    const toIdx = order.indexOf(targetPath);
    if (fromIdx < 0 || toIdx < 0) return;
    // Drop the dragged row *above* the target (matching the insertion-line
    // indicator). Removing the source first shifts every later index left by
    // one, so a downward drag inserts at toIdx-1 to land before the target.
    const [moved] = order.splice(fromIdx, 1);
    order.splice(fromIdx < toIdx ? toIdx - 1 : toIdx, 0, moved);
    void reorder(order);
  };

  const toggleCollapsed = () => {
    setCollapsed((prev) => {
      const next = !prev;
      saveCollapsed(next);
      return next;
    });
  };

  if (bookmarks.length === 0) {
    return (
      <div data-section="bookmarks" data-testid="bookmarks-group">
        <h4 className="bookmarks-header bookmarks-header-empty">
          <span>Bookmarks</span>
        </h4>
        <p className="bookmarks-empty-hint">Hover a file and click ☆ to pin it here.</p>
      </div>
    );
  }

  const handleClick = (e: React.MouseEvent, path: string) => {
    onOpenFile?.(path, openOptsFromClick(e));
  };

  return (
    <div data-section="bookmarks" data-testid="bookmarks-group">
      <h4
        className="bookmarks-header"
        onClick={toggleCollapsed}
        role="button"
        aria-expanded={!collapsed}
        tabIndex={0}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            toggleCollapsed();
          }
        }}
      >
        <span
          className={`bookmarks-chevron${collapsed ? "" : " open"}`}
          aria-hidden
        >
          <ChevronRight size={12} strokeWidth={2} />
        </span>
        <span>Bookmarks</span>
        <span className="bookmarks-count" aria-hidden>{bookmarks.length}</span>
      </h4>
      {!collapsed && (
        <ul>
          {bookmarks.map((entry) => (
            <li
              key={entry.path}
              data-bookmark-path={entry.path}
              className={
                dragPath === entry.path
                  ? "dragging"
                  : overPath === entry.path && dragPath
                    ? "drop-target"
                    : undefined
              }
              draggable
              onDragStart={(e) => {
                setDragPath(entry.path);
                e.dataTransfer.effectAllowed = "move";
                // Required for the drag to actually start in WKWebView (macOS).
                e.dataTransfer.setData("text/plain", entry.path);
              }}
              onDragOver={(e) => {
                if (!dragPath) return;
                e.preventDefault();
                e.dataTransfer.dropEffect = "move";
                if (overPath !== entry.path) setOverPath(entry.path);
              }}
              onDrop={(e) => {
                e.preventDefault();
                handleDrop(entry.path);
              }}
              onDragEnd={() => {
                setDragPath(null);
                setOverPath(null);
              }}
              onClick={(e) => handleClick(e, entry.path)}
              onAuxClick={(e) => {
                if (e.button === 1) onOpenFile?.(entry.path, { newTab: true, background: true });
              }}
              onContextMenu={(e) => openMenu(e, fileMenu(entry.path))}
              style={{ cursor: "pointer" }}
              title={`${entry.path}\n(drag to reorder)`}
            >
              <span className="bookmark-label">{basename(entry.path)}</span>
              <button
                type="button"
                className="bookmark-remove"
                title="Remove bookmark"
                aria-label={`Remove bookmark ${basename(entry.path)}`}
                onClick={(e) => {
                  e.stopPropagation();
                  void remove(entry.path);
                }}
              >
                <X size={11} strokeWidth={2} />
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
