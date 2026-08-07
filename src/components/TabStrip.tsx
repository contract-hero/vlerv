// TabStrip — Chrome-style tab bar. Click to activate, middle-click or ✕ to
// close, double-click empty space for a new tab, drag to reorder.
import * as React from "react";
import { Plus, X } from "lucide-react";
import { useTabs, useTabsDispatch } from "../state/TabsProvider";
import { currentEntry, isLoading } from "../state/tabs";
import type { Tab } from "../state/tabs";
import { FileGlyph } from "./FileIcon";
import { useContextMenu } from "./ContextMenu";
import { useFileMenu } from "../hooks/useFileMenu";
import type { OpenFileOptions } from "../state/TabsProvider";
import { basename } from "../utils/path";

function tabTitle(tab: Tab): string {
  const entry = currentEntry(tab);
  if (!entry) return "New Tab";
  return basename(entry.path);
}

export interface TabStripProps {
  onOpenFile?: (path: string, opts?: OpenFileOptions) => void;
  /**
   * Reserve the macOS traffic-light gutter before the first tab. On while
   * the sidebar is hidden (⌘B, reader mode) — the strip is then the
   * window's leftmost band and inherits overlay-title-bar duty from the
   * sidebar header.
   */
  showGutter?: boolean;
}

export default function TabStrip({ onOpenFile, showGutter = false }: TabStripProps = {}): React.ReactElement {
  const { tabs, activeTabId } = useTabs();
  const dispatch = useTabsDispatch();
  const [dragId, setDragId] = React.useState<string | null>(null);
  const [overIndex, setOverIndex] = React.useState<number | null>(null);
  const { open: openMenu } = useContextMenu();
  const fileMenu = useFileMenu(onOpenFile);

  const tabRefs = React.useRef<Map<string, HTMLDivElement>>(new Map());

  const endDrag = () => {
    setDragId(null);
    setOverIndex(null);
  };

  /**
   * Arrow-key navigation inside the strip. `role="tablist"` promises this;
   * without it the tabs were announced as a tablist that no keyboard could
   * reach. Activation follows focus, which is the standard behavior for tabs
   * whose panels are already loaded.
   */
  const onStripKeyDown = (e: React.KeyboardEvent) => {
    const index = tabs.findIndex((t) => t.id === activeTabId);
    if (index < 0) return;
    let next: number | null = null;
    if (e.key === "ArrowRight") next = (index + 1) % tabs.length;
    else if (e.key === "ArrowLeft") next = (index - 1 + tabs.length) % tabs.length;
    else if (e.key === "Home") next = 0;
    else if (e.key === "End") next = tabs.length - 1;
    if (next === null) return;
    e.preventDefault();
    const target = tabs[next];
    if (!target) return;
    dispatch({ type: "ACTIVATE_TAB", tabId: target.id });
    tabRefs.current.get(target.id)?.focus();
  };

  const tabMenu = (tab: Tab, index: number) => {
    const entry = currentEntry(tab);
    const tabSection = [
      {
        label: "Close Tab",
        onSelect: () => dispatch({ type: "CLOSE_TAB", tabId: tab.id }),
      },
      {
        label: "Close Other Tabs",
        onSelect: () => {
          for (const other of tabs) {
            if (other.id !== tab.id) dispatch({ type: "CLOSE_TAB", tabId: other.id });
          }
        },
      },
      {
        label: "Close Tabs to the Right",
        onSelect: () => {
          for (const other of tabs.slice(index + 1)) {
            dispatch({ type: "CLOSE_TAB", tabId: other.id });
          }
        },
      },
    ];
    return entry ? [tabSection, ...fileMenu(entry.path)] : [tabSection];
  };

  return (
    <div
      className="tab-strip"
      role="tablist"
      data-tauri-drag-region
      onKeyDown={onStripKeyDown}
      onDoubleClick={(e) => {
        // Only on the empty strip area — not on a tab, and not on the
        // traffic-light gutter, where double-click is the macOS zoom gesture
        // that data-tauri-drag-region already handles.
        if ((e.target as HTMLElement).closest(".tab, .titlebar-gutter")) return;
        dispatch({ type: "OPEN_NEW_TAB" });
      }}
    >
      {showGutter ? <div className="titlebar-gutter" data-tauri-drag-region /> : null}
      {tabs.map((tab, index) => {
        const entry = currentEntry(tab);
        const active = tab.id === activeTabId;
        return (
          <div
            key={tab.id}
            ref={(el) => {
              if (el) tabRefs.current.set(tab.id, el);
              else tabRefs.current.delete(tab.id);
            }}
            role="tab"
            id={`tab-${tab.id}`}
            aria-controls="tab-panel"
            aria-selected={active}
            // Roving tabindex over the tabs: arrows move within. Each
            // .tab-close is still separately tabbable, so the strip is not
            // yet a single tab stop.
            tabIndex={active ? 0 : -1}
            className={[
              "tab",
              active ? "active" : "",
              dragId === tab.id ? "dragging" : "",
              overIndex === index && dragId && dragId !== tab.id ? "drop-target" : "",
            ]
              .filter(Boolean)
              .join(" ")}
            title={entry?.path ?? "New Tab"}
            draggable
            onDragStart={(e) => {
              setDragId(tab.id);
              e.dataTransfer.effectAllowed = "move";
              // Required for the drag to start in WKWebView (macOS).
              e.dataTransfer.setData("text/plain", tab.id);
            }}
            onDragOver={(e) => {
              if (!dragId) return;
              e.preventDefault();
              e.dataTransfer.dropEffect = "move";
              if (overIndex !== index) setOverIndex(index);
            }}
            onDrop={(e) => {
              e.preventDefault();
              if (dragId && dragId !== tab.id) {
                dispatch({ type: "MOVE_TAB", tabId: dragId, toIndex: index });
              }
              endDrag();
            }}
            onDragEnd={endDrag}
            onClick={() => dispatch({ type: "ACTIVATE_TAB", tabId: tab.id })}
            onContextMenu={(e) => openMenu(e, tabMenu(tab, index))}
            onAuxClick={(e) => {
              if (e.button === 1) {
                e.preventDefault();
                dispatch({ type: "CLOSE_TAB", tabId: tab.id });
              }
            }}
          >
            <span className="tab-icon" aria-hidden>
              {entry ? <FileGlyph name={tabTitle(tab)} size={13} /> : <Plus size={13} strokeWidth={2} />}
            </span>
            <span className="tab-label">{tabTitle(tab)}</span>
            {/* Background tabs load lazily on activation — only the active
                tab can genuinely be "loading", so don't show background tabs
                as perpetually busy. */}
            {isLoading(tab) && entry && active ? <span className="tab-loading" aria-hidden /> : null}
            <button
              className="tab-close"
              title="Close tab (⌘W)"
              aria-label={`Close ${tabTitle(tab)}`}
              onClick={(e) => {
                e.stopPropagation();
                dispatch({ type: "CLOSE_TAB", tabId: tab.id });
              }}
            >
              <X size={12} strokeWidth={2} />
            </button>
          </div>
        );
      })}
      <button
        className="tab-new"
        title="New tab (⌘T)"
        aria-label="New tab"
        onClick={() => dispatch({ type: "OPEN_NEW_TAB" })}
      >
        <Plus size={14} strokeWidth={2} />
      </button>
    </div>
  );
}
