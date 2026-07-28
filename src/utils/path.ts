// Shared path predicates. Centralized so Sidebar.tsx and App.tsx see the same
// in-root / external-file semantics.

/**
 * Naive prefix match — sufficient for the badge UX (sibling files of a
 * known-in-root file are in-root). Does NOT canonicalize: symlinks pointing
 * across roots and neighbour-prefix collisions (e.g. `/foo/bar` vs `/foo/ba`)
 * are known edge cases. The Rust security gate (`canonicalize_and_check_root`)
 * is the authoritative check for any code path that actually reads the file.
 */
/** Final path component (POSIX — macOS-only app). */
export function basename(path: string): string {
  return path.replace(/^.*\//, "") || path;
}

/** Containing directory (POSIX). Root-level paths return "/". */
export function dirname(path: string): string {
  const idx = path.lastIndexOf("/");
  return idx <= 0 ? "/" : path.slice(0, idx);
}

export function isUnderRoot(path: string, root: string | null | undefined): boolean {
  if (!root) return false;
  const normalized = root.endsWith("/") ? root : `${root}/`;
  return path === root || path.startsWith(normalized);
}

export type PathBarNormalization =
  | { kind: "ok"; path: string }
  | { kind: "error"; reason: string };

/**
 * Normalize a value pasted/typed into the path bar (Feature D). Accepts:
 *  - raw absolute paths (`/foo/bar.html`)
 *  - `file://` URLs
 *  - `vlerv://open?path=…` and `vlerv://reveal?path=…` URLs
 *
 * Rejects empty input, non-absolute paths, and NUL bytes. Percent-decoding
 * happens once (matching `dispatch_deep_link` semantics — double-encoded paths
 * are the user's problem).
 */
export function normalizePathBarInput(raw: string): PathBarNormalization {
  const trimmed = raw.trim();
  if (!trimmed) return { kind: "error", reason: "Path is empty" };

  let candidate = trimmed;

  if (candidate.startsWith("vlerv://")) {
    const queryIndex = candidate.indexOf("?");
    if (queryIndex === -1) {
      return { kind: "error", reason: "vlerv:// URL is missing a path" };
    }
    const query = candidate.slice(queryIndex + 1);
    const params = new URLSearchParams(query);
    const path = params.get("path");
    if (!path) return { kind: "error", reason: "vlerv:// URL has no path= parameter" };
    candidate = path;
  } else if (candidate.startsWith("file://")) {
    candidate = candidate.slice("file://".length);
  }

  try {
    candidate = decodeURIComponent(candidate);
  } catch {
    return { kind: "error", reason: "Path contains invalid percent-encoding" };
  }

  if (candidate.includes("\0")) {
    return { kind: "error", reason: "Path contains a NUL byte" };
  }
  if (!candidate.startsWith("/")) {
    return { kind: "error", reason: "Path must be absolute (start with /)" };
  }

  return { kind: "ok", path: candidate };
}
