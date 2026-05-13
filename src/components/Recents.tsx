// Recents group component — rendered inside Sidebar above the project list.
// C3: uses useRecents hook; clicking an entry calls ipc.readFile AND emits vlerv://reveal.
import * as React from "react";
import { useRecents } from "../hooks/useRecents";
import { defaultIpc } from "../ipc";

export interface RecentsProps {
  ipc?: typeof defaultIpc;
}

export default function Recents({ ipc = defaultIpc }: RecentsProps): React.ReactElement {
  const { recents } = useRecents(ipc);

  const handleClick = (path: string) => {
    // T-137: call ipc.readFile for preview switch.
    void ipc.readFile(path);
    // T-137: emit vlerv://reveal window event for tree expansion.
    window.dispatchEvent(
      new CustomEvent("vlerv://reveal", { detail: { path } }),
    );
  };

  if (recents.length === 0) {
    return <div data-section="recents" data-testid="recents-group" />;
  }

  return (
    <div data-section="recents" data-testid="recents-group">
      <h4>Recent Files</h4>
      <ul>
        {recents.map((entry) => (
          <li
            key={entry.path}
            data-recent-path={entry.path}
            onClick={() => handleClick(entry.path)}
            style={{ cursor: "pointer" }}
          >
            {entry.path}
          </li>
        ))}
      </ul>
    </div>
  );
}
