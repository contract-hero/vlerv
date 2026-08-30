// Settings panel — workspace roots / ignore-set / drag-out / Slack share
// target / Remote pairing (design §6: "finally a concrete reason to mount
// the Settings surface").
//
// One panel, two shapes. The markup is a flat list of `.settings-section`
// cards with label-left / control-right rows, which is the grammar the phone
// needs and the desktop reads fine in; `body.platform-ios` in styles.css
// grows the rows to `--tap-min` and stacks the peer actions. The content is
// what differs: the phone owns no files (PRODUCT.md — the iOS build is a
// read-only companion), so workspace roots, the ignore set, drag-out and the
// Slack target are desktop-only. Remote is the section the phone actually
// came for, and it is the whole panel there.
import * as React from "react";
import { useSettings } from "../hooks/useSettings";
import type { UseSettingsState } from "../hooks/useSettings";
import { defaultIpc } from "../ipc";
import type { RemoteScope } from "../ipc";
import { useRemoteActions, useRemoteState } from "../state/remote";
import { usePlatform } from "../state/platform";
import CopyLinkButton from "./CopyLinkButton";
import ShareLinkButton from "./ShareLinkButton";

export interface SettingsProps {
  ipc?: typeof defaultIpc;
}

const SCOPE_LABEL: Record<RemoteScope, string> = {
  "view-open": "View open",
  browse: "Browse",
  control: "Control",
};

const PRESENCE_LABEL: Record<string, string> = {
  online: "Online",
  connecting: "Connecting…",
  offline: "Offline",
};

/** A titled card. Every section in the panel is one, so the phone gets a
 *  predictable rhythm instead of a wall of headings and bare controls. */
function Section({
  title,
  hint,
  children,
}: {
  title: string;
  hint?: React.ReactNode;
  children: React.ReactNode;
}): React.ReactElement {
  return (
    <section className="settings-section">
      <h3 className="settings-section-title">{title}</h3>
      {hint ? <p className="settings-section-hint">{hint}</p> : null}
      {children}
    </section>
  );
}

/** Label (plus optional second line) on the left, one control on the right.
 *  The whole row is the hit target, which is what makes a 44pt tap work. */
function ToggleRow({
  label,
  hint,
  checked,
  onChange,
}: {
  label: string;
  hint?: string;
  checked: boolean;
  onChange: (next: boolean) => void;
}): React.ReactElement {
  return (
    <label className="settings-row">
      <span className="settings-row-text">
        <span className="settings-row-label">{label}</span>
        {hint ? <span className="settings-row-hint">{hint}</span> : null}
      </span>
      <input
        type="checkbox"
        className="settings-switch"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
      />
    </label>
  );
}

function RemoteSettings({
  ipc,
  state,
  setStateField,
}: {
  ipc: typeof defaultIpc;
  state: UseSettingsState | null;
  setStateField: (key: string, value: unknown) => Promise<void>;
}): React.ReactElement {
  const { peers, presence, invite, pairingBusy, pairingError } = useRemoteState();
  const { beginPairing, dismissInvite, unpair, setScope, subscribePeer } = useRemoteActions();
  const listenOn = state?.preferences?.remote_listen ?? false;

  return (
    <Section
      title="Remote"
      hint={
        <>
          Pair with another Vlervtifacts instance to browse its open tabs,
          live, and — with Control scope — open artifacts on it remotely.
          Transport is peer-to-peer and end-to-end encrypted (no server ever
          sees content).
        </>
      }
    >
      <ToggleRow
        label="Listen at launch"
        hint="Accept paired devices as soon as the app opens."
        checked={listenOn}
        onChange={(next) => void setStateField("preferences.remote_listen", next)}
      />

      <div className="settings-block">
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
              Open this link on the other device. Both screens will show the
              same six words — confirm they match there before accepting.
            </p>
            <div className="settings-actions">
              <CopyLinkButton link={invite.link} />
              <ShareLinkButton ipc={ipc} link={invite.link} title="Vlervtifacts pairing link" />
              <button type="button" className="button button-secondary" onClick={dismissInvite}>
                Done
              </button>
            </div>
          </>
        ) : (
          <div className="settings-actions">
            <button type="button" className="button" disabled={pairingBusy} onClick={beginPairing}>
              {pairingBusy ? "Starting…" : "Pair a device…"}
            </button>
          </div>
        )}
        {pairingError ? <p className="beam-error" role="alert">{pairingError}</p> : null}
      </div>

      <h4 className="settings-subhead">Paired devices</h4>
      {peers.length === 0 ? (
        <p className="settings-empty">No devices paired yet.</p>
      ) : (
        <ul className="settings-list">
          {peers.map((peer) => {
            const peerPresence = presence[peer.node_id]?.state ?? "offline";
            return (
              <li key={peer.node_id} className="settings-peer">
                <div className="settings-peer-info">
                  <span className="settings-peer-name">
                    <span
                      className={`remote-presence-dot remote-presence-${peerPresence}`}
                      aria-hidden
                    />
                    {peer.device}
                  </span>
                  {/* The dot alone carries presence only in a tooltip, which
                      a touch device never shows — so the word is here too. */}
                  <span className="settings-peer-meta" title={peer.node_id}>
                    {PRESENCE_LABEL[peerPresence] ?? peerPresence} · {peer.node_id.slice(0, 12)}…
                  </span>
                </div>
                <div className="settings-peer-actions">
                  <select
                    className="settings-select"
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
                    className="button button-secondary button-danger"
                    onClick={() => unpair(peer.node_id)}
                  >
                    Unpair
                  </button>
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </Section>
  );
}

/** Roots, ignore set, drag-out and the Slack target. All four act on a local
 *  workspace and a macOS share sheet, so none of them mount on the phone. */
function DesktopSettings({
  state,
  setStateField,
  ipc,
}: {
  state: UseSettingsState;
  setStateField: (key: string, value: unknown) => Promise<void>;
  ipc: typeof defaultIpc;
}): React.ReactElement {
  const [newGlob, setNewGlob] = React.useState("");
  // Last committed Slack target; null = nothing committed this session yet.
  // Tracked in a ref (not from `state`) because the hook state doesn't
  // refresh after a write, and Enter-then-blur would double-commit.
  const lastSlackTarget = React.useRef<string | null>(null);

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
    <>
      <Section title="Workspace Roots">
        {roots.length === 0 ? (
          <p className="settings-empty">No roots yet.</p>
        ) : (
          <ul className="settings-list">
            {roots.map((root) => (
              <li key={root} className="settings-list-row">
                <span className="settings-path" title={root}>{root}</span>
                <button
                  type="button"
                  className="button button-secondary button-danger"
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
        )}
        <div className="settings-actions">
          <button type="button" className="button" onClick={() => void handleAddRoot()}>
            Add root…
          </button>
        </div>
      </Section>

      <Section title="Ignore Set" hint="Artifacts matching these globs stay out of the tree.">
        {ignoreGlobs.length === 0 ? (
          <p className="settings-empty">Nothing ignored beyond the built-in set.</p>
        ) : (
          <ul className="settings-list">
            {ignoreGlobs.map((glob) => (
              <li key={glob} className="settings-list-row">
                <span className="settings-path">{glob}</span>
              </li>
            ))}
          </ul>
        )}
        <input
          type="text"
          className="settings-input"
          data-field="ignore-globs"
          aria-label="Add ignore glob"
          value={newGlob}
          onChange={(e) => setNewGlob(e.target.value)}
          onKeyDown={(e) => void handleGlobKeyDown(e)}
          onBlur={() => void handleGlobBlur()}
          placeholder="*.tmp"
        />
      </Section>

      <Section title="Drag-out Preference">
        <label className="settings-choice">
          <input
            type="radio"
            name="drag-out-mode"
            value="file"
            checked={dragOutMode === "file"}
            onChange={() => void handleDragModeChange("file")}
          />
          Drag a file (Finder-friendly)
        </label>
        <label className="settings-choice">
          <input
            type="radio"
            name="drag-out-mode"
            value="url"
            checked={dragOutMode === "url"}
            onChange={() => void handleDragModeChange("url")}
          />
          Drag a URL (Slack-friendly)
        </label>
      </Section>

      <Section
        title="Slack Share Target"
        hint={
          <>
            Slack has no macOS share-sheet extension, so Vlervtifacts opens
            your channel via a deep link instead — drag the file in from
            there. Accepts <code>TEAMID/CHANNELID</code> (e.g.{" "}
            <code>T0123ABCD/C0456EFGH</code>) or a full <code>slack://</code>{" "}
            URL.
          </>
        }
      >
        <input
          type="text"
          className="settings-input"
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
      </Section>
    </>
  );
}

export default function Settings({ ipc = defaultIpc }: SettingsProps): React.ReactElement {
  const { state, setStateField } = useSettings(ipc);
  const { isMacos } = usePlatform();

  if (!state) {
    return <div className="settings-panel" data-testid="settings-panel">Loading…</div>;
  }

  return (
    <div className="settings-panel" data-testid="settings-panel">
      {isMacos ? (
        <DesktopSettings ipc={ipc} state={state} setStateField={setStateField} />
      ) : null}
      <RemoteSettings ipc={ipc} state={state} setStateField={setStateField} />
    </div>
  );
}
