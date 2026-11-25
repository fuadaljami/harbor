import { useState, type RefObject } from "react";
import {
  DndContext,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core";
import {
  SortableContext,
  arrayMove,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import {
  ChevronDown,
  ChevronRight,
  FolderPlus,
  SearchX,
  Upload,
} from "lucide-react";
import type { Category, DockItem } from "../types/dock";
import type { RailCategory } from "./CategoryRail";
import { ShortcutCard } from "./ShortcutCard";

interface ShortcutListProps {
  items: DockItem[];
  selectedCategory: RailCategory;
  searchQuery: string;
  focusedItemId: string | null;
  isDropHover: boolean;
  scrollContainerRef: RefObject<HTMLDivElement>;
  getIconUrl: (item: DockItem) => string | null;
  onRequestIcon: (item: DockItem) => void;
  onLaunch: (item: DockItem, runAsAdmin: boolean) => void;
  onOpenContextMenu: (item: DockItem, x: number, y: number) => void;
  onReorderGroup: (
    category: Category,
    pinned: boolean,
    orderedIds: string[],
  ) => void;
  onAddClick: () => void;
  onClearSearch: () => void;
  registerCardRef?: (id: string, el: HTMLDivElement | null) => void;
}

const SECTION_CATEGORIES: readonly Category[] = ["Games", "Apps", "Tools"];

interface SortableSubgroupProps {
  category: Category;
  pinned: boolean;
  items: DockItem[];
  sortable: boolean;
  focusedItemId: string | null;
  getIconUrl: (item: DockItem) => string | null;
  onRequestIcon: (item: DockItem) => void;
  onLaunch: (item: DockItem, runAsAdmin: boolean) => void;
  onOpenContextMenu: (item: DockItem, x: number, y: number) => void;
  onReorderGroup: (
    category: Category,
    pinned: boolean,
    orderedIds: string[],
  ) => void;
  registerCardRef?: (id: string, el: HTMLDivElement | null) => void;
}

function SortableSubgroup({
  category,
  pinned,
  items,
  sortable,
  focusedItemId,
  getIconUrl,
  onRequestIcon,
  onLaunch,
  onOpenContextMenu,
  onReorderGroup,
  registerCardRef,
}: SortableSubgroupProps) {
  const sensors = useSensors(
    useSensor(PointerSensor, {
      activationConstraint: { distance: 6 },
    }),
  );

  if (items.length === 0) {
    return null;
  }

  const itemIds = items.map((item) => item.id);

  const handleDragEnd = (event: DragEndEvent) => {
    const { active, over } = event;
    if (!over || active.id === over.id) {
      return;
    }
    const oldIndex = itemIds.indexOf(String(active.id));
    const newIndex = itemIds.indexOf(String(over.id));
    if (oldIndex < 0 || newIndex < 0) {
      return;
    }
    const nextIds = arrayMove(itemIds, oldIndex, newIndex);
    onReorderGroup(category, pinned, nextIds);
  };

  return (
    <DndContext
      sensors={sensors}
      collisionDetection={closestCenter}
      onDragEnd={handleDragEnd}
    >
      <SortableContext items={itemIds} strategy={verticalListSortingStrategy}>
        <div className="flex flex-col gap-0.5">
          {items.map((item) => (
            <ShortcutCard
              key={item.id}
              item={item}
              iconUrl={getIconUrl(item)}
              sortable={sortable && items.length > 1}
              isFocused={focusedItemId === item.id}
              onRequestIcon={onRequestIcon}
              onLaunch={onLaunch}
              onOpenContextMenu={onOpenContextMenu}
              registerCardRef={registerCardRef}
            />
          ))}
        </div>
      </SortableContext>
    </DndContext>
  );
}

export function ShortcutList({
  items,
  selectedCategory,
  searchQuery,
  focusedItemId,
  isDropHover,
  scrollContainerRef,
  getIconUrl,
  onRequestIcon,
  onLaunch,
  onOpenContextMenu,
  onReorderGroup,
  onAddClick,
  onClearSearch,
  registerCardRef,
}: ShortcutListProps) {
  const [collapsedSections, setCollapsedSections] = useState<
    Record<Category, boolean>
  >({
    Games: false,
    Apps: false,
    Tools: false,
  });

  const normalizedQuery = searchQuery.trim().toLowerCase();
  const isSearching = normalizedQuery.length > 0;
  const sortable = !isSearching;

  const matchesQuery = (item: DockItem): boolean => {
    if (!isSearching) return true;
    return (
      item.name.toLowerCase().includes(normalizedQuery) ||
      item.description.toLowerCase().includes(normalizedQuery) ||
      item.path.toLowerCase().includes(normalizedQuery)
    );
  };

  const filteredByCategory = (cat: Category): DockItem[] =>
    items.filter((item) => item.category === cat && matchesQuery(item));

  const toggleSection = (cat: Category) => {
    setCollapsedSections((prev) => ({
      ...prev,
      [cat]: !prev[cat],
    }));
  };

  const totalVisible =
    selectedCategory === "All"
      ? items.filter(matchesQuery).length
      : filteredByCategory(selectedCategory).length;

  const dropTargetCategory =
    selectedCategory === "All" ? "Apps" : selectedCategory;

  return (
    <div className="relative flex-1 min-h-0 flex flex-col">
      <div
        ref={scrollContainerRef}
        className="flex-1 overflow-y-auto px-2 py-1.5"
      >
        {totalVisible === 0 ? (
          <div className="h-full min-h-[220px] flex flex-col items-center justify-center text-center px-6">
            {isSearching ? (
              <>
                <div className="w-10 h-10 rounded-xl bg-harbor-rail flex items-center justify-center mb-2.5 text-harbor-textMuted">
                  <SearchX className="w-5 h-5" strokeWidth={1.75} />
                </div>
                <p className="text-[13px] font-medium text-harbor-textPrimary">
                  No shortcuts match &ldquo;{searchQuery.trim()}&rdquo;
                </p>
                <p className="text-[11.5px] text-harbor-textMuted mt-1">
                  Try another search term or switch categories.
                </p>
                <button
                  type="button"
                  onClick={onClearSearch}
                  className="mt-3 px-3 py-1.5 rounded-md bg-harbor-hover hover:bg-harbor-active text-xs font-medium text-harbor-textPrimary transition-colors"
                >
                  Clear Search
                </button>
              </>
            ) : (
              <>
                <div className="w-10 h-10 rounded-xl bg-harbor-rail flex items-center justify-center mb-2.5 text-harbor-textMuted">
                  <FolderPlus className="w-5 h-5" strokeWidth={1.75} />
                </div>
                <p className="text-[13px] font-medium text-harbor-textPrimary">
                  No shortcuts in {selectedCategory}
                </p>
                <p className="text-[11.5px] text-harbor-textMuted mt-1">
                  Drop an executable here or add a shortcut manually.
                </p>
                <button
                  type="button"
                  onClick={onAddClick}
                  className="mt-3 px-3 py-1.5 rounded-md bg-harbor-blurple hover:bg-harbor-blurpleHover text-xs font-medium text-white transition-colors"
                >
                  Add Shortcut
                </button>
              </>
            )}
          </div>
        ) : selectedCategory === "All" ? (
          <div className="flex flex-col gap-3 pb-2">
            {SECTION_CATEGORIES.map((category) => {
              const sectionItems = filteredByCategory(category);
              if (sectionItems.length === 0) {
                return null;
              }
              const pinnedItems = sectionItems.filter((i) => i.pinned);
              const unpinnedItems = sectionItems.filter((i) => !i.pinned);
              const collapsed = !isSearching && collapsedSections[category];

              return (
                <section key={category} aria-label={category}>
                  <button
                    type="button"
                    onClick={() => toggleSection(category)}
                    className="w-full px-1.5 py-1 flex items-center gap-1 text-[11px] font-semibold uppercase tracking-wider text-harbor-textMuted hover:text-harbor-textSecondary transition-colors"
                  >
                    {collapsed ? (
                      <ChevronRight
                        className="w-3 h-3 shrink-0"
                        strokeWidth={2}
                      />
                    ) : (
                      <ChevronDown
                        className="w-3 h-3 shrink-0"
                        strokeWidth={2}
                      />
                    )}
                    <span>{category}</span>
                    <span className="ml-0.5 text-[10px] font-medium text-harbor-textMuted/80">
                      — {sectionItems.length}
                    </span>
                  </button>

                  {!collapsed && (
                    <div className="mt-0.5 flex flex-col gap-0.5">
                      <SortableSubgroup
                        category={category}
                        pinned={true}
                        items={pinnedItems}
                        sortable={sortable}
                        focusedItemId={focusedItemId}
                        getIconUrl={getIconUrl}
                        onRequestIcon={onRequestIcon}
                        onLaunch={onLaunch}
                        onOpenContextMenu={onOpenContextMenu}
                        onReorderGroup={onReorderGroup}
                        registerCardRef={registerCardRef}
                      />
                      <SortableSubgroup
                        category={category}
                        pinned={false}
                        items={unpinnedItems}
                        sortable={sortable}
                        focusedItemId={focusedItemId}
                        getIconUrl={getIconUrl}
                        onRequestIcon={onRequestIcon}
                        onLaunch={onLaunch}
                        onOpenContextMenu={onOpenContextMenu}
                        onReorderGroup={onReorderGroup}
                        registerCardRef={registerCardRef}
                      />
                    </div>
                  )}
                </section>
              );
            })}
          </div>
        ) : (
          <div className="flex flex-col gap-0.5 pb-2">
            <SortableSubgroup
              category={selectedCategory}
              pinned={true}
              items={filteredByCategory(selectedCategory).filter(
                (i) => i.pinned,
              )}
              sortable={sortable}
              focusedItemId={focusedItemId}
              getIconUrl={getIconUrl}
              onRequestIcon={onRequestIcon}
              onLaunch={onLaunch}
              onOpenContextMenu={onOpenContextMenu}
              onReorderGroup={onReorderGroup}
              registerCardRef={registerCardRef}
            />
            <SortableSubgroup
              category={selectedCategory}
              pinned={false}
              items={filteredByCategory(selectedCategory).filter(
                (i) => !i.pinned,
              )}
              sortable={sortable}
              focusedItemId={focusedItemId}
              getIconUrl={getIconUrl}
              onRequestIcon={onRequestIcon}
              onLaunch={onLaunch}
              onOpenContextMenu={onOpenContextMenu}
              onReorderGroup={onReorderGroup}
              registerCardRef={registerCardRef}
            />
          </div>
        )}
      </div>

      {isDropHover && (
        <div className="absolute inset-2 z-30 rounded-lg border-2 border-dashed border-harbor-blurple bg-harbor-rail/90 flex flex-col items-center justify-center text-center px-6 pointer-events-none">
          <div className="w-11 h-11 rounded-xl bg-harbor-blurple/20 text-harbor-blurple flex items-center justify-center mb-2.5">
            <Upload className="w-5 h-5" strokeWidth={1.75} />
          </div>
          <p className="text-[13px] font-semibold text-harbor-textPrimary">
            Drop to Pin in {dropTargetCategory}
          </p>
          <p className="text-[11.5px] text-harbor-textMuted mt-1">
            Executables and .lnk shortcuts will be added and pinned.
          </p>
        </div>
      )}
    </div>
  );
}
