import { describe, expect, it } from "vitest";
import type { DockItem } from "../types/dock";
import {
  categoryForShortcutDigit,
  cycleCategory,
  filterAndRankShortcuts,
  getVisibleShortcutIds,
  resolveHighlightAfterListChange,
  stepHighlightedId,
} from "./keyboard";

function makeItem(
  overrides: Partial<DockItem> & Pick<DockItem, "id" | "name" | "category">,
): DockItem {
  return {
    id: overrides.id,
    name: overrides.name,
    path: overrides.path ?? `C:\\Program Files\\${overrides.name}\\app.exe`,
    arguments: overrides.arguments ?? "",
    description: overrides.description ?? "",
    category: overrides.category,
    custom_icon_path: overrides.custom_icon_path ?? null,
    icon_source_path: overrides.icon_source_path ?? null,
    run_as_admin: overrides.run_as_admin ?? false,
    pinned: overrides.pinned ?? false,
    source: overrides.source ?? "Manual",
    steam_app_id: overrides.steam_app_id ?? null,
    launch_count: overrides.launch_count ?? 0,
  };
}

describe("stepHighlightedId", () => {
  it("returns null on an empty list regardless of current id or direction", () => {
    expect(stepHighlightedId([], null, "next")).toBeNull();
    expect(stepHighlightedId([], "item-1", "prev")).toBeNull();
  });

  it("selects first on next and last on prev when currentId is null or missing", () => {
    const ids = ["a", "b", "c"];
    expect(stepHighlightedId(ids, null, "next")).toBe("a");
    expect(stepHighlightedId(ids, null, "prev")).toBe("c");
    expect(stepHighlightedId(ids, "deleted-id", "next")).toBe("a");
    expect(stepHighlightedId(ids, "deleted-id", "prev")).toBe("c");
  });

  it("wraps around cleanly at both ends", () => {
    const ids = ["a", "b", "c"];
    expect(stepHighlightedId(ids, "a", "next")).toBe("b");
    expect(stepHighlightedId(ids, "c", "next")).toBe("a");
    expect(stepHighlightedId(ids, "a", "prev")).toBe("c");
    expect(stepHighlightedId(ids, "b", "prev")).toBe("a");
  });
});

describe("getVisibleShortcutIds", () => {
  const sampleItems: DockItem[] = [
    makeItem({ id: "app-unpinned", name: "Discord", category: "Apps", pinned: false }),
    makeItem({ id: "app-pinned", name: "VS Code", category: "Apps", pinned: true }),
    makeItem({ id: "game-1", name: "Hollow Knight", category: "Games", pinned: false }),
    makeItem({ id: "tool-1", name: "Terminal", category: "Tools", pinned: false }),
  ];

  it("orders Games -> Apps -> Tools with pinned first and skips collapsed sections when not searching", () => {
    const allOpen = getVisibleShortcutIds(sampleItems, "All", "", {
      Games: false,
      Apps: false,
      Tools: false,
    });
    expect(allOpen).toEqual(["game-1", "app-pinned", "app-unpinned", "tool-1"]);

    const gamesCollapsed = getVisibleShortcutIds(sampleItems, "All", "", {
      Games: true,
      Apps: false,
      Tools: true,
    });
    expect(gamesCollapsed).toEqual(["app-pinned", "app-unpinned"]);
  });

  it("ignores collapsed state during active search so matching items remain navigable", () => {
    const matched = getVisibleShortcutIds(sampleItems, "All", "knight", {
      Games: true,
      Apps: true,
      Tools: true,
    });
    expect(matched).toEqual(["game-1"]);
  });

  it("filters to the active single category when not All", () => {
    expect(
      getVisibleShortcutIds(sampleItems, "Apps", "", {
        Games: false,
        Apps: true,
        Tools: false,
      }),
    ).toEqual(["app-pinned", "app-unpinned"]);
  });
});

describe("filterAndRankShortcuts", () => {
  const items: DockItem[] = [
    makeItem({
      id: "path-hit",
      name: "Browser",
      description: "Web client",
      path: "C:\\Apps\\code\\browser.exe",
      category: "Apps",
    }),
    makeItem({
      id: "desc-hit",
      name: "Editor",
      description: "Code editor for Rust",
      path: "C:\\Apps\\editor.exe",
      category: "Apps",
    }),
    makeItem({
      id: "substr-hit",
      name: "Unicode Helper",
      category: "Tools",
    }),
    makeItem({
      id: "word-hit",
      name: "Visual Code Studio",
      category: "Apps",
    }),
    makeItem({
      id: "prefix-hit",
      name: "CodeSpaces",
      category: "Apps",
    }),
    makeItem({
      id: "exact-hit",
      name: "Code",
      category: "Apps",
    }),
  ];

  it("returns items in original order when query is blank", () => {
    expect(filterAndRankShortcuts(items, "   ").map((i) => i.id)).toEqual(
      items.map((i) => i.id),
    );
  });

  it("ranks exact > prefix > word boundary > substring > description > path", () => {
    const ranked = filterAndRankShortcuts(items, "code").map((i) => i.id);
    expect(ranked).toEqual([
      "exact-hit",
      "prefix-hit",
      "word-hit",
      "substr-hit",
      "desc-hit",
      "path-hit",
    ]);
  });
});

describe("resolveHighlightAfterListChange", () => {
  it("preserves currentId when still visible, picks first during search, and clears otherwise", () => {
    expect(resolveHighlightAfterListChange([], "a", true)).toBeNull();
    expect(resolveHighlightAfterListChange(["a", "b"], "b", false)).toBe("b");
    expect(resolveHighlightAfterListChange(["a", "b"], "b", true)).toBe("b");
    expect(resolveHighlightAfterListChange(["a", "b"], null, true)).toBe("a");
    expect(resolveHighlightAfterListChange(["a", "b"], "missing", true)).toBe("a");
    expect(resolveHighlightAfterListChange(["a", "b"], "missing", false)).toBeNull();
  });
});

describe("cycleCategory and categoryForShortcutDigit", () => {
  it("cycles through All -> Games -> Apps -> Tools symmetrically", () => {
    expect(cycleCategory("All", "next")).toBe("Games");
    expect(cycleCategory("Tools", "next")).toBe("All");
    expect(cycleCategory("All", "prev")).toBe("Tools");
    expect(cycleCategory("Games", "prev")).toBe("All");
  });

  it("maps Ctrl+1..4 digits to categories cleanly", () => {
    expect(categoryForShortcutDigit("1")).toBe("All");
    expect(categoryForShortcutDigit("2")).toBe("Games");
    expect(categoryForShortcutDigit("3")).toBe("Apps");
    expect(categoryForShortcutDigit("4")).toBe("Tools");
    expect(categoryForShortcutDigit("5")).toBeNull();
  });
});
