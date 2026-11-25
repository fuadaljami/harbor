use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Monitor, PhysicalPosition, PhysicalSize, WebviewWindow};

use crate::config::ConfigManager;
use crate::error::{AppError, AppResult};
use crate::models::DockEdge;
use crate::win_shell;

static MAIN_THREAD_ID: OnceLock<ThreadId> = OnceLock::new();

/// Records the main UI thread ID during `setup()` so `SetWindowRgn` can execute on the main
/// thread synchronously when already on it or synchronously wait via `run_on_main_thread` otherwise.
pub fn register_main_thread() {
    let _ = MAIN_THREAD_ID.set(std::thread::current().id());
}

fn run_sync_on_main_thread<F>(app: &AppHandle, task: F) -> AppResult<()>
where
    F: FnOnce() -> AppResult<()> + Send + 'static,
{
    if MAIN_THREAD_ID.get().copied() == Some(std::thread::current().id()) {
        return task();
    }
    let (tx, rx) = std::sync::mpsc::sync_channel::<AppResult<()>>(1);
    app.run_on_main_thread(move || {
        let _ = tx.send(task());
    })?;
    rx.recv()
        .map_err(|_| AppError::Window("Main thread task channel closed".to_owned()))?
}

pub const DOCK_PANEL_WIDTH_LOGICAL: f64 = 380.0;
pub const HANDLE_WIDTH_LOGICAL: f64 = 24.0;
pub const HANDLE_HEIGHT_LOGICAL: f64 = 84.0;
pub const EXPANDED_WIDTH_LOGICAL: f64 = DOCK_PANEL_WIDTH_LOGICAL + HANDLE_WIDTH_LOGICAL;
pub const MAX_DOCK_HEIGHT_LOGICAL: f64 = 640.0;
pub const EDGE_MARGIN_LOGICAL: f64 = 8.0;
pub const WORK_AREA_VERTICAL_PADDING_LOGICAL: f64 = 24.0;
pub const MIN_DRAG_COMMIT_DELTA_LOGICAL: f64 = 5.0;

const BLUR_GRACE_MS: u64 = 250;
const COLLAPSE_FALLBACK_MS: u64 = 300;
const DRAG_POLL_INTERVAL_MS: u64 = 8;

pub const EVENT_LAYOUT_CHANGED: &str = "harbor://layout-changed";
pub const EVENT_REQUEST_COLLAPSE: &str = "harbor://request-collapse";
pub const EVENT_HOTKEY_WARNING: &str = "harbor://hotkey-warning";
pub const EVENT_FILE_DROP_HOVER: &str = "harbor://file-drop-hover";
pub const EVENT_FILE_DROP_LEAVE: &str = "harbor://file-drop-leave";
pub const EVENT_FILE_DROP: &str = "harbor://file-drop";

/// Physical-pixel monitor work area rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalWorkArea {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Physical-pixel window bounds rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Complete computed geometry for both collapsed and expanded dock states.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ComputedDockLayout {
    pub collapsed_rect_phys: PhysRect,
    pub expanded_rect_phys: PhysRect,
    pub clamped_top_logical: f64,
    pub expanded_top_logical: f64,
    pub dock_height_logical: f64,
    pub handle_offset_y_logical: f64,
}

/// Window-relative physical clipping rectangle `(left, top, right, bottom)` for `CreateRectRgn`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl RegionRect {
    #[must_use]
    pub fn as_tuple(self) -> (i32, i32, i32, i32) {
        (self.left, self.top, self.right, self.bottom)
    }
}

/// Union region rects for the expanded dock window (`dock_rect` | `handle_rect`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExpandedWindowRegionRects {
    pub dock_rect: RegionRect,
    pub handle_rect: RegionRect,
}

/// Layout state synchronized to the frontend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowLayoutState {
    pub expanded: bool,
    pub edge: DockEdge,
    pub dock_height: f64,
    pub handle_offset_y: f64,
    pub top_position: f64,
    pub pinned: bool,
}

/// Payload emitted when Rust requests a smooth collapse animation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollapseRequestPayload {
    pub epoch: u64,
}

/// Result returned when finishing an edge-handle vertical drag.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DragEndResult {
    pub moved: bool,
    pub top_position: f64,
}

/// Monitor metadata exposed to Settings UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitorDescriptor {
    pub name: String,
    pub label: String,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f64,
    pub is_primary: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowPhase {
    Collapsed,
    Expanded,
    Collapsing,
}

struct DragSession {
    active: Arc<AtomicBool>,
    start_top_logical: f64,
    current_y_phys: Arc<AtomicI32>,
    work_area_top_phys: i32,
    scale_factor: f64,
    max_top_logical: f64,
}

struct WindowRuntimeState {
    phase: WindowPhase,
    last_expand_instant: Option<Instant>,
    drag_session: Option<DragSession>,
}

/// RAII guard tracking open native file/icon dialogs to suspend blur auto-collapse.
pub struct DialogGuard {
    counter: Arc<AtomicUsize>,
}

impl Drop for DialogGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::AcqRel);
    }
}

/// RAII guard preventing concurrent system rescans.
pub struct RescanGuard {
    flag: Arc<AtomicBool>,
}

impl Drop for RescanGuard {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::Release);
    }
}

/// Coordinates native window sizing, multi-monitor clamping, epoch-safe collapse, and handle drag.
pub struct WindowCoordinator {
    inner: Mutex<WindowRuntimeState>,
    epoch: AtomicU64,
    dock_pinned: AtomicBool,
    is_applying_layout: AtomicBool,
    dialog_open_count: Arc<AtomicUsize>,
    rescan_in_progress: Arc<AtomicBool>,
    hotkey_warning: Mutex<Option<String>>,
    registered_hotkey: Mutex<Option<String>>,
    tray_edge_item: Mutex<Option<tauri::menu::MenuItem<tauri::Wry>>>,
}

impl Default for WindowCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowCoordinator {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(WindowRuntimeState {
                phase: WindowPhase::Collapsed,
                last_expand_instant: None,
                drag_session: None,
            }),
            epoch: AtomicU64::new(0),
            dock_pinned: AtomicBool::new(false),
            is_applying_layout: AtomicBool::new(false),
            dialog_open_count: Arc::new(AtomicUsize::new(0)),
            rescan_in_progress: Arc::new(AtomicBool::new(false)),
            hotkey_warning: Mutex::new(None),
            registered_hotkey: Mutex::new(None),
            tray_edge_item: Mutex::new(None),
        }
    }

    /// Acquires an RAII guard marking an active native dialog (suspending blur auto-collapse).
    #[must_use]
    pub fn begin_dialog(&self) -> DialogGuard {
        self.dialog_open_count.fetch_add(1, Ordering::AcqRel);
        DialogGuard {
            counter: Arc::clone(&self.dialog_open_count),
        }
    }

    /// Attempts to acquire the exclusive rescan guard; returns `None` if already scanning.
    #[must_use]
    pub fn try_begin_rescan(&self) -> Option<RescanGuard> {
        self.rescan_in_progress
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| RescanGuard {
                flag: Arc::clone(&self.rescan_in_progress),
            })
    }

    pub fn set_tray_edge_item(&self, item: tauri::menu::MenuItem<tauri::Wry>) {
        *self.tray_edge_item.lock() = Some(item);
    }

    pub fn update_tray_edge_label(&self, edge: DockEdge) {
        if let Some(item) = self.tray_edge_item.lock().as_ref() {
            let label = match edge {
                DockEdge::Left => "Switch to Right Edge",
                DockEdge::Right => "Switch to Left Edge",
            };
            if let Err(err) = item.set_text(label) {
                tracing::warn!(error = %err, "Failed to update tray edge label");
            }
        }
    }

    #[must_use]
    pub fn hotkey_warning(&self) -> Option<String> {
        self.hotkey_warning.lock().clone()
    }

    pub fn set_hotkey_state(&self, registered: Option<String>, warning: Option<String>) {
        *self.registered_hotkey.lock() = registered;
        *self.hotkey_warning.lock() = warning;
    }

    #[must_use]
    pub fn registered_hotkey(&self) -> Option<String> {
        self.registered_hotkey.lock().clone()
    }

    #[must_use]
    pub fn is_pinned(&self) -> bool {
        self.dock_pinned.load(Ordering::Acquire)
    }

    #[must_use]
    pub fn is_expanded_or_collapsing(&self) -> bool {
        matches!(
            self.inner.lock().phase,
            WindowPhase::Expanded | WindowPhase::Collapsing
        )
    }

    /// Computes current layout state snapshot for the active monitor and config.
    pub fn current_layout_state(
        &self,
        app: &AppHandle,
        config_mgr: &ConfigManager,
    ) -> AppResult<WindowLayoutState> {
        let cfg = config_mgr.snapshot();
        let window = main_window(app)?;
        let (work_area, scale) = resolve_active_work_area(&window, cfg.monitor_name.as_deref())?;
        let computed = compute_dock_layout(work_area, scale, cfg.edge, cfg.top_position);
        let expanded = matches!(self.inner.lock().phase, WindowPhase::Expanded);
        Ok(WindowLayoutState {
            expanded,
            edge: cfg.edge,
            dock_height: computed.dock_height_logical,
            handle_offset_y: computed.handle_offset_y_logical,
            top_position: computed.clamped_top_logical,
            pinned: self.is_pinned(),
        })
    }

    /// Applies initial collapsed `24x84` bounds before showing the window at startup.
    pub fn apply_startup_collapsed(
        &self,
        app: &AppHandle,
        config_mgr: &ConfigManager,
    ) -> AppResult<()> {
        let cfg = config_mgr.snapshot();
        let window = main_window(app)?;
        let (work_area, scale) = resolve_active_work_area(&window, cfg.monitor_name.as_deref())?;
        let computed = compute_dock_layout(work_area, scale, cfg.edge, cfg.top_position);

        window.set_always_on_top(cfg.always_on_top)?;
        self.clear_region_for(app, &window)?;
        self.apply_physical_bounds(&window, computed.collapsed_rect_phys)?;

        let outer = window.outer_size()?;
        tracing::info!(
            width = outer.width,
            height = outer.height,
            scale_factor = scale,
            edge = ?cfg.edge,
            "Startup collapsed window outer_size"
        );

        window.show()?;
        Ok(())
    }

    /// Expands the dock immediately: bumps epoch, resizes & positions native window FIRST,
    /// applies the expanded HWND region, activates/focuses with 50ms foreground check,
    /// and emits `harbor://layout-changed`.
    pub fn expand_dock(
        &self,
        app: &AppHandle,
        config_mgr: &ConfigManager,
    ) -> AppResult<WindowLayoutState> {
        let cfg = config_mgr.snapshot();
        let window = main_window(app)?;
        let (work_area, scale) = resolve_active_work_area(&window, cfg.monitor_name.as_deref())?;
        let computed = compute_dock_layout(work_area, scale, cfg.edge, cfg.top_position);

        let expand_epoch = {
            let mut state = self.inner.lock();
            if let Some(drag) = state.drag_session.take() {
                drag.active.store(false, Ordering::Release);
            }
            let next_epoch = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;
            state.phase = WindowPhase::Expanded;
            state.last_expand_instant = Some(Instant::now());
            next_epoch
        };

        self.apply_expanded_bounds_and_region(app, &window, &cfg, &computed, scale)?;
        let _ = window.show();
        let _ = window.set_focus();
        self.verify_foreground_focus(app, &window, expand_epoch);

        let layout = WindowLayoutState {
            expanded: true,
            edge: cfg.edge,
            dock_height: computed.dock_height_logical,
            handle_offset_y: computed.handle_offset_y_logical,
            top_position: computed.clamped_top_logical,
            pinned: self.is_pinned(),
        };
        let _ = app.emit(EVENT_LAYOUT_CHANGED, &layout);
        Ok(layout)
    }

    /// Initiates an epoch-guarded collapse sequence: sets `Collapsing`, emits `harbor://request-collapse`,
    /// and schedules a `300ms` fallback commit timer.
    pub fn request_collapse_dock(
        &self,
        app: &AppHandle,
        config_mgr: &ConfigManager,
    ) -> AppResult<WindowLayoutState> {
        let maybe_epoch = {
            let mut state = self.inner.lock();
            if state.phase == WindowPhase::Collapsed {
                None
            } else {
                if let Some(drag) = state.drag_session.take() {
                    drag.active.store(false, Ordering::Release);
                }
                let next_epoch = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;
                state.phase = WindowPhase::Collapsing;
                Some(next_epoch)
            }
        };

        if let Some(collapse_epoch) = maybe_epoch {
            let _ = app.emit(
                EVENT_REQUEST_COLLAPSE,
                CollapseRequestPayload {
                    epoch: collapse_epoch,
                },
            );

            let app_clone = app.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_millis(COLLAPSE_FALLBACK_MS)).await;
                let coord = app_clone.state::<WindowCoordinator>();
                let cfg_mgr = app_clone.state::<ConfigManager>();
                if let Err(err) = coord.commit_collapse_window(&app_clone, &cfg_mgr, collapse_epoch)
                {
                    tracing::warn!(error = %err, "Fallback collapse commit failed");
                }
            });
        }

        self.current_layout_state(app, config_mgr)
    }

    /// Idempotent collapse commit: clears `SetWindowRgn` FIRST, then shrinks the window to `24x84`
    /// ONLY if `expected_epoch` still matches and phase is still `Collapsing`.
    pub fn commit_collapse_window(
        &self,
        app: &AppHandle,
        config_mgr: &ConfigManager,
        expected_epoch: u64,
    ) -> AppResult<bool> {
        {
            let mut state = self.inner.lock();
            if self.epoch.load(Ordering::SeqCst) != expected_epoch
                || state.phase != WindowPhase::Collapsing
            {
                return Ok(false);
            }
            state.phase = WindowPhase::Collapsed;
        }

        let cfg = config_mgr.snapshot();
        let window = main_window(app)?;
        let (work_area, scale) = resolve_active_work_area(&window, cfg.monitor_name.as_deref())?;
        let computed = compute_dock_layout(work_area, scale, cfg.edge, cfg.top_position);

        // Strict lifecycle order: clear region BEFORE shrinking to 24x84 so collapsed handle
        // on the Right edge is never clipped by a stale expanded region.
        self.clear_region_for(app, &window)?;
        self.apply_physical_bounds(&window, computed.collapsed_rect_phys)?;

        let outer = window.outer_size()?;
        tracing::info!(
            width = outer.width,
            height = outer.height,
            epoch = expected_epoch,
            "Committed collapsed window outer_size"
        );

        let layout = WindowLayoutState {
            expanded: false,
            edge: cfg.edge,
            dock_height: computed.dock_height_logical,
            handle_offset_y: computed.handle_offset_y_logical,
            top_position: computed.clamped_top_logical,
            pinned: self.is_pinned(),
        };
        let _ = app.emit(EVENT_LAYOUT_CHANGED, &layout);
        Ok(true)
    }

    /// Toggles between collapsed and expanded states.
    pub fn toggle_dock(
        &self,
        app: &AppHandle,
        config_mgr: &ConfigManager,
    ) -> AppResult<WindowLayoutState> {
        let phase = self.inner.lock().phase;
        match phase {
            WindowPhase::Collapsed => self.expand_dock(app, config_mgr),
            WindowPhase::Expanded => self.request_collapse_dock(app, config_mgr),
            WindowPhase::Collapsing => self.expand_dock(app, config_mgr),
        }
    }

    /// Updates runtime dock-pin state and emits `harbor://layout-changed`.
    pub fn set_pinned(
        &self,
        app: &AppHandle,
        config_mgr: &ConfigManager,
        pinned: bool,
    ) -> AppResult<WindowLayoutState> {
        self.dock_pinned.store(pinned, Ordering::Release);
        let layout = self.current_layout_state(app, config_mgr)?;
        let _ = app.emit(EVENT_LAYOUT_CHANGED, &layout);
        Ok(layout)
    }

    /// Re-applies current phase geometry and HWND clipping region
    /// (after edge switch, monitor switch, or DPI change).
    pub fn reapply_current_layout(
        &self,
        app: &AppHandle,
        config_mgr: &ConfigManager,
    ) -> AppResult<WindowLayoutState> {
        let cfg = config_mgr.snapshot();
        let window = main_window(app)?;
        let (work_area, scale) = resolve_active_work_area(&window, cfg.monitor_name.as_deref())?;
        let computed = compute_dock_layout(work_area, scale, cfg.edge, cfg.top_position);
        let phase = self.inner.lock().phase;

        match phase {
            WindowPhase::Collapsed | WindowPhase::Collapsing => {
                window.set_always_on_top(cfg.always_on_top)?;
                self.clear_region_for(app, &window)?;
                self.apply_physical_bounds(&window, computed.collapsed_rect_phys)?;
            }
            WindowPhase::Expanded => {
                self.apply_expanded_bounds_and_region(app, &window, &cfg, &computed, scale)?;
            }
        }

        let layout = WindowLayoutState {
            expanded: phase == WindowPhase::Expanded,
            edge: cfg.edge,
            dock_height: computed.dock_height_logical,
            handle_offset_y: computed.handle_offset_y_logical,
            top_position: computed.clamped_top_logical,
            pinned: self.is_pinned(),
        };
        let _ = app.emit(EVENT_LAYOUT_CHANGED, &layout);
        Ok(layout)
    }

    /// Recomputes and reapplies the expanded HWND region if an external resize occurs while expanded.
    pub fn handle_window_resized(&self, app: &AppHandle, config_mgr: &ConfigManager) {
        if self.is_applying_layout.load(Ordering::Acquire) {
            return;
        }
        let phase = self.inner.lock().phase;
        if phase != WindowPhase::Expanded {
            return;
        }
        let cfg = config_mgr.snapshot();
        let Ok(window) = main_window(app) else {
            return;
        };
        let Ok((work_area, scale)) = resolve_active_work_area(&window, cfg.monitor_name.as_deref())
        else {
            return;
        };
        let computed = compute_dock_layout(work_area, scale, cfg.edge, cfg.top_position);
        if let Err(err) = self.apply_region_from_outer_size(
            app,
            &window,
            cfg.edge,
            scale,
            computed.handle_offset_y_logical,
        ) {
            tracing::warn!(error = %err, "Failed to update region on WindowEvent::Resized");
        }
    }

    fn apply_expanded_bounds_and_region(
        &self,
        app: &AppHandle,
        window: &WebviewWindow,
        cfg: &crate::models::DockConfig,
        computed: &ComputedDockLayout,
        scale: f64,
    ) -> AppResult<()> {
        window.set_always_on_top(cfg.always_on_top)?;
        self.apply_physical_bounds(window, computed.expanded_rect_phys)?;
        self.apply_region_from_outer_size(
            app,
            window,
            cfg.edge,
            scale,
            computed.handle_offset_y_logical,
        )
    }

    fn apply_region_from_outer_size(
        &self,
        app: &AppHandle,
        window: &WebviewWindow,
        edge: DockEdge,
        scale: f64,
        handle_offset_y_logical: f64,
    ) -> AppResult<()> {
        let outer = window.outer_size()?;
        if outer.width < 2 || outer.height < 2 {
            return Ok(());
        }
        let region = compute_expanded_window_region(
            outer.width,
            outer.height,
            scale,
            edge,
            handle_offset_y_logical,
        );
        let hwnd_raw = window.hwnd()?.0 as isize;
        let dock_tuple = region.dock_rect.as_tuple();
        let handle_tuple = region.handle_rect.as_tuple();

        tracing::debug!(
            outer_width = outer.width,
            outer_height = outer.height,
            edge = ?edge,
            scale_factor = scale,
            dock_rect = ?dock_tuple,
            handle_rect = ?handle_tuple,
            "Applying SetWindowRgn for expanded dock"
        );

        run_sync_on_main_thread(app, move || {
            win_shell::set_window_region_union(hwnd_raw, dock_tuple, handle_tuple)
        })
    }

    fn clear_region_for(&self, app: &AppHandle, window: &WebviewWindow) -> AppResult<()> {
        let hwnd_raw = window.hwnd()?.0 as isize;
        run_sync_on_main_thread(app, move || win_shell::clear_window_region(hwnd_raw))
    }

    fn verify_foreground_focus(&self, app: &AppHandle, window: &WebviewWindow, expand_epoch: u64) {
        let Ok(hwnd) = window.hwnd() else {
            return;
        };
        let hwnd_raw = hwnd.0 as isize;
        if win_shell::is_window_foreground(hwnd_raw) {
            return;
        }

        let app_clone = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let coord = app_clone.state::<WindowCoordinator>();
            if coord.epoch.load(Ordering::SeqCst) != expand_epoch {
                return;
            }
            if coord.inner.lock().phase != WindowPhase::Expanded {
                return;
            }
            let Ok(win) = main_window(&app_clone) else {
                return;
            };
            if !win_shell::is_window_foreground(hwnd_raw) {
                let _ = win.set_focus();
                if !win_shell::is_window_foreground(hwnd_raw) {
                    tracing::warn!("Windows did not grant foreground focus after 50ms retry");
                }
            }
        });
    }

    /// Handles `WindowEvent::ScaleFactorChanged` safely without interfering with active drags.
    pub fn handle_scale_factor_changed(&self, app: &AppHandle, config_mgr: &ConfigManager) {
        if self.is_applying_layout.load(Ordering::Acquire) {
            return;
        }
        let is_dragging = self.inner.lock().drag_session.is_some();
        if is_dragging {
            return;
        }
        if let Err(err) = self.reapply_current_layout(app, config_mgr) {
            tracing::warn!(error = %err, "Failed to re-apply layout on DPI scale change");
        }
    }

    /// Evaluates blur auto-collapse guardrails before requesting collapse.
    pub fn handle_window_blur(&self, app: &AppHandle, config_mgr: &ConfigManager) {
        let cfg = config_mgr.snapshot();
        if !cfg.auto_collapse_on_blur || self.is_pinned() {
            return;
        }
        if self.dialog_open_count.load(Ordering::Acquire) > 0 {
            return;
        }

        {
            let state = self.inner.lock();
            if state.phase != WindowPhase::Expanded || state.drag_session.is_some() {
                return;
            }
            if let Some(last_expand) = state.last_expand_instant {
                if last_expand.elapsed() < Duration::from_millis(BLUR_GRACE_MS) {
                    return;
                }
            }
        }

        if let Err(err) = self.request_collapse_dock(app, config_mgr) {
            tracing::warn!(error = %err, "Blur auto-collapse failed");
        }
    }

    /// Starts Win32 `GetCursorPos` polling for vertical handle dragging while collapsed.
    pub fn start_handle_drag(
        &self,
        app: &AppHandle,
        config_mgr: &ConfigManager,
    ) -> AppResult<()> {
        let cfg = config_mgr.snapshot();
        let window = main_window(app)?;
        let (work_area, scale) = resolve_active_work_area(&window, cfg.monitor_name.as_deref())?;
        let computed = compute_dock_layout(work_area, scale, cfg.edge, cfg.top_position);

        let (_cursor_x, cursor_y) = win_shell::get_cursor_pos()?;
        let win_pos = window.outer_position()?;
        let grab_offset_y = cursor_y - win_pos.y;

        let collapsed_x_phys = computed.collapsed_rect_phys.x;
        let collapsed_h_phys = computed.collapsed_rect_phys.height as i32;
        let margin_phys = (EDGE_MARGIN_LOGICAL * scale).round() as i32;
        let min_y_phys = work_area.y + margin_phys;
        let max_y_phys = (work_area.y + work_area.height as i32 - collapsed_h_phys - margin_phys)
            .max(min_y_phys);
        let work_area_logical_h = f64::from(work_area.height) / scale;
        let max_top_logical = (work_area_logical_h - HANDLE_HEIGHT_LOGICAL - EDGE_MARGIN_LOGICAL)
            .max(EDGE_MARGIN_LOGICAL);

        let active = Arc::new(AtomicBool::new(true));
        let current_y_phys = Arc::new(AtomicI32::new(
            win_pos.y.clamp(min_y_phys, max_y_phys),
        ));

        {
            let mut state = self.inner.lock();
            if state.phase != WindowPhase::Collapsed {
                return Err(AppError::InvalidInput(
                    "Handle drag is only allowed while dock is collapsed".to_owned(),
                ));
            }
            if let Some(prev) = state.drag_session.take() {
                prev.active.store(false, Ordering::Release);
            }
            state.drag_session = Some(DragSession {
                active: Arc::clone(&active),
                start_top_logical: computed.clamped_top_logical,
                current_y_phys: Arc::clone(&current_y_phys),
                work_area_top_phys: work_area.y,
                scale_factor: scale,
                max_top_logical,
            });
        }

        let hwnd = window.hwnd()?;
        let hwnd_raw = hwnd.0 as isize;

        std::thread::Builder::new()
            .name("harbor-handle-drag".to_owned())
            .spawn(move || {
                let mut last_y = current_y_phys.load(Ordering::Acquire);
                while active.load(Ordering::Acquire) {
                    if !win_shell::is_primary_mouse_button_down() {
                        active.store(false, Ordering::Release);
                        break;
                    }
                    if let Ok((_cx, cy)) = win_shell::get_cursor_pos() {
                        let clamped_y = (cy - grab_offset_y).clamp(min_y_phys, max_y_phys);
                        if clamped_y != last_y {
                            last_y = clamped_y;
                            current_y_phys.store(clamped_y, Ordering::Release);
                            let _ = win_shell::move_window_no_resize_async(
                                hwnd_raw,
                                collapsed_x_phys,
                                clamped_y,
                            );
                        }
                    }
                    std::thread::sleep(Duration::from_millis(DRAG_POLL_INTERVAL_MS));
                }
            })
            .map_err(|err| AppError::Window(format!("Failed to start handle drag thread: {err}")))?;

        Ok(())
    }

    /// Stops handle drag polling, persists `top_position` if moved >= 5 logical px,
    /// and snaps back if the gesture was a click (< 5px).
    pub fn end_handle_drag(
        &self,
        app: &AppHandle,
        config_mgr: &ConfigManager,
    ) -> AppResult<DragEndResult> {
        let session = {
            let mut state = self.inner.lock();
            state.drag_session.take()
        };

        let Some(session) = session else {
            let cfg = config_mgr.snapshot();
            return Ok(DragEndResult {
                moved: false,
                top_position: cfg.top_position,
            });
        };

        session.active.store(false, Ordering::Release);
        let final_y_phys = session.current_y_phys.load(Ordering::Acquire);
        let raw_top_logical =
            f64::from(final_y_phys - session.work_area_top_phys) / session.scale_factor;
        let clamped_final = raw_top_logical.clamp(EDGE_MARGIN_LOGICAL, session.max_top_logical);
        let moved = (clamped_final - session.start_top_logical).abs() >= MIN_DRAG_COMMIT_DELTA_LOGICAL;

        let _ = if moved {
            let rounded = (clamped_final * 10.0).round() / 10.0;
            config_mgr.update_and_save(|cfg| {
                cfg.top_position = rounded;
                Ok(())
            })?;
            rounded
        } else {
            session.start_top_logical
        };

        let layout = self.reapply_current_layout(app, config_mgr)?;
        Ok(DragEndResult {
            moved,
            top_position: layout.top_position,
        })
    }

    fn apply_physical_bounds(&self, window: &WebviewWindow, rect: PhysRect) -> AppResult<()> {
        self.is_applying_layout.store(true, Ordering::Release);
        let res: AppResult<()> = (|| {
            window.set_position(tauri::Position::Physical(PhysicalPosition::new(
                rect.x, rect.y,
            )))?;
            window.set_size(tauri::Size::Physical(PhysicalSize::new(
                rect.width,
                rect.height,
            )))?;

            // Re-check outer_size once to absorb cross-DPI monitor transitions cleanly.
            let actual = window.outer_size()?;
            if actual.width != rect.width || actual.height != rect.height {
                window.set_position(tauri::Position::Physical(PhysicalPosition::new(
                    rect.x, rect.y,
                )))?;
                window.set_size(tauri::Size::Physical(PhysicalSize::new(
                    rect.width,
                    rect.height,
                )))?;
            }
            Ok(())
        })();
        self.is_applying_layout.store(false, Ordering::Release);
        res
    }
}

/// Pure layout math computing collapsed/expanded physical rectangles and logical offsets.
#[must_use]
pub fn compute_dock_layout(
    work_area_phys: PhysicalWorkArea,
    scale_factor: f64,
    edge: DockEdge,
    top_position_logical: f64,
) -> ComputedDockLayout {
    let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };

    let work_area_logical_h = (f64::from(work_area_phys.height) / scale).max(HANDLE_HEIGHT_LOGICAL);
    let min_dock_h = HANDLE_HEIGHT_LOGICAL.min(work_area_logical_h);
    let dock_height_logical = MAX_DOCK_HEIGHT_LOGICAL
        .min((work_area_logical_h - WORK_AREA_VERTICAL_PADDING_LOGICAL).max(min_dock_h))
        .min(work_area_logical_h);

    let max_top_logical = (work_area_logical_h - HANDLE_HEIGHT_LOGICAL - EDGE_MARGIN_LOGICAL)
        .max(EDGE_MARGIN_LOGICAL)
        .min((work_area_logical_h - HANDLE_HEIGHT_LOGICAL).max(0.0));
    let min_top_logical = EDGE_MARGIN_LOGICAL.min(max_top_logical);
    let safe_top = if top_position_logical.is_finite() {
        top_position_logical
    } else {
        min_top_logical
    };
    let clamped_top_logical = safe_top.clamp(min_top_logical, max_top_logical);

    let desired_dock_top =
        clamped_top_logical + (HANDLE_HEIGHT_LOGICAL / 2.0) - (dock_height_logical / 2.0);
    let max_expanded_top = (work_area_logical_h - dock_height_logical - EDGE_MARGIN_LOGICAL)
        .max(EDGE_MARGIN_LOGICAL)
        .min((work_area_logical_h - dock_height_logical).max(0.0));
    let min_expanded_top = EDGE_MARGIN_LOGICAL.min(max_expanded_top);
    let expanded_top_logical = desired_dock_top.clamp(min_expanded_top, max_expanded_top);
    let handle_offset_y_logical = clamped_top_logical - expanded_top_logical;

    let collapsed_w_phys = (HANDLE_WIDTH_LOGICAL * scale).round().max(1.0) as u32;
    let collapsed_h_phys = (HANDLE_HEIGHT_LOGICAL * scale).round().max(1.0) as u32;
    let expanded_w_phys = (EXPANDED_WIDTH_LOGICAL * scale).round().max(1.0) as u32;
    let expanded_h_phys = (dock_height_logical * scale).round().max(1.0) as u32;

    let (collapsed_x, expanded_x) = match edge {
        DockEdge::Left => (work_area_phys.x, work_area_phys.x),
        DockEdge::Right => {
            let right_edge = work_area_phys.x + work_area_phys.width as i32;
            (
                right_edge - collapsed_w_phys as i32,
                right_edge - expanded_w_phys as i32,
            )
        }
    };

    let collapsed_y = work_area_phys.y + (clamped_top_logical * scale).round() as i32;
    let expanded_y = work_area_phys.y + (expanded_top_logical * scale).round() as i32;

    ComputedDockLayout {
        collapsed_rect_phys: PhysRect {
            x: collapsed_x,
            y: collapsed_y,
            width: collapsed_w_phys,
            height: collapsed_h_phys,
        },
        expanded_rect_phys: PhysRect {
            x: expanded_x,
            y: expanded_y,
            width: expanded_w_phys,
            height: expanded_h_phys,
        },
        clamped_top_logical,
        expanded_top_logical,
        dock_height_logical,
        handle_offset_y_logical,
    }
}

/// Pure clipping region math deriving `dock_rect` and `handle_rect` in physical pixels
/// from read-back `outer_size`, ensuring zero gap or overshoot.
#[must_use]
pub fn compute_expanded_window_region(
    total_w_phys: u32,
    total_h_phys: u32,
    scale_factor: f64,
    edge: DockEdge,
    handle_offset_y_logical: f64,
) -> ExpandedWindowRegionRects {
    let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    let total_w = (total_w_phys as i32).max(2);
    let total_h = (total_h_phys as i32).max(1);
    let dock_w_phys = ((DOCK_PANEL_WIDTH_LOGICAL * scale).round() as i32).clamp(1, total_w - 1);
    let handle_w_phys = total_w - dock_w_phys;

    let handle_h_phys = ((HANDLE_HEIGHT_LOGICAL * scale).round() as i32).clamp(1, total_h);
    let max_handle_y = (total_h - handle_h_phys).max(0);
    let safe_offset = if handle_offset_y_logical.is_finite() {
        handle_offset_y_logical
    } else {
        0.0
    };
    let handle_y_phys = ((safe_offset * scale).round() as i32).clamp(0, max_handle_y);

    match edge {
        DockEdge::Left => ExpandedWindowRegionRects {
            dock_rect: RegionRect {
                left: 0,
                top: 0,
                right: dock_w_phys,
                bottom: total_h,
            },
            handle_rect: RegionRect {
                left: dock_w_phys,
                top: handle_y_phys,
                right: total_w,
                bottom: handle_y_phys + handle_h_phys,
            },
        },
        DockEdge::Right => ExpandedWindowRegionRects {
            handle_rect: RegionRect {
                left: 0,
                top: handle_y_phys,
                right: handle_w_phys,
                bottom: handle_y_phys + handle_h_phys,
            },
            dock_rect: RegionRect {
                left: handle_w_phys,
                top: 0,
                right: total_w,
                bottom: total_h,
            },
        },
    }
}

/// Enumerates connected monitors for the Settings monitor selector.
pub fn list_monitors_info(app: &AppHandle) -> AppResult<Vec<MonitorDescriptor>> {
    let window = main_window(app)?;
    let monitors = window.available_monitors()?;
    let primary = window.primary_monitor()?;

    let mut result = Vec::with_capacity(monitors.len());
    for (idx, mon) in monitors.into_iter().enumerate() {
        let name = monitor_identifier(&mon, idx);
        let is_primary = primary.as_ref().is_some_and(|p| monitors_match(p, &mon));
        let scale = mon.scale_factor();
        let size = mon.size();
        let label = if is_primary {
            format!("Display {} (Primary) — {}x{} ({:.0}%)", idx + 1, size.width, size.height, scale * 100.0)
        } else {
            format!("Display {} — {}x{} ({:.0}%)", idx + 1, size.width, size.height, scale * 100.0)
        };
        result.push(MonitorDescriptor {
            name,
            label,
            width: size.width,
            height: size.height,
            scale_factor: scale,
            is_primary,
        });
    }
    Ok(result)
}

pub fn main_window(app: &AppHandle) -> AppResult<WebviewWindow> {
    app.get_webview_window("main")
        .ok_or_else(|| AppError::Window("Main dock window 'main' not found".to_owned()))
}

fn resolve_active_work_area(
    window: &WebviewWindow,
    preferred_name: Option<&str>,
) -> AppResult<(PhysicalWorkArea, f64)> {
    let monitors = window.available_monitors()?;
    let matched = preferred_name.and_then(|pref| {
        monitors
            .iter()
            .enumerate()
            .find(|(idx, mon)| monitor_identifier(mon, *idx).eq_ignore_ascii_case(pref))
            .map(|(_, mon)| mon.clone())
    });

    let active_mon = match matched {
        Some(m) => Some(m),
        None => window
            .primary_monitor()?
            .or_else(|| window.current_monitor().ok().flatten())
            .or_else(|| monitors.into_iter().next()),
    };

    let Some(monitor) = active_mon else {
        return Err(AppError::Window("No display monitor available".to_owned()));
    };

    let scale = monitor.scale_factor();
    let wa = monitor.work_area();
    if wa.size.width > 0 && wa.size.height > 0 {
        return Ok((
            PhysicalWorkArea {
                x: wa.position.x,
                y: wa.position.y,
                width: wa.size.width,
                height: wa.size.height,
            },
            scale,
        ));
    }

    // Fallback to Win32 GetMonitorInfoW rcWork if Tauri reported an empty work area.
    let center_x = monitor.position().x + (monitor.size().width as i32 / 2);
    let center_y = monitor.position().y + (monitor.size().height as i32 / 2);
    let rect = win_shell::get_monitor_work_area_at(center_x, center_y)?;
    Ok((
        PhysicalWorkArea {
            x: rect.left,
            y: rect.top,
            width: (rect.right - rect.left).max(1) as u32,
            height: (rect.bottom - rect.top).max(1) as u32,
        },
        scale,
    ))
}

fn monitor_identifier(mon: &Monitor, idx: usize) -> String {
    mon.name()
        .cloned()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| format!("Display-{}", idx + 1))
}

fn monitors_match(a: &Monitor, b: &Monitor) -> bool {
    a.position() == b.position() && a.size() == b.size() && a.name() == b.name()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_top_position_to_min_and_max_extremes() {
        let wa = PhysicalWorkArea {
            x: 0,
            y: 0,
            width: 1920,
            height: 1032,
        };

        let low = compute_dock_layout(wa, 1.0, DockEdge::Left, -500.0);
        assert_eq!(low.clamped_top_logical, 8.0);
        assert_eq!(low.collapsed_rect_phys.y, 8);
        assert_eq!(low.expanded_top_logical, 8.0);
        assert_eq!(low.handle_offset_y_logical, 0.0);

        // Max top = 1032 - 84 - 8 = 940.
        let high = compute_dock_layout(wa, 1.0, DockEdge::Left, 9999.0);
        assert_eq!(high.clamped_top_logical, 940.0);
        assert_eq!(high.collapsed_rect_phys.y, 940);
        // Max expanded top = 1032 - 640 - 8 = 384; handle_offset_y = 940 - 384 = 556.
        assert_eq!(high.expanded_top_logical, 384.0);
        assert_eq!(high.handle_offset_y_logical, 556.0);
    }

    #[test]
    fn clamps_dock_height_on_short_work_area() {
        let short_wa = PhysicalWorkArea {
            x: 0,
            y: 40,
            width: 1280,
            height: 540,
        };
        let layout = compute_dock_layout(short_wa, 1.0, DockEdge::Left, 200.0);
        assert_eq!(layout.dock_height_logical, 516.0);
        assert!(layout.dock_height_logical < MAX_DOCK_HEIGHT_LOGICAL);
        assert_eq!(layout.expanded_rect_phys.height, 516);
        assert_eq!(layout.expanded_top_logical, 8.0);
        assert_eq!(layout.expanded_rect_phys.y, 48);

        let layout_bottom = compute_dock_layout(short_wa, 1.0, DockEdge::Left, 240.0);
        assert_eq!(layout_bottom.expanded_top_logical, 16.0);
        assert_eq!(layout_bottom.expanded_rect_phys.y, 56);
    }

    #[test]
    fn aligns_right_edge_flush_to_work_area_boundary() {
        let wa = PhysicalWorkArea {
            x: 1920,
            y: 0,
            width: 2560,
            height: 1400,
        };
        let layout = compute_dock_layout(wa, 1.0, DockEdge::Right, 300.0);
        assert_eq!(
            layout.collapsed_rect_phys.x + layout.collapsed_rect_phys.width as i32,
            1920 + 2560
        );
        assert_eq!(
            layout.expanded_rect_phys.x + layout.expanded_rect_phys.width as i32,
            1920 + 2560
        );
        assert_eq!(layout.collapsed_rect_phys.width, 24);
        assert_eq!(layout.collapsed_rect_phys.height, 84);
        assert_eq!(layout.expanded_rect_phys.width, 404);
    }

    #[test]
    fn computes_exact_physical_geometry_across_dpi_scale_factors() {
        let wa = PhysicalWorkArea {
            x: 100,
            y: 50,
            width: 3840,
            height: 2080,
        };

        for scale in [1.0_f64, 1.25_f64, 1.5_f64, 2.0_f64] {
            let layout = compute_dock_layout(wa, scale, DockEdge::Right, 160.0);
            assert_eq!(
                layout.collapsed_rect_phys.width,
                (24.0 * scale).round() as u32
            );
            assert_eq!(
                layout.collapsed_rect_phys.height,
                (84.0 * scale).round() as u32
            );
            assert_eq!(
                layout.expanded_rect_phys.width,
                (404.0 * scale).round() as u32
            );
            assert_eq!(
                layout.collapsed_rect_phys.x + layout.collapsed_rect_phys.width as i32,
                wa.x + wa.width as i32
            );
            assert_eq!(
                layout.expanded_rect_phys.x + layout.expanded_rect_phys.width as i32,
                wa.x + wa.width as i32
            );
            // Expanded handle position must match collapsed handle screen Y.
            let expanded_handle_top_logical =
                layout.expanded_top_logical + layout.handle_offset_y_logical;
            assert!((expanded_handle_top_logical - layout.clamped_top_logical).abs() < 1e-6);
        }
    }

    #[test]
    fn computes_expanded_region_rects_for_left_right_scales_fractional_and_extremes() {
        for edge in [DockEdge::Left, DockEdge::Right] {
            for scale in [1.0_f64, 1.25_f64, 1.5_f64, 2.0_f64] {
                // Fractional dock_h (e.g. short work area 516.8 logical px).
                let dock_h_logical = 516.8_f64;
                let total_w = (EXPANDED_WIDTH_LOGICAL * scale).round() as u32;
                let total_h = (dock_h_logical * scale).round() as u32;
                let expected_dock_w = (DOCK_PANEL_WIDTH_LOGICAL * scale).round() as i32;
                let expected_handle_h = (HANDLE_HEIGHT_LOGICAL * scale).round() as i32;

                // Extreme 1: handle_offset_y = 0.0
                let top_extreme =
                    compute_expanded_window_region(total_w, total_h, scale, edge, 0.0);
                // Extreme 2: handle_offset_y = dock_h - 84.0
                let bottom_extreme = compute_expanded_window_region(
                    total_w,
                    total_h,
                    scale,
                    edge,
                    dock_h_logical - HANDLE_HEIGHT_LOGICAL,
                );

                for rgn in [top_extreme, bottom_extreme] {
                    assert_eq!(rgn.dock_rect.top, 0);
                    assert_eq!(rgn.dock_rect.bottom, total_h as i32);
                    assert_eq!(
                        rgn.handle_rect.bottom - rgn.handle_rect.top,
                        expected_handle_h
                    );
                    assert!(rgn.handle_rect.top >= 0);
                    assert!(rgn.handle_rect.bottom <= total_h as i32);

                    match edge {
                        DockEdge::Left => {
                            assert_eq!(rgn.dock_rect.left, 0);
                            assert_eq!(rgn.dock_rect.right, expected_dock_w);
                            assert_eq!(rgn.handle_rect.left, rgn.dock_rect.right);
                            assert_eq!(rgn.handle_rect.right, total_w as i32);
                        }
                        DockEdge::Right => {
                            assert_eq!(rgn.handle_rect.left, 0);
                            assert_eq!(rgn.handle_rect.right, rgn.dock_rect.left);
                            assert_eq!(rgn.dock_rect.right - rgn.dock_rect.left, expected_dock_w);
                            assert_eq!(rgn.dock_rect.right, total_w as i32);
                        }
                    }
                }

                assert_eq!(top_extreme.handle_rect.top, 0);
                assert_eq!(bottom_extreme.handle_rect.bottom, total_h as i32);
            }
        }
    }
}
