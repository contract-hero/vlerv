// SettingsModal — the modal host for Settings.tsx (STATUS.md open item: the
// panel existed but was never mounted). A gear button in the sidebar header
// is the entry point (see Sidebar.tsx) — least invasive relative to the
// existing chrome, consistent with the toolbar/sidebar-header pattern
// already used for Refresh / Open file / Change workspace.
import * as React from "react";
import { X } from "lucide-react";
import type { IpcSurface } from "../ipc";
import { useEscape } from "../hooks/useEscape";
import Settings from "./Settings";

export default function SettingsModal({
  ipc,
  onClose,
}: {
  ipc: IpcSurface;
  onClose: () => void;
}): React.ReactElement {
  useEscape(onClose);

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
        <div className="beam-dialog-body">
          <Settings ipc={ipc} />
        </div>
      </div>
    </div>
  );
}
