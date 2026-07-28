// useFileMenu — builds the standard right-click menu for a file path:
// open / open in new tab / reveal in Finder / open in default app /
// copy path / bookmark toggle. Used by tree rows, bookmark rows and tabs.
import * as React from "react";
import {
  Copy,
  ExternalLink,
  FilePlus2,
  Folder,
  Star,
  StarOff,
} from "lucide-react";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
import type { MenuSection } from "../components/ContextMenu";
import { useBookmarksContext } from "../state/bookmarks-context";
import type { OpenFileOptions } from "../state/TabsProvider";

export function useFileMenu(
  onOpenFile?: (path: string, opts?: OpenFileOptions) => void,
): (path: string) => MenuSection[] {
  const { isBookmarked, toggle } = useBookmarksContext();

  return React.useCallback(
    (path: string): MenuSection[] => {
      const bookmarked = isBookmarked(path);
      return [
        [
          {
            label: "Open in New Tab",
            icon: <FilePlus2 size={13} strokeWidth={2} />,
            onSelect: () => onOpenFile?.(path, { newTab: true, background: false }),
          },
        ],
        [
          {
            label: "Reveal in Finder",
            icon: <Folder size={13} strokeWidth={2} />,
            onSelect: () => void revealItemInDir(path).catch(() => {}),
          },
          {
            label: "Open in Default App",
            icon: <ExternalLink size={13} strokeWidth={2} />,
            onSelect: () => void openPath(path).catch(() => {}),
          },
          {
            label: "Copy Path",
            icon: <Copy size={13} strokeWidth={2} />,
            onSelect: () => void navigator.clipboard?.writeText(path).catch(() => {}),
          },
        ],
        [
          bookmarked
            ? {
                label: "Remove Bookmark",
                icon: <StarOff size={13} strokeWidth={2} />,
                onSelect: () => void toggle(path),
              }
            : {
                label: "Bookmark",
                icon: <Star size={13} strokeWidth={2} />,
                onSelect: () => void toggle(path),
              },
        ],
      ];
    },
    [isBookmarked, toggle, onOpenFile],
  );
}
