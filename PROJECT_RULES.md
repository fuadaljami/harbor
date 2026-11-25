# Harbor — Project Rules & Architecture Constraints

> Persistent reference capturing Project Environment, Non-Negotiable Engineering Rules, Phase Amendments, Visual Design System & Tokens, and Pinned Tech Stack so constraints never drift across phases.

- **Project Root:** `C:\Users\Fuad\.gemini\antigravity\scratch\harbor`
- **Windows Node Commands:** Always use `npm.cmd` and `npx.cmd` in PowerShell.

---

## 1. Non-Negotiable Engineering Rules

1. **Zero `.unwrap()` / `.expect()` in production Rust:**
   - Add `#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]` in `src-tauri/src/lib.rs` and `src-tauri/src/main.rs`.
   - Use `thiserror` for a typed `AppError` enum serialized as `{ kind, message }` to frontend toasts and written to disk logs. Include `AppError::LaunchCancelled`, which the frontend displays as a quiet info toast rather than an error.
   - Use `parking_lot::RwLock` (non-poisoning) instead of `std::sync::RwLock`.
   - Have `main()` handle startup errors cleanly (log + exit non-zero) rather than `.expect()`. Tests (`#[cfg(test)]`) are exempt.

2. **Dedicated COM STA Worker Threads & Launching:**
   - **Icon/Shell STA Worker:** Spawn one dedicated OS thread calling `CoInitializeEx(None, COINIT_APARTMENTTHREADED)` guarded by an RAII `ComGuard` (`Drop` calls `CoUninitialize`). Receive requests via a bounded `tokio::sync::mpsc::channel` (async callers use `.send().await`; worker loops on `blocking_recv()`) and reply over `tokio::sync::oneshot`.
   - **Launch STA Worker:** Spawn a second dedicated OS thread (`COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE`, same `ComGuard`) so a pending UAC prompt can never block icon extraction or `.lnk` resolution.
   - Launch via `ShellExecuteExW` (`SHELLEXECUTEINFOW`) with `SEE_MASK_FLAG_NO_UI` so Windows never pops up native error dialogs. Map `ERROR_CANCELLED` (`1223`) to `AppError::LaunchCancelled`.
   - Wrap icon requests in a caller-side `tokio::time::timeout` (~3s) that falls back to the monogram. Skip icon extraction on non-local or unavailable drives (`GetDriveTypeW`) so an unreachable network path cannot stall the worker.

3. **Zero Unmanaged Handle Leaks:**
   - Wrap Win32 `HBITMAP`, `HICON`, and `HDC` resources in RAII structs with `Drop` calling `DeleteObject` / `DestroyIcon` / `ReleaseDC`.

4. **Drag-and-Drop Compatibility (Windows WebView2):**
   - Keep `dragDropEnabled: true` in `tauri.conf.json` for native Windows Explorer file drops (`onDragDropEvent`).
   - Because native drop handling disables HTML5 drag-and-drop inside WebView2, implement internal shortcut reordering strictly with **`@dnd-kit/core` + `@dnd-kit/sortable` using `PointerSensor`** (zero HTML5 `draggable` attributes).

5. **Explicit Portable vs. Installed Storage:**
   - Do NOT test whether the executable directory is writable (NSIS per-user installs are writable).
   - Check for an explicit marker file named `portable.flag` in the executable directory once at startup:
     - **Portable mode (`portable.flag` exists):** Store `config.json`, `logs/`, and `icon_cache/` inside `<exe_dir>\data\`.
     - **Installed mode (default):** Store all three inside `%LOCALAPPDATA%\Harbor\`.

6. **Native Dialogs & Blur Suspension:**
   - Open all file/icon pickers from Rust commands (blocking dialog calls inside `spawn_blocking`), never from a JS dialog plugin. `src/lib/ipc.ts` only invokes those Rust commands.
   - Maintain an `AtomicUsize` `dialog_open_count` in Rust state, incremented before opening a dialog and decremented by an RAII guard. Ignore blur-triggered auto-collapse while `dialog_open_count > 0`.

7. **Window Geometry, DPI & Flicker-Free Resize / Handle Dragging:**
   - Window config: `decorations: false`, `transparent: true`, `shadow: false`, `resizable: false`, `skipTaskbar: true`, `alwaysOnTop: true`, `visible: false`.
   - Compute window position and size in **physical pixels** from the active monitor's `Monitor::work_area()` (fallback Win32 `GetMonitorInfoW` `rcWork`) and `scale_factor`, rounding physical values consistently. Keep CSS in logical pixels. When moving across monitors with different scale factors: set position first, set size second, read back `outer_size()`, and correct once if needed. Re-resolve monitor and re-clamp `top_position` and `dock_h` before every expand, toggle, hotkey, tray action, and on `WindowEvent::ScaleFactorChanged`.
   - **Collapsed window:** `24×84` logical px anchored flush to `edge` at `top_position`. Log the collapsed `outer_size()` and `outer_position()` to verify the click-blocking guarantee.
   - **Expanded window:** `380 × dock_h` logical px flush to `edge`, where `dock_h = min(640.0, work_area_logical_h - 24.0)`. Zero transparent overhang strip (`SetWindowRgn` is never used on the frameless transparent HWND so DWM composition remains intact with zero `WM_NCPAINT` flash).
   - **`top_position`:** handle's top Y relative to `work_area.top` in logical px, clamped to `[8.0, work_area_logical_h - 84.0 - 8.0]`.
   - **Epoch-Safe Expand / Collapse Sequencing:**
     - Track window phase (`Collapsed | Expanded | Collapsing`) and an `AtomicU64` epoch counter.
     - **Expand sequence:** Compute `desired_dock_top = top_position + 42.0 - (dock_h / 2.0)`, clamped to `expanded_top` in `[8.0, work_area_logical_h - dock_h - 8.0]`, and `handle_offset_y = top_position - expanded_top`. Bump epoch, set phase `Expanded`, resize & position native window to `380 × dock_h` FIRST, activate/focus, record expand timestamp, and return/emit `WindowLayoutState { expanded, edge, dock_height, handle_offset_y, top_position, pinned }` so the frontend plays the `160ms` CSS slide-in.
     - **Collapse sequence:** Every collapse path routes through `request_collapse_dock` (idempotent if already `Collapsed` or `Collapsing`), which bumps the epoch, sets phase `Collapsing`, emits `harbor://request-collapse` (with the epoch), and spawns a `~300ms` fallback timer. Both `commit_collapse_window(epoch)` and the fallback timer are idempotent and only shrink to `24×84` flush against the active edge if the epoch and `Collapsing` phase still match. Log collapsed `outer_size()`. Users collapse while expanded via `DockHeader` (`<` / `>` button), outside-blur, `Esc`, tray, or `Alt+Shift+Space`.
   - **Foreground Focus Guarantee:** On hotkey/tray/second-instance/dock expand, verify via Win32 `GetForegroundWindow`; if not foreground, retry focus once after `~50ms` (while still expanded & matching epoch) and log a warning if Windows refuses—never fail expand.
   - **Edge handle vertical drag:** Allowed only while `Collapsed`. Frontend captures pointer (`setPointerCapture`) and calls `start_handle_drag`, recording grab offset (`cursor_y - window_y`) so the handle never jumps under the cursor. A background thread polls `GetCursorPos` (~8ms), clamps Y within the active monitor's physical work area (X locked flush to `edge`), moves via `SetWindowPos(SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS)` without synchronously `.join()`ing on the UI thread, and checks `GetAsyncKeyState` (`SM_SWAPBUTTON` -> `VK_RBUTTON` / `VK_LBUTTON`) as a missed-pointer-up safety net. On release, persist `top_position` if moved `>= 5.0` logical px (`< 5px` vertical delta toggles expand/collapse).
   - **Blur guardrails:** Ignore `WindowEvent::Focused(false)` when not `Expanded`, when pinned, when `!config.auto_collapse_on_blur`, when `dialog_open_count > 0`, while dragging, or within a `~250ms` post-expand grace window.
   - **Plugins, Tray, Hotkey & Drops:**
     - Register `tauri-plugin-single-instance` FIRST; re-launch expands & focuses existing instance.
     - Global shortcut acts ONLY on `ShortcutState::Pressed`; rebind rolls back on failure; startup collision logs warning, stores `hotkey_warning`, and emits `harbor://hotkey-warning`.
     - Tray (`TrayIconBuilder`, `default_window_icon()`, tooltip `"Harbor"`, `show_menu_on_left_click(false)`) toggles only on `MouseButton::Left` + `MouseButtonState::Up`; menu items `"Expand / Collapse"`, dynamic `"Switch to Right Edge"` / `"Switch to Left Edge"`, `"Quit Harbor"`.
     - Guard `rescan_shortcuts` with an `AtomicBool`; serialize disk writes with a dedicated save mutex.
     - Handle `WindowEvent::DragDrop` in Rust, emitting `harbor://file-drop-hover`, `harbor://file-drop-leave`, and `harbor://file-drop`.

8. **Asset Protocol & Security:**
   - `tauri.conf.json`: Keep `assetProtocol.enable = true` without a static `scope` array. CSP `img-src` includes `'self' asset: http://asset.localhost https://asset.localhost data:`.
   - Allow the resolved `icon_cache_dir` dynamically at startup via `app.asset_protocol_scope().allow_directory(&icon_cache_dir, true)` (handling `Result` cleanly, no unwrap) before showing the window.
   - Strip Windows `\\?\` verbatim prefixes before passing paths to Win32 Shell APIs (`SHCreateItemFromParsingName`, `SHGetFileInfoW`, `ShellExecuteExW`, `GetDriveTypeW`) and log icon extraction failures at `warn` level.

9. **Data Model, Steam, Dismissals & Categorization (`models.rs`, `scanner.rs`):**
   - `DockItem`: Include `icon_source_path: Option<String>` (auto-discovered `.exe` used exclusively for icons). Icon resolution priority: `custom_icon_path` -> `icon_source_path` -> `path`.
   - Steam items (`source == Steam`): Set `path = "steam://rungameid/<steam_app_id>"` and store the discovered game binary in `icon_source_path`. "Open File Location" opens the folder containing `icon_source_path`. Edit modal renders `path` as read-only for Steam items.
   - `DockConfig`: Include `dismissed: Vec<String>`. Dismissal keys are `"steam:<app_id>"` for Steam items, or the lowercased canonical target/path for others.
     - Removing an item records its key in `dismissed`.
     - Rescan never re-adds dismissed keys.
     - Manually adding a shortcut removes its key from `dismissed`.
     - "Reset to Discovered Defaults" clears `dismissed`.
   - Add `#[serde(default)]` to every field added after schema v1 so older `config.json` files load cleanly without triggering corruption recovery.
   - Start Menu auto-categorization: Default to `Apps`. Assign `Tools` for curated system utilities (`terminal`, `powershell`, `cmd`, `task manager`, `regedit`, `control panel`, `device manager`, `resource monitor`, `disk cleanup`, `services`, `system information`, `administrative`, `diagnostic`, `monitor`, `cleanup`, `settings`, `calculator`, `snipping tool`). Assign `Games` for Steam items or known launcher/gaming utilities (`steam`, `epic games`, `riot`, `battle.net`, `gog`, `ea app`, `ubisoft`, `rockstar`, `xbox`, `game bar`, `afterburner`, `rivatuner`, `mod manager`). Match keywords as whole words/tokens, case-insensitive—never raw substrings.

10. **Anti-Slop Visual Rules:**
    - NO neon cyberpunk gradients, glassmorphism blur hacks, or glowing rainbow borders.
    - NO raw Unicode emojis anywhere in the UI. Use `lucide-react` SVGs (`strokeWidth={1.75}`) exclusively.
    - Bundle `Inter` locally via `@fontsource/inter` (zero external font CDNs).
    - Style custom slim dark scrollbars (`6px` thumb, transparent track).
    - NO `window.alert()` / `MessageBox` popups for routine feedback. Use non-blocking in-dock toasts.

11. **Strict TypeScript + Browser Preview Adapter:**
    - Zero `any` (enforced via ESLint `@typescript-eslint/no-explicit-any: "error"`).
    - Gate the browser mock adapter strictly on `import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)`, loading `src/lib/mockIpc.ts` via dynamic `import()`.
    - Route ALL Tauri API calls (`invoke`, `listen`, `convertFileSrc`, drag-drop events) exclusively through `src/lib/ipc.ts`, enforced by ESLint `no-restricted-imports` banning `@tauri-apps/*` elsewhere.
    - Tauri maps `camelCase` JS command argument keys to `snake_case` Rust params while struct payload fields keep their exact `serde` names.

12. **Verification Discipline:**
    - Run `cargo clippy --all-targets -- -D warnings -D clippy::unwrap_used -D clippy::expect_used` in Phases 2a, 2b, 3, 5, and 6.
    - Declare Phase 2 modules as `pub mod` in `lib.rs` so public APIs do not trigger `dead_code` before Phase 3 wires `commands.rs`.
    - By the end of Phase 2b, every unused-dependency warning must be resolved.

---

## 2. Visual Design System & Design Tokens (Discord-Inspired Architecture)

Compact, dense, legible dark interface sized at `380×640` logical pixels when expanded (clamped dynamically to `monitor.work_area.height - 24` on smaller displays or high DPI), and `24×84` logical pixels when collapsed.

### Color Tokens
- **Category Rail (Deepest):** `#1E1F22`
- **Main Panel:** `#2B2D31`
- **Bottom Control Strip:** `#232428`
- **Tooltips & Context Menus:** `#111214`
- **Modals:** `#313338`
- **Hover / Keyboard Focus:** `#35373C` / `#404249`
- **Hairline Borders:** `#1F2023` / `rgba(255, 255, 255, 0.06)`
- **Primary Accent (Blurple):** `#5865F2` (hover `#4752C4`)
- **Semantics:** Online/Success `#23A55A`, Pin/Warning `#F0B232`, Danger `#DA373C`
- **Typography:** Primary `#F2F3F5`, Secondary `#B5BAC1`, Muted `#949BA4`; Font: `'Inter', 'Segoe UI Variable', 'Segoe UI', sans-serif`

### Layout & Edge Mirroring
- **Docked Left (default):** `[56px Category Rail] | [324px Main Panel + Bottom Control Strip]` flush to left screen edge when expanded; `24×84` Edge Handle flush to left screen edge when collapsed.
- **Docked Right (mirrored):** Layout mirrors cleanly (`flex-row-reverse`): `[324px Main Panel + Bottom Control Strip] | [56px Category Rail]` flush against the right screen edge when expanded, active/hover pill indicators flip to the right side of each rail icon, rail tooltips open to the left, and the collapsed edge handle anchors flush to the right edge with its chevron mirrored.

### Component Anatomy
1. **Category Rail (`56px`, `#1E1F22`):**
   - `40×40` circular icons morphing to `12px` squircles on hover/active (`150ms cubic-bezier(0.2, 0, 0, 1)`): **All**, **Games**, **Apps**, **Tools**, a `32×2` divider (`#35363C`), and a green **`+` Add Shortcut** button (`#23A55A` on hover).
   - Signature `4px` white edge pill indicator (`0px` rest, `20px` hover, `36px` active) + `#111214` hover tooltip showing label and item count.
2. **Header (`48px`) & Search (`34px`):**
   - Header displays active category name, count badge, and compact icon buttons: Pin Dock, Rescan System, Switch Left/Right Edge, Collapse.
   - Search input (`#1E1F22`, `6px` radius) with `Search` icon, `"Find or launch a program..."` placeholder, `Ctrl+K` hint pill, and clear button.
3. **Shortcut List (`flex-1`):**
   - Pinned items (`pinned: true`) sort to the top of their section/category with a subtle amber pin icon.
   - In `All` view, group under collapsible uppercase section headers (`11px`, `tracking-wider`, `#949BA4`).
   - Each card (`46px` height, `6px` radius): `32×32` icon inside a `#1E1F22` squircle well, primary name (`13.5px`), muted description (`11.5px`), subtle `GAME` / `ADMIN` pill badges (driven by `category === "Games"` and `run_as_admin`), hover-revealed drag grip (`@dnd-kit`) and more-menu trigger.
   - Clean Blurple drop-zone overlay when dragging files from Windows Explorer.
4. **Bottom System Control Strip (`52px`, `#232428`):**
   - Left: `#23A55A` status dot + live clock (`HH:MM`) and date (`ddd, MMM D`)—timer pauses while collapsed.
   - Right: Task Manager (`taskmgr.exe`), Terminal (`wt.exe` with `cmd.exe` fallback), Lock PC (Win32 `LockWorkStation`), and Settings gear.
5. **Collapsed Edge Handle (`24×84`):**
   - `#1E1F22` vertical pill with `1px` `#3F4147` border and `#5865F2` chevron icon.

---

## 3. Pinned Tech Stack & Dependencies

- **Target:** Native Windows (`x86_64-pc-windows-msvc`)
- **Tauri:** `2.x` stable (`tauri` with `tray-icon` + `protocol-asset`, `tauri-build`, `@tauri-apps/api`, `@tauri-apps/cli`, `tauri-plugin-single-instance`, `tauri-plugin-global-shortcut`, `tauri-plugin-dialog` — plugins registered and invoked from Rust only)
- **Rust Crates:** `windows = "0.62"` (aligned to `Cargo.lock` Tauri ecosystem version, explicit minimal Win32 features), `parking_lot = "0.12"`, `thiserror = "2"`, `serde` + `serde_json`, `tokio`, `tracing` + `tracing-subscriber` + `tracing-appender`, `image` (PNG encoding only, `default-features = false, features = ["png"]`), `sha2` (cache keys)
- **Frontend:** Vite + React 18 + TypeScript (strict) + **Tailwind CSS `^3.4`** (classic `tailwind.config.ts`, NOT v4) + `@dnd-kit/core` + `@dnd-kit/sortable` + `@dnd-kit/utilities` + `lucide-react` + `@fontsource/inter` + ESLint (`@typescript-eslint`)
