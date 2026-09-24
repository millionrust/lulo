# System integration audit — 2026-09-24

This audit checked the live reference laptop against what the repository
declares: that every unit, bus name, portal, niri rule, desktop entry and MIME
default is wired up, and that nothing is erroring, missing or busy while idle.

- **Laptop:** i5-5300U, 6.7 GB, niri 26.04, Ubuntu 26.04.
- **Repository:** `dev` at 5bea40c.
- **What runs on the laptop:**
  - Most shell surfaces run dev builds from `~/rmac-dev-bin`, through systemd
    user drop-ins (`dev.conf`).
  - The `/usr` packages `rmac-session` and `rmac-apps` are the old Sep-20
    build, `0.1.0-37`.
  - The session was started at 07:36 IST and the dev surfaces were restarted
    at 09:51 IST.

All evidence was gathered read-only over ssh (`systemctl --user show`,
`journalctl --user`, `busctl --user`, `niri msg --json`, `/proc`). The only
exception was the budget run, which launched and closed apps under the shared
screen lock. Personal data was neither read nor recorded.

Severity scale:

- **P0:** broken for users.
- **P1:** a visible defect or a budget failure.
- **P2:** wrong but contained.
- **P3:** hygiene.

## Findings

| # | Sev | Area | Finding | Evidence | Where / owner |
|---|---|---|---|---|---|
| 1 | P1 | Idle | **System Settings idles at 24.95% CPU and 164 wake-ups/s.** | budget run, `docs/perf/reference-laptop-2026-09-24.md` | `crates/system-settings` (Settings agent) |
| 2 | P1 | Idle | **The top bar idles at 1.98% CPU and 24 wake-ups/s**, more than the rest of the shell combined (the shell total is 2.78% against a 1% budget). Besides the 250 ms idle check, its GPUI timer thread keeps re-arming a timerfd about once a second; nothing shows on the session bus. The cause is not found yet. Candidates: a 1 s GPUI-executor timer in `rmac-menubar` or `rmac-shell-runtime`, or the 50 ms reconcile retry in `shell/bins/rmac-menubar/src/main.rs:3208`, which spins whenever `tracker.len() != desired.len()`. That happens whenever niri reports an output GPUI has no display for, because `reconcile` filters those out but the completion check does not. | `/proc/<pid>/fdinfo` sampling of timerfds, per-thread `ctxt_switches` | `shell/bins/rmac-menubar` |
| 3 | P1 | Idle | **The shortcut broker asked the GlobalShortcuts portal again every 2 s, forever.** The portal refuses rmac with `NotAllowed: An app id is required`. Each retry cost about 15 bus calls and an atomic rewrite of `shortcuts-status.json`: 23 wake-ups/s and 0.28% CPU. | `busctl --user monitor` (10 s: 40 AddMatch, 40 RemoveMatch, 5 CreateSession); status file mtime changing every ~6 s | **Fixed** 0a861f83 (`crates/rmac-shortcuts/src/portal.rs`) |
| 4 | P1 | Idle | Every visible layer-shell window wakes about 4 times a second when nothing changes: the Dock (2 windows) at 8/s, and the wallpaper and OSD at 4/s. The cause is the vendored platform's idle re-check (`idle_check_delay`, 16 ms growing to 250 ms). It is cheap, but it is not "idle means idle". The fix is architectural: GPUI must signal the platform when a window becomes dirty. | timerfd sampling on Dock, wallpaper, OSD and top bar; none on hidden surfaces | `shell/compat/gpui_linux/src/linux/wayland/window.rs:763` (ADR 0013) |
| 5 | P1 | Idle | Apps over the 0.3% idle budget: Files 5.32%, System Monitor 3.13%, Clock 1.58%, Weather 0.67%, Text Editor 0.65%. | budget run (idle counters are per-process, so the concurrent build does not inflate them) | Files → Files agent; the others are unowned |
| 6 | P1 | Portals | **Open and Save panels resolve to GNOME's (Nautilus), not rmac-file-chooser.** The installed `/usr/share/xdg-desktop-portal/rmac-portals.conf` (Sep 20) has no `FileChooser=` line. No `rmac-file-chooser.portal` or D-Bus activation file is installed, and no `rmac-file-chooser.service` exists. Preview started without a file therefore never showed a window, and `xdg-desktop-portal-gnome` activated `org.gnome.Nautilus`. The repository packaging is correct: `rmac-portals.conf` has `FileChooser=rmac-file-chooser;gnome;gtk`, and the stage script installs the portal, activation and unit files. | journal 09:55:06 "Activating service org.gnome.Nautilus requested by xdg-desktop-portal-gnome"; `ls /usr/share/xdg-desktop-portal` | needs the new `rmac-session` package (**sudo**) |
| 7 | P1 | MIME | Images and PDFs opened in Loupe and Papers, not Preview. `rmac-mimeapps.list` named only Archive Utility and Media Player. | `xdg-mime query default image/png` → `org.gnome.Loupe.desktop` | **Fixed** 1ccf4606 (package defaults and a test that every claimed type has an rmac default) |
| 8 | P1 | MIME | Folders open in Nautilus. `org.rmac.Files.desktop` claims no `inode/directory` and its `Exec` takes no path. `rmac-files` accepts only `--path DIR`, so it cannot be the folder handler until it accepts `%U`/`%F` (or a desktop action is added). | `xdg-mime query default inode/directory` → Nautilus | `crates/finder/src/main.rs:37`, `packaging/rmac-apps/applications/org.rmac.Files.desktop` (Files agent) |
| 9 | P1 | Deploy | **The laptop launches old builds for most apps.** User desktop overrides exist only for System Monitor, System Settings, Terminal and Text Editor (dev), and for Files and Apps, which point at the old `/usr/bin`. Notes has no user override, so it runs the old `/usr/bin` build. **Calculator, Clock, Weather, Preview, Archive Utility and Media Player have no desktop entry at all** (the Sep-20 `rmac-apps` package predates them), so the Dock and Launchpad cannot show them. `~/.local/libexec/rmac` holds symlinks for only 9 of 29 dev binaries. | `grep Exec ~/.local/share/applications/org.rmac.*.desktop`; `ls /usr/share/applications` | coordinator's dev deploy, or install the new `rmac-apps` (**sudo**) |
| 10 | P1 | Services | **After the next login four dev surfaces will not start.** The installed `rmac-session.target` (Sep 20) does not `Want` `rmac-app-switcher`, `rmac-screenshot`, `rmac-mission-control` or `rmac-clipboard`. They run today only because they were started by hand. The repository target wants all 19. | `diff` of `/usr/lib/systemd/user/rmac-session.target` against `crates/rmac-session/units/rmac-session.target` | a dev drop-in (`rmac-session.target.d/dev.conf` with `Wants=`), or the new package (**sudo**) |
| 11 | P2 | Services | Declared units that are **not deployed**: `rmac-file-chooser.service`, `rmac-mac-keyboard.service` (also needs `/etc/keyd/rmac.conf`, missing), `rmac-setup-assistant.service` (the first-login hook), `rmac-update-check.service` and `.timer`, and `rmac-safe-mode-notice.service`. The dev binaries exist in `~/rmac-dev-bin`. | `systemctl --user show … LoadState=not-found` | new `rmac-session` package (**sudo**) |
| 12 | P2 | Services | Five services still run the **old `/usr/libexec` build** with no dev drop-in: `rmac-session-supervisor`, `rmac-shortcut-broker`, `rmac-focus`, `rmac-lock-coordinator` and `rmac-idle-lock`. Their fixes on `dev` (for example the broker fix above, and `ESSENTIAL_UNITS` no longer letting the OSD trigger safe mode) are not live. The rest of the session was built today, so the old supervisor and focus service are a protocol-skew risk. The session did enter safe mode on 2026-09-23 because the OSD exhausted its restart budget (`safe-mode.json.cleared-2026-09-24`). | `readlink /proc/<pid>/exe` | coordinator: build and drop in these five, or install the package |
| 13 | P2 | Zombies | `rmac-top-bar` left a zombie `rmac-shortcut-dispatch` for each Control Center, Spotlight, Notification Center or Lock click. Apple-menu commands (`systemctl suspend`, `reboot`, `poweroff`) were never waited on either, so their failures were invisible. | `ps` found a zombie with parent `rmac-top-bar` | **Fixed** b9b40542 (`shell/bins/rmac-menubar/src/main.rs`, `run_to_exit`) |
| 14 | P2 | niri | The live `~/.config/rmac/niri/config.kdl` declares `workspace "rmac-parking"` itself, and `dev-shell.kdl` (from `shell.kdl`) declares it again, so niri created **two** `rmac-parking` workspaces (ids 2 and 3). The repository's `config.kdl` is correct; the extra line is a leftover from before `shell.kdl` owned the parking workspace. The line was not removed by hand, to avoid a niri reload during other agents' runs. | `niri msg --json workspaces` | coordinator: delete the 2 lines from the live `config.kdl` |
| 15 | P2 | niri | The dev `dev-shell.kdl` still sends the volume, mute and brightness keys to `/usr/libexec/rmac/rmac-osd` (old build) while `rmac-osd.service` runs the dev build. Every other spawn was rewritten to `~/rmac-dev-bin`. `shortcuts-generated.kdl` also points at the old `/usr/libexec/rmac/rmac-shortcut-dispatch`. | diff against `packaging/rmac-session/shell.kdl` with the dev path substituted | coordinator's config generator |
| 16 | P2 | Portals | GlobalShortcuts is permanently in fallback mode: the frontend (`xdg-desktop-portal` 1.21) requires a host app ID (`org.freedesktop.host.portal.Registry.Register`), and even with one, the GNOME backend needs gnome-shell to grab keys, which niri is not. The niri fallback bindings are the working path. **Recommendation:** make the fallback the documented default on niri rather than calling `ashpd::register_host_app`, because registering would let the GNOME backend open its GNOME Settings consent dialog. | `/run/user/1000/rmac/shortcuts-status.json` | `crates/rmac-shortcuts` (design decision) |
| 17 | P2 | Portals | `rmac-wallpaper-portal` (backend `org.freedesktop.impl.portal.desktop.rmac.wallpaper`) is implemented and tested, but no binary serves it and no `.portal` file or `portals.conf` entry selects it, so the Wallpaper portal is served by GNOME. The limitation is documented in `docs/system-settings-audit.md`. | `grep` for crate users: none | `crates/rmac-wallpaper-portal`, packaging |
| 18 | P2 | Disk | `~/.local/state/rmac/migrations/development-install-v1` holds **7.5 GB**: the Aug-9 dev `libexec` archived by `scripts/linux/archive-development-install.py`. The archive is reversible by design, but nothing reports its size or offers to prune it. | `du -sh` | owner may delete it; the script could print the size and a prune command |
| 19 | P3 | Session | At 07:33, `rmac-session.target` was started inside a plain `niri.service` session (config `~/.config/niri/config.kdl`, no `NIRI_CONFIG`). The broker exited 4 times with "NIRI_CONFIG is not set to an absolute path", and the broker counts as *essential*, so that session could enter safe mode. Starting the target by hand outside `rmac-wayland-session` should be refused with a clear message instead. | journal 07:33:11–07:33:15 | `crates/rmac-session`, `crates/rmac-shortcuts/src/bin/broker.rs:26` |
| 20 | P3 | niri | niri holds two "Command Spawner" children (08:07) with zombie grandchildren and 744 or more inherited fds each. These are niri's own spawn helpers, not rmac processes, but they hold fds open for the session's lifetime. | `ps`, `/proc/<pid>/fd` | upstream niri; worth an issue if it recurs |
| 21 | P3 | Logs | Top-bar log noise from the pre-08:46 builds, "application menu bus is unavailable" (×45) and "ServiceUnknown … TextEditor.Menu", is gone in the current build (55174eed and 6d3d7699). One "NoReply" remains when an app quits during a menu fetch (09:01:58), which is acceptable. | journal clusters | none |
| 22 | P3 | Deploy | `~/.config/systemd/user` holds full copies of `rmac-app-switcher`, `rmac-clipboard`, `rmac-mission-control` and `rmac-screenshot` units. Unlike the repository units, they lack `ConditionFileIsExecutable=`. There are also stray `dev.conf.bak-*` and `dev-shell.kdl.bak-*` files. | `diff` | coordinator cleanup |

## 1. Services

Every unit named in `rmac-session.target`, `COMPONENT_UNITS` and
`ESSENTIAL_UNITS`, with its state on the laptop at 09:58 IST:

| Unit | State | NRestarts | Binary |
|---|---|---:|---|
| rmac-top-bar, rmac-dock, rmac-wallpaper, rmac-osd, rmac-app-drawer, rmac-quick-settings, rmac-notification-center, rmac-notification-center-panel | active | 0 | dev |
| rmac-launcher | active | 1 (`Restart=on-success`, exits when hidden, by design) | dev |
| rmac-app-switcher, rmac-screenshot, rmac-mission-control, rmac-clipboard | active | 0 | dev (user unit copies; not wanted by the installed target, see finding 10) |
| rmac-session-supervisor, rmac-shortcut-broker, rmac-focus, rmac-lock-coordinator, rmac-idle-lock | active | 0 | **old /usr** |
| rmac-mac-keyboard, rmac-setup-assistant, rmac-update-check.timer, rmac-file-chooser, rmac-safe-mode-notice | **not-found** | – | – |
| rmac-lock, rmac-lock-fallback | inactive (started on demand) | 0 | – |

- No unit was failing or restarting in the last hour.
- Every `rmac-component-failure@*` instance is inactive.
- `session-health.json` reports every component as `active` and
  `safe_mode: null`.

## 2. D-Bus

Names the code owns, and whether each is on the bus:

- **Owned:**
  - `org.rmac.Focus1` (old build)
  - `org.rmac.Clipboard1`
  - `org.rmac.NotificationCenter1`
  - `org.rmac.LockScreen1` (lock coordinator)
  - `org.freedesktop.Notifications`
  - `org.freedesktop.impl.portal.desktop.rmac` (Notification backend)
- **Introspection:** each of these returns the expected interface when
  introspected. Examples: `org.rmac.Focus1` exposes
  Activate, Configuration, DeliveryPolicy, Disable, ReplaceConfiguration,
  SetEnabled, Settings, State and Changed. The portal backend exposes
  `impl.portal.Notification` v2 with its SupportedOptions.
- **Absent:** `org.freedesktop.impl.portal.desktop.rmac.filechooser`
  (finding 6) and `…rmac.wallpaper` (finding 17).
- **Per-app `org.rmac.<App>.Menu` names** appear only while an app runs, as
  designed.
- **Portal selection:** with `XDG_CURRENT_DESKTOP=rmac:niri`, the installed
  `rmac-portals.conf` routes only Notification to rmac. Everything else goes
  to `gnome;gtk`: FileChooser (finding 6), Settings (read by
  `rmac-appearance-portal`, which is correct), Screenshot (niri owns
  `org.gnome.Shell.Screenshot`), GlobalShortcuts (finding 16) and Wallpaper.

## 3. niri

- **Includes:** the live `session.kdl` includes `config.kdl` (which includes
  `dev-shell.kdl`) and `shortcuts-generated.kdl`.
- **Differences from the repository:** apart from the dev path substitution,
  `dev-shell.kdl` differs from `packaging/rmac-session/shell.kdl` only in the
  OSD keys (finding 15). The live `config.kdl` differs from the repository's
  only in its include target and the duplicate parking workspace
  (finding 14).
- **Bindings:** every spawn target in the dev config exists.
- **Layer namespaces on screen:** `rmac-top-bar-4`, `rmac-wallpaper-4`,
  `rmac-dock-4`, `rmac-dock-material-4` and `rmac-osd-4`.
- **Rules:** layer rules cover the material and panel namespaces:
  `rmac-(launcher|quick-settings|notification-center|app-drawer|dock-material|menu-material)`,
  plus `rmac-app-switcher`, `rmac-mission-control`, `rmac-quick-settings`,
  `rmac-app-drawer`, `rmac-dock-material-` and `rmac-osd-`. The top bar,
  plain Dock and wallpaper need no rule, and the launcher, Control Center,
  Notification Center and Apps windows have window rules. No namespace the
  code creates is missing a rule it needs.

## 4. Desktop entries and MIME

- **Repository:** 13 entries in `packaging/rmac-apps/applications`, all with
  `Exec=/usr/bin/…` and icons named `org.rmac.<App>`.
- **Laptop:** 7 system entries (old package) and 6 user overrides. Every
  installed entry's Exec target exists and its icon resolves (in
  `~/.local/share/icons/hicolor/scalable/apps`). The missing apps and stale
  targets are finding 9.
- **Dock pins:** Files, Apps, Notes, Text Editor, Terminal and System
  Settings. All of them are installed.
- **MIME defaults on the laptop:**
  - `text/plain` → Text Editor ✓
  - `image/png`, `image/jpeg` → Loupe ✗ (fixed in the package, commit 1ccf4606)
  - `application/pdf` → Papers ✗ (fixed in the package)
  - `inode/directory` → Nautilus ✗ (finding 8)
  - zip/tar/gz → Nautilus ✗. The package's `rmac-mimeapps.list` already maps
    these to Archive Utility, but the Sep-20 package predates it.
  - `audio/mpeg`, `video/mp4` → none (Media Player is not installed).

## 5. Idle behaviour and budgets

See [`docs/perf/reference-laptop-2026-09-24.md`](perf/reference-laptop-2026-09-24.md),
which replaces the smoke-only record. Over budget:

- **Shell:** combined 2.78% (budget 1%), led by the top bar at 1.98%.
  - Suspected idle redraw: top bar, Dock, wallpaper, OSD (finding 4).
  - Broker (fixed, finding 3).
  - Supervisor at 7 wake-ups/s and Focus at 3.8/s (both old builds).
  - Clipboard at 0.5/s.
- **Apps, idle CPU:** System Settings 24.95%, Files 5.32%, System Monitor
  3.13%, Clock 1.58%, Weather 0.67%, Text Editor 0.65%.
- **Apps, warm launch p95 (measured under build contention):** Notes 719 ms,
  Terminal 584 ms (within its 900 ms budget), System Monitor 532 ms, System
  Settings 520 ms.
- **Not measured:**
  - Apps: exits at once because the resident drawer owns it; this is a harness
    limitation.
  - Preview: shows a file panel through GNOME's portal, not a window.

## 6. Logs (07:30–10:00 IST, all rmac units and niri)

| Count | Message | Trace |
|---:|---|---|
| 45 | `rmac-top-bar: could not activate org.rmac.{Files,TextEditor} menu command: application menu bus is unavailable` | Pre-08:46 top-bar builds. The message no longer exists; `crates/rmac-app-menu/src/lib.rs` now maps an unowned name to `NotPublished` and shares one connection (55174eed, 6d3d7699). Resolved. |
| 2 | `could not read org.rmac.TextEditor menus: … ServiceUnknown` | Same window (08:44–08:45), before 6d3d7699. Resolved. |
| 1 | `could not read org.rmac.Calculator menus: … NoReply` | The app quit during a fetch. Benign. |
| 4 | `rmac-shortcut-broker: NIRI_CONFIG is not set to an absolute path` | `crates/rmac-shortcuts/src/bin/broker.rs:26`, target started outside the rmac session (finding 19). |
| 6 | `niri: libinput error: … event processing lagging behind by 26–210 ms` | Compositor starved during builds, 09:47. Environmental. |
| 3 | `gvfsd-trash: Unsupported operation detected on trash directory` | A non-move operation inside `~/.local/share/Trash/files`, at 09:54:12. That is when Files was being exercised, so check whether Files' trash code (`crates/finder/src/trash_store.rs`) ever modifies files in place rather than renaming them (Files agent). |
| 8 | `spice-vdagent.service: Unknown key 'StandardError' in [Install]` | Ubuntu packaging. Not rmac. |

The rmac binaries log almost nothing at info level: in the last hour there
was one rmac line. Errors are visible, but there is no heartbeat, so an
absence of logs does not prove the surfaces are healthy.

## 7. Resource sanity (09:58 IST)

| Process | PSS | fds | threads |
|---|---:|---:|---:|
| rmac-wallpaper | 85.6 MiB | 62 | 22 |
| rmac-top-bar | 61.2 MiB | 57 | 23 |
| rmac-dock | 54.1 MiB | 65 | 23 |
| rmac-osd | 32.8 MiB | 50 | 18 |
| rmac-launcher | 18.4 MiB | 36 | 13 |
| rmac-quick-settings / notification-center / -panel | 13 MiB each | 29–30 | 11–13 |
| rmac-app-drawer / app-switcher / mission-control / screenshot | 9–10.5 MiB each | 24–30 | 11–12 |
| rmac-clipboard, focus, lock-coordinator, shortcut-broker, supervisor, idle-locker | 0.1–2.7 MiB | 3–9 | 1–4 |
| niri | 26.5 MiB | 199 | 24 |

- The OSD holds 33 MiB and 18 threads for a surface that is almost never
  visible. It is worth checking whether it can drop its GPU context while
  hidden.
- The fd counts are all bounded. No fd growth was seen between samples.
- **Zombies:** the top-bar dispatcher zombie (fixed, finding 13) and niri's
  spawner zombies (finding 20).
- **Temporary files:** 19 `systemd-private-*` directories, one per running
  `PrivateTmp=yes` unit, as expected. `/run/user/1000/rmac` holds one socket
  per running surface and no stale sockets.
- **Disk:** `~/.cache` is 32 MiB. `~/.local/state/rmac` is 7.5 GiB, all of it
  the dev-install archive (finding 18).

## Fixes made (branch `system-audit`)

- **b9b40542:** menu-bar commands run to exit, so there are no zombies and a
  non-zero exit status is logged.
- **1ccf4606:** Preview and Text Editor are the rmac MIME defaults for every
  type they claim. The package test enforces this.
- **0a861f83:** the shortcut broker backs off (2 s doubling to 5 min, reset
  after a real bind) and only re-announces the fallback when its reason
  changes. There is a unit test for the schedule.
- **72d11255:** first real budget run recorded, the harness covers Calculator,
  Clock, Weather and Preview, and the beta checklist is updated.

## Needs the owner or a sudo install

1. Install the current `rmac-session` and `rmac-apps` packages. This fixes
   findings 6, 7 (live), 9, 10, 11 and 12, and installs `rmac-portals.conf`
   with the rmac FileChooser.
2. Install `/etc/keyd/rmac.conf`, or accept that Mac keyboard shortcuts stay
   off (finding 11).
3. Decide whether to delete the 7.5 GB dev-install archive (finding 18).
4. Coordinator, without sudo:
   - Add dev drop-ins for the five old-build services.
   - Add an `rmac-session.target.d` `Wants=` for the four hand-started units.
   - Point the OSD keys at the dev binary.
   - Remove the duplicate `rmac-parking` workspace from the live
     `config.kdl`.
   - Install dev desktop entries for all 13 apps.
