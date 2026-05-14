// useWatcher — subscribes to vlerv://tree-changed and invokes a single
// callback with the change payload. The caller decides what to invalidate.
import * as React from "react";
import { listen } from "@tauri-apps/api/event";

export interface TreeChangedPayload {
  project_root: string;
  kind: "add" | "modify" | "remove";
  path: string;
}

export function useWatcher(onChange: (payload: TreeChangedPayload) => void): void {
  React.useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    listen<TreeChangedPayload>("vlerv://tree-changed", (event) => {
      onChange(event.payload);
    })
      .then((fn) => {
        // Component already unmounted before listen() resolved — tear down now.
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch(() => {
        // Tauri events unavailable (non-Tauri environment); ignore.
      });
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, [onChange]);
}
