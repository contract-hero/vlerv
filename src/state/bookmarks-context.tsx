// BookmarksProvider — ONE useBookmarks instance for the whole app. Previously
// every FolderNode + Explorer + Preview called the hook, each firing its own
// listBookmarks IPC call and registering its own event listener (20+ with a
// deep tree), with optimistic toggles invisible across instances.
import * as React from "react";
import type { IpcSurface } from "../ipc";
import { useBookmarks } from "../hooks/useBookmarks";
import type { UseBookmarksResult } from "../hooks/useBookmarks";

const BookmarksContext = React.createContext<UseBookmarksResult>({
  bookmarks: [],
  isBookmarked: () => false,
  toggle: async () => {},
  remove: async () => {},
  reorder: async () => {},
});

export function useBookmarksContext(): UseBookmarksResult {
  return React.useContext(BookmarksContext);
}

export function BookmarksProvider({
  ipc,
  children,
}: {
  ipc: IpcSurface;
  children: React.ReactNode;
}): React.ReactElement {
  const value = useBookmarks(ipc);
  return <BookmarksContext.Provider value={value}>{children}</BookmarksContext.Provider>;
}
