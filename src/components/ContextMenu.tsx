// Lightweight context-menu system. No dependency: a provider renders a
// fixed-position menu portal; `useContextMenu()` returns an `open(event,
// items)` you call from onContextMenu handlers.
import * as React from "react";

export interface MenuItem {
  label: string;
  icon?: React.ReactNode;
  danger?: boolean;
  onSelect: () => void;
}

export type MenuSection = MenuItem[];

interface MenuState {
  x: number;
  y: number;
  sections: MenuSection[];
}

interface ContextMenuApi {
  open: (e: { clientX: number; clientY: number; preventDefault(): void }, sections: MenuSection[]) => void;
}

const ContextMenuContext = React.createContext<ContextMenuApi>({ open: () => {} });

export function useContextMenu(): ContextMenuApi {
  return React.useContext(ContextMenuContext);
}

export function ContextMenuProvider({ children }: { children: React.ReactNode }): React.ReactElement {
  const [menu, setMenu] = React.useState<MenuState | null>(null);
  const menuRef = React.useRef<HTMLDivElement | null>(null);

  const api = React.useMemo<ContextMenuApi>(
    () => ({
      open(e, sections) {
        e.preventDefault();
        if (sections.every((s) => s.length === 0)) return;
        setMenu({ x: e.clientX, y: e.clientY, sections });
      },
    }),
    [],
  );

  // Close on any outside interaction / Escape / window blur.
  React.useEffect(() => {
    if (!menu) return;
    const close = () => setMenu(null);
    const onMouseDown = (e: MouseEvent) => {
      if (!menuRef.current?.contains(e.target as Node)) close();
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
    };
    window.addEventListener("mousedown", onMouseDown, true);
    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("blur", close);
    window.addEventListener("resize", close);
    return () => {
      window.removeEventListener("mousedown", onMouseDown, true);
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("blur", close);
      window.removeEventListener("resize", close);
    };
  }, [menu]);

  // Clamp into the viewport after first paint.
  React.useLayoutEffect(() => {
    const el = menuRef.current;
    if (!el || !menu) return;
    const rect = el.getBoundingClientRect();
    const x = Math.min(menu.x, window.innerWidth - rect.width - 8);
    const y = Math.min(menu.y, window.innerHeight - rect.height - 8);
    if (x !== menu.x || y !== menu.y) {
      el.style.left = `${Math.max(4, x)}px`;
      el.style.top = `${Math.max(4, y)}px`;
    }
  }, [menu]);

  return (
    <ContextMenuContext.Provider value={api}>
      {children}
      {menu ? (
        <div
          ref={menuRef}
          className="context-menu"
          role="menu"
          style={{ left: menu.x, top: menu.y }}
        >
          {menu.sections.map((section, si) => (
            <React.Fragment key={si}>
              {si > 0 && section.length > 0 ? <div className="context-menu-sep" /> : null}
              {section.map((item) => (
                <button
                  key={item.label}
                  type="button"
                  role="menuitem"
                  className={`context-menu-item${item.danger ? " danger" : ""}`}
                  onClick={() => {
                    setMenu(null);
                    item.onSelect();
                  }}
                >
                  {item.icon ? <span className="context-menu-icon">{item.icon}</span> : null}
                  {item.label}
                </button>
              ))}
            </React.Fragment>
          ))}
        </div>
      ) : null}
    </ContextMenuContext.Provider>
  );
}
