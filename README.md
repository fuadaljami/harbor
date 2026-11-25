# Harbor

Harbor is an ultra-lightweight Windows edge-docking program and game launcher built with **Rust + Tauri v2** on the backend and **React + TypeScript + Tailwind CSS v3 + Lucide Icons** on the frontend.

It docks flush to the left or right edge of the active monitor work area as a compact `24×84` handle, slides out to a unified `380px` launcher panel, auto-discovers Steam games and Start Menu programs, extracts native `.exe`/`.lnk` icons locally, and supports full keyboard navigation and Windows Explorer drag-and-drop pinning.

---

## Architecture & Directory Tree

```text
harbor/
├── index.html                    # Transparent root HTML shell
├── package.json                  # Frontend scripts & dependencies
├── tailwind.config.ts            # Discord Dark color tokens & Inter font family
├── PROJECT_RULES.md              # Architecture & engineering constraints
├── src/
│   ├── App.tsx                   # Unified dock shell, drag/drop & window layout sync
│   ├── index.css                 # Bundled @fontsource/inter & slim dark scrollbar
│   ├── types/dock.ts             # IPC contracts mirrored from Rust serde structs
│   ├── lib/
│   │   ├── dockBounds.ts         # Horizontal dock bounds context for portals
│   │   ├── ipc.ts                # Single typed Tauri IPC boundary (+ dev mock loader)
│   │   ├── keyboard.ts           # Pure search ranking, category cycling & traversal logic
│   │   ├── keyboard.test.ts      # Vitest unit tests for keyboard & search behavior
│   │   └── mockIpc.ts            # Dev-only browser preview adapter (excluded from release)
│   ├── hooks/
│   │   ├── useIconStore.ts       # Concurrent signature-keyed icon extraction store
│   │   └── useKeyboardNav.ts     # Global keyboard traversal & Esc priority stack
│   └── components/
│       ├── AddEditModal.tsx      # Add / Edit shortcut dialog
│       ├── BottomControlBar.tsx  # Live clock & Task Manager / Terminal / Lock PC / Settings
│       ├── CategoryRail.tsx      # 56px morphing squircle category rail
│       ├── ContextMenu.tsx       # Clamped portaled right-click context menu
│       ├── DockHeader.tsx        # Category title, pin dock, rescan, edge toggle, collapse
│       ├── EdgeHandle.tsx        # 24×84 edge pill with vertical pointer-capture dragging
│       ├── SearchBar.tsx         # Instant filter input (Ctrl+K / /)
│       ├── SettingsModal.tsx     # Edge, monitor, hotkey recorder, behavior & missing review
│       ├── ShortcutCard.tsx      # 46px shortcut card with native icon or monogram badge
│       ├── ShortcutList.tsx      # Collapsible category sections & @dnd-kit reordering
│       ├── ToastContainer.tsx    # Non-blocking in-dock toast notifications
│       └── Tooltip.tsx           # Clamped portaled tooltips
└── src-tauri/
    ├── Cargo.toml                # Rust crate config & optimized [profile.release]
    ├── tauri.conf.json           # Tauri v2 window, CSP, asset protocol & NSIS config
    ├── capabilities/default.json # Tauri v2 ACL capabilities
    ├── nsis/hooks.nsh            # NSIS post-uninstall Run registry cleanup hook
    ├── icons/                    # App icons (32px, 128px, icon.ico, icon.icns)
    └── src/
        ├── main.rs               # Windows subsystem entry & fatal startup error reporting
        ├── lib.rs                # Tauri builder, tray menu, hotkey & portable WebView2 setup
        ├── commands.rs           # Typed Tauri IPC command handlers
        ├── config.rs             # Portable/Installed AppPaths & atomic JSON persistence
        ├── error.rs              # Typed thiserror AppError serialized to { kind, message }
        ├── logging.rs            # Rolling file logger setup via tracing-appender
        ├── models.rs             # DockConfig, DockItem, dismissal keys & icon priority
        ├── scanner.rs            # Steam ACF/VDF discovery & Start Menu .lnk resolution
        ├── win_shell.rs          # Dual COM STA workers, icon extraction, ShellExecuteExW & GDI
        └── window_manager.rs     # Physical DPI math, epoch collapse & edge handle drag
```

---

## Development & Build Commands

Run commands on **native Windows (`x86_64-pc-windows-msvc`)** from the repository root using `npm.cmd` / `npx.cmd` in PowerShell:

- **Full Tauri Desktop Dev Mode (Rust backend + Vite WebView2):**
  ```powershell
  npm.cmd run tauri dev
  ```
- **Browser-Only UI Preview (uses dynamic `mockIpc.ts` adapter only when Tauri runtime is absent in dev):**
  ```powershell
  npm.cmd run dev
  ```
- **Quality Checks & Unit Tests:**
  ```powershell
  npx.cmd tsc --noEmit
  npm.cmd run lint
  npm.cmd run test
  npm.cmd run build
  ```
  And in `src-tauri/`:
  ```powershell
  cargo test --offline
  cargo clippy --offline --all-targets -- -D warnings -D clippy::unwrap_used -D clippy::expect_used
  ```
- **Production Release Build & NSIS Installer:**
  ```powershell
  npm.cmd run tauri build
  ```
  Outputs:
  - Standalone executable: `src-tauri/target/release/harbor.exe`
  - Per-user NSIS installer: `src-tauri/target/release/bundle/nsis/Harbor_0.1.0_x64-setup.exe`

---

## Installed Mode vs. Portable Mode

Harbor checks for a marker file named `portable.flag` in the executable directory once at startup (without probing directory write permissions):

| Mode | Trigger | Config, Logs & Icon Cache | WebView2 User Data |
| :--- | :--- | :--- | :--- |
| **Installed (Default)** | No `portable.flag` beside `harbor.exe` | `%LOCALAPPDATA%\Harbor\` (`config.json`, `logs/`, `icon_cache/`) | Default Tauri `%LOCALAPPDATA%\com.harbor.dock\EBWebView\` |
| **Portable** | Empty `portable.flag` beside `harbor.exe` | `<exe_dir>\data\` (`config.json`, `logs/`, `icon_cache/`) | `<exe_dir>\data\webview2\` (set via `WebviewWindowBuilder::data_directory`) |

### Verifying Portable Mode

1. Copy `harbor.exe` into a standalone folder (for example, `D:\Tools\Harbor\`).
2. Create an empty file named `portable.flag` beside `harbor.exe`.
3. Launch `harbor.exe` and confirm `D:\Tools\Harbor\data\` is populated with:
   - `config.json`
   - `logs\`
   - `icon_cache\`
   - `webview2\`
4. Confirm nothing is written into `%LOCALAPPDATA%\Harbor\` or `%LOCALAPPDATA%\com.harbor.dock\`.

---

## Keyboard Shortcuts

- `Alt+Shift+Space` (configurable in Settings): Global expand / collapse toggle
- `Ctrl+K` or `/` (outside text inputs): Focus search box
- `ArrowUp` / `ArrowDown`: Move selection highlight with wrap-around
- `Enter`: Launch selected shortcut (or top search hit)
- `Ctrl+Enter`: Launch selected shortcut elevated ("Run as Administrator")
- `Tab` / `Shift+Tab`: Cycle categories (`All -> Games -> Apps -> Tools`)
- `Ctrl+1` .. `Ctrl+4`: Jump directly to `All`, `Games`, `Apps`, or `Tools`
- `Esc`: Strict priority stack — cancels hotkey recording → closes context menu → closes modal → clears non-empty search → collapses dock

---

## Known Platform Notes

- **SmartScreen / Unsigned Binary:** Both `harbor.exe` and `Harbor_0.1.0_x64-setup.exe` are unsigned builds. Windows Defender SmartScreen may display a prompt on first launch until code-signed with a trusted certificate.
- **Exclusive Fullscreen Games:** In Windows, true exclusive-fullscreen DirectX/Vulkan titles bypass DWM composition and supersede `WS_EX_TOPMOST` / always-on-top windows, so Harbor's edge handle will not appear above exclusive-fullscreen games. Switch games to **Borderless Windowed** mode if you want instant edge-handle access during gameplay (the global hotkey also brings Harbor forward on demand).
- **Uninstall & Startup Cleanup:**
  - The NSIS installer builds with `installMode: "currentUser"` and attaches `src-tauri/nsis/hooks.nsh` (`NSIS_HOOK_POSTUNINSTALL`), which removes `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\Harbor` automatically during uninstall.
  - Personal settings, logs, and icon cache under `%LOCALAPPDATA%\Harbor\` (and `%LOCALAPPDATA%\com.harbor.dock\`) are preserved across uninstalls/upgrades; delete those folders manually if you wish to remove all local user data.
