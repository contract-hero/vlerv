// useDeepLink — subscribes to vlerv://open-file (Backend canonicalizes the
// path against RootSet before emitting) and vlerv://deep-link-error.
import * as React from "react";
import { listen } from "@tauri-apps/api/event";

export type DeepLinkIntent = "open" | "reveal";

export interface OpenFilePayload {
  path: string;
  intent: DeepLinkIntent;
}

export interface DeepLinkErrorPayload {
  url: string;
  reason: string;
}

export interface UseDeepLinkDeps {
  onIntent: (payload: OpenFilePayload) => void;
  onError?: (payload: DeepLinkErrorPayload) => void;
}

export function useDeepLink({ onIntent, onError }: UseDeepLinkDeps): void {
  React.useEffect(() => {
    let unlistenOpen: (() => void) | null = null;
    let unlistenError: (() => void) | null = null;
    try {
      listen("vlerv://open-file", (event) => {
        onIntent(event.payload as OpenFilePayload);
      }).then((fn) => {
        unlistenOpen = fn;
      }).catch(() => {
        // If Tauri events aren't available, ignore.
      });

      listen("vlerv://deep-link-error", (event) => {
        if (onError) onError(event.payload as DeepLinkErrorPayload);
      }).then((fn) => {
        unlistenError = fn;
      }).catch(() => {
        // ignore
      });
    } catch {
      // Synchronous throw (non-Tauri environment), ignore.
    }
    return () => {
      if (unlistenOpen) unlistenOpen();
      if (unlistenError) unlistenError();
    };
  }, [onIntent, onError]);
}
