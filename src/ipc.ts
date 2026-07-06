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
  content: string | null;
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
  shareFile?(paths: string[], anchor: ShareAnchor): Promise<void>;
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

  async shareFile(paths: string[], anchor: ShareAnchor): Promise<void> {
    await invoke<void>("share_file", { paths, anchor });
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
