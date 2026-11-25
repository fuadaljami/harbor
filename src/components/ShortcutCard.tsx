import { useEffect, useRef, useState } from "react";
import { useSortable } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { GripVertical, MoreHorizontal, Pin } from "lucide-react";
import type { DockItem } from "../types/dock";

interface ShortcutCardProps {
  item: DockItem;
  iconUrl: string | null;
  sortable: boolean;
  isFocused: boolean;
  scrollRoot?: HTMLDivElement | null;
  onRequestIcon: (item: DockItem) => void;
  onLaunch: (item: DockItem, runAsAdmin: boolean) => void;
  onOpenContextMenu: (item: DockItem, x: number, y: number) => void;
  registerCardRef?: (id: string, el: HTMLDivElement | null) => void;
}

const MONOGRAM_BG_PALETTE = [
  "bg-[#4752C4]/30 text-[#B9C0FF]",
  "bg-[#23A55A]/25 text-[#7AE4A4]",
  "bg-[#F0B232]/20 text-[#F7CE78]",
  "bg-[#E67E22]/25 text-[#F6B882]",
  "bg-[#9B59B6]/25 text-[#D7A8EB]",
  "bg-[#3498DB]/25 text-[#8BC9F2]",
] as const;

function getMonogram(name: string): string {
  const words = name
    .trim()
    .split(/\s+/)
    .filter((w) => w.length > 0);
  if (words.length >= 2) {
    const first = words[0]?.[0] ?? "";
    const second = words[1]?.[0] ?? "";
    const pair = `${first}${second}`.toUpperCase();
    if (pair.length === 2) return pair;
  }
  const clean = name.replace(/[^a-zA-Z0-9]/g, "");
  return (clean.slice(0, 2) || name.slice(0, 1) || "?").toUpperCase();
}

function getMonogramStyle(id: string): string {
  let hash = 0;
  for (let i = 0; i < id.length; i += 1) {
    hash = (hash * 31 + id.charCodeAt(i)) >>> 0;
  }
  return (
    MONOGRAM_BG_PALETTE[hash % MONOGRAM_BG_PALETTE.length] ??
    MONOGRAM_BG_PALETTE[0]
  );
}

function getSubtitle(item: DockItem): string {
  const desc = item.description.trim();
  if (desc.length > 0) {
    return desc;
  }
  if (item.source === "Steam") {
    return "Steam Game";
  }
  const clean = item.path.replace(/^"+|"+$/g, "");
  const parts = clean.split(/[\\/]/);
  return parts[parts.length - 1] || clean;
}

export function ShortcutCard({
  item,
  iconUrl,
  sortable,
  isFocused,
  scrollRoot,
  onRequestIcon,
  onLaunch,
  onOpenContextMenu,
  registerCardRef,
}: ShortcutCardProps) {
  const [imgBroken, setImgBroken] = useState(false);
  const localCardRef = useRef<HTMLDivElement | null>(null);

  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({
    id: item.id,
    disabled: !sortable,
  });

  useEffect(() => {
    setImgBroken(false);
  }, [iconUrl]);

  useEffect(() => {
    const el = localCardRef.current;
    if (!el) return;

    if (typeof IntersectionObserver === "undefined") {
      onRequestIcon(item);
      return;
    }

    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) {
            onRequestIcon(item);
            observer.disconnect();
            break;
          }
        }
      },
      { root: scrollRoot ?? null, rootMargin: "100px" },
    );
    observer.observe(el);
    return () => observer.disconnect();
  }, [item, onRequestIcon, scrollRoot]);

  const setCombinedRef = (el: HTMLDivElement | null) => {
    localCardRef.current = el;
    setNodeRef(el);
    registerCardRef?.(item.id, el);
  };

  const style: React.CSSProperties = {
    transform: CSS.Transform.toString(transform),
    transition,
  };

  const showImg = iconUrl !== null && !imgBroken;

  return (
    <div
      ref={setCombinedRef}
      style={style}
      role="button"
      tabIndex={-1}
      data-shortcut-id={item.id}
      onClick={(e) => {
        if (isDragging) return;
        onLaunch(item, e.ctrlKey);
      }}
      onContextMenu={(e) => {
        e.preventDefault();
        onOpenContextMenu(item, e.clientX, e.clientY);
      }}
      className={`h-[46px] px-2 rounded-[6px] flex items-center gap-2.5 group transition-colors cursor-pointer select-none ${
        isDragging
          ? "opacity-75 bg-harbor-active shadow-xl z-20 relative"
          : isFocused
            ? "bg-harbor-active ring-1 ring-inset ring-harbor-blurple/75"
            : "hover:bg-harbor-hover"
      }`}
    >
      <div className="w-[32px] h-[32px] rounded-[8px] bg-harbor-rail border border-white/[0.05] flex items-center justify-center shrink-0 overflow-hidden">
        {showImg ? (
          <img
            src={iconUrl}
            alt=""
            onError={() => setImgBroken(true)}
            className="w-[22px] h-[22px] object-contain pointer-events-none"
          />
        ) : (
          <span
            className={`w-full h-full flex items-center justify-center text-[11px] font-bold tracking-tight ${getMonogramStyle(item.id)}`}
          >
            {getMonogram(item.name)}
          </span>
        )}
      </div>

      <div className="min-w-0 flex-1 overflow-hidden">
        <div className="flex items-center gap-1.5 min-w-0">
          <span className="text-[13.5px] font-medium text-harbor-textPrimary truncate leading-tight">
            {item.name}
          </span>
          {item.pinned && (
            <Pin
              className="w-3 h-3 text-harbor-warning shrink-0"
              strokeWidth={2}
            />
          )}
          {item.category === "Games" && (
            <span className="px-1.5 py-[1px] rounded bg-harbor-blurple/20 text-harbor-blurple text-[9.5px] font-semibold tracking-wide shrink-0">
              GAME
            </span>
          )}
          {item.run_as_admin && (
            <span className="px-1.5 py-[1px] rounded bg-harbor-warning/15 text-harbor-warning text-[9.5px] font-semibold tracking-wide shrink-0">
              ADMIN
            </span>
          )}
        </div>
        <div className="text-[11.5px] text-harbor-textMuted truncate leading-tight mt-0.5">
          {getSubtitle(item)}
        </div>
      </div>

      <div
        className={`flex items-center gap-0.5 shrink-0 transition-opacity duration-100 ${
          isFocused
            ? "opacity-100"
            : "opacity-0 group-hover:opacity-100 focus-within:opacity-100"
        }`}
      >
        {sortable && (
          <button
            type="button"
            aria-label={`Reorder ${item.name}`}
            onClick={(e) => e.stopPropagation()}
            {...attributes}
            {...listeners}
            className="p-1 rounded text-harbor-textMuted hover:text-harbor-textPrimary hover:bg-harbor-active cursor-grab active:cursor-grabbing"
          >
            <GripVertical className="w-3.5 h-3.5" strokeWidth={1.75} />
          </button>
        )}
        <button
          type="button"
          aria-label={`More actions for ${item.name}`}
          onClick={(e) => {
            e.stopPropagation();
            const rect = e.currentTarget.getBoundingClientRect();
            onOpenContextMenu(item, rect.right, rect.bottom + 4);
          }}
          className="p-1 rounded text-harbor-textMuted hover:text-harbor-textPrimary hover:bg-harbor-active"
        >
          <MoreHorizontal className="w-4 h-4" strokeWidth={1.75} />
        </button>
      </div>
    </div>
  );
}
