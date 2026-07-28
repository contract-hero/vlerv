// WatcherProvider — single subscriber to the backend filesystem events,
// fanned out through an imperative bus (ref-based, no state) so FS churn
// never re-renders the tree. Consumers: Explorer (folder invalidation),
// TabsProvider (tab auto-reload), QuickOpen (index invalidation).
import * as React from "react";
import type { IpcSurface } from "../ipc";
import { useTauriEvent } from "../hooks/useTauriEvent";

export interface FsChange {
  kind: "add" | "modify" | "remove";
  path: string;
  /** Present for workspace-tree events, absent for external-file events. */
  project_root?: string;
  source: "tree" | "external";
}

type Subscriber = (change: FsChange) => void;

export interface WatcherBus {
  subscribe(cb: Subscriber): () => void;
}

const WatcherContext = React.createContext<WatcherBus>({
  subscribe: () => () => {},
});

export function useWatcherBus(): WatcherBus {
  return React.useContext(WatcherContext);
}

interface TreeChangedPayload {
  project_root: string;
  kind: "add" | "modify" | "remove";
  path: string;
}

interface FileChangedPayload {
  kind: "add" | "modify" | "remove";
  path: string;
}

export interface WatcherProviderProps {
  ipc: IpcSurface;
  root: string | null;
  children: React.ReactNode;
}

export function WatcherProvider({ ipc, root, children }: WatcherProviderProps): React.ReactElement {
  const subscribers = React.useRef(new Set<Subscriber>());

  const bus = React.useMemo<WatcherBus>(
    () => ({
      subscribe(cb: Subscriber) {
        subscribers.current.add(cb);
        return () => {
          subscribers.current.delete(cb);
        };
      },
    }),
    [],
  );

  const emit = React.useCallback((change: FsChange) => {
    subscribers.current.forEach((cb) => cb(change));
  }, []);

  // Start (or replace) the backend workspace watcher whenever the root changes.
  React.useEffect(() => {
    if (!root || !ipc.watchRoot) return;
    void ipc.watchRoot(root).catch((e: unknown) => {
      console.warn("vlerv: failed to start backend watcher for", root, e);
    });
  }, [ipc, root]);

  useTauriEvent<TreeChangedPayload>("vlerv://tree-changed", (p) => {
    emit({ kind: p.kind, path: p.path, project_root: p.project_root, source: "tree" });
  });

  useTauriEvent<FileChangedPayload>("vlerv://file-changed", (p) => {
    emit({ kind: p.kind, path: p.path, source: "external" });
  });

  return <WatcherContext.Provider value={bus}>{children}</WatcherContext.Provider>;
}
