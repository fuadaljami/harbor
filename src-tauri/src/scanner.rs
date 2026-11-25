use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;

use crate::config::is_key_dismissed;
use crate::error::AppResult;
use crate::models::{
    normalize_path_key, Category, DockConfig, DockItem, ItemSource, RescanResult,
};
use crate::win_shell::{
    hex_encode, is_drive_mounted, is_non_file_uri, read_steam_install_path,
    resolve_start_menu_dirs, ResolvedLnk, WinShell,
};

const MAX_SCAN_DEPTH: usize = 8;
const MAX_STEAM_EXE_DEPTH: usize = 3;
const LNK_CONCURRENCY: usize = 8;

static MANUAL_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Abstraction for resolving `.lnk` shortcut files so unit tests stay pure and COM-free.
pub trait ShortcutResolver: Send + Sync {
    fn resolve_lnk(&self, lnk_path: &Path) -> Option<ResolvedLnk>;
}

/// Abstraction for drive availability and file existence checks for deterministic unit tests.
pub trait ScanHost: Send + Sync {
    fn is_drive_available(&self, path: &Path) -> bool;
    fn path_exists(&self, path: &Path) -> bool;
    fn canonicalize_key(&self, raw_path: &str) -> String {
        if is_non_file_uri(raw_path) {
            return normalize_path_key(raw_path);
        }
        let path = Path::new(raw_path.trim().trim_matches('"'));
        let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        normalize_path_key(&resolved.to_string_lossy())
    }
}

/// Default Windows host implementation backed by `GetDriveTypeW` and `std::fs`.
pub struct WindowsScanHost;

impl ScanHost for WindowsScanHost {
    fn is_drive_available(&self, path: &Path) -> bool {
        is_drive_mounted(path)
    }

    fn path_exists(&self, path: &Path) -> bool {
        path.exists()
    }
}

struct MapShortcutResolver<'a>(&'a HashMap<PathBuf, ResolvedLnk>);

impl ShortcutResolver for MapShortcutResolver<'_> {
    fn resolve_lnk(&self, lnk_path: &Path) -> Option<ResolvedLnk> {
        self.0.get(lnk_path).cloned()
    }
}

/// Inputs describing where discovery should look.
#[derive(Debug, Clone)]
pub struct ScanEnvironment {
    pub steam_root: Option<PathBuf>,
    pub start_menu_dirs: Vec<PathBuf>,
    pub system_utilities: Vec<DockItem>,
}

impl ScanEnvironment {
    /// Builds the live Windows discovery environment from registry, known folders, and system paths.
    #[must_use]
    pub fn windows_live() -> Self {
        Self {
            steam_root: read_steam_install_path(),
            start_menu_dirs: resolve_start_menu_dirs(),
            system_utilities: collect_core_windows_utilities(),
        }
    }
}

/// Output of the discovery phase prior to non-destructive merge.
#[derive(Debug, Clone)]
pub struct ScanOutput {
    pub discovered: Vec<DockItem>,
    pub steam_root_available: bool,
    pub unavailable_steam_roots: Vec<PathBuf>,
}

/// Discovers live Windows Steam, System utility, and Start Menu shortcuts off-lock.
pub async fn discover_live(shell: &WinShell) -> ScanOutput {
    let env = ScanEnvironment::windows_live();
    let host = WindowsScanHost;

    let mut lnk_paths = Vec::new();
    for dir in &env.start_menu_dirs {
        if host.is_drive_available(dir) && dir.is_dir() {
            collect_lnk_files(dir, 0, &mut lnk_paths);
        }
    }

    let resolved_map = resolve_lnk_batch(shell, lnk_paths).await;
    let resolver = MapShortcutResolver(&resolved_map);
    discover_items(&env, &resolver, &host)
}

/// Runs a full live Windows scan using the dedicated COM STA `WinShell` worker,
/// merges newly discovered items non-destructively into `config`, and returns [`RescanResult`].
pub async fn scan_and_merge_live(
    shell: &WinShell,
    config: &mut DockConfig,
) -> AppResult<RescanResult> {
    let scan = discover_live(shell).await;
    let host = WindowsScanHost;
    Ok(merge_discovered_items(config, &scan, &host))
}

async fn resolve_lnk_batch(
    shell: &WinShell,
    lnk_paths: Vec<PathBuf>,
) -> HashMap<PathBuf, ResolvedLnk> {
    let sem = Arc::new(Semaphore::new(LNK_CONCURRENCY));
    let mut join_set = tokio::task::JoinSet::new();

    for path in lnk_paths {
        let shell_cloned = shell.clone();
        let sem_cloned = Arc::clone(&sem);
        join_set.spawn(async move {
            let Ok(_permit) = sem_cloned.acquire_owned().await else {
                return None;
            };
            match shell_cloned.resolve_lnk(path.clone()).await {
                Ok(Some(resolved)) => Some((path, resolved)),
                _ => None,
            }
        });
    }

    let mut map = HashMap::new();
    while let Some(joined) = join_set.join_next().await {
        if let Ok(Some((path, resolved))) = joined {
            map.insert(path, resolved);
        }
    }
    map
}

/// Pure discovery pipeline executing Steam, core System utilities, and Start Menu scanning
/// with canonical deduplication.
pub fn discover_items(
    env: &ScanEnvironment,
    resolver: &dyn ShortcutResolver,
    host: &dyn ScanHost,
) -> ScanOutput {
    let mut discovered = Vec::new();
    let mut seen_steam_ids = HashSet::new();
    let mut seen_target_keys = HashSet::new();

    // 1. Steam library & appmanifest discovery.
    let (steam_root_available, unavailable_steam_roots, steam_items) =
        discover_steam_games(env.steam_root.as_deref(), host);
    for item in steam_items {
        if let Some(app_id) = item.steam_app_id.as_deref() {
            let key = app_id.to_ascii_lowercase();
            if seen_steam_ids.insert(key) {
                discovered.push(item);
            }
        }
    }

    // 2. Core Windows system utilities (`Tools`).
    for util in &env.system_utilities {
        let target_key = host.canonicalize_key(&util.path);
        if !target_key.is_empty() && seen_target_keys.insert(target_key) {
            discovered.push(util.clone());
        }
    }

    // 3. Start Menu `.lnk` shortcuts (User + Common folders).
    let mut lnk_files = Vec::new();
    for dir in &env.start_menu_dirs {
        if host.is_drive_available(dir) && dir.is_dir() {
            collect_lnk_files(dir, 0, &mut lnk_files);
        }
    }
    // Sort deterministically so User/Common collisions choose consistently.
    lnk_files.sort();

    for lnk_path in lnk_files {
        let Some(item) = build_start_menu_item(&lnk_path, resolver, host) else {
            continue;
        };
        let canonical_key = host.canonicalize_key(&item.path);
        if canonical_key.is_empty() || !seen_target_keys.insert(canonical_key) {
            continue;
        }
        discovered.push(item);
    }

    ScanOutput {
        discovered,
        steam_root_available,
        unavailable_steam_roots,
    }
}

/// Merges `scan.discovered` into `config` without overwriting any user-edited fields,
/// respecting `config.dismissed`, and identifying missing non-manual entries.
pub fn merge_discovered_items(
    config: &mut DockConfig,
    scan: &ScanOutput,
    host: &dyn ScanHost,
) -> RescanResult {
    let mut existing_steam_ids = HashSet::new();
    let mut existing_target_keys = HashSet::new();
    let mut discovered_steam_by_id: HashMap<String, &DockItem> = HashMap::new();

    for item in &scan.discovered {
        if item.source == ItemSource::Steam {
            if let Some(app_id) = item.steam_app_id.as_deref() {
                discovered_steam_by_id.insert(app_id.to_ascii_lowercase(), item);
            }
        }
    }

    for existing in &mut config.items {
        if existing.source == ItemSource::Steam {
            if let Some(app_id) = existing.steam_app_id.as_deref() {
                let id_key = app_id.to_ascii_lowercase();
                existing_steam_ids.insert(id_key.clone());
                // Backfill icon_source_path only when missing (e.g. legacy v1 config).
                if existing.icon_source_path.is_none() {
                    if let Some(fresh) = discovered_steam_by_id.get(&id_key) {
                        existing.icon_source_path = fresh.icon_source_path.clone();
                    }
                }
            }
        } else {
            existing_target_keys.insert(normalize_path_key(&existing.path));
            existing_target_keys.insert(host.canonicalize_key(&existing.path));
        }
    }

    let mut added = 0_usize;
    for candidate in &scan.discovered {
        if candidate.source == ItemSource::Steam {
            let Some(app_id) = candidate.steam_app_id.as_deref() else {
                continue;
            };
            let id_key = app_id.to_ascii_lowercase();
            let dismissal = candidate.dismissal_key();
            if is_key_dismissed(&config.dismissed, &dismissal, "") {
                continue;
            }
            if existing_steam_ids.insert(id_key) {
                config.items.push(candidate.clone());
                added += 1;
            }
        } else {
            let norm_key = normalize_path_key(&candidate.path);
            let canon_key = host.canonicalize_key(&candidate.path);
            if is_key_dismissed(&config.dismissed, &canon_key, &norm_key) {
                continue;
            }
            if existing_target_keys.contains(&norm_key) || existing_target_keys.contains(&canon_key)
            {
                continue;
            }
            existing_target_keys.insert(norm_key);
            existing_target_keys.insert(canon_key);
            config.items.push(candidate.clone());
            added += 1;
        }
    }

    let missing_ids = compute_missing_ids(config, scan, &discovered_steam_by_id, host);
    RescanResult { added, missing_ids }
}

fn compute_missing_ids(
    config: &DockConfig,
    scan: &ScanOutput,
    discovered_steam_by_id: &HashMap<String, &DockItem>,
    host: &dyn ScanHost,
) -> Vec<String> {
    let mut missing = Vec::new();

    for item in &config.items {
        match item.source {
            ItemSource::Manual => {}
            ItemSource::Steam => {
                let Some(app_id) = item.steam_app_id.as_deref() else {
                    continue;
                };
                let id_key = app_id.to_ascii_lowercase();
                if discovered_steam_by_id.contains_key(&id_key) {
                    continue;
                }
                if !scan.steam_root_available {
                    continue;
                }
                let drive_unknown = match item.icon_source_path.as_deref() {
                    Some(bin) if !bin.trim().is_empty() => {
                        let bin_path = Path::new(bin);
                        !host.is_drive_available(bin_path)
                            || is_under_any_root(bin_path, &scan.unavailable_steam_roots)
                    }
                    _ => !scan.unavailable_steam_roots.is_empty(),
                };
                if !drive_unknown {
                    missing.push(item.id.clone());
                }
            }
            ItemSource::StartMenu | ItemSource::System => {
                if is_non_file_uri(&item.path) {
                    continue;
                }
                let p = Path::new(&item.path);
                if !host.is_drive_available(p) {
                    continue;
                }
                if !host.path_exists(p) {
                    missing.push(item.id.clone());
                }
            }
        }
    }

    missing
}

fn is_under_any_root(path: &Path, roots: &[PathBuf]) -> bool {
    let path_norm = normalize_path_key(&path.to_string_lossy());
    roots.iter().any(|root| {
        let root_norm = normalize_path_key(&root.to_string_lossy());
        !root_norm.is_empty() && path_norm.starts_with(&root_norm)
    })
}

/// Derives a deterministic ID for scanned (`Steam`, `StartMenu`, `System`) items via SHA-256.
#[must_use]
pub fn derive_scanned_id(dismissal_key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(dismissal_key.as_bytes());
    let hex = hex_encode(&hasher.finalize());
    format!("scn_{}", &hex[..24])
}

/// Generates a unique ID for manually added items so multiple entries for the same `.exe`
/// with different arguments never collide.
#[must_use]
pub fn generate_manual_id(path: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = MANUAL_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut hasher = Sha256::new();
    hasher.update(path.as_bytes());
    hasher.update(b"|");
    hasher.update(nanos.to_le_bytes());
    hasher.update(b"|");
    hasher.update(seq.to_le_bytes());
    let hex = hex_encode(&hasher.finalize());
    format!("man_{}", &hex[..24])
}

fn collect_core_windows_utilities() -> Vec<DockItem> {
    let mut specs: Vec<(&str, PathBuf, &str)> = Vec::new();
    let system_root = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\Windows"));
    let system32 = system_root.join("System32");

    specs.push((
        "Task Manager",
        system32.join("taskmgr.exe"),
        "Windows Task Manager",
    ));

    if let Some(local_appdata) = std::env::var_os("LOCALAPPDATA") {
        let wt = PathBuf::from(local_appdata).join("Microsoft\\WindowsApps\\wt.exe");
        if wt.is_file() {
            specs.push(("Terminal", wt, "Windows Terminal"));
        }
    }

    specs.push((
        "Command Prompt",
        system32.join("cmd.exe"),
        "Windows Command Prompt",
    ));

    let pwsh = std::env::var_os("ProgramFiles")
        .map(|pf| PathBuf::from(pf).join("PowerShell\\7\\pwsh.exe"))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| system32.join("WindowsPowerShell\\v1.0\\powershell.exe"));
    specs.push(("PowerShell", pwsh, "Windows PowerShell"));

    specs.push((
        "Registry Editor",
        system_root.join("regedit.exe"),
        "Windows Registry Editor",
    ));
    specs.push((
        "Control Panel",
        system32.join("control.exe"),
        "Windows Control Panel",
    ));
    specs.push((
        "Settings",
        PathBuf::from("ms-settings:"),
        "Windows Settings",
    ));
    specs.push((
        "Calculator",
        system32.join("calc.exe"),
        "Windows Calculator",
    ));

    let mut out = Vec::new();
    for (name, path_buf, description) in specs {
        let path_str = path_buf.to_string_lossy().into_owned();
        if !is_non_file_uri(&path_str) && !path_buf.exists() {
            continue;
        }
        let dismissal_key = normalize_path_key(&path_str);
        out.push(DockItem {
            id: derive_scanned_id(&dismissal_key),
            name: name.to_owned(),
            path: path_str,
            arguments: String::new(),
            description: description.to_owned(),
            category: Category::Tools,
            custom_icon_path: None,
            icon_source_path: None,
            run_as_admin: false,
            pinned: false,
            source: ItemSource::System,
            steam_app_id: None,
            launch_count: 0,
        });
    }
    out
}

fn collect_lnk_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_SCAN_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_symlink() {
            continue;
        }
        let path = entry.path();
        if ft.is_dir() {
            collect_lnk_files(&path, depth + 1, out);
        } else if ft.is_file()
            && path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("lnk"))
        {
            out.push(path);
        }
    }
}

fn build_start_menu_item(
    lnk_path: &Path,
    resolver: &dyn ShortcutResolver,
    host: &dyn ScanHost,
) -> Option<DockItem> {
    let shortcut_name = lnk_path
        .file_stem()
        .and_then(|s| s.to_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?
        .to_owned();

    let resolved = resolver.resolve_lnk(lnk_path)?;
    let target_trimmed = resolved.target_path.trim().trim_matches('"').to_owned();
    if target_trimmed.is_empty() {
        return None;
    }
    if is_non_file_uri(&target_trimmed) {
        return None;
    }

    let target_path = Path::new(&target_trimmed);
    if !host.is_drive_available(target_path) || !host.path_exists(target_path) {
        return None;
    }

    if is_filtered_shortcut(&shortcut_name, &target_trimmed) {
        return None;
    }

    let category = categorize_shortcut(&shortcut_name, &target_trimmed);
    let canonical_key = host.canonicalize_key(&target_trimmed);
    let description = if resolved.description.trim().is_empty() {
        "Start Menu Shortcut".to_owned()
    } else {
        resolved.description.trim().to_owned()
    };

    Some(DockItem {
        id: derive_scanned_id(&canonical_key),
        name: shortcut_name,
        path: target_trimmed,
        arguments: resolved.arguments,
        description,
        category,
        custom_icon_path: None,
        icon_source_path: None,
        run_as_admin: false,
        pinned: false,
        source: ItemSource::StartMenu,
        steam_app_id: None,
        launch_count: 0,
    })
}

fn is_filtered_shortcut(name: &str, target_path: &str) -> bool {
    if let Some(ext) = Path::new(target_path).extension().and_then(|e| e.to_str()) {
        let lower_ext = ext.to_ascii_lowercase();
        if matches!(
            lower_ext.as_str(),
            "html" | "htm" | "url" | "pdf" | "txt" | "chm" | "hlp" | "md" | "rtf" | "doc" | "docx"
        ) {
            return true;
        }
    }

    let name_tokens = tokenize_words(name);
    let target_stem = Path::new(target_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let target_tokens = tokenize_words(target_stem);

    const BLOCKED_PHRASES: &[&[&str]] = &[
        &["uninstall"],
        &["unins000"],
        &["readme"],
        &["release", "notes"],
        &["changelog"],
        &["license"],
        &["eula"],
        &["help"],
        &["manual"],
        &["documentation"],
        &["updater"],
        &["website"],
        &["webpage"],
        &["bug", "report"],
        &["crash", "reporter"],
        &["feedback"],
        &["donate"],
    ];

    contains_any_phrase(&name_tokens, BLOCKED_PHRASES)
        || contains_any_phrase(&target_tokens, BLOCKED_PHRASES)
}

/// Whole-token, case-insensitive auto-categorization per `PROJECT_RULES.md`.
#[must_use]
pub fn categorize_shortcut(name: &str, target_path: &str) -> Category {
    const GAME_PHRASES: &[&[&str]] = &[
        &["steam"],
        &["epic", "games"],
        &["riot"],
        &["battle", "net"],
        &["gog"],
        &["ea", "app"],
        &["ubisoft"],
        &["rockstar"],
        &["xbox"],
        &["game", "bar"],
        &["afterburner"],
        &["rivatuner"],
        &["mod", "manager"],
    ];

    const TOOL_PHRASES: &[&[&str]] = &[
        &["terminal"],
        &["powershell"],
        &["cmd"],
        &["task", "manager"],
        &["regedit"],
        &["control", "panel"],
        &["device", "manager"],
        &["resource", "monitor"],
        &["disk", "cleanup"],
        &["services"],
        &["system", "information"],
        &["administrative"],
        &["diagnostic"],
        &["monitor"],
        &["cleanup"],
        &["settings"],
        &["calculator"],
        &["snipping", "tool"],
    ];

    let name_tokens = tokenize_words(name);
    let target_stem = Path::new(target_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let target_tokens = tokenize_words(target_stem);

    if contains_any_phrase(&name_tokens, GAME_PHRASES)
        || contains_any_phrase(&target_tokens, GAME_PHRASES)
    {
        return Category::Games;
    }

    if contains_any_phrase(&name_tokens, TOOL_PHRASES)
        || contains_any_phrase(&target_tokens, TOOL_PHRASES)
    {
        return Category::Tools;
    }

    Category::Apps
}

fn tokenize_words(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            current.push(ch.to_ascii_lowercase());
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn contains_any_phrase(tokens: &[String], phrases: &[&[&str]]) -> bool {
    phrases.iter().any(|phrase| {
        !phrase.is_empty()
            && tokens
                .windows(phrase.len())
                .any(|window| window.iter().zip(phrase.iter()).all(|(a, b)| a == b))
    })
}

fn discover_steam_games(
    steam_root: Option<&Path>,
    host: &dyn ScanHost,
) -> (bool, Vec<PathBuf>, Vec<DockItem>) {
    let Some(root) = steam_root else {
        return (false, Vec::new(), Vec::new());
    };
    if !host.is_drive_available(root) || !host.path_exists(root) {
        return (false, vec![root.to_path_buf()], Vec::new());
    }

    let vdf_path = root.join("steamapps").join("libraryfolders.vdf");
    let library_roots = match std::fs::read_to_string(&vdf_path) {
        Ok(content) => extract_steam_library_roots(root, &content),
        Err(_) => vec![root.to_path_buf()],
    };

    let mut unavailable_roots = Vec::new();
    let mut games = Vec::new();

    for lib_root in library_roots {
        if !host.is_drive_available(&lib_root) {
            unavailable_roots.push(lib_root);
            continue;
        }
        let steamapps = lib_root.join("steamapps");
        let Ok(entries) = std::fs::read_dir(&steamapps) else {
            continue;
        };

        let mut manifest_paths: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|name| {
                            let lower = name.to_ascii_lowercase();
                            lower.starts_with("appmanifest_") && lower.ends_with(".acf")
                        })
            })
            .collect();
        manifest_paths.sort();

        for manifest_path in manifest_paths {
            let Ok(acf_text) = std::fs::read_to_string(&manifest_path) else {
                continue;
            };
            let Some(manifest) = parse_app_manifest(&acf_text) else {
                continue;
            };
            if is_steam_redistributable(&manifest.appid, &manifest.name) {
                continue;
            }

            let clean_name = clean_steam_game_name(&manifest.name);
            let game_dir = steamapps.join("common").join(&manifest.installdir);
            let icon_source_path = if game_dir.is_dir() {
                find_steam_game_exe(&game_dir, &clean_name)
                    .map(|p| p.to_string_lossy().into_owned())
            } else {
                None
            };

            let dismissal_key = format!("steam:{}", manifest.appid.to_ascii_lowercase());
            games.push(DockItem {
                id: derive_scanned_id(&dismissal_key),
                name: clean_name,
                path: format!("steam://rungameid/{}", manifest.appid),
                arguments: String::new(),
                description: "Steam Game".to_owned(),
                category: Category::Games,
                custom_icon_path: None,
                icon_source_path,
                run_as_admin: false,
                pinned: false,
                source: ItemSource::Steam,
                steam_app_id: Some(manifest.appid),
                launch_count: 0,
            });
        }
    }

    (true, unavailable_roots, games)
}

/// Parsed Steam `appmanifest_*.acf` fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SteamAppManifest {
    pub appid: String,
    pub name: String,
    pub installdir: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum VdfValue {
    Str(String),
    Map(Vec<(String, VdfValue)>),
}

impl VdfValue {
    fn as_map(&self) -> Option<&[(String, VdfValue)]> {
        match self {
            Self::Map(entries) => Some(entries),
            Self::Str(_) => None,
        }
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            Self::Map(_) => None,
        }
    }
}

fn get_vdf_ci<'a>(entries: &'a [(String, VdfValue)], key: &str) -> Option<&'a VdfValue> {
    entries
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum VdfToken {
    Word(String),
    OpenBrace,
    CloseBrace,
}

fn tokenize_vdf(input: &str) -> Vec<VdfToken> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];
        if ch.is_whitespace() {
            i += 1;
            continue;
        }
        // Skip `//` single-line comments.
        if ch == '/' && chars.get(i + 1) == Some(&'/') {
            i += 2;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if ch == '{' {
            tokens.push(VdfToken::OpenBrace);
            i += 1;
            continue;
        }
        if ch == '}' {
            tokens.push(VdfToken::CloseBrace);
            i += 1;
            continue;
        }
        if ch == '"' {
            i += 1;
            let mut s = String::new();
            while i < chars.len() {
                let c = chars[i];
                if c == '"' {
                    i += 1;
                    break;
                }
                if c == '\\' {
                    if let Some(&next) = chars.get(i + 1) {
                        match next {
                            '\\' => {
                                s.push('\\');
                                i += 2;
                                continue;
                            }
                            '"' => {
                                s.push('"');
                                i += 2;
                                continue;
                            }
                            'n' => {
                                s.push('\n');
                                i += 2;
                                continue;
                            }
                            't' => {
                                s.push('\t');
                                i += 2;
                                continue;
                            }
                            _ => {
                                // Preserve literal unescaped Windows backslashes if encountered.
                                s.push('\\');
                                i += 1;
                                continue;
                            }
                        }
                    }
                }
                s.push(c);
                i += 1;
            }
            tokens.push(VdfToken::Word(s));
            continue;
        }

        // Unquoted token.
        let mut s = String::new();
        while i < chars.len() {
            let c = chars[i];
            if c.is_whitespace() || c == '{' || c == '}' || c == '"' {
                break;
            }
            s.push(c);
            i += 1;
        }
        if !s.is_empty() {
            tokens.push(VdfToken::Word(s));
        }
    }

    tokens
}

fn parse_vdf_block(tokens: &[VdfToken], idx: &mut usize, inside_braces: bool) -> Vec<(String, VdfValue)> {
    let mut entries = Vec::new();
    while *idx < tokens.len() {
        match &tokens[*idx] {
            VdfToken::CloseBrace => {
                if inside_braces {
                    *idx += 1;
                }
                break;
            }
            VdfToken::OpenBrace => {
                *idx += 1;
            }
            VdfToken::Word(key) => {
                let key_owned = key.clone();
                *idx += 1;
                if *idx >= tokens.len() {
                    break;
                }
                match &tokens[*idx] {
                    VdfToken::OpenBrace => {
                        *idx += 1;
                        let nested = parse_vdf_block(tokens, idx, true);
                        entries.push((key_owned, VdfValue::Map(nested)));
                    }
                    VdfToken::Word(val) => {
                        let val_owned = val.clone();
                        *idx += 1;
                        entries.push((key_owned, VdfValue::Str(val_owned)));
                    }
                    VdfToken::CloseBrace => {
                        if inside_braces {
                            *idx += 1;
                        }
                        break;
                    }
                }
            }
        }
    }
    entries
}

fn parse_vdf(input: &str) -> Vec<(String, VdfValue)> {
    let tokens = tokenize_vdf(input);
    let mut idx = 0;
    parse_vdf_block(&tokens, &mut idx, false)
}

/// Extracts all Steam library folder roots from `libraryfolders.vdf`, deduplicated.
#[must_use]
pub fn extract_steam_library_roots(steam_root: &Path, vdf_text: &str) -> Vec<PathBuf> {
    let parsed = parse_vdf(vdf_text);
    let mut roots = vec![steam_root.to_path_buf()];
    let mut seen = HashSet::new();
    seen.insert(normalize_path_key(&steam_root.to_string_lossy()));

    let Some(lib_map) = get_vdf_ci(&parsed, "libraryfolders").and_then(VdfValue::as_map) else {
        return roots;
    };

    for (key, value) in lib_map {
        let candidate = match value {
            VdfValue::Map(entries) => get_vdf_ci(entries, "path")
                .and_then(VdfValue::as_str)
                .map(str::to_owned),
            VdfValue::Str(s) if key.chars().all(|c| c.is_ascii_digit()) => Some(s.clone()),
            VdfValue::Str(_) => None,
        };

        if let Some(raw_path) = candidate {
            let trimmed = raw_path.trim();
            if trimmed.is_empty() {
                continue;
            }
            let norm = normalize_path_key(trimmed);
            if seen.insert(norm) {
                roots.push(PathBuf::from(trimmed));
            }
        }
    }

    roots
}

/// Parses a Steam `appmanifest_<id>.acf` file into [`SteamAppManifest`].
#[must_use]
pub fn parse_app_manifest(acf_text: &str) -> Option<SteamAppManifest> {
    let parsed = parse_vdf(acf_text);
    let app_state = get_vdf_ci(&parsed, "AppState").and_then(VdfValue::as_map)?;

    let appid = get_vdf_ci(app_state, "appid")
        .and_then(VdfValue::as_str)?
        .trim()
        .to_owned();
    let name = get_vdf_ci(app_state, "name")
        .and_then(VdfValue::as_str)?
        .trim()
        .to_owned();
    let installdir = get_vdf_ci(app_state, "installdir")
        .and_then(VdfValue::as_str)?
        .trim()
        .to_owned();

    if appid.is_empty() || name.is_empty() || installdir.is_empty() {
        return None;
    }

    Some(SteamAppManifest {
        appid,
        name,
        installdir,
    })
}

/// Strips trademark/copyright symbols (`®`, `™`, `©`) and collapses extra spaces from Steam names.
#[must_use]
pub fn clean_steam_game_name(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|&c| c != '\u{00AE}' && c != '\u{2122}' && c != '\u{00A9}')
        .collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_steam_redistributable(appid: &str, name: &str) -> bool {
    if matches!(
        appid.trim(),
        "480" // Spacewar SDK test app
            | "228980"
            | "1070560"
            | "1391110"
            | "1628350"
            | "1493710"
            | "1887720"
            | "2180100"
            | "2348590"
            | "858260"
            | "961940"
            | "1113280"
            | "1245040"
            | "1420170"
            | "1580130"
    ) {
        return true;
    }

    let tokens = tokenize_words(name);
    const REDIST_PHRASES: &[&[&str]] = &[
        &["spacewar"],
        &["steamworks", "common", "redistributables"],
        &["steam", "linux", "runtime"],
        &["redistributable"],
        &["dedicated", "server"],
        &["proton"],
    ];
    contains_any_phrase(&tokens, REDIST_PHRASES)
}

fn find_steam_game_exe(game_dir: &Path, game_name: &str) -> Option<PathBuf> {
    // Prefer top-level executables first.
    let top_level = collect_exes_at_depth(game_dir, 0, 0);
    if let Some(best) = pick_best_steam_exe(&top_level, game_name) {
        return Some(best);
    }
    let nested = collect_exes_at_depth(game_dir, 0, MAX_STEAM_EXE_DEPTH);
    pick_best_steam_exe(&nested, game_name)
}

fn collect_exes_at_depth(dir: &Path, depth: usize, max_depth: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_symlink() {
            continue;
        }
        let path = entry.path();
        if ft.is_file() {
            let is_exe = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("exe"));
            if is_exe && !is_ignored_steam_exe(&path) {
                out.push(path);
            }
        } else if ft.is_dir() && depth < max_depth {
            out.extend(collect_exes_at_depth(&path, depth + 1, max_depth));
        }
    }
    out
}

fn is_ignored_steam_exe(path: &Path) -> bool {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    const IGNORED_SUBSTRS: &[&str] = &[
        "crash",
        "unins",
        "setup",
        "vconsole",
        "redist",
        "dxsetup",
        "vcredist",
        "ueprereq",
        "bugsplat",
        "reporter",
        "easyanticheat",
        "battleye",
    ];
    IGNORED_SUBSTRS.iter().any(|needle| stem.contains(needle))
}

fn pick_best_steam_exe(candidates: &[PathBuf], game_name: &str) -> Option<PathBuf> {
    let game_tokens: HashSet<String> = tokenize_words(game_name).into_iter().collect();

    candidates
        .iter()
        .max_by_key(|path| {
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            let exe_tokens = tokenize_words(stem);
            let overlap = exe_tokens
                .iter()
                .filter(|t| game_tokens.contains(*t))
                .count() as u64;
            let not_launcher = u64::from(!stem.to_ascii_lowercase().contains("launcher"));
            let file_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
            (overlap, not_launcher, file_size)
        })
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeHost {
        unavailable_prefixes: Vec<String>,
    }

    impl FakeHost {
        fn all_available() -> Self {
            Self {
                unavailable_prefixes: Vec::new(),
            }
        }

        fn with_unavailable(prefixes: &[&str]) -> Self {
            Self {
                unavailable_prefixes: prefixes.iter().map(|s| normalize_path_key(s)).collect(),
            }
        }
    }

    impl ScanHost for FakeHost {
        fn is_drive_available(&self, path: &Path) -> bool {
            let norm = normalize_path_key(&path.to_string_lossy());
            !self
                .unavailable_prefixes
                .iter()
                .any(|prefix| norm.starts_with(prefix))
        }

        fn path_exists(&self, path: &Path) -> bool {
            path.exists()
        }
    }

    struct FakeResolver {
        map: HashMap<PathBuf, ResolvedLnk>,
    }

    impl ShortcutResolver for FakeResolver {
        fn resolve_lnk(&self, lnk_path: &Path) -> Option<ResolvedLnk> {
            self.map.get(lnk_path).cloned()
        }
    }

    fn make_temp_dir(label: &str) -> AppResult<PathBuf> {
        let seq = MANUAL_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "harbor_scan_test_{label}_{}_{}",
            std::process::id(),
            seq
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    #[test]
    fn parses_steam_vdf_and_acf_manifests() {
        let vdf = r#"
// Steam library folders
"libraryfolders"
{
    "0"
    {
        "path"      "C:\\Program Files (x86)\\Steam"
        "label"     ""
        "apps"
        {
            "228980"    "123"
            "730"       "456"
        }
    }
    "1"
    {
        "path"      "D:\\SteamLibrary"
    }
}
"#;
        let steam_root = Path::new("C:\\Program Files (x86)\\Steam");
        let roots = extract_steam_library_roots(steam_root, vdf);
        assert_eq!(roots.len(), 2);
        assert_eq!(roots[0], steam_root);
        assert_eq!(roots[1], PathBuf::from("D:\\SteamLibrary"));

        let acf = r#"
"AppState"
{
    "appid"         "1145360"
    "Universe"      "1"
    "name"          "Hades"
    "StateFlags"    "4"
    "installdir"    "Hades"
}
"#;
        let Some(manifest) = parse_app_manifest(acf) else {
            panic!("expected parsed AppState");
        };
        assert_eq!(manifest.appid, "1145360");
        assert_eq!(manifest.name, "Hades");
        assert_eq!(manifest.installdir, "Hades");
    }

    #[test]
    fn whole_token_categorization_avoids_substring_false_positives() {
        assert_eq!(
            categorize_shortcut("Google Chrome", "C:\\Program Files\\Google\\Chrome\\chrome.exe"),
            Category::Apps
        );
        assert_eq!(
            categorize_shortcut("Steamboat Reader", "C:\\Apps\\steamboat.exe"),
            Category::Apps
        );
        assert_eq!(
            categorize_shortcut("Battle.net", "C:\\Program Files (x86)\\Battle.net\\Battle.net Launcher.exe"),
            Category::Games
        );
        assert_eq!(
            categorize_shortcut("RivaTuner Statistics Server", "C:\\Tools\\RTSS.exe"),
            Category::Games
        );
        assert_eq!(
            categorize_shortcut("Windows Terminal", "C:\\Users\\Me\\wt.exe"),
            Category::Tools
        );
        assert_eq!(
            categorize_shortcut("Resource Monitor", "C:\\Windows\\System32\\perfmon.exe"),
            Category::Tools
        );
    }

    #[test]
    fn rescan_preserves_user_edits_honors_dismissed_and_skips_unavailable_steam_drives(
    ) -> AppResult<()> {
        let root = make_temp_dir("merge")?;
        let steam_root = root.join("Steam");
        let steamapps = steam_root.join("steamapps");
        let hades_dir = steamapps.join("common\\Hades");
        std::fs::create_dir_all(&hades_dir)?;
        std::fs::write(hades_dir.join("Hades.exe"), b"exe")?;
        std::fs::write(hades_dir.join("CrashReporter.exe"), b"crash")?;

        std::fs::write(
            steamapps.join("libraryfolders.vdf"),
            format!(
                "\"libraryfolders\"\n{{\n  \"0\"\n  {{\n    \"path\" \"{}\"\n  }}\n  \"1\"\n  {{\n    \"path\" \"Z:\\\\OfflineLibrary\"\n  }}\n}}\n",
                steam_root.to_string_lossy().replace('\\', "\\\\")
            ),
        )?;

        // Installed Hades + Steamworks Redist (228980) + Spacewar (480) + Call of Duty with ®/™ + Celeste.
        std::fs::write(
            steamapps.join("appmanifest_1145360.acf"),
            "\"AppState\" { \"appid\" \"1145360\" \"name\" \"Hades\" \"installdir\" \"Hades\" }",
        )?;
        std::fs::write(
            steamapps.join("appmanifest_228980.acf"),
            "\"AppState\" { \"appid\" \"228980\" \"name\" \"Steamworks Common Redistributables\" \"installdir\" \"Steamworks Shared\" }",
        )?;
        std::fs::write(
            steamapps.join("appmanifest_480.acf"),
            "\"AppState\" { \"appid\" \"480\" \"name\" \"Spacewar\" \"installdir\" \"Spacewar\" }",
        )?;
        std::fs::write(
            steamapps.join("appmanifest_504230.acf"),
            "\"AppState\" { \"appid\" \"504230\" \"name\" \"Celeste™\" \"installdir\" \"Celeste\" }",
        )?;

        // Dismissed game should NOT be re-added even if present.
        std::fs::write(
            steamapps.join("appmanifest_620.acf"),
            "\"AppState\" { \"appid\" \"620\" \"name\" \"Portal 2\" \"installdir\" \"Portal 2\" }",
        )?;

        let mut config = DockConfig::default();
        config.dismissed.push("steam:620".to_owned());

        // Existing user-edited Hades item.
        config.items.push(DockItem {
            id: "user-hades".to_owned(),
            name: "Hades (Custom Title)".to_owned(),
            path: "steam://rungameid/1145360".to_owned(),
            arguments: "-vulkan".to_owned(),
            description: "My favorite roguelike".to_owned(),
            category: Category::Games,
            custom_icon_path: Some("C:\\Custom\\hades.png".to_owned()),
            icon_source_path: None,
            run_as_admin: true,
            pinned: true,
            source: ItemSource::Steam,
            steam_app_id: Some("1145360".to_owned()),
            launch_count: 99,
        });

        // Existing Steam game located on unavailable Z: drive -> must NOT be reported missing.
        config.items.push(DockItem {
            id: "offline-z-game".to_owned(),
            name: "Elden Ring".to_owned(),
            path: "steam://rungameid/1245620".to_owned(),
            arguments: String::new(),
            description: "Steam Game".to_owned(),
            category: Category::Games,
            custom_icon_path: None,
            icon_source_path: Some("Z:\\OfflineLibrary\\steamapps\\common\\ELDEN RING\\eldenring.exe".to_owned()),
            run_as_admin: false,
            pinned: false,
            source: ItemSource::Steam,
            steam_app_id: Some("1245620".to_owned()),
            launch_count: 5,
        });

        // Existing Steam game on mounted local drive whose manifest is gone -> SHOULD be reported missing.
        let gone_bin = steamapps.join("common\\GoneGame\\gone.exe");
        config.items.push(DockItem {
            id: "uninstalled-local-game".to_owned(),
            name: "Uninstalled Game".to_owned(),
            path: "steam://rungameid/999999".to_owned(),
            arguments: String::new(),
            description: "Steam Game".to_owned(),
            category: Category::Games,
            custom_icon_path: None,
            icon_source_path: Some(gone_bin.to_string_lossy().into_owned()),
            run_as_admin: false,
            pinned: false,
            source: ItemSource::Steam,
            steam_app_id: Some("999999".to_owned()),
            launch_count: 1,
        });

        // Manual item pointing to non-existent file must NEVER be marked missing.
        config.items.push(DockItem {
            id: "manual-missing-ok".to_owned(),
            name: "Custom Tool".to_owned(),
            path: root.join("nonexistent_manual.exe").to_string_lossy().into_owned(),
            arguments: String::new(),
            description: String::new(),
            category: Category::Tools,
            custom_icon_path: None,
            icon_source_path: None,
            run_as_admin: false,
            pinned: false,
            source: ItemSource::Manual,
            steam_app_id: None,
            launch_count: 0,
        });

        let env = ScanEnvironment {
            steam_root: Some(steam_root),
            start_menu_dirs: Vec::new(),
            system_utilities: Vec::new(),
        };
        let host = FakeHost::with_unavailable(&["z:\\"]);
        let resolver = FakeResolver {
            map: HashMap::new(),
        };

        let scan = discover_items(&env, &resolver, &host);
        let rescan = merge_discovered_items(&mut config, &scan, &host);

        // Only Celeste should be added (Hades existed, Portal 2 dismissed, 228980 & 480 skipped).
        assert_eq!(rescan.added, 1);
        assert!(config.items.iter().any(|i| i.name == "Celeste"));
        assert!(!config.items.iter().any(|i| i.name == "Portal 2"));
        assert!(!config
            .items
            .iter()
            .any(|i| i.steam_app_id.as_deref() == Some("228980")));
        assert!(!config
            .items
            .iter()
            .any(|i| i.steam_app_id.as_deref() == Some("480")));
        assert_eq!(
            clean_steam_game_name("Call of Duty®: Modern Warfare® ©"),
            "Call of Duty: Modern Warfare"
        );

        // User-edited fields on Hades must stay completely untouched, while missing icon_source_path backfills.
        let Some(hades) = config.items.iter().find(|i| i.id == "user-hades") else {
            panic!("Hades missing after merge");
        };
        assert_eq!(hades.name, "Hades (Custom Title)");
        assert_eq!(hades.arguments, "-vulkan");
        assert_eq!(hades.description, "My favorite roguelike");
        assert_eq!(hades.custom_icon_path.as_deref(), Some("C:\\Custom\\hades.png"));
        assert!(hades.run_as_admin);
        assert!(hades.pinned);
        assert_eq!(hades.launch_count, 99);
        assert_eq!(
            hades.icon_source_path.as_deref(),
            Some(hades_dir.join("Hades.exe").to_string_lossy().as_ref())
        );

        // Only the uninstalled local game should appear in missing_ids (not Z: offline game, not manual).
        assert_eq!(rescan.missing_ids, vec!["uninstalled-local-game".to_owned()]);

        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }

    #[test]
    fn start_menu_deduplicates_across_user_common_and_existing_exe() -> AppResult<()> {
        let root = make_temp_dir("dedupe")?;
        let user_sm = root.join("UserStartMenu\\Programs\\Browsers");
        let common_sm = root.join("CommonStartMenu\\Programs");
        let apps_dir = root.join("Apps");
        std::fs::create_dir_all(&user_sm)?;
        std::fs::create_dir_all(&common_sm)?;
        std::fs::create_dir_all(&apps_dir)?;

        let firefox_exe = apps_dir.join("firefox.exe");
        let chrome_exe = apps_dir.join("chrome.exe");
        let wt_exe = apps_dir.join("wt.exe");
        std::fs::write(&firefox_exe, b"ff")?;
        std::fs::write(&chrome_exe, b"ch")?;
        std::fs::write(&wt_exe, b"wt")?;

        let user_ff_lnk = user_sm.join("Firefox.lnk");
        let common_ff_lnk = common_sm.join("Firefox.lnk");
        let user_chrome_lnk = user_sm.join("Google Chrome.lnk");
        let uninst_lnk = common_sm.join("Uninstall Firefox.lnk");
        let wt_lnk = user_sm.join("Windows Terminal.lnk");

        std::fs::write(&user_ff_lnk, b"lnk")?;
        std::fs::write(&common_ff_lnk, b"lnk")?;
        std::fs::write(&user_chrome_lnk, b"lnk")?;
        std::fs::write(&uninst_lnk, b"lnk")?;
        std::fs::write(&wt_lnk, b"lnk")?;

        let mut lnk_map = HashMap::new();
        lnk_map.insert(
            user_ff_lnk,
            ResolvedLnk {
                target_path: firefox_exe.to_string_lossy().into_owned(),
                arguments: String::new(),
                description: "Mozilla Firefox".to_owned(),
            },
        );
        lnk_map.insert(
            common_ff_lnk,
            ResolvedLnk {
                // Intentionally alter case & slashes to test canonical deduplication.
                target_path: firefox_exe.to_string_lossy().replace('\\', "/").to_uppercase(),
                arguments: String::new(),
                description: "Firefox Common".to_owned(),
            },
        );
        lnk_map.insert(
            user_chrome_lnk,
            ResolvedLnk {
                target_path: chrome_exe.to_string_lossy().into_owned(),
                arguments: String::new(),
                description: "Web Browser".to_owned(),
            },
        );
        lnk_map.insert(
            uninst_lnk,
            ResolvedLnk {
                target_path: firefox_exe.to_string_lossy().into_owned(),
                arguments: String::new(),
                description: String::new(),
            },
        );
        lnk_map.insert(
            wt_lnk,
            ResolvedLnk {
                target_path: wt_exe.to_string_lossy().into_owned(),
                arguments: String::new(),
                description: String::new(),
            },
        );

        let resolver = FakeResolver { map: lnk_map };
        let host = FakeHost::all_available();
        let env = ScanEnvironment {
            steam_root: None,
            start_menu_dirs: vec![
                root.join("UserStartMenu\\Programs"),
                root.join("CommonStartMenu\\Programs"),
            ],
            system_utilities: Vec::new(),
        };

        // Pre-populate Chrome as an already-added Manual `.exe` entry; `.lnk` pointing to it must not duplicate.
        let mut config = DockConfig::default();
        config.items.push(DockItem {
            id: generate_manual_id(&chrome_exe.to_string_lossy()),
            name: "Chrome (Pinned)".to_owned(),
            path: chrome_exe.to_string_lossy().into_owned(),
            arguments: String::new(),
            description: "Manual browser".to_owned(),
            category: Category::Apps,
            custom_icon_path: None,
            icon_source_path: None,
            run_as_admin: false,
            pinned: true,
            source: ItemSource::Manual,
            steam_app_id: None,
            launch_count: 4,
        });

        let scan = discover_items(&env, &resolver, &host);
        let rescan = merge_discovered_items(&mut config, &scan, &host);

        // Only Firefox (once!) + Windows Terminal should be added; Chrome and Uninstall Firefox are skipped.
        assert_eq!(rescan.added, 2);
        let ff_count = config
            .items
            .iter()
            .filter(|i| i.name == "Firefox")
            .count();
        assert_eq!(ff_count, 1);

        let chrome_count = config
            .items
            .iter()
            .filter(|i| i.name.contains("Chrome"))
            .count();
        assert_eq!(chrome_count, 1);

        let Some(wt) = config.items.iter().find(|i| i.name == "Windows Terminal") else {
            panic!("Windows Terminal missing");
        };
        assert_eq!(wt.category, Category::Tools);

        // Second rescan should add 0 duplicates.
        let scan2 = discover_items(&env, &resolver, &host);
        let rescan2 = merge_discovered_items(&mut config, &scan2, &host);
        assert_eq!(rescan2.added, 0);

        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }
}
