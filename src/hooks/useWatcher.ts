// useWatcher — subscribes to vlerv://tree-changed and invalidates the Tree
// frontend cache.
import * as React from "react";
import { listen } from "@tauri-apps/api/event";

export interface UseWatcherDeps {
  invalidate: (projectRoot: string) => void;
  refetch?: (projectRoot: string, path: string) => void;
}

interface TreeChangedPayload {
  project_root: string;
  kind: "add" | "modify" | "remove";
  path: string;
}

export function useWatcher({ invalidate, refetch }: UseWatcherDeps): void {
  React.useEffect(() => {
    let unlisten: (() => void) | null = null;
    try {
      listen("vlerv://tree-changed", (event) => {
        const payload = event.payload as TreeChangedPayload;
        invalidate(payload.project_root);
        if (refetch) {
          refetch(payload.project_root, payload.path);
        }
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
  }, [invalidate, refetch]);
}
