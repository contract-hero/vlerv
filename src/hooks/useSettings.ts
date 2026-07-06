// useSettings — read/write state document fields; subscribes to
// vlerv://state-updated.
import * as React from "react";
import { listen } from "@tauri-apps/api/event";
import { defaultIpc } from "../ipc";
import type { SettingsState } from "../ipc";

// The IPC layer owns the wire shape; keep a single definition so a new
// preference field can't be added to one copy and silently missed in the other.
export type UseSettingsState = SettingsState;

export interface UseSettingsResult {
  state: UseSettingsState | null;
  setStateField: (key: string, value: unknown) => Promise<void>;
}

export function useSettings(ipc = defaultIpc): UseSettingsResult {
  const [state, setState] = React.useState<UseSettingsState | null>(null);

  // Load state on mount.
  React.useEffect(() => {
    if (ipc.getState) {
      ipc.getState().then((s) => {
        setState(s as UseSettingsState);
      }).catch(() => {
        // Ignore errors; state stays null.
      });
    }
  }, [ipc]);

  // Subscribe to vlerv://state-updated events.
  React.useEffect(() => {
    let unlisten: (() => void) | null = null;
    try {
      listen("vlerv://state-updated", (event) => {
        const payload = event.payload as UseSettingsState;
        setState(payload);
      }).then((fn) => {
        unlisten = fn;
      }).catch(() => {
        // If Tauri events aren't available (e.g. in tests), ignore.
      });
    } catch {
      // Synchronous throw (non-Tauri environment), ignore.
    }
    return () => {
      if (unlisten) unlisten();
    };
  }, []);

  const setStateField = React.useCallback(
    async (key: string, value: unknown): Promise<void> => {
      if (ipc.setStateField) {
        await ipc.setStateField(key, value);
      }
    },
    [ipc],
  );

  return { state, setStateField };
}
