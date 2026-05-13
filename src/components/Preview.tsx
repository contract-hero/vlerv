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
}

export default function Preview({ payload }: PreviewProps): React.ReactElement {
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
        <header>{path}</header>
        <p><strong>{kind}</strong></p>
        <p>{reason}</p>
      </div>
    );
  }

  return (
    <div data-testid="preview-content">
      <header>{payload.path}</header>
      {renderByExtension(payload)}
    </div>
  );
}
