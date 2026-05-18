// useBookmarks — list + toggle starred file paths via the state_store-backed
// `add_bookmark` / `remove_bookmark` Tauri commands. Optimistic local updates
// on toggle (UI flips immediately; backend write is debounced via state_store).
import * as React from "react";
import { listen } from "@tauri-apps/api/event";
import { defaultIpc } from "../ipc";
import type { BookmarkEntry } from "../ipc";

export type { BookmarkEntry };

export interface UseBookmarksResult {
  bookmarks: BookmarkEntry[];
  isBookmarked: (path: string) => boolean;
  toggle: (path: string) => Promise<void>;
  remove: (path: string) => Promise<void>;
}

export function useBookmarks(ipc = defaultIpc): UseBookmarksResult {
  const [bookmarks, setBookmarks] = React.useState<BookmarkEntry[]>([]);

  React.useEffect(() => {
    if (ipc.listBookmarks) {
      ipc.listBookmarks().then(setBookmarks).catch(() => {
        // Ignore: backend may not be wired (older builds).
      });
    }
  }, [ipc]);

  // Subscribe for cross-window updates (single-window today; safe no-op).
  React.useEffect(() => {
    let unlisten: (() => void) | null = null;
    try {
      listen("vlerv://bookmarks-updated", (event) => {
        setBookmarks(event.payload as BookmarkEntry[]);
      }).then((fn) => {
        unlisten = fn;
      }).catch(() => {
        // ignore
      });
    } catch {
      // ignore
    }
    return () => {
      if (unlisten) unlisten();
    };
  }, []);

  const isBookmarked = React.useCallback(
    (path: string) => bookmarks.some((b) => b.path === path),
    [bookmarks],
  );

  const toggle = React.useCallback(
    async (path: string): Promise<void> => {
      const currentlyBookmarked = bookmarks.some((b) => b.path === path);
      // Optimistic local update.
      if (currentlyBookmarked) {
        setBookmarks((prev) => prev.filter((b) => b.path !== path));
        if (ipc.removeBookmark) {
          await ipc.removeBookmark(path);
        }
      } else {
        const now = Math.floor(Date.now() / 1000);
        setBookmarks((prev) => [
          { path, bookmarked_at: now },
          ...prev.filter((b) => b.path !== path),
        ]);
        if (ipc.addBookmark) {
          await ipc.addBookmark(path);
        }
      }
    },
    [bookmarks, ipc],
  );

  const remove = React.useCallback(
    async (path: string): Promise<void> => {
      setBookmarks((prev) => prev.filter((b) => b.path !== path));
      if (ipc.removeBookmark) {
        await ipc.removeBookmark(path);
      }
    },
    [ipc],
  );

  return { bookmarks, isBookmarked, toggle, remove };
}
