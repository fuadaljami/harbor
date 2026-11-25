import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AddEditModal } from "./components/AddEditModal";
import { BottomControlBar } from "./components/BottomControlBar";
import { CategoryRail, type RailCategory } from "./components/CategoryRail";
import { ContextMenu } from "./components/ContextMenu";
import { DockHeader } from "./components/DockHeader";
import { EdgeHandle } from "./components/EdgeHandle";
import { SearchBar } from "./components/SearchBar";
import { ShortcutList } from "./components/ShortcutList";
import { SettingsModal } from "./components/SettingsModal";
import {
  ToastContainer,
  type ToastMessage,
  type ToastVariant,
} from "./components/ToastContainer";
import { useIconStore } from "./hooks/useIconStore";
import { useKeyboardNav } from "./hooks/useKeyboardNav";
import { DockBoundsProvider, type DockHorizontalBounds } from "./lib/dockBounds";
import { ipc, isTauriRuntime, parseAppError } from "./lib/ipc";
import { getVisibleShortcutIds } from "./lib/keyboard";
import type {
  Category,
  DockConfig,
  DockItem,
  MonitorDescriptor,
  NewItemInput,
  SettingsUpdate,
  SystemToolKind,
  UpdateItemInput,
  WindowLayoutState,
} from "./types/dock";

interface ContextMenuState {
  item: DockItem;
  x: number;
  y: number;
}

const VALID_CATEGORIES = new Set<RailCategory>([
  "All",
  "Games",
  "Apps",
  "Tools",
]);

function normalizeSelectedCategory(raw: string): RailCategory {
  return VALID_CATEGORIES.has(raw as RailCategory)
    ? (raw as RailCategory)
    : "All";
}

function sortGroupItems(items: DockItem[], category: Category): DockItem[] {
  const inCat = items.filter((i) => i.category === category);
  return [...inCat.filter((i) => i.pinned), ...inCat.filter((i) => !i.pinned)];
}

export default function App() {
  const [config, setConfig] = useState<DockConfig | null>(null);
  const [layout, setLayout] = useState<WindowLayoutState | null>(null);
  const [monitors, setMonitors] = useState<MonitorDescriptor[]>([]);
  const [hotkeyWarning, setHotkeyWarning] = useState<string | null>(null);
  const [isPortable, setIsPortable] = useState<boolean>(false);

  const [searchQuery, setSearchQuery] = useState<string>("");
  const [focusedItemId, setFocusedItemId] = useState<string | null>(null);
  const [collapsedSections, setCollapsedSections] = useState<
    Record<Category, boolean>
  >({
    Games: false,
    Apps: false,
    Tools: false,
  });
  const [isRescanning, setIsRescanning] = useState<boolean>(false);
  const [isDropHover, setIsDropHover] = useState<boolean>(false);

  const [isClosing, setIsClosing] = useState<boolean>(false);
  const [panelVisible, setPanelVisible] = useState<boolean>(false);

  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);
  const [addEditOpen, setAddEditOpen] = useState<boolean>(false);
  const [editingItem, setEditingItem] = useState<DockItem | null>(null);
  const [settingsOpen, setSettingsOpen] = useState<boolean>(false);
  const [hotkeyRecording, setHotkeyRecording] = useState<boolean>(false);
  const [toasts, setToasts] = useState<ToastMessage[]>([]);

  const dockShellRef = useRef<HTMLDivElement>(null);
  const layoutRef = useRef<WindowLayoutState | null>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const listScrollRef = useRef<HTMLDivElement>(null);
  const cardRefs = useRef<Map<string, HTMLDivElement>>(new Map());
  const pendingCollapseEpochRef = useRef<number | null>(null);
  const collapseFallbackTimerRef = useRef<number | null>(null);
  const selectedCategoryRef = useRef<RailCategory>("All");
  const toastSeqRef = useRef<number>(0);

  const pushToast = useCallback(
    (variant: ToastVariant, title: string, detail?: string) => {
      toastSeqRef.current += 1;
      const id = `toast-${toastSeqRef.current}`;
      setToasts((prev) => [...prev.slice(-3), { id, variant, title, detail }]);
      const ttl = variant === "error" || variant === "warning" ? 5500 : 3500;
      window.setTimeout(() => {
        setToasts((prev) => prev.filter((t) => t.id !== id));
      }, ttl);
    },
    [],
  );

  const dismissToast = useCallback((id: string) => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, []);

  const handleUnknownError = useCallback(
    (err: unknown) => {
      const parsed = parseAppError(err);
      if (parsed.kind === "launch_cancelled") {
        pushToast("info", "Launch cancelled");
      } else {
        pushToast("error", "Operation failed", parsed.message);
      }
    },
    [pushToast],
  );

  const clearCollapseFallbackTimer = useCallback(() => {
    if (collapseFallbackTimerRef.current !== null) {
      window.clearTimeout(collapseFallbackTimerRef.current);
      collapseFallbackTimerRef.current = null;
    }
  }, []);

  const applyLayoutState = useCallback(
    (nextLayout: WindowLayoutState) => {
      const prev = layoutRef.current;
      const becameExpanded = !prev?.expanded && nextLayout.expanded;
      layoutRef.current = nextLayout;
      setLayout(nextLayout);

      if (nextLayout.expanded) {
        pendingCollapseEpochRef.current = null;
        clearCollapseFallbackTimer();
        setIsClosing(false);
        if (becameExpanded || !prev) {
          setPanelVisible(false);
          requestAnimationFrame(() => {
            setPanelVisible(true);
          });
        } else {
          setPanelVisible(true);
        }
      } else {
        pendingCollapseEpochRef.current = null;
        clearCollapseFallbackTimer();
        setIsClosing(false);
        setPanelVisible(false);
        setContextMenu(null);
      }
    },
    [clearCollapseFallbackTimer],
  );

  const commitCollapseOnce = useCallback(
    async (epoch: number) => {
      if (pendingCollapseEpochRef.current !== epoch) {
        return;
      }
      pendingCollapseEpochRef.current = null;
      clearCollapseFallbackTimer();
      try {
        await ipc.commitCollapseWindow(epoch);
      } catch (err) {
        handleUnknownError(err);
      }
    },
    [clearCollapseFallbackTimer, handleUnknownError],
  );

  const { getIconUrl, requestIcon, invalidateIcon } = useIconStore(
    config?.items ?? [],
  );

  const selectedCategory: RailCategory = config
    ? normalizeSelectedCategory(config.selected_category)
    : "All";

  useEffect(() => {
    selectedCategoryRef.current = selectedCategory;
  }, [selectedCategory]);

  // Native WebView2 context menu & production shortcut hardening.
  useEffect(() => {
    const handleContextMenu = (e: MouseEvent) => {
      const target = e.target;
      if (
        target instanceof HTMLElement &&
        (target.tagName === "INPUT" ||
          target.tagName === "TEXTAREA" ||
          target.isContentEditable)
      ) {
        return;
      }
      // Prevent WebView2 native menu without calling stopPropagation so ShortcutCard still opens ContextMenu.
      e.preventDefault();
    };

    window.addEventListener("contextmenu", handleContextMenu);

    if (import.meta.env.DEV) {
      return () => {
        window.removeEventListener("contextmenu", handleContextMenu);
      };
    }

    const handleProdKeyDown = (e: KeyboardEvent) => {
      const lower = e.key.toLowerCase();
      const ctrlOrMeta = e.ctrlKey || e.metaKey;
      if (
        e.key === "F5" ||
        (ctrlOrMeta && (lower === "r" || lower === "p")) ||
        (ctrlOrMeta &&
          (e.key === "+" || e.key === "=" || e.key === "-" || e.key === "0"))
      ) {
        e.preventDefault();
      }
    };

    const handleProdWheel = (e: WheelEvent) => {
      if (e.ctrlKey) {
        e.preventDefault();
      }
    };

    window.addEventListener("keydown", handleProdKeyDown);
    window.addEventListener("wheel", handleProdWheel, { passive: false });
    return () => {
      window.removeEventListener("contextmenu", handleContextMenu);
      window.removeEventListener("keydown", handleProdKeyDown);
      window.removeEventListener("wheel", handleProdWheel);
    };
  }, []);

  // Load initial state and subscribe to Rust window/drop/hotkey events.
  useEffect(() => {
    let active = true;
    const unlisteners: Array<() => void> = [];

    const init = async () => {
      try {
        const initial = await ipc.getInitialState();
        if (!active) return;

        setConfig(initial.config);
        applyLayoutState(initial.layout);
        setMonitors(initial.monitors);
        setHotkeyWarning(initial.hotkey_warning);
        setIsPortable(initial.is_portable);

        if (initial.hotkey_warning) {
          pushToast("warning", "Global Hotkey Collision", initial.hotkey_warning);
        }

        unlisteners.push(
          await ipc.onLayoutChanged((nextLayout) => {
            if (!active) return;
            applyLayoutState(nextLayout);
          }),
        );

        unlisteners.push(
          await ipc.onRequestCollapse(({ epoch }) => {
            clearCollapseFallbackTimer();
            pendingCollapseEpochRef.current = epoch;
            setContextMenu(null);
            setIsClosing(true);
            setPanelVisible(false);
            collapseFallbackTimerRef.current = window.setTimeout(() => {
              void commitCollapseOnce(epoch);
            }, 165);
          }),
        );

        unlisteners.push(
          await ipc.onHotkeyWarning((warning) => {
            setHotkeyWarning(warning);
            if (warning) {
              pushToast("warning", "Global Hotkey Unavailable", warning);
            }
          }),
        );

        unlisteners.push(
          await ipc.onFileDropHover(() => {
            setIsDropHover(true);
          }),
        );

        unlisteners.push(
          await ipc.onFileDropLeave(() => {
            setIsDropHover(false);
          }),
        );

        unlisteners.push(
          await ipc.onFileDrop(async (paths) => {
            setIsDropHover(false);
            if (paths.length === 0) return;
            const targetCat: Category =
              selectedCategoryRef.current === "All"
                ? "Apps"
                : selectedCategoryRef.current;
            try {
              const res = await ipc.pinDroppedFiles(paths, targetCat);
              setConfig(res.config);
              if (res.added > 0) {
                pushToast(
                  "success",
                  `Pinned ${res.added} ${res.added === 1 ? "shortcut" : "shortcuts"} to ${targetCat}`,
                );
              } else if (res.skipped > 0) {
                pushToast(
                  "info",
                  "No new shortcuts added",
                  "Dropped items were already pinned or invalid.",
                );
              }
            } catch (err) {
              handleUnknownError(err);
            }
          }),
        );
      } catch (err) {
        if (active) {
          handleUnknownError(err);
        }
      }
    };

    void init();
    return () => {
      active = false;
      clearCollapseFallbackTimer();
      for (const unlisten of unlisteners) {
        unlisten();
      }
    };
  }, [
    applyLayoutState,
    clearCollapseFallbackTimer,
    commitCollapseOnce,
    handleUnknownError,
    pushToast,
  ]);

  const counts = useMemo(() => {
    const items = config?.items ?? [];
    return {
      All: items.length,
      Games: items.filter((i) => i.category === "Games").length,
      Apps: items.filter((i) => i.category === "Apps").length,
      Tools: items.filter((i) => i.category === "Tools").length,
    } satisfies Record<RailCategory, number>;
  }, [config?.items]);

  const registerCardRef = useCallback(
    (id: string, el: HTMLDivElement | null) => {
      if (el) {
        cardRefs.current.set(id, el);
      } else {
        cardRefs.current.delete(id);
      }
    },
    [],
  );

  const handleExpand = useCallback(async () => {
    try {
      await ipc.expandDock();
    } catch (err) {
      handleUnknownError(err);
    }
  }, [handleUnknownError]);

  const handleRequestCollapse = useCallback(async () => {
    try {
      await ipc.requestCollapseDock();
    } catch (err) {
      handleUnknownError(err);
    }
  }, [handleUnknownError]);

  const handleSelectCategory = useCallback(
    async (category: RailCategory) => {
      setFocusedItemId(null);
      setConfig((prev) =>
        prev ? { ...prev, selected_category: category } : prev,
      );
      try {
        const next = await ipc.setSelectedCategory(category);
        setConfig(next);
      } catch (err) {
        handleUnknownError(err);
      }
    },
    [handleUnknownError],
  );

  const handleTogglePin = useCallback(async () => {
    if (!layout) return;
    try {
      await ipc.setPinned(!layout.pinned);
    } catch (err) {
      handleUnknownError(err);
    }
  }, [layout, handleUnknownError]);

  const handleToggleEdge = useCallback(async () => {
    if (!layout) return;
    const nextEdge = layout.edge === "Left" ? "Right" : "Left";
    try {
      const nextLayout = await ipc.setEdge(nextEdge);
      setConfig((prev) => (prev ? { ...prev, edge: nextEdge } : prev));
      applyLayoutState(nextLayout);
    } catch (err) {
      handleUnknownError(err);
    }
  }, [layout, applyLayoutState, handleUnknownError]);

  const handleRescan = useCallback(async (): Promise<string[]> => {
    setIsRescanning(true);
    try {
      const res = await ipc.rescanShortcuts();
      setConfig(res.config);
      const added = res.rescan.added;
      const missingCount = res.rescan.missing_ids.length;
      if (added > 0) {
        pushToast(
          "success",
          `Discovered ${added} new ${added === 1 ? "shortcut" : "shortcuts"}`,
          missingCount > 0
            ? `${missingCount} missing entries can be reviewed in Settings.`
            : undefined,
        );
      } else {
        pushToast(
          "info",
          "System scan complete",
          missingCount > 0
            ? `${missingCount} missing entries found (review in Settings).`
            : "All shortcuts are up to date.",
        );
      }
      return res.rescan.missing_ids;
    } catch (err) {
      handleUnknownError(err);
      return [];
    } finally {
      setIsRescanning(false);
    }
  }, [handleUnknownError, pushToast]);

  const handleLaunch = useCallback(
    async (item: DockItem, runAsAdmin: boolean) => {
      try {
        const nextCfg = await ipc.launchItem(item.id, runAsAdmin);
        setConfig(nextCfg);
      } catch (err) {
        handleUnknownError(err);
      }
    },
    [handleUnknownError],
  );

  const handleSystemTool = useCallback(
    async (tool: SystemToolKind) => {
      try {
        await ipc.launchSystemTool(tool);
      } catch (err) {
        handleUnknownError(err);
      }
    },
    [handleUnknownError],
  );

  const handleToggleItemPin = useCallback(
    async (item: DockItem) => {
      try {
        const next = await ipc.toggleItemPin(item.id);
        setConfig(next);
      } catch (err) {
        handleUnknownError(err);
      }
    },
    [handleUnknownError],
  );

  const handleOpenLocation = useCallback(
    async (item: DockItem) => {
      try {
        await ipc.openFileLocation(item.id);
      } catch (err) {
        handleUnknownError(err);
      }
    },
    [handleUnknownError],
  );

  const handleMoveCategory = useCallback(
    async (item: DockItem, nextCategory: Category) => {
      try {
        const next = await ipc.moveItemCategory(item.id, nextCategory);
        setConfig(next);
        pushToast("info", `Moved ${item.name} to ${nextCategory}`);
      } catch (err) {
        handleUnknownError(err);
      }
    },
    [handleUnknownError, pushToast],
  );

  const handleReorderGroup = useCallback(
    async (category: Category, pinned: boolean, orderedIds: string[]) => {
      try {
        const next = await ipc.reorderItems(category, pinned, orderedIds);
        setConfig(next);
      } catch (err) {
        handleUnknownError(err);
      }
    },
    [handleUnknownError],
  );

  const handleMoveStep = useCallback(
    async (item: DockItem, direction: "up" | "down") => {
      if (!config) return;
      const subgroup = config.items.filter(
        (i) => i.category === item.category && i.pinned === item.pinned,
      );
      const idx = subgroup.findIndex((i) => i.id === item.id);
      if (idx < 0) return;
      const targetIdx = direction === "up" ? idx - 1 : idx + 1;
      if (targetIdx < 0 || targetIdx >= subgroup.length) return;
      const ids = subgroup.map((i) => i.id);
      const moved = ids.splice(idx, 1)[0];
      if (!moved) return;
      ids.splice(targetIdx, 0, moved);
      await handleReorderGroup(item.category, item.pinned, ids);
    },
    [config, handleReorderGroup],
  );

  const handleRemoveItem = useCallback(
    async (item: DockItem) => {
      try {
        const next = await ipc.removeItem(item.id);
        setConfig(next);
        pushToast("info", `Removed ${item.name}`);
      } catch (err) {
        handleUnknownError(err);
      }
    },
    [handleUnknownError, pushToast],
  );

  const handleSaveNewItem = useCallback(
    async (input: NewItemInput) => {
      const next = await ipc.addItem(input);
      setConfig(next);
      pushToast("success", `Added ${input.name}`);
    },
    [pushToast],
  );

  const handleSaveEditItem = useCallback(
    async (input: UpdateItemInput) => {
      const next = await ipc.updateItem(input);
      invalidateIcon(input.id);
      setConfig(next);
      pushToast("success", `Updated ${input.name}`);
    },
    [invalidateIcon, pushToast],
  );

  const handleSaveSettings = useCallback(
    async (settings: SettingsUpdate) => {
      const res = await ipc.updateSettings(settings);
      setConfig(res.config);
      applyLayoutState(res.layout);
      setHotkeyWarning(res.hotkey_warning);
      pushToast("success", "Settings saved");
    },
    [applyLayoutState, pushToast],
  );

  const handleDismissMissing = useCallback(
    async (ids: string[]) => {
      const next = await ipc.dismissMissingItems(ids);
      setConfig(next);
      pushToast(
        "info",
        `Removed ${ids.length} missing ${ids.length === 1 ? "entry" : "entries"}`,
      );
    },
    [pushToast],
  );

  const handleResetDefaults = useCallback(async () => {
    const next = await ipc.resetToDefaults();
    setConfig(next);
    pushToast("success", "Restored discovered defaults");
  }, [pushToast]);

  const handleToggleSection = useCallback((category: Category) => {
    setCollapsedSections((prev) => ({
      ...prev,
      [category]: !prev[category],
    }));
  }, []);

  const itemsById = useMemo(() => {
    const map = new Map<string, DockItem>();
    for (const item of config?.items ?? []) {
      map.set(item.id, item);
    }
    return map;
  }, [config?.items]);

  const visibleShortcutIds = useMemo(
    () =>
      getVisibleShortcutIds(
        config?.items ?? [],
        selectedCategory,
        searchQuery,
        collapsedSections,
      ),
    [config?.items, selectedCategory, searchQuery, collapsedSections],
  );

  const handleCloseActiveModal = useCallback(() => {
    if (addEditOpen) {
      setAddEditOpen(false);
      setEditingItem(null);
    } else if (settingsOpen) {
      setSettingsOpen(false);
    }
  }, [addEditOpen, settingsOpen]);

  const handleClearSearch = useCallback(() => {
    setSearchQuery("");
    setFocusedItemId(null);
  }, []);

  useKeyboardNav({
    expanded: Boolean(layout?.expanded),
    isClosing,
    itemsById,
    visibleIds: visibleShortcutIds,
    highlightedId: focusedItemId,
    selectedCategory,
    searchQuery,
    modalOpen: addEditOpen || settingsOpen,
    contextMenuOpen: contextMenu !== null,
    hotkeyRecorderActive: hotkeyRecording,
    searchInputRef,
    cardRefs,
    onHighlightChange: setFocusedItemId,
    onSelectCategory: (cat) => void handleSelectCategory(cat),
    onClearSearch: handleClearSearch,
    onCollapseDock: () => void handleRequestCollapse(),
    onCloseContextMenu: () => setContextMenu(null),
    onCloseModal: handleCloseActiveModal,
    onLaunchItem: (item, runAsAdmin) => void handleLaunch(item, runAsAdmin),
  });

  const getDockHorizontalBounds = useCallback((): DockHorizontalBounds => {
    if (isTauriRuntime()) {
      return { minX: 0, maxX: 380 };
    }
    const el = dockShellRef.current;
    if (el) {
      const rect = el.getBoundingClientRect();
      if (rect.width > 0) {
        return { minX: rect.left, maxX: rect.right };
      }
    }
    return { minX: 0, maxX: window.innerWidth };
  }, []);

  const contextSubgroup = useMemo(() => {
    if (!contextMenu || !config) {
      return { canMoveUp: false, canMoveDown: false };
    }
    const group = sortGroupItems(config.items, contextMenu.item.category).filter(
      (i) => i.pinned === contextMenu.item.pinned,
    );
    const idx = group.findIndex((i) => i.id === contextMenu.item.id);
    return {
      canMoveUp: idx > 0,
      canMoveDown: idx >= 0 && idx < group.length - 1,
    };
  }, [contextMenu, config]);

  if (!config || !layout) {
    return <div className="w-full h-full bg-transparent" />;
  }

  const isRight = layout.edge === "Right";
  const inBrowserPreview = !isTauriRuntime();

  const collapsedHandle = (
    <EdgeHandle
      expanded={false}
      edge={layout.edge}
      onExpand={() => void handleExpand()}
      onCollapse={() => void handleRequestCollapse()}
      onError={handleUnknownError}
    />
  );

  if (!layout.expanded && !isClosing) {
    if (inBrowserPreview) {
      return (
        <div className="w-full h-full bg-[#0E0F12]/90 relative overflow-hidden">
          <div
            style={{ top: `${Math.round(layout.top_position)}px` }}
            className={`absolute ${isRight ? "right-0" : "left-0"}`}
          >
            {collapsedHandle}
          </div>
        </div>
      );
    }
    return <div className="w-[24px] h-[84px] bg-transparent">{collapsedHandle}</div>;
  }

  const slideTransformClass =
    panelVisible && !isClosing
      ? "opacity-100 translate-x-0"
      : isRight
        ? "opacity-0 translate-x-full"
        : "opacity-0 -translate-x-full";

  const expandedContent = (
    <DockBoundsProvider value={getDockHorizontalBounds}>
      <div
        style={
          inBrowserPreview
            ? { height: `${Math.round(layout.dock_height)}px` }
            : undefined
        }
        className={`w-[380px] ${inBrowserPreview ? "" : "h-full"} bg-transparent overflow-hidden`}
      >
        <div
          onTransitionEnd={(e) => {
            if (
              e.target === e.currentTarget &&
              (e.propertyName === "transform" || e.propertyName === "opacity") &&
              isClosing &&
              pendingCollapseEpochRef.current !== null
            ) {
              void commitCollapseOnce(pendingCollapseEpochRef.current);
            }
          }}
          className={`w-[380px] h-full transition-[transform,opacity] ease-discord ${
            isClosing ? "duration-[150ms]" : "duration-[160ms]"
          } ${slideTransformClass}`}
        >
          <div
            ref={dockShellRef}
            className={`relative w-[380px] h-full flex ${isRight ? "flex-row-reverse" : "flex-row"} bg-[#1E1F22] border border-[#1F2023] rounded-[12px] overflow-hidden`}
          >
            <CategoryRail
              edge={layout.edge}
              selectedCategory={selectedCategory}
              counts={counts}
              onSelectCategory={(cat) => void handleSelectCategory(cat)}
              onAddClick={() => {
                setEditingItem(null);
                setAddEditOpen(true);
              }}
            />

            <div className="flex-1 h-full bg-[#2B2D31] flex flex-col min-w-0 relative">
              <DockHeader
                selectedCategory={selectedCategory}
                itemCount={counts[selectedCategory]}
                pinned={layout.pinned}
                edge={layout.edge}
                isRescanning={isRescanning}
                onTogglePin={() => void handleTogglePin()}
                onRescan={() => void handleRescan()}
                onToggleEdge={() => void handleToggleEdge()}
                onCollapse={() => void handleRequestCollapse()}
              />

              <SearchBar
                query={searchQuery}
                inputRef={searchInputRef}
                onChange={(val) => {
                  setSearchQuery(val);
                  setFocusedItemId(null);
                }}
              />

              <ShortcutList
                items={config.items}
                selectedCategory={selectedCategory}
                searchQuery={searchQuery}
                focusedItemId={focusedItemId}
                collapsedSections={collapsedSections}
                isDropHover={isDropHover}
                scrollContainerRef={listScrollRef}
                getIconUrl={getIconUrl}
                onRequestIcon={requestIcon}
                onLaunch={(item, admin) => void handleLaunch(item, admin)}
                onOpenContextMenu={(item, x, y) =>
                  setContextMenu({ item, x, y })
                }
                onReorderGroup={(cat, pin, ids) =>
                  void handleReorderGroup(cat, pin, ids)
                }
                onToggleSection={handleToggleSection}
                onAddClick={() => {
                  setEditingItem(null);
                  setAddEditOpen(true);
                }}
                onClearSearch={handleClearSearch}
                registerCardRef={registerCardRef}
              />

              <BottomControlBar
                expanded={layout.expanded && !isClosing}
                hasHotkeyWarning={hotkeyWarning !== null}
                onSystemTool={(tool) => void handleSystemTool(tool)}
                onOpenSettings={() => setSettingsOpen(true)}
              />

              <ToastContainer toasts={toasts} onDismiss={dismissToast} />
            </div>

            <AddEditModal
              isOpen={addEditOpen}
              editingItem={editingItem}
              defaultCategory={
                selectedCategory === "All" ? "Apps" : selectedCategory
              }
              onClose={() => {
                setAddEditOpen(false);
                setEditingItem(null);
              }}
              onSaveNew={handleSaveNewItem}
              onSaveEdit={handleSaveEditItem}
            />

            <SettingsModal
              isOpen={settingsOpen}
              config={config}
              monitors={monitors}
              hotkeyWarning={hotkeyWarning}
              isPortable={isPortable}
              isRescanning={isRescanning}
              onClose={() => setSettingsOpen(false)}
              onRecordingChange={setHotkeyRecording}
              onSaveSettings={handleSaveSettings}
              onRescan={handleRescan}
              onDismissMissing={handleDismissMissing}
              onResetDefaults={handleResetDefaults}
            />
          </div>
        </div>

        {contextMenu && (
          <ContextMenu
            item={contextMenu.item}
            x={contextMenu.x}
            y={contextMenu.y}
            canMoveUp={contextSubgroup.canMoveUp}
            canMoveDown={contextSubgroup.canMoveDown}
            onClose={() => setContextMenu(null)}
            onLaunch={(item, admin) => void handleLaunch(item, admin)}
            onTogglePin={(item) => void handleToggleItemPin(item)}
            onEdit={(item) => {
              setEditingItem(item);
              setAddEditOpen(true);
            }}
            onOpenLocation={(item) => void handleOpenLocation(item)}
            onMoveCategory={(item, cat) => void handleMoveCategory(item, cat)}
            onMoveStep={(item, dir) => void handleMoveStep(item, dir)}
            onRemove={(item) => void handleRemoveItem(item)}
          />
        )}
      </div>
    </DockBoundsProvider>
  );

  if (inBrowserPreview) {
    return (
      <div className="w-full h-full bg-[#0E0F12]/90 relative overflow-hidden">
        <div
          className={`absolute top-3 ${isRight ? "right-0" : "left-0"}`}
        >
          {expandedContent}
        </div>
      </div>
    );
  }

  return expandedContent;
}
