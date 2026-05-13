// useRecents — subscribes to vlerv://recents-updated and exposes open(path).
import * as React from "react";
import { listen } from "@tauri-apps/api/event";
import { defaultIpc } from "../ipc";

export interface RecentEntry {
  path: string;
  opened_at: number;
}

export interface UseRecentsResult {
  recents: RecentEntry[];
  open: (path: string) => void;
}

export function useRecents(ipc = defaultIpc): UseRecentsResult {
  const [recents, setRecents] = React.useState<RecentEntry[]>([]);

  // Load recents on mount.
  React.useEffect(() => {
    if (ipc.listRecents) {
      ipc.listRecents().then((list) => {
        setRecents(list);
      }).catch(() => {
        // Ignore errors.
      });
    }
  }, [ipc]);

  // Subscribe to vlerv://recents-updated events.
  React.useEffect(() => {
    let unlisten: (() => void) | null = null;
    try {
      listen("vlerv://recents-updated", (event) => {
        const payload = event.payload as RecentEntry[];
        setRecents(payload);
      }).then((fn) => {
        unlisten = fn;
      }).catch(() => {
        // If Tauri events aren't available, ignore.
      });
    } catch {
      // Synchronous throw (non-Tauri environment), ignore.
    }
    return () => {
      if (unlisten) unlisten();
    };
  }, []);

  const open = React.useCallback(
    (path: string) => {
      if (ipc.pushRecent) {
        void ipc.pushRecent(path);
      }
      if (ipc.readFile) {
        void ipc.readFile(path);
      }
    },
    [ipc],
  );

  return { recents, open };
}
