// IosStartPage — the iOS companion's empty-tab surface (PRODUCT.md §
// Operating Context: read-only companion, the phone owns no files). There
// is no workspace to open here, so this replaces StartPage's file/workspace
// actions with the two things that actually matter on iOS: getting paired,
// and reading what has already arrived (Beam + PushArtifact). Reuses the
// same start-page/start-list/start-row classes as the macOS StartPage —
// same visual language, different content.
import * as React from "react";
import { MonitorSmartphone } from "lucide-react";
import { useRemoteState } from "../state/remote";
import { useBeamActions, useBeamState } from "../state/beam";
import { FileGlyph } from "./FileIcon";
import { humanBytes } from "../utils/beam-format";

export interface IosStartPageProps {
  /** Opens the Settings modal, which mounts the Remote pane (pairing UI is
   * reused as-is — see PRODUCT.md). */
  onOpenSettings?: () => void;
}

export default function IosStartPage({ onOpenSettings }: IosStartPageProps): React.ReactElement {
  const { peers, presence } = useRemoteState();
  const { received } = useBeamState();
  const { openReceived, refreshReceived } = useBeamActions();

  React.useEffect(() => {
    refreshReceived();
  }, [refreshReceived]);

  const hasPeers = peers.length > 0;

  return (
    <div className="start-page" data-testid="start-page">
      <div className="start-page-inner">
        <div className="start-brand">
          <span className="start-mark" aria-hidden>V</span>
          <h1 className="start-title">Vlervtifacts</h1>
        </div>

        {!hasPeers ? (
          <>
            <p className="start-subtitle">
              Read what your Mac is showing, from your phone.
            </p>
            <div className="start-actions">
              <button
                type="button"
                className="start-action"
                data-testid="ios-pair-cta"
                onClick={onOpenSettings}
              >
                <MonitorSmartphone size={15} strokeWidth={2} /> Pair with a Mac…
              </button>
            </div>
            <p className="start-empty">
              Pairing is one-time and end-to-end encrypted, direct between
              this phone and your Mac — no account, no server.
            </p>
          </>
        ) : (
          <>
            <section className="start-section">
              <h2>
                <MonitorSmartphone size={13} strokeWidth={2} /> Paired Macs
              </h2>
              <ul className="start-list">
                {peers.map((peer) => {
                  const state = presence[peer.node_id]?.state ?? "offline";
                  return (
                    <li key={peer.node_id}>
                      <div className="start-row" data-testid="ios-peer-row">
                        <span
                          className={`remote-presence-dot remote-presence-${state}`}
                          title={state}
                          aria-hidden
                        />
                        <span className="start-row-name">{peer.device}</span>
                        <span className="start-row-dir">{state}</span>
                      </div>
                    </li>
                  );
                })}
              </ul>
              <button type="button" className="start-action" onClick={onOpenSettings}>
                Manage pairing…
              </button>
            </section>

            {received.length > 0 ? (
              <section className="start-section">
                <h2>Received</h2>
                <ul className="start-list">
                  {received.map((entry) => (
                    <li key={entry.path}>
                      <button
                        type="button"
                        className="start-row"
                        title={entry.path}
                        onClick={() => openReceived(entry.path)}
                      >
                        <span className="start-row-icon">
                          <FileGlyph name={entry.name} size={15} />
                        </span>
                        <span className="start-row-name">{entry.name}</span>
                        <span className="start-row-dir">{humanBytes(entry.size)}</span>
                      </button>
                    </li>
                  ))}
                </ul>
              </section>
            ) : (
              <p className="start-empty">
                Nothing received yet. Beam an artifact from your Mac, or open
                one of its tabs from the Remote drawer.
              </p>
            )}
          </>
        )}
      </div>
    </div>
  );
}
