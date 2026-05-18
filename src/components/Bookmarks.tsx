// Bookmarks group — collapsible section in the Sidebar above the Explorer.
// Click → open the file; right-click → remove from bookmarks. Mirrors
// Recents.tsx for layout but with a chevron-driven expand/collapse header
// (state persisted to localStorage so the choice survives reloads).
import * as React from "react";
import { ChevronRight } from "lucide-react";
import { useBookmarks } from "../hooks/useBookmarks";
import { defaultIpc } from "../ipc";
import { isUnderRoot } from "../utils/path";

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
  ipc?: typeof defaultIpc;
  onSelectFile?: (path: string, external?: boolean) => void;
  workspaceRoot?: string | null;
}

export default function Bookmarks({
  ipc = defaultIpc,
  onSelectFile,
  workspaceRoot = null,
}: BookmarksProps): React.ReactElement {
  const { bookmarks, remove } = useBookmarks(ipc);
  const [collapsed, setCollapsed] = React.useState<boolean>(readSavedCollapsed);

  const toggleCollapsed = () => {
    setCollapsed((prev) => {
      const next = !prev;
      saveCollapsed(next);
      return next;
    });
  };

  if (bookmarks.length === 0) {
    return <div data-section="bookmarks" data-testid="bookmarks-group" />;
  }

  const handleClick = (path: string) => {
    onSelectFile?.(path, !isUnderRoot(path, workspaceRoot));
  };

  const handleContextMenu = (e: React.MouseEvent, path: string) => {
    e.preventDefault();
    void remove(path);
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
              onClick={() => handleClick(entry.path)}
              onContextMenu={(e) => handleContextMenu(e, entry.path)}
              style={{ cursor: "pointer" }}
              title={`${entry.path}\n(right-click to remove)`}
            >
              {entry.path.replace(/^.*\//, "")}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
