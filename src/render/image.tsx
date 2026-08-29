// Image renderer — raster via data URI; SVG inline (sanitized).
import * as React from "react";
import type { FilePayload } from "../ipc";
import MetadataRenderer from "./metadata";
import { sanitizeTree } from "./sanitize";

export interface ImageRendererProps {
  payload: FilePayload;
}

function extOf(path: string): string {
  const i = path.lastIndexOf(".");
  if (i < 0) return "";
  return path.slice(i).toLowerCase();
}

const MIME: Record<string, string> = {
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".jpeg": "image/jpeg",
  ".gif": "image/gif",
  ".webp": "image/webp",
  ".bmp": "image/bmp",
  ".ico": "image/x-icon",
  ".avif": "image/avif",
};

export default function ImageRenderer({ payload }: ImageRendererProps): React.ReactElement {
  const ext = extOf(payload.path);

  if (ext === ".svg") {
    return <SvgInline source={payload.content ?? ""} />;
  }

  const mime = MIME[ext];
  if (mime && payload.content) {
    const dataUri = `data:${mime};base64,${payload.content}`;
    return (
      <div className="image-renderer">
        <img src={dataUri} alt={payload.path} />
      </div>
    );
  }

  return (
    <MetadataRenderer
      path={payload.path}
      size={payload.size}
      mtime={payload.mtime}
      reason={payload.oversized ? "oversize" : "binary"}
    />
  );
}

function SvgInline({ source }: { source: string }): React.ReactElement {
  const ref = React.useRef<HTMLDivElement>(null);
  React.useEffect(() => {
    const el = ref.current;
    if (!el) return;
    while (el.firstChild) el.removeChild(el.firstChild);
    try {
      const doc = new DOMParser().parseFromString(source, "image/svg+xml");
      const svg = doc.documentElement;
      // An SVG imported here goes into the PRIVILEGED host DOM, where
      // `window.__TAURI_INTERNALS__.invoke` is reachable. Dropping <script>
      // alone left `<svg onload=…>` and `javascript:` hrefs live, and the
      // author of this file is not always the local user — a Scope peer's
      // artifact and a beamed file both reach this renderer. Sanitize every
      // SVG, whatever its provenance: a drawing loses nothing by it.
      sanitizeTree(svg);
      el.appendChild(document.importNode(svg, true));
    } catch {
      // Source unparseable — show empty container.
    }
  }, [source]);
  return <div ref={ref} data-svg-inline="true" className="image-renderer" />;
}
