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
    // `cancelled` guards the async `listen()` resolve from racing the
    // cleanup function — under React 18 StrictMode the cleanup runs
    // before `.then()` resolves, so a naive `unlisten = fn` assignment
    // would orphan the unlisten and leak duplicate listeners on remount.
    let cancelled = false;
    let unlistenOpen: (() => void) | null = null;
    let unlistenError: (() => void) | null = null;
    try {
      listen("vlerv://open-file", (event) => {
        onIntent(event.payload as OpenFilePayload);
      }).then((fn) => {
        if (cancelled) fn();
        else unlistenOpen = fn;
      }).catch(() => {
        // If Tauri events aren't available, ignore.
      });

      listen("vlerv://deep-link-error", (event) => {
        if (onError) onError(event.payload as DeepLinkErrorPayload);
      }).then((fn) => {
        if (cancelled) fn();
        else unlistenError = fn;
      }).catch(() => {
        // ignore
      });
    } catch {
      // Synchronous throw (non-Tauri environment), ignore.
    }
    return () => {
      cancelled = true;
      if (unlistenOpen) unlistenOpen();
      if (unlistenError) unlistenError();
    };
  }, [onIntent, onError]);
}
