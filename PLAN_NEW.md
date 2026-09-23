# rmac Platform Plan

> As of 2026-09-23, checked against `dev` at `57798ce7` (identical to GitHub
> `snehacodex/rmac` `dev`) and the last 100 commits.
> Estimates are person-weeks for one full-time developer starting 2026-09-28.
> Supersedes the roadmap sections of `PLAN_V2.md` for GPUI, Ubuntu and Windows work;
> `PLAN_V2.md` stays as history.

## 1. Decision

**Move rmac from `gpui-component 0.5.2` to `gpui-kit =0.6.6` with default features off,
so only `gpui-base` is used. `rmac-design` and `rmac-ui` become the design system.
Ship first as an Ubuntu 26.04 login session on niri. Port to Windows afterwards.**

Why:

- `gpui-component` has become `longbridge/gpui-kit`. rmac's pin, `0775df3`, is on the
  old 0.5 line, and the upstream fixes all go to 0.6.
- `gpui-base` fits how rmac already works. Nearly all recent UI work draws components
  measured from macOS 26 inside `rmac-ui`. `gpui-component`'s styled widgets are used in
  only 67 places across 22 crates.
- `gpui-base` provides the parts rmac shouldn't maintain itself: focus management, focus
  traps, accessibility roles, text input and editing, virtual lists, motion and reduced
  motion, and theme tokens.
- GPUI would come from exact crates.io pins (`gpui-pre =0.3.6`, a snapshot of Zed
  `bcf6582`). Today it comes from an untagged Zed git URL, pinned only by the lockfiles.

Corrections to earlier conversation notes, after re-checking the source:

- The apps and the shell **already share one GPUI revision**. Both `Cargo.lock` and
  `shell/Cargo.lock` resolve Zed `76c9396`. Layer-shell is already available at that
  revision. Neither of these is a reason to upgrade.
- The upgrade is **1,125 upstream commits**, so this is a GPUI bump as well as a
  component-library swap.
- ADR numbers 0010–0014 are taken. The new decision record is **ADR 0015**.
- ADR 0013 now vendors a patched `gpui_linux` in `shell/compat/gpui_linux`. That patch has
  to be re-assessed for the new GPUI (see A2).
- The toolchain is Rust **1.95.0** in both workspaces.
- A Linux reference laptop is already in use; ADRs 0010 and 0013 cite measurements from
  it. The formal `run-reference-gates.sh` evidence bundle has not been committed or
  captured in this checkout.

## 2. Current state

### 2.1 Source

| Item | Value |
|---|---|
| Crates | 109 in `crates/`, plus the `shell/` workspace (bins: dock, menubar, osd, app-switcher, wallpaper, mission-control, screenshot…) |
| Rust source | about 305k lines |
| GPUI | Zed git `76c9396` (reports 0.2.2), in both workspaces |
| Component library | `gpui-component =0.5.2` and `gpui-component-assets =0.5.1` at `0775df3` |
| GPUI patches | `shell/compat/gpui_linux`: an unmodified import (`8b268ee3`), the idle frame fix (`90949817`) and kinetic scrolling (`93daffd8`); `shell/compat/ztracing` |
| `gpui_component` users | 22 crates, 67 references: activity-monitor, app-drawer, archive-utility, calculator, clock, component-gallery, finder, launcher-app, notes, notification-center-app, platform-lab, player, preview, quick-settings-app, rmac-editor, rmac-file-chooser, rmac-quick-look, rmac-ui, system-settings, terminal, text-editor, weather |
| Platform gates | 349 Linux-only, 307 "not macOS", 117 macOS-only, **0 Windows** |

### 2.2 The last 100 commits

- All 100 landed on 2026-09-23, from `d5b65a26` to `57798ce7`. Together they changed
  722 files (+85,985 / −12,909 lines).
- The main pattern is macOS 26 parity. Most features land as two commits: a "Mock X at
  the Mac's measured sizes" commit, then a "Make X look and behave like macOS" commit.
  This covered the Dock, menu bar, Control Center, Spotlight, Notification Center,
  banners, window chrome, the lock and login screens, Files, Preview, Quick Look, Archive
  Utility, System Settings, TextEdit, Terminal, Activity Monitor, Notes, Mission Control
  and screenshots.
- New apps: Calculator, Clock, Weather, Media Player, Preview and Archive Utility.
- New session services with ADRs 0010–0014: screenshots over `grim`, clipboard history
  over `wl-clipboard`, the portal FileChooser backend, the patched `gpui_linux`, and
  Mission Control.
- Seven commits touch GPUI, niri or Wayland plumbing: the `gpui_linux` import and its two
  fixes, niri window fitting and scale correction, layer-shell feature declarations, and
  the menu shadow fix "for this GPUI revision".
- Most-touched areas: `system-settings` (117 file changes), `assets`, `scripts`,
  `shell/assets`, `packaging`, `finder`, `design-lab`, `shell/compat/gpui_linux`,
  `rmac-ui`, `rmac-dock`.

**What this means for the plan:** UI code on the old GPUI and `gpui-component` APIs grows
by thousands of lines a day. The migration needs a short, protected window (section 3.4).
Otherwise it will spend its time chasing rebases.

## 3. Workstream A: move to `gpui-kit` and `gpui-base`

### 3.1 What upstream provides (checked on GitHub and crates.io, 2026-09-23)

- **`longbridge/gpui-kit`:** latest release **v0.6.6** (2026-09-21), crates `gpui-kit`,
  `gpui-base`, `gpui-component`, `gpui-kit-assets` and others. Apache-2.0, 2024 edition.
- **`gpui-kit`** re-exports GPUI as `gpui_kit::*` and `gpui_kit::platform`, and
  `gpui-base` as `gpui_kit::base`. The `component` and `assets` features are on by default;
  turn them off.
- **GPUI:** `gpui-pre =0.3.6`, `gpui-pre-platform =0.3.6` and `gpui-pre-linux 0.3.6` on
  crates.io, all a snapshot of Zed `bcf6582` (2026-09-21).
- **Upstream `gpui_linux` at `bcf6582`:**
  - It has a frame-loop state machine with a `Parked` state, so idle surfaces stop
    requesting frame callbacks. rmac's idle fix `90949817` is probably no longer needed.
    Verify idle CPU on the laptop before dropping it.
  - It has **no** `wl_pointer` `AxisStop` handling. rmac's kinetic-scrolling patch
    `93daffd8` is still required.
- **`gpui-base` does not provide:** icons, context menus, or a sortable data table
  with a delegate. `rmac-ui` supplies these.

### 3.2 API mapping

| rmac uses (`gpui_component::`) | `gpui-base` replacement | Work |
|---|---|---|
| `Root`, `init` | `base::Root`, `base::init` | Change the import |
| `StyledExt`, `h_flex`, `v_flex`, `InteractiveElementExt` | Same names in `base` | Change the import |
| `Tooltip` | `base::Tooltip` (unstyled) | Add the measured rmac styling |
| `scroll::ScrollableElement` | `base::scrollbar`, `ScrollableMask` | macOS overlay scrollbar in `rmac-ui` |
| `theme::{Theme, ThemeMode, ActiveTheme}` | `base::Theme`, `theme_tokens` | Map the `rmac-design` tokens |
| `slider::{SliderState, SliderEvent}` | `Slider`, `SliderTrack`, `SliderThumb` | Assemble in `rmac-ui` |
| `input::{InputState, InputEvent, RopeExt, Position, SelectAll}` | `input::{Editor, Textarea, InputBase}`, `rope_ext` | **High risk.** Used directly by `rmac-editor` (Notes, Text Editor), `rmac-ui` text fields, `system-settings` and `platform-lab` |
| `table::{TableDelegate, TableState, Column, ColumnSort, TableEvent}` | Basic `Table` plus `virtual_list` | Sortable virtual table in `rmac-ui` (used today by System Monitor through `rmac-ui`) |
| `menu::{PopupMenu, PopupMenuItem}` | `Popup`, `Popover`, `Positioner` | Put the existing measured `rmac-ui` menus on top of these |
| `Icon`, `IconName`, `Size`, `Sizable` | none | `rmac-icon` (never Apple SF Symbols) |

### 3.3 Steps

| Step | Work | Duration | Exit gate |
|---|---|---|---|
| A0 | **ADR 0015.** One GPUI version through `gpui-kit =0.6.6`, default features off. Amends ADRs 0002, 0006 and 0013. Apps never name `gpui` directly. Bumps are exact pins in a dedicated pull request, at most monthly. | 2 days | ADR accepted |
| A1 | **Spike on a branch.** Port `rmac-ui` core (Root, theme, window, chrome, menus) and `component-gallery` to `gpui_kit::base`, then build and run on macOS. Spike text input with a single Text Editor window. | 1 week | Gallery and the editor window run with no changes to `gpui-kit` itself; otherwise stop and re-evaluate |
| A2 | **Re-base the GPUI patch.** Vendor `gpui-pre-linux 0.3.6` into `shell/compat/gpui_linux` unmodified and use it through `[patch.crates-io]`. Re-apply kinetic scrolling. Drop the idle fix only if the laptop shows idle surfaces parked. | 2–3 days | Idle niri CPU no worse than today; touchpad momentum works |
| A3 | **Port the probes** (`a11y`, `layer-shell`, `top-bar`, `platform-lab`) to `gpui-pre 0.3.6`. | 3–4 days | They build on macOS and on the laptop |
| A4 | **Design system on `gpui-base`:** Button, Toggle, Switch, Slider, TextField, SearchField, Tooltip, Scrollbar, Popover, Menu, ContextMenu, Tabs, Dialog, Alert, Toast, Table, Tree, Icon. Cover light, dark, high contrast and reduced motion, at the sizes in `design-lab/`. | 2–3 weeks | The gallery matches the `design-lab` references; only `rmac-ui` imports from `gpui-kit` |
| A5 | **Apps, one pull request each.** Order: Calculator, Clock, Weather, Player, Preview, Archive Utility → System Monitor → System Settings → Files, File Chooser, Quick Look → Launcher/Spotlight, Apps, Notification Center, Quick Settings → Notes, Text Editor, `rmac-editor` → Terminal. | 3–4 weeks | Each app's journey tests pass and its screenshots still match `design-lab` |
| A6 | **Shell.** Port `shell/bins/*` and `rmac-shell-layer` to the new layer-shell and accessibility API. Merging `shell/` into the root workspace is **optional**, because both already share one revision. | 1–2 weeks | Every shell host builds; the laptop smoke test passes |
| A7 | **Clean up.** Remove the Zed git dependencies and `gpui-component`. Update `deny.toml`, `scripts/linux/native_package_contract.py`, `install-upstream-shell-candidate.sh` (which reads the pin from `shell/Cargo.lock`) and `shell/README.md`. | 2–3 days | `cargo deny` passes; no `gpui_component` remains |

### 3.4 Handling the pace of development

- **Branch:** do A1–A7 on `migrate/gpui-kit`, rebased onto `dev` every day.
- **Freeze:** during A4–A5, new macOS-parity UI work goes only through `rmac-ui`
  components. No new direct `gpui_component` imports are allowed; add a CI grep check
  that enforces it.
- **Merge order:** land `rmac-ui` first behind a compatibility layer that keeps the old
  names, then move one app per pull request so `dev` is never broken.
- **Build rules (`AGENTS.md`):** check `df -h /System/Volumes/Data` and stop below
  25 GiB free. Build one package at a time, reuse `target/`, never run workspace-wide
  `--all-features` or `--all-targets`, and never run Cargo pipelines concurrently.

## 4. Workstream B: Ubuntu

### 4.1 Reference laptop setup

The laptop already exists. Confirm each step, and record whatever is missing.

1. **Base system:** Ubuntu Desktop 26.04 LTS with current firmware, the stock GNOME
   session kept, and an AMD or Intel GPU. Keep at least 25 GiB free (15 GiB is the
   absolute floor). Use a separate test user if the laptop holds personal data. Before
   testing rmac, confirm Wi-Fi, audio, Bluetooth, suspend/resume, scaling and the
   keyboard layout.
2. **Prepare the repository:**
   ```sh
   git clone https://github.com/snehacodex/rmac.git && cd rmac && git checkout dev
   bash scripts/linux/prepare-reference-pc.sh --check
   bash scripts/linux/prepare-reference-pc.sh --execute
   ```
3. **GNOME baseline** from the "Ubuntu" session:
   ```sh
   echo "$XDG_SESSION_TYPE"            # wayland
   vulkaninfo --summary                # real GPU, not llvmpipe
   bash scripts/linux/run-reference-gates.sh --preflight-only
   bash scripts/linux/run-reference-gates.sh --with-upstream-smoke
   cargo run -p rmac-platform-lab      # record every card
   ```
   Keep `target/linux-evidence/<timestamp>/gate-summary.tsv` and review it first.
4. **niri 26.04 and the rmac session**, with GNOME kept in GDM:
   ```sh
   niri --version && niri msg --json outputs | jq
   bash scripts/linux/run-reference-gates.sh --session niri --preflight-only
   bash scripts/linux/install-session-units.sh
   bash scripts/linux/install-upstream-shell-candidate.sh --check
   bash scripts/linux/install-upstream-shell-candidate.sh --execute
   ~/.local/bin/rmac-session-start
   systemctl --user --no-pager status rmac-session.target
   niri validate
   ```
   The session also depends on `grim` and `wl-clipboard` (ADRs 0010 and 0011).
5. **GPUI gate** on the ported probes from A3 (`gpui-pre 0.3.6`), with Orca on
   (Super+Alt+S):
   ```sh
   cd shell
   cargo run --features wayland --bin a11y
   cargo run --features wayland --bin layer-shell
   cargo run --features wayland --bin top-bar
   ```
   Pass criteria:
   - Orca reads the heading, counter, switch, focus order, actions and state.
   - The 40 px exclusive zone is reserved and the bar never takes keyboard focus.
   - The overview, maximize/fullscreen, monitor hotplug and mixed scaling behave correctly.
   - Idle CPU is parked, touchpad momentum works, and the four-hour soak from ADR 0001
     finishes without a crash.

The full procedure is in `docs/linux-reference-bringup.md`.

### 4.2 Phases

| Phase | Work | Duration | Exit gate |
|---|---|---|---|
| B1 | Confirm laptop setup and capture the committed baseline evidence (4.1 steps 1–4) | 1 week, during A1 | Preflight passes; `gate-summary.tsv` reviewed |
| B2 | **GPUI gate** on `gpui-pre 0.3.6` (4.1 step 5) | 1–2 weeks | Orca, layer-shell, idle and scrolling pass and the soak finishes clean. **If this fails, stop the migration and stay on `76c9396`.** |
| B3 | Apps on the laptop as each A5 pull request lands: keyboard, Orca, 100–200% scaling, IME, clipboard, the portal file chooser (ADR 0012) | 2–3 weeks | Each app passes its journey on Ubuntu |
| B4 | Shell and session after A6: Dock, menu bar, Mission Control (ADR 0014), screenshots, clipboard history, notifications, Quick Settings, shortcuts, multi-monitor, restarting niri and each component | 3–4 weeks | Journey 1 works by keyboard and pointer; idle and frame budgets met; a crashing component doesn't end the session |
| B5 | Packaging: signed APT repository; install, upgrade, rollback and uninstall in a VM and on the laptop; safe mode; GNOME recovery | 3–4 weeks | Every packaging check passes |
| B6 | Evidence and security: `run-accessibility-evidence.sh`, `run-privacy-security-evidence.sh`, visual and performance runs; chaos tests (suspend, hotplug, low disk, D-Bus restart, update); 8-hour and 7-day soaks. Security review of the lock screen and PAM wrapper, desktop-entry `Exec`, file operations, D-Bus, polkit, the clipboard service and the FileChooser backend. | 4–6 weeks | No open data-loss, privilege, lockout or critical accessibility bugs → **Alpha** |
| B7 | Hardware matrix: NVIDIA, a second AMD form factor, arm64. Beta cohort. | 6–10 weeks | At least 95% pass per journey on all five stations → **Beta** |
| B8 | 1.0 candidate: two consecutive distinct builds pass the full matrix (`docs/one-dot-zero-candidate.md`) | 2–4 weeks | **1.0** |

**After 1.0:** a custom Smithay compositor for real minimize, the genie animation,
per-window Mission Control pictures and gesture control (ADRs 0007 and 0014). Also global
menus and other distributions.

## 5. Workstream C: Windows

### 5.1 Scope

- **Target:** Windows 11 24H2 or later on x64 first, arm64 later. Home and Pro, running
  **alongside Explorer**. Replacing the shell outright is officially supported only through
  Shell Launcher on the Enterprise and Education editions, so it's optional.
- **Deliverables:** the rmac apps with macOS chrome, plus a macOS-like shell: menu bar,
  Dock, Spotlight, Mission Control, Control Center, Notification Center and screenshots.
- **Out of scope:**
  - Restyling other apps' title bars or buttons.
  - The genie animation for other apps' windows.
  - The login and lock screens.
  - The Windows Settings app, UAC prompts, standard Open/Save dialogs and other apps'
    context menus.
  - Any code injection into other processes.

### 5.2 Development machine setup

1. Windows 11 24H2 x64, with at least 60 GiB free and a DirectX 11-capable GPU.
2. Visual Studio 2022 Build Tools with **Desktop development with C++**, **MSVC v143** and
   the **Windows 11 SDK**.
3. Git, `gh`, and rustup with the repository's pinned Rust 1.95.0 toolchain.
   `x86_64-pc-windows-msvc` is installed by default; add `aarch64-pc-windows-msvc` later.
4. Enable Developer Mode, needed for symlinks and sideloading MSIX packages.
5. Clone to a short path such as `C:\src\rmac` to avoid path-length limits.
6. CI: add a `windows-latest` GitHub Actions job running package-scoped `cargo check`
   and tests. Don't cross-compile from the Mac; it needs `cargo-xwin` and extra disk.

### 5.3 Platform adapters

| rmac crate(s) | Windows API |
|---|---|
| `rmac-apps`, `rmac-app-launch` | Start Menu `.lnk` files and `shell:AppsFolder`; `ShellExecuteEx`; `IApplicationActivationManager` |
| `rmac-places`, `rmac-places-system` | `SHGetKnownFolderPath`; `IFileOperation` for the Recycle Bin |
| `rmac-portal` (open/reveal) | `ShellExecuteEx`, `SHOpenFolderAndSelectItems` |
| `rmac-file-chooser` | rmac's panel for rmac apps only; other apps keep the system dialog |
| `rmac-mounts` | `GetLogicalDrives`, volume APIs, `CM_Request_Device_Eject` |
| `rmac-network` | `Windows.Devices.WiFi`, `Windows.Networking.Connectivity`, WLAN API |
| `rmac-bluetooth` | `Windows.Devices.Bluetooth`, `Windows.Devices.Radios` |
| `rmac-audio`, `rmac-sound`, `rmac-media` | Core Audio (`IMMDeviceEnumerator`, `IAudioEndpointVolume`); `GlobalSystemMediaTransportControlsSessionManager` |
| `rmac-power` | `GetSystemPowerStatus`, power overlay schemes |
| `rmac-notifications-*` | WinRT `ToastNotificationManager`; `UserNotificationListener` (needs user consent) |
| `rmac-clipboard` | `AddClipboardFormatListener`; honour `ExcludeClipboardContentFromMonitorProcessing` |
| `rmac-appearance`, `rmac-theme` | Registry `AppsUseLightTheme`; `UISettings` accent colour |
| `rmac-wallpaper-*` | `IDesktopWallpaper` |
| `rmac-search` | Windows Search (`ISearchQueryHelper`) |
| `rmac-thumbnails` | `IShellItemImageFactory` |
| `rmac-shortcuts` | `RegisterHotKey`; a low-level keyboard hook for Cmd-style remapping |
| `rmac-login-items` | `StartupTask` (MSIX) or the `HKCU\...\Run` key |
| Screenshots (ADR 0010 equivalent) | `Windows.Graphics.Capture` |
| `rmac-compositor-windows` (new) | `SetWinEventHook` to observe; `SetWindowPos`/`ShowWindow` to act; DWM thumbnails for Mission Control and minimized tiles |
| `terminal` | ConPTY via `portable-pty` |
| `activity-monitor` | `sysinfo` |
| `rmac-focus`, `rmac-lock-*`, `rmac-privacy`, `rmac-updates` | Marked unavailable; no public API exists (lock is limited to `LockWorkStation`) |

### 5.4 Phases

Windows starts after A5, once every app is on `gpui-kit`.

| Phase | Work | Duration | Exit gate |
|---|---|---|---|
| C1 | Development machine and CI (5.2) | 1 week | CI job green on one crate |
| C2 | **Compile on Windows.** Change the 307 "not macOS" gates to explicit Linux gates. Make `zbus`, `wayland-*`, `pam-sys2`, `rustix` and `libc` Linux-only dependencies. Add Windows stubs that report unavailable. | 2–3 weeks | Every app package builds on Windows |
| C3 | Platform adapters (5.3) | 5–7 weeks | Each adapter has fakes and tests for absent, denied and restarted services |
| C4 | **Window chrome.** Traffic lights through `WindowControlArea` Min/Max/Close so Snap Layouts appear on the green button; Mica, rounded corners, dark title bar. `send_window_action` in `rmac-ui/src/chrome.rs` calls `ShowWindow`/`SetWindowPos` on Windows. | 2–3 weeks | Native snapping, Win+Arrow, touch and pen work |
| C5 | App hardening: per-monitor DPI, multi-monitor, dark mode, Narrator/NVDA over UI Automation | 3–4 weeks | Journeys pass; **gate: GPUI accessibility reaches UI Automation** |
| C6 | **Shell.** Menu bar as an AppBar (`SHAppBarMessage`); the Dock (always on top, never takes focus); taskbar auto-hide; Spotlight hotkey; Mission Control from DWM thumbnails; Control Center; Notification Center; screenshots; app-name and standard menus for every app, with real menus only where Win32 or UI Automation expose them | 10–14 weeks | Journey 1 works on Windows; idle CPU within budget |
| C7 | **Packaging.** Signed MSIX (Azure Trusted Signing or an OV/EV certificate), App Installer updates, a winget manifest | 2–3 weeks | Clean install, update, rollback and uninstall |
| C8 | Validation: 8-hour soak, sleep/resume, monitor hotplug, elevated windows (UIPI), antivirus false positives | 3–4 weeks | **Windows Beta** |

**Total:** 28–39 weeks. The apps can ship once C5 is done (about 13–18 weeks), before
the shell.

## 6. Timeline

Starting 2026-09-28 with one developer. At the commit rate seen on 2026-09-23 the calendar
may shrink considerably; the gates don't change.

| Weeks | Dates | Work |
|---|---|---|
| 1 | Sep 28 – Oct 4 | A0, A1 · B1 |
| 2 | Oct 5 – Oct 11 | A2, A3 |
| 2–4 | Oct 5 – Oct 25 | **B2 GPUI gate** (go/stop decision) |
| 3–5 | Oct 12 – Nov 1 | A4 design system |
| 5–9 | Oct 26 – Nov 29 | A5 apps · B3 as each app lands |
| 9–11 | Nov 23 – Dec 13 | A6, A7 |
| 11–15 | Dec 7 – Jan 10 | B4 shell and session |
| 15–21 | Jan 4 – Feb 21 | B5, B6 → **Alpha about Feb 22, 2027** |
| 21–31 | Feb 15 – May 2 | B7 → **Beta about early May 2027** |
| 31+ | May 2027 → | B8 1.0 candidate · Windows C1–C8 |

With a **second developer**, Windows C1–C5 starts in week 9 (Nov 23, 2026), and the
Windows apps arrive at about the same time as the Ubuntu Beta.

## 7. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| `gpui-pre 0.3.6` fails the Linux Orca, layer-shell, idle or scrolling gate | Migration blocked | Gate in weeks 2–4, before most porting; fallback is staying on `76c9396` with ADR 0013 patches |
| 1,125 upstream GPUI commits break shell code (layer surfaces, menu shadows, window fitting) | Shell slips | A3 probes first; port the shell last (A6) |
| Rapid macOS-parity commits conflict with the migration | Rebase churn | Section 3.4: branch, CI import guard, `rmac-ui`-only UI changes |
| `gpui-base` text input differs from `InputState` | Notes, Text Editor and Settings search slip | Spike in A1; migrate last in A5 |
| ADR 0013 patches need re-applying on every bump | Maintenance | Keep them as small separate commits; offer kinetic scrolling upstream to Zed |
| `gpui-base` lacks icons, menus and data table | Extra `rmac-ui` work | Planned in A4 on `rmac-icon` and the existing menus |
| Weekly `gpui-pre` snapshot churn | Build breaks | Exact pins; at most one bump a month |
| niri gaps: no minimize, no per-window capture, fixed gestures | Parity with macOS | Accept them for 1.0 (ADRs 0007, 0014); Smithay compositor after 1.0 |
| GPUI accessibility doesn't reach Windows UI Automation | Windows accessibility | Gate C5 before shell work |
| Windows hooks trigger antivirus; elevated windows can't be controlled | Shell reliability | No injection; document the UIPI limit; optional signed `uiAccess` build |
| Mac disk space | Blocked builds | `AGENTS.md` rules |
| Signing keys (APT GPG key, Windows certificate) | Release blocked | Provision by week 15 (Linux) and C7 (Windows) |

## 8. Decisions needed

1. Approve ADR 0015: `gpui-kit =0.6.6`, `gpui-base` only, amending ADRs 0002, 0006
   and 0013.
2. Approve the migration window in 3.4, including the pause on direct `gpui_component`
   use.
3. Windows timing: after Ubuntu Beta with one developer, or from week 9 with two.
4. Windows scope: ship the apps first (after C5), or wait for apps and shell together.
5. Whether to support Windows Enterprise shell replacement through Shell Launcher.
6. Who holds the APT signing key and the Windows code-signing certificate.

## 9. Next steps

- [ ] Write ADR 0015 (A0).
- [ ] Add the CI guard against new `gpui_component` imports.
- [ ] Create `migrate/gpui-kit` and spike `rmac-ui`, `component-gallery` and one Text
      Editor window on `gpui_kit::base` (A1).
- [ ] Vendor `gpui-pre-linux 0.3.6` and re-apply kinetic scrolling (A2).
- [ ] Port the four probes to `gpui-pre 0.3.6` (A3).
- [ ] Capture and commit a `run-reference-gates.sh` baseline from the laptop (B1).
- [ ] Run the GPUI gate on the laptop and record the go/stop decision (B2).
