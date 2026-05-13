// Tree — depth-1 entries of the active project. C3: refresh-key triggers re-fetch.
import * as React from "react";
import type { IpcSurface, TreeEntry } from "../ipc";

const DEFAULT_IGNORED = new Set([
  ".git", "node_modules", "target", "dist", "build", ".next", ".venv",
]);

export interface TreeProps {
  ipc: IpcSurface;
  projectRoot: string;
  onSelectFile?: (path: string) => void;
  refreshKey?: number;
}

export default function Tree({
  ipc, projectRoot, onSelectFile, refreshKey = 0,
}: TreeProps): React.ReactElement {
  const [entries, setEntries] = React.useState<TreeEntry[] | null>(null);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    let cancelled = false;
    setEntries(null);
    setError(null);
    ipc.listDir(projectRoot)
      .then((list) => {
        if (!cancelled) {
          setEntries(list);
          setError(null);
        }
      })
      .catch((e: Error) => {
        if (!cancelled) {
          setError(e.message);
          setEntries([]);
        }
      });
    return () => { cancelled = true; };
  }, [ipc, projectRoot, refreshKey]);

  if (error !== null) {
    return (
      <div role="alert" data-testid="tree-error" style={{ color: "red", padding: "8px" }}>
        {error}
      </div>
    );
  }

  if (entries === null) {
    return <div style={{ padding: "8px" }}><span>Loading…</span></div>;
  }

  const filtered = entries.filter((e) => !DEFAULT_IGNORED.has(e.name));

  return (
    <ul>
      {filtered.map((entry) => (
        <li
          key={entry.path}
          data-kind={entry.is_dir ? "dir" : "file"}
          onClick={() => { if (!entry.is_dir) onSelectFile?.(entry.path); }}
          style={{ cursor: entry.is_dir ? "default" : "pointer" }}
        >
          <span aria-hidden="true">{entry.is_dir ? "📁 " : "📄 "}</span>
          <span>{entry.name}</span>
        </li>
      ))}
    </ul>
  );
}
