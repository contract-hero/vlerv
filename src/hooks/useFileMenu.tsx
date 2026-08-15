// useFileMenu — builds the standard right-click menu for a file path:
// open in new tab / reveal in Finder / copy path / bookmark toggle.
// Used by tree rows, bookmark rows and tabs.
//
// Deliberately NO "Open in Default App": that requires the broad
// `opener:allow-open-path` capability, which would hand arbitrary program
// launch to anything that can reach IPC from the webview — too big a grant
// for a convenience reachable via Reveal in Finder + double-click.
import * as React from "react";
import {
  Copy,
  FilePlus2,
  Folder,
  Star,
  StarOff,
  Zap,
} from "lucide-react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import type { MenuSection } from "../components/ContextMenu";
import { useBookmarksContext } from "../state/bookmarks-context";
import { useBeamActions } from "../state/beam";
import type { OpenFileOptions } from "../state/TabsProvider";

export function useFileMenu(
  onOpenFile?: (path: string, opts?: OpenFileOptions) => void,
): (path: string) => MenuSection[] {
  const { isBookmarked, toggle } = useBookmarksContext();
  const { beginSend } = useBeamActions();

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
            label: "Copy Path",
            icon: <Copy size={13} strokeWidth={2} />,
            onSelect: () => void navigator.clipboard?.writeText(path).catch(() => {}),
          },
          {
            label: "Beam to Vlervcode…",
            icon: <Zap size={13} strokeWidth={2} />,
            onSelect: () => beginSend(path),
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
