// Settings panel — workspace roots / ignore-set / drag-out preference. C3.
import * as React from "react";
import { useSettings } from "../hooks/useSettings";
import { defaultIpc } from "../ipc";

export interface SettingsProps {
  ipc?: typeof defaultIpc;
}

export default function Settings({ ipc = defaultIpc }: SettingsProps): React.ReactElement {
  const { state, setStateField } = useSettings(ipc);
  const [newGlob, setNewGlob] = React.useState("");

  if (!state) {
    return <div data-testid="settings-panel">Loading…</div>;
  }

  const roots: string[] = state.roots ?? [];
  const ignoreGlobs: string[] = state.preferences?.ignore_globs ?? [];
  const dragOutMode: string = state.preferences?.drag_out_mode ?? "file";

  const handleAddRoot = async () => {
    if (ipc.pickDirectory) {
      const picked = await ipc.pickDirectory();
      if (picked) {
        await setStateField("roots", [...roots, picked]);
      }
    }
  };

  const handleRemoveRoot = async (root: string) => {
    await setStateField("roots", roots.filter((r) => r !== root));
  };

  const handleGlobCommit = async () => {
    const trimmed = newGlob.trim();
    if (trimmed) {
      await setStateField("preferences.ignore_globs", [...ignoreGlobs, trimmed]);
      setNewGlob("");
    }
  };

  const handleGlobKeyDown = async (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") {
      await handleGlobCommit();
    }
  };

  const handleGlobBlur = async () => {
    if (newGlob.trim()) {
      await handleGlobCommit();
    }
  };

  const handleDragModeChange = async (mode: "file" | "url") => {
    await setStateField("preferences.drag_out_mode", mode);
  };

  return (
    <div data-testid="settings-panel">
      <section>
        <h3>Workspace Roots</h3>
        <ul>
          {roots.map((root) => (
            <li key={root}>
              <span>{root}</span>
              <button
                aria-label={`remove ${root}`}
                data-action="remove-root"
                data-path={root}
                onClick={() => void handleRemoveRoot(root)}
              >
                Remove
              </button>
            </li>
          ))}
        </ul>
        <button onClick={() => void handleAddRoot()}>Add root…</button>
      </section>

      <section>
        <h3>Ignore Set</h3>
        <ul>
          {ignoreGlobs.map((glob) => (
            <li key={glob}>{glob}</li>
          ))}
        </ul>
        <input
          type="text"
          data-field="ignore-globs"
          aria-label="Add ignore glob"
          value={newGlob}
          onChange={(e) => setNewGlob(e.target.value)}
          onKeyDown={(e) => void handleGlobKeyDown(e)}
          onBlur={() => void handleGlobBlur()}
          placeholder="*.tmp"
        />
      </section>

      <section>
        <h3>Drag-out Preference</h3>
        <label>
          <input
            type="radio"
            name="drag-out-mode"
            value="file"
            checked={dragOutMode === "file"}
            onChange={() => void handleDragModeChange("file")}
          />
          Drag a file (Finder-friendly)
        </label>
        <label>
          <input
            type="radio"
            name="drag-out-mode"
            value="url"
            checked={dragOutMode === "url"}
            onChange={() => void handleDragModeChange("url")}
          />
          Drag a URL (Slack-friendly)
        </label>
      </section>
    </div>
  );
}
