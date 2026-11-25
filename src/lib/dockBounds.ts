import { createContext, useCallback, useContext } from "react";

export interface DockHorizontalBounds {
  minX: number;
  maxX: number;
}

const DockBoundsContext = createContext<(() => DockHorizontalBounds) | null>(
  null,
);

export const DockBoundsProvider = DockBoundsContext.Provider;

export function useDockHorizontalBounds(): () => DockHorizontalBounds {
  const getter = useContext(DockBoundsContext);
  return useCallback(() => {
    if (getter) {
      return getter();
    }
    return { minX: 0, maxX: window.innerWidth };
  }, [getter]);
}
