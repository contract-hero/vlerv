// Preview — dispatches to the right renderer for the selected file.
import * as React from "react";
import { Check, Copy, Star } from "lucide-react";
import type { FilePayload } from "../ipc";
import { renderByExtension } from "../render/router";
import { useBookmarks } from "../hooks/useBookmarks";
import { tauriIpc } from "../ipc";

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
      className="preview-badge-external"
      data-testid="preview-external-badge"
      title="This file is outside the workspace root"
    >
      external file
    </span>
  );
}

function BookmarkToggleButton({ path }: { path: string }): React.ReactElement {
  const { isBookmarked, toggle } = useBookmarks(tauriIpc);
  const bookmarked = isBookmarked(path);
  return (
    <button
      type="button"
      className={`preview-bookmark${bookmarked ? " bookmarked" : ""}`}
      data-testid="preview-bookmark-toggle"
      title={bookmarked ? "Remove bookmark" : "Bookmark this file"}
      aria-label={bookmarked ? "Remove bookmark" : "Bookmark this file"}
      aria-pressed={bookmarked}
      onClick={(e) => {
        e.preventDefault();
        e.stopPropagation();
        void toggle(path);
      }}
    >
      <Star size={12} strokeWidth={2} fill={bookmarked ? "currentColor" : "none"} />
    </button>
  );
}

function CopyPathButton({ path }: { path: string }): React.ReactElement {
  const [copied, setCopied] = React.useState(false);
  // Hold a ref to the reset timer so rapid clicks don't leave the "copied!"
  // state stuck after the latest click's window expires.
  const resetTimer = React.useRef<number | null>(null);

  const handleClick = async (e: React.MouseEvent<HTMLButtonElement>) => {
    e.preventDefault();
    e.stopPropagation();
    try {
      await navigator.clipboard.writeText(path);
      setCopied(true);
      if (resetTimer.current) window.clearTimeout(resetTimer.current);
      resetTimer.current = window.setTimeout(() => setCopied(false), 1200);
    } catch {
      // Clipboard API unavailable (rare in Tauri webview) — fail silently.
    }
  };

  React.useEffect(() => {
    return () => {
      if (resetTimer.current) window.clearTimeout(resetTimer.current);
    };
  }, []);

  return (
    <button
      type="button"
      className="preview-copy-path"
      data-testid="preview-copy-path"
      title={copied ? "Copied!" : "Copy path"}
      aria-label="Copy file path"
      onClick={(e) => void handleClick(e)}
    >
      {copied ? <Check size={12} strokeWidth={2.5} /> : <Copy size={12} strokeWidth={2} />}
    </button>
  );
}

export default function Preview({ payload, externalFile = false }: PreviewProps): React.ReactElement {
  if (payload === null) {
    return (
      <div className="preview-empty">
        No file selected
      </div>
    );
  }

  if (isErrorPayload(payload)) {
    const { kind, path, reason } = payload.error;
    return (
      <div role="alert" data-testid="preview-error" style={{ padding: "16px" }}>
        <header>
          <span className="preview-path">{path}</span>
          <BookmarkToggleButton path={path} />
          <CopyPathButton path={path} />
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
        <span className="preview-path">{payload.path}</span>
        <BookmarkToggleButton path={payload.path} />
        <CopyPathButton path={payload.path} />
        {externalFile ? <ExternalBadge /> : null}
      </header>
      {renderByExtension(payload)}
    </div>
  );
}
