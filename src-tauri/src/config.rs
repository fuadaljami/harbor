use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::{Mutex, RwLock};

use crate::error::{AppError, AppResult};
use crate::models::{
    build_dismissal_key, normalize_path_key, DockConfig, DockItem, CURRENT_SCHEMA_VERSION,
    DEFAULT_GLOBAL_HOTKEY,
};

const PORTABLE_FLAG_FILE: &str = "portable.flag";
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Resolved storage locations determined once at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    pub is_portable: bool,
    pub data_dir: PathBuf,
    pub config_path: PathBuf,
    pub logs_dir: PathBuf,
    pub icon_cache_dir: PathBuf,
}

impl AppPaths {
    /// Resolves portable vs. installed storage paths using the current executable directory
    /// and `%LOCALAPPDATA%`.
    pub fn resolve() -> AppResult<Self> {
        let exe_path = std::env::current_exe().map_err(|err| {
            AppError::Config(format!("Failed to locate executable path: {err}"))
        })?;
        let exe_dir = exe_path.parent().ok_or_else(|| {
            AppError::Config("Executable path has no parent directory".to_owned())
        })?;

        let local_app_data = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
        Self::resolve_from(exe_dir, local_app_data.as_deref())
    }

    /// Deterministic path resolution helper that checks for `<exe_dir>\portable.flag`
    /// without probing directory writability.
    pub fn resolve_from(exe_dir: &Path, local_app_data: Option<&Path>) -> AppResult<Self> {
        let portable_flag = exe_dir.join(PORTABLE_FLAG_FILE);
        let is_portable = portable_flag.exists();

        let data_dir = if is_portable {
            exe_dir.join("data")
        } else {
            let base = local_app_data.ok_or_else(|| {
                AppError::Config("LOCALAPPDATA environment variable is not set".to_owned())
            })?;
            base.join("Harbor")
        };

        Ok(Self {
            is_portable,
            config_path: data_dir.join("config.json"),
            logs_dir: data_dir.join("logs"),
            icon_cache_dir: data_dir.join("icon_cache"),
            data_dir,
        })
    }

    /// Creates `data_dir`, `logs_dir`, and `icon_cache_dir` if they do not already exist.
    pub fn ensure_dirs(&self) -> AppResult<()> {
        std::fs::create_dir_all(&self.data_dir)?;
        std::fs::create_dir_all(&self.logs_dir)?;
        std::fs::create_dir_all(&self.icon_cache_dir)?;
        Ok(())
    }
}

/// Metadata describing how configuration was loaded at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadReport {
    pub created_default: bool,
    pub recovered_corrupt_path: Option<PathBuf>,
}

/// Thread-safe configuration state manager backed by atomic JSON persistence.
pub struct ConfigManager {
    paths: AppPaths,
    state: RwLock<DockConfig>,
    save_lock: Mutex<()>,
}

impl ConfigManager {
    /// Loads existing `config.json` (recovering corrupt files to `.corrupt.<timestamp>`)
    /// or initializes a fresh default configuration.
    pub fn load_or_init(paths: AppPaths) -> AppResult<(Self, LoadReport)> {
        paths.ensure_dirs()?;

        let (config, report) = load_config_with_recovery(&paths.config_path)?;
        if report.created_default || report.recovered_corrupt_path.is_some() {
            save_config_atomic(&paths.config_path, &config)?;
        }

        let manager = Self {
            paths,
            state: RwLock::new(config),
            save_lock: Mutex::new(()),
        };
        Ok((manager, report))
    }

    /// Returns resolved storage paths.
    #[must_use]
    pub fn paths(&self) -> &AppPaths {
        &self.paths
    }

    /// Returns a cloned snapshot of the current configuration.
    #[must_use]
    pub fn snapshot(&self) -> DockConfig {
        self.state.read().clone()
    }

    /// Applies an in-memory mutation and persists the resulting config atomically.
    pub fn update_and_save<F, R>(&self, mutator: F) -> AppResult<R>
    where
        F: FnOnce(&mut DockConfig) -> AppResult<R>,
    {
        let _save_guard = self.save_lock.lock();
        let (result, snapshot) = {
            let mut guard = self.state.write();
            let res = mutator(&mut guard)?;
            sanitize_config(&mut guard);
            (res, guard.clone())
        };
        save_config_atomic(&self.paths.config_path, &snapshot)?;
        Ok(result)
    }

    /// Removes an item by ID and records its dismissal key so rescan does not re-add it.
    pub fn remove_item_and_dismiss(&self, item_id: &str) -> AppResult<Option<DockItem>> {
        self.update_and_save(|cfg| {
            let Some(pos) = cfg.items.iter().position(|item| item.id == item_id) else {
                return Ok(None);
            };
            let removed = cfg.items.remove(pos);
            let key = removed.dismissal_key();
            if !key.is_empty() && !cfg.dismissed.iter().any(|existing| existing == &key) {
                cfg.dismissed.push(key);
            }
            Ok(Some(removed))
        })
    }

    /// Adds a manual item and clears any prior dismissal key matching it.
    pub fn add_manual_item(&self, item: DockItem) -> AppResult<DockConfig> {
        self.update_and_save(|cfg| {
            let item_key = item.dismissal_key();
            let path_key = normalize_path_key(&item.path);
            cfg.dismissed
                .retain(|k| k != &item_key && k != &path_key);
            cfg.items.push(item);
            Ok(cfg.clone())
        })
    }

    /// Replaces discovered items and clears `dismissed` (for "Reset to Discovered Defaults").
    pub fn reset_to_discovered(&self, items: Vec<DockItem>) -> AppResult<DockConfig> {
        self.update_and_save(|cfg| {
            cfg.dismissed.clear();
            cfg.items = items;
            Ok(cfg.clone())
        })
    }

    /// Checks whether a candidate item matches an entry in `dismissed`.
    #[must_use]
    pub fn is_dismissed(&self, candidate: &DockItem) -> bool {
        let cfg = self.state.read();
        is_key_dismissed(
            &cfg.dismissed,
            &candidate.dismissal_key(),
            &build_dismissal_key(
                candidate.source,
                candidate.steam_app_id.as_deref(),
                &candidate.path,
            ),
        )
    }
}

/// Checks whether either the resolved target key or raw path key is dismissed.
#[must_use]
pub fn is_key_dismissed(dismissed: &[String], primary_key: &str, secondary_key: &str) -> bool {
    dismissed
        .iter()
        .any(|entry| entry == primary_key || (!secondary_key.is_empty() && entry == secondary_key))
}

/// Loads `config.json`, renaming unreadable/invalid JSON to `config.json.corrupt.<ts>`.
pub fn load_config_with_recovery(config_path: &Path) -> AppResult<(DockConfig, LoadReport)> {
    if !config_path.exists() {
        return Ok((
            DockConfig::default(),
            LoadReport {
                created_default: true,
                recovered_corrupt_path: None,
            },
        ));
    }

    match read_and_parse_config(config_path) {
        Ok(mut cfg) => {
            sanitize_config(&mut cfg);
            Ok((
                cfg,
                LoadReport {
                    created_default: false,
                    recovered_corrupt_path: None,
                },
            ))
        }
        Err(err) => {
            tracing::error!(
                path = %config_path.display(),
                error = %err,
                "Failed to parse config.json; backing up corrupt file and restoring defaults"
            );
            let backup_path = backup_corrupt_config(config_path)?;
            Ok((
                DockConfig::default(),
                LoadReport {
                    created_default: false,
                    recovered_corrupt_path: Some(backup_path),
                },
            ))
        }
    }
}

fn read_and_parse_config(config_path: &Path) -> AppResult<DockConfig> {
    let raw = std::fs::read_to_string(config_path)?;
    if raw.trim().is_empty() {
        return Err(AppError::Config("config.json is empty".to_owned()));
    }
    let parsed: DockConfig = serde_json::from_str(&raw)?;
    Ok(parsed)
}

fn backup_corrupt_config(config_path: &Path) -> AppResult<PathBuf> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let file_name = config_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("config.json");

    let parent = config_path
        .parent()
        .ok_or_else(|| AppError::Config("Config path has no parent directory".to_owned()))?;

    let mut candidate = parent.join(format!("{file_name}.corrupt.{timestamp}"));
    let mut suffix = 1_u32;
    while candidate.exists() {
        candidate = parent.join(format!("{file_name}.corrupt.{timestamp}.{suffix}"));
        suffix = suffix.saturating_add(1);
    }

    std::fs::rename(config_path, &candidate)?;
    Ok(candidate)
}

fn sanitize_config(cfg: &mut DockConfig) {
    if cfg.schema_version == 0 {
        cfg.schema_version = CURRENT_SCHEMA_VERSION;
    }
    if cfg.global_hotkey.trim().is_empty() {
        cfg.global_hotkey = DEFAULT_GLOBAL_HOTKEY.to_owned();
    }
    if cfg.selected_category.trim().is_empty() {
        cfg.selected_category = "All".to_owned();
    }
    if !cfg.top_position.is_finite() || cfg.top_position < 0.0 {
        cfg.top_position = 120.0;
    }
    for key in &mut cfg.dismissed {
        *key = normalize_path_key(key);
    }
    cfg.dismissed.retain(|k| !k.is_empty());
}

/// Atomically writes `config` via a same-directory temporary file synced to disk, then replaces target.
pub fn save_config_atomic(config_path: &Path, config: &DockConfig) -> AppResult<()> {
    let parent = config_path
        .parent()
        .ok_or_else(|| AppError::Config("Config path has no parent directory".to_owned()))?;
    std::fs::create_dir_all(parent)?;

    let json = serde_json::to_string_pretty(config)?;
    let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let file_name = config_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("config.json");
    let tmp_path = parent.join(format!(".{file_name}.tmp.{pid}.{counter}"));

    let write_res: AppResult<()> = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)?;
        file.write_all(json.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        Ok(())
    })();

    if let Err(err) = write_res {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(err);
    }

    if let Err(err) = std::fs::rename(&tmp_path, config_path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(AppError::Io(err));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Category, DockEdge, ItemSource};

    fn make_temp_dir(label: &str) -> AppResult<PathBuf> {
        let seq = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "harbor_test_{label}_{}_{}",
            std::process::id(),
            seq
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    #[test]
    fn resolves_portable_vs_installed_explicitly_via_flag() -> AppResult<()> {
        let root = make_temp_dir("paths")?;
        let exe_dir = root.join("bin");
        let local_app_data = root.join("localappdata");
        std::fs::create_dir_all(&exe_dir)?;
        std::fs::create_dir_all(&local_app_data)?;

        // Without portable.flag -> Installed mode in %LOCALAPPDATA%\Harbor even if exe_dir is writable.
        let installed = AppPaths::resolve_from(&exe_dir, Some(&local_app_data))?;
        assert!(!installed.is_portable);
        assert_eq!(installed.data_dir, local_app_data.join("Harbor"));
        assert_eq!(installed.config_path, local_app_data.join("Harbor/config.json"));
        assert_eq!(installed.logs_dir, local_app_data.join("Harbor/logs"));
        assert_eq!(
            installed.icon_cache_dir,
            local_app_data.join("Harbor/icon_cache")
        );

        // With portable.flag -> Portable mode in <exe_dir>\data.
        std::fs::write(exe_dir.join("portable.flag"), b"")?;
        let portable = AppPaths::resolve_from(&exe_dir, Some(&local_app_data))?;
        assert!(portable.is_portable);
        assert_eq!(portable.data_dir, exe_dir.join("data"));
        assert_eq!(portable.config_path, exe_dir.join("data/config.json"));

        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    #[test]
    fn atomic_save_and_reload_roundtrips_cleanly() -> AppResult<()> {
        let root = make_temp_dir("roundtrip")?;
        let exe_dir = root.join("app");
        std::fs::create_dir_all(&exe_dir)?;
        std::fs::write(exe_dir.join("portable.flag"), b"")?;

        let paths = AppPaths::resolve_from(&exe_dir, None)?;
        let (manager, report) = ConfigManager::load_or_init(paths.clone())?;
        assert!(report.created_default);
        assert!(report.recovered_corrupt_path.is_none());
        assert!(paths.config_path.exists());

        let item = DockItem {
            id: "item-1".to_owned(),
            name: "Counter-Strike 2".to_owned(),
            path: "steam://rungameid/730".to_owned(),
            arguments: String::new(),
            description: "Steam Game".to_owned(),
            category: Category::Games,
            custom_icon_path: None,
            icon_source_path: Some("C:\\Steam\\cs2.exe".to_owned()),
            run_as_admin: false,
            pinned: true,
            source: ItemSource::Steam,
            steam_app_id: Some("730".to_owned()),
            launch_count: 3,
        };

        manager.update_and_save(|cfg| {
            cfg.edge = DockEdge::Right;
            cfg.top_position = 240.0;
            cfg.items.push(item.clone());
            Ok(())
        })?;

        let (reloaded, report2) = ConfigManager::load_or_init(paths.clone())?;
        assert!(!report2.created_default);
        assert!(report2.recovered_corrupt_path.is_none());

        let snap = reloaded.snapshot();
        assert_eq!(snap.edge, DockEdge::Right);
        assert_eq!(snap.top_position, 240.0);
        assert_eq!(snap.items, vec![item]);

        // Verify no leftover temp files remain.
        let leftover_tmps: Vec<_> = std::fs::read_dir(&paths.data_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(leftover_tmps.is_empty());

        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    #[test]
    fn corrupt_config_is_backed_up_and_replaced_with_defaults() -> AppResult<()> {
        let root = make_temp_dir("corrupt")?;
        let exe_dir = root.join("app");
        std::fs::create_dir_all(&exe_dir)?;
        std::fs::write(exe_dir.join("portable.flag"), b"")?;

        let paths = AppPaths::resolve_from(&exe_dir, None)?;
        paths.ensure_dirs()?;

        let bad_payload = "{ invalid json ::: truncated";
        std::fs::write(&paths.config_path, bad_payload)?;

        let (manager, report) = ConfigManager::load_or_init(paths.clone())?;
        assert!(!report.created_default);
        let Some(backup) = report.recovered_corrupt_path else {
            panic!("expected corrupt backup path");
        };
        assert!(backup.exists());
        let Some(backup_name) = backup.file_name() else {
            panic!("expected backup filename");
        };
        assert!(backup_name
            .to_string_lossy()
            .starts_with("config.json.corrupt."));
        assert_eq!(std::fs::read_to_string(&backup)?, bad_payload);

        let snap = manager.snapshot();
        assert_eq!(snap.schema_version, CURRENT_SCHEMA_VERSION);
        assert!(snap.items.is_empty());
        assert!(snap.dismissed.is_empty());

        // New config.json must be valid JSON on disk now.
        let fresh_raw = std::fs::read_to_string(&paths.config_path)?;
        let reparsed: DockConfig = serde_json::from_str(&fresh_raw)?;
        assert_eq!(reparsed, snap);

        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    #[test]
    fn loads_legacy_schema_v1_config_without_new_default_fields() -> AppResult<()> {
        let root = make_temp_dir("legacy_v1")?;
        let exe_dir = root.join("app");
        std::fs::create_dir_all(&exe_dir)?;
        std::fs::write(exe_dir.join("portable.flag"), b"")?;

        let paths = AppPaths::resolve_from(&exe_dir, None)?;
        paths.ensure_dirs()?;

        // Legacy v1 payload lacking `dismissed` on DockConfig and `icon_source_path` on DockItem.
        let legacy_v1_json = r#"{
  "schema_version": 1,
  "edge": "Left",
  "monitor_name": null,
  "top_position": 160.0,
  "auto_collapse_on_launch": true,
  "auto_collapse_on_blur": false,
  "always_on_top": true,
  "start_with_windows": true,
  "global_hotkey": "Alt+Shift+Space",
  "selected_category": "Games",
  "items": [
    {
      "id": "legacy-1",
      "name": "Portal 2",
      "path": "C:\\Games\\Portal2\\portal2.exe",
      "arguments": "-novid",
      "description": "Valve",
      "category": "Games",
      "custom_icon_path": null,
      "run_as_admin": false,
      "pinned": true,
      "source": "Steam",
      "steam_app_id": "620",
      "launch_count": 12
    }
  ]
}"#;
        std::fs::write(&paths.config_path, legacy_v1_json)?;

        let (manager, report) = ConfigManager::load_or_init(paths)?;
        assert!(!report.created_default);
        assert!(report.recovered_corrupt_path.is_none());

        let snap = manager.snapshot();
        assert_eq!(snap.schema_version, 1);
        assert!(!snap.auto_collapse_on_blur);
        assert!(snap.start_with_windows);
        assert_eq!(snap.selected_category, "Games");
        assert!(snap.dismissed.is_empty());
        assert_eq!(snap.items.len(), 1);
        assert_eq!(snap.items[0].icon_source_path, None);
        assert_eq!(snap.items[0].name, "Portal 2");

        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    #[test]
    fn dismissal_tracks_remove_manual_restore_and_reset() -> AppResult<()> {
        let root = make_temp_dir("dismiss")?;
        let exe_dir = root.join("app");
        std::fs::create_dir_all(&exe_dir)?;
        std::fs::write(exe_dir.join("portable.flag"), b"")?;

        let paths = AppPaths::resolve_from(&exe_dir, None)?;
        let (manager, _) = ConfigManager::load_or_init(paths)?;

        let item = DockItem {
            id: "id-1".to_owned(),
            name: "VLC".to_owned(),
            path: "C:\\Program Files\\VideoLAN\\VLC\\vlc.exe".to_owned(),
            arguments: String::new(),
            description: String::new(),
            category: Category::Apps,
            custom_icon_path: None,
            icon_source_path: None,
            run_as_admin: false,
            pinned: false,
            source: ItemSource::StartMenu,
            steam_app_id: None,
            launch_count: 0,
        };

        manager.add_manual_item(item.clone())?;
        assert!(!manager.is_dismissed(&item));

        let removed = manager.remove_item_and_dismiss("id-1")?;
        assert!(removed.is_some());
        assert!(manager.is_dismissed(&item));

        // Manually re-adding clears the dismissal key.
        manager.add_manual_item(item.clone())?;
        assert!(!manager.is_dismissed(&item));

        // Reset clears all dismissals.
        manager.remove_item_and_dismiss("id-1")?;
        assert!(manager.is_dismissed(&item));
        manager.reset_to_discovered(Vec::new())?;
        assert!(!manager.is_dismissed(&item));

        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }
}
