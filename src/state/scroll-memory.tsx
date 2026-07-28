// ScrollMemory — mutable per-(tab, path) scroll positions. Lives OUTSIDE the
// reducer: scroll fires at 60 Hz and nothing renders from it; it's read
// imperatively at restore time.
import * as React from "react";

export interface ScrollPos {
  x: number;
  y: number;
}

export interface ScrollMemory {
  save(key: string, pos: ScrollPos): void;
  get(key: string): ScrollPos | undefined;
}

const noop: ScrollMemory = { save: () => {}, get: () => undefined };
const ScrollMemoryContext = React.createContext<ScrollMemory>(noop);

export function useScrollMemory(): ScrollMemory {
  return React.useContext(ScrollMemoryContext);
}

export function scrollKeyFor(tabId: string, path: string): string {
  return `${tabId}::${path}`;
}

export function ScrollMemoryProvider({ children }: { children: React.ReactNode }): React.ReactElement {
  const mapRef = React.useRef(new Map<string, ScrollPos>());
  const memory = React.useMemo<ScrollMemory>(
    () => ({
      save(key, pos) {
        mapRef.current.set(key, pos);
      },
      get(key) {
        return mapRef.current.get(key);
      },
    }),
    [],
  );
  return <ScrollMemoryContext.Provider value={memory}>{children}</ScrollMemoryContext.Provider>;
}
