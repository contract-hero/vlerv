// RemoteDrawer — one sidebar drawer per online paired peer (design §6):
// "Remote: <device>", built from the SidebarSection shape, listing the
// peer's open tabs live, with presence in the header and a Follow toggle.
// Under browse scope and up it also grows a lazy workspace tree, rooted at
// the directories the peer has already told us about (its open tabs) — the
// only paths this client can address without a dedicated "list roots" verb,
// which the wire protocol does not have (design §6 sketches ListTree as a
// per-directory call, not a root enumerator).
import * as React from "react";
import { Cast, ChevronRight } from "lucide-react";
import type { IpcSurface, RemotePathEntry, RemotePeer, TreeEntry } from "../ipc";
import { useRemoteActions, useRemoteState } from "../state/remote";
import { activeTabPath, shouldFollowNavigate } from "../state/remote-tabs";
import { formatRemoteAddress, REMOTE_SCHEME } from "../utils/remote-address";
import { basename, dirname } from "../utils/path";
import { openOptsFromClick } from "../state/TabsProvider";
import type { OpenFileOptions } from "../state/TabsProvider";
import { FileGlyph, FolderGlyph } from "./FileIcon";
import SidebarSection from "./SidebarSection";

export interface RemoteDrawerListProps {
  ipc: IpcSurface;
  onOpenFile?: (path: string, opts?: OpenFileOptions) => void;
}

/** Mounted once in the sidebar; renders zero or more per-peer drawers. */
export default function RemoteDrawerList({
  ipc,
  onOpenFile,
}: RemoteDrawerListProps): React.ReactElement | null {
  const { peers, presence } = useRemoteState();
  const online = peers.filter((p) => presence[p.node_id]?.state === "online");
  if (online.length === 0) return null;
  return (
    <>
      {online.map((peer) => (
        <RemoteDrawer key={peer.node_id} ipc={ipc} peer={peer} onOpenFile={onOpenFile} />
      ))}
    </>
  );
}

function RemoteDrawer({
  ipc,
  peer,
  onOpenFile,
}: {
  ipc: IpcSurface;
  peer: RemotePeer;
  onOpenFile?: (path: string, opts?: OpenFileOptions) => void;
}): React.ReactElement {
  const { presence, tabsByPeer, followingPeer } = useRemoteState();
  const { subscribePeer, unsubscribePeer, toggleFollow } = useRemoteActions();
  const info = presence[peer.node_id];
  const tabs = tabsByPeer[peer.node_id] ?? [];
  const following = followingPeer === peer.node_id;
  // The scope THIS session was granted, confirmed at handshake — not
  // `peer.scope`, which is what WE granted the peer for the other direction.
  const grantedScope = info?.scope;
  const canBrowse = grantedScope === "browse" || grantedScope === "control";
  const device = info?.device ?? peer.device;

  React.useEffect(() => {
    subscribePeer(peer.node_id);
    return () => unsubscribePeer(peer.node_id);
  }, [peer.node_id, subscribePeer, unsubscribePeer]);

  // Follow mode: mirror the host's active tab as TabActivated events land.
  const activePath = activeTabPath(tabs);
  const prevActiveRef = React.useRef<string | null>(null);
  React.useEffect(() => {
    if (shouldFollowNavigate(prevActiveRef.current, activePath, following)) {
      // `followPrefix` keeps the mirror in ONE tab per peer: without it the
      // open funnel navigates the locally active tab, so every switch on the
      // host would overwrite whatever the user is reading here.
      onOpenFile?.(formatRemoteAddress(peer.node_id, activePath as string), {
        external: true,
        followPrefix: `${REMOTE_SCHEME}${peer.node_id}`,
      });
    }
    prevActiveRef.current = activePath;
  }, [activePath, following, onOpenFile, peer.node_id]);

  // The peer's bookmarks and recents (design §6: "its open tabs, bookmarks
  // and recents, fetchable on demand"). Both are one-shot reads on connect —
  // the wire has no change event for either list, so they refresh when the
  // session does, not live like the tab list.
  const [bookmarks, setBookmarks] = React.useState<RemotePathEntry[]>([]);
  const [recents, setRecents] = React.useState<RemotePathEntry[]>([]);
  React.useEffect(() => {
    let cancelled = false;
    const load = (
      call: Promise<RemotePathEntry[]> | undefined,
      set: (list: RemotePathEntry[]) => void,
    ) => {
      call
        ?.then((list) => {
          if (!cancelled) set(list);
        })
        .catch(() => {
          // A refusal here is normal, not exceptional: the host may have
          // narrowed our scope between the handshake and this read.
          if (!cancelled) set([]);
        });
    };
    load(ipc.remoteListBookmarks?.(peer.node_id), setBookmarks);
    load(ipc.remoteListRecents?.(peer.node_id), setRecents);
    return () => {
      cancelled = true;
    };
  }, [ipc, peer.node_id, info?.state]);

  // Candidate tree roots: the directories of tabs this drawer already knows
  // about. Deduped and sorted so the list doesn't reorder itself as tabs
  // open/close elsewhere on the host.
  const treeRoots = React.useMemo(() => {
    const dirs = new Set(tabs.map((t) => dirname(t.path)));
    return Array.from(dirs).sort();
  }, [tabs]);

  return (
    <SidebarSection
      id={`remote-${peer.node_id}`}
      title={`Remote: ${device}`}
      count={tabs.length}
      headerExtra={
        <>
          <span
            className={`remote-presence-dot remote-presence-${info?.state ?? "offline"}`}
            title={info?.state ?? "offline"}
            aria-hidden
          />
          <button
            type="button"
            className={`remote-follow-toggle${following ? " active" : ""}`}
            aria-pressed={following}
            title={following ? "Stop following" : "Follow this device's active tab"}
            onClick={(e) => {
              e.stopPropagation();
              toggleFollow(peer.node_id);
            }}
          >
            <Cast size={12} strokeWidth={2} />
          </button>
        </>
      }
    >
      <ul>
        {tabs.map((t) => (
          <li
            key={t.path}
            className={t.active ? "active" : undefined}
            title={t.path}
            style={{ cursor: "pointer" }}
            onClick={(e) =>
              onOpenFile?.(formatRemoteAddress(peer.node_id, t.path), openOptsFromClick(e))
            }
          >
            <span className="section-row-icon" aria-hidden>
              <FileGlyph name={basename(t.path)} size={13} />
            </span>
            <span className="section-row-label">{basename(t.path)}</span>
          </li>
        ))}
      </ul>
      <RemotePathList
        label="Bookmarks"
        entries={bookmarks}
        peer={peer.node_id}
        onOpenFile={onOpenFile}
      />
      <RemotePathList
        label="Recent"
        entries={recents}
        peer={peer.node_id}
        onOpenFile={onOpenFile}
      />
      {canBrowse && treeRoots.length > 0 ? (
        <ul data-testid={`remote-tree-${peer.node_id}`}>
          {treeRoots.map((dir) => (
            <RemoteTreeNode
              key={dir}
              ipc={ipc}
              peer={peer.node_id}
              path={dir}
              depth={0}
              onOpenFile={onOpenFile}
            />
          ))}
        </ul>
      ) : null}
    </SidebarSection>
  );
}

/** The peer's bookmarks or recents: a labelled, flat list of openable rows.
 * Renders nothing when the list is empty, so a view-open peer that shared
 * neither adds no chrome to the drawer. */
function RemotePathList({
  label,
  entries,
  peer,
  onOpenFile,
}: {
  label: string;
  entries: RemotePathEntry[];
  peer: string;
  onOpenFile?: (path: string, opts?: OpenFileOptions) => void;
}): React.ReactElement | null {
  if (entries.length === 0) return null;
  return (
    <>
      <div className="remote-list-label">{label}</div>
      <ul>
        {entries.map((entry) => (
          <li
            key={entry.path}
            title={entry.path}
            style={{ cursor: "pointer" }}
            onClick={(e) =>
              onOpenFile?.(formatRemoteAddress(peer, entry.path), openOptsFromClick(e))
            }
          >
            <span className="section-row-icon" aria-hidden>
              <FileGlyph name={entry.name} size={13} />
            </span>
            <span className="section-row-label">{entry.name}</span>
          </li>
        ))}
      </ul>
    </>
  );
}

/** One lazily-expanded directory (or file) row in the browse-scope tree.
 * Deliberately unadorned relative to the local Explorer — no drag-out, no
 * bookmark star: a remote row's only actions are "open" and "expand". */
function RemoteTreeNode({
  ipc,
  peer,
  path,
  depth,
  onOpenFile,
}: {
  ipc: IpcSurface;
  peer: string;
  path: string;
  depth: number;
  onOpenFile?: (path: string, opts?: OpenFileOptions) => void;
}): React.ReactElement {
  const [expanded, setExpanded] = React.useState(false);
  const [children, setChildren] = React.useState<TreeEntry[] | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const name = basename(path) || path;

  React.useEffect(() => {
    if (!expanded || children !== null || !ipc.remoteListTree) return;
    let cancelled = false;
    ipc.remoteListTree(peer, path)
      .then((entries) => {
        if (cancelled) return;
        const sorted = [...entries].sort((a, b) => {
          if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
          return a.name.toLowerCase().localeCompare(b.name.toLowerCase());
        });
        setChildren(sorted);
      })
      .catch((e: unknown) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [expanded, children, ipc, peer, path]);

  return (
    <li style={{ paddingLeft: depth * 14 }}>
      <div
        style={{ cursor: "pointer" }}
        title={path}
        onClick={() => setExpanded((v) => !v)}
      >
        <span className={`section-chevron${expanded ? " open" : ""}`} aria-hidden>
          <ChevronRight size={11} strokeWidth={2} />
        </span>
        <span className="section-row-icon" aria-hidden>
          <FolderGlyph open={expanded} size={13} />
        </span>
        <span className="section-row-label">{name}</span>
      </div>
      {expanded ? (
        error ? (
          <div className="remote-tree-error">{error}</div>
        ) : (
          <ul>
            {(children ?? []).map((entry) =>
              entry.is_dir ? (
                <RemoteTreeNode
                  key={entry.path}
                  ipc={ipc}
                  peer={peer}
                  path={entry.path}
                  depth={depth + 1}
                  onOpenFile={onOpenFile}
                />
              ) : (
                <li
                  key={entry.path}
                  style={{ paddingLeft: (depth + 1) * 14, cursor: "pointer" }}
                  title={entry.path}
                  onClick={(e) =>
                    onOpenFile?.(formatRemoteAddress(peer, entry.path), openOptsFromClick(e))
                  }
                >
                  <span className="section-row-icon" aria-hidden>
                    <FileGlyph name={entry.name} size={13} />
                  </span>
                  <span className="section-row-label">{entry.name}</span>
                </li>
              ),
            )}
          </ul>
        )
      ) : null}
    </li>
  );
}
