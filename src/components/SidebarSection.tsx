// SidebarSection — the one collapsible drawer shape every sidebar group
// uses (Bookmarks / Recent / Files). Header is a real <h4> with a real
// <button> inside (screen-reader rotor + keyboard reachability), collapse
// state persisted per-section to localStorage.
import * as React from "react";
import { ChevronRight } from "lucide-react";

// Key changed from vlerv.bookmarks.collapsed when sections were generalized;
// the old value is intentionally dropped rather than migrated.
function storageKey(id: string): string {
  return `vlerv.section.${id}.collapsed`;
}

function readSavedCollapsed(id: string, fallback: boolean): boolean {
  try {
    const raw = globalThis.localStorage?.getItem(storageKey(id));
    if (raw === null || raw === undefined) return fallback;
    return raw === "1";
  } catch {
    return fallback;
  }
}

function saveCollapsed(id: string, collapsed: boolean): void {
  try {
    globalThis.localStorage?.setItem(storageKey(id), collapsed ? "1" : "0");
  } catch {
    // ignore
  }
}

export interface SidebarSectionProps {
  /** Stable id — the data-section value; forms the storage key `vlerv.section.<id>.collapsed`. */
  id: string;
  title: string;
  /** Row count shown at the header's right edge; omit to hide. */
  count?: number;
  /** Extra header content between the title and the count — the remote
   * drawer's presence dot + Follow toggle live here (design §6: "presence
   * shows in the drawer header"). Absent for every other section. */
  headerExtra?: React.ReactNode;
  /** Collapse state when the user has never toggled this section. */
  defaultCollapsed?: boolean;
  /** Give the section the sidebar's leftover height (the tree). */
  grow?: boolean;
  children: React.ReactNode;
}

export default function SidebarSection({
  id,
  title,
  count,
  headerExtra,
  defaultCollapsed = false,
  grow = false,
  children,
}: SidebarSectionProps): React.ReactElement {
  const [collapsed, setCollapsed] = React.useState<boolean>(() =>
    readSavedCollapsed(id, defaultCollapsed),
  );

  const toggle = () => {
    setCollapsed((prev) => {
      const next = !prev;
      saveCollapsed(id, next);
      return next;
    });
  };

  return (
    <div
      data-section={id}
      data-testid={`${id}-group`}
      className={grow && !collapsed ? "section-grow" : undefined}
    >
      <h4 className="section-header">
        <button
          type="button"
          className="section-header-toggle"
          onClick={toggle}
          aria-expanded={!collapsed}
        >
          <span className={`section-chevron${collapsed ? "" : " open"}`} aria-hidden>
            <ChevronRight size={12} strokeWidth={2} />
          </span>
          <span>{title}</span>
        </button>
        {headerExtra}
        {count !== undefined ? (
          <span className="section-count" aria-hidden>{count}</span>
        ) : null}
      </h4>
      {!collapsed && children}
    </div>
  );
}
