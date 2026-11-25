use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_global_shortcut::GlobalShortcutExt;

use crate::config::ConfigManager;
use crate::error::{AppError, AppResult};
use crate::models::{
    normalize_path_key, Category, DockConfig, DockEdge, DockItem, ItemSource, RescanResult,
};
use crate::scanner::{
    discover_live, generate_manual_id, merge_discovered_items, WindowsScanHost,
};
use crate::win_shell::{self, is_non_file_uri, WinShell};
use crate::window_manager::{
    main_window, DragEndResult, MonitorDescriptor, WindowCoordinator, WindowLayoutState,
    EVENT_HOTKEY_WARNING,
};

#[derive(Debug, Clone, Serialize)]
pub struct InitialStatePayload {
    pub config: DockConfig,
    pub layout: WindowLayoutState,
    pub monitors: Vec<MonitorDescriptor>,
    pub hotkey_warning: Option<String>,
    pub is_portable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SystemToolKind {
    Taskmgr,
    Terminal,
    Lock,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewItemInput {
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub arguments: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub category: Category,
    #[serde(default)]
    pub custom_icon_path: Option<String>,
    #[serde(default)]
    pub run_as_admin: bool,
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateItemInput {
    pub id: String,
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub arguments: String,
    #[serde(default)]
    pub description: String,
    pub category: Category,
    #[serde(default)]
    pub custom_icon_path: Option<String>,
    #[serde(default)]
    pub run_as_admin: bool,
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SettingsUpdate {
    pub edge: DockEdge,
    pub monitor_name: Option<String>,
    pub auto_collapse_on_launch: bool,
    pub auto_collapse_on_blur: bool,
    pub always_on_top: bool,
    pub start_with_windows: bool,
    pub global_hotkey: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateSettingsResult {
    pub config: DockConfig,
    pub layout: WindowLayoutState,
    pub hotkey_warning: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RescanPayload {
    pub rescan: RescanResult,
    pub config: DockConfig,
}

#[derive(Debug, Clone, Serialize)]
pub struct DropPinResult {
    pub added: usize,
    pub skipped: usize,
    pub config: DockConfig,
}

#[tauri::command]
pub fn get_initial_state(
    app: AppHandle,
    config_mgr: State<'_, ConfigManager>,
    coord: State<'_, WindowCoordinator>,
) -> AppResult<InitialStatePayload> {
    let config = config_mgr.snapshot();
    let layout = coord.current_layout_state(&app, &config_mgr)?;
    let monitors = crate::window_manager::list_monitors_info(&app)?;
    let hotkey_warning = coord.hotkey_warning();
    let is_portable = config_mgr.paths().is_portable;
    Ok(InitialStatePayload {
        config,
        layout,
        monitors,
        hotkey_warning,
        is_portable,
    })
}

#[tauri::command]
pub fn list_monitors(app: AppHandle) -> AppResult<Vec<MonitorDescriptor>> {
    crate::window_manager::list_monitors_info(&app)
}

#[tauri::command]
pub fn expand_dock(
    app: AppHandle,
    config_mgr: State<'_, ConfigManager>,
    coord: State<'_, WindowCoordinator>,
) -> AppResult<WindowLayoutState> {
    coord.expand_dock(&app, &config_mgr)
}

#[tauri::command]
pub fn request_collapse_dock(
    app: AppHandle,
    config_mgr: State<'_, ConfigManager>,
    coord: State<'_, WindowCoordinator>,
) -> AppResult<WindowLayoutState> {
    coord.request_collapse_dock(&app, &config_mgr)
}

#[tauri::command]
pub fn commit_collapse_window(
    app: AppHandle,
    epoch: u64,
    config_mgr: State<'_, ConfigManager>,
    coord: State<'_, WindowCoordinator>,
) -> AppResult<bool> {
    coord.commit_collapse_window(&app, &config_mgr, epoch)
}

#[tauri::command]
pub fn toggle_dock(
    app: AppHandle,
    config_mgr: State<'_, ConfigManager>,
    coord: State<'_, WindowCoordinator>,
) -> AppResult<WindowLayoutState> {
    coord.toggle_dock(&app, &config_mgr)
}

#[tauri::command]
pub fn set_pinned(
    app: AppHandle,
    pinned: bool,
    config_mgr: State<'_, ConfigManager>,
    coord: State<'_, WindowCoordinator>,
) -> AppResult<WindowLayoutState> {
    coord.set_pinned(&app, &config_mgr, pinned)
}

#[tauri::command]
pub fn set_edge(
    app: AppHandle,
    edge: DockEdge,
    config_mgr: State<'_, ConfigManager>,
    coord: State<'_, WindowCoordinator>,
) -> AppResult<WindowLayoutState> {
    config_mgr.update_and_save(|cfg| {
        cfg.edge = edge;
        Ok(())
    })?;
    coord.update_tray_edge_label(edge);
    coord.reapply_current_layout(&app, &config_mgr)
}

#[tauri::command]
pub fn set_selected_category(
    category: String,
    config_mgr: State<'_, ConfigManager>,
) -> AppResult<DockConfig> {
    let trimmed = category.trim();
    if trimmed.is_empty() {
        return Err(AppError::InvalidInput("Category cannot be empty".to_owned()));
    }
    config_mgr.update_and_save(|cfg| {
        cfg.selected_category = trimmed.to_owned();
        Ok(cfg.clone())
    })
}

#[tauri::command]
pub fn start_handle_drag(
    app: AppHandle,
    config_mgr: State<'_, ConfigManager>,
    coord: State<'_, WindowCoordinator>,
) -> AppResult<()> {
    coord.start_handle_drag(&app, &config_mgr)
}

#[tauri::command]
pub fn end_handle_drag(
    app: AppHandle,
    config_mgr: State<'_, ConfigManager>,
    coord: State<'_, WindowCoordinator>,
) -> AppResult<DragEndResult> {
    coord.end_handle_drag(&app, &config_mgr)
}

#[tauri::command]
pub async fn launch_item(
    app: AppHandle,
    id: String,
    run_as_admin: Option<bool>,
    config_mgr: State<'_, ConfigManager>,
    shell: State<'_, WinShell>,
    coord: State<'_, WindowCoordinator>,
) -> AppResult<DockConfig> {
    let item = config_mgr
        .snapshot()
        .items
        .into_iter()
        .find(|i| i.id == id)
        .ok_or_else(|| AppError::NotFound(format!("Shortcut '{id}' not found")))?;

    let effective_admin = run_as_admin.unwrap_or(item.run_as_admin);
    shell
        .launch_target(&item.path, &item.arguments, effective_admin)
        .await?;

    let updated_cfg = config_mgr.update_and_save(|cfg| {
        if let Some(target_item) = cfg.items.iter_mut().find(|i| i.id == id) {
            target_item.launch_count = target_item.launch_count.saturating_add(1);
        }
        Ok(cfg.clone())
    })?;

    if updated_cfg.auto_collapse_on_launch && !coord.is_pinned() {
        let _ = coord.request_collapse_dock(&app, &config_mgr);
    }

    Ok(updated_cfg)
}

#[tauri::command]
pub async fn launch_system_tool(
    tool: SystemToolKind,
    shell: State<'_, WinShell>,
) -> AppResult<()> {
    match tool {
        SystemToolKind::Taskmgr => shell.launch_target("taskmgr.exe", "", false).await,
        SystemToolKind::Terminal => {
            let wt_path = std::env::var_os("LOCALAPPDATA")
                .map(|dir| PathBuf::from(dir).join("Microsoft\\WindowsApps\\wt.exe"))
                .filter(|p| p.is_file());
            if let Some(wt) = wt_path {
                if shell
                    .launch_target(&wt.to_string_lossy(), "", false)
                    .await
                    .is_ok()
                {
                    return Ok(());
                }
            }
            shell.launch_target("cmd.exe", "", false).await
        }
        SystemToolKind::Lock => win_shell::lock_workstation(),
    }
}

#[tauri::command]
pub async fn open_file_location(
    id: String,
    config_mgr: State<'_, ConfigManager>,
    shell: State<'_, WinShell>,
) -> AppResult<()> {
    let item = config_mgr
        .snapshot()
        .items
        .into_iter()
        .find(|i| i.id == id)
        .ok_or_else(|| AppError::NotFound(format!("Shortcut '{id}' not found")))?;

    let Some(target) = item.file_location_target() else {
        return Err(AppError::NotFound(
            "No local executable path is recorded for this shortcut".to_owned(),
        ));
    };

    shell.open_file_location(target).await
}

#[tauri::command]
pub async fn get_item_icon(
    id: String,
    config_mgr: State<'_, ConfigManager>,
    shell: State<'_, WinShell>,
) -> AppResult<Option<String>> {
    let item = config_mgr
        .snapshot()
        .items
        .into_iter()
        .find(|i| i.id == id);

    let Some(item) = item else {
        return Ok(None);
    };

    let lookup = item.icon_lookup_path().to_owned();
    shell
        .extract_icon_cached(&lookup, &config_mgr.paths().icon_cache_dir)
        .await
}

#[tauri::command]
pub fn add_item(
    input: NewItemInput,
    config_mgr: State<'_, ConfigManager>,
) -> AppResult<DockConfig> {
    let name = input.name.trim().to_owned();
    let path = input.path.trim().trim_matches('"').to_owned();
    if name.is_empty() {
        return Err(AppError::InvalidInput("Shortcut name is required".to_owned()));
    }
    if path.is_empty() {
        return Err(AppError::InvalidInput("Target path is required".to_owned()));
    }

    let item = DockItem {
        id: generate_manual_id(&path),
        name,
        path,
        arguments: input.arguments.trim().to_owned(),
        description: input.description.trim().to_owned(),
        category: input.category,
        custom_icon_path: normalize_optional_string(input.custom_icon_path),
        icon_source_path: None,
        run_as_admin: input.run_as_admin,
        pinned: input.pinned,
        source: ItemSource::Manual,
        steam_app_id: None,
        launch_count: 0,
    };

    config_mgr.add_manual_item(item)
}

#[tauri::command]
pub fn update_item(
    input: UpdateItemInput,
    config_mgr: State<'_, ConfigManager>,
) -> AppResult<DockConfig> {
    let name = input.name.trim().to_owned();
    let path = input.path.trim().trim_matches('"').to_owned();
    if name.is_empty() {
        return Err(AppError::InvalidInput("Shortcut name is required".to_owned()));
    }

    config_mgr.update_and_save(|cfg| {
        let Some(existing) = cfg.items.iter_mut().find(|i| i.id == input.id) else {
            return Err(AppError::NotFound(format!("Shortcut '{}' not found", input.id)));
        };

        existing.name = name;
        if existing.source != ItemSource::Steam {
            if path.is_empty() {
                return Err(AppError::InvalidInput("Target path is required".to_owned()));
            }
            existing.path = path;
        }
        existing.arguments = input.arguments.trim().to_owned();
        existing.description = input.description.trim().to_owned();
        existing.category = input.category;
        existing.custom_icon_path = normalize_optional_string(input.custom_icon_path);
        existing.run_as_admin = input.run_as_admin;
        existing.pinned = input.pinned;
        Ok(cfg.clone())
    })
}

#[tauri::command]
pub fn remove_item(id: String, config_mgr: State<'_, ConfigManager>) -> AppResult<DockConfig> {
    let removed = config_mgr.remove_item_and_dismiss(&id)?;
    if removed.is_none() {
        return Err(AppError::NotFound(format!("Shortcut '{id}' not found")));
    }
    Ok(config_mgr.snapshot())
}

#[tauri::command]
pub fn toggle_item_pin(id: String, config_mgr: State<'_, ConfigManager>) -> AppResult<DockConfig> {
    config_mgr.update_and_save(|cfg| {
        let Some(item) = cfg.items.iter_mut().find(|i| i.id == id) else {
            return Err(AppError::NotFound(format!("Shortcut '{id}' not found")));
        };
        item.pinned = !item.pinned;
        Ok(cfg.clone())
    })
}

#[tauri::command]
pub fn move_item_category(
    id: String,
    category: Category,
    config_mgr: State<'_, ConfigManager>,
) -> AppResult<DockConfig> {
    config_mgr.update_and_save(|cfg| {
        let Some(item) = cfg.items.iter_mut().find(|i| i.id == id) else {
            return Err(AppError::NotFound(format!("Shortcut '{id}' not found")));
        };
        item.category = category;
        Ok(cfg.clone())
    })
}

#[tauri::command]
pub fn reorder_items(
    category: Category,
    pinned: bool,
    ordered_ids: Vec<String>,
    config_mgr: State<'_, ConfigManager>,
) -> AppResult<DockConfig> {
    config_mgr.update_and_save(|cfg| {
        let matching_indices: Vec<usize> = cfg
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.category == category && item.pinned == pinned)
            .map(|(idx, _)| idx)
            .collect();

        if matching_indices.len() <= 1 {
            return Ok(cfg.clone());
        }

        let mut pool: HashMap<String, DockItem> = HashMap::new();
        let mut original_order = Vec::with_capacity(matching_indices.len());
        for &idx in &matching_indices {
            let item = cfg.items[idx].clone();
            original_order.push(item.id.clone());
            pool.insert(item.id.clone(), item);
        }

        let mut reordered = Vec::with_capacity(matching_indices.len());
        for wanted_id in ordered_ids {
            if let Some(item) = pool.remove(&wanted_id) {
                reordered.push(item);
            }
        }
        // Preserve any remaining matching items not included in `ordered_ids`.
        for orig_id in original_order {
            if let Some(item) = pool.remove(&orig_id) {
                reordered.push(item);
            }
        }

        for (slot, item) in matching_indices.into_iter().zip(reordered) {
            cfg.items[slot] = item;
        }

        Ok(cfg.clone())
    })
}

#[tauri::command]
pub async fn rescan_shortcuts(
    config_mgr: State<'_, ConfigManager>,
    shell: State<'_, WinShell>,
    coord: State<'_, WindowCoordinator>,
) -> AppResult<RescanPayload> {
    let Some(_guard) = coord.try_begin_rescan() else {
        return Err(AppError::InvalidInput(
            "A system rescan is already in progress".to_owned(),
        ));
    };

    let scan = discover_live(&shell).await;
    let host = WindowsScanHost;

    let (rescan, config) = config_mgr.update_and_save(|cfg| {
        let rescan = merge_discovered_items(cfg, &scan, &host);
        Ok((rescan, cfg.clone()))
    })?;

    Ok(RescanPayload { rescan, config })
}

#[tauri::command]
pub async fn reset_to_defaults(
    config_mgr: State<'_, ConfigManager>,
    shell: State<'_, WinShell>,
    coord: State<'_, WindowCoordinator>,
) -> AppResult<DockConfig> {
    let Some(_guard) = coord.try_begin_rescan() else {
        return Err(AppError::InvalidInput(
            "A system rescan is already in progress".to_owned(),
        ));
    };

    let scan = discover_live(&shell).await;
    config_mgr.reset_to_discovered(scan.discovered)
}

#[tauri::command]
pub fn dismiss_missing_items(
    ids: Vec<String>,
    config_mgr: State<'_, ConfigManager>,
) -> AppResult<DockConfig> {
    let id_set: HashSet<String> = ids.into_iter().collect();
    if id_set.is_empty() {
        return Ok(config_mgr.snapshot());
    }

    config_mgr.update_and_save(|cfg| {
        let mut new_dismissals = Vec::new();
        cfg.items.retain(|item| {
            if id_set.contains(&item.id) {
                let key = item.dismissal_key();
                if !key.is_empty() {
                    new_dismissals.push(key);
                }
                false
            } else {
                true
            }
        });

        for key in new_dismissals {
            if !cfg.dismissed.iter().any(|existing| existing == &key) {
                cfg.dismissed.push(key);
            }
        }
        Ok(cfg.clone())
    })
}

#[tauri::command]
pub fn update_settings(
    app: AppHandle,
    settings: SettingsUpdate,
    config_mgr: State<'_, ConfigManager>,
    coord: State<'_, WindowCoordinator>,
) -> AppResult<UpdateSettingsResult> {
    let new_hotkey = settings.global_hotkey.trim().to_owned();
    if new_hotkey.is_empty() {
        return Err(AppError::Hotkey("Global hotkey cannot be empty".to_owned()));
    }

    let old_cfg = config_mgr.snapshot();
    let prev_registered = coord.registered_hotkey();
    let need_rebind =
        new_hotkey != old_cfg.global_hotkey || prev_registered.as_deref() != Some(new_hotkey.as_str());

    if need_rebind {
        let gs = app.global_shortcut();
        if let Some(prev) = prev_registered.as_deref() {
            let _ = gs.unregister(prev);
        }
        if let Err(err) = gs.register(new_hotkey.as_str()) {
            // Roll back previous shortcut so user is never left without their working hotkey.
            if let Some(prev) = prev_registered.as_deref() {
                let _ = gs.register(prev);
            }
            return Err(AppError::Hotkey(format!(
                "Failed to register global shortcut '{new_hotkey}': {err}"
            )));
        }
        coord.set_hotkey_state(Some(new_hotkey.clone()), None);
        let _ = app.emit(EVENT_HOTKEY_WARNING, Option::<String>::None);
    }

    if settings.start_with_windows != old_cfg.start_with_windows {
        win_shell::set_start_with_windows(settings.start_with_windows)?;
    }

    let updated_cfg = config_mgr.update_and_save(|cfg| {
        cfg.edge = settings.edge;
        cfg.monitor_name = normalize_optional_string(settings.monitor_name);
        cfg.auto_collapse_on_launch = settings.auto_collapse_on_launch;
        cfg.auto_collapse_on_blur = settings.auto_collapse_on_blur;
        cfg.always_on_top = settings.always_on_top;
        cfg.start_with_windows = settings.start_with_windows;
        cfg.global_hotkey = new_hotkey;
        Ok(cfg.clone())
    })?;

    if updated_cfg.edge != old_cfg.edge {
        coord.update_tray_edge_label(updated_cfg.edge);
    }

    let layout = coord.reapply_current_layout(&app, &config_mgr)?;
    let hotkey_warning = coord.hotkey_warning();

    Ok(UpdateSettingsResult {
        config: updated_cfg,
        layout,
        hotkey_warning,
    })
}

#[tauri::command]
pub async fn pick_executable_file(
    app: AppHandle,
    coord: State<'_, WindowCoordinator>,
) -> AppResult<Option<String>> {
    let _dialog_guard = coord.begin_dialog();
    let window = main_window(&app)?;

    let picked = tokio::task::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Select Program or Shortcut")
            .add_filter("Programs & Shortcuts", &["exe", "lnk", "bat", "cmd", "msc"])
            .add_filter("All Files", &["*"])
            .set_parent(&window)
            .blocking_pick_file()
    })
    .await
    .map_err(|err| AppError::Dialog(format!("File dialog task failed: {err}")))?;

    match picked {
        Some(file_path) => {
            let path = file_path
                .into_path()
                .map_err(|err| AppError::Dialog(format!("Invalid selected file path: {err}")))?;
            Ok(Some(path.to_string_lossy().into_owned()))
        }
        None => Ok(None),
    }
}

#[tauri::command]
pub async fn pick_icon_file(
    app: AppHandle,
    coord: State<'_, WindowCoordinator>,
) -> AppResult<Option<String>> {
    let _dialog_guard = coord.begin_dialog();
    let window = main_window(&app)?;

    let picked = tokio::task::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Select Custom Icon")
            .add_filter("Icons & Executables", &["png", "ico", "exe", "dll"])
            .add_filter("All Files", &["*"])
            .set_parent(&window)
            .blocking_pick_file()
    })
    .await
    .map_err(|err| AppError::Dialog(format!("Icon dialog task failed: {err}")))?;

    match picked {
        Some(file_path) => {
            let path = file_path
                .into_path()
                .map_err(|err| AppError::Dialog(format!("Invalid selected icon path: {err}")))?;
            Ok(Some(path.to_string_lossy().into_owned()))
        }
        None => Ok(None),
    }
}

#[tauri::command]
pub async fn pin_dropped_files(
    paths: Vec<String>,
    category: Category,
    config_mgr: State<'_, ConfigManager>,
    shell: State<'_, WinShell>,
) -> AppResult<DropPinResult> {
    let mut prepared: Vec<DockItem> = Vec::new();
    let mut skipped = 0_usize;

    for raw in paths {
        let trimmed = raw.trim().trim_matches('"');
        if trimmed.is_empty() || is_non_file_uri(trimmed) {
            skipped += 1;
            continue;
        }
        let path = Path::new(trimmed);
        if !path.exists() {
            skipped += 1;
            continue;
        }

        let display_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("Shortcut")
            .to_owned();

        let is_lnk = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("lnk"));

        let (final_target, args, desc) = if is_lnk {
            match shell.resolve_lnk(path.to_path_buf()).await {
                Ok(Some(resolved)) => {
                    let target_path = resolved.target_path.trim().trim_matches('"').to_owned();
                    if target_path.is_empty()
                        || (!is_non_file_uri(&target_path) && !Path::new(&target_path).exists())
                    {
                        skipped += 1;
                        continue;
                    }
                    (target_path, resolved.arguments, resolved.description)
                }
                _ => {
                    skipped += 1;
                    continue;
                }
            }
        } else {
            (trimmed.to_owned(), String::new(), String::new())
        };

        let target_key = canonicalize_or_norm(&final_target);
        if prepared
            .iter()
            .any(|item| canonicalize_or_norm(&item.path) == target_key)
        {
            skipped += 1;
            continue;
        }

        prepared.push(DockItem {
            id: generate_manual_id(&final_target),
            name: display_name,
            path: final_target,
            arguments: args,
            description: desc,
            category,
            custom_icon_path: None,
            icon_source_path: None,
            run_as_admin: false,
            pinned: true,
            source: ItemSource::Manual,
            steam_app_id: None,
            launch_count: 0,
        });
    }

    let (added, final_skipped, config) = config_mgr.update_and_save(|cfg| {
        let mut existing_keys: HashSet<String> = cfg
            .items
            .iter()
            .map(|i| canonicalize_or_norm(&i.path))
            .collect();

        let mut added_count = 0_usize;
        let mut local_skipped = skipped;

        for item in prepared {
            let key = canonicalize_or_norm(&item.path);
            let norm_key = normalize_path_key(&item.path);
            if !existing_keys.insert(key.clone()) {
                local_skipped += 1;
                continue;
            }
            cfg.dismissed.retain(|d| d != &key && d != &norm_key);
            cfg.items.push(item);
            added_count += 1;
        }

        Ok((added_count, local_skipped, cfg.clone()))
    })?;

    Ok(DropPinResult {
        added,
        skipped: final_skipped,
        config,
    })
}

fn canonicalize_or_norm(raw: &str) -> String {
    if is_non_file_uri(raw) {
        return normalize_path_key(raw);
    }
    let path = Path::new(raw.trim().trim_matches('"'));
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    normalize_path_key(&resolved.to_string_lossy())
}

fn normalize_optional_string(opt: Option<String>) -> Option<String> {
    opt.and_then(|s| {
        let trimmed = s.trim().trim_matches('"').to_owned();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}
