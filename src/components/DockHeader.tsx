import {
  ArrowLeftRight,
  ChevronLeft,
  ChevronRight,
  Pin,
  RefreshCw,
} from "lucide-react";
import type { DockEdge } from "../types/dock";
import type { RailCategory } from "./CategoryRail";
import { Tooltip } from "./Tooltip";

interface DockHeaderProps {
  selectedCategory: RailCategory;
  itemCount: number;
  pinned: boolean;
  edge: DockEdge;
  isRescanning: boolean;
  onTogglePin: () => void;
  onRescan: () => void;
  onToggleEdge: () => void;
  onCollapse: () => void;
}

const CATEGORY_TITLES: Record<RailCategory, string> = {
  All: "All Shortcuts",
  Games: "Games",
  Apps: "Apps",
  Tools: "Tools",
};

export function DockHeader({
  selectedCategory,
  itemCount,
  pinned,
  edge,
  isRescanning,
  onTogglePin,
  onRescan,
  onToggleEdge,
  onCollapse,
}: DockHeaderProps) {
  return (
    <header className="h-[48px] px-3 flex items-center justify-between border-b border-harbor-border shrink-0">
      <div className="min-w-0 flex items-center gap-2">
        <h1 className="text-[14px] font-semibold tracking-tight text-harbor-textPrimary truncate">
          {CATEGORY_TITLES[selectedCategory]}
        </h1>
        <span className="px-1.5 py-0.5 rounded-full bg-harbor-rail text-[11px] font-semibold text-harbor-textSecondary tabular-nums shrink-0">
          {itemCount}
        </span>
      </div>

      <div className="flex items-center gap-1 shrink-0">
        <Tooltip
          label={pinned ? "Unpin Dock" : "Pin Dock Open"}
          side="bottom"
        >
          <button
            type="button"
            aria-label={pinned ? "Unpin Dock" : "Pin Dock Open"}
            aria-pressed={pinned}
            onClick={onTogglePin}
            className={`w-7 h-7 rounded-md flex items-center justify-center transition-colors ${
              pinned
                ? "bg-harbor-warning/15 text-harbor-warning hover:bg-harbor-warning/25"
                : "text-harbor-textSecondary hover:text-harbor-textPrimary hover:bg-harbor-hover"
            }`}
          >
            <Pin className="w-4 h-4" strokeWidth={1.75} />
          </button>
        </Tooltip>

        <Tooltip
          label={isRescanning ? "Scanning System..." : "Rescan System"}
          side="bottom"
        >
          <button
            type="button"
            aria-label="Rescan System"
            disabled={isRescanning}
            onClick={onRescan}
            className="w-7 h-7 rounded-md flex items-center justify-center text-harbor-textSecondary hover:text-harbor-textPrimary hover:bg-harbor-hover disabled:opacity-60 transition-colors"
          >
            <RefreshCw
              className={`w-4 h-4 ${isRescanning ? "animate-spin text-harbor-blurple" : ""}`}
              strokeWidth={1.75}
            />
          </button>
        </Tooltip>

        <Tooltip
          label={
            edge === "Left" ? "Dock to Right Edge" : "Dock to Left Edge"
          }
          side="bottom"
        >
          <button
            type="button"
            aria-label="Switch Dock Edge"
            onClick={onToggleEdge}
            className="w-7 h-7 rounded-md flex items-center justify-center text-harbor-textSecondary hover:text-harbor-textPrimary hover:bg-harbor-hover transition-colors"
          >
            <ArrowLeftRight className="w-4 h-4" strokeWidth={1.75} />
          </button>
        </Tooltip>

        <Tooltip label="Collapse Dock" side="bottom">
          <button
            type="button"
            aria-label="Collapse Dock"
            onClick={onCollapse}
            className="w-7 h-7 rounded-md flex items-center justify-center text-harbor-textSecondary hover:text-harbor-textPrimary hover:bg-harbor-hover transition-colors"
          >
            {edge === "Left" ? (
              <ChevronLeft className="w-4 h-4" strokeWidth={1.75} />
            ) : (
              <ChevronRight className="w-4 h-4" strokeWidth={1.75} />
            )}
          </button>
        </Tooltip>
      </div>
    </header>
  );
}
