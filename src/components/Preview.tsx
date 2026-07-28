// Preview — renders a loaded file payload. The header chrome (path, star,
// copy, badge) lives in the Toolbar now; this component owns the scroll
// container, per-tab zoom, and scroll-position memory for non-iframe content.
import * as React from "react";
import type { FilePayload } from "../ipc";
import { renderByExtension, isHtmlPath } from "../render/router";
import HtmlRenderer from "../render/html";
import { scrollKeyFor, useScrollMemory } from "../state/scroll-memory";

export interface PreviewProps {
  payload: FilePayload;
  tabId: string;
  zoom: number;
}

export default function Preview({ payload, tabId, zoom }: PreviewProps): React.ReactElement {
  const memory = useScrollMemory();
  const scrollRef = React.useRef<HTMLDivElement | null>(null);
  const rafRef = React.useRef<number | null>(null);
  const scrollKey = scrollKeyFor(tabId, payload.path);

  // Restore scroll after the content for this (tab, path) renders. Markdown
  // fills its container asynchronously — MdRenderer re-signals via
  // onRendered, which re-runs the restore.
  const restore = React.useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    const pos = memory.get(scrollKey);
    if (pos) {
      el.scrollLeft = pos.x;
      el.scrollTop = pos.y;
    }
  }, [memory, scrollKey]);

  React.useLayoutEffect(() => {
    restore();
  }, [restore, payload]);

  React.useEffect(() => {
    return () => {
      if (rafRef.current) cancelAnimationFrame(rafRef.current);
    };
  }, []);

  if (isHtmlPath(payload.path) && !payload.is_binary && !payload.oversized) {
    // The iframe manages its own scrolling; scroll memory rides the
    // vlerv:scroll / vlerv:restoreScroll postMessage protocol.
    return (
      <HtmlRenderer
        source={payload.content ?? ""}
        path={payload.path}
        zoom={zoom}
        scrollKey={scrollKey}
      />
    );
  }

  const onScroll = () => {
    if (rafRef.current) return;
    rafRef.current = requestAnimationFrame(() => {
      rafRef.current = null;
      const el = scrollRef.current;
      if (el) memory.save(scrollKey, { x: el.scrollLeft, y: el.scrollTop });
    });
  };

  return (
    <div
      ref={scrollRef}
      className="preview-scroll"
      data-testid="preview-content"
      onScroll={onScroll}
    >
      <div className="preview-zoom" style={zoom !== 1 ? { zoom } : undefined}>
        {renderByExtension(payload, { onRendered: restore })}
      </div>
    </div>
  );
}
