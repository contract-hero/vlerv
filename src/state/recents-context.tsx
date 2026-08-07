// RecentsProvider — one recents subscription for the whole app (StartPage
// and the sidebar Recent drawer).
import * as React from "react";
import type { IpcSurface, RecentEntry } from "../ipc";
import { useTauriEvent } from "../hooks/useTauriEvent";

export interface RecentsContextValue {
  recents: RecentEntry[];
  /** Re-fetch from the backend (recents have no update event today). */
  refresh: () => void;
}

const RecentsContext = React.createContext<RecentsContextValue>({
  recents: [],
  refresh: () => {},
});

export function useRecentsContext(): RecentsContextValue {
  return React.useContext(RecentsContext);
}

export function RecentsProvider({
  ipc,
  children,
}: {
  ipc: IpcSurface;
  children: React.ReactNode;
}): React.ReactElement {
  const [recents, setRecents] = React.useState<RecentEntry[]>([]);

  const refresh = React.useCallback(() => {
    if (!ipc.listRecents) return;
    ipc.listRecents().then(setRecents).catch((e: unknown) => {
      // Keep the last good list, but leave a trace: a silent failure here
      // makes the Recent drawer indistinguishable from "no recents yet".
      console.error("vlerv: failed to list recents", e);
    });
  }, [ipc]);

  React.useEffect(() => {
    refresh();
  }, [refresh]);

  // No backend emits `vlerv://recents-updated` today; the subscription is
  // kept for forward compat with a backend that broadcasts recents updates.
  useTauriEvent<RecentEntry[]>("vlerv://recents-updated", setRecents);

  const value = React.useMemo(() => ({ recents, refresh }), [recents, refresh]);
  return <RecentsContext.Provider value={value}>{children}</RecentsContext.Provider>;
}
