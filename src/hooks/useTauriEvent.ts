// useTauriEvent — generic Tauri event subscription with the StrictMode-safe
// cancelled guard (cleanup can run before listen() resolves; a naive
// assignment would orphan the unlisten and leak duplicate listeners).
import * as React from "react";
import { listen } from "@tauri-apps/api/event";

export function useTauriEvent<T>(event: string, onEvent: (payload: T) => void): void {
  // Keep the handler in a ref so consumers don't have to memoize it — the
  // subscription itself only depends on the event name.
  const handlerRef = React.useRef(onEvent);
  handlerRef.current = onEvent;

  React.useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    listen<T>(event, (e) => {
      handlerRef.current(e.payload);
    })
      .then((fn) => {
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
  }, [event]);
}
