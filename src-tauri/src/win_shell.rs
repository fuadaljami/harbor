use std::collections::HashSet;
use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::Write;
use std::mem::size_of;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use image::imageops::FilterType;
use image::{ImageFormat, RgbaImage};
use parking_lot::RwLock;
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, oneshot};
use windows::core::{HRESULT, PCWSTR, PWSTR};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, HINSTANCE, HWND, POINT, RECT, SIZE};
use windows::Win32::Graphics::Gdi::{
    CombineRgn, CreateRectRgn, DeleteObject, GetDC, GetDIBits, GetMonitorInfoW, GetObjectW,
    MonitorFromPoint, ReleaseDC, SetWindowRgn, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ, HRGN, MONITORINFO, MONITOR_DEFAULTTONEAREST, RGN_ERROR,
    RGN_OR,
};
use windows::Win32::Storage::FileSystem::{GetDriveTypeW, FILE_ATTRIBUTE_NORMAL};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IPersistFile,
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, STGM_READ,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW,
    RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE,
    REG_SZ, REG_VALUE_TYPE,
};
use windows::Win32::System::Shutdown::LockWorkStation;
use windows::Win32::System::WindowsProgramming::{
    DRIVE_FIXED, DRIVE_NO_ROOT_DIR, DRIVE_RAMDISK, DRIVE_REMOVABLE, DRIVE_UNKNOWN,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON};
use windows::Win32::UI::Shell::{
    FOLDERID_CommonPrograms, FOLDERID_Programs, IShellItemImageFactory, IShellLinkW,
    SHCreateItemFromParsingName, SHGetFileInfoW, SHGetKnownFolderPath, ShellExecuteExW, ShellLink,
    KF_FLAG_DEFAULT, SEE_MASK_FLAG_NO_UI, SHELLEXECUTEINFOW, SHFILEINFOW, SHGFI_ICON,
    SHGFI_LARGEICON, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, GetCursorPos, GetForegroundWindow, GetIconInfo, GetSystemMetrics, MessageBoxW,
    SetWindowPos, HICON, ICONINFO, MB_ICONERROR, MB_OK, SM_SWAPBUTTON, SWP_ASYNCWINDOWPOS,
    SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SW_SHOWNORMAL,
};

use crate::config::AppPaths;
use crate::error::{AppError, AppResult};
use crate::models::normalize_path_key;

const ICON_SIZE_PX: i32 = 64;
const ICON_TIMEOUT_SECS: u64 = 3;
const ICON_CHANNEL_CAPACITY: usize = 64;
const LAUNCH_CHANNEL_CAPACITY: usize = 16;
const HRESULT_ERROR_CANCELLED: HRESULT = HRESULT::from_win32(1223);
const STARTUP_RUN_SUBKEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const STARTUP_VALUE_NAME: &str = "Harbor";
const STEAM_REG_SUBKEY: &str = "Software\\Valve\\Steam";

static CACHE_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Resolved metadata extracted from a Windows `.lnk` shortcut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLnk {
    pub target_path: String,
    pub arguments: String,
    pub description: String,
}

/// Logical work-area bounds queried from Win32 `GetMonitorInfoW`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Win32Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

enum IconShellJob {
    ExtractIcon {
        source_path: String,
        icon_cache_dir: PathBuf,
        reply: oneshot::Sender<AppResult<Option<String>>>,
    },
    ResolveLnk {
        lnk_path: PathBuf,
        reply: oneshot::Sender<AppResult<Option<ResolvedLnk>>>,
    },
}

enum LaunchJob {
    Execute {
        target: String,
        arguments: String,
        run_as_admin: bool,
        reply: oneshot::Sender<AppResult<()>>,
    },
    OpenLocation {
        target: String,
        reply: oneshot::Sender<AppResult<()>>,
    },
}

/// RAII wrapper ensuring `CoUninitialize` balances a successful `CoInitializeEx`.
struct ComGuard {
    initialized: bool,
}

impl ComGuard {
    fn initialize(coinit_flags: windows::Win32::System::Com::COINIT) -> AppResult<Self> {
        // SAFETY: Initializes COM apartment state once for the current dedicated OS worker thread.
        let hr = unsafe { CoInitializeEx(None, coinit_flags) };
        if hr.is_err() {
            return Err(AppError::Shell(format!(
                "CoInitializeEx failed: HRESULT 0x{:08X}",
                hr.0 as u32
            )));
        }
        Ok(Self { initialized: true })
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: Balances the successful `CoInitializeEx` on this exact worker thread.
            unsafe {
                CoUninitialize();
            }
        }
    }
}

/// RAII wrapper for Win32 `HBITMAP` calling `DeleteObject` on drop.
struct OwnedHbitmap(HBITMAP);

impl OwnedHbitmap {
    fn new(handle: HBITMAP) -> Self {
        Self(handle)
    }

    fn as_raw(&self) -> HBITMAP {
        self.0
    }

    fn is_valid(&self) -> bool {
        !self.0.is_invalid()
    }
}

impl Drop for OwnedHbitmap {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: Releases the owned GDI bitmap handle exactly once after validity check.
            unsafe {
                let _ = DeleteObject(HGDIOBJ(self.0 .0));
            }
        }
    }
}

/// RAII wrapper for Win32 `HRGN` calling `DeleteObject` on drop unless disarmed after
/// handing ownership to `SetWindowRgn`.
struct OwnedHrgn(HRGN);

impl OwnedHrgn {
    fn new(handle: HRGN) -> Self {
        Self(handle)
    }

    fn as_raw(&self) -> HRGN {
        self.0
    }

    fn is_valid(&self) -> bool {
        !self.0.is_invalid()
    }

    fn disarm(&mut self) {
        self.0 = HRGN::default();
    }
}

impl Drop for OwnedHrgn {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: Deletes an owned GDI region handle that was not transferred to `SetWindowRgn`.
            unsafe {
                let _ = DeleteObject(HGDIOBJ(self.0 .0));
            }
        }
    }
}

/// RAII wrapper for Win32 `HICON` calling `DestroyIcon` on drop.
struct OwnedHicon(HICON);

impl OwnedHicon {
    fn new(handle: HICON) -> Self {
        Self(handle)
    }

    fn as_raw(&self) -> HICON {
        self.0
    }

    fn is_valid(&self) -> bool {
        !self.0.is_invalid()
    }
}

impl Drop for OwnedHicon {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: Destroys the owned Win32 icon handle obtained from `SHGetFileInfoW`.
            unsafe {
                let _ = DestroyIcon(self.0);
            }
        }
    }
}

/// RAII wrapper for screen `HDC` calling `ReleaseDC` on drop.
struct OwnedScreenDc(HDC);

impl OwnedScreenDc {
    fn acquire_screen() -> AppResult<Self> {
        // SAFETY: Acquires a display device context for the entire virtual screen; checked below.
        let hdc = unsafe { GetDC(None) };
        if hdc.is_invalid() {
            return Err(AppError::Shell("GetDC(None) returned null handle".to_owned()));
        }
        Ok(Self(hdc))
    }

    fn as_raw(&self) -> HDC {
        self.0
    }
}

impl Drop for OwnedScreenDc {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: Releases the screen DC acquired in `acquire_screen`.
            unsafe {
                let _ = ReleaseDC(None, self.0);
            }
        }
    }
}

/// RAII wrapper for Win32 `HKEY` calling `RegCloseKey` on drop.
struct OwnedHkey(HKEY);

impl OwnedHkey {
    fn as_raw(&self) -> HKEY {
        self.0
    }
}

impl Drop for OwnedHkey {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: Closes an open Win32 registry key handle owned by this guard.
            unsafe {
                let _ = RegCloseKey(self.0);
            }
        }
    }
}

/// RAII wrapper for COM task-allocated buffers freed via `CoTaskMemFree`.
struct CoTaskMemPwstr(PWSTR);

impl Drop for CoTaskMemPwstr {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: Frees a valid COM task-allocated buffer returned by `SHGetKnownFolderPath`.
            unsafe {
                CoTaskMemFree(Some(self.0 .0.cast()));
            }
        }
    }
}

/// Dual-STA Windows Shell & Launcher service.
///
/// Uses one dedicated STA thread for icon extraction and `.lnk` resolution and a separate
/// dedicated STA thread (`COINIT_DISABLE_OLE1DDE`) for `ShellExecuteExW` launches so a pending
/// UAC prompt cannot stall icon extraction.
#[derive(Clone)]
pub struct WinShell {
    icon_tx: mpsc::Sender<IconShellJob>,
    launch_tx: mpsc::Sender<LaunchJob>,
    negative_cache: Arc<RwLock<HashSet<String>>>,
}

impl WinShell {
    /// Spawns both dedicated COM STA worker threads and verifies COM initialization.
    pub fn new() -> AppResult<Self> {
        let (icon_tx, mut icon_rx) = mpsc::channel::<IconShellJob>(ICON_CHANNEL_CAPACITY);
        let (icon_init_tx, icon_init_rx) = std::sync::mpsc::sync_channel::<AppResult<()>>(1);

        std::thread::Builder::new()
            .name("harbor-shell-sta".to_owned())
            .spawn(move || {
                let _com = match ComGuard::initialize(COINIT_APARTMENTTHREADED) {
                    Ok(guard) => {
                        let _ = icon_init_tx.send(Ok(()));
                        guard
                    }
                    Err(err) => {
                        let _ = icon_init_tx.send(Err(err));
                        return;
                    }
                };

                while let Some(job) = icon_rx.blocking_recv() {
                    match job {
                        IconShellJob::ExtractIcon {
                            source_path,
                            icon_cache_dir,
                            reply,
                        } => {
                            if reply.is_closed() {
                                continue;
                            }
                            let res = extract_and_cache_icon_sync(&source_path, &icon_cache_dir);
                            let _ = reply.send(res);
                        }
                        IconShellJob::ResolveLnk { lnk_path, reply } => {
                            if reply.is_closed() {
                                continue;
                            }
                            let res = resolve_lnk_sync(&lnk_path);
                            let _ = reply.send(res);
                        }
                    }
                }
            })
            .map_err(|err| AppError::Shell(format!("Failed to spawn shell STA worker: {err}")))?;

        icon_init_rx
            .recv()
            .map_err(|_| AppError::WorkerUnavailable("Shell STA worker failed to start".to_owned()))??;

        let (launch_tx, mut launch_rx) = mpsc::channel::<LaunchJob>(LAUNCH_CHANNEL_CAPACITY);
        let (launch_init_tx, launch_init_rx) = std::sync::mpsc::sync_channel::<AppResult<()>>(1);

        std::thread::Builder::new()
            .name("harbor-launch-sta".to_owned())
            .spawn(move || {
                let _com = match ComGuard::initialize(
                    COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE,
                ) {
                    Ok(guard) => {
                        let _ = launch_init_tx.send(Ok(()));
                        guard
                    }
                    Err(err) => {
                        let _ = launch_init_tx.send(Err(err));
                        return;
                    }
                };

                while let Some(job) = launch_rx.blocking_recv() {
                    match job {
                        LaunchJob::Execute {
                            target,
                            arguments,
                            run_as_admin,
                            reply,
                        } => {
                            if reply.is_closed() {
                                continue;
                            }
                            let res = launch_target_sync(&target, &arguments, run_as_admin);
                            let _ = reply.send(res);
                        }
                        LaunchJob::OpenLocation { target, reply } => {
                            if reply.is_closed() {
                                continue;
                            }
                            let res = open_file_location_sync(&target);
                            let _ = reply.send(res);
                        }
                    }
                }
            })
            .map_err(|err| AppError::Shell(format!("Failed to spawn launch STA worker: {err}")))?;

        launch_init_rx.recv().map_err(|_| {
            AppError::WorkerUnavailable("Launch STA worker failed to start".to_owned())
        })??;

        Ok(Self {
            icon_tx,
            launch_tx,
            negative_cache: Arc::new(RwLock::new(HashSet::new())),
        })
    }

    /// Extracts and PNG-caches an icon for `source_path` with a 3-second timeout and session negative cache.
    pub async fn extract_icon_cached(
        &self,
        source_path: &str,
        icon_cache_dir: &Path,
    ) -> AppResult<Option<String>> {
        let trimmed = source_path.trim();
        if trimmed.is_empty() || is_non_file_uri(trimmed) {
            return Ok(None);
        }

        let neg_key = normalize_path_key(trimmed);
        if self.negative_cache.read().contains(&neg_key) {
            return Ok(None);
        }

        let path = Path::new(trimmed);
        if !is_drive_local_and_available(path) {
            return Ok(None);
        }

        let (reply_tx, reply_rx) = oneshot::channel();
        let job = IconShellJob::ExtractIcon {
            source_path: trimmed.to_owned(),
            icon_cache_dir: icon_cache_dir.to_path_buf(),
            reply: reply_tx,
        };

        let fut = async {
            self.icon_tx.send(job).await.map_err(|_| {
                AppError::WorkerUnavailable("Shell STA worker channel closed".to_owned())
            })?;
            reply_rx.await.map_err(|_| {
                AppError::WorkerUnavailable("Shell STA worker dropped reply".to_owned())
            })?
        };

        match tokio::time::timeout(Duration::from_secs(ICON_TIMEOUT_SECS), fut).await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(err)) => {
                tracing::warn!(
                    path = %trimmed,
                    error = %err,
                    "Icon extraction failed; falling back to monogram"
                );
                Ok(None)
            }
            Err(_) => {
                tracing::warn!(
                    path = %trimmed,
                    "Icon extraction timed out after {ICON_TIMEOUT_SECS}s; negative-caching path"
                );
                self.negative_cache.write().insert(neg_key);
                Ok(None)
            }
        }
    }

    /// Resolves a `.lnk` file on the dedicated Shell STA thread under a 3-second timeout.
    pub async fn resolve_lnk(&self, lnk_path: PathBuf) -> AppResult<Option<ResolvedLnk>> {
        if !is_drive_local_and_available(&lnk_path) {
            return Ok(None);
        }

        let (reply_tx, reply_rx) = oneshot::channel();
        let job = IconShellJob::ResolveLnk {
            lnk_path,
            reply: reply_tx,
        };

        let fut = async {
            self.icon_tx.send(job).await.map_err(|_| {
                AppError::WorkerUnavailable("Shell STA worker channel closed".to_owned())
            })?;
            reply_rx.await.map_err(|_| {
                AppError::WorkerUnavailable("Shell STA worker dropped reply".to_owned())
            })?
        };

        match tokio::time::timeout(Duration::from_secs(ICON_TIMEOUT_SECS), fut).await {
            Ok(res) => res,
            Err(_) => Ok(None),
        }
    }

    /// Launches a program, shortcut, or URI via `ShellExecuteExW` on the dedicated Launch STA thread.
    pub async fn launch_target(
        &self,
        target: &str,
        arguments: &str,
        run_as_admin: bool,
    ) -> AppResult<()> {
        let trimmed = target.trim();
        if trimmed.is_empty() {
            return Err(AppError::InvalidInput("Target path cannot be empty".to_owned()));
        }

        let (reply_tx, reply_rx) = oneshot::channel();
        let job = LaunchJob::Execute {
            target: trimmed.to_owned(),
            arguments: arguments.to_owned(),
            run_as_admin,
            reply: reply_tx,
        };

        self.launch_tx.send(job).await.map_err(|_| {
            AppError::WorkerUnavailable("Launch STA worker channel closed".to_owned())
        })?;

        reply_rx.await.map_err(|_| {
            AppError::WorkerUnavailable("Launch STA worker dropped reply".to_owned())
        })?
    }

    /// Opens Windows Explorer selecting the file or opening its containing folder.
    pub async fn open_file_location(&self, target: &str) -> AppResult<()> {
        let trimmed = target.trim();
        if trimmed.is_empty() {
            return Err(AppError::InvalidInput("Target path cannot be empty".to_owned()));
        }
        if is_non_file_uri(trimmed) {
            return Err(AppError::InvalidInput(
                "Cannot open file location for a URI target".to_owned(),
            ));
        }

        let (reply_tx, reply_rx) = oneshot::channel();
        let job = LaunchJob::OpenLocation {
            target: trimmed.to_owned(),
            reply: reply_tx,
        };

        self.launch_tx.send(job).await.map_err(|_| {
            AppError::WorkerUnavailable("Launch STA worker channel closed".to_owned())
        })?;

        reply_rx.await.map_err(|_| {
            AppError::WorkerUnavailable("Launch STA worker dropped reply".to_owned())
        })?
    }
}

/// Returns `true` when `raw` uses a URI scheme (`steam://`, `ms-settings:`, `shell:`, etc.)
/// rather than a Windows filesystem/UNC path.
#[must_use]
pub fn is_non_file_uri(raw: &str) -> bool {
    let trimmed = raw.trim().trim_matches('"');
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.contains("://") {
        return true;
    }
    if trimmed.starts_with("\\\\") {
        return false;
    }
    // Check `X:\` or `X:/` Windows drive prefix.
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return false;
    }
    // Any other `scheme:...` before path separators is treated as a URI/shell namespace.
    if let Some(colon_idx) = trimmed.find(':') {
        let prefix = &trimmed[..colon_idx];
        return !prefix.is_empty()
            && !prefix.contains('\\')
            && !prefix.contains('/');
    }
    false
}

/// Checks via `GetDriveTypeW` whether `path` resides on a local, currently available drive.
#[must_use]
pub fn is_drive_local_and_available(path: &Path) -> bool {
    let raw = path.to_string_lossy();
    if is_non_file_uri(&raw) {
        return false;
    }
    let Some(root) = extract_drive_root(path) else {
        // Reject UNC network shares (`\\server\share`) and relative paths for icon extraction.
        return false;
    };
    let wide = to_wide_null(&root);
    // SAFETY: `wide` is a null-terminated UTF-16 drive root string alive across the Win32 call.
    let drive_type = unsafe { GetDriveTypeW(PCWSTR(wide.as_ptr())) };
    match drive_type {
        DRIVE_FIXED | DRIVE_RAMDISK => true,
        DRIVE_REMOVABLE => Path::new(&root).exists(),
        _ => false,
    }
}

/// Checks whether the drive/volume for `path` is currently mounted/accessible.
#[must_use]
pub fn is_drive_mounted(path: &Path) -> bool {
    let raw = path.to_string_lossy();
    if is_non_file_uri(&raw) {
        return true;
    }
    if let Some(root) = extract_drive_root(path) {
        let wide = to_wide_null(&root);
        // SAFETY: `wide` is a null-terminated UTF-16 drive root string alive across the call.
        let drive_type = unsafe { GetDriveTypeW(PCWSTR(wide.as_ptr())) };
        drive_type != DRIVE_UNKNOWN && drive_type != DRIVE_NO_ROOT_DIR
    } else {
        path.exists()
    }
}

/// Strips Windows `\\?\` verbatim prefixes so Win32 Shell APIs (`SHCreateItemFromParsingName`,
/// `SHGetFileInfoW`, `ShellExecuteExW`, `GetDriveTypeW`) accept the path cleanly.
#[must_use]
pub fn strip_verbatim_prefix(path: &Path) -> PathBuf {
    let raw = path.to_string_lossy();
    if let Some(unc) = raw.strip_prefix("\\\\?\\UNC\\") {
        return PathBuf::from(format!("\\\\{unc}"));
    }
    if let Some(local) = raw.strip_prefix("\\\\?\\") {
        return PathBuf::from(local);
    }
    path.to_path_buf()
}

fn extract_drive_root(path: &Path) -> Option<String> {
    let clean = strip_verbatim_prefix(path);
    let raw = clean.to_string_lossy();
    let trimmed = raw.trim().trim_matches('"');
    if trimmed.starts_with("\\\\") {
        return None;
    }
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        let drive = (bytes[0] as char).to_ascii_uppercase();
        return Some(format!("{drive}:\\"));
    }
    None
}

/// Pure conversion from premultiplied BGRA pixels to straight RGBA pixels.
///
/// If every alpha byte is `0` while RGB channels contain non-zero data, treats the bitmap
/// as opaque (`alpha = 255`) before channel reordering.
pub fn bgra_premultiplied_to_straight_rgba(bgra: &[u8]) -> AppResult<Vec<u8>> {
    if !bgra.len().is_multiple_of(4) {
        return Err(AppError::Shell(format!(
            "BGRA pixel buffer length {} is not divisible by 4",
            bgra.len()
        )));
    }

    let (pixels, _) = bgra.as_chunks::<4>();
    let mut all_alpha_zero = true;
    let mut any_rgb_nonzero = false;
    for px in pixels {
        if px[3] != 0 {
            all_alpha_zero = false;
            break;
        }
        if px[0] != 0 || px[1] != 0 || px[2] != 0 {
            any_rgb_nonzero = true;
        }
    }
    let force_opaque = all_alpha_zero && any_rgb_nonzero;

    let mut rgba = Vec::with_capacity(bgra.len());
    for px in pixels {
        let b = px[0];
        let g = px[1];
        let r = px[2];
        let a = px[3];

        if force_opaque {
            rgba.extend_from_slice(&[r, g, b, 255]);
        } else if a == 0 {
            rgba.extend_from_slice(&[0, 0, 0, 0]);
        } else if a == 255 {
            rgba.extend_from_slice(&[r, g, b, 255]);
        } else {
            let alpha_u32 = u32::from(a);
            let r_straight = ((u32::from(r) * 255) / alpha_u32).min(255) as u8;
            let g_straight = ((u32::from(g) * 255) / alpha_u32).min(255) as u8;
            let b_straight = ((u32::from(b) * 255) / alpha_u32).min(255) as u8;
            rgba.extend_from_slice(&[r_straight, g_straight, b_straight, a]);
        }
    }

    Ok(rgba)
}

fn extract_and_cache_icon_sync(
    source_path: &str,
    icon_cache_dir: &Path,
) -> AppResult<Option<String>> {
    let input_path = strip_verbatim_prefix(Path::new(source_path));
    if !is_drive_local_and_available(&input_path) {
        return Ok(None);
    }

    let meta = match std::fs::metadata(&input_path) {
        Ok(m) => m,
        Err(err) => {
            tracing::warn!(path = %input_path.display(), error = %err, "Icon target metadata unavailable");
            return Ok(None);
        }
    };
    if !meta.is_file() {
        return Ok(None);
    }

    let canonical = strip_verbatim_prefix(
        &std::fs::canonicalize(&input_path).unwrap_or_else(|_| input_path.clone()),
    );
    let canonical_key = normalize_path_key(&canonical.to_string_lossy());
    let file_size = meta.len();
    let mtime_secs = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let clean_cache_dir = strip_verbatim_prefix(icon_cache_dir);
    let cache_hash = compute_icon_cache_key(&canonical_key, file_size, mtime_secs);
    let cache_file = clean_cache_dir.join(format!("{cache_hash}.png"));
    if cache_file.is_file() {
        return Ok(Some(cache_file.to_string_lossy().into_owned()));
    }

    std::fs::create_dir_all(&clean_cache_dir)?;

    // Support direct custom PNG icon files cleanly without going through thumbnail providers.
    let is_png = canonical
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("png"));

    if is_png {
        let bytes = std::fs::read(&canonical)?;
        let decoded = image::load_from_memory_with_format(&bytes, ImageFormat::Png)
            .map_err(|err| AppError::Shell(format!("Invalid custom PNG icon: {err}")))?;
        let rgba_img = if decoded.width() > 128 || decoded.height() > 128 {
            decoded
                .resize(ICON_SIZE_PX as u32, ICON_SIZE_PX as u32, FilterType::Triangle)
                .to_rgba8()
        } else {
            decoded.to_rgba8()
        };
        write_rgba_png_atomic(&cache_file, &rgba_img)?;
        return Ok(Some(cache_file.to_string_lossy().into_owned()));
    }

    let wide_path = to_wide_null(&canonical.to_string_lossy());
    let extracted = match extract_via_image_factory(&wide_path) {
        Ok(Some(pixels)) => Some(pixels),
        Ok(None) => extract_via_shgetfileinfo(&wide_path)?,
        Err(primary_err) => match extract_via_shgetfileinfo(&wide_path) {
            Ok(Some(pixels)) => Some(pixels),
            Ok(None) => {
                tracing::warn!(
                    path = %canonical.display(),
                    primary_error = %primary_err,
                    "Icon extraction returned empty handle from both IShellItemImageFactory and SHGetFileInfoW"
                );
                None
            }
            Err(fallback_err) => {
                tracing::warn!(
                    path = %canonical.display(),
                    primary_error = %primary_err,
                    fallback_error = %fallback_err,
                    "Icon extraction failed on both IShellItemImageFactory and SHGetFileInfoW"
                );
                return Err(fallback_err);
            }
        },
    };

    let Some((width, height, rgba)) = extracted else {
        return Ok(None);
    };

    let Some(img) = RgbaImage::from_raw(width, height, rgba) else {
        return Err(AppError::Shell("Failed to construct RGBA image buffer".to_owned()));
    };

    write_rgba_png_atomic(&cache_file, &img)?;
    Ok(Some(cache_file.to_string_lossy().into_owned()))
}

/// Computes a deterministic SHA-256 cache key from canonical path, size, and mtime.
#[must_use]
pub fn compute_icon_cache_key(canonical_path: &str, file_size: u64, mtime_secs: u64) -> String {
    let mut hasher = Sha256::new();
    hasher.update(canonical_path.as_bytes());
    hasher.update(b"|");
    hasher.update(file_size.to_le_bytes());
    hasher.update(b"|");
    hasher.update(mtime_secs.to_le_bytes());
    hex_encode(&hasher.finalize())
}

fn write_rgba_png_atomic(target: &Path, img: &RgbaImage) -> AppResult<()> {
    let parent = target
        .parent()
        .ok_or_else(|| AppError::Shell("Icon cache path missing parent".to_owned()))?;
    let seq = CACHE_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = parent.join(format!(".icon.{}.{}.tmp", std::process::id(), seq));

    let write_res: AppResult<()> = (|| {
        let mut png_bytes = std::io::Cursor::new(Vec::new());
        img.write_to(&mut png_bytes, ImageFormat::Png)
            .map_err(|err| AppError::Shell(format!("PNG encode failed: {err}")))?;
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        file.write_all(png_bytes.get_ref())?;
        file.sync_all()?;
        Ok(())
    })();

    if let Err(err) = write_res {
        let _ = std::fs::remove_file(&tmp);
        return Err(err);
    }

    if let Err(err) = std::fs::rename(&tmp, target) {
        let _ = std::fs::remove_file(&tmp);
        // If another concurrent job created the identical content-addressed file, reuse it.
        if target.is_file() {
            return Ok(());
        }
        return Err(AppError::Io(err));
    }

    Ok(())
}

fn extract_via_image_factory(wide_path: &[u16]) -> AppResult<Option<(u32, u32, Vec<u8>)>> {
    // SAFETY: Runs on the STA worker after COM init; `wide_path` is null-terminated UTF-16.
    let factory: IShellItemImageFactory =
        unsafe { SHCreateItemFromParsingName(PCWSTR(wide_path.as_ptr()), None) }
            .map_err(|err| AppError::Shell(format!("SHCreateItemFromParsingName failed: {err}")))?;

    // SAFETY: `factory` is a valid STA COM pointer and `SIZE` is a plain value struct.
    let hbmp_raw = unsafe {
        factory.GetImage(
            SIZE {
                cx: ICON_SIZE_PX,
                cy: ICON_SIZE_PX,
            },
            SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK,
        )
    }
    .map_err(|err| AppError::Shell(format!("IShellItemImageFactory::GetImage failed: {err}")))?;

    let hbmp = OwnedHbitmap::new(hbmp_raw);
    if !hbmp.is_valid() {
        return Ok(None);
    }
    extract_hbitmap_rgba(hbmp.as_raw())
}

fn extract_via_shgetfileinfo(wide_path: &[u16]) -> AppResult<Option<(u32, u32, Vec<u8>)>> {
    let mut shfi = SHFILEINFOW::default();
    // SAFETY: `wide_path` is null-terminated UTF-16 and `shfi` is properly sized & aligned.
    let res = unsafe {
        SHGetFileInfoW(
            PCWSTR(wide_path.as_ptr()),
            FILE_ATTRIBUTE_NORMAL,
            Some(&mut shfi),
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        )
    };
    let hicon = OwnedHicon::new(shfi.hIcon);
    if res == 0 || !hicon.is_valid() {
        return Ok(None);
    }

    let mut icon_info = ICONINFO::default();
    // SAFETY: `hicon` is validated non-null above and `icon_info` is valid out-param memory.
    unsafe { GetIconInfo(hicon.as_raw(), &mut icon_info) }
        .map_err(|err| AppError::Shell(format!("GetIconInfo failed: {err}")))?;

    // Immediately wrap BOTH GDI bitmaps in RAII guards so neither leaks on any branch.
    let color_bmp = OwnedHbitmap::new(icon_info.hbmColor);
    let _mask_bmp = OwnedHbitmap::new(icon_info.hbmMask);

    if !color_bmp.is_valid() {
        return Ok(None);
    }

    extract_hbitmap_rgba(color_bmp.as_raw())
}

fn extract_hbitmap_rgba(hbitmap: HBITMAP) -> AppResult<Option<(u32, u32, Vec<u8>)>> {
    let mut bmp = BITMAP::default();
    // SAFETY: `bmp` has exact `size_of::<BITMAP>()` bytes reserved on stack for `GetObjectW`.
    let obj_bytes = unsafe {
        GetObjectW(
            HGDIOBJ(hbitmap.0),
            size_of::<BITMAP>() as i32,
            Some((&mut bmp as *mut BITMAP).cast()),
        )
    };
    if obj_bytes == 0 || bmp.bmWidth <= 0 || bmp.bmHeight <= 0 {
        return Ok(None);
    }
    if bmp.bmWidth > 1024 || bmp.bmHeight > 1024 {
        return Err(AppError::Shell("Icon bitmap dimensions exceed safe limit".to_owned()));
    }

    let width = bmp.bmWidth as u32;
    let height = bmp.bmHeight as u32;
    let byte_len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|px| px.checked_mul(4))
        .ok_or_else(|| AppError::Shell("Bitmap buffer size overflow".to_owned()))?;

    let mut bgra = vec![0_u8; byte_len];
    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: bmp.bmWidth,
            // Negative height requests top-down scanline order matching PNG rows.
            biHeight: -bmp.bmHeight,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };

    let screen_dc = OwnedScreenDc::acquire_screen()?;
    // SAFETY: `bgra` is allocated to exact `width * height * 4` 32bpp scanline byte length.
    let scanlines = unsafe {
        GetDIBits(
            screen_dc.as_raw(),
            hbitmap,
            0,
            height,
            Some(bgra.as_mut_ptr().cast()),
            &mut bmi,
            DIB_RGB_COLORS,
        )
    };

    if scanlines == 0 {
        return Err(AppError::Shell("GetDIBits failed to read icon pixels".to_owned()));
    }

    let rgba = bgra_premultiplied_to_straight_rgba(&bgra)?;
    Ok(Some((width, height, rgba)))
}

fn resolve_lnk_sync(lnk_path: &Path) -> AppResult<Option<ResolvedLnk>> {
    if !lnk_path.is_file() {
        return Ok(None);
    }

    // SAFETY: Instantiates standard ShellLink CLSID inside the initialized STA COM thread.
    let shell_link: IShellLinkW = unsafe {
        CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
    }
    .map_err(|err| AppError::Shell(format!("CoCreateInstance(ShellLink) failed: {err}")))?;

    let persist: IPersistFile = windows::core::Interface::cast(&shell_link)
        .map_err(|err| AppError::Shell(format!("QueryInterface(IPersistFile) failed: {err}")))?;

    let wide_lnk = to_wide_null(&lnk_path.to_string_lossy());
    // SAFETY: `wide_lnk` is a null-terminated UTF-16 path string alive across `Load`.
    if unsafe { persist.Load(PCWSTR(wide_lnk.as_ptr()), STGM_READ) }.is_err() {
        return Ok(None);
    }

    let mut target_buf = [0_u16; 1024];
    // SAFETY: `target_buf` provides a writable 1024-u16 buffer; optional WIN32_FIND_DATAW is null.
    if unsafe { shell_link.GetPath(&mut target_buf, std::ptr::null_mut(), 0) }.is_err() {
        return Ok(None);
    }
    let target_path = from_wide_null_terminated(&target_buf).trim().to_owned();
    if target_path.is_empty() {
        return Ok(None);
    }

    let mut args_buf = [0_u16; 1024];
    // SAFETY: `args_buf` is a writable stack slice sized for ShellLink argument output.
    let arguments = if unsafe { shell_link.GetArguments(&mut args_buf) }.is_ok() {
        from_wide_null_terminated(&args_buf).trim().to_owned()
    } else {
        String::new()
    };

    let mut desc_buf = [0_u16; 512];
    // SAFETY: `desc_buf` is a writable stack slice sized for ShellLink description output.
    let description = if unsafe { shell_link.GetDescription(&mut desc_buf) }.is_ok() {
        from_wide_null_terminated(&desc_buf).trim().to_owned()
    } else {
        String::new()
    };

    Ok(Some(ResolvedLnk {
        target_path,
        arguments,
        description,
    }))
}

fn launch_target_sync(target: &str, arguments: &str, run_as_admin: bool) -> AppResult<()> {
    let verb_wide = if run_as_admin {
        Some(to_wide_null("runas"))
    } else {
        None
    };
    let file_wide = to_wide_null(target);
    let args_trimmed = arguments.trim();
    let params_wide = if args_trimmed.is_empty() {
        None
    } else {
        Some(to_wide_null(args_trimmed))
    };

    let working_dir_wide = if !is_non_file_uri(target) {
        Path::new(target)
            .parent()
            .filter(|p| p.is_dir())
            .map(|p| to_wide_null(&p.to_string_lossy()))
    } else {
        None
    };

    let mut sei = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_FLAG_NO_UI,
        hwnd: HWND::default(),
        lpVerb: verb_wide
            .as_ref()
            .map_or_else(PCWSTR::null, |v| PCWSTR(v.as_ptr())),
        lpFile: PCWSTR(file_wide.as_ptr()),
        lpParameters: params_wide
            .as_ref()
            .map_or_else(PCWSTR::null, |p| PCWSTR(p.as_ptr())),
        lpDirectory: working_dir_wide
            .as_ref()
            .map_or_else(PCWSTR::null, |d| PCWSTR(d.as_ptr())),
        nShow: SW_SHOWNORMAL.0,
        hInstApp: HINSTANCE::default(),
        ..Default::default()
    };

    // SAFETY: `sei` fields point solely to UTF-16 buffers pinned in stack locals for the call duration.
    if let Err(err) = unsafe { ShellExecuteExW(&mut sei) } {
        if err.code() == HRESULT_ERROR_CANCELLED {
            return Err(AppError::LaunchCancelled);
        }
        return Err(AppError::LaunchFailed(format!("{target}: {err}")));
    }

    Ok(())
}

fn open_file_location_sync(target: &str) -> AppResult<()> {
    let path = strip_verbatim_prefix(Path::new(target));
    if path.is_file() {
        let canonical = strip_verbatim_prefix(
            &std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone()),
        );
        let select_arg = format!("/select,\"{}\"", canonical.to_string_lossy());
        return launch_target_sync("explorer.exe", &select_arg, false);
    }
    if path.is_dir() {
        return launch_target_sync(&path.to_string_lossy(), "", false);
    }
    if let Some(parent) = path.parent().filter(|p| p.is_dir()) {
        return launch_target_sync(&parent.to_string_lossy(), "", false);
    }
    Err(AppError::NotFound(format!(
        "File or parent folder does not exist: {target}"
    )))
}

/// Locks the Windows workstation via Win32 `LockWorkStation()`.
pub fn lock_workstation() -> AppResult<()> {
    // SAFETY: `LockWorkStation` takes no pointers or shared state.
    unsafe { LockWorkStation() }
        .map_err(|err| AppError::Shell(format!("LockWorkStation failed: {err}")))
}

/// Queries the current cursor screen coordinates in physical pixels via `GetCursorPos`.
pub fn get_cursor_pos() -> AppResult<(i32, i32)> {
    let mut pt = POINT::default();
    // SAFETY: `pt` is a properly initialized stack `POINT` passed by mutable pointer.
    unsafe { GetCursorPos(&mut pt) }
        .map_err(|err| AppError::Window(format!("GetCursorPos failed: {err}")))?;
    Ok((pt.x, pt.y))
}

/// Queries the work-area rectangle (`rcWork`) of the monitor nearest `(x, y)` via `GetMonitorInfoW`.
pub fn get_monitor_work_area_at(x: i32, y: i32) -> AppResult<Win32Rect> {
    // SAFETY: Pure query returning a monitor handle or fallback nearest monitor.
    let hmonitor = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `info.cbSize` equals `size_of::<MONITORINFO>()` and `info` is valid out-param memory.
    let ok = unsafe { GetMonitorInfoW(hmonitor, &mut info) };
    if !ok.as_bool() {
        return Err(AppError::Window("GetMonitorInfoW failed".to_owned()));
    }
    let RECT {
        left,
        top,
        right,
        bottom,
    } = info.rcWork;
    Ok(Win32Rect {
        left,
        top,
        right,
        bottom,
    })
}

/// Checks whether the logical primary mouse button is currently pressed (honoring `SM_SWAPBUTTON`).
#[must_use]
pub fn is_primary_mouse_button_down() -> bool {
    // SAFETY: Pure Win32 system metric / asynchronous key state queries with no pointers.
    let swapped = unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0;
    let vk = if swapped { VK_RBUTTON.0 } else { VK_LBUTTON.0 };
    // SAFETY: Queries bit state for a valid Win32 virtual key constant.
    let state = unsafe { GetAsyncKeyState(i32::from(vk)) };
    (state as u16 & 0x8000) != 0
}

/// Moves a window without resizing, activating, or blocking the calling thread.
pub fn move_window_no_resize_async(hwnd_raw: isize, x: i32, y: i32) -> AppResult<()> {
    let hwnd = HWND(hwnd_raw as *mut core::ffi::c_void);
    // SAFETY: `hwnd` refers to the app's own live `HWND` and flags avoid sizing/activation.
    unsafe {
        SetWindowPos(
            hwnd,
            None,
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
        )
    }
    .map_err(|err| AppError::Window(format!("SetWindowPos failed: {err}")))
}

/// Checks whether `hwnd_raw` is currently the active Win32 foreground window.
#[must_use]
pub fn is_window_foreground(hwnd_raw: isize) -> bool {
    // SAFETY: Thread-safe Win32 foreground handle read compared numerically against `hwnd_raw`.
    unsafe { GetForegroundWindow() }.0 as isize == hwnd_raw
}

/// Applies a combined two-rect window clipping region (`dock_rect` | `handle_rect`) to `hwnd_raw`.
/// Must be called on the UI/main thread.
pub fn set_window_region_union(
    hwnd_raw: isize,
    dock_rect: (i32, i32, i32, i32),
    handle_rect: (i32, i32, i32, i32),
) -> AppResult<()> {
    // SAFETY: Creates rectangular GDI regions immediately wrapped in RAII `OwnedHrgn` guards.
    let dock_rgn = OwnedHrgn::new(unsafe {
        CreateRectRgn(dock_rect.0, dock_rect.1, dock_rect.2, dock_rect.3)
    });
    if !dock_rgn.is_valid() {
        return Err(AppError::Window("CreateRectRgn(dock) returned null".to_owned()));
    }

    // SAFETY: Creates handle GDI region wrapped in `OwnedHrgn`.
    let handle_rgn = OwnedHrgn::new(unsafe {
        CreateRectRgn(handle_rect.0, handle_rect.1, handle_rect.2, handle_rect.3)
    });
    if !handle_rgn.is_valid() {
        return Err(AppError::Window("CreateRectRgn(handle) returned null".to_owned()));
    }

    // SAFETY: Creates destination GDI region wrapped in `OwnedHrgn`.
    let mut combined_rgn = OwnedHrgn::new(unsafe { CreateRectRgn(0, 0, 0, 0) });
    if !combined_rgn.is_valid() {
        return Err(AppError::Window(
            "CreateRectRgn(combined) returned null".to_owned(),
        ));
    }

    // SAFETY: All three HRGN handles were validated non-null immediately above.
    let combine_res = unsafe {
        CombineRgn(
            Some(combined_rgn.as_raw()),
            Some(dock_rgn.as_raw()),
            Some(handle_rgn.as_raw()),
            RGN_OR,
        )
    };
    if combine_res == RGN_ERROR {
        return Err(AppError::Window("CombineRgn(RGN_OR) failed".to_owned()));
    }

    let hwnd = HWND(hwnd_raw as *mut core::ffi::c_void);
    // SAFETY: Executed synchronously on the UI main thread with a valid `HWND` and valid `HRGN`.
    let set_res = unsafe { SetWindowRgn(hwnd, Some(combined_rgn.as_raw()), true) };
    if set_res == 0 {
        return Err(AppError::Window("SetWindowRgn(combined) failed".to_owned()));
    }

    // Windows owns the HRGN once SetWindowRgn succeeds; disarm RAII deletion.
    combined_rgn.disarm();
    Ok(())
}

/// Clears any active window clipping region so the collapsed `24x84` handle is unclipped.
/// Must be called on the UI/main thread.
pub fn clear_window_region(hwnd_raw: isize) -> AppResult<()> {
    let hwnd = HWND(hwnd_raw as *mut core::ffi::c_void);
    // SAFETY: Executed on the UI main thread; passing `None` clears the HWND region safely.
    let set_res = unsafe { SetWindowRgn(hwnd, None, true) };
    if set_res == 0 {
        tracing::debug!("SetWindowRgn(None) returned 0 (no active region or window closing)");
    }
    Ok(())
}

/// Persists a fatal startup error to the log directory (or temp fallback) and displays a
/// single native error dialog so release GUI builds (`windows_subsystem = "windows"`) do not
/// silently exit without diagnostics.
pub fn report_fatal_startup_error(err: &dyn std::fmt::Display) {
    let message = format!("Harbor failed to start:\n\n{err}");
    tracing::error!(error = %err, "Fatal startup error");

    let log_dir = AppPaths::resolve()
        .map(|p| p.logs_dir)
        .unwrap_or_else(|_| std::env::temp_dir().join("Harbor"));
    if std::fs::create_dir_all(&log_dir).is_ok() {
        let log_path = log_dir.join("harbor-startup.log");
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&log_path) {
            let _ = writeln!(file, "Fatal startup error: {err}");
        }
    }

    let title_wide = to_wide_null("Harbor Startup Error");
    let body_wide = to_wide_null(&message);
    // SAFETY: Both pointers reference null-terminated UTF-16 buffers valid for the call duration.
    let _ = unsafe {
        MessageBoxW(
            None,
            PCWSTR(body_wide.as_ptr()),
            PCWSTR(title_wide.as_ptr()),
            MB_OK | MB_ICONERROR,
        )
    };
}

/// Enables or disables starting Harbor with Windows via `HKCU\...\Run`.
pub fn set_start_with_windows(enabled: bool) -> AppResult<()> {
    let subkey_wide = to_wide_null(STARTUP_RUN_SUBKEY);
    let value_wide = to_wide_null(STARTUP_VALUE_NAME);

    let mut raw_hkey = HKEY::default();
    // SAFETY: `subkey_wide` is null-terminated UTF-16 and `raw_hkey` receives the opened HKEY.
    let create_err = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey_wide.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut raw_hkey,
            None,
        )
    };
    if create_err.0 != 0 {
        return Err(AppError::Shell(format!(
            "Failed to open startup Run registry key: Win32 error {}",
            create_err.0
        )));
    }
    let key = OwnedHkey(raw_hkey);

    if enabled {
        let exe_path = std::env::current_exe().map_err(|err| {
            AppError::Shell(format!("Failed to locate current executable: {err}"))
        })?;
        let quoted = format!("\"{}\"", exe_path.display());
        let wide_val = to_wide_null(&quoted);
        // SAFETY: `wide_val` is a contiguous `[u16]` slice (`align_of::<u8>() <= align_of::<u16>()`),
        // so viewing `wide_val.len() * 2` initialized bytes as `&[u8]` is sound.
        let bytes = unsafe {
            std::slice::from_raw_parts(wide_val.as_ptr().cast::<u8>(), wide_val.len() * 2)
        };
        // SAFETY: Registry handle `key` is valid for `KEY_SET_VALUE`, and `value_wide` & `bytes` are valid.
        let set_err = unsafe {
            RegSetValueExW(
                key.as_raw(),
                PCWSTR(value_wide.as_ptr()),
                None,
                REG_SZ,
                Some(bytes),
            )
        };
        if set_err.0 != 0 {
            return Err(AppError::Shell(format!(
                "Failed to write startup Run registry value: Win32 error {}",
                set_err.0
            )));
        }
    } else {
        // SAFETY: `key` is valid for `KEY_SET_VALUE` and `value_wide` is null-terminated UTF-16.
        let del_err = unsafe { RegDeleteValueW(key.as_raw(), PCWSTR(value_wide.as_ptr())) };
        if del_err.0 != 0 && del_err != ERROR_FILE_NOT_FOUND {
            return Err(AppError::Shell(format!(
                "Failed to remove startup Run registry value: Win32 error {}",
                del_err.0
            )));
        }
    }

    Ok(())
}

/// Reads Steam's installation directory from `HKCU\Software\Valve\Steam`.
#[must_use]
pub fn read_steam_install_path() -> Option<PathBuf> {
    let subkey = to_wide_null(STEAM_REG_SUBKEY);
    let mut raw_hkey = HKEY::default();
    // SAFETY: `subkey` is null-terminated UTF-16 and `raw_hkey` receives the opened read-only HKEY.
    let open_err = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            None,
            KEY_READ,
            &mut raw_hkey,
        )
    };
    if open_err.0 != 0 {
        return None;
    }
    let key = OwnedHkey(raw_hkey);

    if let Some(path_str) = read_registry_sz(key.as_raw(), "SteamPath") {
        let p = PathBuf::from(path_str);
        if p.is_dir() {
            return Some(p);
        }
    }
    if let Some(exe_str) = read_registry_sz(key.as_raw(), "SteamExe") {
        if let Some(parent) = Path::new(&exe_str).parent() {
            if parent.is_dir() {
                return Some(parent.to_path_buf());
            }
        }
    }
    None
}

fn read_registry_sz(hkey: HKEY, value_name: &str) -> Option<String> {
    let name_wide = to_wide_null(value_name);
    let mut reg_type = REG_VALUE_TYPE::default();
    let mut byte_len = 0_u32;
    // SAFETY: First call passes `None` for the data buffer to query required byte length only.
    let query_size = unsafe {
        RegQueryValueExW(
            hkey,
            PCWSTR(name_wide.as_ptr()),
            None,
            Some(&mut reg_type),
            None,
            Some(&mut byte_len),
        )
    };
    if query_size.0 != 0 || byte_len < 2 || reg_type != REG_SZ {
        return None;
    }

    let mut buf = vec![0_u8; byte_len as usize];
    // SAFETY: `buf` holds at least `byte_len` writable bytes for `RegQueryValueExW`.
    let query_data = unsafe {
        RegQueryValueExW(
            hkey,
            PCWSTR(name_wide.as_ptr()),
            None,
            Some(&mut reg_type),
            Some(buf.as_mut_ptr()),
            Some(&mut byte_len),
        )
    };
    if query_data.0 != 0 {
        return None;
    }

    let u16_slice: Vec<u16> = buf[..(byte_len as usize).min(buf.len())]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    let decoded = from_wide_null_terminated(&u16_slice);
    let trimmed = decoded.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

/// Resolves the User and Common Start Menu Programs folders via `SHGetKnownFolderPath`.
#[must_use]
pub fn resolve_start_menu_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for folder_id in [&FOLDERID_Programs, &FOLDERID_CommonPrograms] {
        // SAFETY: Standard known-folder GUID reference; returned PWSTR is freed by `CoTaskMemPwstr`.
        if let Ok(raw_pwstr) = unsafe { SHGetKnownFolderPath(folder_id, KF_FLAG_DEFAULT, None) } {
            let guard = CoTaskMemPwstr(raw_pwstr);
            if !guard.0.is_null() {
                // SAFETY: `guard.0` is non-null COM-allocated null-terminated UTF-16 from `SHGetKnownFolderPath`.
                if let Ok(path_str) = unsafe { guard.0.to_string() } {
                    let candidate = PathBuf::from(path_str);
                    if candidate.is_dir() && !dirs.contains(&candidate) {
                        dirs.push(candidate);
                    }
                }
            }
        }
    }

    if dirs.is_empty() {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            let user_sm = PathBuf::from(appdata).join("Microsoft\\Windows\\Start Menu\\Programs");
            if user_sm.is_dir() {
                dirs.push(user_sm);
            }
        }
        if let Some(program_data) = std::env::var_os("ProgramData") {
            let common_sm =
                PathBuf::from(program_data).join("Microsoft\\Windows\\Start Menu\\Programs");
            if common_sm.is_dir() && !dirs.contains(&common_sm) {
                dirs.push(common_sm);
            }
        }
    }

    dirs
}

/// Computes a compact lowercase hex representation of `bytes`.
#[must_use]
pub fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

fn to_wide_null(s: &str) -> Vec<u16> {
    OsString::from(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn from_wide_null_terminated(slice: &[u16]) -> String {
    let len = slice.iter().position(|&c| c == 0).unwrap_or(slice.len());
    OsString::from_wide(&slice[..len])
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_non_file_uris_without_touching_windows_paths() {
        assert!(is_non_file_uri("steam://rungameid/730"));
        assert!(is_non_file_uri("ms-settings:windowsupdate"));
        assert!(is_non_file_uri(
            "shell:AppsFolder\\Microsoft.WindowsTerminal_8wekyb3d8bbwe!App"
        ));
        assert!(is_non_file_uri("https://example.com"));

        assert!(!is_non_file_uri("C:\\Windows\\System32\\notepad.exe"));
        assert!(!is_non_file_uri("D:/Games/Hades/Hades.exe"));
        assert!(!is_non_file_uri("\\\\server\\share\\app.exe"));
        assert!(!is_non_file_uri("\"C:\\Program Files\\App\\app.exe\""));
    }

    #[test]
    fn converts_premultiplied_bgra_to_straight_rgba() -> AppResult<()> {
        // [B, G, R, A]:
        // 1. Transparent pixel [0, 0, 0, 0] -> [0, 0, 0, 0]
        // 2. Opaque pixel [50, 100, 200, 255] -> [200, 100, 50, 255]
        // 3. 50% alpha premultiplied [25, 50, 100, 128] -> straight [199, 99, 49, 128]
        // 4. Clamped overshoot [255, 100, 200, 128] -> straight [255, 199, 255, 128]
        let input = vec![
            0, 0, 0, 0, //
            50, 100, 200, 255, //
            25, 50, 100, 128, //
            255, 100, 200, 128, //
        ];

        let rgba = bgra_premultiplied_to_straight_rgba(&input)?;
        assert_eq!(
            rgba,
            vec![
                0, 0, 0, 0, //
                200, 100, 50, 255, //
                199, 99, 49, 128, //
                255, 199, 255, 128, //
            ]
        );
        Ok(())
    }

    #[test]
    fn treats_all_zero_alpha_with_nonzero_rgb_as_opaque() -> AppResult<()> {
        let input = vec![
            10, 20, 30, 0, //
            0, 0, 0, 0, //
            255, 128, 64, 0, //
        ];
        let rgba = bgra_premultiplied_to_straight_rgba(&input)?;
        assert_eq!(
            rgba,
            vec![
                30, 20, 10, 255, //
                0, 0, 0, 255, //
                64, 128, 255, 255, //
            ]
        );

        // Completely empty transparent buffer must stay transparent.
        let empty = vec![0, 0, 0, 0, 0, 0, 0, 0];
        let empty_rgba = bgra_premultiplied_to_straight_rgba(&empty)?;
        assert_eq!(empty_rgba, empty);
        Ok(())
    }

    #[test]
    fn strips_windows_verbatim_prefixes() {
        assert_eq!(
            strip_verbatim_prefix(Path::new(r"\\?\C:\Windows\System32\notepad.exe")),
            PathBuf::from(r"C:\Windows\System32\notepad.exe")
        );
        assert_eq!(
            strip_verbatim_prefix(Path::new(r"\\?\UNC\server\share\app.exe")),
            PathBuf::from(r"\\server\share\app.exe")
        );
        assert_eq!(
            strip_verbatim_prefix(Path::new(r"E:\Games\Hades\Hades.exe")),
            PathBuf::from(r"E:\Games\Hades\Hades.exe")
        );
    }

    #[tokio::test]
    async fn extracts_real_windows_icon_to_png_cache() -> AppResult<()> {
        let notepad = Path::new(r"C:\Windows\System32\notepad.exe");
        if !notepad.is_file() {
            return Ok(());
        }
        let seq = CACHE_TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temp_cache = std::env::temp_dir().join(format!(
            "harbor_icon_test_{}_{}",
            std::process::id(),
            seq
        ));
        let _ = std::fs::remove_dir_all(&temp_cache);

        let shell = WinShell::new()?;
        let extracted = shell
            .extract_icon_cached(r"C:\Windows\System32\notepad.exe", &temp_cache)
            .await?;

        let Some(png_path) = extracted else {
            panic!("Expected extracted PNG icon for notepad.exe");
        };
        assert!(!png_path.starts_with(r"\\?\"));
        let png_file = Path::new(&png_path);
        assert!(png_file.is_file());
        let bytes = std::fs::read(png_file)?;
        assert!(bytes.starts_with(&[0x89, b'P', b'N', b'G']));

        let _ = std::fs::remove_dir_all(&temp_cache);
        Ok(())
    }
}
