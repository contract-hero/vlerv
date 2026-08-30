// SettingsModal — the host for Settings.tsx. One panel, two hosts, because
// a centered desktop card is the wrong object on a phone: it lands mid-screen,
// away from the thumb, and its 13px close affordance is a 24px target.
//
//  - macOS: a centered dialog over a scrim. Entry point is the gear in the
//    sidebar header (Sidebar.tsx) — consistent with the toolbar/sidebar-header
//    pattern already used for Refresh / Open file / Change workspace.
//  - iOS: literally the same `PhoneSheet` the Library and the tab list mount,
//    scrim included, so Settings arrives from the bar like every other phone
//    surface. It is the tall variant: Settings scrolls, the tab list does not.
import * as React from "react";
import { X } from "lucide-react";
import type { IpcSurface } from "../ipc";
import { useEscape } from "../hooks/useEscape";
import { usePlatform } from "../state/platform";
import PhoneSheet from "./PhoneSheet";
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
      <PhoneSheet label="Settings" title="Settings" tall onClose={onClose}>
        <Settings ipc={ipc} />
      </PhoneSheet>
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
