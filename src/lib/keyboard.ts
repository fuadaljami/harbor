import type { Category, DockItem } from "../types/dock";

export type RailCategory = "All" | Category;

export const SECTION_CATEGORIES: readonly Category[] = [
  "Games",
  "Apps",
  "Tools",
];

export const RAIL_CATEGORY_ORDER: readonly RailCategory[] = [
  "All",
  "Games",
  "Apps",
  "Tools",
];

function isWordBoundaryChar(ch: string | undefined): boolean {
  if (!ch) return true;
  return /[\s\-_.:/\\]/.test(ch);
}

/**
 * Returns a numeric relevance score where lower is better, or `null` when
 * the item does not match the normalized lowercase search query.
 */
export function scoreShortcutMatch(
  item: DockItem,
  normalizedQuery: string,
): number | null {
  if (!normalizedQuery) {
    return 0;
  }

  const lowerName = item.name.toLowerCase();
  if (lowerName === normalizedQuery) {
    return 0;
  }
  if (lowerName.startsWith(normalizedQuery)) {
    return 1;
  }

  const nameIdx = lowerName.indexOf(normalizedQuery);
  if (nameIdx !== -1) {
    return isWordBoundaryChar(lowerName[nameIdx - 1]) ? 2 : 3;
  }

  if (item.description.toLowerCase().includes(normalizedQuery)) {
    return 4;
  }
  if (item.path.toLowerCase().includes(normalizedQuery)) {
    return 5;
  }

  return null;
}

export function filterAndRankShortcuts(
  items: readonly DockItem[],
  searchQuery: string,
): DockItem[] {
  const normalizedQuery = searchQuery.trim().toLowerCase();
  if (!normalizedQuery) {
    return [...items];
  }

  const ranked: Array<{ item: DockItem; score: number; index: number }> = [];
  for (let index = 0; index < items.length; index += 1) {
    const item = items[index];
    if (!item) continue;
    const score = scoreShortcutMatch(item, normalizedQuery);
    if (score !== null) {
      ranked.push({ item, score, index });
    }
  }

  ranked.sort((a, b) => a.score - b.score || a.index - b.index);
  return ranked.map((entry) => entry.item);
}

export interface OrderedCategoryItems {
  pinned: DockItem[];
  unpinned: DockItem[];
  combined: DockItem[];
}

export function getOrderedCategoryItems(
  items: readonly DockItem[],
  category: Category,
  searchQuery: string,
): OrderedCategoryItems {
  const inCategory = items.filter((item) => item.category === category);
  const ranked = filterAndRankShortcuts(inCategory, searchQuery);
  const pinned: DockItem[] = [];
  const unpinned: DockItem[] = [];

  for (const item of ranked) {
    if (item.pinned) {
      pinned.push(item);
    } else {
      unpinned.push(item);
    }
  }

  return {
    pinned,
    unpinned,
    combined: [...pinned, ...unpinned],
  };
}

export function getVisibleShortcutIds(
  items: readonly DockItem[],
  selectedCategory: RailCategory,
  searchQuery: string,
  collapsedSections: Readonly<Record<Category, boolean>>,
): string[] {
  const isSearching = searchQuery.trim().length > 0;

  if (selectedCategory === "All") {
    const visibleIds: string[] = [];
    for (const category of SECTION_CATEGORIES) {
      const { combined } = getOrderedCategoryItems(items, category, searchQuery);
      if (combined.length === 0) continue;
      if (!isSearching && collapsedSections[category]) continue;
      for (const item of combined) {
        visibleIds.push(item.id);
      }
    }
    return visibleIds;
  }

  const { combined } = getOrderedCategoryItems(
    items,
    selectedCategory,
    searchQuery,
  );
  return combined.map((item) => item.id);
}

export function stepHighlightedId(
  visibleIds: readonly string[],
  currentId: string | null,
  direction: "next" | "prev",
): string | null {
  if (visibleIds.length === 0) {
    return null;
  }

  const idx = currentId !== null ? visibleIds.indexOf(currentId) : -1;
  if (idx === -1) {
    return direction === "next"
      ? (visibleIds[0] ?? null)
      : (visibleIds[visibleIds.length - 1] ?? null);
  }

  const nextIdx =
    direction === "next"
      ? (idx + 1) % visibleIds.length
      : (idx - 1 + visibleIds.length) % visibleIds.length;
  return visibleIds[nextIdx] ?? null;
}

export function resolveHighlightAfterListChange(
  visibleIds: readonly string[],
  currentId: string | null,
  isSearching: boolean,
): string | null {
  if (visibleIds.length === 0) {
    return null;
  }
  if (currentId !== null && visibleIds.includes(currentId)) {
    return currentId;
  }
  return isSearching ? (visibleIds[0] ?? null) : null;
}

export function cycleCategory(
  current: RailCategory,
  direction: "next" | "prev",
): RailCategory {
  const idx = RAIL_CATEGORY_ORDER.indexOf(current);
  const safeIdx = idx === -1 ? 0 : idx;
  const nextIdx =
    direction === "next"
      ? (safeIdx + 1) % RAIL_CATEGORY_ORDER.length
      : (safeIdx - 1 + RAIL_CATEGORY_ORDER.length) % RAIL_CATEGORY_ORDER.length;
  return RAIL_CATEGORY_ORDER[nextIdx] ?? "All";
}

export function categoryForShortcutDigit(key: string): RailCategory | null {
  switch (key) {
    case "1":
      return "All";
    case "2":
      return "Games";
    case "3":
      return "Apps";
    case "4":
      return "Tools";
    default:
      return null;
  }
}
