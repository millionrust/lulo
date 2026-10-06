# ADR 0023 — Lulo on Windows: apps first, then a Mac-style shell that runs alongside Explorer

- **Status:** proposed 2026-10-06. Phase 1 (the app seam and a non-blocking CI job) is on
  branch `op/windows-plan`; phase 2's first slice (the gaps that blocked using the three
  apps) is on `op/win-phase2a`; the second slice (Preview, Clock, Weather, Terminal build
  and open) is on `op/win-phase2b`; the third slice (those four apps' own shortcuts move to
  `rmac_ui::bind_keys`, single instance for Calculator/Clock/Weather/Preview, real PDF
  rendering, and the CI proof for all of it) is on `op/win-phase2c`. The rest of phases 2–5
  needs owner approval and the hardware and signing items under "What the owner must
  provide".
- **Scope:** every crate under `crates/` and `shell/`, the workspace `Cargo.toml`,
  `deny.toml`, `.github/workflows/ci.yml`, and a future `packaging/windows/`.
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
  `ztracing` is pure Rust.
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
  stream — `PdfPage::RenderToStreamWithOptionsAsync` only renders to a stream either way, and
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

## Phase plan

The goal is "usable on Windows without Linux". Phases are ordered by how much value they
give early. Estimates are in agent-days (one focused agent working with CI) and in calendar
weeks with review and owner testing. They assume one to three agents in parallel and a
Windows test PC from phase 2.

| Phase | What the user gets | Main work | Agent-days | Calendar |
|---|---|---|---|---|
| **1. Seam + CI** (this branch) | Nothing to download yet. Calculator, Notes and TextEdit build and launch on Windows in CI. | cfg seam, Windows CI job, cargo-deny target, this ADR | 1–2 | 1 week |
| **2. The Lulo apps on Windows** (first slice built: shortcuts, menu strip, single instance, choosers, alert sound) | Download one signed installer and get Mac-feel Notes, TextEdit, Calculator, Preview, Clock, Weather, Terminal (ConPTY), Activity Monitor, then Calendar and Mail, in Start, with auto-update. **The first release that is valuable on its own.** | First the phase 1 gaps: `ctrl-` twins for every `cmd-` shortcut, an in-window menu strip, `PlaySound` cues, `ISpellChecker` spelling, Import/Export through GPUI prompts, a named-pipe single instance. Then port the remaining app crates; storage, locking and paths on Windows; open/save panels; printing through the Windows PDF path; toasts for Lulo apps; Credential Manager + loopback OAuth; MSIX packaging, signing and `.appinstaller` in CI; a "Lulo apps" behaviour subset under UI Automation | 15–25 | 4–6 weeks |
| **3. The shell alongside Explorer** | The menu bar, Dock, Spotlight, ⌘Tab, Control Centre and Notification Centre on top of Windows, with the taskbar auto-hidden; uninstall restores it | `lulo-session` supervisor + watchdog; AppBar host for the bar and Dock; `rmac-compositor-win32` (EnumWindows, WinEvent hooks, activation); low-level keyboard hook for ⌘ shortcuts and ⌘Tab; Spotlight on Windows Search + Start-menu apps; Control Centre on WlanApi, Bluetooth, Core Audio, power and brightness; Notification Centre on `UserNotificationListener`; Now Playing on GSMTC; the menu bar shows Lulo apps' menus over Lulo's IPC (they already export menus through `rmac-app-menu`, whose D-Bus transport (`zbus`) needs a named-pipe backend), and an App/Window menu for other apps built from UI Automation | 30–45 | 8–12 weeks |
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
