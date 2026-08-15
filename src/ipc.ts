// IPC surface between the React frontend and the Rust Tauri core.

import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";

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
  beamReceive?(ticket: string, name?: string, size?: number): Promise<BeamReceivedFile>;
  /** Where received artifacts land (badge prefix check). */
  beamReceivedDir?(): Promise<string>;
  /** Past beams, newest first. */
  beamListReceived?(): Promise<BeamReceivedEntry[]>;
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

  async beamReceive(ticket: string, name?: string, size?: number): Promise<BeamReceivedFile> {
    return await invoke<BeamReceivedFile>("beam_receive", { ticket, name, size });
  }

  async beamReceivedDir(): Promise<string> {
    return await invoke<string>("beam_received_dir");
  }

  async beamListReceived(): Promise<BeamReceivedEntry[]> {
    return await invoke<BeamReceivedEntry[]>("beam_list_received");
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
