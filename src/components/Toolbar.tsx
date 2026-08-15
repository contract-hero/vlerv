// Toolbar — browser-style navigation row above the preview: back/forward/
// reload, an editable address bar showing the active tab's path, bookmark
// star, copy-path, external badge, and zoom controls.
import * as React from "react";
import {
  ArrowLeft,
  ArrowRight,
  BookOpen,
  Check,
  Copy,
  PanelLeft,
  RotateCw,
  Share,
  Slack,
  Star,
  Zap,
} from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useActiveTab, useTabsDispatch } from "../state/TabsProvider";
import { canGoBack, canGoForward, currentEntry } from "../state/tabs";
import { useBookmarksContext } from "../state/bookmarks-context";
import { useWorkspace } from "../state/workspace";
import { useBeamActions } from "../state/beam";
import { BeamIndicator } from "./BeamDialog";
import { displayPath, isUnderRoot, normalizePathBarInput } from "../utils/path";
import { slackUrlFromTarget } from "../utils/slack";
import { useCopyFeedback } from "../hooks/useCopyFeedback";
import { useSettings } from "../hooks/useSettings";
import { tauriIpc } from "../ipc";

export interface ToolbarProps {
  addressBarRef: React.MutableRefObject<HTMLInputElement | null>;
  onSubmitPath: (path: string) => void;
  /**
   * Whether the sidebar is currently shown. Picks the toggle button's title
   * and aria-label only — the button carries no pressed state.
   */
  sidebarVisible: boolean;
  onToggleSidebar: () => void;
  onEnterReaderMode: () => void;
}

function ShareButton({ path }: { path: string }): React.ReactElement {
  const handleClick = (e: React.MouseEvent<HTMLButtonElement>) => {
    e.preventDefault();
    e.stopPropagation();
    // Capture the rect synchronously — it's viewport-relative and goes stale
    // if the pane scrolls between click and native display.
    const r = e.currentTarget.getBoundingClientRect();
    void tauriIpc
      .shareFile?.([path], { x: r.x, y: r.y, width: r.width, height: r.height })
      .catch(() => {
        // Share errors (path vanished, non-macOS) fail silently, matching
        // CopyPathButton's clipboard fallback.
      });
  };
  return (
    <button
      type="button"
      className="toolbar-button"
      data-testid="preview-share"
      title="Share…"
      aria-label="Share file"
      onClick={handleClick}
    >
      <Share size={13} strokeWidth={2} />
    </button>
  );
}

/**
 * Slack ships no macOS share extension, so the share sheet can't reach it.
 * When a Slack target is configured this button foregrounds Slack on that
 * channel/DM; the file itself travels by drag-out from the tree.
 */
function OpenInSlackButton({ url }: { url: string }): React.ReactElement {
  return (
    <button
      type="button"
      className="toolbar-button"
      data-testid="preview-open-in-slack"
      title="Open Slack channel (drag the file in to share)"
      aria-label="Open Slack channel"
      onClick={(e) => {
        e.preventDefault();
        e.stopPropagation();
        void openUrl(url).catch(() => {
          // Opener scope or missing Slack app — fail silently.
        });
      }}
    >
      <Slack size={13} strokeWidth={2} />
    </button>
  );
}

/** Beam v1: stage this file and mint a shareable `vlerv://receive` link. */
function BeamButton({ path }: { path: string }): React.ReactElement {
  const { beginSend } = useBeamActions();
  return (
    <button
      type="button"
      className="toolbar-button"
      data-testid="preview-beam"
      title="Beam to Vlervcode…"
      aria-label="Beam this file to another Vlervcode"
      onClick={() => beginSend(path)}
    >
      <Zap size={13} strokeWidth={2} />
    </button>
  );
}

function IconButton({
  title,
  disabled,
  onClick,
  children,
}: {
  title: string;
  disabled?: boolean;
  onClick: () => void;
  children: React.ReactNode;
}): React.ReactElement {
  return (
    <button
      type="button"
      className="toolbar-button"
      title={title}
      aria-label={title}
      disabled={disabled}
      onClick={onClick}
    >
      {children}
    </button>
  );
}

function CopyPathButton({ path }: { path: string }): React.ReactElement {
  const { copied, copy } = useCopyFeedback();
  return (
    <button
      type="button"
      className="toolbar-button"
      data-testid="preview-copy-path"
      title={copied ? "Copied!" : "Copy path"}
      aria-label="Copy file path"
      onClick={() => copy(path)}
    >
      {copied ? <Check size={13} strokeWidth={2.5} /> : <Copy size={13} strokeWidth={2} />}
    </button>
  );
}

export default function Toolbar({
  addressBarRef,
  onSubmitPath,
  sidebarVisible,
  onToggleSidebar,
  onEnterReaderMode,
}: ToolbarProps): React.ReactElement {
  const tab = useActiveTab();
  const dispatch = useTabsDispatch();
  const { root } = useWorkspace();
  const { receivedDir } = useBeamActions();
  const { isBookmarked, toggle } = useBookmarksContext();
  // One settings subscription for the toolbar; the Slack affordance stays
  // hidden until a target is configured.
  const { state: settings } = useSettings(tauriIpc);
  const slackTarget = settings?.preferences?.slack_target;
  const slackUrl = slackTarget ? slackUrlFromTarget(slackTarget) : null;

  const entry = currentEntry(tab);
  const currentPath = entry?.path ?? "";

  // The input is controlled locally; it resyncs to the tab's path whenever
  // the tab or its current entry changes, and tracks user edits in between.
  const [draft, setDraft] = React.useState(currentPath);
  const [editing, setEditing] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (!editing) {
      setDraft(currentPath);
      setError(null);
    }
  }, [currentPath, tab.id, editing]);

  const submit = () => {
    const result = normalizePathBarInput(draft);
    if (result.kind === "error") {
      setError(result.reason);
      return;
    }
    setError(null);
    setEditing(false);
    onSubmitPath(result.path);
    addressBarRef.current?.blur();
  };

  const bookmarked = entry ? isBookmarked(entry.path) : false;
  const zoomPct = Math.round(tab.zoom * 100);

  return (
    <div className="toolbar">
      <IconButton
        title={sidebarVisible ? "Hide sidebar (⌘B)" : "Show sidebar (⌘B)"}
        onClick={onToggleSidebar}
      >
        <PanelLeft size={14.5} strokeWidth={2} />
      </IconButton>

      <span className="toolbar-sep" aria-hidden />

      <IconButton
        title="Back (⌘[)"
        disabled={!canGoBack(tab)}
        onClick={() => dispatch({ type: "GO_BACK" })}
      >
        <ArrowLeft size={15} strokeWidth={2} />
      </IconButton>
      <IconButton
        title="Forward (⌘])"
        disabled={!canGoForward(tab)}
        onClick={() => dispatch({ type: "GO_FORWARD" })}
      >
        <ArrowRight size={15} strokeWidth={2} />
      </IconButton>
      <IconButton
        title="Reload this file (⌘R)"
        disabled={!entry}
        onClick={() => dispatch({ type: "RELOAD" })}
      >
        <RotateCw size={13.5} strokeWidth={2} />
      </IconButton>

      <span className="toolbar-sep" aria-hidden />

      <div className="address-bar" data-error={error ? "true" : undefined}>
        <input
          ref={addressBarRef}
          type="text"
          spellCheck={false}
          autoCapitalize="off"
          autoCorrect="off"
          placeholder="Enter a file path, file:// or vlerv:// URL  (⌘L)"
          // Idle, the bar shows the workspace-relative path — the absolute
          // prefix is the same on every row and says nothing. Focus swaps in
          // the full path (draft) for editing and selects it, so the swap is
          // never mistaken for a caret jump.
          value={editing ? draft : displayPath(currentPath, root)}
          data-testid="path-bar-input"
          onFocus={(e) => {
            setEditing(true);
            // Select after the value swaps from relative to absolute (next
            // commit), not the soon-to-be-replaced display text.
            const input = e.currentTarget;
            window.setTimeout(() => input.select(), 0);
          }}
          onBlur={() => setEditing(false)}
          onChange={(e) => {
            setDraft(e.target.value);
            if (error) setError(null);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              submit();
            } else if (e.key === "Escape") {
              e.preventDefault();
              setDraft(currentPath);
              setError(null);
              (e.target as HTMLInputElement).blur();
            }
          }}
        />
        {error ? (
          <span className="address-bar-error" role="alert">
            {error}
          </span>
        ) : null}
        {entry?.external ? (
          // One badge span; beamed files are technically external (they
          // live in the app's state dir), but their provenance is the
          // interesting fact, so it wins the label.
          (() => {
            const beamed = receivedDir != null && isUnderRoot(entry.path, receivedDir);
            return (
              <span
                className="preview-badge-external"
                data-testid={beamed ? "preview-beamed-badge" : "preview-external-badge"}
                title={
                  beamed
                    ? "Received via Beam from another Vlervcode"
                    : "This file is outside the workspace root"
                }
              >
                {beamed ? "beamed" : "external"}
              </span>
            );
          })()
        ) : null}
        {/* The zoom chip lives INSIDE the address bar so that showing it
            shrinks the input instead of pushing the whole action cluster
            left every time you press ⌘+. */}
        {zoomPct !== 100 ? (
          <button
            type="button"
            className="toolbar-zoom"
            title="Reset zoom (⌘0)"
            aria-label={`Zoom ${zoomPct} percent. Reset zoom`}
            onClick={() => dispatch({ type: "SET_ZOOM", tabId: tab.id, zoom: 1 })}
          >
            {zoomPct}%
          </button>
        ) : null}
      </div>

      {entry ? (
        <>
          <button
            type="button"
            className={`toolbar-button${bookmarked ? " star-active" : ""}`}
            data-testid="preview-bookmark-toggle"
            title={bookmarked ? "Remove bookmark" : "Bookmark this file"}
            aria-label={bookmarked ? "Remove bookmark" : "Bookmark this file"}
            aria-pressed={bookmarked}
            onClick={() => void toggle(entry.path)}
          >
            <Star size={13.5} strokeWidth={2} fill={bookmarked ? "currentColor" : "none"} />
          </button>
          {/* Keep (star) and hand over (copy / share / Slack) are different
              jobs; the rule between them is the grouping cue. */}
          <span className="toolbar-sep" aria-hidden />
          <CopyPathButton path={entry.path} />
          <ShareButton path={entry.path} />
          <BeamButton path={entry.path} />
          {slackUrl ? <OpenInSlackButton url={slackUrl} /> : null}
        </>
      ) : null}

      <BeamIndicator />

      <span className="toolbar-sep" aria-hidden />

      <IconButton
        title="Reader mode (⇧⌘F — Esc to leave)"
        onClick={onEnterReaderMode}
      >
        <BookOpen size={14} strokeWidth={2} />
      </IconButton>
    </div>
  );
}
