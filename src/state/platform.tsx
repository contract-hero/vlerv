// Platform context — the one place the frontend learns which shell it is
// running in. Backed by the `platform_info` Tauri command (owned by the
// parallel Rust agent); this file only has to survive that command being
// slow, missing, or wrong-shaped, since the two sides are landing at once.
//
// macOS is the safe fallback everywhere: it is today's only shipped
// platform, and every existing surface already assumes it. A failed or
// stale read must never silently switch the desktop build into the iOS
// (read-only, workspace-less) posture.
import * as React from "react";
import type { IpcSurface } from "../ipc";

export type PlatformOs = "macos" | "ios";

export const DEFAULT_PLATFORM_OS: PlatformOs = "macos";

/** Narrow an arbitrary `platform_info()` response to a known OS, pure and
 * total: anything that isn't exactly `{ os: "ios" }` falls back to macOS
 * rather than throwing, so a future third platform value degrades instead
 * of breaking iOS/macOS branching. */
export function parsePlatformOs(raw: unknown): PlatformOs {
  if (raw && typeof raw === "object" && "os" in raw) {
    const os = (raw as { os?: unknown }).os;
    if (os === "ios") return "ios";
  }
  return DEFAULT_PLATFORM_OS;
}

/** Resolve the platform once, swallowing every failure mode: the command
 * doesn't exist on this build (`ipc.platformInfo` absent), it rejects, or it
 * resolves to something unexpected. */
export async function resolvePlatformOs(ipc: Pick<IpcSurface, "platformInfo">): Promise<PlatformOs> {
  if (!ipc.platformInfo) return DEFAULT_PLATFORM_OS;
  try {
    return parsePlatformOs(await ipc.platformInfo());
  } catch {
    return DEFAULT_PLATFORM_OS;
  }
}

/** The `<body>` class an iOS-specific stylesheet rule hooks (larger hit
 * targets, safe-area padding) — kept as a pure function so it's testable
 * without mounting the provider. */
export function platformBodyClass(os: PlatformOs): string {
  return os === "ios" ? "platform-ios" : "platform-macos";
}

export interface PlatformContextValue {
  os: PlatformOs;
  isIos: boolean;
  isMacos: boolean;
  /** True until the first `platform_info` resolution lands. Callers that
   * must not flash macOS-only chrome before the real answer arrives (there
   * are none yet — iOS launches straight into its own bundle) can gate on
   * this; everyone else can ignore it and take the safe `macos` default. */
  loading: boolean;
}

const PlatformContext = React.createContext<PlatformContextValue>({
  os: DEFAULT_PLATFORM_OS,
  isIos: false,
  isMacos: true,
  loading: false,
});

export function usePlatform(): PlatformContextValue {
  return React.useContext(PlatformContext);
}

export function PlatformProvider({
  ipc,
  children,
}: {
  ipc: IpcSurface;
  children: React.ReactNode;
}): React.ReactElement {
  const [os, setOs] = React.useState<PlatformOs>(DEFAULT_PLATFORM_OS);
  const [loading, setLoading] = React.useState(true);

  React.useEffect(() => {
    let cancelled = false;
    resolvePlatformOs(ipc).then((resolved) => {
      if (!cancelled) {
        setOs(resolved);
        setLoading(false);
      }
    });
    return () => {
      cancelled = true;
    };
  }, [ipc]);

  // Drives `<body class="platform-ios|platform-macos">` for the CSS-only
  // affordances (safe-area padding, larger touch targets) — a side effect,
  // so it belongs here rather than in render.
  React.useEffect(() => {
    const body = globalThis.document?.body;
    if (!body) return;
    body.classList.remove("platform-ios", "platform-macos");
    body.classList.add(platformBodyClass(os));
  }, [os]);

  const value = React.useMemo(
    (): PlatformContextValue => ({ os, isIos: os === "ios", isMacos: os === "macos", loading }),
    [os, loading],
  );

  return <PlatformContext.Provider value={value}>{children}</PlatformContext.Provider>;
}
