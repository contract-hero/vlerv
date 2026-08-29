// IPC surface between the React frontend and the Rust Tauri core.

import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { parseRemoteAddress } from "./utils/remote-address";

export interface ProjectEntry {
  name: string;
  path: string;
}

export interface TreeEntry {
  name: string;
  path: string;
  is_dir: boolean;
}

export interface FilePayload {
  path: string;
  size: number;
  mtime: number;
  is_binary: boolean;
  oversized: boolean;
  /** How `content` is encoded: UTF-8 text or base64 (raster images). */
  encoding?: "text" | "base64";
  content: string | null;
}

/** Flat recursive file index returned by `list_files_recursive` (⌘P). */
export interface FileIndex {
  root: string;
  /** Paths relative to `root`, BFS (shallow-first) order. */
  files: string[];
  truncated: boolean;
}

export interface RecentEntry {
  path: string;
  opened_at: number;
}

export interface BookmarkEntry {
  path: string;
  bookmarked_at: number;
}

export interface SettingsState {
  schema_version: number;
  roots: string[];
  recents?: RecentEntry[];
  bookmarks?: BookmarkEntry[];
  panes?: {
    sidebar_px?: number;
    preview_px?: number;
    sidebar_visible?: boolean;
    /** The reading session: open tabs, their history and zoom. */
    tabs?: unknown;
  };
  preferences: {
    ignore_globs: string[];
    drag_out_mode: "file" | "url";
    slack_target?: string | null;
    beam_ttl_hours?: number;
    /** Boot the endpoint at launch when peers.json is non-empty (design §4). */
    remote_listen?: boolean;
    /** Reserved: a self-hosted iroh-relay override. Not yet read by the core. */
    relay_url?: string | null;
  };
}

/** Anchor rect for the native share popover, from getBoundingClientRect(). */
export interface ShareAnchor {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** One active Beam offer (this instance is serving the staged blob). */
export interface BeamOffer {
  /** Offer id == the blob's BLAKE3 hash (hex). */
  id: string;
  path: string;
  name: string;
  size: number;
  ticket: string;
  /** The shareable `vlerv://receive?…` deep link. */
  link: string;
  created_at: number;
  expires_at: number;
  fetches: number;
}

/** A completed receive: where the verified blob landed. */
export interface BeamReceivedFile {
  path: string;
  name: string;
  size: number;
  hash: string;
}

/** One past beam in the received/ tree. */
export interface BeamReceivedEntry {
  path: string;
  name: string;
  size: number;
  received_at: number;
}

// ── Scope v2 (Remote Control §6) ────────────────────────────────────────────

export type RemoteScope = "view-open" | "browse" | "control";

/** One trusted, paired peer. */
export interface RemotePeer {
  node_id: string;
  device: string;
  scope: RemoteScope;
  paired_at: number;
  last_seen: number;
}

/** `remote_pair_begin` — the link + fingerprint material for the host face. */
export interface RemotePairInvite {
  ticket: string;
  /** `vlerv://pair?ticket=…` */
  link: string;
  node_id: string;
  device: string;
  expires_at: number;
}

/** A pairing that reached the fingerprint step, on EITHER side. */
export interface RemotePendingPair {
  node_id: string;
  device: string;
  /** The six words both screens must show. */
  fingerprint: string[];
  role: "host" | "guest";
  created_at: number;
}

export interface RemoteTabEntry {
  path: string;
  active: boolean;
}

export interface RemotePathEntry {
  path: string;
  name: string;
}

/** A verified artifact fetched from a peer, landed in the local
 * content-addressed cache. `path` is the LOCAL file the render pipeline
 * reads; `remote_path` is the identity on the host — what the drawer and the
 * FileChanged/FileRemoved events use. */
export interface RemoteArtifact {
  peer: string;
  remote_path: string;
  path: string;
  hash: string;
  size: number;
  mtime: number;
  warn: boolean;
}

/** `vlerv://remote-presence`. */
export interface RemotePresenceEvent {
  peer: string;
  state: "connecting" | "online" | "offline";
  device: string | null;
  scope: RemoteScope | null;
  reason: string | null;
}

/** `vlerv://remote-event` — tagged on `kind`, fields stay snake_case (the
 * wire shape the backend emits verbatim). */
export type RemoteEvent =
  | { kind: "tab-opened"; peer: string; path: string }
  | { kind: "tab-closed"; peer: string; path: string }
  | { kind: "tab-activated"; peer: string; path: string }
  | { kind: "file-changed"; peer: string; path: string; hash: string }
  | { kind: "file-removed"; peer: string; path: string }
  | {
      kind: "pair-pending";
      peer: string;
      device: string;
      fingerprint: string[];
      role: "host" | "guest";
    }
  | { kind: "pair-link"; peer: string; peer_short: string; device: string; ticket: string }
  | { kind: "peers-updated" };

/** `platform_info` — the one signal the frontend uses to tell the desktop
 * and iOS builds apart (PRODUCT.md Operating Context). Optional: older
 * builds / test doubles without it fall back to macOS (see
 * `state/platform.tsx`). */
export interface PlatformInfo {
  os: "macos" | "ios";
}

export interface IpcSurface {
  listProjects(): Promise<ProjectEntry[] | string[]>;
  listDir(projectPath: string): Promise<TreeEntry[]>;
  readFile(path: string): Promise<FilePayload>;
  setWorkspaceRoot?(path: string): void;
  watchRoot?(path: string): Promise<void>;
  refreshProject?(projectPath: string): Promise<void>;
  getState?(): Promise<SettingsState>;
  setStateField?(key: string, value: unknown): Promise<void>;
  pickDirectory?(): Promise<string | null>;
  pickFile?(): Promise<string | null>;
  listRecents?(): Promise<RecentEntry[]>;
  pushRecent?(path: string): Promise<void>;
  listBookmarks?(): Promise<BookmarkEntry[]>;
  addBookmark?(path: string): Promise<void>;
  removeBookmark?(path: string): Promise<void>;
  reorderBookmarks?(paths: string[]): Promise<void>;
  listFilesRecursive?(root: string): Promise<FileIndex>;
  /**
   * Replace the set of individually watched out-of-root files (open external
   * tabs). Empty array clears the watcher. Changes arrive as
   * `vlerv://file-changed` events.
   */
  watchExternalPaths?(paths: string[]): Promise<void>;
  shareFile?(paths: string[], anchor: ShareAnchor): Promise<void>;
  /** Stage a file and mint its beam ticket (boots the endpoint lazily). */
  beamOffer?(path: string): Promise<BeamOffer>;
  /** Revoke an active offer — the ticket dies instantly. */
  beamStop?(offerId: string): Promise<void>;
  /** Active offers. Never boots the endpoint. */
  beamListOffers?(): Promise<BeamOffer[]>;
  /** Post-confirm fetch; progress arrives as `vlerv://beam-progress`. */
  beamReceive?(ticket: string, name?: string): Promise<BeamReceivedFile>;
  /** Where received artifacts land (badge prefix check). */
  beamReceivedDir?(): Promise<string>;
  /** Past beams, newest first. */
  beamListReceived?(): Promise<BeamReceivedEntry[]>;

  /** Trusted peers, newest pairing first. Never boots the endpoint. */
  remoteListPeers?(): Promise<RemotePeer[]>;
  /** Mint a one-time pairing ticket + link. Boots the endpoint. */
  remotePairBegin?(): Promise<RemotePairInvite>;
  /** Open a pairing ticket and park at the fingerprint step. */
  remotePairComplete?(ticket: string): Promise<RemotePendingPair>;
  /** Resolve a parked pairing after the human compares the six words. */
  remotePairConfirm?(
    nodeId: string,
    accept: boolean,
    scope?: RemoteScope,
  ): Promise<RemotePeer | null>;
  /** Revoke a peer. Never boots. */
  remoteUnpair?(nodeId: string): Promise<void>;
  /** Change what a peer may do. Never boots. */
  remoteSetScope?(nodeId: string, scope: RemoteScope): Promise<void>;
  /** The host bridge from the tabs reducer — called on every tabs commit. */
  remotePublishTabs?(tabs: RemoteTabEntry[]): Promise<void>;
  /** The peer's open tabs, live. */
  remoteListTabs?(peer: string): Promise<RemoteTabEntry[]>;
  /** The peer's bookmarks. */
  remoteListBookmarks?(peer: string): Promise<RemotePathEntry[]>;
  /** The peer's recents. */
  remoteListRecents?(peer: string): Promise<RemotePathEntry[]>;
  /** One directory level of the peer's workspace (browse scope and up). */
  remoteListTree?(peer: string, path: string): Promise<TreeEntry[]>;
  /** Fetch an artifact from a peer into the local cache. */
  remoteGet?(peer: string, path: string): Promise<RemoteArtifact>;
  /** Drive the peer: open an artifact on ITS screen (control scope only). */
  remoteOpenOnHost?(peer: string, path: string, readerMode: boolean): Promise<void>;
  /** Start receiving the peer's events (`vlerv://remote-event`). */
  remoteSubscribe?(peer: string): Promise<void>;
  /** Stop receiving the peer's events. */
  remoteUnsubscribe?(peer: string): Promise<void>;

  /** Which shell this build runs in — macOS desktop or the iOS companion. */
  platformInfo?(): Promise<PlatformInfo>;
}

const WORKSPACE_ROOT_KEY = "vlerv.workspaceRoot";

function loadWorkspaceRoot(): string | null {
  try {
    return globalThis.localStorage?.getItem(WORKSPACE_ROOT_KEY) ?? null;
  } catch {
    return null;
  }
}

function saveWorkspaceRoot(path: string): void {
  try {
    globalThis.localStorage?.setItem(WORKSPACE_ROOT_KEY, path);
  } catch {
    // ignore
  }
}

class TauriIpc implements IpcSurface {
  private workspaceRoot: string | null = loadWorkspaceRoot();

  setWorkspaceRoot(path: string): void {
    this.workspaceRoot = path;
    saveWorkspaceRoot(path);
  }

  async listProjects(): Promise<ProjectEntry[]> {
    if (!this.workspaceRoot) {
      throw new Error("NO_WORKSPACE_ROOT");
    }
    return await invoke<ProjectEntry[]>("list_workspace_roots", {
      path: this.workspaceRoot,
    });
  }

  async listDir(projectPath: string): Promise<TreeEntry[]> {
    return await invoke<TreeEntry[]>("list_dir", { path: projectPath });
  }

  async readFile(path: string): Promise<FilePayload> {
    // A `vlerv-remote://<peer>/abs/path` tab address (design §6): fetch the
    // verified artifact into the local content-addressed cache, then read
    // THAT file for bytes — `read_file` is deliberately ungated (see
    // reader.rs), and the cache path is our own app-data file, not
    // attacker-controlled. The returned payload's `path` is overwritten back
    // to the remote address: that address, not the hash-addressed cache
    // file, is the stable identity scroll memory and tab reload key off, and
    // it must survive a live-reload refetch landing at a NEW cache path.
    const remote = parseRemoteAddress(path);
    if (remote) {
      const artifact = await invoke<RemoteArtifact>("remote_get", {
        peer: remote.peer,
        path: remote.path,
      });
      const payload = await invoke<FilePayload>("read_file", { path: artifact.path });
      return { ...payload, path };
    }
    return await invoke<FilePayload>("read_file", { path });
  }

  async watchRoot(path: string): Promise<void> {
    await invoke<void>("set_workspace_root", { path });
  }

  async pickDirectory(): Promise<string | null> {
    const result = await openDialog({
      directory: true,
      multiple: false,
      title: "Choose workspace folder",
    });
    if (typeof result === "string") return result;
    return null;
  }

  async pickFile(): Promise<string | null> {
    const result = await openDialog({
      directory: false,
      multiple: false,
      title: "Open file",
    });
    if (typeof result === "string") return result;
    return null;
  }

  async getState(): Promise<SettingsState> {
    return await invoke<SettingsState>("get_state");
  }

  async setStateField(key: string, value: unknown): Promise<void> {
    await invoke<void>("set_state_field", { key, value });
  }

  async listRecents(): Promise<RecentEntry[]> {
    return await invoke<RecentEntry[]>("list_recents");
  }

  async pushRecent(path: string): Promise<void> {
    await invoke<void>("push_recent", { path });
  }

  async listBookmarks(): Promise<BookmarkEntry[]> {
    return await invoke<BookmarkEntry[]>("list_bookmarks");
  }

  async addBookmark(path: string): Promise<void> {
    await invoke<void>("add_bookmark", { path });
  }

  async removeBookmark(path: string): Promise<void> {
    await invoke<void>("remove_bookmark", { path });
  }

  async reorderBookmarks(paths: string[]): Promise<void> {
    await invoke<void>("reorder_bookmarks", { paths });
  }

  async listFilesRecursive(root: string): Promise<FileIndex> {
    return await invoke<FileIndex>("list_files_recursive", { path: root });
  }

  async watchExternalPaths(paths: string[]): Promise<void> {
    await invoke<void>("watch_external_paths", { paths });
  }

  async shareFile(paths: string[], anchor: ShareAnchor): Promise<void> {
    await invoke<void>("share_file", { paths, anchor });
  }

  async beamOffer(path: string): Promise<BeamOffer> {
    return await invoke<BeamOffer>("beam_offer", { path });
  }

  async beamStop(offerId: string): Promise<void> {
    await invoke<void>("beam_stop", { offerId });
  }

  async beamListOffers(): Promise<BeamOffer[]> {
    return await invoke<BeamOffer[]>("beam_list_offers");
  }

  async beamReceive(ticket: string, name?: string): Promise<BeamReceivedFile> {
    return await invoke<BeamReceivedFile>("beam_receive", { ticket, name });
  }

  async beamReceivedDir(): Promise<string> {
    return await invoke<string>("beam_received_dir");
  }

  async beamListReceived(): Promise<BeamReceivedEntry[]> {
    return await invoke<BeamReceivedEntry[]>("beam_list_received");
  }

  async remoteListPeers(): Promise<RemotePeer[]> {
    return await invoke<RemotePeer[]>("remote_list_peers");
  }

  async remotePairBegin(): Promise<RemotePairInvite> {
    return await invoke<RemotePairInvite>("remote_pair_begin");
  }

  async remotePairComplete(ticket: string): Promise<RemotePendingPair> {
    return await invoke<RemotePendingPair>("remote_pair_complete", { ticket });
  }

  async remotePairConfirm(
    nodeId: string,
    accept: boolean,
    scope?: RemoteScope,
  ): Promise<RemotePeer | null> {
    return await invoke<RemotePeer | null>("remote_pair_confirm", { nodeId, accept, scope });
  }

  async remoteUnpair(nodeId: string): Promise<void> {
    await invoke<void>("remote_unpair", { nodeId });
  }

  async remoteSetScope(nodeId: string, scope: RemoteScope): Promise<void> {
    await invoke<void>("remote_set_scope", { nodeId, scope });
  }

  async remotePublishTabs(tabs: RemoteTabEntry[]): Promise<void> {
    await invoke<void>("remote_publish_tabs", { tabs });
  }

  async remoteListTabs(peer: string): Promise<RemoteTabEntry[]> {
    return await invoke<RemoteTabEntry[]>("remote_list_tabs", { peer });
  }

  async remoteListBookmarks(peer: string): Promise<RemotePathEntry[]> {
    return await invoke<RemotePathEntry[]>("remote_list_bookmarks", { peer });
  }

  async remoteListRecents(peer: string): Promise<RemotePathEntry[]> {
    return await invoke<RemotePathEntry[]>("remote_list_recents", { peer });
  }

  async remoteListTree(peer: string, path: string): Promise<TreeEntry[]> {
    return await invoke<TreeEntry[]>("remote_list_tree", { peer, path });
  }

  async remoteGet(peer: string, path: string): Promise<RemoteArtifact> {
    return await invoke<RemoteArtifact>("remote_get", { peer, path });
  }

  async remoteOpenOnHost(peer: string, path: string, readerMode: boolean): Promise<void> {
    await invoke<void>("remote_open_on_host", { peer, path, readerMode });
  }

  async remoteSubscribe(peer: string): Promise<void> {
    await invoke<void>("remote_subscribe", { peer });
  }

  async remoteUnsubscribe(peer: string): Promise<void> {
    await invoke<void>("remote_unsubscribe", { peer });
  }

  async platformInfo(): Promise<PlatformInfo> {
    return await invoke<PlatformInfo>("platform_info");
  }
}

export const tauriIpc: IpcSurface = new TauriIpc();

export const defaultIpc: IpcSurface = {
  async listProjects() {
    throw new Error("ipc.listProjects: not wired");
  },
  async listDir(_p) {
    throw new Error("ipc.listDir: not wired");
  },
  async readFile(_p) {
    throw new Error("ipc.readFile: not wired");
  },
};
