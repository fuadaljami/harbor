use serde::{Deserialize, Serialize};

/// Current persisted configuration schema version.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Default global toggle shortcut string.
pub const DEFAULT_GLOBAL_HOTKEY: &str = "Alt+Shift+Space";

/// Default vertical handle offset from the active monitor work-area top (logical px).
pub const DEFAULT_TOP_POSITION: f64 = 120.0;

/// Screen edge where Harbor docks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum DockEdge {
    #[default]
    Left,
    Right,
}

/// Launcher item category shown in the rail and section headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Category {
    Games,
    #[default]
    Apps,
    Tools,
}

/// Origin of a shortcut item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum ItemSource {
    #[default]
    Manual,
    Steam,
    StartMenu,
    System,
}

/// Single launcher entry persisted in `config.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockItem {
    pub id: String,
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
    pub icon_source_path: Option<String>,
    #[serde(default)]
    pub run_as_admin: bool,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub source: ItemSource,
    #[serde(default)]
    pub steam_app_id: Option<String>,
    #[serde(default)]
    pub launch_count: u64,
}

impl DockItem {
    /// Resolves the preferred path for icon extraction:
    /// `custom_icon_path` -> `icon_source_path` -> `path`.
    #[must_use]
    pub fn icon_lookup_path(&self) -> &str {
        if let Some(custom) = self.custom_icon_path.as_deref() {
            let trimmed = custom.trim();
            if !trimmed.is_empty() {
                return trimmed;
            }
        }
        if let Some(icon_src) = self.icon_source_path.as_deref() {
            let trimmed = icon_src.trim();
            if !trimmed.is_empty() {
                return trimmed;
            }
        }
        self.path.trim()
    }

    /// Resolves the path opened by "Open File Location" (`icon_source_path` for Steam items, otherwise `path`).
    #[must_use]
    pub fn file_location_target(&self) -> Option<&str> {
        if self.source == ItemSource::Steam {
            if let Some(icon_src) = self.icon_source_path.as_deref() {
                let trimmed = icon_src.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed);
                }
            }
            return None;
        }
        let trimmed = self.path.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    }

    /// Computes the stable dismissal key for this item.
    #[must_use]
    pub fn dismissal_key(&self) -> String {
        build_dismissal_key(
            self.source,
            self.steam_app_id.as_deref(),
            self.icon_source_path
                .as_deref()
                .filter(|_| self.source != ItemSource::Steam)
                .unwrap_or(&self.path),
        )
    }
}

/// Builds a dismissal key: `"steam:<app_id>"` for Steam items or lowercased normalized target/path.
#[must_use]
pub fn build_dismissal_key(
    source: ItemSource,
    steam_app_id: Option<&str>,
    target_or_path: &str,
) -> String {
    if source == ItemSource::Steam {
        if let Some(app_id) = steam_app_id {
            let clean_id = app_id.trim();
            if !clean_id.is_empty() {
                return format!("steam:{}", clean_id.to_ascii_lowercase());
            }
        }
    }
    normalize_path_key(target_or_path)
}

/// Normalizes a Windows path or URI for case-insensitive matching and dismissal keys.
#[must_use]
pub fn normalize_path_key(raw: &str) -> String {
    let trimmed = raw.trim().trim_matches('"').trim();
    if trimmed.contains("://") {
        trimmed.to_ascii_lowercase()
    } else {
        trimmed.replace('/', "\\").to_ascii_lowercase()
    }
}

/// Full persisted Harbor configuration (`config.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DockConfig {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub edge: DockEdge,
    #[serde(default)]
    pub monitor_name: Option<String>,
    #[serde(default = "default_top_position")]
    pub top_position: f64,
    #[serde(default = "default_true")]
    pub auto_collapse_on_launch: bool,
    #[serde(default = "default_true")]
    pub auto_collapse_on_blur: bool,
    #[serde(default = "default_true")]
    pub always_on_top: bool,
    #[serde(default)]
    pub start_with_windows: bool,
    #[serde(default = "default_global_hotkey")]
    pub global_hotkey: String,
    #[serde(default = "default_selected_category")]
    pub selected_category: String,
    #[serde(default)]
    pub items: Vec<DockItem>,
    #[serde(default)]
    pub dismissed: Vec<String>,
}

impl Default for DockConfig {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            edge: DockEdge::Left,
            monitor_name: None,
            top_position: DEFAULT_TOP_POSITION,
            auto_collapse_on_launch: true,
            auto_collapse_on_blur: true,
            always_on_top: true,
            start_with_windows: false,
            global_hotkey: DEFAULT_GLOBAL_HOTKEY.to_owned(),
            selected_category: "All".to_owned(),
            items: Vec::new(),
            dismissed: Vec::new(),
        }
    }
}

fn default_schema_version() -> u32 {
    CURRENT_SCHEMA_VERSION
}

fn default_top_position() -> f64 {
    DEFAULT_TOP_POSITION
}

fn default_true() -> bool {
    true
}

fn default_global_hotkey() -> String {
    DEFAULT_GLOBAL_HOTKEY.to_owned()
}

fn default_selected_category() -> String {
    "All".to_owned()
}

/// Summary returned to the UI after a non-destructive system rescan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RescanResult {
    pub added: usize,
    pub missing_ids: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_lookup_respects_priority_chain() {
        let mut item = DockItem {
            id: "1".to_owned(),
            name: "Game".to_owned(),
            path: "steam://rungameid/730".to_owned(),
            arguments: String::new(),
            description: String::new(),
            category: Category::Games,
            custom_icon_path: Some("C:\\Custom\\icon.png".to_owned()),
            icon_source_path: Some("C:\\Games\\cs2.exe".to_owned()),
            run_as_admin: false,
            pinned: false,
            source: ItemSource::Steam,
            steam_app_id: Some("730".to_owned()),
            launch_count: 0,
        };

        assert_eq!(item.icon_lookup_path(), "C:\\Custom\\icon.png");

        item.custom_icon_path = Some("   ".to_owned());
        assert_eq!(item.icon_lookup_path(), "C:\\Games\\cs2.exe");

        item.icon_source_path = None;
        assert_eq!(item.icon_lookup_path(), "steam://rungameid/730");
    }

    #[test]
    fn dismissal_key_handles_steam_and_standard_paths() {
        let steam = DockItem {
            id: "1".to_owned(),
            name: "Hades".to_owned(),
            path: "steam://rungameid/1145360".to_owned(),
            arguments: String::new(),
            description: String::new(),
            category: Category::Games,
            custom_icon_path: None,
            icon_source_path: Some("C:\\Games\\Hades\\Hades.exe".to_owned()),
            run_as_admin: false,
            pinned: false,
            source: ItemSource::Steam,
            steam_app_id: Some("1145360".to_owned()),
            launch_count: 0,
        };
        assert_eq!(steam.dismissal_key(), "steam:1145360");

        let app = DockItem {
            id: "2".to_owned(),
            name: "Notepad".to_owned(),
            path: "\"C:/Windows/System32/notepad.exe\"".to_owned(),
            arguments: String::new(),
            description: String::new(),
            category: Category::Tools,
            custom_icon_path: None,
            icon_source_path: None,
            run_as_admin: false,
            pinned: false,
            source: ItemSource::System,
            steam_app_id: None,
            launch_count: 0,
        };
        assert_eq!(app.dismissal_key(), "c:\\windows\\system32\\notepad.exe");
    }
}
