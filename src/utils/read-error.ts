// Turning a backend read failure into something a reader can act on.

/**
 * The load path stamps every failure as kind "Io" and carries the real message
 * in `reason`, so the classification has to come from the text.
 *
 * The backend renders paths with `{:?}`, which quotes them, and a path can
 * contain any word we match on — `.../pages/NotFound.tsx` is a real filename —
 * so quoted segments are stripped before matching. Order matters after that:
 * `path is out of root: "…"` must not be read as a permission failure.
 */
export function readErrorTitle(kind: string, reason: string): string {
  const text = `${kind} ${reason}`.replace(/"[^"]*"/g, " ").toLowerCase();
  if (text.includes("out of root")) return "Outside the workspace";
  if (text.includes("permission") || text.includes("denied")) return "Permission denied";
  if (text.includes("not found") || text.includes("no such file")) return "File not found";
  return "Could not read this file";
}
