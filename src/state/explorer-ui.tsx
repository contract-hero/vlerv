// ExplorerUiContext — hoisted tree-expansion state + reveal machinery.
// FolderNode expansion lives here (not in per-node useState) so reveal can
// expand a whole ancestor chain in one update, and so deep-link "reveal"
// intent finally works.
import * as React from "react";

export interface RevealTarget {
  path: string;
  nonce: number;
}

export interface ExplorerUi {
  expandedPaths: ReadonlySet<string>;
  toggle(path: string): void;
  collapse(path: string): void;
  expand(path: string): void;
  /** Expand every ancestor of `path` (within `root`) and arm the scroll
   *  target; the FileRow scrolls itself into view when it mounts. */
  reveal(path: string, root: string | null): void;
  revealTarget: RevealTarget | null;
  /** Roving-tabindex focus for tree keyboard navigation. */
  focusedPath: string | null;
  setFocusedPath(path: string | null): void;
}

const noop = () => {};
const ExplorerUiContext = React.createContext<ExplorerUi>({
  expandedPaths: new Set(),
  toggle: noop,
  collapse: noop,
  expand: noop,
  reveal: noop,
  revealTarget: null,
  focusedPath: null,
  setFocusedPath: noop,
});

export function useExplorerUi(): ExplorerUi {
  return React.useContext(ExplorerUiContext);
}

/** Ancestor directories of `path` strictly below `root` (POSIX paths). */
export function ancestorsWithin(path: string, root: string): string[] {
  const rootNorm = root.endsWith("/") ? root.slice(0, -1) : root;
  if (!path.startsWith(`${rootNorm}/`)) return [];
  const out: string[] = [];
  let dir = path.slice(0, path.lastIndexOf("/"));
  while (dir.length > rootNorm.length) {
    out.unshift(dir);
    dir = dir.slice(0, dir.lastIndexOf("/"));
  }
  return out;
}

export function ExplorerUiProvider({ children }: { children: React.ReactNode }): React.ReactElement {
  const [expandedPaths, setExpandedPaths] = React.useState<ReadonlySet<string>>(() => new Set());
  const [revealTarget, setRevealTarget] = React.useState<RevealTarget | null>(null);
  const [focusedPath, setFocusedPath] = React.useState<string | null>(null);
  const revealNonce = React.useRef(0);

  const toggle = React.useCallback((path: string) => {
    setExpandedPaths((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  }, []);

  const expand = React.useCallback((path: string) => {
    setExpandedPaths((prev) => {
      if (prev.has(path)) return prev;
      const next = new Set(prev);
      next.add(path);
      return next;
    });
  }, []);

  const collapse = React.useCallback((path: string) => {
    setExpandedPaths((prev) => {
      if (!prev.has(path)) return prev;
      const next = new Set(prev);
      next.delete(path);
      return next;
    });
  }, []);

  const reveal = React.useCallback((path: string, root: string | null) => {
    if (!root) return;
    const chain = ancestorsWithin(path, root);
    setExpandedPaths((prev) => {
      const missing = chain.filter((p) => !prev.has(p));
      if (missing.length === 0) return prev;
      const next = new Set(prev);
      for (const p of missing) next.add(p);
      return next;
    });
    revealNonce.current += 1;
    // Capture the nonce NOW: reading the ref at fire time would let an older
    // timer clear a newer reveal target before its row has scrolled into view.
    const nonce = revealNonce.current;
    setRevealTarget({ path, nonce });
    // Clear after the async expand-cascade has had time to render, so stale
    // targets don't re-scroll on later unrelated re-mounts.
    window.setTimeout(() => {
      setRevealTarget((cur) => (cur?.nonce === nonce ? null : cur));
    }, 2000);
  }, []);

  const value = React.useMemo<ExplorerUi>(
    () => ({
      expandedPaths,
      toggle,
      expand,
      collapse,
      reveal,
      revealTarget,
      focusedPath,
      setFocusedPath,
    }),
    [expandedPaths, toggle, expand, collapse, reveal, revealTarget, focusedPath],
  );

  return <ExplorerUiContext.Provider value={value}>{children}</ExplorerUiContext.Provider>;
}
