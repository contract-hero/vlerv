// Renderer router — dispatches by file extension.
import * as React from "react";
import type { FilePayload } from "../ipc";
import HtmlRenderer from "./html";
import MdRenderer from "./md";
import TextRenderer from "./text";
import ImageRenderer from "./image";
import MetadataRenderer from "./metadata";

const TEXT_EXTS = new Set([
  ".txt", ".md", ".markdown", ".ts", ".tsx", ".js", ".jsx", ".json",
  ".move", ".rs", ".toml", ".yml", ".yaml", ".css", ".sh", ".py", ".go",
  ".html", ".xml", ".svg",
]);

const IMAGE_EXTS = new Set([
  ".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg", ".bmp", ".ico", ".avif",
]);

export function extOf(path: string): string {
  const i = path.lastIndexOf(".");
  if (i < 0) return "";
  return path.slice(i).toLowerCase();
}

export function isHtmlPath(path: string): boolean {
  const ext = extOf(path);
  return ext === ".html" || ext === ".htm";
}

export interface RenderOptions {
  /** Fired after async content (markdown) lands in the DOM — used by the
   *  host to restore scroll position. */
  onRendered?: () => void;
}

export function renderByExtension(
  payload: FilePayload,
  opts: RenderOptions = {},
): React.ReactElement {
  const ext = extOf(payload.path);

  // Raster images arrive base64-encoded with is_binary=true — route them to
  // the image renderer before the generic binary fallback.
  if (IMAGE_EXTS.has(ext) && !payload.oversized) {
    return <ImageRenderer payload={payload} />;
  }

  if (payload.oversized || payload.is_binary) {
    return <MetadataRenderer
        path={payload.path}
        size={payload.size}
        mtime={payload.mtime}
        reason={payload.oversized ? "oversize" : "binary"}
      />;
  }

  if (isHtmlPath(payload.path)) {
    return <HtmlRenderer source={payload.content ?? ""} path={payload.path} />;
  }

  if (ext === ".md" || ext === ".markdown") {
    return (
      <MdRenderer
        source={payload.content ?? ""}
        path={payload.path}
        onRendered={opts.onRendered}
      />
    );
  }

  if (TEXT_EXTS.has(ext) || ext === "") {
    return <TextRenderer source={payload.content ?? ""} path={payload.path} />;
  }

  return <MetadataRenderer
        path={payload.path}
        size={payload.size}
        mtime={payload.mtime}
        reason={payload.oversized ? "oversize" : "binary"}
      />;
}
