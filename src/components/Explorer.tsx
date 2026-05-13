// Explorer — recursive file tree with inline folder expansion + native drag-out.
import * as React from "react";
import { startDrag } from "@crabnebula/tauri-plugin-drag";
import { resolveResource } from "@tauri-apps/api/path";
import type { IpcSurface, TreeEntry } from "../ipc";

// Drag-preview icon: resolved from the bundled `resources` declaration in
// tauri.conf.json. Cached on first call so subsequent drags don't hit IPC.
let cachedDragIcon: string | null = null;
async function dragIcon(): Promise<string | null> {
  if (cachedDragIcon !== null) return cachedDragIcon;
  try {
    cachedDragIcon = await resolveResource("icons/drag-icon.png");
    return cachedDragIcon;
  } catch {
    return null;
  }
}

const DEFAULT_IGNORED = new Set([
  ".git", "node_modules", "target", "dist", "build", ".next", ".venv",
]);

export interface ExplorerProps {
  ipc: IpcSurface;
  root: string;
  onSelectFile?: (path: string) => void;
  selectedFile?: string | null;
}

function sortEntries(entries: TreeEntry[]): TreeEntry[] {
  return [...entries].sort((a, b) => {
    if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
    return a.name.toLowerCase().localeCompare(b.name.toLowerCase());
  });
}

interface NodeProps {
  ipc: IpcSurface;
  entry: TreeEntry;
  depth: number;
  onSelectFile?: (path: string) => void;
  selectedFile?: string | null;
}

function FolderNode({ ipc, entry, depth, onSelectFile, selectedFile }: NodeProps): React.ReactElement {
  const [expanded, setExpanded] = React.useState(false);
  const [entries, setEntries] = React.useState<TreeEntry[] | null>(null);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (!expanded || entries !== null) return;
    let cancelled = false;
    ipc.listDir(entry.path)
      .then((list) => {
        if (!cancelled) setEntries(list.filter((e) => !DEFAULT_IGNORED.has(e.name)));
      })
      .catch((e: Error) => {
        if (!cancelled) setError(e.message);
      });
    return () => { cancelled = true; };
  }, [expanded, entry.path, entries, ipc]);

  const indentPx = 10 + depth * 14;
  const isSelected = selectedFile === entry.path;

  return (
    <>
      <div
        className={`row folder ${isSelected ? "selected" : ""}`}
        style={{ paddingLeft: `${indentPx}px` }}
        onClick={() => setExpanded((v) => !v)}
        data-path={entry.path}
        data-kind="dir"
      >
        <span className={`chevron ${expanded ? "open" : ""}`}>▸</span>
        <span className="icon folder-icon" aria-hidden>{expanded ? "📂" : "📁"}</span>
        <span className="label">{entry.name}</span>
      </div>
      {expanded && error !== null && (
        <div className="row error-row" style={{ paddingLeft: `${indentPx + 20}px` }}>
          {error}
        </div>
      )}
      {expanded && entries !== null && sortEntries(entries).map((child) =>
        child.is_dir ? (
          <FolderNode
            key={child.path}
            ipc={ipc}
            entry={child}
            depth={depth + 1}
            onSelectFile={onSelectFile}
            selectedFile={selectedFile}
          />
        ) : (
          <FileRow
            key={child.path}
            entry={child}
            depth={depth + 1}
            onSelectFile={onSelectFile}
            selected={selectedFile === child.path}
          />
        )
      )}
    </>
  );
}

interface FileRowProps {
  entry: TreeEntry;
  depth: number;
  onSelectFile?: (path: string) => void;
  selected: boolean;
}

function FileRow({ entry, depth, onSelectFile, selected }: FileRowProps): React.ReactElement {
  const indentPx = 10 + depth * 14;
  return (
    <div
      className={`row file ${selected ? "selected" : ""}`}
      style={{ paddingLeft: `${indentPx}px` }}
      onClick={() => onSelectFile?.(entry.path)}
      draggable
      onDragStart={(e) => {
        e.preventDefault();
        void (async () => {
          const icon = await dragIcon();
          if (!icon) return;
          await startDrag({ item: [entry.path], icon }).catch(() => {
            // ignore — plugin may not be available in some environments
          });
        })();
      }}
      data-path={entry.path}
      data-kind="file"
    >
      <span className="chevron-placeholder" aria-hidden>·</span>
      <span className="icon file-icon" aria-hidden>📄</span>
      <span className="label">{entry.name}</span>
    </div>
  );
}

export default function Explorer({ ipc, root, onSelectFile, selectedFile }: ExplorerProps): React.ReactElement {
  const [entries, setEntries] = React.useState<TreeEntry[] | null>(null);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    let cancelled = false;
    setEntries(null);
    setError(null);
    ipc.listDir(root)
      .then((list) => {
        if (!cancelled) setEntries(list.filter((e) => !DEFAULT_IGNORED.has(e.name)));
      })
      .catch((e: Error) => {
        if (!cancelled) setError(e.message);
      });
    return () => { cancelled = true; };
  }, [ipc, root]);

  if (error !== null) {
    return <div role="alert" className="explorer-error">{error}</div>;
  }
  if (entries === null) {
    return <div className="explorer-loading">Loading…</div>;
  }

  return (
    <div className="explorer">
      {sortEntries(entries).map((entry) =>
        entry.is_dir ? (
          <FolderNode
            key={entry.path}
            ipc={ipc}
            entry={entry}
            depth={0}
            onSelectFile={onSelectFile}
            selectedFile={selectedFile}
          />
        ) : (
          <FileRow
            key={entry.path}
            entry={entry}
            depth={0}
            onSelectFile={onSelectFile}
            selected={selectedFile === entry.path}
          />
        )
      )}
    </div>
  );
}
