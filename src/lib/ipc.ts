/**
 * Sole Tauri IPC boundary for Harbor.
 *
 * Serialization conventions:
 * - Tauri maps `camelCase` JS command argument keys to `snake_case` Rust parameters
 *   (e.g. `{ runAsAdmin }` -> `run_as_admin`, `{ orderedIds }` -> `ordered_ids`).
 * - Serialized Rust struct payload fields retain their exact `serde` names
 *   (predominantly `snake_case`, e.g. `top_position`, `steam_app_id`, `dock_height`).
 */

import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  AppErrorPayload,
  Category,
  CollapseRequestPayload,
  DockConfig,
  DockEdge,
  DragEndResult,
  DropPinResult,
  HarborIpc,
  InitialStatePayload,
  MonitorDescriptor,
  NewItemInput,
  RescanPayload,
  SettingsUpdate,
  SystemToolKind,
  UpdateItemInput,
  UpdateSettingsResult,
  WindowLayoutState,
} from "../types/dock";

export const EVENT_LAYOUT_CHANGED = "harbor://layout-changed";
export const EVENT_REQUEST_COLLAPSE = "harbor://request-collapse";
export const EVENT_HOTKEY_WARNING = "harbor://hotkey-warning";
export const EVENT_FILE_DROP_HOVER = "harbor://file-drop-hover";
export const EVENT_FILE_DROP_LEAVE = "harbor://file-drop-leave";
export const EVENT_FILE_DROP = "harbor://file-drop";

export function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function shouldUseBrowserMock(): boolean {
  return import.meta.env.DEV && !isTauriRuntime();
}

let cachedMockPromise: Promise<HarborIpc> | null = null;

async function getMock(): Promise<HarborIpc> {
  if (!cachedMockPromise) {
    cachedMockPromise = import("./mockIpc.ts").then((m) => m.createMockIpc());
  }
  return cachedMockPromise;
}

export function toAssetUrl(filePath: string | null): string | null {
  if (!filePath || !filePath.trim()) {
    return null;
  }
  if (isTauriRuntime()) {
    return convertFileSrc(filePath);
  }
  if (filePath.startsWith("data:") || filePath.startsWith("http")) {
    return filePath;
  }
  return null;
}

export function parseAppError(err: unknown): AppErrorPayload {
  if (typeof err === "object" && err !== null) {
    const record = err as Record<string, unknown>;
    if (
      typeof record.kind === "string" &&
      typeof record.message === "string"
    ) {
      return { kind: record.kind, message: record.message };
    }
    if (typeof record.message === "string") {
      return { kind: "unknown", message: record.message };
    }
  }
  if (typeof err === "string") {
    try {
      const parsed: unknown = JSON.parse(err);
      if (typeof parsed === "object" && parsed !== null) {
        const record = parsed as Record<string, unknown>;
        if (
          typeof record.kind === "string" &&
          typeof record.message === "string"
        ) {
          return { kind: record.kind, message: record.message };
        }
      }
    } catch {
      return { kind: "unknown", message: err };
    }
    return { kind: "unknown", message: err };
  }
  return { kind: "unknown", message: "An unexpected error occurred" };
}

const tauriIpc: HarborIpc = {
  toAssetUrl,
  getInitialState: () => invoke<InitialStatePayload>("get_initial_state"),
  listMonitors: () => invoke<MonitorDescriptor[]>("list_monitors"),
  expandDock: () => invoke<WindowLayoutState>("expand_dock"),
  requestCollapseDock: () =>
    invoke<WindowLayoutState>("request_collapse_dock"),
  commitCollapseWindow: (epoch: number) =>
    invoke<boolean>("commit_collapse_window", { epoch }),
  toggleDock: () => invoke<WindowLayoutState>("toggle_dock"),
  setPinned: (pinned: boolean) =>
    invoke<WindowLayoutState>("set_pinned", { pinned }),
  setEdge: (edge: DockEdge) =>
    invoke<WindowLayoutState>("set_edge", { edge }),
  setSelectedCategory: (category: string) =>
    invoke<DockConfig>("set_selected_category", { category }),
  startHandleDrag: () => invoke<void>("start_handle_drag"),
  endHandleDrag: () => invoke<DragEndResult>("end_handle_drag"),
  launchItem: (id: string, runAsAdmin?: boolean) =>
    invoke<DockConfig>("launch_item", {
      id,
      runAsAdmin: runAsAdmin ?? null,
    }),
  launchSystemTool: (tool: SystemToolKind) =>
    invoke<void>("launch_system_tool", { tool }),
  openFileLocation: (id: string) =>
    invoke<void>("open_file_location", { id }),
  getItemIcon: (id: string) => invoke<string | null>("get_item_icon", { id }),
  addItem: (input: NewItemInput) => invoke<DockConfig>("add_item", { input }),
  updateItem: (input: UpdateItemInput) =>
    invoke<DockConfig>("update_item", { input }),
  removeItem: (id: string) => invoke<DockConfig>("remove_item", { id }),
  toggleItemPin: (id: string) => invoke<DockConfig>("toggle_item_pin", { id }),
  moveItemCategory: (id: string, category: Category) =>
    invoke<DockConfig>("move_item_category", { id, category }),
  reorderItems: (category: Category, pinned: boolean, orderedIds: string[]) =>
    invoke<DockConfig>("reorder_items", { category, pinned, orderedIds }),
  rescanShortcuts: () => invoke<RescanPayload>("rescan_shortcuts"),
  resetToDefaults: () => invoke<DockConfig>("reset_to_defaults"),
  dismissMissingItems: (ids: string[]) =>
    invoke<DockConfig>("dismiss_missing_items", { ids }),
  updateSettings: (settings: SettingsUpdate) =>
    invoke<UpdateSettingsResult>("update_settings", { settings }),
  pickExecutableFile: () => invoke<string | null>("pick_executable_file"),
  pickIconFile: () => invoke<string | null>("pick_icon_file"),
  pinDroppedFiles: (paths: string[], category: Category) =>
    invoke<DropPinResult>("pin_dropped_files", { paths, category }),
  onLayoutChanged: (handler) =>
    listen<WindowLayoutState>(EVENT_LAYOUT_CHANGED, (event) =>
      handler(event.payload),
    ),
  onRequestCollapse: (handler) =>
    listen<CollapseRequestPayload>(EVENT_REQUEST_COLLAPSE, (event) =>
      handler(event.payload),
    ),
  onHotkeyWarning: (handler) =>
    listen<string | null>(EVENT_HOTKEY_WARNING, (event) =>
      handler(event.payload),
    ),
  onFileDropHover: (handler) =>
    listen<string[]>(EVENT_FILE_DROP_HOVER, (event) => handler(event.payload)),
  onFileDropLeave: (handler) =>
    listen<void>(EVENT_FILE_DROP_LEAVE, () => handler()),
  onFileDrop: (handler) =>
    listen<string[]>(EVENT_FILE_DROP, (event) => handler(event.payload)),
};

export const ipc: HarborIpc = {
  toAssetUrl,
  getInitialState: async () =>
    shouldUseBrowserMock()
      ? (await getMock()).getInitialState()
      : tauriIpc.getInitialState(),
  listMonitors: async () =>
    shouldUseBrowserMock()
      ? (await getMock()).listMonitors()
      : tauriIpc.listMonitors(),
  expandDock: async () =>
    shouldUseBrowserMock()
      ? (await getMock()).expandDock()
      : tauriIpc.expandDock(),
  requestCollapseDock: async () =>
    shouldUseBrowserMock()
      ? (await getMock()).requestCollapseDock()
      : tauriIpc.requestCollapseDock(),
  commitCollapseWindow: async (epoch) =>
    shouldUseBrowserMock()
      ? (await getMock()).commitCollapseWindow(epoch)
      : tauriIpc.commitCollapseWindow(epoch),
  toggleDock: async () =>
    shouldUseBrowserMock()
      ? (await getMock()).toggleDock()
      : tauriIpc.toggleDock(),
  setPinned: async (pinned) =>
    shouldUseBrowserMock()
      ? (await getMock()).setPinned(pinned)
      : tauriIpc.setPinned(pinned),
  setEdge: async (edge) =>
    shouldUseBrowserMock()
      ? (await getMock()).setEdge(edge)
      : tauriIpc.setEdge(edge),
  setSelectedCategory: async (category) =>
    shouldUseBrowserMock()
      ? (await getMock()).setSelectedCategory(category)
      : tauriIpc.setSelectedCategory(category),
  startHandleDrag: async () =>
    shouldUseBrowserMock()
      ? (await getMock()).startHandleDrag()
      : tauriIpc.startHandleDrag(),
  endHandleDrag: async () =>
    shouldUseBrowserMock()
      ? (await getMock()).endHandleDrag()
      : tauriIpc.endHandleDrag(),
  launchItem: async (id, runAsAdmin) =>
    shouldUseBrowserMock()
      ? (await getMock()).launchItem(id, runAsAdmin)
      : tauriIpc.launchItem(id, runAsAdmin),
  launchSystemTool: async (tool) =>
    shouldUseBrowserMock()
      ? (await getMock()).launchSystemTool(tool)
      : tauriIpc.launchSystemTool(tool),
  openFileLocation: async (id) =>
    shouldUseBrowserMock()
      ? (await getMock()).openFileLocation(id)
      : tauriIpc.openFileLocation(id),
  getItemIcon: async (id) =>
    shouldUseBrowserMock()
      ? (await getMock()).getItemIcon(id)
      : tauriIpc.getItemIcon(id),
  addItem: async (input) =>
    shouldUseBrowserMock()
      ? (await getMock()).addItem(input)
      : tauriIpc.addItem(input),
  updateItem: async (input) =>
    shouldUseBrowserMock()
      ? (await getMock()).updateItem(input)
      : tauriIpc.updateItem(input),
  removeItem: async (id) =>
    shouldUseBrowserMock()
      ? (await getMock()).removeItem(id)
      : tauriIpc.removeItem(id),
  toggleItemPin: async (id) =>
    shouldUseBrowserMock()
      ? (await getMock()).toggleItemPin(id)
      : tauriIpc.toggleItemPin(id),
  moveItemCategory: async (id, category) =>
    shouldUseBrowserMock()
      ? (await getMock()).moveItemCategory(id, category)
      : tauriIpc.moveItemCategory(id, category),
  reorderItems: async (category, pinned, orderedIds) =>
    shouldUseBrowserMock()
      ? (await getMock()).reorderItems(category, pinned, orderedIds)
      : tauriIpc.reorderItems(category, pinned, orderedIds),
  rescanShortcuts: async () =>
    shouldUseBrowserMock()
      ? (await getMock()).rescanShortcuts()
      : tauriIpc.rescanShortcuts(),
  resetToDefaults: async () =>
    shouldUseBrowserMock()
      ? (await getMock()).resetToDefaults()
      : tauriIpc.resetToDefaults(),
  dismissMissingItems: async (ids) =>
    shouldUseBrowserMock()
      ? (await getMock()).dismissMissingItems(ids)
      : tauriIpc.dismissMissingItems(ids),
  updateSettings: async (settings) =>
    shouldUseBrowserMock()
      ? (await getMock()).updateSettings(settings)
      : tauriIpc.updateSettings(settings),
  pickExecutableFile: async () =>
    shouldUseBrowserMock()
      ? (await getMock()).pickExecutableFile()
      : tauriIpc.pickExecutableFile(),
  pickIconFile: async () =>
    shouldUseBrowserMock()
      ? (await getMock()).pickIconFile()
      : tauriIpc.pickIconFile(),
  pinDroppedFiles: async (paths, category) =>
    shouldUseBrowserMock()
      ? (await getMock()).pinDroppedFiles(paths, category)
      : tauriIpc.pinDroppedFiles(paths, category),
  onLayoutChanged: async (handler) =>
    shouldUseBrowserMock()
      ? (await getMock()).onLayoutChanged(handler)
      : tauriIpc.onLayoutChanged(handler),
  onRequestCollapse: async (handler) =>
    shouldUseBrowserMock()
      ? (await getMock()).onRequestCollapse(handler)
      : tauriIpc.onRequestCollapse(handler),
  onHotkeyWarning: async (handler) =>
    shouldUseBrowserMock()
      ? (await getMock()).onHotkeyWarning(handler)
      : tauriIpc.onHotkeyWarning(handler),
  onFileDropHover: async (handler) =>
    shouldUseBrowserMock()
      ? (await getMock()).onFileDropHover(handler)
      : tauriIpc.onFileDropHover(handler),
  onFileDropLeave: async (handler) =>
    shouldUseBrowserMock()
      ? (await getMock()).onFileDropLeave(handler)
      : tauriIpc.onFileDropLeave(handler),
  onFileDrop: async (handler) =>
    shouldUseBrowserMock()
      ? (await getMock()).onFileDrop(handler)
      : tauriIpc.onFileDrop(handler),
};
