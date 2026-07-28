// QuickOpen — ⌘P fuzzy file palette over the workspace file index.
import * as React from "react";
import { Search } from "lucide-react";
import type { IpcSurface } from "../ipc";
import { fuzzyFilter } from "../utils/fuzzy";
import { useWatcherBus } from "../state/watcher-bus";
import { openOptsFromClick } from "../state/TabsProvider";
import type { OpenFileOptions } from "../state/TabsProvider";
import { FileGlyph } from "./FileIcon";
import { basename } from "../utils/path";

const RESULT_LIMIT = 50;

export interface QuickOpenProps {
  ipc: IpcSurface;
  root: string;
  onOpenFile: (path: string, opts?: OpenFileOptions) => void;
  onClose: () => void;
}

export default function QuickOpen({ ipc, root, onOpenFile, onClose }: QuickOpenProps): React.ReactElement {
  const [query, setQuery] = React.useState("");
  const [files, setFiles] = React.useState<string[] | null>(null);
  const [truncated, setTruncated] = React.useState(false);
  const [selectedIndex, setSelectedIndex] = React.useState(0);
  const inputRef = React.useRef<HTMLInputElement | null>(null);
  const listRef = React.useRef<HTMLUListElement | null>(null);
  const bus = useWatcherBus();

  React.useEffect(() => {
    inputRef.current?.focus();
  }, []);

  // Fetch the index once per open; re-fetch if the tree changes while open.
  const staleRef = React.useRef(false);
  React.useEffect(() => {
    let cancelled = false;
    let refreshTimer: number | null = null;
    const fetchIndex = () => {
      if (!ipc.listFilesRecursive) return;
      ipc
        .listFilesRecursive(root)
        .then((idx) => {
          if (cancelled) return;
          setFiles(idx.files);
          setTruncated(idx.truncated);
        })
        .catch(() => {
          if (!cancelled) setFiles([]);
        });
    };
    fetchIndex();
    const unsubscribe = bus.subscribe((change) => {
      if (change.source !== "tree" || staleRef.current) return;
      staleRef.current = true;
      // Refresh at most once per second of churn.
      refreshTimer = window.setTimeout(() => {
        refreshTimer = null;
        staleRef.current = false;
        fetchIndex();
      }, 1000);
    });
    return () => {
      cancelled = true;
      unsubscribe();
      // Don't let a pending refresh fire a full workspace walk after ⌘P is
      // already dismissed.
      if (refreshTimer !== null) window.clearTimeout(refreshTimer);
    };
  }, [ipc, root, bus]);

  const results = React.useMemo(() => {
    if (files === null) return [];
    if (!query.trim()) return files.slice(0, RESULT_LIMIT).map((value) => ({ value, score: 0 }));
    return fuzzyFilter(query.trim(), files, RESULT_LIMIT);
  }, [files, query]);

  React.useEffect(() => {
    setSelectedIndex(0);
  }, [query]);

  React.useEffect(() => {
    const el = listRef.current?.children[selectedIndex] as HTMLElement | undefined;
    el?.scrollIntoView({ block: "nearest" });
  }, [selectedIndex]);

  const open = (relPath: string, opts?: OpenFileOptions) => {
    onOpenFile(`${root}/${relPath}`, opts);
    onClose();
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      setSelectedIndex((i) => Math.min(results.length - 1, i + 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSelectedIndex((i) => Math.max(0, i - 1));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const hit = results[selectedIndex];
      if (hit) {
        open(hit.value, openOptsFromClick(e));
      }
    }
  };

  return (
    <div className="quick-open-backdrop" onMouseDown={onClose} data-testid="quick-open">
      <div
        className="quick-open"
        role="dialog"
        aria-label="Quick open"
        onMouseDown={(e) => e.stopPropagation()}
        onKeyDown={onKeyDown}
      >
        <div className="quick-open-input">
          <Search size={14} strokeWidth={2} aria-hidden />
          <input
            ref={inputRef}
            type="text"
            placeholder="Go to file…"
            spellCheck={false}
            autoCapitalize="off"
            autoCorrect="off"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </div>
        <ul className="quick-open-list" ref={listRef} role="listbox">
          {results.map((r, i) => (
            <li
              key={r.value}
              role="option"
              aria-selected={i === selectedIndex}
              className={i === selectedIndex ? "selected" : undefined}
              onMouseEnter={() => setSelectedIndex(i)}
              onClick={(e) => open(r.value, openOptsFromClick(e))}
              title={`${root}/${r.value}`}
            >
              <span className="quick-open-icon" aria-hidden>
                <FileGlyph name={basename(r.value)} size={14} />
              </span>
              <span className="quick-open-name">{basename(r.value)}</span>
              <span className="quick-open-dir">{r.value.includes("/") ? r.value.slice(0, r.value.lastIndexOf("/")) : ""}</span>
            </li>
          ))}
          {files !== null && results.length === 0 ? (
            <li className="quick-open-empty">No matches</li>
          ) : null}
        </ul>
        {truncated ? (
          <div className="quick-open-footer">Index truncated at 20,000 files</div>
        ) : null}
      </div>
    </div>
  );
}
