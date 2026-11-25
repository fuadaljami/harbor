import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";

export type TooltipSide = "left" | "right" | "top" | "bottom";

interface TooltipProps {
  label: string;
  badge?: string | number;
  side?: TooltipSide;
  disabled?: boolean;
  children: ReactNode;
}

interface TooltipCoords {
  left: number;
  top: number;
  arrowLeft: number;
  arrowTop: number;
  resolvedSide: TooltipSide;
}

const VIEWPORT_PADDING = 6;
const TRIGGER_GAP = 9;

export function Tooltip({
  label,
  badge,
  side = "top",
  disabled = false,
  children,
}: TooltipProps) {
  const triggerRef = useRef<HTMLDivElement | null>(null);
  const tipRef = useRef<HTMLDivElement | null>(null);
  const [open, setOpen] = useState(false);
  const [coords, setCoords] = useState<TooltipCoords | null>(null);

  const updatePosition = useCallback(() => {
    const triggerEl = triggerRef.current;
    const tipEl = tipRef.current;
    if (!triggerEl || !tipEl) return;

    const tr = triggerEl.getBoundingClientRect();
    const tp = tipEl.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;

    let resolvedSide = side;
    if (side === "right" && tr.right + TRIGGER_GAP + tp.width > vw - VIEWPORT_PADDING) {
      resolvedSide = "left";
    } else if (side === "left" && tr.left - TRIGGER_GAP - tp.width < VIEWPORT_PADDING) {
      resolvedSide = "right";
    } else if (side === "top" && tr.top - TRIGGER_GAP - tp.height < VIEWPORT_PADDING) {
      resolvedSide = "bottom";
    } else if (side === "bottom" && tr.bottom + TRIGGER_GAP + tp.height > vh - VIEWPORT_PADDING) {
      resolvedSide = "top";
    }

    let left = 0;
    let top = 0;
    if (resolvedSide === "right") {
      left = tr.right + TRIGGER_GAP;
      top = tr.top + tr.height / 2 - tp.height / 2;
    } else if (resolvedSide === "left") {
      left = tr.left - TRIGGER_GAP - tp.width;
      top = tr.top + tr.height / 2 - tp.height / 2;
    } else if (resolvedSide === "bottom") {
      left = tr.left + tr.width / 2 - tp.width / 2;
      top = tr.bottom + TRIGGER_GAP;
    } else {
      left = tr.left + tr.width / 2 - tp.width / 2;
      top = tr.top - TRIGGER_GAP - tp.height;
    }

    const clampedLeft = Math.min(
      Math.max(left, VIEWPORT_PADDING),
      Math.max(VIEWPORT_PADDING, vw - tp.width - VIEWPORT_PADDING),
    );
    const clampedTop = Math.min(
      Math.max(top, VIEWPORT_PADDING),
      Math.max(VIEWPORT_PADDING, vh - tp.height - VIEWPORT_PADDING),
    );

    const arrowLeft = Math.min(
      Math.max(tr.left + tr.width / 2 - clampedLeft - 4, 6),
      Math.max(6, tp.width - 14),
    );
    const arrowTop = Math.min(
      Math.max(tr.top + tr.height / 2 - clampedTop - 4, 6),
      Math.max(6, tp.height - 14),
    );

    setCoords({
      left: clampedLeft,
      top: clampedTop,
      arrowLeft,
      arrowTop,
      resolvedSide,
    });
  }, [side]);

  useLayoutEffect(() => {
    if (!open || disabled) return;
    updatePosition();
  }, [open, disabled, label, badge, updatePosition]);

  useEffect(() => {
    if (!open) return;
    const hide = () => setOpen(false);
    window.addEventListener("blur", hide);
    window.addEventListener("scroll", hide, true);
    return () => {
      window.removeEventListener("blur", hide);
      window.removeEventListener("scroll", hide, true);
    };
  }, [open]);

  return (
    <div
      ref={triggerRef}
      className="inline-flex items-center justify-center"
      onMouseEnter={() => {
        if (!disabled) setOpen(true);
      }}
      onMouseLeave={() => setOpen(false)}
      onPointerDown={() => setOpen(false)}
    >
      {children}
      {open &&
        !disabled &&
        createPortal(
          <div
            ref={tipRef}
            role="tooltip"
            style={
              coords
                ? { left: `${coords.left}px`, top: `${coords.top}px` }
                : { left: "-9999px", top: "-9999px" }
            }
            className="fixed z-50 pointer-events-none px-2.5 py-1.5 rounded-md bg-harbor-floating border border-white/[0.08] shadow-lg text-xs font-medium text-harbor-textPrimary flex items-center gap-1.5 whitespace-nowrap"
          >
            <span>{label}</span>
            {badge !== undefined && (
              <span className="px-1.5 py-0.5 rounded bg-harbor-panel text-[10px] font-semibold text-harbor-textSecondary">
                {badge}
              </span>
            )}
            {coords && (
              <span
                style={
                  coords.resolvedSide === "right"
                    ? { left: "-4px", top: `${coords.arrowTop}px` }
                    : coords.resolvedSide === "left"
                      ? { right: "-4px", top: `${coords.arrowTop}px` }
                      : coords.resolvedSide === "bottom"
                        ? { top: "-4px", left: `${coords.arrowLeft}px` }
                        : { bottom: "-4px", left: `${coords.arrowLeft}px` }
                }
                className="absolute w-2 h-2 rotate-45 bg-harbor-floating border-white/[0.08]"
              />
            )}
          </div>,
          document.body,
        )}
    </div>
  );
}
