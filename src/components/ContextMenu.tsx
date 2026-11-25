import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import {
  ArrowDown,
  ArrowUp,
  FolderOpen,
  Pencil,
  Pin,
  PinOff,
  Play,
  Shield,
  Trash2,
} from "lucide-react";
import { useDockHorizontalBounds } from "../lib/dockBounds";
import type { Category, DockItem } from "../types/dock";

interface ContextMenuProps {
  item: DockItem;
  x: number;
  y: number;
  canMoveUp: boolean;
  canMoveDown: boolean;
  onClose: () => void;
  onLaunch: (item: DockItem, runAsAdmin: boolean) => void;
  onTogglePin: (item: DockItem) => void;
  onEdit: (item: DockItem) => void;
  onOpenLocation: (item: DockItem) => void;
  onMoveCategory: (item: DockItem, category: Category) => void;
  onMoveStep: (item: DockItem, direction: "up" | "down") => void;
  onRemove: (item: DockItem) => void;
}

const ALL_CATEGORIES: readonly Category[] = ["Games", "Apps", "Tools"];
const VIEWPORT_PADDING = 6;

function canOpenFileLocation(item: DockItem): boolean {
  if (item.source === "Steam") {
    return Boolean(item.icon_source_path && item.icon_source_path.trim());
  }
  const p = item.path.trim().toLowerCase();
  if (!p) return false;
  if (p.includes("://") || p.startsWith("ms-settings:") || p.startsWith("shell:")) {
    return false;
  }
  return true;
}

export function ContextMenu({
  item,
  x,
  y,
  canMoveUp,
  canMoveDown,
  onClose,
  onLaunch,
  onTogglePin,
  onEdit,
  onOpenLocation,
  onMoveCategory,
  onMoveStep,
  onRemove,
}: ContextMenuProps) {
  const menuRef = useRef<HTMLDivElement | null>(null);
  const getDockBounds = useDockHorizontalBounds();
  const [pos, setPos] = useState<{ left: number; top: number }>({
    left: x,
    top: y,
  });

  useLayoutEffect(() => {
    const el = menuRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const dockBounds = getDockBounds();
    const minX = Math.max(0, dockBounds.minX);
    const maxX = Math.min(window.innerWidth, dockBounds.maxX);
    const minLeft = minX + VIEWPORT_PADDING;
    const maxLeft = Math.max(minLeft, maxX - rect.width - VIEWPORT_PADDING);
    const maxTop = Math.max(
      VIEWPORT_PADDING,
      window.innerHeight - rect.height - VIEWPORT_PADDING,
    );
    setPos({
      left: Math.min(Math.max(x, minLeft), maxLeft),
      top: Math.min(Math.max(y, VIEWPORT_PADDING), maxTop),
    });
  }, [x, y, item.id, getDockBounds]);

  useEffect(() => {
    const handlePointerDown = (e: PointerEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        onClose();
      }
    };
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    const handleBlur = () => onClose();

    window.addEventListener("pointerdown", handlePointerDown, true);
    window.addEventListener("keydown", handleKeyDown, true);
    window.addEventListener("blur", handleBlur);
    return () => {
      window.removeEventListener("pointerdown", handlePointerDown, true);
      window.removeEventListener("keydown", handleKeyDown, true);
      window.removeEventListener("blur", handleBlur);
    };
  }, [onClose]);

  const locationAvailable = canOpenFileLocation(item);

  return createPortal(
    <div
      ref={menuRef}
      role="menu"
      style={{ left: `${pos.left}px`, top: `${pos.top}px` }}
      className="fixed z-50 w-[212px] rounded-lg bg-harbor-floating border border-white/[0.08] shadow-2xl p-1.5 text-[12.5px] text-harbor-textSecondary select-none"
    >
      <button
        type="button"
        role="menuitem"
        onClick={() => {
          onClose();
          onLaunch(item, false);
        }}
        className="w-full px-2 py-1.5 rounded flex items-center gap-2 text-left text-harbor-textPrimary hover:bg-harbor-blurple hover:text-white transition-colors"
      >
        <Play className="w-3.5 h-3.5 shrink-0" strokeWidth={1.75} />
        <span>Launch</span>
      </button>

      <button
        type="button"
        role="menuitem"
        onClick={() => {
          onClose();
          onLaunch(item, true);
        }}
        className="w-full px-2 py-1.5 rounded flex items-center gap-2 text-left hover:bg-harbor-blurple hover:text-white transition-colors"
      >
        <Shield className="w-3.5 h-3.5 shrink-0" strokeWidth={1.75} />
        <span>Run as Administrator</span>
      </button>

      <div className="my-1 h-[1px] bg-white/[0.06]" />

      <button
        type="button"
        role="menuitem"
        onClick={() => {
          onClose();
          onTogglePin(item);
        }}
        className="w-full px-2 py-1.5 rounded flex items-center gap-2 text-left hover:bg-harbor-hover hover:text-harbor-textPrimary transition-colors"
      >
        {item.pinned ? (
          <PinOff className="w-3.5 h-3.5 shrink-0" strokeWidth={1.75} />
        ) : (
          <Pin className="w-3.5 h-3.5 shrink-0" strokeWidth={1.75} />
        )}
        <span>{item.pinned ? "Unpin" : "Pin to Top"}</span>
      </button>

      <button
        type="button"
        role="menuitem"
        onClick={() => {
          onClose();
          onEdit(item);
        }}
        className="w-full px-2 py-1.5 rounded flex items-center gap-2 text-left hover:bg-harbor-hover hover:text-harbor-textPrimary transition-colors"
      >
        <Pencil className="w-3.5 h-3.5 shrink-0" strokeWidth={1.75} />
        <span>Edit Shortcut</span>
      </button>

      <button
        type="button"
        role="menuitem"
        disabled={!locationAvailable}
        onClick={() => {
          onClose();
          onOpenLocation(item);
        }}
        className="w-full px-2 py-1.5 rounded flex items-center gap-2 text-left hover:bg-harbor-hover hover:text-harbor-textPrimary disabled:opacity-40 disabled:pointer-events-none transition-colors"
      >
        <FolderOpen className="w-3.5 h-3.5 shrink-0" strokeWidth={1.75} />
        <span>Open File Location</span>
      </button>

      <div className="my-1 h-[1px] bg-white/[0.06]" />

      <div className="px-2 py-1">
        <div className="text-[10px] font-semibold uppercase tracking-wider text-harbor-textMuted mb-1">
          Move to Category
        </div>
        <div className="grid grid-cols-3 gap-1">
          {ALL_CATEGORIES.map((cat) => {
            const isCurrent = item.category === cat;
            return (
              <button
                key={cat}
                type="button"
                disabled={isCurrent}
                onClick={() => {
                  onClose();
                  onMoveCategory(item, cat);
                }}
                className={`px-1.5 py-1 rounded text-[11px] font-medium transition-colors ${
                  isCurrent
                    ? "bg-harbor-blurple/25 text-harbor-blurple cursor-default"
                    : "bg-harbor-rail hover:bg-harbor-hover text-harbor-textSecondary hover:text-harbor-textPrimary"
                }`}
              >
                {cat}
              </button>
            );
          })}
        </div>
      </div>

      <div className="grid grid-cols-2 gap-1 px-1 py-0.5">
        <button
          type="button"
          role="menuitem"
          disabled={!canMoveUp}
          onClick={() => {
            onClose();
            onMoveStep(item, "up");
          }}
          className="px-2 py-1 rounded flex items-center justify-center gap-1 bg-harbor-rail hover:bg-harbor-hover hover:text-harbor-textPrimary disabled:opacity-40 disabled:pointer-events-none transition-colors text-[11.5px]"
        >
          <ArrowUp className="w-3 h-3" strokeWidth={1.75} />
          <span>Move Up</span>
        </button>
        <button
          type="button"
          role="menuitem"
          disabled={!canMoveDown}
          onClick={() => {
            onClose();
            onMoveStep(item, "down");
          }}
          className="px-2 py-1 rounded flex items-center justify-center gap-1 bg-harbor-rail hover:bg-harbor-hover hover:text-harbor-textPrimary disabled:opacity-40 disabled:pointer-events-none transition-colors text-[11.5px]"
        >
          <ArrowDown className="w-3 h-3" strokeWidth={1.75} />
          <span>Move Down</span>
        </button>
      </div>

      <div className="my-1 h-[1px] bg-white/[0.06]" />

      <button
        type="button"
        role="menuitem"
        onClick={() => {
          onClose();
          onRemove(item);
        }}
        className="w-full px-2 py-1.5 rounded flex items-center gap-2 text-left text-harbor-danger hover:bg-harbor-danger hover:text-white transition-colors"
      >
        <Trash2 className="w-3.5 h-3.5 shrink-0" strokeWidth={1.75} />
        <span>Remove from Harbor</span>
      </button>
    </div>,
    document.body,
  );
}
