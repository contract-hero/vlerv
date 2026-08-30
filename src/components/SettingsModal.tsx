// SettingsModal — the host for Settings.tsx. One panel, two hosts, because
// a centered desktop card is the wrong object on a phone: it lands mid-screen,
// away from the thumb, and its 13px close affordance is a 24px target.
//
//  - macOS: a centered dialog over a scrim. Entry point is the gear in the
//    sidebar header (Sidebar.tsx) — consistent with the toolbar/sidebar-header
//    pattern already used for Refresh / Open file / Change workspace.
//  - iOS: the same bottom sheet the Library and the tab list use (PhoneShell),
//    scrim included, so Settings arrives from the bar like every other phone
//    surface. It is the tall variant: Settings scrolls, the tab list does not.
import * as React from "react";
import { X } from "lucide-react";
import type { IpcSurface } from "../ipc";
import { useEscape } from "../hooks/useEscape";
import { usePlatform } from "../state/platform";
import Settings from "./Settings";

export default function SettingsModal({
  ipc,
  onClose,
}: {
  ipc: IpcSurface;
  onClose: () => void;
}): React.ReactElement {
  const { isIos } = usePlatform();
  useEscape(onClose);

  if (isIos) {
    return (
      <>
        <button
          type="button"
          className="phone-scrim"
          aria-label="Dismiss"
          data-testid="settings-backdrop"
          onClick={onClose}
        />
        <div className="phone-sheet phone-sheet-tall" role="dialog" aria-label="Settings">
          <div className="phone-sheet-grab" aria-hidden />
          <div className="phone-sheet-head">
            <span className="phone-sheet-title">Settings</span>
            <button
              type="button"
              className="phone-sheet-action"
              aria-label="Close"
              onClick={onClose}
            >
              <X size={17} strokeWidth={1.8} />
            </button>
          </div>
          <div className="phone-sheet-body">
            <Settings ipc={ipc} />
          </div>
        </div>
      </>
    );
  }

  return (
    <div
      className="settings-backdrop"
      data-testid="settings-backdrop"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="settings-dialog" role="dialog" aria-label="Settings">
        <div className="beam-dialog-header">
          <span>Settings</span>
          <button
            type="button"
            className="beam-dialog-close"
            title="Close"
            aria-label="Close"
            onClick={onClose}
          >
            <X size={13} strokeWidth={2} />
          </button>
        </div>
        <Settings ipc={ipc} />
      </div>
    </div>
  );
}
