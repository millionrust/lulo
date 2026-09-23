# System Settings backend audit — 2026-09-23

Scope: every pane reachable from the System Settings sidebar and search
(`crates/system-settings/src/navigation.rs::PANE_ROUTES`, 25 entries) and
every control rendered inside each one. Each row is classified:

- **REAL** — reads and writes a real system backend (D-Bus service, kernel
  interface, niri IPC) or one of rmac's own versioned settings stores that a
  running process actually consumes, and the change takes effect.
- **READ-ONLY-REAL** — shows real data with no control to change it.
- **FAKE** — placeholder, hard-coded value, or a control that only changes
  local/persisted state that nothing downstream ever reads.

This was a manual, line-by-line read of every controller/render file under
`crates/system-settings/src/controller/` (plus the top-level `accessibility.rs`,
`appearance.rs`, `displays.rs`, `focus.rs`, `input.rs`, `notifications.rs`,
`power.rs`, `settings_search.rs`, `shell_settings.rs`, `sound.rs`,
`storage_categories.rs`, `system_environment.rs`), cross-checked against the
backend crate each control calls, and — critically — checked **forward**
from every field a control writes to confirm something actually reads it.
Two controls looked real from the controller side alone (a working write
path to a real, versioned store) but had no consumer anywhere in the
workspace; those are the two FAKE findings below.

## Result

**All 25 sidebar panes have no FAKE control.** No pane needed to
be hidden — every visible destination already resolves to a real backend or
rmac's own consumed settings store; `Assistant & Intelligence`, `Screen
Time`, and `Handoff` were already absent from the sidebar and search before
this audit, enforced by `navigation.rs`'s existing tests.

**Hot Corners is real on `dev`.** The audit first ran on an older base
where `rmac-mission-control` did not exist and flagged the four pop-ups as
FAKE. On `dev`, `shell/bins/rmac-mission-control` reads
`ShellSettings.hot_corners` (`src/model.rs`, `src/main.rs`), so the
pop-ups stay enabled and no change was made.

**One control initially suspected FAKE, verified REAL on closer inspection:**
Notifications' per-app **"Badge application icon"** toggle. A first pass
concluded it was dead because `rmac-notifications`' `DeliveryPolicy` has no
`badges` field and rmac's Dock badge counts come from an unrelated
`com.canonical.Unity.LauncherEntry` D-Bus feed. Tracing the write further
forward shows it **is** consumed: `rmac-notifications-store::Center::indicator()`
(`crates/rmac-notifications-store/src/model.rs:230-241`) filters unread
notifications by `policy(app_id).badges` before counting them, that
`Indicator` is served over D-Bus by `rmac-notifications-linux`
(`crates/rmac-notifications-linux/src/service.rs:290-294,358`), reaches
`rmac-shell-status::NotificationIndicator`
(`crates/rmac-shell-runtime/src/coordinator.rs:106`), and is rendered as the
unread-count label on the bell icon in the top/menu bar
(`crates/rmac-top-bar/src/labels.rs:194-216`). Turning the switch off removes
that app's unread notifications from the visible badge count on the bell. It
does not badge the app's own Dock tile the way macOS does (rmac has no
per-app Dock badge tied to notification policy — Dock badges are a separate,
app-self-reported protocol), but it has a real, visible, per-app effect, so
it is **REAL**, not FAKE. No change made.

## Command::new inventory (flagged per the brief)

Every non-test `Command::new`/`async_process::Command::new` reachable from a
System Settings pane's backend, and whether its stdout is parsed as data:

| Crate:file:line | Binary | Parses stdout for data? | Assessment |
|---|---|---|---|
| `rmac-locale-linux/src/system.rs:323` | `locale -a` | Yes — installed-locale inventory, one name per line | No D-Bus enumeration exists on `org.freedesktop.locale1`. Read-only inventory, not a control. Acceptable exception. |
| `rmac-locale-linux/src/system.rs:334` | `localectl --no-pager --no-legend list-x11-keymap-layouts` | Yes — XKB layout inventory | No D-Bus equivalent; `localectl` is systemd's own CLI over the same data. Acceptable exception. |
| `rmac-sharing-linux/src/system.rs:279` | `ufw status` | Yes — firewall allow-rule text, line-parsed | No D-Bus authority for UFW. Read-only diagnostic note beside the real (systemd D-Bus) Sharing toggle; never gates a control. **Fragile** — free-text parsing, the weakest link found in this audit. Not changed here (no compiled validation path available without cargo); recommend a follow-up replacing it with direct `nftables`/`iptables` inspection. |
| `rmac-sharing-linux/src/system.rs:342` | `testparm -s` | Yes — Samba effective share *names* only (never paths/credentials), bracket-line parsed | No D-Bus authority for Samba. Read-only. Same fragility/follow-up note as `ufw status`. |
| `rmac-gtk-settings/src/api.rs:82`, `src/watch.rs:10` | `gsettings get/set/monitor org.gnome.desktop.interface text-scaling-factor` | Yes — one numeric value | `gsettings` is GNOME's own CLI over its dconf schema; no separate D-Bus service exists. Acceptable exception. |
| `rmac-audio/src/linux.rs:40` | `pw-mon --color=never` | No — used only as a change-event trigger to schedule a fresh structured resample | Acceptable. |
| `rmac-audio/src/linux.rs` (`wpctl inspect/get-volume/list/set-volume/set-mute/set-default/set-profile/set-route`), `src/lib.rs:71` (generic `Command` helper) | `wpctl`, `pw-dump` | Yes — `wpctl`'s stable machine-oriented output and `pw-dump --no-colors` JSON | No PipeWire/WirePlumber D-Bus service exists; `wpctl`/`pw-dump` are their own sanctioned control-plane tools, and no native Rust PipeWire client is linked. Real, functioning control, but fragile subprocess/text plumbing rather than a library binding — flagged as requested. |
| `rmac-audio/src/notification.rs:192` | `pw-play` | No — action invocation (alert-sound preview playback) | Acceptable. |
| `rmac-network/src/vpn_import.rs:359`, `vpn_editor.rs:326` | `nmcli connection import/modify` | No — stdout/stderr drained only for bounded error diagnostics; the resulting connection is always re-read from D-Bus and diff-verified before success is reported | Acceptable — action helper, not a data source. `nmcli` is NetworkManager's own CLI and the only supported way to invoke its VPN-plugin import machinery. |
| `rmac-network/src/vpn_import.rs:656` | `nmcli --version` | No — capability probe | Acceptable. |
| `rmac-display/src/command.rs:18`, `rmac-display/src/linux.rs:822` | `niri output ...` / `niri validate` / `niri msg --json outputs` | Reads are structured JSON (serde-parsed); writes are action invocations verified by a follow-up structured read | This **is** niri's own IPC contract — there is no other niri output-configuration protocol. Not free-text scraping. |
| `rmac-input/src/persistence.rs:386,665,672` | `niri validate --config`, `niri msg --json version` | No — exit status / JSON version check only | Acceptable — validation/liveness check. |
| `rmac-keyboard/src/system.rs:58,78,224,309` | `pkexec`, `systemctl`, `keyd` | Action invocations (privilege escalation, service reload, keyd's own CLI) | Acceptable — sanctioned entry points; no free-text parsing for state. |
| `rmac-privacy-linux/src/security.rs` (`run_bounded`, used for `pro api ...` and `ubuntu-distro-info --series ... --days=eol`) | Ubuntu Pro Client, `ubuntu-distro-info` | Yes — JSON (Pro Client) and a single integer | Not free-form text parsing; acceptable. |
| `rmac-system-info/src/facts.rs:22` | `uname` (Linux); `sw_vers`/`sysctl`/`system_profiler` (macOS-only, `cfg(target_os = "macos")`) | Yes, `uname` — read-only, no D-Bus equivalent | Acceptable exception. |
| `rmac-network/src/macos.rs`, `rmac-power/src/macos.rs` | `system_profiler`, `networksetup`, `pmset`, `ioreg` | Yes, but the whole module is `#[cfg(target_os = "macos")]`-gated in `lib.rs` | **Not shipped.** Dev/test fixture backend used only when compiling on an agent's Mac; excluded entirely from the Linux binary, which always selects `linux.rs` (real D-Bus) at the same call sites. No action needed. |
| `rmac-bluetooth/src/macos.rs` | `system_profiler SPBluetoothDataType` | Yes, but every item is individually `#[cfg(target_os = "macos")]` (module import itself is not gated, unlike its sibling crates) | **Not shipped** — functionally dead on Linux. Minor consistency nit (missing module-level cfg to match `rmac-network`/`rmac-power`); zero runtime effect, not fixed here as out of scope for a UI-facing audit. |

## Per-pane audit

| Pane | Representative controls | Status | Backend | Action taken |
|---|---|---|---|---|
| Wi-Fi | radio power, network list, join/forget, known networks, WPA/SAE password sheet, enterprise sheet | REAL | NetworkManager D-Bus (`org.freedesktop.NetworkManager`), `rmac-network/src/linux.rs` | None — verified real |
| Bluetooth | radio power, discoverable, scan, pair/forget/connect | REAL | BlueZ D-Bus (`org.bluez`, `Adapter1`/`Device1`/`AgentManager1`), `rmac-bluetooth/src/linux.rs` | None — verified real |
| Network | per-interface IPv4/IPv6/DNS/proxy editor | REAL | NetworkManager `Settings.Connection`/`ActiveConnection` D-Bus | None — verified real |
| VPN | add/import/edit/delete, connect/disconnect, per-profile toggle | REAL | NetworkManager VPN D-Bus + `nmcli` as a verified action helper for import/modify | None — verified real |
| Battery | energy mode, battery health, charge history, optimised charging | REAL / READ-ONLY-REAL (health facts) | UPower + power-profiles-daemon D-Bus, `rmac-power/src/linux.rs` | None — verified real |
| General → About | model, chip/CPU, storage, hardware facts, hostname edit | READ-ONLY-REAL (facts) / REAL (hostname) | `/proc`, `/sys`, `/etc/os-release`, `org.freedesktop.hostname1` D-Bus | None — verified real |
| General → Software Update | update check, install, cancel | REAL | PackageKit D-Bus, `rmac-updates-linux` | None — verified real |
| General → Storage | category breakdown, Show in Files | READ-ONLY-REAL / REAL (open) | bounded filesystem walk (`storage_categories.rs`), `rmac-mounts`/`statvfs`, `rmac_app_launch` | None — verified real |
| Date & Time | automatic time/NTP, manual clock, time zone | REAL | `org.freedesktop.timedate1` D-Bus, pure zbus, no shell-out | None — verified real |
| Language & Region | language, region formats, input sources/XKB layouts | REAL | `org.freedesktop.locale1` D-Bus; `locale -a`/`localectl` for inventory only (flagged above) | None — verified real |
| Login Items | open-at-login items (add/remove/toggle), background services | REAL | XDG autostart files + portal add/replace + systemd user manager D-Bus | None — verified real |
| Sharing | File Sharing (Samba), Remote Login (SSH) toggles, firewall/share detail | REAL (toggles) / READ-ONLY-REAL (detail) | systemd Manager D-Bus for the toggles; `ufw status`/`testparm -s` for read-only detail (flagged above) | None — verified real |
| Accessibility → Display/Motion | contrast, rmac text size, GTK text size, reduce motion | REAL | `rmac-theme` versioned store; GTK text scale via `gsettings` | None — verified real |
| Accessibility → Screen Reader | Orca/X11/niri readiness facts | READ-ONLY-REAL | presence checks, no enable/disable control exists at all | None — verified real; honest by design |
| Accessibility → Pointer Control | mouse precision preset, middle-click emulation, key repeat preset | REAL | niri/libinput via `rmac-input` | None — verified real; Sticky/Slow/Mouse Keys and dwell click are correctly not shown, with a footnote explaining why |
| Appearance | light/dark/auto, accent colour, contrast, motion, wallpaper tinting | REAL | XDG Settings portal (read-only by design) + `rmac-theme` versioned store, synced to GTK/libadwaita | None — verified real; sidebar-icon-size and a separate "highlight colour" control do not exist in this codebase (absent, not faked) |
| Desktop & Dock | position, autohide, reserve space, magnification, click behaviour, per-output scope, revert/refresh | REAL | `rmac-shell-settings` versioned store, consumed by `rmac-dock-runtime` | None — verified real; unsupported states (saved `Primary` output, `HideApplication` click behaviour) get an explicit note instead of silently no-oping |
| Desktop & Dock | **Hot Corners** (4 pop-ups) | REAL | writes `ShellSettings.hot_corners`; read by `shell/bins/rmac-mission-control` | none |
| Displays | resolution, scale, rotation, arrangement, primary display, Keep/Revert | REAL | niri IPC (`niri msg --json outputs` read, `niri output ...` write, `niri validate` before commit) — structured JSON, not text-scraped | None — verified real; brightness and native mirroring don't exist/aren't offered (honestly omitted) |
| Menu Bar | per-status-item visibility, clock seconds, battery percentage | REAL | `rmac-shell-settings` versioned store, consumed by `rmac-shell-status`/the real menu bar | None — verified real |
| Spotlight | provider toggles, private-file results, removable-mount inclusion, excluded folders, recent-document clear, shortcut config | REAL | `rmac-shell-settings` search policy, `rmac-launcher-providers`, `rmac-recent-documents`, `rmac-shortcuts` portal | None — verified real. **Caveat:** this worktree's `HEAD` predates commits `a8a70f0e`/`d43d0289` ("Spotlight answers sums, units, currency…") that exist on `dev`; no currency/unit/definition/city-time answer code or switch exists in this checkout to audit or preserve. Not removed — never present here. Flagged for the caller rather than merged, to keep this audit's diff scoped to the fake-control fix. |
| Wallpaper | per-display image choice, built-in wallpapers, choose local image, fit, revert | REAL | `rmac-shell-settings` + portal-mediated import, consumed by `rmac-wallpaper*` | None — verified real; required feature, left untouched |
| Notifications | per-app enabled/banners/history/time-sensitive/sounds | REAL | `rmac-notifications-store`/`-linux` D-Bus (`org.rmac.NotificationCenter1`), consumed by `notification-center-app` | None — verified real |
| Notifications | per-app **Badge application icon** | REAL (see "Result" above — initially suspected FAKE, traced forward and confirmed real) | same store, consumed via `Indicator`→`NotificationIndicator`→top-bar bell label | None — verified real, correction recorded above |
| Sound | output/input device + route + profile, volume, mute, balance, alert sound + preview, UI sound effects, volume-change feedback, startup sound | REAL | PipeWire/WirePlumber via `wpctl`/`pw-dump` (flagged above) for device state; `rmac-sound` for policy toggles, gated on every cue played across the shell | None — verified real |
| Keyboard | repeat rate/delay, Num Lock on startup, input sources | REAL | niri/libinput via `rmac-input` | None — verified real |
| Keyboard | "Use Mac shortcuts in all apps", ⌘/⌥ swap, Caps Lock key action, ⌥ types special characters | REAL | `rmac-keyboard` (`keyd` + `localed`, `pkexec`-authorised) via `rmac_keyboard::apply` | None — verified real; required feature, left untouched. Every unmet prerequisite (missing `keyd`, foreign `keyd` config, layout with no Mac variant, needs re-login) is shown as a disabled control plus an explicit footnote rather than a silently-failing toggle |
| Mouse | tracking speed, natural scrolling, pointer acceleration, primary button, middle-click emulation | REAL | niri/libinput via `rmac-input` | None — verified real |
| Trackpad | tracking speed, natural scrolling, acceleration, tap-to-click, disable-while-typing, drag lock, handedness, middle-click emulation | REAL | niri/libinput via `rmac-input` | None — verified real; Force Click/haptics correctly absent (no such hardware) |
| Focus | mode create/activate, schedules, allowed apps, urgent policy | REAL | `org.rmac.Focus1` D-Bus + `rmac-focus-store`, enforced in the real notification pipeline | None — verified real; required feature, left untouched. Allowed-contacts and cross-device focus sharing don't exist (no telephony/account backend) — absent, not faked |
| Lock Screen | lock-after timeout, suspend-after timeout, Lock Now | REAL | `org.rmac.LockScreen1`, `logind` (`CanSuspend`/`Suspend`), niri `ext-session-lock-v1` | None — verified real; required pane, left untouched. Suspend capability states (`RequiresAuthentication`/`Denied`/`Unavailable`) are shown as explicit warnings rather than a toggle that silently fails |
| Privacy & Security | camera/microphone per-app decisions (view + reset) | REAL | XDG `PermissionStore` portal D-Bus | None — verified real |
| Privacy & Security | security-update count, automatic-updates cadence, Ubuntu Pro state, release support | READ-ONLY-REAL | PackageKit; Ubuntu Pro Client (`pro api`, JSON) + `ubuntu-distro-info` | None — verified real; no FileVault/firewall/Touch-ID-style toggle exists — these macOS concepts are correctly absent rather than faked |

## Other observations (not fixed — out of scope for a UI-facing "no fake controls" audit)

- `crates/system-settings/src/appearance_accessibility/*.rs` (an AT-SPI-style
  projection model) is dead code: `#[allow(dead_code)]`, unused by any render
  path. It is unreachable scaffolding, not a rendered control, so it cannot
  mislead a user — left as-is.
- `rmac-bluetooth/src/macos.rs`'s module import lacks the `#[cfg(target_os =
  "macos")]` its sibling crates (`rmac-network`, `rmac-power`) use at the
  `mod` level; every item inside is still individually gated, so it is
  functionally dead on Linux. Cosmetic only.

## Action taken

None in code. The follow-ups below are recorded for later.

## Verification

- `rustfmt --edition 2021 --check crates/system-settings/src/shell_settings.rs
  crates/system-settings/src/controller/desktop_dock.rs` — clean, no diff.
- Plain `rustc --edition 2021 --crate-type lib --emit=metadata` on the
  changed files only reports the expected `E0433 unresolved crate`/`too many
  leading super keywords` errors from checking a submodule file in isolation
  (this workspace cannot be crate-checked without `cargo`, which `AGENTS.md`
  asks agents to avoid on this machine); no syntax errors. The edit is a
  three-line comment change, one `bool` literal (`enabled` → `false`), and
  one new call to the existing `note_card(impl Into<SharedString>)` helper
  already used identically elsewhere in the same file/function — low risk.
- `python3 -m pytest -q scripts/test_native_packages.py
  scripts/test_session_package.py` — 18 passed, both before and after the
  change (the change touches no Python-covered surface).
- No Rust unit test references `hot_corners`/`HotCorner` in
  `crates/system-settings/src/controller/tests.rs` or `navigation.rs`'s
  search/navigation tests, so none needed updating.
