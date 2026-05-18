// useTheme — follow the macOS system appearance via Tauri's window theme API,
// fall back to matchMedia in non-Tauri contexts (tests). Sets
// `<html data-theme="dark|light">` as a single source of truth so CSS rules
// and consumers (Shiki, HTML iframe injection) can both read it.
import * as React from "react";

export type Theme = "dark" | "light";

function applyDocumentTheme(theme: Theme): void {
  if (typeof document === "undefined") return;
  document.documentElement.setAttribute("data-theme", theme);
}

function readMatchMediaTheme(): Theme {
  if (typeof window === "undefined" || !window.matchMedia) return "dark";
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

/**
 * Subscribe to system theme. Mounts once at the root and drives the global
 * `<html data-theme>` attribute; consumers can either read this hook's
 * return value or `document.documentElement.dataset.theme` directly.
 *
 * Tauri 2's `getCurrentWindow().theme()` is the source of truth on macOS;
 * matchMedia is the fallback for unit tests / non-Tauri environments.
 */
export function useTheme(): Theme {
  // Initial paint: matchMedia is synchronous and good enough; the Tauri probe
  // below corrects it on the next tick if it disagrees.
  const [theme, setTheme] = React.useState<Theme>(() => {
    if (typeof document !== "undefined") {
      const existing = document.documentElement.getAttribute("data-theme");
      if (existing === "light" || existing === "dark") return existing;
    }
    return readMatchMediaTheme();
  });

  React.useEffect(() => {
    applyDocumentTheme(theme);
  }, [theme]);

  React.useEffect(() => {
    let unlistenTauri: (() => void) | null = null;
    let cancelled = false;

    // Try Tauri first.
    (async () => {
      try {
        const mod = await import("@tauri-apps/api/window");
        const win = mod.getCurrentWindow();
        const current = await win.theme();
        if (cancelled) return;
        if (current === "dark" || current === "light") {
          setTheme(current);
        }
        const unlisten = await win.onThemeChanged(({ payload }) => {
          if (payload === "dark" || payload === "light") {
            setTheme(payload);
          }
        });
        if (cancelled) {
          unlisten();
          return;
        }
        unlistenTauri = unlisten;
      } catch {
        // Non-Tauri context — fall back to matchMedia subscription.
      }
    })();

    // matchMedia fallback: always wired so the hook also updates when running
    // outside Tauri. In Tauri the Tauri event fires first; both staying
    // consistent is fine because setState bails on equal values.
    let mql: MediaQueryList | null = null;
    let mqlListener: ((e: MediaQueryListEvent) => void) | null = null;
    if (typeof window !== "undefined" && window.matchMedia) {
      mql = window.matchMedia("(prefers-color-scheme: dark)");
      mqlListener = (e: MediaQueryListEvent) => setTheme(e.matches ? "dark" : "light");
      mql.addEventListener("change", mqlListener);
    }

    return () => {
      cancelled = true;
      if (unlistenTauri) unlistenTauri();
      if (mql && mqlListener) mql.removeEventListener("change", mqlListener);
    };
  }, []);

  return theme;
}
