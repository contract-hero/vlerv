// WorkspaceContext — single owner of the workspace root (previously buried in
// Sidebar's local state + localStorage reads scattered across files).
import * as React from "react";
import type { IpcSurface } from "../ipc";

const WORKSPACE_ROOT_KEY = "vlerv.workspaceRoot";

export function readSavedWorkspaceRoot(): string | null {
  try {
    return globalThis.localStorage?.getItem(WORKSPACE_ROOT_KEY) ?? null;
  } catch {
    return null;
  }
}

export interface WorkspaceContextValue {
  root: string | null;
  setRoot: (path: string) => void;
  clearRoot: () => void;
}

const WorkspaceContext = React.createContext<WorkspaceContextValue>({
  root: null,
  setRoot: () => {},
  clearRoot: () => {},
});

export function useWorkspace(): WorkspaceContextValue {
  return React.useContext(WorkspaceContext);
}

export interface WorkspaceProviderProps {
  ipc: IpcSurface;
  children: React.ReactNode;
}

export function WorkspaceProvider({ ipc, children }: WorkspaceProviderProps): React.ReactElement {
  const [root, setRootState] = React.useState<string | null>(readSavedWorkspaceRoot);

  const setRoot = React.useCallback(
    (path: string) => {
      // TauriIpc.setWorkspaceRoot persists to localStorage as a side effect.
      ipc.setWorkspaceRoot?.(path);
      setRootState(path);
    },
    [ipc],
  );

  const clearRoot = React.useCallback(() => {
    try {
      globalThis.localStorage?.removeItem(WORKSPACE_ROOT_KEY);
    } catch {
      // ignore
    }
    setRootState(null);
  }, []);

  const value = React.useMemo(
    () => ({ root, setRoot, clearRoot }),
    [root, setRoot, clearRoot],
  );

  return <WorkspaceContext.Provider value={value}>{children}</WorkspaceContext.Provider>;
}
