// RecentFiles — sidebar section listing the last-opened artifacts. This is
// the working set: the files you actually read surface themselves, so the
// full workspace tree can stay folded. Backed by the recents store the
// StartPage already uses; rows share the Bookmarks row shape minus reorder.
import * as React from "react";
import { useRecentsContext } from "../state/recents-context";
import { openOptsFromClick } from "../state/TabsProvider";
import type { OpenFileOptions } from "../state/TabsProvider";
import { useContextMenu } from "./ContextMenu";
import { useFileMenu } from "../hooks/useFileMenu";
import { basename } from "../utils/path";
import { FileGlyph } from "./FileIcon";
import SidebarSection from "./SidebarSection";

/** Sidebar shows the head of the recents list; the StartPage shows it all. */
const MAX_ROWS = 8;

export interface RecentFilesProps {
  onOpenFile?: (path: string, opts?: OpenFileOptions) => void;
}

export default function RecentFiles({
  onOpenFile,
}: RecentFilesProps): React.ReactElement | null {
  const { recents, refresh } = useRecentsContext();
  const { open: openMenu } = useContextMenu();
  const fileMenu = useFileMenu(onOpenFile);

  // Mount-only: after this, the open funnel (useOpenFile) refreshes the
  // store once each push settles, so the drawer never races the write and
  // background-tab opens are covered too.
  React.useEffect(() => {
    refresh();
  }, [refresh]);

  const rows = recents.slice(0, MAX_ROWS);
  // No empty-state hint: an empty Recent section teaches nothing (opening
  // any file fills it), so it simply isn't there yet.
  if (rows.length === 0) return null;

  return (
    <SidebarSection id="recents" title="Recent">
      <ul>
        {rows.map((entry) => (
          <li
            key={entry.path}
            data-recent-path={entry.path}
            onClick={(e) => onOpenFile?.(entry.path, openOptsFromClick(e))}
            onAuxClick={(e) => {
              if (e.button === 1) onOpenFile?.(entry.path, { newTab: true, background: true });
            }}
            onContextMenu={(e) => openMenu(e, fileMenu(entry.path))}
            tabIndex={0}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                onOpenFile?.(entry.path, openOptsFromClick(e));
              }
            }}
            style={{ cursor: "pointer" }}
            title={entry.path}
          >
            <span className="section-row-icon" aria-hidden>
              <FileGlyph name={basename(entry.path)} size={13} />
            </span>
            <span className="section-row-label">{basename(entry.path)}</span>
          </li>
        ))}
      </ul>
    </SidebarSection>
  );
}
