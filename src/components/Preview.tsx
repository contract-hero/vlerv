// Preview — dispatches to the right renderer for the selected file.
import * as React from "react";
import type { FilePayload } from "../ipc";
import { renderByExtension } from "../render/router";

type ErrorPayload = {
  error: { kind: string; path: string; reason: string };
};

function isErrorPayload(p: unknown): p is ErrorPayload {
  return !!p && typeof p === "object" && "error" in (p as Record<string, unknown>);
}

export interface PreviewProps {
  payload: FilePayload | ErrorPayload | null;
  /**
   * True when the displayed file lies outside the workspace root — either a
   * user-picked ad-hoc file or an out-of-root vlerv:// deep link. Renders a
   * small "external file" badge to signal that the explorer tree won't
   * navigate to this path.
   */
  externalFile?: boolean;
}

function ExternalBadge(): React.ReactElement {
  return (
    <span
      data-testid="preview-external-badge"
      title="This file is outside the workspace root"
      style={{
        marginLeft: "8px",
        padding: "2px 6px",
        fontSize: "0.75em",
        borderRadius: "4px",
        background: "#3b3b3b",
        color: "#f0c674",
        verticalAlign: "middle",
      }}
    >
      external file
    </span>
  );
}

export default function Preview({ payload, externalFile = false }: PreviewProps): React.ReactElement {
  if (payload === null) {
    return (
      <div style={{ padding: "16px", color: "#999" }}>
        No file selected
      </div>
    );
  }

  if (isErrorPayload(payload)) {
    const { kind, path, reason } = payload.error;
    return (
      <div role="alert" data-testid="preview-error" style={{ padding: "16px" }}>
        <header>
          {path}
          {externalFile ? <ExternalBadge /> : null}
        </header>
        <p><strong>{kind}</strong></p>
        <p>{reason}</p>
      </div>
    );
  }

  return (
    <div data-testid="preview-content">
      <header>
        {payload.path}
        {externalFile ? <ExternalBadge /> : null}
      </header>
      {renderByExtension(payload)}
    </div>
  );
}
