export type DockEdge = "Left" | "Right";
export type Category = "Games" | "Apps" | "Tools";
export type ItemSource = "Manual" | "Steam" | "StartMenu" | "System";
export type SystemToolKind = "taskmgr" | "terminal" | "lock";

export interface DockItem {
  id: string;
  name: string;
  path: string;
  arguments: string;
  description: string;
  category: Category;
  custom_icon_path: string | null;
  icon_source_path: string | null;
  run_as_admin: boolean;
  pinned: boolean;
  source: ItemSource;
  steam_app_id: string | null;
  launch_count: number;
}

export interface DockConfig {
  schema_version: number;
  edge: DockEdge;
  monitor_name: string | null;
  top_position: number;
  auto_collapse_on_launch: boolean;
  auto_collapse_on_blur: boolean;
  always_on_top: boolean;
  start_with_windows: boolean;
  global_hotkey: string;
  selected_category: string;
  items: DockItem[];
  dismissed: string[];
}

export interface WindowLayoutState {
  expanded: boolean;
  edge: DockEdge;
  dock_height: number;
  handle_offset_y: number;
  top_position: number;
  pinned: boolean;
}

export interface CollapseRequestPayload {
  epoch: number;
}

export interface DragEndResult {
  moved: boolean;
  top_position: number;
}

export interface MonitorDescriptor {
  name: string;
  label: string;
  width: number;
  height: number;
  scale_factor: number;
  is_primary: boolean;
}

export interface InitialStatePayload {
  config: DockConfig;
  layout: WindowLayoutState;
  monitors: MonitorDescriptor[];
  hotkey_warning: string | null;
  is_portable: boolean;
}

export interface RescanResult {
  added: number;
  missing_ids: string[];
}

export interface RescanPayload {
  rescan: RescanResult;
  config: DockConfig;
}

export interface DropPinResult {
  added: number;
  skipped: number;
  config: DockConfig;
}

export interface NewItemInput {
  name: string;
  path: string;
  arguments: string;
  description: string;
  category: Category;
  custom_icon_path: string | null;
  run_as_admin: boolean;
  pinned: boolean;
}

export interface UpdateItemInput {
  id: string;
  name: string;
  path: string;
  arguments: string;
  description: string;
  category: Category;
  custom_icon_path: string | null;
  run_as_admin: boolean;
  pinned: boolean;
}

export interface SettingsUpdate {
  edge: DockEdge;
  monitor_name: string | null;
  auto_collapse_on_launch: boolean;
  auto_collapse_on_blur: boolean;
  always_on_top: boolean;
  start_with_windows: boolean;
  global_hotkey: string;
}

export interface UpdateSettingsResult {
  config: DockConfig;
  layout: WindowLayoutState;
  hotkey_warning: string | null;
}

export interface AppErrorPayload {
  kind: string;
  message: string;
}

export interface HarborIpc {
  toAssetUrl(filePath: string | null): string | null;
  getInitialState(): Promise<InitialStatePayload>;
  listMonitors(): Promise<MonitorDescriptor[]>;
  expandDock(): Promise<WindowLayoutState>;
  requestCollapseDock(): Promise<WindowLayoutState>;
  commitCollapseWindow(epoch: number): Promise<boolean>;
  toggleDock(): Promise<WindowLayoutState>;
  setPinned(pinned: boolean): Promise<WindowLayoutState>;
  setEdge(edge: DockEdge): Promise<WindowLayoutState>;
  setSelectedCategory(category: string): Promise<DockConfig>;
  startHandleDrag(): Promise<void>;
  endHandleDrag(): Promise<DragEndResult>;
  launchItem(id: string, runAsAdmin?: boolean): Promise<DockConfig>;
  launchSystemTool(tool: SystemToolKind): Promise<void>;
  openFileLocation(id: string): Promise<void>;
  getItemIcon(id: string): Promise<string | null>;
  addItem(input: NewItemInput): Promise<DockConfig>;
  updateItem(input: UpdateItemInput): Promise<DockConfig>;
  removeItem(id: string): Promise<DockConfig>;
  toggleItemPin(id: string): Promise<DockConfig>;
  moveItemCategory(id: string, category: Category): Promise<DockConfig>;
  reorderItems(
    category: Category,
    pinned: boolean,
    orderedIds: string[],
  ): Promise<DockConfig>;
  rescanShortcuts(): Promise<RescanPayload>;
  resetToDefaults(): Promise<DockConfig>;
  dismissMissingItems(ids: string[]): Promise<DockConfig>;
  updateSettings(settings: SettingsUpdate): Promise<UpdateSettingsResult>;
  pickExecutableFile(): Promise<string | null>;
  pickIconFile(): Promise<string | null>;
  pinDroppedFiles(paths: string[], category: Category): Promise<DropPinResult>;
  onLayoutChanged(
    handler: (layout: WindowLayoutState) => void,
  ): Promise<() => void>;
  onRequestCollapse(
    handler: (payload: CollapseRequestPayload) => void,
  ): Promise<() => void>;
  onHotkeyWarning(
    handler: (warning: string | null) => void,
  ): Promise<() => void>;
  onFileDropHover(handler: (paths: string[]) => void): Promise<() => void>;
  onFileDropLeave(handler: () => void): Promise<() => void>;
  onFileDrop(handler: (paths: string[]) => void): Promise<() => void>;
}
