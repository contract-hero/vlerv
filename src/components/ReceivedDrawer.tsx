// ReceivedDrawer — sidebar section listing every artifact Beam has landed
// on this instance (accepted receives + control-scope pushes). On macOS
// this stays the toolbar's incidental "beaming" popover (BeamDialog.tsx);
// on iOS it is a primary surface (PRODUCT.md: the phone owns no files, so
// what Beam delivered IS the reading list) — mounted only in the iOS
// Sidebar branch.
import * as React from "react";
import { useBeamActions, useBeamState } from "../state/beam";
import { FileGlyph } from "./FileIcon";
import { humanBytes } from "../utils/beam-format";
import SidebarSection from "./SidebarSection";

export interface ReceivedDrawerProps {
  /** Fires after a row opens — the phone sheet closes itself with this. */
  onOpen?: () => void;
}

export default function ReceivedDrawer({ onOpen }: ReceivedDrawerProps = {}): React.ReactElement {
  const { received, receivedError } = useBeamState();
  const { openReceived, refreshReceived } = useBeamActions();

  React.useEffect(() => {
    refreshReceived();
  }, [refreshReceived]);

  return (
    <SidebarSection id="received" title="Received" count={received.length}>
      {/* A push that lands inside a delivery run opens no tab, so this list
          is the only sign of it. A refresh that failed leaves the list stale
          rather than empty, which is indistinguishable from "nothing
          arrived" unless it says so. */}
      {receivedError ? (
        <p className="beam-error" role="alert">{receivedError}</p>
      ) : null}
      {received.length === 0 ? (
        <p className="section-empty-hint">
          Artifacts beamed or pushed to this device will show up here.
        </p>
      ) : (
        <ul>
          {received.map((entry) => (
            <li
              key={entry.path}
              title={entry.path}
              style={{ cursor: "pointer" }}
              onClick={() => {
                openReceived(entry.path);
                onOpen?.();
              }}
            >
              <span className="section-row-icon" aria-hidden>
                <FileGlyph name={entry.name} size={13} />
              </span>
              <span className="section-row-label">{entry.name}</span>
              <span className="beam-popover-meta">{humanBytes(entry.size)}</span>
            </li>
          ))}
        </ul>
      )}
    </SidebarSection>
  );
}
