import { useCallback, useRef } from "react";
import { ChevronLeft, ChevronRight, GripHorizontal } from "lucide-react";
import { ipc } from "../lib/ipc";
import type { DockEdge } from "../types/dock";

interface EdgeHandleProps {
  expanded: boolean;
  edge: DockEdge;
  onExpand: () => void;
  onCollapse: () => void;
  onError: (err: unknown) => void;
}

export function EdgeHandle({
  expanded,
  edge,
  onExpand,
  onCollapse,
  onError,
}: EdgeHandleProps) {
  const dragActiveRef = useRef<boolean>(false);
  const startClientYRef = useRef<number>(0);

  const finishDrag = useCallback(
    async (allowExpand: boolean, endClientY?: number) => {
      if (!dragActiveRef.current) {
        return;
      }
      dragActiveRef.current = false;
      const startY = startClientYRef.current;

      try {
        const res = await ipc.endHandleDrag();
        const delta =
          endClientY !== undefined ? Math.abs(endClientY - startY) : 999;
        if (allowExpand && !res.moved && delta < 5) {
          onExpand();
        }
      } catch (err) {
        onError(err);
      }
    },
    [onExpand, onError],
  );

  const handlePointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (expanded || e.button !== 0) {
      return;
    }
    e.currentTarget.setPointerCapture(e.pointerId);
    dragActiveRef.current = true;
    startClientYRef.current = e.clientY;
    void ipc.startHandleDrag().catch((err) => {
      dragActiveRef.current = false;
      onError(err);
    });
  };

  const handlePointerUp = (e: React.PointerEvent<HTMLDivElement>) => {
    if (expanded) {
      onCollapse();
      return;
    }
    if (e.currentTarget.hasPointerCapture(e.pointerId)) {
      e.currentTarget.releasePointerCapture(e.pointerId);
    }
    void finishDrag(true, e.clientY);
  };

  const handlePointerCancel = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.currentTarget.hasPointerCapture(e.pointerId)) {
      e.currentTarget.releasePointerCapture(e.pointerId);
    }
    void finishDrag(false);
  };

  const handleLostPointerCapture = () => {
    void finishDrag(false);
  };

  const handleRadiusClass =
    edge === "Left" ? "rounded-r-lg border-l-0" : "rounded-l-lg border-r-0";

  return (
    <div
      role="button"
      tabIndex={0}
      aria-label={expanded ? "Collapse Harbor dock" : "Expand Harbor dock"}
      onPointerDown={handlePointerDown}
      onPointerUp={handlePointerUp}
      onPointerCancel={handlePointerCancel}
      onLostPointerCapture={handleLostPointerCapture}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          if (expanded) {
            onCollapse();
          } else {
            onExpand();
          }
        }
      }}
      className={`w-[24px] h-[84px] bg-harbor-rail border border-harbor-handleBorder ${handleRadiusClass} flex flex-col items-center justify-center gap-1 select-none shadow-lg transition-colors duration-150 hover:bg-harbor-hover ${
        expanded ? "cursor-pointer" : "cursor-ns-resize"
      }`}
      title={
        expanded
          ? "Collapse Harbor"
          : "Click to expand or drag vertically to reposition"
      }
    >
      {!expanded && (
        <GripHorizontal
          className="w-3 h-3 text-harbor-textMuted/70"
          strokeWidth={1.75}
        />
      )}
      {edge === "Left" ? (
        <ChevronRight
          className={`w-4 h-4 text-harbor-blurple transition-transform duration-150 ease-discord ${
            expanded ? "rotate-180" : "rotate-0"
          }`}
          strokeWidth={2}
        />
      ) : (
        <ChevronLeft
          className={`w-4 h-4 text-harbor-blurple transition-transform duration-150 ease-discord ${
            expanded ? "rotate-180" : "rotate-0"
          }`}
          strokeWidth={2}
        />
      )}
    </div>
  );
}
