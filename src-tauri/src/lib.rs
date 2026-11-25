#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]

pub mod commands;
pub mod config;
pub mod error;
pub mod logging;
pub mod models;
pub mod scanner;
pub mod win_shell;
pub mod window_manager;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, DragDropEvent, Emitter, Manager, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::config::{AppPaths, ConfigManager};
use crate::models::DockEdge;
use crate::win_shell::WinShell;
use crate::window_manager::{
    WindowCoordinator, EVENT_FILE_DROP, EVENT_FILE_DROP_HOVER, EVENT_FILE_DROP_LEAVE,
    EVENT_HOTKEY_WARNING,
};

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    // Register single-instance FIRST so secondary launches expand and focus the running instance.
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            let coord = app.state::<WindowCoordinator>();
            let cfg_mgr = app.state::<ConfigManager>();
            if let Err(err) = coord.expand_dock(app, &cfg_mgr) {
                tracing::warn!(error = %err, "Failed to expand dock from single-instance activation");
            }
        }))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        let coord = app.state::<WindowCoordinator>();
                        let cfg_mgr = app.state::<ConfigManager>();
                        if let Err(err) = coord.toggle_dock(app, &cfg_mgr) {
                            tracing::warn!(error = %err, "Failed to toggle dock via global hotkey");
                        }
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let paths = AppPaths::resolve()?;
            paths.ensure_dirs()?;

            let log_guard = logging::init_logging(&paths.logs_dir)?;
            let (config_mgr, load_report) = ConfigManager::load_or_init(paths.clone())?;
            if let Some(backup) = &load_report.recovered_corrupt_path {
                tracing::warn!(
                    backup = %backup.display(),
                    "Recovered from corrupt config.json; old file backed up"
                );
            }

            let shell = WinShell::new()?;
            let coord = WindowCoordinator::new();

            app.asset_protocol_scope()
                .allow_directory(&paths.icon_cache_dir, true)?;

            app.manage(log_guard);
            app.manage(config_mgr);
            app.manage(shell);
            app.manage(coord);

            let main_cfg = app
                .config()
                .app
                .windows
                .iter()
                .find(|w| w.label == "main")
                .ok_or_else(|| {
                    crate::error::AppError::Window("Missing 'main' window config".to_owned())
                })?
                .clone();
            let mut win_builder = tauri::WebviewWindowBuilder::from_config(app.handle(), &main_cfg)?;
            if let Some(webview2_dir) = &paths.webview2_dir {
                win_builder = win_builder.data_directory(webview2_dir.clone());
            }
            win_builder.build()?;

            setup_tray(app.handle())?;
            register_initial_hotkey(app.handle());

            let coord_ref = app.state::<WindowCoordinator>();
            let cfg_ref = app.state::<ConfigManager>();
            coord_ref.apply_startup_collapsed(app.handle(), &cfg_ref)?;

            tracing::info!(
                portable = paths.is_portable,
                data_dir = %paths.data_dir.display(),
                webview2_dir = ?paths.webview2_dir,
                "Harbor initialized"
            );
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            let app = window.app_handle();
            match event {
                WindowEvent::Focused(false) => {
                    let coord = app.state::<WindowCoordinator>();
                    let cfg_mgr = app.state::<ConfigManager>();
                    coord.handle_window_blur(app, &cfg_mgr);
                }
                WindowEvent::ScaleFactorChanged { .. } => {
                    let coord = app.state::<WindowCoordinator>();
                    let cfg_mgr = app.state::<ConfigManager>();
                    coord.handle_scale_factor_changed(app, &cfg_mgr);
                }
                WindowEvent::DragDrop(dd_event) => match dd_event {
                    DragDropEvent::Enter { paths, .. } => {
                        let strs: Vec<String> = paths
                            .iter()
                            .map(|p| p.to_string_lossy().into_owned())
                            .collect();
                        let _ = app.emit(EVENT_FILE_DROP_HOVER, strs);
                    }
                    DragDropEvent::Over { .. } => {}
                    DragDropEvent::Leave => {
                        let _ = app.emit(EVENT_FILE_DROP_LEAVE, ());
                    }
                    DragDropEvent::Drop { paths, .. } => {
                        let strs: Vec<String> = paths
                            .iter()
                            .map(|p| p.to_string_lossy().into_owned())
                            .collect();
                        let _ = app.emit(EVENT_FILE_DROP, strs);
                    }
                    _ => {}
                },
                WindowEvent::CloseRequested { api, .. } => {
                    // Keep the dock resident; collapse instead of terminating on Alt+F4.
                    api.prevent_close();
                    let coord = app.state::<WindowCoordinator>();
                    let cfg_mgr = app.state::<ConfigManager>();
                    let _ = coord.request_collapse_dock(app, &cfg_mgr);
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_initial_state,
            commands::list_monitors,
            commands::expand_dock,
            commands::request_collapse_dock,
            commands::commit_collapse_window,
            commands::toggle_dock,
            commands::set_pinned,
            commands::set_edge,
            commands::set_selected_category,
            commands::start_handle_drag,
            commands::end_handle_drag,
            commands::launch_item,
            commands::launch_system_tool,
            commands::open_file_location,
            commands::get_item_icon,
            commands::add_item,
            commands::update_item,
            commands::remove_item,
            commands::toggle_item_pin,
            commands::move_item_category,
            commands::reorder_items,
            commands::rescan_shortcuts,
            commands::reset_to_defaults,
            commands::dismiss_missing_items,
            commands::update_settings,
            commands::pick_executable_file,
            commands::pick_icon_file,
            commands::pin_dropped_files,
        ]);

    builder.run(tauri::generate_context!())?;
    Ok(())
}

fn setup_tray(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let cfg = app.state::<ConfigManager>().snapshot();
    let edge_label = match cfg.edge {
        DockEdge::Left => "Switch to Right Edge",
        DockEdge::Right => "Switch to Left Edge",
    };

    let toggle_item = MenuItem::with_id(app, "toggle", "Expand / Collapse", true, None::<&str>)?;
    let switch_edge_item = MenuItem::with_id(app, "switch_edge", edge_label, true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit Harbor", true, None::<&str>)?;

    app.state::<WindowCoordinator>()
        .set_tray_edge_item(switch_edge_item.clone());

    let menu = Menu::with_items(app, &[&toggle_item, &switch_edge_item, &separator, &quit_item])?;

    let mut tray_builder = TrayIconBuilder::with_id("harbor-tray")
        .tooltip("Harbor")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "toggle" => {
                let coord = app.state::<WindowCoordinator>();
                let cfg_mgr = app.state::<ConfigManager>();
                if let Err(err) = coord.toggle_dock(app, &cfg_mgr) {
                    tracing::warn!(error = %err, "Tray toggle failed");
                }
            }
            "switch_edge" => {
                let coord = app.state::<WindowCoordinator>();
                let cfg_mgr = app.state::<ConfigManager>();
                let res = cfg_mgr.update_and_save(|cfg| {
                    cfg.edge = match cfg.edge {
                        DockEdge::Left => DockEdge::Right,
                        DockEdge::Right => DockEdge::Left,
                    };
                    Ok(cfg.edge)
                });
                match res {
                    Ok(new_edge) => {
                        coord.update_tray_edge_label(new_edge);
                        if let Err(err) = coord.reapply_current_layout(app, &cfg_mgr) {
                            tracing::warn!(error = %err, "Failed to reapply layout after edge switch");
                        }
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "Failed to persist tray edge switch");
                    }
                }
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                let coord = app.state::<WindowCoordinator>();
                let cfg_mgr = app.state::<ConfigManager>();
                if let Err(err) = coord.toggle_dock(app, &cfg_mgr) {
                    tracing::warn!(error = %err, "Tray left-click toggle failed");
                }
            }
        });

    if let Some(icon) = app.default_window_icon() {
        tray_builder = tray_builder.icon(icon.clone());
    }

    tray_builder.build(app)?;
    Ok(())
}

fn register_initial_hotkey(app: &AppHandle) {
    let hotkey = app.state::<ConfigManager>().snapshot().global_hotkey;
    let coord = app.state::<WindowCoordinator>();
    let gs = app.global_shortcut();

    match gs.register(hotkey.as_str()) {
        Ok(()) => {
            coord.set_hotkey_state(Some(hotkey), None);
        }
        Err(err) => {
            let warning = format!(
                "Global hotkey '{hotkey}' is unavailable ({err}). You can rebind it in Settings."
            );
            tracing::warn!(hotkey = %hotkey, error = %err, "Global hotkey collision on startup");
            coord.set_hotkey_state(None, Some(warning.clone()));
            let _ = app.emit(EVENT_HOTKEY_WARNING, Some(warning));
        }
    }
}
