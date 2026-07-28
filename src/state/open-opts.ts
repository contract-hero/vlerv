// Click-modifier convention for opening files — pure, shared by every row
// and in-preview link (Explorer, Bookmarks, StartPage, QuickOpen, links).
export interface OpenFileOptions {
  newTab?: boolean;
  background?: boolean;
  /** Override the computed in-root/external classification. */
  external?: boolean;
}

/**
 * ⌘-click → background new tab, ⌘⇧-click → foreground new tab,
 * middle-click → background new tab, plain click → undefined (same tab).
 */
export function openOptsFromClick(e: {
  metaKey?: boolean;
  ctrlKey?: boolean;
  shiftKey?: boolean;
  button?: number;
}): OpenFileOptions | undefined {
  const meta = Boolean(e.metaKey || e.ctrlKey);
  const middle = e.button === 1;
  if (!meta && !middle) return undefined;
  return { newTab: true, background: !e.shiftKey };
}
