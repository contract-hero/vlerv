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

function tabTitle(tab: Tab): string {
  const entry = currentEntry(tab);
  if (!entry) return "New Tab";
  return entry.path.replace(/^.*\//, "") || entry.path;
}

export interface TabStripProps {
  onOpenFile?: (path: string, opts?: OpenFileOptions) => void;
}

export default function TabStrip({ onOpenFile }: TabStripProps = {}): React.ReactElement {
  const { tabs, activeTabId } = useTabs();
  const dispatch = useTabsDispatch();
  const [dragId, setDragId] = React.useState<string | null>(null);
  const [overIndex, setOverIndex] = React.useState<number | null>(null);
  const { open: openMenu } = useContextMenu();
  const fileMenu = useFileMenu(onOpenFile);

  const endDrag = () => {
    setDragId(null);
    setOverIndex(null);
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
      onDoubleClick={(e) => {
        // Only on the empty strip area, not on a tab.
        if ((e.target as HTMLElement).closest(".tab")) return;
        dispatch({ type: "OPEN_NEW_TAB" });
      }}
    >
      {tabs.map((tab, index) => {
        const entry = currentEntry(tab);
        const active = tab.id === activeTabId;
        return (
          <div
            key={tab.id}
            role="tab"
            aria-selected={active}
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
            {isLoading(tab) && entry ? <span className="tab-loading" aria-hidden /> : null}
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
