// Shared path helpers — root predicates, POSIX basename/dirname, and
// address-bar input normalization. Centralized so every pane sees the same
// in-root / external-file semantics.

/** Final path component (POSIX — macOS-only app). */
export function basename(path: string): string {
  return path.replace(/^.*\//, "") || path;
}

/** Containing directory (POSIX). Root-level paths return "/". */
export function dirname(path: string): string {
  const idx = path.lastIndexOf("/");
  return idx <= 0 ? "/" : path.slice(0, idx);
}

/**
 * Naive prefix match — sufficient for the badge UX (sibling files of a
 * known-in-root file are in-root). Does NOT canonicalize: symlinks pointing
 * across roots and neighbour-prefix collisions (e.g. `/foo/bar` vs `/foo/ba`)
 * are known edge cases. The Rust security gate (`canonicalize_and_check_root`)
 * is the authoritative check for any code path that actually reads the file.
 */
/**
 * The directory to show beside a filename in a retrieval surface. It exists to
 * tell two same-named artifacts apart, and an absolute path spends its whole
 * width on the prefix every row shares — so inside the workspace we show the
 * path relative to the root.
 *
 * The trailing slash in the prefix test is load-bearing: without it a root of
 * `/w/vlerv` would match `/w/vlerv-worktrees/x` and render a relative path
 * that does not exist. Git worktrees produce exactly that layout.
 */
export function displayDir(path: string, root: string | null): string {
  const dir = dirname(path);
  if (!root) return dir;
  if (dir === root) return "";
  if (dir.startsWith(`${root}/`)) return dir.slice(root.length + 1);
  return dir;
}

/**
 * What the idle address bar shows: the path relative to the workspace root.
 * The absolute prefix is identical on every workspace file — pure noise in a
 * reading surface — and editing still operates on the full path (the bar
 * swaps back to it on focus). Out-of-root paths stay absolute.
 */
export function displayPath(path: string, root: string | null): string {
  if (root && path !== root && isUnderRoot(path, root)) {
    return path.slice(root.endsWith("/") ? root.length : root.length + 1);
  }
  return path;
}

export function isUnderRoot(path: string, root: string | null | undefined): boolean {
  if (!root) return false;
  const normalized = root.endsWith("/") ? root : `${root}/`;
  return path === root || path.startsWith(normalized);
}

export type PathBarNormalization =
  | { kind: "ok"; path: string }
  | { kind: "error"; reason: string };

/** Strict single percent-decode; error result on bad sequences. */
function decodeOnce(value: string): { ok: true; value: string } | { ok: false } {
  try {
    return { ok: true, value: decodeURIComponent(value) };
  } catch {
    return { ok: false };
  }
}

/**
 * Normalize a value pasted/typed into the address bar. Accepts:
 *  - raw absolute paths (`/foo/bar.html`) — used LITERALLY, no decoding, so
 *    real files containing `%` open fine
 *  - `file://` URLs — percent-decoded exactly once
 *  - `vlerv://open?path=…` / `vlerv://reveal?path=…` — the `path` param is
 *    percent-decoded exactly once, matching `dispatch_deep_link` semantics
 *    (`+` is NOT form-decoded to a space, unlike URLSearchParams)
 *
 * Rejects empty input, non-absolute paths, NUL bytes, and invalid
 * percent-encoding in URL forms.
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
    // Manual query parsing: URLSearchParams would form-decode `+` to a space
    // and percent-decode, and we must decode exactly once with `+` untouched
    // to stay byte-compatible with the Rust deep-link parser.
    let rawPath: string | null = null;
    for (const pair of candidate.slice(queryIndex + 1).split("&")) {
      const eq = pair.indexOf("=");
      const key = eq === -1 ? pair : pair.slice(0, eq);
      if (key === "path") {
        rawPath = eq === -1 ? "" : pair.slice(eq + 1);
        break;
      }
    }
    if (!rawPath) return { kind: "error", reason: "vlerv:// URL has no path= parameter" };
    const decoded = decodeOnce(rawPath);
    if (!decoded.ok) return { kind: "error", reason: "Path contains invalid percent-encoding" };
    candidate = decoded.value;
  } else if (candidate.startsWith("file://")) {
    const decoded = decodeOnce(candidate.slice("file://".length));
    if (!decoded.ok) return { kind: "error", reason: "Path contains invalid percent-encoding" };
    candidate = decoded.value;
  }
  // Raw pasted paths are used literally — a path out of Finder or `pwd` is
  // not percent-encoded, and decoding it would break names containing `%`.

  if (candidate.includes("\0")) {
    return { kind: "error", reason: "Path contains a NUL byte" };
  }
  if (!candidate.startsWith("/")) {
    return { kind: "error", reason: "Path must be absolute (start with /)" };
  }

  return { kind: "ok", path: candidate };
}
