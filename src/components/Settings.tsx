// Settings panel — workspace roots / ignore-set / drag-out / Slack share
// target / Remote pairing (design §6: "finally a concrete reason to mount
// the Settings surface").
import * as React from "react";
import { Check, Copy } from "lucide-react";
import { useSettings } from "../hooks/useSettings";
import type { UseSettingsState } from "../hooks/useSettings";
import { defaultIpc } from "../ipc";
import type { RemoteScope } from "../ipc";
import { useRemoteActions, useRemoteState } from "../state/remote";
import { useCopyFeedback } from "../hooks/useCopyFeedback";

export interface SettingsProps {
  ipc?: typeof defaultIpc;
}

const SCOPE_LABEL: Record<RemoteScope, string> = {
  "view-open": "View open",
  browse: "Browse",
  control: "Control",
};

function CopyPairLinkButton({ link }: { link: string }): React.ReactElement {
  const { copied, copy } = useCopyFeedback();
  return (
    <button type="button" className="button" onClick={() => copy(link)}>
      {copied ? <Check size={13} strokeWidth={2.5} /> : <Copy size={13} strokeWidth={2} />}
      {copied ? "Copied" : "Copy link"}
    </button>
  );
}

function RemoteSettings({
  state,
  setStateField,
}: {
  state: UseSettingsState | null;
  setStateField: (key: string, value: unknown) => Promise<void>;
}): React.ReactElement {
  const { peers, presence, invite, pairingBusy, pairingError } = useRemoteState();
  const { beginPairing, dismissInvite, unpair, setScope, subscribePeer } = useRemoteActions();
  const listenOn = state?.preferences?.remote_listen ?? false;

  return (
    <section>
      <h3>Remote</h3>
      <p>
        Pair with another Vlervtifacts instance to browse its open tabs, live,
        and — with Control scope — open artifacts on it remotely. Transport is
        peer-to-peer and end-to-end encrypted (no server ever sees content).
      </p>
      <label>
        <input
          type="checkbox"
          checked={listenOn}
          onChange={(e) => void setStateField("preferences.remote_listen", e.target.checked)}
        />
        {" "}Listen for paired devices at launch
      </label>

      <div style={{ marginTop: "var(--space-3)" }}>
        {invite ? (
          <>
            <input
              className="beam-link-input"
              readOnly
              value={invite.link}
              onFocus={(e) => e.currentTarget.select()}
              data-testid="remote-pair-link"
            />
            <p className="beam-hint">
              Open this link on the other machine. Both screens will show the
              same six words — confirm they match there before accepting.
            </p>
            <div className="beam-actions">
              <CopyPairLinkButton link={invite.link} />
              <button type="button" className="button button-secondary" onClick={dismissInvite}>
                Done
              </button>
            </div>
          </>
        ) : (
          <button type="button" className="button" disabled={pairingBusy} onClick={beginPairing}>
            {pairingBusy ? "Starting…" : "Pair a device…"}
          </button>
        )}
        {pairingError ? <p className="beam-error" role="alert">{pairingError}</p> : null}
      </div>

      <h4 style={{ marginTop: "var(--space-4)" }}>Paired devices</h4>
      {peers.length === 0 ? (
        <p className="section-empty-hint">No devices paired yet.</p>
      ) : (
        <ul>
          {peers.map((peer) => {
            const peerPresence = presence[peer.node_id]?.state ?? "offline";
            return (
              <li key={peer.node_id} className="remote-peer-row">
                <span
                  className={`remote-presence-dot remote-presence-${peerPresence}`}
                  title={peerPresence}
                  aria-hidden
                />
                <div className="remote-peer-info">
                  <span className="remote-peer-name">{peer.device}</span>
                  <span className="remote-peer-meta" title={peer.node_id}>
                    {peer.node_id.slice(0, 12)}…
                  </span>
                </div>
                <select
                  aria-label={`Scope for ${peer.device}`}
                  value={peer.scope}
                  onChange={(e) => setScope(peer.node_id, e.target.value as RemoteScope)}
                >
                  {(Object.keys(SCOPE_LABEL) as RemoteScope[]).map((s) => (
                    <option key={s} value={s}>{SCOPE_LABEL[s]}</option>
                  ))}
                </select>
                {peerPresence === "offline" ? (
                  <button
                    type="button"
                    className="button button-secondary"
                    onClick={() => subscribePeer(peer.node_id)}
                  >
                    Connect
                  </button>
                ) : null}
                <button
                  type="button"
                  className="button button-secondary"
                  onClick={() => unpair(peer.node_id)}
                >
                  Unpair
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
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
          Slack has no macOS share-sheet extension, so Vlervtifacts opens your
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

      <RemoteSettings state={state} setStateField={setStateField} />
    </div>
  );
}
