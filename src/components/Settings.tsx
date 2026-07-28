// Settings panel — workspace roots / ignore-set / drag-out / Slack share target.
import * as React from "react";
import { useSettings } from "../hooks/useSettings";
import { defaultIpc } from "../ipc";

export interface SettingsProps {
  ipc?: typeof defaultIpc;
}

export default function Settings({ ipc = defaultIpc }: SettingsProps): React.ReactElement {
  const { state, setStateField } = useSettings(ipc);
  const [newGlob, setNewGlob] = React.useState("");
  // Last committed Slack target; null = nothing committed this session yet.
  // Tracked in a ref (not from `state`) because the hook state doesn't
  // refresh after a write, and Enter-then-blur would double-commit.
  const lastSlackTarget = React.useRef<string | null>(null);

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

  const handleSlackTargetCommit = async (value: string) => {
    const trimmed = value.trim();
    // Enter-then-blur fires this twice; skip the redundant IPC write.
    const previous = lastSlackTarget.current ?? (state.preferences?.slack_target ?? "");
    if (trimmed === previous) return;
    lastSlackTarget.current = trimmed;
    try {
      await setStateField("preferences.slack_target", trimmed.length > 0 ? trimmed : null);
    } catch {
      // Roll back so a retry with the same value isn't silently skipped.
      lastSlackTarget.current = previous;
    }
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

      <section>
        <h3>Slack Share Target</h3>
        <p>
          Slack has no macOS share-sheet extension, so Vlervcode opens your
          channel via a deep link instead — drag the file in from there.
          Accepts <code>TEAMID/CHANNELID</code> (e.g.{" "}
          <code>T0123ABCD/C0456EFGH</code>) or a full <code>slack://</code> URL.
        </p>
        <input
          type="text"
          data-field="slack-target"
          aria-label="Slack share target"
          defaultValue={state.preferences?.slack_target ?? ""}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              void handleSlackTargetCommit(e.currentTarget.value);
            }
          }}
          onBlur={(e) => void handleSlackTargetCommit(e.currentTarget.value)}
          placeholder="T0123ABCD/C0456EFGH"
        />
      </section>
    </div>
  );
}
