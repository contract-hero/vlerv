// SidebarResizer — pointer-event drag handle on the right edge of the
// sidebar. Updates the parent's width state live during drag; the parent is
// responsible for clamping (min/max) and for persisting the final value on
// pointerup (so we don't spam the state_store debounced writer).
import * as React from "react";

export interface SidebarResizerProps {
  width: number;
  onResize: (newWidth: number) => void;
  onCommit?: (finalWidth: number) => void;
  /** Clamp values (default 200/480 — must match `.pane-sidebar` CSS). */
  min?: number;
  max?: number;
}

export default function SidebarResizer({
  width,
  onResize,
  onCommit,
  min = 200,
  max = 480,
}: SidebarResizerProps): React.ReactElement {
  const [dragging, setDragging] = React.useState(false);
  const dragOriginX = React.useRef(0);
  const dragOriginWidth = React.useRef(width);
  // Stash the latest width so onCommit fires with the final value (state read
  // inside pointerup would lag because of how the closure captures).
  const latestWidthRef = React.useRef(width);
  latestWidthRef.current = width;

  const onPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    e.preventDefault();
    (e.target as HTMLDivElement).setPointerCapture(e.pointerId);
    dragOriginX.current = e.clientX;
    dragOriginWidth.current = width;
    setDragging(true);
  };

  const onPointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!dragging) return;
    const delta = e.clientX - dragOriginX.current;
    const next = Math.max(min, Math.min(max, dragOriginWidth.current + delta));
    onResize(next);
  };

  const onPointerUp = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!dragging) return;
    (e.target as HTMLDivElement).releasePointerCapture(e.pointerId);
    setDragging(false);
    onCommit?.(latestWidthRef.current);
  };

  return (
    <div
      className={`sidebar-resizer${dragging ? " dragging" : ""}`}
      data-testid="sidebar-resizer"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
      role="separator"
      aria-orientation="vertical"
    />
  );
}
