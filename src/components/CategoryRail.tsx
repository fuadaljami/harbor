import { Anchor, AppWindow, Gamepad2, Plus, Wrench } from "lucide-react";
import type { Category, DockEdge } from "../types/dock";
import { Tooltip } from "./Tooltip";

export type RailCategory = "All" | Category;

interface CategoryRailProps {
  edge: DockEdge;
  selectedCategory: RailCategory;
  counts: Record<RailCategory, number>;
  onSelectCategory: (category: RailCategory) => void;
  onAddClick: () => void;
}

const RAIL_ITEMS: ReadonlyArray<{
  id: RailCategory;
  label: string;
  Icon: typeof Anchor;
}> = [
  { id: "All", label: "All Shortcuts", Icon: Anchor },
  { id: "Games", label: "Games", Icon: Gamepad2 },
  { id: "Apps", label: "Apps", Icon: AppWindow },
  { id: "Tools", label: "Tools", Icon: Wrench },
];

export function CategoryRail({
  edge,
  selectedCategory,
  counts,
  onSelectCategory,
  onAddClick,
}: CategoryRailProps) {
  const isRight = edge === "Right";
  const tooltipSide = isRight ? "left" : "right";

  return (
    <nav
      aria-label="Shortcut categories"
      className={`w-[56px] h-full bg-[#1E1F22] shrink-0 py-2.5 flex flex-col items-center gap-2 ${
        isRight
          ? "border-l border-[#1F2023]"
          : "border-r border-[#1F2023]"
      }`}
    >
      {RAIL_ITEMS.map(({ id, label, Icon }) => {
        const active = selectedCategory === id;
        return (
          <div
            key={id}
            className="relative w-full h-[42px] flex items-center justify-center group"
          >
            <span
              className={`absolute top-1/2 -translate-y-1/2 w-[4px] bg-harbor-textPrimary transition-all duration-150 ease-discord ${
                isRight ? "right-0 rounded-l-full" : "left-0 rounded-r-full"
              } ${
                active
                  ? "h-[36px] opacity-100"
                  : "h-0 opacity-0 group-hover:h-[20px] group-hover:opacity-100"
              }`}
            />
            <Tooltip label={label} badge={counts[id]} side={tooltipSide}>
              <button
                type="button"
                aria-label={label}
                aria-pressed={active}
                onClick={() => onSelectCategory(id)}
                className={`w-[40px] h-[40px] flex items-center justify-center transition-all duration-150 ease-discord focus:outline-none focus-visible:ring-2 focus-visible:ring-harbor-blurple ${
                  active
                    ? "rounded-[12px] bg-harbor-blurple text-white"
                    : "rounded-[20px] bg-harbor-panel text-harbor-textSecondary hover:rounded-[12px] hover:bg-harbor-blurple hover:text-white"
                }`}
              >
                <Icon className="w-5 h-5" strokeWidth={1.75} />
              </button>
            </Tooltip>
          </div>
        );
      })}

      <div className="w-[32px] h-[2px] rounded-full bg-harbor-divider my-0.5 shrink-0" />

      <div className="relative w-full h-[42px] flex items-center justify-center group">
        <span
          className={`absolute top-1/2 -translate-y-1/2 w-[4px] bg-harbor-success transition-all duration-150 ease-discord h-0 opacity-0 group-hover:h-[20px] group-hover:opacity-100 ${
            isRight ? "right-0 rounded-l-full" : "left-0 rounded-r-full"
          }`}
        />
        <Tooltip label="Add Shortcut" side={tooltipSide}>
          <button
            type="button"
            aria-label="Add Shortcut"
            onClick={onAddClick}
            className="w-[40px] h-[40px] rounded-[20px] bg-harbor-panel text-harbor-success hover:rounded-[12px] hover:bg-harbor-success hover:text-white flex items-center justify-center transition-all duration-150 ease-discord focus:outline-none focus-visible:ring-2 focus-visible:ring-harbor-success"
          >
            <Plus className="w-5 h-5" strokeWidth={2} />
          </button>
        </Tooltip>
      </div>
    </nav>
  );
}
