# ADR 0023 — Lulo on Windows: apps first, then a Mac-style shell that runs alongside Explorer

- **Status:** proposed 2026-10-06. Phase 1 (the app seam and a non-blocking CI job) is on
  branch `op/windows-plan`; phase 2's first slice (the gaps that blocked using the three
  apps) is on `op/win-phase2a`; the second slice (Preview, Clock, Weather, Terminal build
  and open) is on `op/win-phase2b`; the third slice (those four apps' own shortcuts move to
  `rmac_ui::bind_keys`, single instance for Calculator/Clock/Weather/Preview, real PDF
  rendering, and the CI proof for all of it) is on `op/win-phase2c`. A follow-up pass using
  the reference laptop's first numbers (idle CPU and launch time measured in CI, a real
  foreground fix, Clock's alarms scheduled through Task Scheduler, a cross-platform
  format-bar clipping fix) is on `op/win-polish`. Files is on `op/win-phase2d`;
  System Settings and a cheap Clock minute tick are on `op/win-settings`. Phase 3's design and first slice (the
  Lulo layer: menu bar, Dock and Spotlight over the Windows desktop) are on
  `op/win-shell`. A real per-user installer -- a WiX MSI, an AppUserModelID for every
  app, file associations offered but not forced, version info resources, Files and the
  Lulo layer included alongside the seven apps, and an uninstaller that also clears
  Clock's scheduled tasks and restores the Windows desktop -- is on `op/win-installer`
  (see "Installer" below). The rest of phases 2–5 needs owner approval and the hardware
  and signing items under "What the owner must provide".
- **Scope:** every crate under `crates/` and `shell/`, the workspace `Cargo.toml`,
  `deny.toml`, `.github/workflows/ci.yml`, `.github/workflows/windows-preview.yml`,
  `.github/workflows/release.yml`, and `packaging/windows/`.
- **Supersedes:** nothing. Builds on ADR 0006 (shell/app split), ADR 0007 (compositor
  choice), ADR 0014 (Mission Control), ADR 0017 (Mac keyboard) and ADR 0022 (accounts).

## The question

People find installing Linux painful. The owner wants someone to download Lulo and use it
directly on Windows 10 or 11, with no Linux, no WSL and no dual boot. They should get the
whole experience, as on Lulo OS: the menu bar, Dock, Spotlight, Control Centre, Notification
Centre, ⌘Tab and Mission Control, plus all the Lulo apps.

The requirements:

- **One click to install.** A signed per-user installer that needs no admin rights. It puts
  Lulo's shell and apps on top of Windows and starts Lulo at login.
- **Safe coexistence with Explorer.** Lulo hides or auto-hides the taskbar and Start; it
  does not break them. Windows' own apps, file associations, the tray and notifications keep
  working.
- **Fully reversible.** Quitting Lulo or uninstalling it gives back the normal Windows
  desktop: the taskbar comes back and no system files are left changed.
- **One repository.** The apps share their code with the Linux build; only the backends
  differ.

So the question is: how much of today's code is Linux-specific, what replaces each piece on
Windows, and in what order do we build it so that "usable on Windows without Linux" arrives
as early as possible?

## What is in the repository today (read 2026-10-06)

The workspace has about 150 crates. The app layer (`rmac-ui`, `rmac-editor`, the app crates)
is mostly portable; the service layer is built on D-Bus, and the shell is built on niri and
`wlr-layer-shell`. The macOS build already compiles in CI, so most Linux code is already
gated with `cfg`. But that gating is usually `not(target_os = "macos")` (meaning "Linux"),
and its non-Linux branch calls macOS tools (`pmset`, `ioreg`, `networksetup`). Neither branch
is right for Windows.

### GPUI

- The pinned Zed revision `76c93968da5b8b8809bdd72e4ad9e7d0e946bad0` (in both lockfiles)
  ships `gpui_windows`. It uses Direct3D 11 with DirectComposition and DirectWrite text, and
  falls back to WARP software rendering when no hardware adapter is found. It has an
  `accesskit_windows` accessibility tree (UI Automation). Release builds compile its HLSL
  shaders with `fxc.exe` from the Windows SDK. Debug builds compile them at run time.
- `gpui_platform` picks `gpui_windows` automatically on `target_os = "windows"` and turns on
  `gpui`'s `windows-manifest` feature, which embeds a DPI-aware manifest through
  `embed-resource`. All of these are already in `Cargo.lock`.
- Our patches (`shell/compat/gpui_linux`, `gpui_wgpu`, `ztracing`) are not used on Windows.
  `ztracing` is pure Rust. (Amended 2026-10-07: `gpui_windows` is now vendored too, in
  `shell/compat/gpui_windows`, for its idle frame loop and start-up time; see ADR 0025.)
- The shell (`shell/`) depends on Wayland layer-shell surfaces, which have no Windows
  equivalent inside GPUI. Each Windows shell surface will be a normal GPUI window that we
  then turn into an AppBar or tool window through its `HWND` (`raw-window-handle`).

### Platform-specific crates and their Windows equivalents

**Session, shell surfaces and window management**

| Linux piece (crate) | What it does | Windows equivalent |
|---|---|---|
| niri + `rmac-compositor-niri` | Window list, focus, minimize, fullscreen and tiling over niri's IPC socket | `EnumWindows` + `SetWinEventHook` (`EVENT_OBJECT_CREATE/DESTROY/FOREGROUND`, `EVENT_SYSTEM_MINIMIZESTART`), `ShowWindow`, `SetForegroundWindow` (with `AllowSetForegroundWindow`), `SetWindowPos`. A new `rmac-compositor-win32` behind the same `rmac_compositor::{Snapshot, Action}` model. |
| `wlr-layer-shell` via `gpui_linux` (`shell/crates/rmac-shell-layer`, `shell/bins/*`) | Menu bar, Dock, OSD and switcher as overlays that stay out of window lists | Menu bar and Dock: AppBars through `SHAppBarMessage` (`ABM_NEW`, `ABM_QUERYPOS`/`ABM_SETPOS` on `ABE_TOP`/`ABE_BOTTOM`, `ABM_SETAUTOHIDEBAREX`). Other overlays: `WS_EX_TOOLWINDOW` + `WS_EX_TOPMOST` + `WS_EX_NOACTIVATE`, kept out of Alt-Tab and the taskbar. |
| `rmac-mission-control` (`screencopy.rs`, `capture.rs`) | Live window pictures through wlr-screencopy | DWM thumbnails (`DwmRegisterThumbnail`/`DwmUpdateThumbnailProperties`): live, GPU-composited and zero-copy. `Windows.Graphics.Capture` where a still image is needed (the Dock's minimised tile). |
| `rmac-screenshot` | Region, window and screen capture | `Windows.Graphics.Capture` (`GraphicsCaptureItem` for a monitor or an `HWND`). |
| `rmac-wallpaper*`, `rmac-wallpaper-portal` | Wallpaper and its XDG portal backend | `IDesktopWallpaper` (per monitor). No portal is needed. |
| `rmac-session`, systemd units in `packaging/rmac-session/systemd` | Session supervision and safe mode | One `lulo-session.exe` that supervises the shell processes. It starts through an MSIX `StartupTask` or an `HKCU\…\Run` entry. |
| `rmac-lock-provider(-linux)`, PAM (`pam-sys2`), `ext-session-lock` | The Lulo lock screen | `LockWorkStation()`. Windows owns the secure lock screen and the credential UI. We do not replace it: Windows Hello and the credential providers stay. |
| `rmac-polkit-agent`, `org.freedesktop.PolicyKit1` | Admin authentication | UAC (`ShellExecuteEx` with the `runas` verb) for the rare admin action. Lulo itself never runs elevated. |
| logind (`org.freedesktop.login1`, in `rmac-session`, `rmac-osd`, `rmac-shortcuts`, `calendar-agent`, `notes`) | Sleep, shutdown, idle and resume | `WM_POWERBROADCAST`/`RegisterSuspendResumeNotification`, `WTSRegisterSessionNotification`, `WM_QUERYENDSESSION`/`WM_ENDSESSION` (ADR 0020's unsaved-work flow), `ExitWindowsEx`, `SetSuspendState`. |

**Input and shortcuts**

| Linux piece | Windows equivalent |
|---|---|
| `rmac-shortcuts` (portal GlobalShortcuts + niri fallback) | `RegisterHotKey` for plain global shortcuts. A `WH_KEYBOARD_LL` low-level hook for chords that Windows reserves: the Win key opening Start, ⌘Tab, ⌘Space and ⌘Q, with ⌘ mapped to Win or Alt. |
| `rmac-keyboard` + keyd + XKB options (ADR 0017) | The same pure generator, plus a hook-based remapper: ⌘ becomes Ctrl for non-Lulo apps, chosen per foreground app. A Scancode Map is not used; it needs admin rights and a reboot. |
| `rmac-input` (niri pointer and touchpad settings) | `SystemParametersInfo` (`SPI_SETMOUSESPEED`, wheel lines) and the Precision Touchpad settings registry under HKCU. Scroll direction is per device. |
| `xkbcommon` in `rmac-lock-provider-linux` | Not needed: GPUI's Windows backend handles the keyboard layout. |

**System services (Control Centre, menu-bar extras, System Settings)**

| Linux piece (crate) | Windows equivalent |
|---|---|
| NetworkManager (`rmac-network`, `rmac-shell-status-linux`, `rmac-quick-settings-system`) | `WlanApi` (`WlanEnumInterfaces`, `WlanGetAvailableNetworkList`, `WlanConnect`, `WlanRegisterNotification`) or `Windows.Devices.WiFi`. `INetworkListManager` for connectivity and captive portals. VPN: `RasEnumConnections` or `Windows.Networking.Vpn` (read-only at first). |
| BlueZ (`rmac-bluetooth`, pairing agent) | `Windows.Devices.Bluetooth` and `Windows.Devices.Enumeration` (`DeviceWatcher`, `DeviceInformationCustomPairing`). Turning the radio on and off uses `Windows.Devices.Radios`. |
| PipeWire/WirePlumber via `wpctl`/`pactl` (`rmac-audio`, `rmac-sound`, `player`) | Core Audio: `IMMDeviceEnumerator` with `IMMNotificationClient` for devices, `IAudioEndpointVolume` with a callback for volume and mute. The default device is set through the undocumented `IPolicyConfig` (risk). Alert sounds use `PlaySound`. |
| UPower and power-profiles-daemon (`rmac-power`) | `GetSystemPowerStatus`, `Windows.System.Power.PowerManager` (events), and `PowerSetActiveOverlayScheme` for power modes. Battery health comes from `Windows.Devices.Power.Battery`. |
| Brightness (`rmac-osd`, `rmac-quick-settings`) | WMI `WmiMonitorBrightnessMethods` for internal panels, DDC/CI (`SetMonitorBrightness`) for external monitors. The OSD replaces Windows' flyout only for keys Lulo handles. |
| MPRIS (`rmac-media`, Now Playing) | `Windows.Media.Control.GlobalSystemMediaTransportControlsSessionManager`. |
| `org.freedesktop.Notifications` + `rmac-notifications-linux` (we are the server) | We cannot be the notification server on Windows. Lulo's own apps raise toasts (`Windows.UI.Notifications` via the App SDK, which needs an AUMID, given by MSIX). The Notification Centre reads every app's toasts through `UserNotificationListener`, which needs the `userNotificationListener` capability and user consent. `Shell_NotifyIcon` is for the tray icon only. |
| timedate1 and locale1 (`rmac-time-linux`, `rmac-locale-linux`) | Time zone: `SetDynamicTimeZoneInformation` needs admin, so we open Windows Settings instead. Locale: read-only through `GetUserDefaultLocaleName`. |
| hostname1, AccountsService (`rmac-users-linux`, `setup-assistant`) | `GetComputerNameEx`, `NetUserGetInfo` and `Windows.System.User` for the user's name and picture. Managing users is left to Windows Settings. |
| PackageKit (`rmac-updates-linux`) | Lulo's own updater (MSIX App Installer auto-update or a signed update feed). Windows Update stays with Windows. |
| systemd and firewall inspection (`rmac-sharing-linux`) | Out of scope. Sharing settings link to Windows Settings. |
| XDG PermissionStore (`rmac-privacy-linux`) | `Windows.Security.Authorization.AppCapabilityAccess` for reading. Changes are made in Windows Settings ▸ Privacy. |
| Login items via XDG autostart (`rmac-login-items-linux`) | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` and the `StartupApproved` key. MSIX packages use `Windows.ApplicationModel.StartupTask`. |
| `sysinfo` (Activity Monitor) | Already supports Windows. |
| `rmac-system-info` (D-Bus and `/proc`) | WMI `Win32_ComputerSystem`/`Win32_Processor`, `GlobalMemoryStatusEx`, `RtlGetVersion`. |

**Accounts and personal data**

| Linux piece | Windows equivalent |
|---|---|
| GNOME Online Accounts (`rmac-accounts-linux`, ADR 0022) | Lulo's own OAuth: authorization code with PKCE in the system browser and a loopback redirect (`http://127.0.0.1:<port>`). This is the same flow ADR 0022 plans for Lulo's own sign-in UI, so `rmac-accounts` stays shared. `WebAuthenticationBroker` is an option for the MSIX build. |
| Secret Service / keyring | Credential Manager (`CredWriteW`/`CredReadW`, protected by DPAPI) or `Windows.Security.Credentials.PasswordVault`. |
| EDS calendars (`rmac-calendar-eds`, `calendar-agent`) | Lulo's own CalDAV, ICS and Microsoft Graph backends inside `rmac-calendar-store`. There is no system calendar service to reuse; the WinRT `AppointmentStore` is limited and is not the source of truth. |
| Mail (`rmac-mail-*`) | Already its own Rust engine (IMAP, SMTP, Graph). It ports once storage and TLS roots work: `rustls-native-certs` and `platform-verifier` both support Windows. |

**Files, storage, search and printing**

| Linux piece (crate) | Windows equivalent |
|---|---|
| inotify through the `notify` crate (`rmac-apps`, `rmac-theme`, `rmac-shell-settings`, `text-editor`, `finder`, …) | `notify` already uses `ReadDirectoryChangesW` on Windows. The direct inotify use in `rmac-shell-status-linux` stays Linux-only. |
| POSIX file safety (`rmac-storage`, `rmac-recent-documents`, `rmac-notes-storage`: `O_NOFOLLOW`, `flock`, `geteuid`, mode 0600) | `FILE_FLAG_OPEN_REPARSE_POINT` with a reparse-point check, `LockFileEx`, and per-user `%LOCALAPPDATA%`, which an ACL already protects. Each crate keeps a `unix` and a `windows` module behind one function. |
| xattr tags (`rmac-search`, `finder`, via `rustix::fs::{get,set}xattr`) | An NTFS alternate data stream (`file:lulo.tags`), or a tag index in our own store for files on FAT and exFAT. |
| Spotlight file search (`rmac-search`, `rmac-launcher-providers`) | Windows Search through OLE DB (`ISearchQueryHelper` → `SystemIndex` SQL). The `walkdir` scan stays as a fallback when indexing is off. |
| App discovery from `.desktop` files (`rmac-apps`) | The Start menu's `.lnk` files (`%APPDATA%` and `%ProgramData%\Microsoft\Windows\Start Menu`), plus `IShellItem` with `FOLDERID_AppsFolder` for Store apps. Launch with `ShellExecuteEx` or `IApplicationActivationManager`. |
| Icons (`rmac-icon`, freedesktop icon theme) | `SHGetFileInfo`, `IShellItemImageFactory` and `SHGetStockIconInfo` for Windows apps. Lulo's own app icons stay. |
| udisks (`finder`, `rmac-dbus`), `rmac-mounts` | `GetLogicalDrives`, `GetVolumeInformation`, `CM_Request_Device_Eject`, and `WM_DEVICECHANGE` notifications. |
| Trash (`trash` crate) | Already supports the Windows Recycle Bin. |
| XDG portals (`rmac-portal`, `rmac-file-chooser`, `ashpd`) | `ShellExecuteEx` for open-with. Open and Save use our own `rmac-file-chooser` panel by default (Mac look); the native `IFileOpenDialog` is a fallback. Open With uses `SHAssocEnumHandlers`. |
| CUPS and the Print portal (`rmac-print-linux`, `rmac-printers-linux`) | `rmac-print` already produces a PDF. Print it with the Windows PDF print path (`Windows.Graphics.Printing` + `PrintDocument`, or the XPS Print API), or hand it to `ShellExecute` with the `print` verb. Printers are listed with `EnumPrinters`. |
| Thumbnails (`rmac-thumbnails`) | `IThumbnailCache` and `IShellItemImageFactory`. |
| Terminal: `portable-pty` + `libc` | `portable-pty` already supports ConPTY. Change the default shell to PowerShell or `cmd`, and set the profile path. |
| Fonts: fontconfig (`packaging/fontconfig`, `rmac-gtk-settings`) | DirectWrite through GPUI. We ship our UI fonts in the package and load them privately with `AddFontResourceEx(FR_PRIVATE)`; GPUI loads embedded fonts itself. |
| GTK/Qt theming (`rmac-gtk-settings`, `rmac-appearance-portal`, ADR 0019) | Windows apps follow `AppsUseLightTheme` (HKCU `Personalize`); Lulo writes it when the user switches Light/Dark. The accent colour goes to `HKCU\…\DWM\AccentColor`. Lulo reads Windows' setting through `UISettings.ColorValuesChanged`. |

**Packaging**

| Linux piece | Windows equivalent |
|---|---|
| `.deb`/APT repo, Flatpak, rollout (`packaging/apt`, `flatpak`, `rollout.yml`) | A signed MSIX package with an `.appinstaller` feed for auto-update and staged rollout; per-user MSI (WiX) as the fallback. |
| `rmac-wayland-session`, greeter, `system-sleep` hooks | Not used. Windows owns sign-in. Lulo starts after the user signs in. |

## Decisions

### 1. Run alongside Explorer; do not replace the shell (by default)

Explorer stays the Windows shell. It keeps owning file associations, Start, the tray, toasts,
Windows Hello and the sign-in, lock and Ctrl-Alt-Del screens. Lulo runs on top of it.

- **Menu bar and Dock:** separate top and bottom AppBars. Windows then shrinks the work area
  for every app, and maximised windows respect the bar and the Dock, as on the Mac.
- **The taskbar:** set to auto-hide while Lulo runs, through `SHAppBarMessage(ABM_SETSTATE,
  ABS_AUTOHIDE)`. Lulo records the previous state in `HKCU\Software\Lulo` first. If the user
  chooses "Hide the Windows taskbar", Lulo also hides it with `ShowWindow(Shell_TrayWnd,
  SW_HIDE)`, plus `Shell_SecondaryTrayWnd` on each extra monitor. It shows the taskbar again
  on exit, on crash (a watchdog in `lulo-session`) and on uninstall.
- **Explorer restarts:** Lulo listens for the registered `TaskbarCreated` message and puts
  the taskbar state and its AppBars back.
- **Start:** the Win key opens Spotlight, or Launchpad (Apps) when chosen. The low-level
  hook swallows the Win key's release so Start does not open. Ctrl-Esc and the Start button
  still work, so Windows is never out of reach.
- **The tray:** the system tray is still reachable when the taskbar is shown. Phase 3 adds
  a menu-bar extra that mirrors tray icons, read through UI Automation on the overflow window.

Replacing the shell through `HKCU\Software\Microsoft\Windows NT\CurrentVersion\Winlogon\Shell`
becomes an optional "Lulo only" mode in phase 5, and is never the default. Without Explorer
there is no Start, file-association UI, tray or toast host, and several Settings pages
misbehave. A broken Lulo would then leave the user at a black screen; the only recovery is
Task Manager → Run `explorer.exe`, or Safe Mode. The mode would need its own recovery key
and a watchdog that falls back to `explorer.exe`.

### 2. Per-user only, never elevated

Lulo installs and runs as the signed-in user. It writes only to `HKCU`, `%LOCALAPPDATA%`
and its package folder. It never asks for UAC during install or use. User Interface
Privilege Isolation (UIPI) means a medium-integrity process cannot hook or control elevated
windows: ⌘-key remapping, window moves and thumbnails will not work on admin windows. That is
acceptable and we document it. We do not ship a `uiAccess` manifest, which would need
installation under Program Files and admin rights.

### 3. Installer: a signed MSIX first, a per-user MSI as the fallback

| Option | For | Against |
|---|---|---|
| **MSIX (full trust, desktop bridge)** | One-click install from a web page through App Installer; clean, complete uninstall; built-in delta auto-update (`.appinstaller`); `StartupTask` for login start; an AUMID for toasts; identity for `userNotificationListener` | Writes to `HKCU` and AppData are virtualised (use `unvirtualizedResources`/`desktop6:RegistryWriteVirtualization` for the taskbar and theme keys); needs a trusted signature (no self-signing for users); App Installer must be present (it is on Windows 10 1809+ and 11) |
| Per-user MSI (WiX 4, `ALLUSERS=2`, `MSIINSTALLPERUSER=1`) | Familiar; no virtualisation; works where App Installer is blocked | Weaker uninstall guarantees (custom actions must restore the taskbar); we must build auto-update ourselves; SmartScreen reputation is per file |
| Our own self-extracting installer | Total control | We would rebuild uninstall, repair and update; AV heuristics distrust unknown installers |

Decision: MSIX first. A WiX per-user MSI is a stretch fallback for managed PCs. Both must be
signed.

**The uninstall contract**, tested in CI on every release:

1. The taskbar's auto-hide and visibility return to the values recorded at first run.
2. The `Run`/`StartupTask` entry, any `Winlogon\Shell` override (phase 5 only) and the Lulo
   keys under `HKCU` are removed.
3. `AppsUseLightTheme` and the accent colour are left as the user last chose them. They are
   the user's settings, and Windows works with either value.
4. User documents (Notes, Mail and so on) stay. "Remove my Lulo data" is a separate,
   explicit choice.
5. No file outside the package and the user's AppData is changed.

MSIX removes the package files and their virtualised state by itself. Steps 1–2 run in
`lulo-session --restore-windows-desktop`, which is called on normal exit and by the crash
watchdog. Full-trust MSIX apps get no uninstall hook, so Lulo never leaves hidden state that
would outlive it. The taskbar is only ever *auto-hidden* in the user's own setting, which
Lulo restores whenever it stops. `ShowWindow(SW_HIDE)` lasts only as long as Lulo runs, and
Explorer shows the taskbar again once Lulo is gone. For the rare case where Lulo dies without
running the watchdog, `lulo-session` puts a "Restore Windows taskbar" shortcut in the Start
menu.

### 4. Code signing

Unsigned downloads show SmartScreen's "Windows protected your PC" until the file builds up
reputation, and MSIX will not install unsigned at all. Options:

- **Azure Trusted Signing** (now "Artifact Signing"; Microsoft-managed keys, about $10 a
  month): needs identity validation of a company, or of an individual in the regions where
  that is offered. It works from GitHub Actions with OIDC, and the signing identity, not each
  file, builds SmartScreen reputation. **Recommended.** Check current eligibility and price
  when signing up; both have changed since launch.
- **An OV certificate on a hardware token or cloud HSM:** about $200–400 a year; reputation
  builds slowly; CI signing needs a cloud HSM.
- **An EV certificate:** about $300–600 a year. Instant SmartScreen trust is no longer
  guaranteed (the policy changed in 2024). The token makes CI harder.
- **The Microsoft Store:** Microsoft signs the package. It needs a Partner Center account and
  Store certification of a full-trust app that installs a keyboard hook; there is review risk.

### 5. The abstraction seam

There is no single "platform" crate. Lulo already splits each system service into a
platform-neutral model and a backend. We make the backend choice explicit and four-way:

```
crates/rmac-<service>/src/
  model.rs        // shared types, parsers, reducers (all platforms, unit-tested)
  linux.rs        // cfg(target_os = "linux"): D-Bus, files under /sys, etc.
  macos.rs        // cfg(target_os = "macos"): developer adapters only
  windows.rs      // cfg(windows): Win32/WinRT, added in the phase that needs it
  unavailable.rs  // every other target: honest "unavailable" errors, no fake data
```

Rules:

- **Dependency gating.** Linux-only dependencies (`zbus`, `ashpd`, `rmac-dbus`,
  `wayland-*`, `xkbcommon`, `pam-sys2`, Linux-only `rustix` features) go under
  `[target.'cfg(target_os = "linux")'.dependencies]` or `cfg(unix)`, never under
  `cfg(not(target_os = "macos"))`. Windows-only dependencies (`windows`, `windows-sys`, at
  versions already in `Cargo.lock` where possible) go under `cfg(windows)`.
- **cargo-deny.** `deny.toml`'s `[graph] targets` gains `x86_64-pc-windows-msvc`, so the
  licence and advisory policy covers the Windows graph too.
- **Window management.** App crates never call `rmac_compositor_niri` directly. `rmac-ui`
  owns a small window-manager module with one set of functions: snapshot, minimise, hide,
  quit, fullscreen and the Mission Control requests. It routes to niri IPC on Linux. On
  Windows it uses GPUI's own window calls until `rmac-compositor-win32` exists: minimise,
  zoom, close and quit act on this process's windows only.
- **Shell-only crates** (`rmac-top-bar`, `rmac-dock`, `rmac-shell-status`, and through them
  `rmac-network`, `rmac-bluetooth` and `rmac-power`) must not be needed to build an app.
  `rmac-ui` uses them only to fit a new window between the Linux menu bar and Dock, so they
  are `cfg(unix)` dependencies there.
- **Unix process and file plumbing** (signals, `flock`, `O_NOFOLLOW`, `geteuid`,
  `prctl(PDEATHSIG)`, Unix sockets) sits in `cfg(unix)` functions with a `cfg(windows)`
  twin of the same signature. The process-group link uses a Job Object with
  `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. Locks use `LockFileEx`. IPC uses named pipes or
  `AF_UNIX` (Windows 10 1803+ has Unix sockets, which keeps the IPC code shared).
- **Honest stubs.** A Windows build may lack a feature, such as the spell checker before
  dictionaries ship. Then the menu item is hidden or disabled with a reason; it never
  silently does nothing. This is the same rule as on Linux.
- **Paths.** About sixty files read `XDG_*_HOME` and `HOME` directly. Rather than touch each
  one, `rmac_ui::application()` (every app's first call) fills in the unset variables on
  Windows before any thread starts: `HOME` from `%USERPROFILE%`, config and data under
  `%APPDATA%\Lulo`, state and cache under `%LOCALAPPDATA%\Lulo`. Phase 2 moves the readers to
  one `rmac-storage` paths function and drops the variables. There is no `XDG_RUNTIME_DIR`
  on Windows; code that needs one (Unix sockets) is `cfg(unix)`.

## Phase 1 slice as built

Calculator, Notes and Text Editor build, pass Clippy (`-D warnings`, all targets) and their
unit tests, and open their windows on Windows. So do the shared crates they need (`rmac-ui`,
`rmac-editor`, `rmac-storage`, `rmac-recent-documents` and the `rmac-notes-*` crates). The
CI job `windows` in `ci.yml` (non-blocking) proves it on `windows-latest`. It launches each
app with a private profile through `scripts/windows/launch_smoke.py` and uploads a screenshot
of each window.

On Windows the three apps and their dependencies are 28 workspace crates, down from 44.
Sixteen are no longer built there: `rmac-network`, `rmac-bluetooth`, `rmac-power`,
`rmac-audio`, `rmac-process`, `rmac-dbus`, `rmac-focus*` (four crates),
`rmac-notifications`, `rmac-compositor-niri`, `rmac-top-bar`, `rmac-dock`, `rmac-places`
and `rmac-shell-status`. The only D-Bus code left in the graph is `rmac-app-menu`'s transport (`zbus`
compiles on Windows and is not called there).

| Seam | Linux / macOS | Windows (phase 1) |
|---|---|---|
| Window manager (`rmac-ui` `chrome.rs`, `window.rs`) | niri IPC and Mission Control's socket; `rmac-compositor-niri`, `rmac-dock` and `rmac-top-bar` are `cfg(unix)` dependencies | GPUI's own calls on this app's windows: minimise, full screen, Zoom as Maximise, ⌘H minimises the app's windows, and ⌘Q sends each window `RequestClose`, so it goes through its close guard. Tiling and ⌥⌘H report "not available on Windows yet". Windows itself keeps new windows inside the work area. |
| Base directories (`rmac-ui` `platform.rs`) | `HOME` and `XDG_*` from the session | `rmac_ui::application()` fills the unset variables: `HOME` from `%USERPROFILE%`, config and data under `%APPDATA%\Lulo`, state and cache under `%LOCALAPPDATA%\Lulo`. Unit-tested on every platform. |
| Durable writes (`rmac-storage` `sync_directory`) | fsync the parent directory after a rename | No directory fsync (Windows cannot open a directory with write access); NTFS journals the rename. |
| File locks (`rmac-notes-storage` writer lease, `rmac-recent-documents`) | `flock` with `O_NOFOLLOW` and owner checks | `LockFileEx` through std's `File::try_lock`; a link in place of the lock file is refused. The kernel-lock test runs on Windows. |
| Alert sounds (`rmac-sound`) | PipeWire playback; mute and Focus state from `rmac-audio` and `rmac-focus-linux` | No cue plays yet (`PlaySound` comes in phase 2); neither dependency is built. |
| Colour tags (`rmac-search`) | `user.rmac.tag` xattr through `rustix` (`cfg(unix)`) | No file has a tag until Files writes one into an NTFS alternate data stream (phase 4). |
| Notes ▸ Record Audio | `pw-record` | Says "Notes can't record audio on Windows yet." |
| Spelling (`rmac-spelling`) | hunspell dictionaries in `/usr/share/hunspell` | None is found, so checking fails open (no underlines). Phase 2: Windows' own `ISpellChecker`. |
| `cargo deny` | Linux and macOS targets | Adds `x86_64-pc-windows-msvc`; `dwrote` 0.11.5 (MPL-2.0, DirectWrite bindings under font-kit) gets a reviewed exception like `spellbook`. |

Known gaps that phase 2 must close before anyone downloads the apps:

- **No menus.** The menu model is exported only to the Lulo menu bar over D-Bus, and GPUI on
  Windows does not draw native menus, so on Windows today only the keyboard shortcuts reach
  menu commands. Phase 2 draws a Mac-style menu strip inside each window until the phase 3
  menu bar exists.
- **Shortcuts use the Windows key.** Every binding is written `cmd-…`, which GPUI reads as
  the Windows key on Windows, and Windows reserves most Win+letter chords. Until the phase 3
  keyboard hook maps ⌘ (ADR 0017), the apps must also bind `ctrl-…` on Windows; this is the
  first phase 2 task, in `rmac-ui::shortcuts`.
- The title-bar double-click follows Windows (maximise or restore), not the Desktop & Dock
  setting.
- Open and Save use GPUI's prompts, which on Windows are the native file dialogs, not the
  Mac-style Lulo panel. Notes' Import and Export choosers go through `rmac-portal` (the XDG
  portal), which reports them unavailable on Windows; phase 2 routes them through GPUI's
  prompts too.
- Printing is unavailable (`rmac-print-linux` is Linux-only). Export as PDF works.
- Single instance: a second launch opens a second process (the D-Bus hand-off is Linux-only).
  Phase 2 uses a named pipe.

## Phase 2 first slice as built (branch `op/win-phase2a`)

The five gaps above that block anyone from using the three apps are closed, in the shared
crates, so every app that moves to Windows gets them:

| Gap | Linux / macOS | Windows (phase 2a) |
|---|---|---|
| Shortcuts (`rmac-ui` `shortcuts.rs`, `text_keys.rs`) | Bindings are written `cmd-…`; keyd makes the ⌘-position key send Super (ADR 0017) | `rmac_ui::bind_keys` installs each binding with Ctrl as the primary modifier: `cmd-s` becomes `ctrl-s`, `alt-cmd-c` becomes `ctrl-alt-c`. ⌃⌘ chords keep both modifiers (Win+Ctrl), and Cocoa's ⌃-letter Emacs keys are dropped, because Ctrl+letter is now a command. Text fields keep gpui-component's PC set (Ctrl+←, Home, Ctrl+Y), with Ctrl+F handed back to the app's Find, Ctrl+Shift+Z as Redo and Ctrl+Alt+Shift+V as Paste and Match Style. Menus show "Ctrl+Shift+S" for "⇧⌘S" (`shortcuts::display_hint`). Calculator, Notes, Text Editor and `rmac-ui`'s own bindings go through it. |
| Menus (`rmac-ui` `menu_strip.rs`, vendored `gpui-component` `RootHeader`) | The Lulo menu bar shows them over D-Bus | Each app window draws a 24 pt Mac-style menu strip along its top edge, where Windows' own title bar would be (Lulo's title bars are part of each app's content, so a strip below them would need every app's layout to change): the bold app name (About, the app's items, Hide, Quit), the app's menus from the same `rmac-app-menu` table with the same live state, then Window (Minimize plus the app's items) and Help. Menus open with the shared `ContextMenu` renderer and send commands the way the menu bar does (`menu_target::dispatch_menu_action`). Alt alone opens the first menu, Alt+letter the menu with that initial, ←/→ move between menus while one is open, and Esc closes. Hovering another title while a menu is open switches to it. The empty part of the strip drags the window. Windows grow by the strip's height, so content keeps its size. The strip is off where the Lulo menu bar runs (`LULO_MENU_BAR`, for phase 3); `RMAC_IN_WINDOW_MENUS=1` turns it on elsewhere for development. Without a menu bar a windowless app would be unreachable, so closing an app's last window quits it. |
| Single instance (`rmac-ui` `instance_windows.rs`) | D-Bus hand-off (`org.rmac.AppInstance1`) | The running app serves `\\.\pipe\lulo-<app id>-<user>` (first instance only, no remote clients, the default same-user security). A later launch writes one line per window (arguments separated by NUL), allows the running process to take the foreground, and exits; the running app opens the windows. Text Editor and Notes use it; a document opened from Explorer reaches the running Text Editor. |
| Choosers (`notes` `file_choosers.rs`) | The XDG portal | Notes' Attach Photo, Attach File, Import and Export use GPUI's prompts, which are Windows' own Open and Save As dialogs; Export starts in Documents. Attachment chips open with the default app (`open_with_system`). Text Editor's Open and Save already used GPUI's prompts. |
| Alert sounds (`rmac-sound`) | `pw-play` with Lulo's own cues | The alert is Windows' Default Beep (`MessageBeep(MB_OK)`), the error is Critical Stop and a notification is Asterisk, from the user's sound scheme at the system volume. Other cues stay silent until Lulo's cue files ship in the package. |

CI (`windows` job): `launch_smoke.py` now also taps Alt in each app and checks that the first
menu opens below the strip (a screenshot of it is uploaded), presses Ctrl+N (Text Editor must
open a second window), and launches Text Editor a second time to check the hand-off.

Still open from phase 2's list: `ISpellChecker` spelling, printing through the Windows PDF
path, the Mac-style Open and Save panel, toasts, the remaining app crates, MSIX packaging and
signing, and the UI Automation behaviour subset. The `WIN-OS-*` rows in `docs/parity.md` track
the Windows gaps.

## Phase 2 slice as built (Preview, Clock, Weather, Terminal)

Branch `op/win-phase2b` extended the apps that build, pass Clippy and open a window on
`windows-latest` CI past Calculator/Notes/Text Editor, in the order the phase plan named —
Preview, Clock, Weather, then Terminal — because most of their platform seams turned out to
already follow this ADR's `cfg(target_os = "linux")`/`cfg(not(target_os = "linux"))` pattern
(the "not Linux" branch already shared by macOS and, it turns out, Windows): clipboard,
printing and notification code in Preview/Clock/Terminal needed no change at all to compile
and behave honestly on Windows. The `windows` CI job's package list and
`scripts/windows/launch_smoke.py` now cover all seven apps.

- **Preview.** Images are unaffected — nothing in that path was Linux-specific. PDF
  rendering stays poppler-utils (no Windows build of it exists), so
  `poppler::missing_tool_message` now gives a Windows-specific "Preview can't open PDF
  documents on Windows yet." instead of pointing at an apt package that doesn't exist there;
  a real Windows.Data.Pdf or pdfium backend is still open (see docs/parity.md PREV-29).
- **Clock.** Time zones are real on Windows, not a stub: `chrono-tz` (already a workspace
  dependency via the calendar crates, so this adds no new dependency to review) backs named
  IANA zones for World Clock and alarm scheduling, and `chrono::Local` backs the OS's own
  zone when no IANA name can be read, replacing the Unix-only `/usr/share/zoneinfo` reader.
  Alarms/timers are the honest stub this ADR's "Honest stubs" rule asks for: Windows has no
  systemd user units, so `schedule::apply` now returns a clear "not available on this
  platform yet" error on every non-Linux target instead of shelling out to a `systemctl` that
  was never going to exist (previously ungated, so it also silently tried this on macOS).
- **Weather.** No change needed: it fetches over HTTPS through the system `curl` (present on
  Windows 10 1803+/11) and has no location lookup to begin with — already "manual cities
  only", the phase-2 fallback this ADR allows.
- **Terminal.** `portable-pty`'s ConPTY backend runs PowerShell as the default shell; the
  existing `cfg(unix)` gating around PTY job control (`libc::tcgetpgrp`, process-group
  `SIGHUP`) and Shell ▸ Open…'s Unix executable-bit check needed only a Windows default shell
  and an extension-based (`.exe`/`.bat`/`.cmd`) stand-in for "executable", not new platform
  code. "Run command inside a shell" and Shell ▸ Open… on a non-executable file now use
  `powershell.exe -Command`/`-File` on Windows in place of `/bin/sh -c`/`/bin/sh`.

Not done in this slice (tracked in docs/parity.md): real PDF rendering and Windows alarm/timer
delivery. These four apps still bind their own shortcuts as `cmd-…`; moving them onto
`rmac_ui::bind_keys` (phase 2a above) is the next step.

## Phase 2c as built (branch `op/win-phase2c`)

Branch `op/win-phase2c` closed the four gaps the phase 2b section above left open for
Preview, Clock, Weather and Terminal, plus real PDF rendering:

- **Preview's missing menu strip.** CI on `integ` caught this before the shortcut work above
  even landed: Preview's `open_window` opens its window with a bare `cx.open_window` (it has
  one window per document, not Calculator/Clock/Weather's single `app_id`-keyed one), and
  calls `rmac_ui::track_key_window` directly rather than the usual `rmac_ui::observe_window_
  state`, which bundles that with giving the window a strip (`menu_strip::register_window`).
  Preview's own call site never did the second half, so no window had a strip to open: Alt
  did nothing, and the launch check's "choose About from the menu" step failed too, since
  there was no menu to choose it from. A new `rmac_ui::register_menu_strip_window` exposes
  just that half, and Preview's `open_window` now calls it next to `track_key_window`.
  Clock, Weather and Terminal were not affected — they already go through `observe_window_
  state` (Clock, Weather directly; Terminal through `rmac_ui::open_app_window`, which
  `boot_app_instance` uses) — but this was still worth checking everywhere once Preview's
  copy of the pattern turned out to have dropped half of it.

- **Shortcuts.** Preview, Clock and Weather move their own `fn bind_keys`/inline
  `cx.bind_keys` calls onto `rmac_ui::bind_keys`, exactly as Calculator/Notes/Text Editor did
  in phase 2a: every `cmd-…` binding gets a Ctrl-primary twin on Windows, with no change to
  the Linux/macOS keystrokes they already had.

  Terminal needed care rather than a plain swap: most of its shortcuts are `cmd-…` like any
  other app's, but Copy (⌘C) and Paste (⌘V) are not — a Windows terminal's whole point is
  that Ctrl+C still reaches the shell (the interrupt) and PSReadLine/console keeps its own
  Ctrl+A/E/K/R/U/W and friends, none of which Terminal may capture as a GPUI key binding
  (a bound key never reaches the PTY at all, regardless of context). The rule this branch
  picked, matching Windows Terminal's own defaults: Copy and Paste move to Ctrl+Shift+C and
  Ctrl+Shift+V there, and nothing binds bare Ctrl+C or Ctrl+V on Windows, so both always
  reach the shell — a stronger and simpler guarantee than "only when there is no selection".
  Paste Selection (⇧⌘V on the Mac) would land on the same `ctrl-shift-v` under the ordinary
  mapping, so on Windows only it moves one chord over, to Ctrl+Alt+Shift+V. Every other
  Terminal shortcut (Find, Select All, New Tab, Clear, …) takes the ordinary mapping like
  any other app's, which is a deliberate, documented trade-off rather than a silent one:
  several of those bare Ctrl+letter chords (Ctrl+F, Ctrl+R, Ctrl+A, Ctrl+K, …) coincide with
  a PSReadLine/console binding of the same key, and Terminal's menu command wins while its
  window is focused. A future pass could widen the Ctrl+Shift+ treatment further, but Copy,
  Paste and the shell's own interrupt were the ones that had to be right for Terminal to be
  usable at all. The right-click context menu's Copy/Paste hints follow the same rule
  (`copy_shortcut`/`paste_shortcut` in `overlays.rs`); the in-window menu strip's Edit menu
  still shows the Mac-derived "Ctrl+C"/"Ctrl+V" hint text, a known cosmetic mismatch left for
  a later pass (it would need `rmac-app-menu`'s static per-app menu tables to carry a
  platform-specific hint, which no app needs today).

- **Single instance.** Calculator, Clock and Weather now hand off to a running instance the
  same way Text Editor, Notes and Preview already did in phase 2a/2b (`hand_off_to_running_
  instance` before GPUI starts, `install_app_instance` once it has): on Windows the named
  pipe from phase 2a, unchanged on Linux's D-Bus hand-off. Unlike Preview (which opens a new
  window per document) these three have exactly one window, so a successful hand-off also
  calls a new `rmac_ui::focus_running_app`/`activate_app_window` pair — the launching process
  asks the compositor (Linux) or the foreground-lock exception it already holds (Windows) to
  bring the running window forward, since the long-running process does not hold the user's
  own activation. Terminal already had this through `boot_app_instance`.

- **Real PDF rendering.** Preview's Windows build renders PDFs for real instead of refusing
  them: a new `winpdf.rs` module calls `Windows.Data.Pdf` (WinRT, through the `windows`
  crate — already a workspace dependency for GPUI's own Windows backend and the phase 2a
  named-pipe code, so this adds no new crate to review, only more of its features:
  `Data_Pdf`, `Storage`, `Storage_Streams`, `Foundation`, `Win32_System_WinRT`) and feeds the
  result into the exact same page pipeline poppler's Linux/macOS path already built:
  `PdfInfo`/`PageBox` for the page list and layout, a BGRA `RgbaImage` per page for the
  viewer. `render.rs`'s `load_pdf_info` and `render_page` now have a `#[cfg(windows)]` body
  that calls `winpdf` and a `#[cfg(not(windows))]` body that is poppler's existing code,
  unchanged. Every call blocks its thread on the WinRT async result, but `render.rs` only
  ever calls these from a background thread already (`blocking::unblock`/
  `cx.background_executor()`), never GPUI's render loop, so this is no UI-thread work, just
  like the poppler subprocesses were. A thread-pool worker may never have touched WinRT
  before, so `winpdf::ensure_winrt_apartment` initialises it (idempotent, never undone) before
  every call. Rendering itself goes through a throwaway temp file rather than an in-memory
  stream — `PdfPage::RenderWithOptionsToStreamAsync` only renders to a stream either way, and
  reading the result back with a plain `std::fs::read` needed no further WinRT calls to get
  wrong. `PdfPage::Size` already reflects the page's own `/Rotate` (unlike poppler's
  `pdfinfo`, which reports the raw media box plus a separate rotation), so `winpdf` reports
  `PageBox::rotation` as zero to avoid rotating an already-rotated page a second time; the
  viewer's own manual rotation still applies on top in `render::render_page`, exactly as it
  does for poppler's pages. `cargo-deny`'s Windows graph needed no changes: `windows` was
  already an allowed dependency, and feature flags are not part of its policy surface.

  Still not real on Windows: the search/selection text layer (`Windows.Data.Pdf` is a
  renderer, not a text reader, so `extract_text` stays poppler-only and fails with an honest
  "Preview can't search or select text in PDF documents on Windows yet."), password-protected
  PDFs (no credential prompt), and printing (`rmac-print-linux` is Unix-only, already gated).

- **CI proof.** `scripts/windows/launch_smoke.py` already opened every app's menu strip with
  Alt — that was already true for all seven apps as of phase 2b, since the check runs
  unconditionally for whichever apps the caller lists, and the `windows` job in `ci.yml`
  already listed all seven. This branch's addition is Preview's PDF check: the script writes
  a tiny one-page PDF itself (a red square, hand-built — no poppler or other tool involved,
  so the check does not depend on anything the Windows runner lacks), launches Preview with
  it on the command line, waits for `winpdf` to rasterise the page, and looks for the
  fixture's colour anywhere in the captured window (`has_reddish_pixel`, loose on exact
  values since WARP's software rasteriser and PNG recompression both shift them slightly).
  The screenshot it saves (`rmac-preview-pdf.png`) is uploaded with the rest.

## Phase 2c follow-up as built (branch `op/win-polish`)

Branch `op/win-polish` worked the speed and polish gaps phase 2c left open,
using the reference laptop's first real numbers (Ryzen 3 7320U, 8 GB, warm
release build: 0.27–0.31 s to open each app, 0.5–3 % of one core sampled
1.5–4.5 s after launch) as the baseline to improve on and CI as the only
place that can prove a Windows change, since this pass had no GUI access to
the laptop (session 0/SSH only) or to any Mac:

- **Idle CPU, measured.** `scripts/windows/launch_smoke.py` now times each
  app's own CPU (`GetProcessTimes`) over a 20 s window after a 10 s settle
  with no input, printed as a share of Windows' 15.6 ms scheduling tick —
  non-blocking, since the `windows` job already is. Auditing every
  suspect the task named turned up one real gap and otherwise a clean
  bill: the named-pipe single-instance server blocks on `ConnectNamedPipe`
  rather than polling; the menu strip has no timer; Terminal's cursor
  blink and foreground-job tracking already stop the moment a window is
  unfocused or idle; the shared text-field caret (gpui-component's
  `blink_cursor.rs`) already parks after 2 s of no interaction. Weather's
  once-a-minute "keep the clock faces current" redraw has no
  `window.is_window_active()` guard the way Clock's own ticker does, so it
  is a real (if infrequent) wakeup — left open rather than threading a
  `Window` handle through its three call sites without being able to
  compile-check the result locally. Whatever idle CPU the new number still
  shows is most likely `gpui_windows` itself (its DirectComposition present
  loop): upstream Zed code, not vendored into this repo the way
  `gpui_linux` was for ADR 0013's own idle-frame fix, so not something this
  pass could patch — see `docs/parity.md` WIN-OS-11.
- **Launch time, measured, not yet cut.** The same script now times
  process start to a visible window and prints it per app. This pass's own
  startup code (the named-pipe hand-off probe, settings reads) is small
  and synchronous already; the time is believed to be mostly
  `gpui_windows`'s own Direct3D/DirectComposition device creation and
  DirectWrite font enumeration, again outside this repo's vendor tree.
  Cutting it toward Lulo's Linux figure (~0.1 s) would need the same kind
  of fork `gpui_linux` already is, which is its own project, not a fix
  inside this one — see WIN-OS-12.
- **Foreground, fixed.** Weather and Terminal opened behind an
  already-open window on the laptop while every other app came forward.
  Real: both apps' view constructors do real work before the window is
  ready to show (Terminal's `Session::spawn` opens a PTY and starts the
  shell; Weather reads cached forecasts from disk) where every other app's
  constructor is a cheap settings read, and Windows only auto-foregrounds
  a brand-new window for a short grace period after process start.
  `rmac_ui::window::open_app_window` (Terminal, via `boot_app_instance`),
  Weather's own window-open closure, and — defensively, since every app
  relies on the same `cx.activate(true)`, confirmed a no-op on Windows by
  reading `gpui_windows::platform.rs` — Calculator, Clock and Preview now
  call `window.activate_window()` explicitly once the view is built,
  rather than relying on that window of leniency, behind `#[cfg(windows)]`.
  That gate is deliberate, not cosmetic: a first version called it
  unconditionally, reasoning it would be a harmless no-op on Linux the
  way `cx.activate` already is; it is not — `gpui_linux`'s own
  `activate()` sends a real `xdg_activation_v1` request. Calling it
  unconditionally would have been a real Linux behaviour change this
  task explicitly ruled out, so it stays Windows-only even though two
  separate CI runs on this branch (one with the call unconditional, one
  Windows-only) showed identical `runtime.yml` numbers either way —
  `desktop-paint`/`menu-dismiss` both failed on both runs, which is
  pre-existing, already-tracked flakiness (`desktop-paint` is DESK-12's
  own documented "icon in the very first captured frame" follow-up, not
  new) rather than anything this change caused. The comparison still
  earned keeping the gate: it removes a real, provable difference in
  what Linux apps now do on open, whether or not these two checks happen
  to notice it.
  `launch_smoke.py --foreground-check` reproduces the two-windows-open
  scenario in CI (its own per-app loop otherwise kills each app before the
  next starts, so could not have caught this) — see WIN-OS-13.
- **Clock's alarms and timers, delivered for real.** `schedule::apply` on
  Windows was an unconditional error, so the Alarms tab showed a permanent
  red "not available on this platform yet" banner and nothing ever rang.
  It now mirrors the Linux systemd design with Task Scheduler in
  `schedule.rs`: each due alarm, snooze and running countdown becomes one
  `schtasks`-created task that runs `rmac-clock --ring-due`, the same
  headless ring process Linux already uses and which already played the
  alert sound in a loop on any non-Linux target — only the trigger was
  missing. Rings happen whether or not Clock itself is running, matching
  "real delivery" rather than "only while the app happens to be open". The
  banner, now only shown when scheduling genuinely fails, no longer uses
  `mac::danger()` red; a possibly-silent alarm is not a destructive
  failure and reads as quiet secondary text instead. A toast alongside the
  sound is still open — `ToastNotificationManager::CreateToastNotifierWithId`
  needs an AUMID, which an unpackaged dev build has no shortcut/MSIX
  identity to provide; that is phase 3's job, not this pass's — see
  WIN-OS-14.
- **Text Editor's clipped format bar, fixed (not Windows-specific).**
  `text-editor.png` showed the alignment buttons cut off at the default
  586 pt window width. The cause was in shared layout code, not a Windows
  seam: `Size::Small`'s default 12 pt side padding alone made the B/I/U/S
  and alignment buttons wider than the Mac's 22 pt segment buttons, before
  any gap or label width was even counted. Both segments now set
  `.px(px(6.0)).min_w(px(24.0))` directly — `rmac_ui::Button` implements
  `Styled` but has no `.compact()` the way gpui-component's own
  `ButtonGroup` does, which a real `cargo check` on the reference laptop
  caught before this landed — and the Mac's own 2 pt inner gap, which
  brings the bar back inside 586 pt on every platform — see
  `docs/parity.md` UIA-07.

Not attempted this pass (done since on `op/gpui-windows-idle`, ADR 0025, which
vendors `gpui_windows`, parks idle windows and cuts CI launch to about 95 ms),
and recorded rather than silently skipped: patching
`gpui_windows` itself for idle CPU or launch time (would need forking it the
way `gpui_linux` already is, ADR 0013 scale, its own project); a Windows
toast for Clock's alarms (needed an AUMID this build could not provide yet --
`op/win-installer`, below, now sets one for every app, both on its Start Menu
shortcut and at process start, but wiring `ToastNotificationManager` itself
is still open).

## Installer

(`op/win-installer`.) Phase 2's plan called for "MSIX packaging, signing and `.appinstaller`
in CI"; MSIX needs a trusted signature just to install, which blocks it until the owner's
signing account exists (see "What the owner must provide"), so this pass builds the
double-click installer people can use today and leaves MSIX for later, as a packaging
format rather than a hard requirement.

### Tool choice: a WiX MSI, not Inno Setup or MSIX

**Requirements:** a per-user install (no admin prompt) to `%LOCALAPPDATA%\Programs\Lulo`,
upgrade in place, a real Add/Remove Programs uninstall entry, and a build that a single
declarative source file plus one generated fragment can drive from `apps.json` -- so that
Files and System Settings, which another branch is adding, are a one-line addition
(append to `packaging/windows/apps.json`), not a installer-script edit.

- **MSIX:** ruled out by the task itself and confirmed while reading the ecosystem: an
  unsigned or self-signed MSIX needs Developer Mode or a manually trusted certificate to
  install at all, which fails the "double-click, no prior setup" bar this pass is for. It
  stays the phase-2 target once Azure Trusted Signing exists (see "Signing" below); nothing
  here forecloses it.
- **Inno Setup:** free to use (its own permissive "Inno Setup License"), and its output
  installer is an ordinary unsigned `.exe` SmartScreen already warns about the same way the
  existing preview zip does, so it would work. It loses to WiX for two reasons rather than
  one: its `.iss` script format is written procedurally (`[Files]`/`[Icons]` sections with
  per-line directives), which is harder to generate safely from a list of apps than XML
  elements with one attribute per fact; and the installer `.exe` it produces embeds Inno
  Setup's own compiled Pascal runtime (the wizard UI and installation engine) in every copy
  Lulo ships -- legally fine under its license, but a second kind of third-party code
  shipped in the product for no benefit WiX's approach does not need.
- **WiX Toolset (chosen, pinned to v5.0.2; `Package/@Scope`, `StandardDirectory`, the core
  `ShortcutProperty` element, the implicit feature and `MajorUpgrade`'s default strategy
  this installer relies on were all introduced in v4/v5 and confirmed unchanged in v5.0.2,
  v6 and v7's own compiler source):** produces a plain MSI, a standard Windows Installer
  database with no code of WiX's own baked into it -- `msiexec.exe`, part of Windows, reads
  the tables WiX compiled and does the actual install. `Package Scope="perUser"` is exactly
  the no-admin install this needs, declarative XML plus a CLI
  (`wix build Product.wxs Apps.wxs ...`) is easy to generate from `apps.json` and run from
  CI non-interactively, `MajorUpgrade` with a fixed `UpgradeCode` gives upgrade-in-place for
  free, and MSI's own Add/Remove Programs integration needs no extra code to appear in
  Settings ▸ Apps.

**Why v5.0.2, not the current v7 release:** found only by actually running `wix build` in
CI, not by reading license text ahead of time -- WiX 6.0.0 (April 2025) added an "Open
Source Maintenance Fee" (OSMF) gate on its own pre-built binaries: `wix.exe` refuses to run
at all ("WIX7015") until a `-acceptEula wix7` flag (or a `.wixproj`
`<AcceptEula>` property) is passed. The source stays MS-RL (OSI-approved, confirmed from
`OSMFEULA.txt` in the WiX repository: "the Fee is not a license fee... the Software's
source code is licensed to User under the OSI License"), and self-compiling WiX from that
source is explicitly carved out of the Agreement entirely ("User may independently compile
binaries from the Software's source code without this Agreement"); only FireGiant's own
pre-built binary release is gated, and only a *User* "as part of revenue-generating
activities" with annual gross revenue at or above US$10,000 owes an actual Fee -- smaller
users and non-commercial use are exempt, and accepting the EULA (which the flag alone does)
does not by itself create a payment obligation either way. Still, deciding whether Lulo
qualifies for an exemption is the owner's call, not this pass's to make by passing a flag on
their behalf. **v5.0.2** (October 2024) is the newest release that predates OSMF entirely --
confirmed by diffing its GitHub Release notes against 6.0.0's and reading its own
`Compiler_Package.cs` for every element this installer uses (above) -- so it needs no EULA
decision at all. If the owner later wants the newest WiX (performance fixes, bug fixes),
that is a one-line version bump plus an explicit `-acceptEula` choice once gross revenue is
known, not a blocker today.

**cargo-deny and the repository's licence policy:** neither tool is ever a Cargo
dependency -- both are external build tools invoked from CI (`wix build` / `ISCC.exe`), so
`cargo deny check licenses` (`deny.toml`'s `[licenses]` table) never sees either one; this
is the same posture as using `dpkg-buildpackage` and `debhelper` (GPL) to build the Debian
packages in `scripts/linux/`, or GCC to compile C code -- a build tool's licence does not
attach to its output unless the tool's own source or binary ships inside that output.
That *is* worth checking for an installer, since unlike a compiler, both WiX and Inno Setup
*can* end up redistributed: WiX's MSI format contains none of WiX's own code (just standard
Windows Installer tables and Lulo's own exes), so its MS-RL licence (confirmed from
`LICENSE.TXT` in the WiX repository, not assumed) never attaches to anything Lulo ships,
and the OSMF Agreement above says the same of its own Fee ("does not limit User's ability
to access, modify, or distribute the Software's source code or self-compiled binaries").
Inno Setup's compiled installer stub, by contrast, is Inno Setup's own code, embedded by
design -- permitted under its licence, but the comparison above is why WiX was still
preferred. The one new build-time Cargo dependency this pass adds, `winres` (MIT, confirmed
from its published crate metadata, so already on `deny.toml`'s allow list with no new
exception needed), is gated `[target.'cfg(windows)'.build-dependencies]` in each app crate
(`crates/rmac-windows-resource-build`), so it is never resolved, let alone compiled, for the
Linux or macOS dependency graph -- the same seam ADR 0025 uses for `gpui_windows`.

### What the installer does

- **Installs every Lulo Windows app** (Calculator, Notes, Text Editor, Preview, Clock,
  Weather, Terminal, and now Files and the Lulo layer -- `lulo-session.exe` and
  `lulo-shell.exe`, ADR 0023 phase 3, merged from `op/win-shell`) to
  `%LOCALAPPDATA%\Programs\Lulo`, one `<Component>` per app generated from
  `packaging/windows/apps.json` by `generate_apps_wxs.py` into `Apps.wxs`. System Settings
  (still in progress on another branch, WIN-OS-22) becomes a one-line addition to that JSON
  file once it builds on Windows; nothing in `Product.wxs`, the generator, or this CI
  changes. Two fields past the original schema cover binaries that are not a plain
  one-shortcut-one-exe app: `"shortcut": false` installs a helper exe (`lulo-shell.exe`,
  which `lulo-session.exe` launches itself) with no Start Menu entry of its own, and
  `"icon_svg"` names artwork outside the usual per-app Linux `.desktop` icon set (the Lulo
  layer's own mark, `assets/icons/lulo.svg`, for the single "Lulo" shortcut that runs
  `lulo-session.exe`). An entry with neither `app_id` nor `icon_svg` gets no
  `ShortcutProperty` -- `lulo-session.exe` has no `app_id` because nothing in
  `rmac-win-shell` sets a matching `AppUserModelID` at process start yet (unlike the seven
  apps below); giving its shortcut one anyway would be a mismatched identity, worse than
  none.
- **Start Menu shortcuts with real icons, for the seven original apps.**
  `scripts/windows/make_icons.sh` rasterises each app's existing artwork
  (`packaging/rmac-apps/icons/org.rmac.<App>.svg`, the same files `scripts/build-icons.py`
  writes for the Linux `.desktop` icons -- never Apple's -- or, for the Lulo layer's own
  shortcut, `assets/icons/lulo.svg`) into a multi-resolution `.ico` with ImageMagick
  (preinstalled on `windows-latest`), before the apps are built. `rmac-windows-resource-build
  ::embed` (called from a one-line `build.rs`) embeds that icon, plus
  `FileDescription`/`ProductName`/`CompanyName` "Lulo" and the exact Cargo version, as the
  exe's own Win32 resources, for Calculator, Notes, Text Editor, Preview, Clock, Weather and
  Terminal -- so both the exe itself and every shortcut to it (which inherits an exe's icon
  when none is set explicitly) show the right artwork, with one thing embedding it rather
  than two copies to keep in sync. A build that skips the icon step (plain local
  `cargo build`) still succeeds, with the platform's default icon and a `cargo:warning`
  naming why. **Files and the Lulo layer do not call `embed`** (`crates/finder/build.rs`,
  `crates/rmac-win-shell/build.rs` are deliberately empty): a real `cargo build --release`
  of all nine binaries together hit CVTRES error CVT1100, "duplicate resource.
  type:VERSION, name:1, language:0x0409", linking `rmac-files.exe` -- `resource.lib`
  (`rmac-windows-resource-build`'s own output) listed twice in the linker command for a
  reason this pass could not pin down in the time it had (gpui's own embedded resource, the
  only other one in the graph, links once, correctly, and nothing else should be requesting
  a native `resource` lib by that name; adding a unique `package.links` key to every crate
  that calls `embed`, Cargo's documented fix for a build script's native-link output being
  applied more than once, made no difference). Files and the Lulo layer still install, get
  their shortcuts (where due) and run correctly; they keep the platform's default icon and
  no `FileDescription`/`CompanyName` until this is understood -- see "What is left".
- **AppUserModelID**, so Clock's alarms (WIN-OS-14) and every app's taskbar grouping and
  jump lists can work: each Start Menu shortcut's `ShortcutProperty` sets
  `System.AppUserModel.ID` to `Lulo.<App>` (`generate_apps_wxs.py`'s `aumid()`), and each app
  now calls `SetCurrentProcessExplicitAppUserModelID` with the identical string at startup
  (`rmac_ui::app_menu::install`, gated `#[cfg(windows)]`) -- both sides must agree for
  Windows to treat a toast, taskbar group or jump list as the app's own, so a test
  (`crates/rmac-ui/src/app_menu.rs`'s `aumid_tests`) pins all seven apps' exact strings.
  Wiring an actual toast (`ToastNotificationManager`) for Clock is still open.
- **File associations, offered but not forced:** Text Editor for `.txt`/`.md`/`.rtf`,
  Preview for `.pdf` and images. Each app registers itself under
  `HKCU\Software\Classes\Applications\<exe>.exe` (`FriendlyAppName`, `shell\open\command`,
  `SupportedTypes`) -- the standard Windows mechanism for appearing in a file's "Open with"
  list without claiming the default handler, so a user's existing default association is
  never overwritten. `HKCU`, not `HKCR`, is written explicitly rather than relied on via
  implicit per-user redirection, so the result does not depend on exactly how MSI's
  `ALLUSERS`/`Scope` redirection behaves on a given Windows build.
- **An uninstaller in Settings ▸ Apps** comes from the MSI format itself (no extra code);
  it removes every file and registry entry this installer wrote and leaves user documents
  and notes alone (nothing under `Documents` or `AppData\Roaming` is ever a component). Two
  things MSI cannot track on its own, because they are written at runtime, not at install
  time: Clock's `RmacClockAlarm-*` Task Scheduler tasks (`crates/clock/src/schedule.rs`,
  WIN-OS-14) and the Lulo layer's own `HKCU\Software\Lulo\Shell` settings and its opt-in
  `Run` key sign-in entry (`crates/rmac-win-shell/src/win/registry.rs`, WIN-OS-26/27). Three
  immediate `CustomAction`/`RemoveRegistryKey`/`RemoveRegistryValue` entries (`Product.wxs`)
  handle them: `Unregister-ScheduledTask` against the Clock task name pattern,
  `lulo-session.exe --restore-windows-desktop` (run while the exe still exists on disk, so
  the taskbar and work area are always given back even if the layer's own crash/shutdown
  handling never got the chance to), and removing the `Shell` key and the `Run` key's `Lulo`
  value outright. All three are conditioned on `REMOVE="ALL" AND NOT UPGRADINGPRODUCTCODE`
  so an upgrade's own remove-then-install step never touches any of them -- see WIN-OS-28.
- **Upgrade in place:** a fixed `UpgradeCode` (never change it) plus WiX's default
  `MajorUpgrade` strategy removes the previous version's files and installs the new ones in
  one transaction; `ProductCode` stays `*` (a fresh GUID each build), which `MajorUpgrade`
  does not need to be fixed.
- **Version info resources** (`rmac-windows-resource-build`, above): `FileVersion` and
  `ProductVersion` carry the exact Cargo version string (e.g. `0.9.0-beta.1`); Win32's
  numeric `VS_FIXEDFILEINFO` fields, which have no room for a pre-release tag, carry its
  numeric prefix (`0.9.0.0`). The MSI's own `Version` property is the same numeric prefix
  (`build-installer.sh` strips the suffix); the pre-release tag lives in the installer's
  filename (`Lulo-Setup-0.9.0-beta.1-x64.msi`) instead.

### Signing

Not yet: Azure Trusted Signing needs an account the owner has not created yet (ADR 0023's
"What the owner must provide"). The CI step (`azure/trusted-signing-action`) is wired in and
conditioned on `vars.RMAC_TRUSTED_SIGNING_ACCOUNT` being set, with `continue-on-error: true`
so a transient signing failure never hides the unsigned installer everyone can already use
-- it never fakes a signature, and every installer built today is honestly unsigned
(SmartScreen will warn, same as the existing preview zip's README already tells people to
expect).

### CI

`windows-preview.yml` (every push to that branch, and `workflow_dispatch`) and
`release.yml` (tagged releases, mirroring the Linux `build-amd64`/`attach-release` shape,
but listed in `attach-release`'s `needs` without being required to succeed -- a build
failure shows red on that one job without blocking the Linux release, the same pattern
`keyring` already uses there) both: generate icons, build the apps (now including Files and
the Lulo layer), build the MSI, sign it if the secrets exist, then on `windows-latest` run
`scripts/windows/installer_smoke.py`, which installs silently, checks every app's exe,
Start Menu shortcut and "Open with" registration exist, launches Calculator from its
shortcut and checks the process starts, seeds a dummy `RmacClockAlarm-*` task (Clock itself
creates one only once an alarm is actually scheduled), uninstalls silently, and checks the
install directory, the shortcuts, the registry entries, the Add/Remove Programs entry and
that scheduled task are all gone. (One check is a warning rather than a hard failure: an
Add/Remove Programs entry for "Lulo" right after install, which MSI's own
RegisterProduct/PublishProduct standard actions write with no authoring from this installer
-- GitHub's hosted Windows runner's non-interactive logon showed it consistently absent
immediately after an otherwise fully working install in CI run 37689979220, which looks
like a runner-session quirk rather than a real defect; the uninstall-time absence check
still runs and still matters whenever an entry was there to begin with.) The installer is
uploaded as the `lulo-windows-installer` (preview) / `windows-installer` (release) artifact
alongside the existing unsigned exe zip.

### What is left

- Real code signing, once the owner's Azure Trusted Signing account exists.
- System Settings, once it builds on Windows (WIN-OS-22, WIN-OS-29's reserved slot): one
  entry in `packaging/windows/apps.json`.
- A real toast for Clock's alarms (`ToastNotificationManager`) now that an AUMID exists.
- An `AppUserModelID` for `lulo-session.exe`'s own shortcut, once `rmac-win-shell` sets a
  matching one at process start (WIN-OS-28).
- The CVT1100 duplicate-resource link failure that keeps Files and the Lulo layer from
  embedding an icon or version info ("What the installer does" above). Worth real
  investigation (`cargo build -v` to see every build script's exact output, trying
  `embed-resource` in place of `winres`, or a minimal reproduction outside this workspace)
  before trying another blind fix.
- A standing "Restore Windows taskbar" Start Menu shortcut (WIN-OS-26); today's fix only
  runs that restore automatically during a real uninstall, not as an anytime escape hatch.
- MSIX packaging as an additional distribution format, once signing exists -- this
  installer does not block it.
- Whether to move WiX past v5.0.2 is the owner's call ("Why v5.0.2" above): it needs an
  `-acceptEula wix7`-style decision, which depends on Lulo's gross revenue, not on anything
  this branch can determine.

## Phase 2d as built (branch `op/win-phase2d`)

Branch `op/win-phase2d` brought Files (`crates/finder`) to Windows and
scoped System Settings (`crates/system-settings`), continuing from `integ`
with CI and the owner's Windows laptop (SSH, session 0, serial builds) as
the only verification available.

- **Files, fully ported and verified.** Favourites' Desktop/Documents/
  Downloads resolve through `SHGetKnownFolderPath` rather than a guessed
  home-relative path (`crates/finder/src/places.rs`); Locations lists real
  drives via `GetLogicalDrives`/`GetVolumeInformationW`/`GetDriveTypeW`
  (`crates/rmac-mounts/src/inventory.rs`), with free space from
  `GetDiskFreeSpaceExW`; Open uses `ShellExecuteW` (the user's default app)
  and Show in Explorer shells out to `explorer /select,` (`crates/rmac-
  portal/src/open.rs`); Move to Bin goes through the real Recycle Bin — the
  existing `trash` dependency already supported this once the crate graph
  compiled; colour tags live in an NTFS alternate data stream
  (`<path>:lulo.tags`), the simplest honest substitute for Linux's xattr
  tags; Cut/Copy/Paste use the CF_HDROP clipboard format and Explorer's own
  "Preferred DropEffect" format to distinguish cut from copy
  (`crates/rmac-pasteboard`); the crash-safe copy/move journal and Undo use
  real Windows file locking (`std::fs::File::lock`/`try_lock`, already
  proven in `rmac-recent-documents`) and `MoveFileExW` for a same-volume
  rename, with Win32's `HRESULT_FROM_WIN32` unpacked back into the matching
  `std::io::ErrorKind` so existing `AlreadyExists`-dependent logic keeps
  working on Windows too. File watching uses `notify`'s existing
  `ReadDirectoryChangesW` backend rather than any polling of this crate's
  own — the one known cost (a watcher thread waking roughly ten times a
  second, WIN-OS-15, already documented and already inside the idle gate's
  one-tick budget) is shared with every other app that watches anything,
  not something this phase introduced.

  The one genuine, non-cosmetic bug, found only by running the real test
  suite on the laptop: `sync_copied_tree`'s `File::open(path)?.sync_all()`
  failed every real copy with Access Denied, because Windows'
  `FlushFileBuffers` (what `sync_all` calls) needs a write-access handle,
  unlike Unix's `fsync` — a read-only `File::open` handle cannot call it.
  Opening with `.write(true)` on Windows only fixed essentially all of the
  roughly 20 failing tests this had been masquerading as. `cargo clippy
  --all-targets -- -D warnings` is clean and every unit test passes on the
  laptop for `rmac-finder`, `rmac-mounts`, `rmac-portal`, `rmac-search`,
  `rmac-app-launch`, `rmac-pasteboard`, `rmac-archive` and `rmac-quick-
  look`. Honest, test-covered stubs remain for ejecting a drive, adding to
  the Dock (Windows has none), an atomic two-way rename-exchange (no single
  Windows syscall does this the way Linux's `RENAME_EXCHANGE` does),
  browsing/restoring from the Bin through Finder's own list rather than
  Explorer's, and archive expand/compress. Get Info hides the Permissions/
  Owner/Group rows rather than fabricate Unix-style values, and a Windows
  "identity" (used to notice a tracked item survived elsewhere) is a
  canonicalised path rather than a `(device, inode)` pair, since
  `MetadataExt::file_index`/`volume_serial_number` need the unstable
  `windows_by_handle` feature this toolchain does not have — see
  `docs/parity.md` WIN-OS-18 through WIN-OS-20.

  A pre-existing, Finder-adjacent gap turned up while porting the crate
  graph: `rmac-shortcuts`' Unix-socket IPC (the mechanism that wakes the
  launcher/app-drawer/quick-settings/notification-center surfaces) now
  compiles on Windows behind `#[cfg(not(windows))]`, but dispatch itself is
  an honest "not available on Windows yet" stub there; only the lock action
  is real on Windows today, since it calls `lock::request()` directly.
  Planned as `RegisterHotKey`/a keyboard hook in phase 3 — see WIN-OS-21.

- **System Settings, scoped but not built.** `navigation.rs` gained a
  `#[cfg(target_os = "windows")]` pane list restricted to Appearance,
  Wallpaper, Sound, Displays, About This PC and Keyboard Shortcuts, and
  `Cargo.toml` moved roughly 35 Linux-only service crates (network,
  Bluetooth, users, printers, sharing, VPN, accounts, …) to
  `[target.'cfg(target_os = "linux")'.dependencies]`. `cargo check -p
  rmac-system-settings` on Windows still fails with about 700 errors across
  70-plus files in `controller/`, including shared infrastructure
  (`state.rs`, `initialization/*`, `view_helpers/*`) and files belonging to
  panes this phase intended to *keep* (`sound.rs`, `wallpaper/render.rs`,
  `displays/brightness.rs`) that assume a Linux service is always present.
  That scope matches this document's own 25-to-40-agent-day phase-4
  estimate for System Settings; delivering a mechanical gate pass that
  compiles but is entirely unverified was judged worse than stopping here
  with the real scope on record — see WIN-OS-22. System Settings on
  Windows remains future work, not a phase-2d deliverable.

- **Clock's idle gate, made robust to its own legitimate redraw.** CI
  intermittently failed the idle gate on Clock (run 37633070594 at 19.03
  ticks, run 37657567719 at 28.05 ticks, against a budget of one) even
  though nothing regressed: the World Clock tab schedules a real redraw
  for the next minute boundary while its window is active, matching the
  Mac's minute-precision display, and the 20 s idle window has roughly a
  one-in-three chance of containing that boundary; both runs showed the
  same small wake-source shape (frame idle/vsync tick/message 0x0403),
  just a different tick cost depending on the runner's own load. Rather
  than loosen the gate generally, `scripts/windows/idle_gate.py` gained a
  small `PER_APP_BUDGET_TICKS` table with one entry (`rmac-clock: 48.0`,
  about double the higher sample and documented inline with both runs'
  numbers) — still roughly two orders of magnitude under the ~1,280 ticks
  a real regression (a poll, or a failure to re-park) would show across
  the whole window. Every other app, and Clock itself if it ever ticks
  every second or fails to re-park, still fails at the standard one-tick
  budget — see WIN-OS-17.

## Phase 2e as built (branch `op/win-settings`)

Branch `op/win-settings` brought System Settings to Windows (WIN-OS-22)
and made Clock's minute tick cheap (WIN-OS-17), with CI and the owner's
laptop (SSH, session 0, serial builds in `E:\lulo`) as the only places to
verify.

- **System Settings, restructured rather than gated file by file.** Lulo
  OS's Settings (`src/controller`, about 25,000 lines) drives a Linux
  service on nearly every pane and keeps all of them in one `Settings`
  struct, so phase 2d's 700 errors were the shape of the design, not 700
  small gaps. `main.rs` now puts that whole controller, and every
  top-level module only it uses (connectivity, focus, power, input,
  notifications, the system snapshot, the service-update plumbing, …),
  behind `cfg(unix)`, so Linux and the macOS developer build compile
  exactly what they did before. Windows gets its own small Settings in
  `src/win` (about 3,400 lines with its tests) that shares the
  platform-neutral pieces as they are: the pane inventory
  (`navigation.rs`), the theme-store logic (`appearance.rs`), the
  wallpaper store, preview and validation (`shell_settings.rs`), and the
  measured geometry and colours (`controller/settings_style.rs`, included
  by `#[path]`), so its window, sidebar, toolbar and grouped forms are
  drawn to the same numbers as on Lulo OS. Everything Windows-specific
  goes through one facade, `win::host::Host` (`about`, `displays`,
  `output_volume`, `set_desktop_wallpaper`); `WindowsHost` answers it with
  Win32 calls and tests with fixed facts. Nothing in it polls: each pane
  reads once when it opens, and volume and displays are read again when
  the window becomes active.
- **The six panes, all real.**
  - *Appearance:* Auto, Light or Dark and the accent colour (Multicolour
    plus the eight Mac accents), saved through the same authoritative
    theme-store path Lulo OS uses. Every Lulo app now follows the file on
    Windows too: `rmac-ui` watches it with one thread parked in
    `ReadDirectoryChangesW` (`file_watch_windows.rs`; notify's Windows
    backend wakes ten times a second, WIN-OS-15), so a change repaints
    every open Lulo app at once. Auto follows Windows' own app mode:
    `rmac-appearance-portal` reads `AppsUseLightTheme` and watches it with
    `RegNotifyChangeKeyValue`, again a parked thread.
  - *Wallpaper:* the Lulo gallery (artwork, the user's photos, gradients)
    and placement, saved in `rmac-shell-settings` for the Lulo shell; the
    Windows desktop changes only on "Use as Windows Desktop Picture",
    which hands Windows the packaged JPEG, the user's own JPEG/PNG/BMP, or
    a PNG rendered once into Lulo's cache, sets `WallpaperStyle`/
    `TileWallpaper` for the placement and calls
    `SystemParametersInfoW(SPI_SETDESKWALLPAPER)`. The preview build now
    ships the wallpapers beside the apps (`rmac-wallpaper` looks in
    `<exe>\wallpapers` on Windows).
  - *Sound:* Lulo's alert sound (Alert, Error, Notification), played on
    choosing, and the default output device's name and volume from Core
    Audio (`IAudioEndpointVolume`), read-only.
  - *Displays:* each display's resolution, refresh rate, Windows scale and
    the resulting desktop size (`EnumDisplayMonitors`,
    `EnumDisplaySettingsW`, `GetDpiForMonitor`; friendly names and the
    built-in panel from `QueryDisplayConfig`), with a button to Windows'
    own Display settings.
  - *General ▸ About This PC:* computer name, maker and model, processor,
    cores and threads, installed memory, graphics adapters, the Windows
    edition, release and build (named Windows 11 from build 22000, since
    `ProductName` still says 10), and each fixed drive's free space
    (`rmac-mounts`).
  - *Keyboard:* the shortcuts Lulo apps answer, shown with Ctrl.
- **Menus and keys.** The Windows module registers only the six panes'
  `system_settings::Show…` actions, so the shared menu table's View menu
  lists exactly those; ⌘[ ⌘] ⌘F ⌘W ⌘Q ⌘M go through `rmac_ui::bind_keys`
  and so answer Ctrl. The Lulo layer's Lulo ▸ System Settings… now opens
  this exe when it sits beside the layer (Windows' Settings otherwise).
- **Proof.** `rmac-system-settings` and `rmac-appearance-portal` joined
  the Windows CI package list (Clippy `-D warnings`, 19
  Settings unit tests on Windows), the app build, `launch_smoke.py`
  (window, menu strip with Alt, App ▸ About, Ctrl+N), the idle gate,
  `windows-preview.yml` (with Lulo's wallpapers beside the exes) and the
  installer's app list (`packaging/windows/apps.json`, its Start Menu
  shortcut and exe icon). CI run 37711054714: Settings visible
  203 ms after start, 0.00 ticks over the 20 s
  idle window.

- **Clock's minute tick, made cheap.** Every World Clock redraw
  re-rasterised the whole map on the CPU (the land mask, then each
  pixel's day or night colour: 47 ms on the laptop's Ryzen 3 in a debug
  build at 1×, 71 ms at 1.25×), uploaded a new 2–3 MB texture and drew two
  frames, one for the time and one when the late map arrived. To measure
  it rather than wait for a minute boundary, `RMAC_CLOCK_WORLD_TICK_MS`
  shortens the redraw period and `launch_smoke.py --world-tick-check`
  charges the idle window's CPU to the ten redraws inside it: 15.83 ticks
  per minute tick before (run 37678918002, 20 frames for 10 ticks). Now the
  land, ocean and meridians are rasterised once per size; each minute's
  map is that layer with the night side darkened through a lookup table
  and the terminator drawn in (`map::paint_night`, 9 ms in a debug build
  on the laptop at 1×), made on a background thread a
  minute ahead and swapped in on the tick, so a tick draws one frame and
  re-rasterises nothing. (A vector night overlay was tried first: drawing
  a path costs GPUI a full-window multisampled layer, and per-column
  rectangles cost more than the image.) After: 8.11 ticks per tick in
  all, of which 0.90 are Clock's own and 7.21 are WARP drawing the one
  frame (run 37711054714). That 7 to 9 ticks is the runner's software
  rasteriser drawing any full frame of a 1024 × 768 window: removing the
  map, the cards, the pins and the clock faces, separately and together,
  left it at 9.2 to 10.5 ticks per frame (profiling run 37703039321). WARP
  runs on the same system thread pool as GPUI's background tasks, so
  `gpui_windows`' wake trace now reports each pool task's CPU (ADR 0025)
  and `launch_smoke.py` counts the pool's remaining time as the
  rasteriser's; on a real PC that is the GPU's work. The idle gate now
  judges each app on its own CPU, fails a World Clock tick over 3 own
  ticks (`--max-world-tick-ticks 3`), and Clock's special idle budget
  drops from 48 ticks to 3, room for the one minute boundary a 20 s window
  can hold. 3 rather than 2 because a reading swings by about a tick:
  thread times move a whole tick at a time and WARP's share is a
  difference of two such sums (four runs read 0.90, 0.90, 1.10 and 2.00
  per redraw); the old map re-rasterisation alone was 4 to 5 own ticks.
  The Lulo bar (`lulo-shell`), whose clock also redraws once a minute,
  gets 2 (run 37713970007 caught it at 2.00 before WARP was left out of
  its reading too, 1.00 since). Lulo OS
  runs the same Clock code, so its minute tick sheds the same
  rasterisation; its runtime `idle-cpu` soak keeps Clock's window
  inactive, where it does not tick (0.01 % before and after, runs
  37678917994 and 37711054692).

What is stubbed or left out on Windows, honestly rather than faked: every
pane that needs a Linux service (network, Bluetooth, users, printers,
sharing, VPN, accounts, focus, notifications, power, input devices, date
and time, language, login items, privacy, updates, Lulo Intelligence) is
not compiled or listed; the output volume, display modes and Windows
scale are read-only (Windows' own controls change them); Lulo's alerts
play Windows' system sounds at the Windows volume, so there is no alert
volume slider (WIN-OS-07); per-display wallpapers and wallpaper tinting
need the Lulo shell's renderer on Windows; Settings has no compact
(single-column) layout and no Search results beyond the six panes' own
names and terms.

## Phase 3 design: the Lulo layer

Phase 3 puts Lulo's shell on top of the Windows desktop as a layer the user
switches on: `lulo-session.exe` turns it on, the Lulo menu's Turn Off Lulo (or
`lulo-session --stop`) turns it off, and the normal Windows desktop comes back
either way. Explorer stays the Windows shell (decision 1). The decisions:

**One process for the surfaces.** `lulo-shell.exe` (crate `rmac-win-shell`)
draws the menu bar, the Dock, Spotlight and the menu panels as windows of one
GPUI process. On Lulo OS each surface is its own process (`shell/bins/*`), but
there every one is a Wayland layer surface with its own small client; on
Windows each GPUI process pays for its own Direct3D device, DirectWrite font
collection and swap chains, so one process keeps start-up and memory small on
8 GB machines. `lulo-session.exe` is a tiny watchdog without GPUI: it runs the
shell, waits on its process handle (no CPU), restores the desktop whenever the
shell stops and restarts it after a crash (at most three times a minute).
`shell/bins/rmac-menubar`, `rmac-dock` and the Linux Spotlight are built on
`wlr-layer-shell`, niri, D-Bus and Linux status services all the way through
(each depends on `rmac-compositor-niri`, `zbus` and the NetworkManager, BlueZ
and PipeWire crates), so the Windows surfaces reuse the shared pieces under
them instead: `rmac-ui`'s components (`ContextMenu`, `TextField`,
`svg_icon`), the `mac` design tokens, the Lulo app icons and the shell's
glyphs, `rmac-app-menu`'s menu model and the Linux bar's measurements (24 pt
bar, 48 pt Dock tiles with the Mac's 10/64 shelf padding). The platform seam
is the crate boundary: `rmac-win-shell` is all `cfg(windows)` behind a model
module that every platform builds and tests; on Linux and macOS its two
binaries only say that they are for Windows. Nothing in `shell/` changes.

**Surfaces.**

| Lulo OS | Windows |
|---|---|
| Menu bar: a `wlr-layer-shell` surface with an exclusive zone | A `WS_EX_TOOLWINDOW` + `WS_EX_TOPMOST` + `WS_EX_NOACTIVATE` window registered as a top AppBar (`SHAppBarMessage` `ABM_NEW`, `ABM_QUERYPOS`/`ABM_SETPOS` on `ABE_TOP`), so Windows takes its strip out of the work area and maximised windows stop below it. It never takes the keyboard from the app in front. |
| Dock: a layer surface along the bottom | A bottom AppBar the Dock's height (67 pt); the window is the shelf alone, centred in the strip, so no clear area covers app windows. A floating, auto-hiding Dock comes with Desktop & Dock settings. |
| Menus: layer pop-ups | A clear, activatable panel covering the screen below the bar: the menu draws at its title, the rest of the panel catches the click that closes it (as on the Mac, the click does not reach the window below), and the panel takes the keyboard (↑/↓, Return, Esc) while it is open. Closing gives the foreground back to the app in front, before a command is sent to it. |
| Spotlight: ⌘Space through the portal shortcut | `RegisterHotKey(Alt+Space)`. Win+Space is Windows' input-language switch and cannot be registered; Alt+Space only opens a classic window's system menu, which the ⌘-key hook (next slice) will route. The hotkey lands on the hook thread's queue, which also gives Lulo the right to take the foreground. The window is made hidden at start-up, so it shows at once. |

Every Win32 call that sends messages to Lulo's own windows (moving,
showing, hiding, the foreground) runs from a GPUI task outside any app update,
as `gpui_windows` does for its own `SetWindowPos`: GPUI's window procedure
calls back into the app, which must not be borrowed at that moment.

**App menus over a named pipe.** On Lulo OS the bar reads each app's
`org.rmac.AppMenu2` over D-Bus. On Windows the bar serves
`\\.\pipe\lulo-menubar-<user>` (no remote clients, default same-user
security) and sets the manual-reset event `Local\lulo-menubar-<user>` once
the pipe exists. Every Lulo app (`rmac-ui`'s `menubar_link`) connects at start
when the bar is there, or waits on the event in a blocked thread and connects
when it starts. A synchronous handle serialises a blocking read with a write,
so an app opens two connections: one it writes (a hello, then its menus) and
one it reads (`activate <action>`, `validate`). The menus are the in-window
strip's (the bold app menu with About, Hide and Quit, the app's menus,
Window, Help) in the same pre-order rows as `org.rmac.AppMenu2`, one line per
message (`rmac_app_menu::pipe`), read back through the same decoder and
limits. The bar asks for `validate` as a menu opens, as `Layout` does on
Linux; the open menu updates when the reply arrives. The bar learns which
process sent the menus from the pipe (`GetNamedPipeClientProcessId`) and
shows them when that process's window is in front. While connected, the app
hides its menu strip; when the bar goes, the strip comes back. The first pipe
instance uses `FILE_FLAG_FIRST_PIPE_INSTANCE`, which also keeps a second Lulo
layer from starting. For a Windows app, whose menus stay in its own window,
the bar shows its name (the executable's `FileDescription`) with Hide and
Quit, and the Window menu (Minimize, Zoom, Close Window); menus read through
UI Automation are a later slice. The desktop itself shows File Explorer's
pair, where the Mac shows the Finder's.

**The taskbar.** The Dock takes its place, so while Lulo runs the taskbar is
set to auto-hide (`ABM_SETSTATE`), which gives its strip back to the work
area, and hidden (`ShowWindow(SW_HIDE)` on `Shell_TrayWnd` and each
`Shell_SecondaryTrayWnd`), so it does not slide up over the Dock (this amends decision 1, which hid it
only on request: an auto-hidden taskbar would pop up over the Dock whenever
the pointer reached the bottom edge). Before the
first change the user's own state is recorded under
`HKCU\Software\Lulo\Shell\TaskbarState`; an existing record is kept, because
it holds the setting from before an earlier run that could not restore it.
The record is the only lasting change, and it is undone on every way out:
`lulo-shell`'s Turn Off and quit path, its `WM_QUERYENDSESSION`/`WM_ENDSESSION`
hook (sign-out and shutdown), `lulo-session` after the shell exits or crashes
(it also removes AppBars a dead shell left, by the window handles the shell
recorded), `lulo-session` again at its next start, and
`lulo-session --restore-windows-desktop` by hand. `TaskbarCreated` (Explorer
restarting) hides the new taskbar and registers the AppBars again.
`LULO_KEEP_TASKBAR=1` leaves the taskbar alone. Start, the Win key and the
tray keep working.

**Running apps without polling.** The Dock lists the windows Alt+Tab would
(`EnumWindows`: visible, top-level, not tool windows unless `WS_EX_APPWINDOW`,
unowned, not DWM-cloaked, titled, not Explorer's desktop or taskbar), grouped
by executable (Store apps, which all run in `ApplicationFrameHost.exe`, by
title). It reads the list again only when an out-of-context WinEvent hook
reports a change: `EVENT_SYSTEM_FOREGROUND`, `EVENT_SYSTEM_MINIMIZESTART`/`END`
and `EVENT_OBJECT_DESTROY`/`SHOW`/`HIDE` on top-level windows. The hooks and
the hotkey live on one thread blocked in `GetMessageW`, and repeated events
before the list is read fold into one. Pinned tiles are File Explorer and
the Lulo apps; Windows apps' icons come from `IShellItemImageFactory` on a
background thread. A click activates the app's front window (restoring it if
minimised) or opens the app: Lulo apps from the folder `lulo-shell.exe` is in,
anything else through `ShellExecute` on a COM thread.

**Spotlight's catalogue.** Apps are the Lulo apps plus Windows' Apps folder
(`FOLDERID_AppsFolder`, enumerated with `IEnumShellItems`), which holds the
Start menu's shortcuts and the Store apps alike and opens each through
`shell:AppsFolder\<parsing name>`; the Start menu's `.lnk` files are the
fallback. Files are the names under Desktop, Documents, Downloads, Pictures,
Music and Videos (five levels, 40,000 entries at most, links not followed).
Both are read once on a background thread and again on the next Spotlight
open after `FindFirstChangeNotification` reports a name change in their
folders; nothing indexes or polls. Ranking is Lulo OS Spotlight's tiers
(whole name, prefix, word prefix, substring, letters in order), apps first.
Windows Search (`SystemIndex`) for file contents is a later slice.

**Status items.** Read-only in this slice, each updated only by Windows'
notifications: Wi-Fi from WlanApi (`WlanRegisterNotification`), sound from
the default endpoint's `IAudioEndpointVolumeCallback`, battery from
`PowerSettingRegisterNotification`. A PC without the hardware shows no item.
The clock wakes once a minute, at the minute.

**Autostart.** Opt-in only: the Lulo menu's Start Lulo at Sign-In writes
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run\Lulo` = `lulo-session.exe`
and unticking it deletes the value. The MSIX package's `StartupTask` replaces
it once the package exists.

**The Lulo menu.** About This PC (Settings ▸ About), System Settings…
(Windows Settings until Lulo's runs on Windows), Force Quit… (Task Manager),
Sleep (`SetSuspendState`), Restart… and Shut Down… (`InitiateShutdownW`,
after a confirmation, as on the Mac), Lock Screen (`LockWorkStation`), Log
Out <user>… (`ExitWindowsEx`, after a confirmation), Start Lulo at Sign-In and
Turn Off Lulo.

## Phase 3 slice 1 as built (branch `op/win-shell`)

`rmac-win-shell` builds `lulo-shell.exe` and `lulo-session.exe`; the preview
zip (`windows-preview.yml`) carries both. What works, as designed above: the
bar (Lulo menu, the front app's name and menus, Lulo apps' own menus over the
pipe with live enabled and checked state, Wi-Fi/sound/battery, Spotlight,
the clock), the Dock (pinned and running apps, running dots, click to open or
activate), Spotlight (apps and files, ↑/↓, Return, Esc, click outside), the
taskbar and work area restored on every exit, and opt-in autostart.

CI (`windows` job, `launch_smoke.py --shell` running `shell_smoke.py` after
the app checks) proves on the runner's desktop, with real input: the bar and
the Dock reserve the work area and the taskbar is hidden; a click on the
Dock's Calculator tile opens Calculator and its tile gets the dot; the bar
shows Calculator's menus and its Calculator ▸ About Calculator opens
Calculator's About panel; a maximised Notepad stays between the bar and the
Dock; Alt+Space opens Spotlight, "text editor" finds Text Editor and Return
opens it; `lulo-session --stop` gives back the work area and the taskbar's
visibility and state exactly as before. `idle_gate.py` gates `lulo-shell` and
`lulo-session` like the apps (at most one tick over 20 s idle). Screenshots
are in the `windows-shell-screens` artifact.

Numbers (CI run 37670849555, debug build, 1024×768 WARP desktop): the first
shell window 281 ms after `lulo-session` starts, bar and Dock placed 281 ms
after `lulo-shell` starts; idle over 20 s: `lulo-shell` 0.00 ticks,
`lulo-session` 0.00 ticks. Getting there took two fixes worth knowing:

- `gpui_windows` (ADR 0025): several inactive windows in one process kept
  each other awake. Each one-shot throttle retry's message re-checked the
  other parked windows, whose idle frames then looked "soon after" a frame
  and armed their own retries: about 40 timers a second, 25 ticks over the
  idle window. A retry that fired now arms no second one until the window
  draws (`throttle_retry_spent`).
- A hidden window gets no `WM_PAINT`, so a frame a hidden panel asked for
  kept the vsync thread running. Panels (Spotlight, the menu panel) wait
  cloaked (`DWMWA_CLOAK`) and off screen instead, where they paint and park.
  Surfaces are also told to redraw only when what they show changed, not on
  every window event from other apps.

On the runner the Apps folder lists only six apps and no Notepad (Windows
Server), so Spotlight's Start-menu path is proven there only through the
Lulo apps and those six; the owner's PC shows the full list.

Not in this slice (the next ones, in order): the low-level keyboard hook (⌘
as Ctrl for Windows apps, ⌘Tab, ⌘Space, Win key opening Spotlight); Control
Centre with real Wi-Fi, Bluetooth, sound and brightness controls; the Dock's
right-click menus, drag to reorder and pin, magnification and minimised-window
tiles; Windows apps' menus through UI Automation; Windows Search for file
contents and Spotlight's answers (calculator, conversions); Notification
Centre on `UserNotificationListener`; several monitors and the menu bar on
each; Lulo apps staying open without windows now that a bar can reach them
(WIN-OS-04).

## Phase plan

The goal is "usable on Windows without Linux". Phases are ordered by how much value they
give early. Estimates are in agent-days (one focused agent working with CI) and in calendar
weeks with review and owner testing. They assume one to three agents in parallel and a
Windows test PC from phase 2.

| Phase | What the user gets | Main work | Agent-days | Calendar |
|---|---|---|---|---|
| **1. Seam + CI** (this branch) | Nothing to download yet. Calculator, Notes and TextEdit build and launch on Windows in CI. | cfg seam, Windows CI job, cargo-deny target, this ADR | 1–2 | 1 week |
| **2. The Lulo apps on Windows** (first slice built: shortcuts, menu strip, single instance, choosers, alert sound) | Download one signed installer and get Mac-feel Notes, TextEdit, Calculator, Preview, Clock, Weather, Terminal (ConPTY), Activity Monitor, then Calendar and Mail, in Start, with auto-update. **The first release that is valuable on its own.** | First the phase 1 gaps: `ctrl-` twins for every `cmd-` shortcut, an in-window menu strip, `PlaySound` cues, `ISpellChecker` spelling, Import/Export through GPUI prompts, a named-pipe single instance. Then port the remaining app crates; storage, locking and paths on Windows; open/save panels; printing through the Windows PDF path; toasts for Lulo apps; Credential Manager + loopback OAuth; MSIX packaging, signing and `.appinstaller` in CI; a "Lulo apps" behaviour subset under UI Automation | 15–25 | 4–6 weeks |
| **3. The shell alongside Explorer** (slice 1 built: menu bar, Dock, Spotlight, taskbar handling, see "Phase 3 design") | The menu bar, Dock, Spotlight, ⌘Tab, Control Centre and Notification Centre on top of Windows, with the taskbar auto-hidden; uninstall restores it | `lulo-session` supervisor + watchdog; AppBar host for the bar and Dock; `rmac-compositor-win32` (EnumWindows, WinEvent hooks, activation); low-level keyboard hook for ⌘ shortcuts and ⌘Tab; Spotlight on Windows Search + Start-menu apps; Control Centre on WlanApi, Bluetooth, Core Audio, power and brightness; Notification Centre on `UserNotificationListener`; Now Playing on GSMTC; the menu bar shows Lulo apps' menus over Lulo's IPC (they already export menus through `rmac-app-menu`, whose D-Bus transport (`zbus`) needs a named-pipe backend), and an App/Window menu for other apps built from UI Automation | 30–45 | 8–12 weeks |
| **4. Mission Control and the rest** | Mission Control and window previews, Dock minimise into tiles, Quick Look, Files (Finder) on Windows Shell APIs, screenshots, System Settings panes that make sense on Windows | DWM thumbnails, `Windows.Graphics.Capture`, the Finder backend on `IShellItem`, `IThumbnailCache` and NTFS tags; the System Settings pane set mapped to Windows Settings deep links | 25–40 | 6–10 weeks |
| **5. Optional "Lulo only" mode** | Lulo replaces Explorer as the shell, for kiosks and enthusiasts | `Winlogon\Shell` per user, recovery key, an Explorer fallback watchdog, our own tray host | 10–15 | 3–4 weeks |

Phases 1 → 2 → 3 must go in order. Phase 4 can start in parallel with phase 3 once
`rmac-compositor-win32` exists. Phase 5 is optional and should wait for feedback from
phase 3 users.

**What gives the most value earliest:**

1. Phase 2's signed installer with the apps. One download and no Linux; the apps are
   already the most Mac-like part of Lulo.
2. In phase 3, the Dock and Spotlight first (high visibility, relatively simple APIs). Then
   the menu bar with Lulo apps' menus, then ⌘Tab and Control Centre. The Notification Centre
   comes last, because it needs the listener permission.

## Testing: what CI can prove and what needs real hardware

**GitHub `windows-latest` (Windows Server 2022/2025, no GPU, one 1024×768 virtual display,
an interactive desktop session):**

- Build, clippy with `-D warnings`, and unit tests for the ported crates. Every phase
  extends the package list in the CI job.
- Launch smoke tests. GPUI renders through WARP on the runner's Basic Render Driver, so a
  process can start, open its window, stay alive and exit cleanly. Screenshots for
  pixel-level checks are possible but fragile.
- Behaviour checks through UI Automation (`accesskit_windows` exposes GPUI's accessibility
  tree): open a menu, type in Notes, check a value. This is the Windows counterpart of the
  AT-SPI scenarios in `tests/behavior`.
- AppBar registration, `RegisterHotKey`, `EnumWindows` and DWM thumbnail API calls all work
  in the runner's session, so their logic can be tested, though not how they look.
- MSIX packaging, a signing dry run (a test certificate imported into the runner's store),
  and install → run → uninstall → check that the taskbar state is restored.
- `cargo deny` on the Windows graph (Linux runner, `targets` list).

**Needs real hardware (owner's Windows PC):**

- Real GPUs and drivers, frame pacing and idle CPU and memory on low-spec machines, which
  matters most for Lulo's target users.
- HiDPI, mixed-DPI multi-monitor setups, monitor hot-plug, and AppBars across monitors.
- Touchpads (Precision Touchpad gestures), touchscreens and pen.
- Wi-Fi, Bluetooth pairing, audio devices, battery, brightness, sleep and resume, lid
  close.
- SmartScreen and antivirus behaviour on a clean consumer install. Windows 10 22H2 versus
  Windows 11 24H2/25H2 differences: the taskbar, toast UI, and Start's Win-key handling.
- Explorer restarts, Windows Update reboots and fast user switching.

## Top risks

1. **The keyboard hook and antivirus.** A global `WH_KEYBOARD_LL` hook is a keylogger
   pattern. AV and EDR products may flag it, especially when unsigned. Mitigations: signed
   binaries, a small hook process that handles only chords, Microsoft Defender submission,
   and documentation.
2. **Explorer coexistence.** The taskbar can reappear after an Explorer restart or a Windows
   Update, Windows 11 has given third-party taskbar control less room with each release, and
   Start/Win-key interception differs across builds. Mitigations: `TaskbarCreated`
   handling; auto-hide rather than hiding; an always-available "Restore Windows taskbar"
   escape hatch.
3. **Signing and SmartScreen.** Without an identity-validated certificate, the one-click
   promise fails at "Windows protected your PC". This blocks the phase 2 release.
4. **MSIX limits.** Registry and AppData virtualisation can make the taskbar and theme
   writes invisible to Explorer unless they are declared unvirtualised. Some corporate
   PCs block App Installer. Mitigation: the WiX fallback.
5. **UIPI and elevated windows.** Lulo cannot manage, preview or remap keys for admin
   windows. Users will meet this with installers and Task Manager.
6. **Window management fidelity.** Windows does not give one process niri-like control of
   other apps' windows. Some apps fight `SetForegroundWindow` and minimise animations, and
   UWP and WinUI frame windows (`ApplicationFrameHost`) need special cases.
7. **Menus of non-Lulo apps.** Windows apps keep their menus inside the window. A global
   menu bar can show only the app name, Window and Quit, plus whatever UI Automation
   exposes. This is a visible difference from the Mac that we document rather than fake.
8. **Maintaining two backends.** Each service gains a Windows backend that must stay at
   parity, which raises review and CI cost. Mitigations: shared model crates and contract
   tests (the existing `contract.rs`/`fake.rs` pattern) run on both platforms.
9. **The GPUI Windows backend is less used than Zed's macOS backend.** The
   runners' 1024×768 WARP display hides GPU bugs, so we need real-hardware smoke tests before each
   release.

## What the owner must provide

- **A Windows test PC.** Windows 11 24H2 or later, ideally low-spec like Lulo's targets
  (4–8 GB RAM, an Intel UHD or AMD APU with integrated graphics, a Precision Touchpad and,
  if possible, a touchscreen), plus a Windows 10 22H2 install or VM for the older taskbar.
  SSH (OpenSSH Server) access for agents, as with the reference laptop, in a separate local
  account so agents never touch the owner's session.
- **Code signing.** An Azure Trusted Signing account with identity validation, which is
  recommended, or an OV/EV certificate on a cloud HSM usable from GitHub Actions.
- **A Microsoft Partner Center account,** only if Store distribution is wanted.
- **Decisions:** the supported Windows versions (proposed: Windows 11 23H2 and later.
  Windows 10 left support in October 2025 and consumer extended updates end in October 2026,
  so it is best-effort only), whether phase 5 ("Lulo only") is wanted at all, and the
  product name and publisher identity for the package.

## Consequences

- Linux remains the primary platform. Windows support must never make the Linux build
  slower or bigger: all Windows code and dependencies are target-gated.
- Every new service backend follows the four-way seam. Reviews reject new
  `cfg(not(target_os = "macos"))` gates that mean "Linux".
- The CI Windows job starts non-blocking. It becomes blocking when phase 2's first signed
  release ships.
- `docs/parity.md` gets a "Windows" column, or `WIN-*` rows, once phase 2 starts, so that
  Windows gaps live in the same single table.
