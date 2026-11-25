import type {
  Category,
  CollapseRequestPayload,
  DockConfig,
  DockEdge,
  DockItem,
  HarborIpc,
  MonitorDescriptor,
  WindowLayoutState,
} from "../types/dock";

const DEFAULT_MOCK_ITEMS: DockItem[] = [
  {
    id: "mock-steam-1145360",
    name: "Hades",
    path: "steam://rungameid/1145360",
    arguments: "",
    description: "Steam Game",
    category: "Games",
    custom_icon_path: null,
    icon_source_path: "C:\\Games\\Steam\\steamapps\\common\\Hades\\Hades.exe",
    run_as_admin: false,
    pinned: true,
    source: "Steam",
    steam_app_id: "1145360",
    launch_count: 14,
  },
  {
    id: "mock-steam-730",
    name: "Counter-Strike 2",
    path: "steam://rungameid/730",
    arguments: "-novid",
    description: "Steam Game",
    category: "Games",
    custom_icon_path: null,
    icon_source_path:
      "C:\\Games\\Steam\\steamapps\\common\\Counter-Strike Global Offensive\\game\\bin\\win64\\cs2.exe",
    run_as_admin: false,
    pinned: false,
    source: "Steam",
    steam_app_id: "730",
    launch_count: 29,
  },
  {
    id: "mock-app-vscode",
    name: "Visual Studio Code",
    path: "C:\\Users\\Demo\\AppData\\Local\\Programs\\Microsoft VS Code\\Code.exe",
    arguments: "",
    description: "Code Editor",
    category: "Apps",
    custom_icon_path: null,
    icon_source_path: null,
    run_as_admin: false,
    pinned: true,
    source: "StartMenu",
    steam_app_id: null,
    launch_count: 42,
  },
  {
    id: "mock-app-firefox",
    name: "Mozilla Firefox",
    path: "C:\\Program Files\\Mozilla Firefox\\firefox.exe",
    arguments: "",
    description: "Web Browser",
    category: "Apps",
    custom_icon_path: null,
    icon_source_path: null,
    run_as_admin: false,
    pinned: false,
    source: "StartMenu",
    steam_app_id: null,
    launch_count: 19,
  },
  {
    id: "mock-tool-wt",
    name: "Terminal",
    path: "C:\\Users\\Demo\\AppData\\Local\\Microsoft\\WindowsApps\\wt.exe",
    arguments: "",
    description: "Windows Terminal",
    category: "Tools",
    custom_icon_path: null,
    icon_source_path: null,
    run_as_admin: true,
    pinned: true,
    source: "System",
    steam_app_id: null,
    launch_count: 33,
  },
  {
    id: "mock-tool-taskmgr",
    name: "Task Manager",
    path: "C:\\Windows\\System32\\taskmgr.exe",
    arguments: "",
    description: "Windows Task Manager",
    category: "Tools",
    custom_icon_path: null,
    icon_source_path: null,
    run_as_admin: false,
    pinned: false,
    source: "System",
    steam_app_id: null,
    launch_count: 8,
  },
];

const MOCK_MONITORS: MonitorDescriptor[] = [
  {
    name: "\\\\.\\DISPLAY1",
    label: "Display 1 (Primary) — 2560x1440 (100%)",
    width: 2560,
    height: 1440,
    scale_factor: 1.0,
    is_primary: true,
  },
  {
    name: "\\\\.\\DISPLAY2",
    label: "Display 2 — 1920x1080 (125%)",
    width: 1920,
    height: 1080,
    scale_factor: 1.25,
    is_primary: false,
  },
];

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

export function createMockIpc(): HarborIpc {
  let config: DockConfig = {
    schema_version: 1,
    edge: "Left",
    monitor_name: "\\\\.\\DISPLAY1",
    top_position: 140,
    auto_collapse_on_launch: true,
    auto_collapse_on_blur: true,
    always_on_top: true,
    start_with_windows: false,
    global_hotkey: "Alt+Shift+Space",
    selected_category: "All",
    items: clone(DEFAULT_MOCK_ITEMS),
    dismissed: [],
  };

  let expanded = true;
  let pinned = false;
  let epoch = 1;
  let idSeq = 100;
  let hotkeyWarning: string | null = null;

  const layoutListeners = new Set<(layout: WindowLayoutState) => void>();
  const collapseListeners = new Set<(payload: CollapseRequestPayload) => void>();
  const warningListeners = new Set<(warning: string | null) => void>();
  const dropHoverListeners = new Set<(paths: string[]) => void>();
  const dropLeaveListeners = new Set<() => void>();
  const dropListeners = new Set<(paths: string[]) => void>();

  function buildLayout(): WindowLayoutState {
    const dockHeight = 640;
    const clampedTop = Math.min(Math.max(config.top_position, 8), 840);
    const desiredDockTop = clampedTop + 42 - dockHeight / 2;
    const expandedTop = Math.min(Math.max(desiredDockTop, 8), 1440 - dockHeight - 8);
    return {
      expanded,
      edge: config.edge,
      dock_height: dockHeight,
      handle_offset_y: clampedTop - expandedTop,
      top_position: clampedTop,
      pinned,
    };
  }

  function emitLayout(): WindowLayoutState {
    const layout = buildLayout();
    for (const listener of layoutListeners) {
      listener(layout);
    }
    return layout;
  }

  return {
    toAssetUrl: (filePath) =>
      filePath && (filePath.startsWith("data:") || filePath.startsWith("http"))
        ? filePath
        : null,
    getInitialState: async () => ({
      config: clone(config),
      layout: buildLayout(),
      monitors: clone(MOCK_MONITORS),
      hotkey_warning: hotkeyWarning,
      is_portable: false,
    }),
    listMonitors: async () => clone(MOCK_MONITORS),
    expandDock: async () => {
      epoch += 1;
      expanded = true;
      return emitLayout();
    },
    requestCollapseDock: async () => {
      if (!expanded) {
        return buildLayout();
      }
      epoch += 1;
      const currentEpoch = epoch;
      for (const listener of collapseListeners) {
        listener({ epoch: currentEpoch });
      }
      return buildLayout();
    },
    commitCollapseWindow: async (expectedEpoch: number) => {
      if (expectedEpoch !== epoch) {
        return false;
      }
      expanded = false;
      emitLayout();
      return true;
    },
    toggleDock: async () => {
      if (expanded) {
        epoch += 1;
        for (const listener of collapseListeners) {
          listener({ epoch });
        }
        return buildLayout();
      }
      expanded = true;
      return emitLayout();
    },
    setPinned: async (nextPinned: boolean) => {
      pinned = nextPinned;
      return emitLayout();
    },
    setEdge: async (edge: DockEdge) => {
      config.edge = edge;
      return emitLayout();
    },
    setSelectedCategory: async (category: string) => {
      config.selected_category = category;
      return clone(config);
    },
    startHandleDrag: async () => {},
    endHandleDrag: async () => ({
      moved: false,
      top_position: config.top_position,
    }),
    launchItem: async (id: string) => {
      config.items = config.items.map((item) =>
        item.id === id
          ? { ...item, launch_count: item.launch_count + 1 }
          : item,
      );
      if (config.auto_collapse_on_launch && !pinned) {
        epoch += 1;
        for (const listener of collapseListeners) {
          listener({ epoch });
        }
      }
      return clone(config);
    },
    launchSystemTool: async () => {},
    openFileLocation: async () => {},
    getItemIcon: async () => null,
    addItem: async (input) => {
      idSeq += 1;
      config.items.push({
        id: `mock-man-${idSeq}`,
        name: input.name,
        path: input.path,
        arguments: input.arguments,
        description: input.description,
        category: input.category,
        custom_icon_path: input.custom_icon_path,
        icon_source_path: null,
        run_as_admin: input.run_as_admin,
        pinned: input.pinned,
        source: "Manual",
        steam_app_id: null,
        launch_count: 0,
      });
      return clone(config);
    },
    updateItem: async (input) => {
      config.items = config.items.map((item) =>
        item.id === input.id
          ? {
              ...item,
              name: input.name,
              path: item.source === "Steam" ? item.path : input.path,
              arguments: input.arguments,
              description: input.description,
              category: input.category,
              custom_icon_path: input.custom_icon_path,
              run_as_admin: input.run_as_admin,
              pinned: input.pinned,
            }
          : item,
      );
      return clone(config);
    },
    removeItem: async (id: string) => {
      const target = config.items.find((i) => i.id === id);
      if (target) {
        const key =
          target.source === "Steam" && target.steam_app_id
            ? `steam:${target.steam_app_id}`
            : target.path.toLowerCase();
        if (!config.dismissed.includes(key)) {
          config.dismissed.push(key);
        }
      }
      config.items = config.items.filter((i) => i.id !== id);
      return clone(config);
    },
    toggleItemPin: async (id: string) => {
      config.items = config.items.map((i) =>
        i.id === id ? { ...i, pinned: !i.pinned } : i,
      );
      return clone(config);
    },
    moveItemCategory: async (id: string, category: Category) => {
      config.items = config.items.map((i) =>
        i.id === id ? { ...i, category } : i,
      );
      return clone(config);
    },
    reorderItems: async (
      category: Category,
      isPinned: boolean,
      orderedIds: string[],
    ) => {
      const matchingIndices = config.items
        .map((item, idx) =>
          item.category === category && item.pinned === isPinned ? idx : -1,
        )
        .filter((idx) => idx >= 0);
      const pool = new Map<string, DockItem>();
      for (const idx of matchingIndices) {
        const item = config.items[idx];
        if (item) {
          pool.set(item.id, item);
        }
      }
      const nextGroup: DockItem[] = [];
      for (const id of orderedIds) {
        const found = pool.get(id);
        if (found) {
          nextGroup.push(found);
          pool.delete(id);
        }
      }
      for (const remaining of pool.values()) {
        nextGroup.push(remaining);
      }
      matchingIndices.forEach((slotIdx, i) => {
        const nextItem = nextGroup[i];
        if (nextItem) {
          config.items[slotIdx] = nextItem;
        }
      });
      return clone(config);
    },
    rescanShortcuts: async () => ({
      rescan: { added: 0, missing_ids: [] },
      config: clone(config),
    }),
    resetToDefaults: async () => {
      config.dismissed = [];
      config.items = clone(DEFAULT_MOCK_ITEMS);
      return clone(config);
    },
    dismissMissingItems: async (ids: string[]) => {
      const idSet = new Set(ids);
      config.items = config.items.filter((i) => !idSet.has(i.id));
      return clone(config);
    },
    updateSettings: async (settings) => {
      config = {
        ...config,
        ...settings,
      };
      hotkeyWarning = null;
      for (const listener of warningListeners) {
        listener(null);
      }
      return {
        config: clone(config),
        layout: emitLayout(),
        hotkey_warning: hotkeyWarning,
      };
    },
    pickExecutableFile: async () => "C:\\Windows\\System32\\notepad.exe",
    pickIconFile: async () => null,
    pinDroppedFiles: async (paths, category) => {
      let added = 0;
      for (const p of paths) {
        idSeq += 1;
        config.items.push({
          id: `mock-drop-${idSeq}`,
          name: p.split(/[\\/]/).pop() ?? "Dropped App",
          path: p,
          arguments: "",
          description: "Dropped Shortcut",
          category,
          custom_icon_path: null,
          icon_source_path: null,
          run_as_admin: false,
          pinned: true,
          source: "Manual",
          steam_app_id: null,
          launch_count: 0,
        });
        added += 1;
      }
      return { added, skipped: 0, config: clone(config) };
    },
    onLayoutChanged: async (handler) => {
      layoutListeners.add(handler);
      return () => layoutListeners.delete(handler);
    },
    onRequestCollapse: async (handler) => {
      collapseListeners.add(handler);
      return () => collapseListeners.delete(handler);
    },
    onHotkeyWarning: async (handler) => {
      warningListeners.add(handler);
      return () => warningListeners.delete(handler);
    },
    onFileDropHover: async (handler) => {
      dropHoverListeners.add(handler);
      return () => dropHoverListeners.delete(handler);
    },
    onFileDropLeave: async (handler) => {
      dropLeaveListeners.add(handler);
      return () => dropLeaveListeners.delete(handler);
    },
    onFileDrop: async (handler) => {
      dropListeners.add(handler);
      return () => dropListeners.delete(handler);
    },
  };
}
