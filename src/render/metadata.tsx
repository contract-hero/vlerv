// Metadata-only card — used for oversize and binary payloads. R4.3, R4.4.
// C3 T-040 fix: mtime rendered via Intl.DateTimeFormat medium+short.
import * as React from "react";

export interface MetadataRendererProps {
  path: string;
  size: number;
  mtime: number;
  reason: "binary" | "oversize";
}

function formatBytes(bytes: number): string {
  if (bytes >= 1024 * 1024) {
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }
  if (bytes >= 1024) {
    return `${(bytes / 1024).toFixed(1)} KB`;
  }
  return `${bytes} B`;
}

/// T-040: human-readable mtime via Intl.DateTimeFormat medium+short.
/// mtime is Unix seconds (integer).
function formatMtime(mtime: number): string {
  try {
    return new Intl.DateTimeFormat(undefined, {
      dateStyle: "medium",
      timeStyle: "short",
    }).format(new Date(mtime * 1000));
  } catch {
    return String(mtime);
  }
}

export default function MetadataRenderer({
  path,
  size,
  mtime,
  reason,
}: MetadataRendererProps): React.ReactElement {
  return (
    <div data-testid="metadata-renderer" className="metadata-renderer">
      <div data-field="path" className="metadata-path">
        {path}
      </div>
      <div data-field="size">Size: {formatBytes(size)}</div>
      <div data-field="mtime">Modified: {formatMtime(mtime)}</div>
      <div data-field="reason" className="metadata-reason">
        {reason === "binary" ? "binary file" : "oversize file"}
      </div>
    </div>
  );
}
