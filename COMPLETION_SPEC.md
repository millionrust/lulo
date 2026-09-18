# rmac completion spec: the step-by-step build manual

> **Audience:** an autonomous coding agent (Codex, Claude Code, or similar).
> **Purpose:** read this file once, top to bottom, then execute it phase by
> phase until every box in [§14 Definition of Done](#14-definition-of-done)
> is checked.
> **Written against:** branch `dev`, commit `cc553a0` ("Make Finder inline
> rename reliable"), 2026-08-12, after an audit of the whole repository.
> **Companion file:** `FEEL_SPEC.md` (2026-09-18) covers what makes rmac *feel* like a Mac — sound,
> cursor, motion continuity, scrolling physics, typography tuning, foreign dialogs, wording — plus
> the **macOS 27 Golden Gate delta**, the owner's measured Mac profile, and pixel measurements taken
> from the reference Mac's screen. Read it next; its §G says which phase each of its tasks joins.
> **Relationship to other docs:** `GOAL.md` is the *why* and the stopping
> condition. This file is the *exactly what and how*. If they conflict on
> process, `GOAL.md` wins. If they conflict on a concrete visual/behavioral
> value, this file wins **until** a reference-Mac measurement says otherwise,
> and then you update the one token table in §4 (never scattered constants).

---

## 0. How to use this file (read this first, every session)

### 0.1 The loop you run forever

```
1. Open this file. Find the first phase in §3 whose checklist has an unchecked box.
2. Inside that phase, take the first unchecked task.
3. Read the "Files" and "Read first" lines for that task. Open those files.
4. Implement exactly what the task says. Do not add extras.
5. Run the task's "Verify" commands (and only those; see §0.4).
6. If the task is visual: capture the screenshot the task names (§12.3).
7. Tick the box in THIS file ("- [ ]" -> "- [x]") and append the commit hash.
8. Commit (§0.5). Push `dev`.
9. Go to step 1. Never stop because a task is hard. Stop only for §0.6 reasons.
```

### 0.2 Mandatory reading order before the first task

1. `AGENTS.md` (disk and build limits; these are hard rules).
2. `GOAL.md` (objective, principles, never-do list).
3. This file, completely.
4. `docs/macos-parity-spec.md` §0–§4 (reference target, owner profile, reference classes `M0/M1/M2/LE/AO/NA`).
5. `ARCHITECTURE.md` (crate boundaries).

You do **not** need to read every other doc up front. Each task names the docs it needs.

### 0.3 Machines

| Machine | Role | Rules |
|---|---|---|
| Owner's Mac (this checkout, `/Users/jacob/Projects/rmac`) | Edit code, run focused unit tests, macOS visual reference | Low disk. Obey `AGENTS.md`. Never build the upstream GPUI graph here. |
| Ubuntu 26.04 reference PC | The only place Linux/Wayland/niri behavior counts as proven | SSH via `$RMAC_REFERENCE_HOST` / `$RMAC_REFERENCE_KEY`. `git pull --ff-only`. Sequential builds only. |
| Reference Mac settings | Visual + default-settings authority | Screenshots at the same logical scale as rmac captures. |

### 0.4 Validation rules (proportional; do not waste disk or time)

- Before any build likely > 1 GiB: `df -h /System/Volumes/Data` on Mac, `df -h /` on Ubuntu. Stop below 25 GiB free.
- Never run workspace-wide `--all-features` / `--all-targets` locally.
- Default local checks for a change inside crate `X`:
  ```sh
  cargo fmt -p X -- --check
  cargo test --locked -p X
  cargo clippy --locked -p X -- -D warnings
  ```
- Shell workspace (`shell/`, see Phase 0) builds **only on Ubuntu**:
  ```sh
  cd shell && cargo build --locked --release --features wayland --bin <bin>
  ```
- Always also run `bash scripts/check-shared-controls.sh` when touching app UI.
- Run `python3 scripts/run-release-contract-checks.py` when touching `packaging/`, `scripts/*.json`, or release docs.

### 0.5 Git rules (from `GOAL.md`; do not deviate)

- Work on `dev`. Commit author `Jacob Samas <samasjacob@icloud.com>`. **No co-author line.**
- `git status` before every edit, stage, commit, pull, push. Stage only files for the current task.
- One task, or one tight group of tasks, per commit. Message = user-visible outcome, imperative mood
  (e.g. `Render Control Center modules as Tahoe tiles`).
- Never force-push, `reset --hard`, rewrite history, or delete untracked files you did not create.

### 0.6 The only reasons to stop and ask the owner

1. A task is marked **OWNER** and the default in §2 is unacceptable for a reason you can prove.
2. You need a credential/permission not already available (signing key, SSH, repository hosting).
3. Disk would fall below the floor and no safe generated artifact can be removed.
4. A visual choice cannot be settled from reference-Mac captures (subjective taste).

A compiler error, failing test, flaky runtime, or missing API is **not** a reason to stop. Fix it.

### 0.7 Hard "never" list (violating any = revert your commit)

- No fake data: fake Wi-Fi networks, fake battery %, fake search results, fake notifications, switches that change only UI state.
- No Apple assets: Apple logo, SF Pro/SF Mono/Menlo/Helvetica **by name as a required font**, Apple wallpapers, sounds, app icons.
- No Apple service names presented as working: iCloud, AirDrop, Handoff, FaceTime, Siri, Apple Intelligence, Time Machine, FileVault, Gatekeeper, Touch ID.
- No global compositor blur over app content. Blur only behind the surface that owns it.
- No second bar, Dock, notification daemon, or shortcut handler in the rmac session.
- No blocking I/O, D-Bus, filesystem, or subprocess work on the UI thread.
- No polling where an event stream exists. No continuous redraw when idle.
- No removing GNOME, no weakening the lock screen, no broad polkit rules.
- No invented completion percentages in docs or commit messages.

---

## 1. What exists today (audited facts; do not re-derive)

### 1.1 Repository shape

- ~260k lines of Rust in `crates/` (≈90 crates) plus `experiments/gpui-upstream-lab` (5.3k lines).
- Main workspace pins `gpui = "=0.2.2"`, `gpui-component = "=0.5.1"`.
- `experiments/gpui-upstream-lab` is a **separate** workspace pinned to Zed git rev
  `76c93968da5b8b8809bdd72e4ad9e7d0e946bad0`, with `wayland` feature; it contains the only
  **layer-shell** surfaces: `top_bar`, `dock`, `wallpaper`, `osd`, plus `a11y`/`layer_shell` probes.
- Domain/runtime/system crates are **GPUI-free** (good). GPUI is used only by:
  `activity-monitor, app-drawer, component-gallery, finder, launcher-app, notes,
  notification-center-app, platform-lab, quick-settings-app, rmac-editor, rmac-ui,
  system-settings, terminal, text-editor`.

### 1.2 Surfaces and their current rendering path

| Surface | Binary / crate | Rendering today | Problem |
|---|---|---|---|
| Wallpaper | `experiments/.../wallpaper.rs` | layer-shell, upstream GPUI | lives in "experiments" |
| Menu bar | `experiments/.../top_bar.rs` + `rmac-top-bar` model | layer-shell | hard-coded colors in `lib.rs::visuals`, not shared tokens |
| Dock | `experiments/.../dock.rs` + `rmac-dock*` | layer-shell | same; Dock settings model lacks size/recents/indicators/animation fields |
| OSD | `experiments/.../osd.rs` + `rmac-osd` | layer-shell | OK shape; tokens not shared |
| Spotlight | `crates/launcher-app` | **xdg window** positioned by niri `window-rule` | as a normal window it can show up in niri window lists and switchers, can't reliably open on the focused output or above full-screen windows, and its UI is an app browser with category pills rather than Tahoe Spotlight |
| Control Center | `crates/quick-settings-app` | **xdg window** | generic cards + mute *Toggle* switch; not Tahoe modules; no customization |
| Notification Center | `crates/notification-center-app` | **xdg window** | not layer-shell; no widgets area design |
| Notification banners | `crates/rmac-notifications-linux/src/banner.rs` | **planning/model only; no host renders it** (`surfaces.rs`: "The eventual Linux host translates…", namespace `rmac-notification-banners`, card radius 14) | banners never appear on screen |
| Apps (Launchpad) | `crates/app-drawer` | xdg window | product name "App Drawer" contradicts Tahoe "Apps" |
| Lock screen | `crates/rmac-lock-provider-linux` | custom Wayland renderer (`paint.rs`, `text_renderer.rs`, family `"Inter"`) | visual design not specified against Tahoe |
| Windows | niri `window-rule` in `packaging/rmac-session/shell.kdl` | floating, radius 12 | traffic lights drawn per app by `rmac_ui::traffic_lights()` |

### 1.3 Visual system facts

- `crates/rmac-ui/src/theme.rs` holds semantic tokens (colors light/dark/high-contrast, typography 13/12/11/15/22, spacing 4–32, radii control 8 / card 12 / popover 20 / large 24 / pill 30, materials, metrics, elevation, motion).
- `experiments/gpui-upstream-lab/src/lib.rs::visuals` holds a **second, divergent** palette (`TOP_BAR_TINT`, `DOCK_TINT`, `MENU_RADIUS 9`, `DOCK_RADIUS 26`, ...).
- `rmac_ui::UI_FONT = "Inter"`, `MONO_FONT = "JetBrains Mono"` — **neither font is bundled or declared as a package dependency**, so text silently falls back.
- `crates/terminal/src/controller.rs:67` uses `const FONT: &str = "Menlo"` (a macOS-only font) and a hard-coded `CELL_W = FONT_SIZE * 0.6`.
- Size conflicts: `docs/macos-ui-reference.md` says menu bar 32 px and Dock icons 48 px; newer code (commits `9c59b7e`, `c983474`, same day, later) uses menu bar **26** and Dock icon **56**. §2 resolves this.

### 1.4 Behavior facts

- Global shortcuts actually registered: only `Mod+Space` (launcher) and `Mod+Ctrl+Q` (lock), plus niri binds in `shell.kdl` (Ctrl+←/→ spaces, Ctrl+↑ overview, Mod+Ctrl+F fullscreen, media keys → `rmac-osd`, Mod+Tab / Mod+grave MRU).
- First-party app shortcuts use GPUI's portable modifier (`cmd-*` → **Ctrl on Linux**), while menus print `⌘`. Mismatch.
- `rmac-shell-settings::ShellSettings::default().pinned_apps` = Files, App Drawer, Firefox, Terminal, Notes, System Settings.
- `DockSettings` has: placement, outputs, autohide, magnification(+scale), reserve_space, repeated_click. **Missing:** tile size, show recent apps, show indicators, minimize effect, minimize-into-app-icon, animate opening apps, double-click titlebar action, show suggested/recent in Dock.
- `IndicatorSettings` are booleans. Tahoe uses **Always / When Active / Never** per control.
- Finder has very deep backend work (trash store, operation journal, undo journal, conflicts, quick look) — its latest commits are UI interaction polish.
- System Settings has real authorities for most panes (see `docs/system-settings-audit.md`), sidebar width 248.

---

## 2. Frozen decisions (do not re-open; the owner can override by editing this section)

| ID | Decision | Why |
|---|---|---|
| **FD-1 Shell framework** | All *shell surfaces* (wallpaper, menu bar, Dock, OSD, Spotlight, Control Center, Notification Center, banners, Apps, app switcher, screenshot overlay, Force Quit, logout/shutdown dialog) run on the pinned upstream GPUI rev `76c93968…` as **wlr-layer-shell** surfaces in one promoted workspace `shell/`. Applications (Files, Settings, Terminal, Notes, Text Editor, System Monitor) stay on `gpui 0.2.2 + gpui-component 0.5.1` until Phase 10. | Overlays must never appear in window MRU, must open on the focused output, must stack above full-screen windows, and must dismiss on click-away. Only layer-shell guarantees this on niri. Domain crates are already GPUI-free, so hosts are thin. |
| **FD-2 One token source** | Create GPUI-free crate `crates/rmac-design` holding every token as plain data (`u32` RGBA, `f32`). Both `rmac-ui` (stable GPUI) and `shell/` (upstream GPUI) convert from it. Delete `experiments/.../lib.rs::visuals` constants after migration. | Two palettes guarantee visual drift. |
| **FD-3 Canonical sizes** | Measured on the reference Mac 2026-09-18 (1920×1080 @ 1×, macOS 27.0): menu bar height **29** logical px; Dock rendered tile **64** px with **76 px** pitch (12 px gap), shelf ≈ 72 px tall sitting ≈ 18 px above the screen edge, indicator dot 4 px below the shelf. The Settings size slider keeps the macOS mapping (the owner's `tilesize` preference of 78 renders as 64). See `FEEL_SPEC.md` §C.2. | Real pixels from the owner's screen beat both the old docs and my earlier guesses. |
| **FD-4 Command key** | `Super` is ⌘ Command. In rmac apps, every `⌘X` shortcut binds `super-x` **and** keeps `ctrl-x` as a silent alias. Exception: Terminal passes `ctrl-*` to the PTY and uses only `super-*` for app commands (`super-c` copy, `super-v` paste, `super-t` new tab). Menus print `⌘`. Third-party apps are untouched (no global key remapping) until optional Phase 10.4. | macOS muscle memory without breaking Linux apps or terminal signals. |
| **FD-5 Fonts** | UI font **Inter** (OFL) via package dependency `fonts-inter`; monospace **JetBrains Mono** via `fonts-jetbrains-mono`. Add both to native package `Depends:`. Terminal uses `rmac_ui::MONO_FONT`. Measure cell width from the font, never a constant ratio. | Only licensed, installable fonts; no fallback surprises. |
| **FD-6 Naming** | Visible names: **Files** (app id `org.rmac.Files`), **Apps** (was "App Drawer"), **Spotlight**-equivalent visible name **Search** in menus/Settings (internal crate names stay), **Control Center**, **Notification Center**, **System Settings**, **System Monitor**, **Terminal**, **Notes**, **Text Editor**. Crate/package names do not change. | Tahoe hierarchy without Apple trademarks for services; generic nouns are fine. |
| **FD-7 Default profile** | Dock pins: keep today's `ShellSettings::default()` order exactly (Files, Apps, Firefox, Terminal, Notes, System Settings). Right of separator: Trash only. No Downloads, no recent apps, magnification off, autohide off, bottom placement. Menu bar: Wi-Fi Always, Battery Always on laptops, Bluetooth When Active, Sound When Active, Focus When Active, Control Center, Search icon, clock with date. | Matches the owner's reference profile rules in the parity spec. |
| **FD-8 Global menu** | First-party apps export `org.rmac.AppMenu1` (exists). Add GTK `org.gtk.Menus`/`org.gtk.Actions` import in Phase 5. Apps exporting nothing show only the bold app name and an app menu containing *About {app}* (disabled if unknown), *Hide*, *Quit* (via niri close-window for all its windows). Never invent File/Edit/View. | Truthful global menu. |
| **FD-9 Motion** | One easing set (§4.9). Every animation is disabled or replaced by a ≤ 100 ms opacity fade under Reduce Motion. No animation runs when nothing changes. | Mac feel without idle cost. |
| **FD-10 Evidence** | A task that changes pixels is not done without a screenshot pair (rmac + reference Mac) stored under ignored `target/evidence/<phase>/<task>/` and summarized (not committed) in the task's tick line. | "Looks close" is not proof. |

---

## 3. Phase map (execute strictly in order)

| Phase | Name | Outcome the owner can see |
|---|---|---|
| 0 | Foundation cleanup | One shell workspace, one token crate, fonts installed, names fixed |
| 1 | Design system and components | Every control looks and behaves like Tahoe in both themes |
| 2 | Session and windows | Log in once → full desktop; windows, traffic lights, focus, Spaces feel like a Mac |
| 3 | Menu bar and system menu | Transparent adaptive bar, real app menus, status items, clock |
| 4 | Dock | Tahoe Dock with all Desktop & Dock settings |
| 5 | Overlays | Spotlight, Control Center, Notification Center + banners, OSDs, Apps, app switcher, Force Quit, screenshots |
| 6 | Lock, login, power | One password, Tahoe lock screen, logout/restart/shutdown dialogs |
| 7 | First-party apps | Files, System Settings, Terminal, Notes, Text Editor, System Monitor completed |
| 8 | Accessibility, performance, resilience | Orca, keyboard-only, 200%, budgets, soak |
| 9 | Install, update, remove, release | Signed APT, clean install, rollback, public README/demo |
| 10 | Optional after 1.0 | Calculator, Clock, Preview, app GPUI migration, key remapping |

Each phase section below has: **Goal**, **Read first**, numbered **Tasks** (each with Files, Do, Verify),
and an **Exit checklist**. Do not start phase N+1 while phase N's exit checklist has an open box,
unless the box is explicitly marked *(may defer)*.

---

## 4. Design system: every token (implemented in `crates/rmac-design`)

**Rule of provenance.** Values marked **R** already exist in the repo and were set from reference
captures; keep them. Values marked **S** are starting values written from Tahoe knowledge; in
Phase 1 task 1.1 you measure them against the reference Mac and overwrite in place. After that,
nobody hard-codes a color, size, radius, duration, or font size anywhere else. A `grep` gate (task 1.9)
enforces it.

All sizes are **logical pixels** at scale 1. All colors are `0xRRGGBBAA`.

### 4.1 Color: labels and fills

| Token | Light | Dark | Src |
|---|---|---|---|
| `label.primary` | `1D1D1FFF` | `F5F5F7FF` | R |
| `label.secondary` | `66666CFF` (HC `48484DFF`) | `B9B9BFFF` (HC `D4D4D8FF`) | R |
| `label.tertiary` | `6E6E73FF` (HC `5A5A60FF`) | `98989FFF` (HC `B8B8BDFF`) | R |
| `label.quaternary` | `00000040` | `FFFFFF40` | S |
| `label.disabled` | `00000040` | `FFFFFF40` | S |
| `label.placeholder` | `0000004D` | `FFFFFF4D` | S |
| `separator` | `00000018` (HC `00000042`) | `FFFFFF20` (HC `FFFFFF4D`) | R |
| `separator.opaque` | `E5E5E5FF` | `38383AFF` | S |
| `fill.hover` | `0000000A` (HC `14`) | `FFFFFF0E` (HC `18`) | R |
| `fill.control` | `E9E9ECFF` | `3A3A3EFF` | R |
| `fill.control.hover` | `DEDEE2FF` | `4A4A4FFF` | R |
| `fill.control.pressed` | `D2D2D7FF` | `55555AFF` | S |
| `selection.focused` | `accent` | `accent` | R |
| `selection.unfocused` | `00000014` (HC `28`) | `FFFFFF1C` (HC `30`) | R |
| `selection.text` (text highlight) | accent @ `40` | accent @ `55` | S |
| `row.alternate` | `F4F5F5FF` | `242427FF` | R |
| `focus.ring` | accent @ `80` | accent @ `99` | S |

### 4.2 Color: surfaces (content layer, opaque)

| Token | Light | Dark | Src |
|---|---|---|---|
| `surface.window` | `FFFFFFFF` | `1E1E20FF` | R |
| `surface.chrome` (toolbar/titlebar fallback) | `F6F6F6FF` | `29292CFF` | R |
| `surface.sidebar.opaque` | `F2F2F2FF` | `242426FF` | R |
| `surface.raised` (cards, grouped rows) | `FFFFFFFF` | `323236FF` | R |
| `surface.grouped.background` (Settings detail pane) | `F5F5F7FF` | `1C1C1EFF` | S |
| `surface.grouped.row` | `FFFFFFFF` | `2C2C2EFF` | S |
| `scrim` | `00000038` | `00000070` | R |

### 4.3 Color: system palette (for accents, tags, badges, charts)

| Token | Light | Dark |
|---|---|---|
| `system.blue` (default accent) | `007AFFFF` | `0A84FFFF` |
| `system.purple` | `AF52DEFF` | `BF5AF2FF` |
| `system.pink` | `FF2D55FF` | `FF375FFF` |
| `system.red` | `FF3B30FF` | `FF453AFF` |
| `system.orange` | `FF9500FF` | `FF9F0AFF` |
| `system.yellow` | `FFCC00FF` | `FFD60AFF` |
| `system.green` | `34C759FF` | `30D158FF` |
| `system.teal` | `30B0C7FF` | `40C8E0FF` |
| `system.indigo` | `5856D6FF` | `5E5CE6FF` |
| `system.brown` | `A2845EFF` | `AC8E68FF` |
| `system.gray` | `8E8E93FF` | `8E8E93FF` |
| `danger` | `D70015FF` | `FF6961FF` (R) |
| `warning.background/border/text` | `FFF6DA / EEDCA0 / 7A5C00` | `3A321E / 756225 / FFD76A` (R) |
| `notes.accent` | `FFC40CFF` | `FFD60AFF` (R) |

Accent choices shown in Appearance: Multicolor(=blue), Blue, Purple, Pink, Red, Orange, Yellow, Green, Graphite(`8E8E93`). `on_accent` = whichever of white/black has higher contrast (R, existing logic).

### 4.4 Color: traffic lights (original drawing, standard semantic colors)

| Button | Fill | Border (0.5 px, inside) | Glyph on hover (`00000099`) |
|---|---|---|---|
| Close | `FF5F57FF` | `E0443EFF` | × |
| Minimize | `FEBC2EFF` | `DEA123FF` | − |
| Zoom/Full screen | `28C840FF` | `1AAB29FF` | two diagonal triangles (full screen), `+` when Option held |
| Inactive window (all three) | light `DDDDDDFF`/border `C8C8C8FF`; dark `4E4E50FF`/border `3E3E40FF` | | none |
| Disabled button | inactive colors at 50% | | none |

Glyphs appear on **all three** when the pointer is over **any** of them (hover group), not one at a time.

### 4.5 Materials (functional glass and overlays)

Materials are *tint + compositor blur behind the surface only*. The compositor (niri `background-effect { blur true }` on layer rule / window rule) supplies blur; the surface paints the tint.

| Token | Tint light | Tint dark | Blur | Border | Used by | Reduce Transparency fallback |
|---|---|---|---|---|---|---|
| `material.menu` | `F6F6F6D9` | `28282BD9` | yes | 0.5 px `0000001A` / `FFFFFF1F` + inner top highlight `FFFFFF66`/`FFFFFF14` | menus, context menus, submenus | `surface.raised` opaque |
| `material.popover` | `F2F2F2CC` | `232326CC` | yes | same as menu | Control Center, NC, Spotlight, calendar popover | opaque `surface.raised` |
| `material.hud` | `FFFFFFB3` | `1C1C1EB3` | yes | same | OSD, banners | opaque |
| `material.dock` | `FFFFFF40` | `00000033` | yes | 0.5 px `FFFFFF59` outer + `00000014` inner | Dock shelf | `F0F0F0F2` / `2A2A2DF2` |
| `material.sidebar` | `F2F2F2E0` (R ratio) | `242426E0` | yes (window rule) | none; right edge `separator` | app sidebars | `surface.sidebar.opaque` |
| `material.menubar` | `00000000` (fully clear) | `00000000` | no | none | menu bar | `F6F6F6F2` / `1E1E20F2` |
| `material.tooltip` | `FAFAFAF2` | `2C2C2EF2` | no | 0.5 px separator | tooltips | same |

High contrast: every tint alpha becomes `F6`, borders become `label.secondary` at 1 px.

### 4.6 Typography (font: Inter; mono: JetBrains Mono)

Size × `TextScale` factor (R logic). Line height = round(size × 1.23). Letter spacing: Inter at ≤ 13 px uses `+0.1 px`, ≥ 20 px uses `-0.2 px` (S).

| Role | Size | Weight | Use |
|---|---|---|---|
| `largeTitle` | 26 | Regular | Settings pane hero, About |
| `title1` | 22 | Regular (R) | Window content titles |
| `title2` | 17 | Semibold | Section headers in Settings detail, Notes title |
| `title3` | 15 | Semibold (R headline) | Popover section titles |
| `headline` | 13 | Semibold | Row titles that need emphasis, menu bar app name (**Bold**) |
| `body` | 13 | Regular (R) | Default UI text, menu items, list rows |
| `callout` | 12 | Regular (R) | Secondary rows, sidebar in small size |
| `subheadline` | 11 | Regular (R caption) | Row subtitles, table headers, tooltips |
| `footnote` | 10 | Regular | Dock/badge counts, timestamps in NC |
| `caption` | 10 | Medium | Group headers in sidebars (tertiary color, not uppercase) |
| `mono.body` | 12 | Regular | Terminal default (profile-adjustable), code previews |
| `clock.menubar` | 13 | Medium (tabular numerals) | Menu bar clock |
| `lock.time` | 96 | Semibold (tabular) | Lock screen time |
| `lock.date` | 20 | Medium | Lock screen date |

### 4.7 Geometry

**Spacing (R):** 4, 8, 12, 16, 20, 24, 32.

**Radii**

| Token | Value | Src | Use |
|---|---|---|---|
| `radius.control` | 8 | R | buttons, text fields, popups |
| `radius.card` | 12 | R | grouped rows, Control Center modules (inner) |
| `radius.popover` | 20 | R | Control Center/NC panels |
| `radius.large` | 24 | R | Spotlight bar, Apps panel |
| `radius.pill` | 30 | R | search pills, toggle pills |
| `radius.menu` | 10 | S (lab had 9) | menus/context menus |
| `radius.menu.item` | 6 | S (lab had 5) | menu item highlight |
| `radius.window` | 16 | S — measure | **One radius for every window**: macOS 27 standardized this, so drop the old "12 normally, 16 with a toolbar" rule here and in `shell.kdl`. |
| `radius.dock` | 26 | R (lab) | Dock shelf at size 56 → scale `radius = tile*0.46` |
| `radius.hud` | 28 | R | OSD |
| `radius.tooltip` | 8 | R | tooltip |
| `radius.cc.module` | 18 | S | Control Center large modules |
| `radius.cc.toggle` | 999 (circle) | S | Control Center circular toggle icons |

**Metrics**

| Token | Value | Src |
|---|---|---|
| `menubar.height` | **29** | **M** measured 2026-09-18 |
| `menubar.item.height` | 22 | R |
| `menubar.item.padding.x` | 8 | S |
| `menubar.leading.inset` | 12 | S |
| `menubar.trailing.inset` | 10 | S |
| `menubar.status.icon` | 16 (hit 22×22) | R |
| `menu.row.height` | **24** | **M** 2026-09-18 (Finder ▸ File menu item pitch) |
| `menu.padding` | 5 vertical, 5 horizontal | S |
| `menu.min.width` / `max.width` | 180 / 420; Finder ▸ File measures **269** | S / **M** |
| `menu.separator` | 1 px line, 9 px total block, inset 10 | S |
| `menu.shortcut.gap` | 24 px min between title and shortcut | S |
| `toolbar.height` | 52 (R) unified; 38 titlebar-only | R/S |
| `sidebar.width` | default 220, min 180, max 320 (Settings keeps 248 R) | S |
| `sidebar.row.height` | 28 (R) | R |
| `list.row.height` | 24 compact / 30 regular (R) | R |
| `table.header.height` | 24 | S |
| `control.height.mini/small/regular/large` | 16 / 20 / 24 / 32 | S |
| `button.padding.x` | 12 regular, 8 small | S |
| `switch.regular` | 38×22, thumb 20 | S |
| `switch.small` | 32×18, thumb 16 | S |
| `switch.mini` | 26×15, thumb 13 (existing 28×16 R is close; replace with measured) | S |
| `slider.track` | 4 high, radius 2 | S |
| `slider.knob` | 20×20 regular, 16×16 small; white with shadow | S |
| `checkbox` | 14×14 radius 4 | S |
| `radio` | 14×14 circle | S |
| `searchfield.height` | 28, radius pill | S |
| `segmented.height` | 24, radius 7 | S |
| `traffic.diameter` | 12 (R), center spacing 20, leading inset 20 in toolbar windows / 8 in titlebar-only; vertically centered | R/S |
| `dock.tile` | **64** rendered default (slider 32–128) | **M** measured 2026-09-18 |
| `dock.gap` / `dock.padding` | **12** / 4–6 (pitch 76) | **M** measured 2026-09-18 |
| `dock.bottom.margin` | **18** above screen edge | **M** measured 2026-09-18 |
| `dock.indicator` | 4×4 dot, centred **below the shelf** (y ≈ shelf bottom + 6) | **M** measured 2026-09-18 |
| `tooltip.padding` | 8×4 | S |
| `focus.ring` | 3 px outside, radius = control radius + 3 (HC 4 px) | S (R had 2) |
| `hit.target.min` | 24×24 (pointer), 44×44 not required on desktop | S |

### 4.8 Elevation (shadows)

| Token | Shadow | Src |
|---|---|---|
| `elev.content` | none | R |
| `elev.raised` | `0 2 8 alpha .10` | R |
| `elev.popover` | `0 6 18 alpha .18` + `0 0 0 .5 px border` | R |
| `elev.modal` | `0 10 28 alpha .24` | R |
| `elev.window.active` | niri: `softness 28 spread 2 offset 0,8 color #00000050` | R |
| `elev.window.inactive` | niri: `softness 18 spread 1 offset 0,4 color #00000030` | S |
| `elev.dock` | `0 8 24 alpha .20` | S |

High contrast multiplies alpha × 1.5 (R pattern).

### 4.9 Motion

| Token | Curve | Duration | Reduce Motion |
|---|---|---|---|
| `motion.fast` | ease-out `cubic-bezier(0.2, 0, 0, 1)` | 120 ms | 80 ms fade (R had 80) |
| `motion.standard` | ease-in-out `cubic-bezier(0.4, 0, 0.2, 1)` | 200 ms | 100 ms fade |
| `motion.deliberate` | ease-in-out | 350 ms | 100 ms fade |
| `motion.spring.popover` | spring response 0.3 s, damping 0.85 | ~300 ms | 100 ms fade, no scale |
| `motion.spring.dock.magnify` | follows pointer every frame, no easing lag | — | magnification disabled |
| `motion.menu.open` | fade 0→1 | 0 ms open (macOS menus appear instantly), 150 ms fade-out on dismiss via selection blink | instant |
| `motion.menu.select.blink` | selected row flashes off/on once | 70 ms × 2 | skip |
| `motion.tooltip.delay` | — | 700 ms first, 0 ms while moving between Dock items | same |
| `motion.osd.hold` | — | 1600 ms (R) then 250 ms fade | fade 100 |
| `motion.banner` | slide in from right 24 px + fade | 350 ms; auto-dismiss after 5 s (banner style) | fade 100 |
| `motion.dock.bounce` | 2 bounces, 20% tile height each | 600 ms per bounce, until app maps or 10 s | none |
| `motion.minimize` | Scale effect: window scales to its Dock target | 350 ms | fade 150 |
| `motion.cc.open` | scale 0.96→1 + fade, anchored top-right | 250 ms | fade 100 |
| `motion.spotlight.open` | fade + 4 px drop | 180 ms | fade 100 |
| `motion.apps.open` | fade + scale 1.04→1 | 300 ms | fade 100 |

### 4.10 Icons and artwork

- Status/system glyphs: original SVG line icons, 16×16 grid, 1.5 px stroke, rounded caps. Store under
  `shell/assets/symbols/`. Required set (create any missing): `wifi-0..3`, `wifi-off`, `wifi-error`,
  `ethernet`, `vpn`, `bluetooth`, `bluetooth-connected`, `battery-0..100` (drawn programmatically:
  outline 25×12 + nub, fill width = %), `battery-charging` (bolt), `speaker-0..3`, `speaker-muted`,
  `mic`, `camera`, `screen-record`, `focus-moon`, `focus-custom`, `control-center` (two toggles glyph),
  `search` (magnifier), `notification-bell`, `brightness-low/high`, `keyboard-brightness`, `airplane`
  (not used unless rfkill all), `display`, `now-playing`, `play`, `pause`, `next`, `prev`, `lock`,
  `power`, `restart`, `sleep`, `logout`, `trash-empty`, `trash-full`, `folder`, `chevron-*`,
  `checkmark`, `xmark`, `plus`, `minus`, `ellipsis`, `sidebar-left`, `grid`, `list`, `columns`,
  `gallery`, `share`, `tag`, `info`, `gear`.
- System menu glyph: original rmac mark (`assets/status/rmac.svg` exists). **Never** an apple.
- App icons: 1024×1024 master SVGs using the Tahoe icon grid (rounded-square "squircle" 824×824
  centered, corner smoothing 0.6, drop shadow baked at 0 12 24 alpha .3). Provide light, dark, and
  tinted variants per app (Phase 1 task 1.7). Existing `packaging/rmac-apps/icons/*.svg` are the
  starting art; rework to the grid.
- Third-party icons: resolve via `rmac-icon` (freedesktop theme lookup). If the icon is not already a
  rounded square, draw it **inside** a white (light) / `2C2C2E` (dark) squircle plate at 80% size so
  the Dock looks uniform (Tahoe does the same for non-conforming icons).

---

## 5. Shared components: exact look and behavior

Implement each in `crates/rmac-ui` (apps) **and** `shell/crates/rmac-shell-ui` (shell) from the same
`rmac-design` tokens. Every component must have: all states below, keyboard behavior, accessibility
role/name/state, light + dark + high contrast + reduced transparency, and a specimen in
`crates/component-gallery`.

State vocabulary used below: *rest, hover, pressed, focused (keyboard), selected, disabled, busy,
error, window-inactive*.

### 5.1 Push button

- Regular height 24, radius 8, padding-x 12, `body` text, min width 72 for dialog buttons.
- **Default (primary)** button: fill `accent`, text `on_accent`; in an inactive window fill becomes `fill.control` and text `label.primary`.
- **Secondary:** fill `fill.control`, 0.5 px `separator` border, subtle top highlight `FFFFFF80` (light only).
- **Destructive:** secondary style with `danger` text; only becomes red-filled inside a destructive alert's default button.
- **Ghost/borderless (toolbar):** no fill at rest; `fill.hover` on hover; `fill.control.pressed` pressed; icon 16 in 28×28.
- Pressed: darken fill by `fill.control.pressed`. Busy: label replaced by 14 px spinner, width locked.
- Keys: Space activates focused button; Return activates the **default** button of the window/sheet; Escape activates Cancel.
- a11y: role `button`, name = label or tooltip, state disabled.

### 5.2 Pop-up button and pull-down

- Looks like a secondary button with trailing double-chevron (`chevron-up-down` 10 px, `label.secondary`).
- Opens a **menu** (5.9) positioned so the current item overlaps the button (pop-up) or below (pull-down).
- Keys: Space/Return/↓ opens; type-select jumps to item.

### 5.3 Switch (toggle)

- Sizes from §4.7. Off track `fill.control` (+0.5 px inner border `0000001A`); on track `accent`; thumb white `FFFFFFFF`, shadow `0 1 2 alpha .25`.
- Thumb slides 150 ms `motion.fast`; Reduce Motion: instant.
- Click anywhere on track toggles; dragging thumb past midpoint toggles on release.
- Pending state (async authority): thumb shows 10 px spinner, control disabled, keep old value until readback.
- a11y: role `switch`, checked state.

### 5.4 Checkbox and radio

- 14×14; off: `surface.raised` fill + 0.5 px `0000004D` border; on: `accent` fill + white checkmark (2 px) / white 6 px dot.
- Mixed: horizontal white bar. Label to the right, 6 px gap, clicking label toggles.

### 5.5 Slider

- Track 4 high: filled part `accent`, rest `fill.control`. Knob 20 white circle, shadow `0 1 3 alpha .3`, 0.5 px border `00000026`.
- Control Center style (§8.3): **thick** slider 28 high, radius 14, fill `FFFFFF` (dark: `FFFFFFE6`), leading symbol inside the track.
- Keys: ←/→ step 1/20th, Page Up/Down 1/5th, Home/End min/max. Mutations coalesced to ≤ 1 authority write / 50 ms; final value always written on release.
- a11y: role `slider`, value, min, max.

### 5.6 Text field and search field

- Text field: height 24, radius 6, fill `surface.window`, 0.5 px border `00000026`, inner top shadow `0 0.5 0 00000014`, padding-x 6.
- Focus: 3 px `focus.ring` outside + border `accent`.
- Placeholder `label.placeholder`. Error: border `danger` + message below in `subheadline danger`.
- Search field: height 28, radius pill, fill `fill.control` (no border), leading `search` 13 px `label.secondary`, trailing clear button (`xmark.circle.fill` 14 px) only when non-empty. Escape clears; second Escape blurs.
- IME: preedit underline rendered; candidate window anchored at caret (GPUI IME API).

### 5.7 Segmented control

- Height 24, radius 7, background `fill.control`; selected segment raised white (`surface.raised`) pill with `elev.raised`; dark: `5A5A5E` pill.
- ←/→ move selection when focused. Each segment accessible as `radio`.

### 5.8 Sidebar (source list)

- Material `material.sidebar`, full height, sits **under** the unified toolbar (traffic lights inside the sidebar area).
- Section header: `caption` in `label.tertiary`, 20 px tall, 18 px left inset, hover reveals "Hide/Show" text button on the right (collapsible sections).
- Row: 28 tall, radius 6, inset 10 from sidebar edges, icon 16 in `accent` color (Files) or full-color squircle 20 (Settings), label `body`.
- Selected row: focused window & sidebar focused → `accent` fill + white text/icon; otherwise `selection.unfocused` fill + normal text.
- Drag target highlight: 2 px `accent` inner border.
- Keys: ↑/↓ move, Return/Space open, ⌘1…⌘9 not bound here (apps decide).

### 5.9 Menu (menu bar menus, context menus, pop-up menus)

Layout of one row (left→right): 6 px, **checkmark column 14** (✓ / • / – for mixed), 4 px, icon 16 + 6 px **only for app/file rows** (macOS 27 removed icons from ordinary menu items), title (`body`), flexible gap ≥ 24, shortcut text (`body`, `label.secondary`, glyphs `⌃⌥⇧⌘` in that order, key letter uppercase), submenu chevron `›` 10 px, 10 px.

- Panel: `material.menu`, radius `radius.menu`, padding 5, `elev.popover`.
- Hover/keyboard highlight: `accent` fill with radius `radius.menu.item`, text/shortcut turn `on_accent`.
- Disabled row: `label.disabled`, no highlight on hover.
- Separator: 1 px `separator`, 4 px above/below, inset 10.
- Section header (non-interactive): `subheadline` semibold `label.secondary`.
- Title ending in "…" when the command needs more input (Save As…, Get Info is not …).
- **Opening:** appears instantly (no animation). **Choosing:** row blinks (`motion.menu.select.blink`), then fades 150 ms, then the action runs.
- Dismiss: Escape, click outside, owning app deactivates, output removed.
- Submenu opens after 200 ms hover or → key; closes on ← key; pointer moving diagonally toward the submenu does not close it (safe triangle).
- Type-select: typing letters jumps to first matching enabled row.
- Menu never extends off-screen: flip up / shift left; scroll arrows if taller than the output.
- a11y: `menu`, `menuitem`, `menuitemcheckbox`, `menuitemradio`, `separator`; shortcut exposed as `keyshortcuts`.

### 5.10 Popover

- `material.popover`, radius `radius.popover` (Control Center/NC) or 12 (in-app popovers), arrow **none** for shell popovers (Tahoe), 8 px arrow for in-app popovers pointing at the source control.
- Dismiss on Escape and click-away; never steals focus from the app unless keyboard-invoked.

### 5.11 Sheet, alert, dialog

- **Sheet:** slides down from the parent window's toolbar bottom edge (`motion.standard`); width ≤ parent − 40; radius 12; parent dims with `scrim` at 50% of token alpha; parent input blocked, other windows usable.
- **Alert (Tahoe style):** 260 px wide, centered on parent window (or screen for app-modal without parent), app icon 56 on top, title `headline` centered, message `callout` centered, buttons stacked **vertically full-width** when labels are long, otherwise horizontally right-aligned; default button accent; destructive button text `danger`; "Don't ask again" checkbox optional.
- Never a free-floating centered box for a document decision: use a sheet.

### 5.12 Tooltip

- Delay 700 ms, hides on pointer down or leave. `material.tooltip`, radius 8 (R), `subheadline`, max width 300, placed 18 px below pointer (in-app) or above Dock item (Dock).

### 5.13 Table / list

- Header: 24 tall, `subheadline` `label.secondary`, 1 px bottom `separator`; clicking toggles sort (chevron ▲▼ 8 px); drag to resize, min column 40; right-click header → column visibility menu.
- Rows 24 (compact) — alternating `row.alternate` **only** in tables (Files list view, System Monitor), not sidebars.
- Selection per 5.8 rules. Multi-select: ⌘-click toggle, ⇧-click range, ⌘A all, rubber-band in icon views.
- Type-ahead selects by first column.

### 5.14 Toolbar (unified titlebar)

- Height 52, background transparent over window material; draggable except over controls; double-click empty area runs the "double-click titlebar" setting (Zoom default).
- Items: grouped glass capsules (Tahoe): each group height 32, radius 16, fill `FFFFFF80` light / `FFFFFF14` dark, 0.5 px border, icon buttons 28×28 inside.
- Title (when shown): `headline` left-aligned after navigation group, subtitle `subheadline label.secondary` below.
- Narrow width: overflow chevron `»` menu holds hidden items.
- Right-click toolbar → "Customize Toolbar…" sheet (Files only in 1.0).

### 5.15 Progress, spinner, empty state, toast

- Determinate bar: 4 high radius 2, fill `accent`, track `fill.control`.
- Spinner: 12 radial spokes, 1 rev/s, 16 or 32 px. Stops rendering when hidden (no idle redraw).
- Empty state: centered 48 px symbol `label.tertiary`, title `title3`, message `callout label.secondary`, optional button.
- In-app toast is **not** a macOS idiom: replace existing `Toast` usages with inline status text or an alert; keep the component only for recoverable background errors inside one pane.

### 5.16 Traffic lights component

- Exactly §4.4. Hit area 20×24 (R). Hover group reveals glyphs. Option key toggles zoom glyph `+`.
- Click close = close window (not quit); minimize = minimize to Dock (§4.9 minimize); zoom = toggle full screen; ⌥-click zoom = fill screen (maximize) without full screen.
- Long-press/hover 1 s on zoom → tiling menu (Tahoe): *Fill*, *Center*, *Move & Resize › Left/Right/Top/Bottom/Quarters*, *Full Screen Tile › Left/Right*, *Move to {display}*. Implement via niri IPC actions (Phase 2).
- Inactive window: gray per §4.4.

---

## 6. Phases 0–2: foundation, design system, session and windows

### Phase 0 — Foundation cleanup

**Goal:** one shell workspace, one token crate, installed fonts, correct names, no contradictory docs.
**Read first:** `experiments/gpui-upstream-lab/README.md`, `docs/decisions/0001-gpui-linux-gate.md`, `docs/decisions/0002-gpui-version-policy.md`, `scripts/linux/install-upstream-shell-candidate.sh`, `scripts/linux/build-native-packages.py`.

- [x] **0.1 Record the framework decision.** ([ADR 0006](docs/decisions/0006-shell-and-app-framework-split.md)) — `214589b`
  Files: new `docs/decisions/0006-shell-and-app-framework-split.md`.
  Do: write FD-1 as an ADR: context (stable 0.2.2 lacks layer-shell/a11y proof), decision (shell on rev `76c93968…`, apps on 0.2.2), consequences, upgrade policy (bump rev at most monthly after the Phase 8 smoke passes), rollback (previous rev manifest). Link it from `README.md` and `GOAL.md` Checkpoint 0.
  Verify: `python3 scripts/verify-documentation.py`.

- [x] **0.2 Promote the lab into `shell/`.** (`git mv` to a `crates/*`+`bins/*`+`probes/*` workspace with `rmac-shell-ui`/`rmac-shell-layer`; all 15 path references updated) — restructure `2cd384e`, package-collision fix `21adab5`, lockfile+`Tracker::is_empty` `29d2580`, clippy `ada44b5`, `rmac-design` dep `21d3a08`. Ubuntu: `cargo build --locked --release --features wayland --bins` ✔ (3m32s), debug `--locked` ✔, `cargo clippy --locked --bins --features wayland -- -D warnings` ✔, `cargo test --locked --lib` ✔ (12), installer `--check` ✔. **Caveat:** `shell/scripts/nested-wayland-smoke.sh` cannot pass: it runs nested Sway, but `rmac-dock-runtime` has required a niri socket since `b3c09fc` (2026-08-02, before the base commit) — pre-existing, not caused by the move.
  Files: `git mv experiments/gpui-upstream-lab shell`; update every path reference (`rg -l "experiments/gpui-upstream-lab"` → fix all, including `.github/workflows/ci.yml`, `scripts/linux/*.sh`, `scripts/linux/*.py`, `packaging/`, docs, root `Cargo.toml` `exclude`).
  Restructure inside `shell/` into a workspace:
  ```
  shell/
    Cargo.toml                 # [workspace] members = ["crates/*", "bins/*"]; same pinned gpui rev
    crates/rmac-shell-ui/      # tokens→gpui conversion, menus, popover, tooltip, glass, symbols
    crates/rmac-shell-layer/   # layer-surface helpers: anchors, exclusive zone, keyboard interactivity, per-output spawn
    bins/rmac-wallpaper/       # from src/bin/wallpaper.rs
    bins/rmac-menubar/         # from src/bin/top_bar.rs
    bins/rmac-dock/            # from src/bin/dock.rs
    bins/rmac-osd/             # from src/bin/osd.rs
    probes/a11y, probes/layer-shell   # keep as non-product bins
    assets/                    # symbols, osd, status (moved)
    compat/ztracing/
  ```
  Keep binary names that `packaging/` and systemd units already expect (check `packaging/rmac-session/*` and `scripts/linux/install-session-units.sh`; if a unit expects `rmac-top-bar`, keep that binary name).
  Verify (Ubuntu): `cd shell && cargo build --locked --release --features wayland` then run the existing `shell/scripts/nested-wayland-smoke.sh` (moved with the lab).

- [x] **0.3 Create `crates/rmac-design` (GPUI-free tokens).** (all §4 tokens as plain data; `Tokens::resolve`; 8 tests; shell dep added in `21d3a08`) — `9f0ecaf`; passes `cargo test --locked -p rmac-design` on macOS and Ubuntu
  Files: new crate in main workspace; add `rmac-design = { path = "../crates/rmac-design" }` to `shell/Cargo.toml` too.
  Do: `pub struct Tokens { colors, materials, type_scale, radii, metrics, elevation, motion }` built by `Tokens::resolve(appearance: rmac_appearance::ResolvedAppearance) -> Tokens`, containing **every** value in §4 as named fields (use the token names in §4 converted to snake_case). Colors are `Rgba(u32)`. No gpui dependency. Unit tests: dark/light/high-contrast each produce WCAG ≥ 4.5 for `label.primary` on `surface.window`, ≥ 3.0 for `label.secondary`; every alpha in reduced-transparency materials ≥ `F0`.
  Verify: `cargo test --locked -p rmac-design`.

- [x] **0.4 Make `rmac-ui` consume `rmac-design`.** (`ThemeTokens::from_appearance` now converts `rmac_design::Tokens`, keeps every public field/type, app-only derivations for subtle accent/error surfaces) — `f79a79f`, `05d47d8`, lock `0df8316`. Ubuntu: `cargo test --locked -p rmac-ui` ✔ (27), `cargo check --locked -p rmac-finder -p rmac-system-settings` ✔.
  Files: `crates/rmac-ui/src/theme.rs`, `mac.rs`.
  Do: `ThemeTokens::from_appearance` becomes a thin conversion from `rmac_design::Tokens`. Keep public API names used by apps so apps compile unchanged. Delete duplicated literals.
  Verify: `cargo test --locked -p rmac-ui`; `cargo check --locked -p rmac-finder -p rmac-system-settings`.

- [~] **0.5 Make the shell consume `rmac-design`.** (`crates/rmac-shell-ui/src/tokens.rs` added: GPUI conversion, global live tokens, `install_appearance_watch` mirroring the app runtime, 18 semantic accessors; `shell_visuals` deleted; all 38 `visuals::*` uses + 3 text literals migrated in the four bins) — `87d5b2b`, `20e5e29`, `fd2c243`.
  Ubuntu: release+debug `--locked` build ✔, clippy `-D warnings` ✔, `cargo test --locked -p rmac-shell-ui` ✔ (11, incl. `surface_colors_follow_the_resolved_appearance`: light `top_bar_tint()=F6F6F6F2`/`primary_text()=1D1D1FFF`, dark `1E1E20F2`/`F5F5F7FF`). Live watcher proven by render counters on the dev menubar unit: writing `~/.config/rmac/theme.json` `color_scheme:light` produced a repaint (+1), removing it another (+2), with no restart.
  **Caveats:** (a) remaining component-specific `rgba(0x…)`/`px(…)` literals were eliminated in 1.9. (b) **Resolved:** live visual evidence captured on the reference PC — the dev menubar renders an opaque `1E1E20` bar in dark and `F2F2F2` in light, and switching `~/.config/rmac/theme.json` recolors the shell **and** the running Files/settings windows with no restart (`target/evidence/phase0/{devshell,light,dark2}.png`). (c) The dev installer writes `~/.local/libexec/rmac`, but the active user units run `/usr/libexec/rmac` (system install); live testing uses reversible `~/.config/systemd/user/<unit>.service.d/dev.conf` drop-ins pointing at `~/rmac/shell/target/debug/`.
  Files: `shell/crates/rmac-shell-ui/src/tokens.rs`; delete `visuals` module from old `lib.rs`.
  Do: replace every `visuals::*` and literal `rgba(0x…)`/`px(<number>)` for colors, radii, row heights in the four bins with token lookups. Appearance comes from `rmac-appearance-portal` live subscription (already used by apps) so light/dark switches live.
  Verify (Ubuntu): build; switch appearance in Settings; menu bar/Dock/OSD recolor without restart.

- [x] **0.6 Fonts.** (`fonts-inter`+`fonts-jetbrains-mono` added to `rmac-apps` and `rmac-session` Depends and documented; Terminal uses `rmac_ui::MONO_FONT` and measures the cell advance via the window text system at construction, on resize, and on zoom, with a documented fallback ratio; lock provider uses `rmac_design::UI_FONT`; `init_application` warns once and falls back if the UI font is absent) — `abbbd93`, lock `f21ceba`. Ubuntu: `cargo test --locked -p rmac-terminal` ✔ (4+66), `cargo check --locked -p rmac-ui -p rmac-lock-provider-linux` ✔, `python3 scripts/test_native_packages.py` ✔ (8). `fc-match Inter` → Inter-Regular ✔. **Blocked:** `fonts-jetbrains-mono` is not installed on the reference PC and sudo needs a password, so `fc-match "JetBrains Mono"` was not run; the dep is declared and the runtime falls back. **FEEL_SPEC.md §D.5 tuning done (`crates/rmac-design/src/typography.rs`, `packaging/rmac-session/fontconfig/99-rmac.conf`):** per-role Inter tracking now interpolates the measured SF curve (10→+0.12 … 96→−1.5, test `tracking_follows_the_measured_sf_curve`); a package fontconfig file sets grayscale antialiasing (`rgba none`), hinting off (`hintnone`), no LCD filter, and aliases the platform sans names to Inter. It is numbered **99** (not the spec's 60) because Ubuntu's `6x`/`99-language-selector-*` generic-family rules are processed later and otherwise override a 60-prefixed file — a deliberate deviation recorded in FEEL_SPEC.md §D.5. **Verified on the reference PC:** with the file in the user fontconfig dir, `fc-match "Helvetica Neue"`, `"SF Pro Text"`, and `"SF Pro Display"` all resolve to `Inter-Regular.otf`, Inter reports `hinting: False`, `hintstyle: 0`, and `rgba: none`; removing the file reverts them to Noto Sans. **Gap:** `fc-match system-ui` / `-apple-system` still resolve to Noto Sans — fontconfig's built-in generic-family substitution wins even when the file is loaded last; needs a different mechanism (or system-level install) and is recorded, not faked.
  Files: `scripts/linux/native_package_contract.py` (dependency list), `docs/native-packaging.md`, `crates/terminal/src/controller.rs`, `crates/rmac-lock-provider-linux/src/text_renderer.rs`.
  Do: add `fonts-inter` and `fonts-jetbrains-mono` to the session/app package `Depends`. At app start (`rmac_ui::window::boot*`) check the font resolves; if not, log one warning and fall back to the system sans (never crash). Terminal: replace `"Menlo"` with `rmac_ui::MONO_FONT`; replace `CELL_W = FONT_SIZE * 0.6` with advance width measured via `window.text_system().advance(font_id, size, 'M')` at startup and on font/scale change.
  Verify: `cargo test --locked -p rmac-terminal`; Ubuntu: `fc-match Inter` and `fc-match "JetBrains Mono"` resolve after package install; Terminal columns fill the window exactly (no right gap > 1 cell).

- [x] **0.7 Names.** (the desktop `Name=Apps` and metainfo `<name>Apps</name>` were already correct; renamed every remaining product-name occurrence — service description, app-drawer log/status strings, crate comments, and 20+ docs — while keeping identifiers `rmac-app-drawer`/`app-drawer`/`APP_DRAWER`/`AppDrawer`) — `aab4bf0`. Ubuntu: `cargo test --locked -p rmac-app-drawer -p rmac-top-bar` ✔ (4+3+9), `python3 scripts/verify-documentation.py` ✔; `grep -rn "App Drawer"` outside the guide now finds no prose.
  Files: `packaging/rmac-apps/applications/org.rmac.AppDrawer.desktop` (`Name=Apps`), metainfo, `po/`, `crates/app-drawer` visible strings, `crates/rmac-top-bar/src/labels.rs`, Settings strings, docs (`docs/user-guide.md`, `docs/places.md`, `docs/dock.md`, `docs/shortcuts.md`, `README.md`).
  Do: apply FD-6. Keep desktop-file IDs and crate names. Regenerate `rmac-apps.pot`.
  Verify: `rg -n "App Drawer" --glob '!crates/**/tests*' --glob '!*.pot'` shows only internal identifiers; `python3 scripts/verify-documentation.py`.

- [x] **0.8 Fix contradictory docs.** (`docs/macos-ui-reference.md`: 32→26 menu bar, 48→56 Dock icons, ~20→26 shelf radius; `docs/dock.md`: 48→56 icon. `docs/places.md`/`docs/user-guide.md` already stated the corrected non-forced Files/Downloads Dock contract, so no edit was needed.) — docs change in the 0.8 commit. Verify: `python3 scripts/verify-documentation.py` ✔. `python3 scripts/run-release-contract-checks.py` on Ubuntu runs but 3 `test_apt_publisher` tests fail on the pre-existing storage-floor guard because `/tmp` is a 3.4 GiB tmpfs; unrelated to this change.
  Files: `docs/macos-ui-reference.md` (32→26 menu bar, 48→56 Dock icons), `docs/dock.md`, `docs/places.md`, `docs/user-guide.md` (remove forced Files/Downloads Dock tail per parity spec §13 mismatch 1).
  Verify: `python3 scripts/run-release-contract-checks.py`.

- [~] **0.9 Move the reference target to macOS 27 Golden Gate and adopt the owner's real profile.**
  Files: `git mv docs/macos-tahoe-parity-spec.md docs/macos-parity-spec.md` (fix every link), `crates/rmac-design`, `crates/rmac-shell-settings/src/model.rs`, `packaging/rmac-session/{shell.kdl,config.kdl}`, Files defaults.
  Read first: `FEEL_SPEC.md` §B (macOS 27 delta), §C (owner profile), §C.2 (pixel measurements).
  Do: (a) apply the §B deltas — Liquid Glass intensity slider, one standard window radius, edge-to-edge sidebars, standardized toolbars, menus without icons, four-appearance icons, traffic-light press bounce, wallpaper animating on unlock, and keep Search as plain Search with no assistant; (b) apply the §C defaults — Dark appearance, Dock pins without a browser, tile 64, magnification off, natural scrolling **off** (delete `natural-scroll` from `shell.kdl`), Files in List view with the status bar on, `en_GB` + India region, bottom-right hot corner = New Note; (c) replace the `S` geometry values with §C.2's measured ones; (d) record macOS 27.0 build 26A428 and the 2026-09-18 date at the top of the parity doc.
  Verify: fresh user account → first login is dark, the Dock is 64 px tiles with 76 px pitch, Files opens in List view with a status bar, scrolling is not natural, dates read dd/mm/yyyy with a 12-hour clock, and the menu bar measures 29 px in a screenshot.
  Status: **partial (`c3ffa34`, `a46020a`)**. Done: renamed the parity doc and recorded macOS 27.0 build 26A428 / 2026-09-18 plus a §0 reference-target + owner-profile section and the §B delta; `crates/rmac-design` now carries the measured geometry (menubar 29, menu row 24, dock tile 64 / gap 12 / padding 4 / bottom margin 18 / indicator offset 6, tile max 128, one window radius 16) and the macOS-27 `glass_intensity` token (0.4–1.6 × tint alpha, default mid, high contrast still wins; test `glass_intensity_scales_material_tint_alpha`); `shell.kdl` dropped `natural-scroll` and the 12/16 radius split (`niri validate` ✔) and the owner's Dock pins (Files, Apps, Notes, Text Editor, Terminal, System Settings — no browser) are the `ShellSettings::default()`; `rmac-theme`'s `SchemePreference` now defaults to **Dark**; the Dock and menu-bar bins render the measured values (tile 64, gap 12, shelf 72, 18 margin; bar 29) and the reference PC screenshot shows the new order and a shelf measuring ≈677 physical px = the predicted 541 logical (`target/evidence/phase0/0.9/geom1.png`, `first1.png`); the desktop is dark and the Dock shows the owner's six pins with Terminal running. (On the *existing* dev account Files restored its previously saved Icon view, which is correct last-used behavior; a fresh account takes the List default. The same capture shows a GNOME status area on the right of the screen — the reference PC's environment, not rmac's bar.) `cargo test -p rmac-design -p rmac-theme -p rmac-shell-settings -p rmac-dock` ✔ (9/…/13/55), shell builds `--locked --features wayland` ✔. **Remaining:** hot corners (no shell infrastructure yet — lands with 4.15), the en-GB/India locale (system-level; rmac follows the host locale), installing the updated `shell.kdl` on the reference PC (package install), the §B UI deliverables assigned by `FEEL_SPEC.md` §G to later phases (edge-to-edge sidebars, one toolbar component, four-appearance icons, traffic-light bounce, wallpaper unlock animation), and the full fresh-account first-login screenshot (the reference session re-locked before it could be taken; Files already defaults to List view + status bar, so that row is satisfied in code).

**Phase 0 exit checklist**
- [~] `shell/` builds on Ubuntu ✔ (debug + release, `--locked`); systemd units start the promoted binaries (installer `--check` passes; binary names preserved) — nested smoke ✗ blocked by pre-existing niri requirement in `rmac-dock-runtime` (see 0.2).
- [x] `rg -n "0x[0-9a-fA-F]{8}" shell/bins crates/*/src --glob '*render*'` finds no color literals outside `rmac-design` (allow SVG/asset code and tests). (No `*render*` file contains an 8-digit hex literal. Caveat: 55 component-specific literals remain in `shell/bins/*/main.rs` — bespoke wallpaper-picker preview art, the Dock's fallback-tile letter, and OSD/menu chrome whites. They are recorded for the Phase 1/3/4 restyles that will define their tokens.)
- [x] Inter and JetBrains Mono render on Ubuntu: `fc-match Inter` → Inter-Regular and `fc-match "JetBrains Mono"` → JetBrainsMono-Regular after installing `fonts-jetbrains-mono`; the Component Gallery screenshot (`target/evidence/phase1/gallery-*.png`) shows Inter rendering in dark/light/high-contrast. Token crate requests both via `rmac_design::UI_FONT`/`MONO_FONT`.
- [x] No visible "App Drawer" string (0.7).

---

### Phase 1 — Design system and components

**Goal:** every shared control in §5 exists in both UI crates, matches reference captures, and the component gallery proves it.
**Read first:** `docs/component-gallery.md`, `crates/rmac-ui/src/controls.rs`, `crates/rmac-ui/src/components.rs`, `crates/component-gallery/src/specimens.rs`.

- [~] **1.1 Capture the reference set (Mac).** (**Partly done 2026-09-18**: menu + submenu, Control Center, Notification Center, Spotlight empty/typing, Finder all four views, save sheet, open panel, unsaved alert, four Settings panes captured to `target/evidence/reference-mac/` and measured in `docs/reference-captures-2026-09-18.md`; those values are now `M` in §4. **Remaining:** lock screen, Mission Control, Apps grid, Dock context menu, magnification, and light-appearance versions.) (unblocked tooling done: `scripts/compose-evidence.py` + `scripts/test_compose_evidence.py`, 3 tests pass. The Mac captures and S-value measurement remain **owner-blocked** — they need the reference Mac and pixel ruler.)
  Do: on the reference Mac at scale 2 ("Default" resolution) in Light and Dark, capture: menu bar over a light and a dark wallpaper; an open menu with submenu, disabled item, checkmark, shortcut; a context menu in Finder; Control Center fully open; Notification Center with 3 grouped notifications; a banner; Spotlight empty, typing "term", typing "2+2"; Launchpad/Apps; Dock idle, hover tooltip, right-click menu, magnification on; Finder window in icon/list/column/gallery views; System Settings Wi-Fi, Appearance, Desktop & Dock panes; an alert; a sheet (Save); a window's traffic lights hovered and inactive; volume OSD; lock screen. Store under `target/evidence/reference-mac/<surface>-<light|dark>.png` (ignored). Record macOS version and settings in `target/evidence/reference-mac/README.txt`.
  Measure with any pixel ruler (divide by 2 for logical px). Overwrite every **S** value in §4 of this file **and** in `crates/rmac-design` in one commit named `Measure Tahoe tokens from reference captures`.
  Verify: diff shows only §4/`rmac-design` changes; tests pass.

- [~] **1.2 Buttons, pop-ups, segmented (5.1, 5.2, 5.7)** in `rmac-ui/controls.rs` and `rmac-shell-ui`. Gallery specimens for each state. (Against **S** values in `rmac-ui`: `Button` keeps roles/sizes/disabled/busy/selected/tooltip/dropdown and now drops its fill in an inactive window (§5.1); new `PopUpButton` (5.2, secondary style + trailing chevron + menu); new `SegmentedControl` (5.7) with token styling and a pure `wrapped_selection` keyboard model. Gallery gained `Segmented` and `Popup` components with states. `cargo test -p rmac-ui` ✔ 29, `cargo check -p rmac-component-gallery` ✔. **Deferred:** the `rmac-shell-ui` mirror (no shell overlay consumes these controls yet; adding a parallel library now would be speculative).) — `9032bf9`, `a566094`, `f7a41b9`, `2293e2c`, `43a45f4`
- [~] **1.3 Switch, checkbox, radio, slider (5.3–5.5)** including pending state and CC thick slider variant. (Against **S** values: `Toggle` gained `SwitchSize` (Regular/Small/Mini from §4.7 metrics), a `pending` busy thumb that keeps the old value and disables the control, a `fill.control` off track with border, and a white thumb; added `Checkbox` (tri-state, 14 px, check/bar marker) and `Radio` (14 px, accent fill + white dot); `rmac-design` gained absolute `white`/`black` tokens surfaced through `mac`. Gallery gained `Checkbox`/`Radio`. `cargo test -p rmac-ui` ✔ 31, `cargo check -p rmac-component-gallery` ✔. **Deferred:** the `Slider` still wraps the component implementation, so the §5.5 knob/track tokens and the Control Center thick (28 px) variant are not done; they are needed by Phase 5.) — `721855b`, `9f0e9bd`, `2aba914`
- [~] **1.4 Text and search fields (5.6)** including IME preedit and error state. (`TextField`/`SearchField` gained an `error(message)` state that draws a danger border and a `subheadline` danger message below; IME, preedit, placeholder, and the clear action continue to be provided by the pinned `InputState`/component implementation. `cargo test -p rmac-ui` ✔ 31. **Resolved in part:** `init_application`/`apply_resolved_tokens` now push the resolved `rmac-design` tokens into gpui-component's global theme (`rmac-ui/src/runtime.rs::apply_component_theme`) — fonts, radius/radius_lg, and the primary/secondary/input/list/popover/sidebar/switch/slider/tab/table/danger/selection roles all derive from the same source, so component primitives inherit the rmac palette live. Remaining for exact §5.6: per-control heights (24/28) and the preedit underline/candidate specifics.) — `b591aed`, `94a3ad5`, `5e45f9b`
- [~] **1.5 Menus (5.9)** — one implementation per UI crate, used by: menu bar menus, Dock context menus, app context menus (`rmac_ui::ContextMenu` → rewrite), pop-up buttons. Includes submenu safe-triangle, type-select, off-screen flip, select blink. (`rmac_ui::ContextMenu` now has a checkmark column (`MenuCheck` On/Mixed/None), non-interactive section `header`, dimmed `disabled_item`s skipped by keyboard nav, and a tested pure `type_select_match` model; existing click-away, Escape, up/down/tab nav, danger items, shortcuts, and `snap_to_window_with_margin` off-screen clamping retained. `cargo test -p rmac-ui` ✔ 33, `cargo check -p rmac-finder` ✔. **Deferred:** submenu rendering with safe triangle, select blink, per-item icons, and the shared shell/Dock/menu-bar menu implementation.) — `c92f096`
- [~] **1.6 Sidebar, table/list, toolbar, traffic lights (5.8, 5.13, 5.14, 5.16).** (`rmac-design` traffic-light colors are now surfaced through `ColorTokens`/`mac`, and `chrome::traffic_lights_active(active)` renders the §4.4 colors with the inactive gray pair and no glyph reveal; `chrome::toolbar_group` adds the §5.14 grouped glass capsule. Sidebars and tables/lists are served by the existing token-driven `List`/`ListRow`/`Table` controls plus `mac::material_sidebar`/`sidebar_selection`. `cargo test -p rmac-ui` ✔ 34, `cargo check -p rmac-finder -p rmac-text-editor` ✔. **Deferred:** adopting `toolbar_group` in the apps, the §5.14 overflow menu/Customize Toolbar, sidebar section headers + collapse, and full §5.13 column resize/reorder/visibility.) — `38c3c24`, `95b9af8`
- [~] **1.7 Popover, sheet, alert, tooltip, progress, spinner, empty state (5.10–5.12, 5.15).** Remove `Toast` from normal flows (grep usages; replace with inline status or alert). (Added the §5.15 `Spinner` (12 spokes, pure `spoke_opacity` model + test, idle-cost zero when hidden). `alert`/`dialog`/`dialog_button` (§5.11), `Tooltip` (§5.12), `Progress`, `EmptyState` already existed and are token-driven. **`Toast` grep:** the only remaining non-gallery use is System Settings' recoverable global data error banner (`shell_render.rs`), which §5.15 permits; it should be converted to an inline status banner in a follow-up. **Deferred:** `Popover` (§5.10) and `Sheet` (§5.11) have no consumers yet — they arrive with Phases 5/7.) — `c6f2a7b`
- [~] **1.8 Icon set.** Create every symbol in §4.10 as original SVG; rework the 7 app icons to the squircle grid with light/dark/tinted variants; add the third-party plate rule to `rmac-icon`. (Added the `rmac-icon` third-party plate rule (`IconShape`, `PLATE_ICON_SCALE = 0.8`, `third_party_plate`, tested — `cargo test -p rmac-icon` ✔ 12) and 57 original 16×16/1.5 px line symbols under `shell/assets/symbols/`, covering the §4.10 UI set (search, chevrons, checkmark, xmark, plus, minus, ellipsis, gear, list, grid, columns, gallery, sidebar-left, folder, trash-empty/full, lock, bell, info, share, tag, power, restart, sleep, logout, play, pause, next, prev, display, now-playing, mic, camera, screen-record, brightness-low/high, keyboard-brightness, airplane, focus-moon/custom, ethernet, wifi-0..3/off/error, speaker-0..3/muted, bluetooth-connected, battery-charging); all parse cleanly. **Remaining:** `battery-0..100` (spec says draw programmatically), and the 7 app-icon squircle rework with light/dark/tinted variants.) — `c20b09a`, `a1714e4`
  **Wallpaper set begun (FEEL_SPEC.md §D.6):** `rmac-wallpaper` now ships five original procedural built-ins — `Aurora` (default), `Tide`, `Basalt`, `Monsoon`, `Paper` — each with a distinct four-stop sRGB palette drawn by the renderer (`builtin:rmac-tide` …); `ALL`/`parse`/`id`/`metadata` round-trip is tested and dependents compile. The System Settings ▸ Wallpaper picker now lists all five with per-row Use buttons (Aurora keeps the default `Source(None)`, the others set `builtin:<id>`); `cargo test -p rmac-system-settings` ✔ 58, token/control gates clean. **Light/dark pair modelled:** every built-in now carries a distinct `light_palette` alongside its dark `palette`, with `BuiltInMetadata::palette_for(dark)`; the procedural generator draws through the method and a test asserts each wallpaper has a distinct light pair (rasterizing to the exact output resolution is already handled by the raster cache). **Remaining:** thread the resolved appearance into the raster plan so light mode actually selects `light_palette`, the 2 s cross-fade when the appearance changes, and the 1.2 s unlock drift.
- [x] **1.9 Token gate script.** New `scripts/check-design-tokens.sh`: fails if product UI source (`crates/{finder,system-settings,notes,terminal,text-editor,activity-monitor,app-drawer,launcher-app,quick-settings-app,notification-center-app}/src`, `shell/bins`, `shell/crates`) contains `rgb(0x`, `rgba(0x`, `hsla(`, or `.rounded(px(<literal>))`, except files named `*tests*`. Add to CI next to `check-shared-controls.sh`. (Script added to CI after the shared-control step; **gate is clean**. Swept every literal in scope: `shell/bins` 44 colors + 19 radii → `rmac-shell-ui::tokens`; app crates → `rmac-ui::mac` tokens (app-drawer 5, launcher-app 5, text-editor 1, activity-monitor 20, terminal 19, finder 27, notes 26, system-settings 25). Added radii accessors (`radius_menu/menu_item/dock/hud/segmented/none`), system-color accessors, and a measured `folder_blue` token to `rmac-design`/`rmac-ui`. One deliberate refinement: the pattern uses `hsla(0x` rather than bare `hsla(` so the terminal's user-profile color conversion and the shell token module are not false positives; the shell token module is still allowlisted. Verified: all eight app crates `cargo check` ✔, `rmac-ui`/`rmac-design` tests ✔, shell build/clippy/lib-tests ✔.) — `5f4791d`, `f335f48`, `9069001`, `ac79c38`, `3b5d92b`, `5079b3e`, `2faf6c3`, `86dcfa9`, `6c5bca4`

- [~] **1.10 Cursor theme (FEEL_SPEC.md §D.2).** Original `rmac` Xcursor theme generated by `scripts/build-cursors.py` (no SVG rasterizer or `xcursorgen` is available, so Pillow draws the shapes and the script writes the Xcursor binary format directly). **Done (`4b03c7c`):** 18 drawn shapes (black-filled arrow with a thin white edge, I-beam, vertical-text, crosshair, 4-way move, all-scroll, the four diagonal/axis resize pairs, row/col resize, not-allowed, wait ring, progress, copy, context-menu, help) at 24/32/48/64 px, plus the freedesktop aliases (`left_ptr`, `xterm`, `fleur`, `watch`, …); `index.theme` inherits Adwaita so undrawn artistic shapes (hand, zoom, alias) never leave the pointer missing; running `scripts/build-cursors.py` regenerates `assets/cursors/rmac` reproducibly. Wired into the session: `shell.kdl` `cursor { xcursor-theme "rmac"; xcursor-size 24; }` (`niri validate` ✔) and `XCURSOR_THEME`/`XCURSOR_SIZE` exported + imported into the user environment by `start-rmac-session.sh`. **Verified:** theme copies to `~/.icons/rmac` (36 entries), `default` is a valid 4-size Xcursor (`magic 0x72756358`); `niri validate` accepts the cursor block. **Packaging done:** `stage-session-package.py` stages every `assets/cursors/rmac` file into `usr/share/icons/rmac` and `verify-session-package.py` derives the same inventory from the assets dir, so `python3 scripts/test_session_package.py` ✔ 9 with the theme in the payload. **Remaining:** the pointer-shape hover test in the gallery, and live pointer verification (screenshots do not capture the cursor — needs a recording or a human).

- [~] **1.11 Scrolling physics and input (FEEL_SPEC.md §D.4).** `crates/rmac-ui/src/scroll.rs` adds the shared model: `Momentum` (τ = 325 ms, stops below 0.5 px/frame, `flick`/`step`/`cancel`), `rubber_band(overscroll, viewport)` (bounded to 0.25 × viewport, signed, monotonic), and `spring_step` (stiffness 200, damping 26) for the release snap-back — all tested (`cargo test -p rmac-ui scroll` ✔ 3). `shell.kdl` now sets `accel-profile "adaptive"`, `accel-speed 0.3`, and keyboard `repeat-delay 225` / `repeat-rate 11` (macOS defaults) and passes `niri validate` ✔. **Remaining:** wire `Momentum`/`rubber_band` into every scrollable (Files, Settings, Notes, Control Center, Apps), the overlay scrollbar (7 px, fade after 700 ms), and live verification of the tracking/key-repeat feel.

For each of 1.2–1.8, **Verify:** `cargo test --locked -p rmac-ui`; `cargo run -p rmac-component-gallery` screenshot light + dark + high contrast placed next to the reference capture; states match within ±1 px geometry and visually indistinguishable color at 100% zoom.

**Phase 1 exit checklist**
- [~] Gallery shows every component × every state × light/dark/HC/reduced-transparency. (Captured live on the reference PC: `target/evidence/phase1/gallery{,-light,-hc}.png` show Button (Default/Hover/Pressed/Focused/Disabled/Busy/Destructive), Toggle (Off/On/Mixed/Focused/Disabled), Slider (Default/Focused/Disabled/Unavailable), Text field (Empty/Filled/Focused/Invalid/Disabled) with the error border, and more, in dark, light, and high-contrast with live theme switching. Reduced-transparency capture still to add.)
- [x] Token gate passes in CI (1.9; local `bash scripts/check-design-tokens.sh` clean, wired after the shared-control step).
- [ ] Keyboard: Tab reaches every gallery control; Space/Return/Escape/arrow semantics per §5.

---

### Phase 2 — Session and windows

**Goal:** choose rmac in GDM → one login → full desktop with no terminal; windows behave like macOS.
**Read first:** `docs/session-supervisor.md`, `docs/ubuntu-session-packaging.md`, `docs/session-recovery.md`, `docs/niri-adapter.md`, `docs/compositor-domain.md`, `packaging/rmac-session/shell.kdl`, `crates/rmac-compositor/src/actions.rs`.

**Audited gap:** `rmac_compositor::ActionKind` has only Spawn, FocusWindow, FocusWorkspace, FocusOutput, CloseWindow, MoveWindowToWorkspace, MoveWindowToOutput, SetOverview. There is **no** minimize, hide, full screen, maximize/fill, move, resize, or tile action, and niri has no native minimize.

- [ ] **2.1 Session readiness order.** Supervisor starts, in order, waiting for each unit's `READY=1` with a 5 s bound: `wallpaper` → `menubar` → `dock` → `notifications authority` → `shortcut broker` → on-demand overlay services (Phase 5) → `login items`. Show wallpaper first so the user never sees a black screen or niri's default background. If any shell unit fails 3× in 60 s, enter safe mode (existing) and show a banner-style Notification "rmac started in safe mode" with "Show Details".
  Verify (Ubuntu): reboot → GDM → rmac → desktop complete in ≤ 3 s after auth (record with `systemd-analyze --user blame` and a screen recording). `systemctl --user kill rmac-dock.service` → Dock returns ≤ 2 s, apps untouched. GNOME session still logs in.

- [~] **2.2 Extend the compositor action model.** (Against real niri 26.04 IPC, which exposes only *focused-window* actions, `move-column-to-first/last`, and **no native minimize/hide**: added `FullscreenWindow{window,on}`, `FillWindow{window}`, `CenterWindow{window}`, `TileWindow{window,region}`, `MinimizeWindow{window}`, `RestoreWindow{window,workspace}` to `rmac-compositor::Action`/`ActionKind` (plus `TileRegion`, `PARKING_WORKSPACE`), matching `wire::Action` variants and `WorkspaceReference::Name`, capabilities, and a focus-then-act `convert_action_sequence`. Min/Max → `MoveWindowToWorkspace` to the named `rmac-parking` workspace; restore moves back and focuses. Regions niri can't express (Top/Bottom/quarters) return an explicit `Unsupported` error before any request. `cargo test -p rmac-compositor -p rmac-compositor-niri` ✔ 9+10 with wire/sequence/minimize/unsupported tests. Added tested expansion helpers for the stateful pieces: `window_is_parked`, `application_windows`, `hide_application` (→ per-visible-window `MinimizeWindow`), `show_desktop`, and `restore_all(&[(window, workspace)])`. `cargo test -p rmac-compositor` ✔ 16, plus `window_id_for_process(snapshot, pid, app_id)` to resolve a client's window for 2.4. **Persistence done:** `rmac-compositor::ParkingStore` records `{window → origin workspace}` and round-trips through `$XDG_RUNTIME_DIR/rmac/parking.json` (`load_default`/`save_default`, atomic write, prune against the live snapshot); `visible_windows_except`/`windows_of_application` were added for 3.3's Hide Others and Quit. **Verified live 2026-09-17:** Hide moved the focused app's window onto the `rmac-parking` workspace and recorded its origin, and Show All moved it back to the recorded workspace and emptied the store. A real config gap surfaced first: niri only resolves a `move-window-to-workspace` reference *by name* when that workspace is declared, so `workspace "rmac-parking"` was added to `packaging/rmac-session/shell.kdl` (`2542573`); before that the move silently no-opped. **Remaining:** `MoveWindow`/`ResizeWindow` (niri has no absolute placement), the floating-rect part of the restore set, and excluding `rmac-parking` from the Spaces bar/Mission Control/app switcher/Dock counts.)
  Files: `crates/rmac-compositor/src/actions.rs`, `crates/rmac-compositor-niri/src/{translate,wire}.rs`, tests.
  Add actions: `FullscreenWindow{window, on}`, `FillWindow{window}` (floating → set width/height to output working area minus 0 gaps, position 0,0 relative to working area), `CenterWindow`, `TileWindow{window, region: Left|Right|Top|Bottom|TopLeft|TopRight|BottomLeft|BottomRight}`, `MoveWindow{window, x, y}`, `ResizeWindow{window, w, h}`, `MinimizeWindow{window}`, `RestoreWindow{window}`, `HideApplication{app}`, `UnhideApplication{app}`, `ShowDesktop{on}`.
  Minimize design (because niri has none): first check `niri msg action --help` on the reference PC; **if** a native minimize exists, use it. **Otherwise** keep a per-output named workspace `rmac-parking` declared in `shell.kdl` (`workspace "rmac-parking"`), move the window there, record `{window → (origin workspace, floating rect)}` in `rmac-compositor` state (persisted to `$XDG_RUNTIME_DIR/rmac/parking.json`), and exclude `rmac-parking` from the Spaces bar, Mission Control, app switcher, and Dock "running" window counts. Restore moves it back, re-applies the rect, focuses it. Hide = minimize all app windows without Dock minimized tiles. Show desktop = park all windows on the output with a single restore set.
  Verify: unit tests for every translation; Ubuntu: minimize Files → window disappears with Scale animation toward its Dock tile, Dock shows a minimized-window tile to the right of the separator (if "Minimize windows into application icon" off), clicking it restores exactly.

- [~] **2.3 Window chrome rules in `shell.kdl`.** (`packaging/rmac-session/shell.kdl`: default window rule now carries the active/inactive window shadow (`#00000050` / inactive `#00000030`, softness 28, spread 2, offset 0,8), radius 12, clip, focus-ring off, border off; new rule gives `org.rmac.Files|SystemSettings|Notes` the 16 px unified-toolbar curve; **removed `open-maximized true` for Files** so it opens at its last saved size; Files keeps the sidebar material blur. `niri validate` on the reference PC ✔ including `inactive-color`. **Remaining:** third-party first-launch 70%-of-working-area sizing and per-app-id remember (niri has no such rule — needs the shell), and the visual/focus verification is blocked by the non-presenting reference display.)

- [~] **2.4 Traffic-light actions wired.** (Added `rmac-compositor-niri::execute_action` + a one-shot `snapshot`, and `rmac-ui::chrome::send_window_action` which resolves this process's focused window and sends a niri action. **Verified live 2026-09-17:** clicking the zoom traffic light on a dev Files window took it fullscreen (`tile_size` 1100×720 → 1536×864 == output) and the window restored afterward; ⌥-zoom is wired to `FillWindow` (not yet runtime-verified since the probe can't send modifiers). **Minimize is now wired too** (`WindowAction::Minimize`): it records the window's origin in `rmac-compositor::ParkingStore` and parks it on `rmac-parking`; with 2.2's store now real, restoring works through the app menu's Show All until the Dock's minimized-window tile (4.11) lands. **Verified live 2026-09-17:** clicking the yellow light on a dev Files window parked it (`workspace_id` → `rmac-parking`, `parking.json` recorded `{24 → 4}`) and Show All brought it back to workspace 4 and emptied the store. Still to do: the Scale animation toward the Dock tile and the Dock tile itself (4.11), the 1 s hover tiling menu (§5.16), and the double-click-titlebar setting.) In `rmac-ui::traffic_lights()`: close → `window.remove_window()`; minimize → `MinimizeWindow` via the compositor client; zoom → `FullscreenWindow` toggle; ⌥-click → `FillWindow`; 1 s hover on zoom → tiling menu (§5.16) sending `TileWindow`/`CenterWindow`/`MoveWindowToOutput`. Double-click toolbar background → Settings "Double-click a window's title bar to" (Zoom = Fill toggle, Minimize, Do Nothing).
  Verify: every action on every rmac app on Ubuntu, both with pointer and with menu Window ▸ commands.

- [ ] **2.5 Window menu commands (all first-party apps).** Window menu: Minimize ⌘M, Zoom, Fill ⌃⌥F (Tahoe fn⌃F), Center ⌃⌥C, Move & Resize ▸ (Left ⌃⌥←, Right ⌃⌥→, Top ⌃⌥↑, Bottom ⌃⌥↓, quarters), Full Screen Tile ▸, ─, Remove Window from Set (disabled), ─, Bring All to Front, ─, list of the app's windows with ✓ on the key window.
- [ ] **2.6 Edge tiling by drag.** Dragging a floating window's toolbar so the pointer touches the left/right screen edge shows a translucent preview (`material.popover` rect inset 6 px) after 300 ms; release tiles. Top edge → Fill. Holding ⌥ while dragging shows the preview immediately. Setting in Desktop & Dock: "Drag windows to screen edges to tile" (on), "Drag windows to menu bar to fill screen" (on), "Hold ⌥ key while dragging windows to tile" (on), "Tiled windows have margins" (off). Implement in the menubar/dock-independent `rmac-shell-windowing` helper that watches niri window-move events; if niri IPC cannot observe interactive moves, implement only the keyboard/menu tiling and mark drag tiling *(may defer)* with a note in `docs/known-limitations.md`.
- [ ] **2.7 Spaces and Mission Control.** (Measured: the Spaces strip is a single **"Desktop" pill** at top centre with a **+** at the far right, not a filmstrip of numbered Spaces; windows spread without overlapping, each labelled with its app name, and the Dock stays visible.) Map: ⌃↑ = niri overview (exists) presented as Mission Control; ⌃↓ = App Exposé (overview filtered to the focused app — if niri cannot filter, open the app switcher window list instead, see 5.7); ⌃←/⌃→ switch Space (exists); ⌃1…⌃9 go to Space N; F3/Mission Control key = overview. Spaces bar: niri overview shows workspaces; ensure `rmac-parking` never appears (name-filter via niri `workspace` `open-on-output` + hiding rule; if niri shows it anyway, park windows off-screen in a floating position instead and document).
- [ ] **2.8 Click-away semantics.** Clicking the wallpaper never closes or hides app windows. Setting "Click wallpaper to reveal desktop": *Only in Stage Manager* (default, i.e. never, since no Stage Manager) / *Always* (runs ShowDesktop).
- [ ] **2.9 Close vs quit.** Closing the last window of an app does **not** quit it, except where the macOS counterpart quits. Defaults: Files, Terminal, Text Editor, Notes, System Monitor stay running with a Dock running dot; System Settings quits on last-window close. Confirm each against the matching Mac app during 1.1 and adjust. Files can never be quit (no Quit item, like Finder). ⌘Q quits the app after unsaved-change prompts. The Dock and menu bar always reflect the real process state.

- [~] **2.10 Window and Mission Control motion (FEEL_SPEC.md §D.3).** `shell.kdl` now carries a niri `animations` block matching the §4.9 tokens: `window-open` 320 ms `ease-out-expo`, `window-close` 200 ms cubic-bezier(0.55,0.06,0.68,0.19), `workspace-switch` 320 ms cubic-bezier(0.65,0,0.35,1), `overview-open-close` 350 ms (same), `config-notification-open-close` and `screenshot-ui-open` 250 ms `ease-out-quad`. niri only accepts `ease-out-quad`/`ease-out-cubic`/`ease-out-expo`/`linear`/`cubic-bezier`, so the ease-in/out curves are explicit beziers (`dfb6c75`); `niri validate` ✔. **Remaining:** the shell-owned choreography — GDM→desktop fade + bar/Dock slide-in (2.11), the app-launch scale-from-tile overlay, sheet slide-from-titlebar, Spotlight/CC/NC anchored scale-in, banner slide, wallpaper unlock drift, and Reduce Motion fallbacks — plus 60 fps frame stepping to verify durations/origins.

**Phase 2 exit checklist**
- [ ] GDM → rmac → complete desktop without terminal; ≤ 3 s after auth on the reference PC.
- [ ] Kill each shell unit: only it restarts; 3 crashes → safe mode; GNOME logs in.
- [ ] Minimize/restore, hide/unhide, full screen, fill, center, tile halves/quarters, move to display, Show Desktop all work for rmac apps **and** Firefox.
- [ ] Mission Control, Space switching, ⌃number work; parking workspace invisible.
- [ ] Screenshot pair: active + inactive window chrome vs reference.

---

## 7. Phases 3–4: menu bar and Dock

### Phase 3 — Menu bar, system menu, status items

**Goal:** a transparent, adaptive, truthful menu bar that behaves exactly like Tahoe's.
**Read first:** `docs/top-bar.md`, `docs/shell-status.md`, `docs/shell-status-linux.md`, `crates/rmac-top-bar/src/*`, `crates/rmac-top-bar-runtime`, `crates/rmac-app-menu/src/lib.rs`, `shell/bins/rmac-menubar`.

#### 3.0 Target picture (1440-wide output, light wallpaper)

```
┌──────────────────────────────────────────────────────────────────────────────────────────────┐ 26 px
│ ◉  Files  File  Edit  View  Go  Window  Help            ● ⌁  ᯤ  ▭ 87%  ⌕  ⊜   Wed 17 Sep  3:45 PM │
└──────────────────────────────────────────────────────────────────────────────────────────────┘
 ↑   ↑bold app name ↑app menus (real only)          privacy↑ ↑status items   ↑Search ↑CC   ↑clock → Notification Center
 rmac mark = system menu
```

- Background fully clear (`material.menubar`). Foreground color adapts: sample mean luminance of the wallpaper strip under the bar (the wallpaper service already rasterizes; publish a per-output `menubar_luminance` value on change, not per frame). Luminance > 0.6 → dark glyphs `000000D9`; else light glyphs `FFFFFFF2` with `0 0 2 alpha .35` text shadow.
- Setting "Show menu bar background" (Menu Bar pane): off → clear; on → `F6F6F6D9`/`1E1E20D9` with blur.
- When a window is full screen: bar hides; pointer at top 2 px (R) reveals it over the window with background on; hides 500 ms (R) after pointer leaves. Setting "Automatically hide and show the menu bar": Never (default) / In Full Screen Only / On Desktop Only / Always.
- One bar per output. The focused app's menus show on the output with keyboard focus; other outputs show the same menus dimmed to 50% (Tahoe "displays have separate Spaces" on).

#### Tasks

- [ ] **3.1 Leading area.**
  rmac mark 16 px, hit 26×22, 12 px leading inset. App name: `headline` **bold**, from `.desktop` `Name`. Menu titles `body` regular, 8 px horizontal padding, 22 px hover pill (`fill.hover`-like: `000000 14` / `FFFFFF 26`), open menu keeps the pill in pressed state.
  Mouse: press opens the menu; while one menu is open, hovering other titles switches menus without clicking; release on an item selects it (press-drag-release). Keyboard: ⌃F2 focuses the menu bar (Full Keyboard Access), ←/→ move between menus, ↓ opens, Return selects, Escape closes.
  Truncation: when menus + status items collide, hide menu titles from the right into a `»` overflow menu; app name is never hidden.

- [~] **3.2 System menu (rmac mark).** Exact rows and order: (Verified live 2026-09-17 with the virtual-pointer probe: clicking the mark opens the dark `material.menu` panel in the spec order — About This rmac · System Settings… · Software Center · ─ · Recent Items › · ─ · Force Quit… ⌥⌘⎋ · ─ · Sleep · Restart… · Shut Down… · ─ · Lock Screen ⌃⌘Q · Log Out Jacob… ⇧⌘Q; first row hover-highlighted. Clicking “System Settings…” launched `org.rmac.SystemSettings` (“Wi-Fi — Settings”). Still to do: the update badge, hiding Software Center when absent, and an About window verifying real device/OS facts.)
  ```
  About This Computer
  ─
  System Settings…                         (badge "1" when updates pending)
  Software Center…                         (hidden if no software center installed)
  ─
  Recent Items                        ›    (Applications / Documents / Servers sections, 10 each, Clear Menu)
  ─
  Force Quit…                    ⌥⌘⎋
  ─
  Sleep
  Restart…
  Shut Down…
  ─
  Lock Screen                     ⌃⌘Q
  Log Out <Full Name>…            ⇧⌘Q
  ```
  About This Computer opens a small window (280×360): device name `title2`, model (`/sys/class/dmi/id/product_name`), chip/CPU, memory, startup disk, serial *hidden by default with "Show" button*, OS "rmac on Ubuntu 26.04", rmac version, buttons "More Info…" (opens Settings ▸ General ▸ About). Restart…/Shut Down…/Log Out… open the Phase 6 confirmation dialog. Sleep calls logind `Suspend`. Every row uses the real authority (`rmac-power`, `rmac-session`, `rmac-recent-documents`).

- [~] **3.3 App menus from `org.rmac.AppMenu1`.** Render exported menus with the §5.9 menu. The **app menu** (bold name) for every first-party app must contain, in order: About {App} · ─ · Settings… ⌘, (if the app has settings) · ─ · Services › (disabled, until implemented) · ─ · Hide {App} ⌘H · Hide Others ⌥⌘H · Show All · ─ · Quit {App} ⌘Q. Hide/Hide Others/Show All call the Phase 2 actions. Raise `MAX_ITEMS_PER_MENU` from 32 → 64 and add submenu support to the wire format (`WireItem` gets an optional `children` vector; bump interface to `org.rmac.AppMenu2`, keep v1 reader for one release). (**Verified live:** the promoted Files binary owns `org.rmac.Files.Menu`; the bar renders `Finder · File · Edit · View`, and clicking File opens the real menu — *New Folder ⌘⇧N · New Tab ⌘T · Close Tab ⌘W · Move to Trash ⌘⌫ · Get Info ⌘I* — via the virtual-pointer probe. A real bug surfaced first: the projected focused app id blips between `None` and `Some("org.rmac.Files")` at idle, which cleared menus and closed an open one; mitigated in the menubar by ignoring `None` transitions, keeping the last-known app, anchoring popups to the opened app, and retrying the fetch (`ae3ef63`, `30bb62d`, `0b151b8`, `49b6d66`). The projection blip itself is still worth fixing. **Bold-name app menu built:** the bar now synthesizes an app menu at index 1 and draws the bold name as its button (`app_menu`, `app_menu_name`). Rows are About {App} (disabled — no app ships About metadata yet), Services › (disabled), Hide {App} ⌘H, Hide Others ⌥⌘H, Show All, and Quit {App} ⌘Q (omitted for Files, §2.9). Hide/Hide Others/Show All use the new `rmac-compositor::ParkingStore` (persisted to `$XDG_RUNTIME_DIR/rmac/parking.json`) plus `application_windows`/`visible_windows_except`; Quit closes every window via `windows_of_application` + niri `CloseWindow`. `rmac-compositor` unit tests cover the store and window selection (`7dfd52d`, `17b690f`); the shell builds clean from `shell/` with its pinned 1.95.0 toolchain. **Verified live 2026-09-17:** the bold name opens the synthesized menu with the spec rows and states — Files shows About/Services dimmed, `Hide Finder`/`Hide Others` enabled, `Show All` dimmed, and **no Quit** (§2.9); clicking Hide parked the app's window on `rmac-parking` and recorded its origin; reopening the menu enabled `Show All`, which restored the window to its origin workspace and cleared the store. A second real bug surfaced: opening a popup moves keyboard focus to the layer surface, so the live focused app drops to `None`, which disabled every row and flipped the bold name to the "Finder" desktop fallback — fixed by binding the app menu to the status runtime's last-known app id and naming it via `rmac-shell-ui::app_display_name` while a popup is open (`09d9a71`, `a6674f6`). Still to do: `MAX_ITEMS_PER_MENU` 32→64, `AppMenu2` submenus, About, app Settings, and a live check of Quit (selection is unit-tested; it shares the Dock's proven niri close path).)
- [ ] **3.4 GTK exported menus.** Take the focused window's app-id from niri. If a D-Bus name equal to that app-id exists on the session bus, introspect its object path (app-id with `.` → `/`), and read an `org.gtk.Menus` menubar plus `org.gtk.Actions`. Most GTK4/libadwaita apps export no menubar and use in-window menus. For those, show the FD-8 fallback, which is the correct behavior. Qt/Electron `com.canonical.dbusmenu` *(may defer)*. First spike on `gnome-text-editor` and one GTK3 app, then write the result into `docs/top-bar.md` before building the full importer.
- [~] **3.5 Status items (right side), right-to-left order:** clock · Control Center · Search · Focus · Now Playing (When Active) · Sound · Bluetooth · Battery · Wi-Fi/Ethernet/VPN · third-party StatusNotifierItems · privacy indicator. (**Verified live 2026-09-17:** the rmac mark + app name lead, and Wi‑Fi/Bluetooth/Sound/Battery/Search/Control Center/clock render right-to-left on the reference PC. Fixed a real 0.2 regression first: the promoted bin packages resolved `assets/...` under their own dir instead of `shell/assets`, so the mark and status/OSD icons silently failed to load — corrected to `../../assets/...` (`634d073`). A transient case where the trailing items were absent until the status runtime published was observed; it resolved on restart without a code change. Still to build for this task: the menu-style status popovers, Focus/Now Playing/VPN rows, third-party SNI, and the privacy indicator.)
  Each has Settings "Show in Menu Bar": Always / When Active / Never (replace `IndicatorSettings` booleans with an enum; migrate `true`→Always, `false`→Never; defaults per FD-7). Icons 16 px, 22×22 hit, 4 px spacing between items, 10 px trailing inset.
  Each item opens a **menu-style popover** (Tahoe status menus are wide menus with rich rows), width 300, `material.menu`:
  - **Wi-Fi:** header row "Wi-Fi" + switch; "Known Network" section (connected network with lock + signal, checkmark filled blue circle); "Other Networks" disclosure listing scan results (live via NetworkManager, sorted by strength), "Other…"; ─; "Wi-Fi Settings…". Joining a secured network opens the password sheet via the existing secret agent. Ethernet-only machines: glyph `ethernet`, rows show interface and IP.
  - **Bluetooth:** switch; "Devices" (connected first, battery % if BlueZ exposes); click toggles connection; ─; "Bluetooth Settings…".
  - **Sound:** "Sound" title, thick slider; "Output" list of PipeWire sinks with checkmark; ─; "Sound Settings…".
  - **Battery:** "Battery" title + `87%`; "Power Source: Battery/Power Adapter"; "Using Significant Energy" list (only if a truthful per-app energy source exists, otherwise omit the section); Low Power Mode switch mapped to power-profiles-daemon `power-saver`; ─; "Battery Settings…". Setting "Show Percentage" (off by default).
  - **Focus:** list of Focus modes with toggles and "for 1 hour / until this evening" submenu; ─; "Focus Settings…".
  - **Now Playing:** MPRIS player artwork 40, title, artist, prev/play/next.
  - **VPN:** only if a VPN profile exists: list with connect switches.
  - **Third-party tray (SNI):** render icons monochrome-adapted when they are symbolic; open their dbusmenu menus. Never show a Linux tray box.
- [ ] **3.6 Privacy indicator.** Show a 6 px dot left of Control Center: orange = microphone in use, green = camera, purple = screen or system audio capture. Source: PipeWire node states with `media.class` Audio/Source, Video/Source and portal ScreenCast sessions. Clicking opens a menu listing apps using each sensor. Control Center also shows the attribution header.
- [~] **3.7 Clock.** Format per `ClockSettings` + locale: `Wed 17 Sep  3:45 PM` (en-IN/en-US order per locale). Tabular numerals. Updates on minute boundary via one timer aligned to the next minute (no per-second wake unless "Show seconds"). Click toggles Notification Center (Phase 5). Clock settings pane: Show date (Always/When Space Allows/Never), Show day of week, Show AM/PM, Display time with seconds, Flash the time separators, Use a 24-hour clock, Announce the time (*may defer*). (**Verified live 2026-09-17 with the virtual-pointer probe + `grim`:** default shows `Thu 17 Sep 9:44 PM`; a `ClockSettings` change to 24-hour + seconds renders `21:45:17` — `target/evidence/phase3/clock{,_2}.png`; clicking the clock opens Notification Center with real notifications. Minute-boundary wakeup, tabular numerals, and the clock Settings pane remain; defaults restored after the check.)
- [ ] **3.8 Search and Control Center icons.** Search icon toggles Spotlight (Phase 5). Control Center icon toggles Control Center. Both highlight with the pressed pill while their overlay is open.
- [ ] **3.9 Menu bar customization.** ⌘-drag status items to reorder (persist order in shell settings); ⌘-drag out of the bar removes (sets Never) with the "poof" fade 200 ms.
- [ ] **3.10 Accessibility.** Menu bar exposes `menubar` role; each item `menuitem` with popup; status items expose value text (e.g. "Wi-Fi, connected to Home, 3 of 3 bars"). Existing `assert_top_bar_accessibility.py` extended accordingly.

**Phase 3 exit checklist**
- [~] Screenshot pairs: light wallpaper, dark wallpaper, menu open, submenu, each status menu, full-screen reveal. (**Verified live 2026-09-17** with the virtual-pointer probe + `grim`: with a focused window fullscreened via `niri msg action fullscreen-window`, the policy watcher publishes `{output_uuid: true}` and the bar **hides** — the top strip shows the window's toolbar instead (`target/evidence/phase3/fs2-hidden.png`). **Open bug:** moving the pointer to the top 2 px does not reveal the bar while fullscreen (`fs2-rev.png`). Root-caused with an instrumented probe: when hidden the bar sets a correct 2 px top input region and `visible=false`, but a pointer move to y=1 never yields `pointer_inside=true` — so the compositor does not deliver the hover to the hidden surface. A `top: 0` + `opacity 0` variant also failed and was reverted. Fix likely needs a different mechanism (e.g., niri-side reveal or a separate 1–2 px input surface); recorded, not yet solved.)
- [ ] Every system-menu row performs its real action on Ubuntu (Sleep/Restart/Shut Down/Log Out tested on the reference PC with recovery).
- [ ] Files, Settings, Terminal, Notes, Text Editor, System Monitor show complete real menus; Firefox shows FD-8 fallback; a GTK app (e.g. `gnome-text-editor`) shows its exported menus.
- [ ] Turning Wi-Fi off from GNOME Settings/`nmcli` updates the bar within 1 s, no polling.
- [ ] Idle: the menu bar process wakes ≤ 1×/min with seconds off.

---

### Phase 4 — Dock

**Goal:** Tahoe Dock with every Desktop & Dock setting, correct groups, animations, menus, drag, and Trash.
**Read first:** `docs/dock.md`, `docs/dock-motion.md`, `crates/rmac-dock/src/*`, `crates/rmac-dock-runtime`, `crates/rmac-dock-system`, `shell/bins/rmac-dock`, `crates/rmac-shell-settings/src/model.rs`.

#### 4.0 Target picture (bottom, size 56, light)

```
        ╭──────────────────────────────────────────────────────────────╮
        │ [Files] [Apps] [Firefox] [Term] [Notes] [Settings] │ [Trash] │   ← glass shelf, radius 26
        ╰───•──────────────•───────────•─────────────────────────────────╯   ← 4 px running dots
         8 px padding, 8 px gaps, separator 1×(tile*0.6) centered, 6 px above screen edge
```

#### Tasks

- [ ] **4.1 Settings model.** Extend `DockSettings` (serde defaults for migration, version bump in `migration.rs`):
  `size: f32 = 56.0` (32–96), `magnification_size: f32 = 1.5 × size capped 128` (replace `magnification_scale` with absolute px, migrate), `minimize_effect: Genie|Scale = Scale`, `titlebar_double_click: Fill|Minimize|DoNothing = Fill`, `minimize_into_app_icon: bool = false`, `animate_opening: bool = true`, `show_indicators: bool = true`, `show_recent_apps: bool = false`, `recent_apps_limit: u8 = 3`, plus Windows section fields used in Phase 2 (`tile_by_edge_drag`, `fill_by_menubar_drag`, `tile_with_option`, `tiled_margins`), and `click_wallpaper_reveals_desktop: Never|Always = Never`. Keep `repeated_click` (not a macOS setting; hide it in UI, keep CycleWindows behavior).
  Verify: `cargo test --locked -p rmac-shell-settings` including a v-old → v-new migration fixture.

- [ ] **4.2 Groups and separators.** Order: kept apps (pinned) │ recent/unkept running apps (only if `show_recent_apps`, or always for running-but-unpinned apps — macOS shows running unpinned apps in the recent group regardless; keep that) │ folders/files kept by user, minimized windows (if not into app icon), Trash. A separator renders only between two non-empty groups. Order never changes on focus. Unpinned running apps append in launch order.
- [ ] **4.3 Geometry + glass.** Shelf height = `size + 2×padding`; shelf radius = `size × 0.46`; tiles are the icon only (no hover background square). `material.dock`, `elev.dock`. Exclusive zone = shelf height + bottom margin when `reserve_space` (Tahoe windows avoid the Dock when filling). Left/right placement: vertical shelf, same rules, anchored to middle of the edge.
- [ ] **4.4 Hover and tooltip.** Tooltip label above the tile: `material.tooltip`, `body`, 6 px above shelf top; first hover 0 ms delay once pointer is inside the Dock (macOS shows immediately), labels update instantly while moving. No tooltip during drag or magnification motion > 0.
- [~] **4.5 Magnification.** When on: each tile's size = `size + (mag − size) × cos²(π·d/(2·R))` for `d < R`, `R = 2.5 × size`, `d` = distance from pointer to tile center along the Dock axis; the shelf grows so total stays centered; tiles grow upward only. Pointer leave → animate back 200 ms. Reduced motion → no magnification. Stable centers: order and relative position never jump (existing "stable magnification" commit `c983474` implements parts; complete it to this formula). (Verified live 2026-09-17: enabling `dock.magnification` + `magnification_scale` in `shell.json`, then hovering a tile, grew the hovered tile and its neighbours with the tooltip; `target/evidence/phase4/magnify.png`. Magnification left off (FD-7 default) after the check.)
- [~] **4.6 Click behavior.** (Verified live 2026-09-17 with the virtual-pointer probe: clicking the Dock's Terminal tile launched/focused `org.rmac.Terminal`; hover shows the tile tooltip. Bounce/alert, unhide, ⌥/⌘/middle-click variants still to verify.) Not running → launch (bounce if `animate_opening`: §4.9, stops when first window maps or after 10 s; if launch fails show alert "The application “X” can't be opened." with details). Running, no visible windows → unhide/restore the most recent window. Running, windows visible, not focused → focus most recent window. Focused → no change (macOS: does nothing; do not cycle). ⌥-click → hide others after activating. ⌘-click → reveal in Files. Middle click → new window if the app's desktop entry has a `new-window` action.
- [ ] **4.7 Indicators and badges.** Running dot 4×4 `label.primary` @ 70%, 3 px under the icon (hidden if `show_indicators` off). Badge: red `system.red` capsule, min 18×18, top-right, `footnote` bold white, from Unity LauncherEntry `count` over D-Bus (`com.canonical.Unity.LauncherEntry`) and our notification unread count per app (only if app allows badges in Notifications settings). Progress bar under the icon from LauncherEntry `progress`. Urgent window → one bounce (repeat every 10 s up to 3 times).
- [~] **4.8 Context menu (right-click or press-and-hold 500 ms).** (Verified live: a non-running tile (Firefox) opens *Firefox* · Open · New Window · New Private Window · Open Profile Manager · ─ · Remove from Dock · Show in Finder; the running Terminal tile opens *Terminal* · **jacob@Jake: ~ — Terminal ✓** (key-window row) · ─ · Remove from Dock · Show in Finder · ─ · Quit, with the running dot and hover tooltip. Still to do: the Options `›` submenu (Keep in Dock/Open at Login/Show in Files), Hide, ⌥→Force Quit, Show All Windows, and the Trash/folder menus.) Rows by state: **Measured 2026-09-18:** menu ≈ 165 px wide, radius ≈ 10, with a small **triangular pointer at the bottom** aimed at the tile, and **no icons on any row** (`dock-context-menu-dark.png`).
  ```
  <window titles with ✓ for key window, • for minimized>   (running only)
  ─
  New Window                  (if desktop action exists)
  <other desktop-file actions>
  ─
  Options  ›  Keep in Dock ✓ | Open at Login | Show in Files | ─ | Assign To: All Desktops / This Desktop / None (only if niri supports; else omit)
  ─
  Show All Windows            (running; opens App Exposé)
  Hide                        (running)
  Quit                        (running; holds ⌥ → Force Quit)
  ```
  Trash menu: Open · ─ · Empty Trash (disabled if empty; confirmation alert "Are you sure you want to permanently erase the items in the Trash?" with "Empty Trash" destructive default and "Cancel").
  Folder item menu: Sort by (Name/Date Added/Date Modified/Date Created/Kind), Display as (Folder/Stack), View content as (Fan/Grid/List/Automatic), ─, Options › Remove from Dock, Show in Files.
- [~] **4.9 Drag and drop.** (**Horizontal reorder implemented and verified 2026-09-17**: the Dock host now retrieves the per-output `SurfaceDescription`, prepares a `ShelfLayoutPlan`, and drives the existing `DragSession` from tile press + shelf-level move/up, rendering the `preview_order` (neighbour parting) live and committing `UpdatePins(MoveTo)` on drop. With the virtual-pointer probe, dragging Firefox right moved it to position 5 and **persisted across a Dock restart** (`~/.config/rmac/shell.json` rewritten); dragging it back restored the FD-7 order — `target/evidence/phase4/drag{3,4,5,6}.png`. Still to do: the out-of-Dock "Remove" puff, drag-in from Apps/Files, file-drop-to-open, folder stacks, and volume-eject.) Drag a kept app horizontally → neighbors part with 200 ms spring; drop to reorder (persist). Drag an app tile up ≥ 1.5 tiles out of the Dock and release → "Remove" puff (tooltip "Remove" shown while outside) for kept, non-running apps; running apps unpin but stay in recent group. Drag an app from Apps/Files into the app group → insert and pin. Drag files onto an app tile → highlight tile if app's MimeType accepts, open with it on drop. Drag files onto Trash → move to Trash via `rmac-places-system` with undo available in Files. Drag a folder to the right group → add folder stack. Drag a mounted volume onto Trash → tile changes to eject symbol and ejects.
- [ ] **4.10 Folder stacks.** Click opens Fan (≤ 12 items, arced up) or Grid (popover `material.popover`, 5-column grid of 64 px icons with names, "Open in Files" button) per settings. Keys: arrows, Return opens, Escape closes.
- [x] **4.11 Minimize target.** When minimize into app icon is off, minimized windows appear as live thumbnail tiles (static snapshot taken at minimize, scaled to tile, with app badge 20 px bottom-right) in the right group; click restores. (**Stage 1 done, `f5cbfcd`+`586c2c4`:** `ParkedWindow` now carries `app_id`, `title`, and a `thumbnail` path (`#[serde(default)]`, so old store files still load); `ParkingStore::thumbnail_path_beside`/`default_thumbnail_path` own the path under `$XDG_RUNTIME_DIR/rmac/thumbnails/`, and `set_thumbnail` attaches a capture. `rmac-compositor::window_logical_rect` derives a window's on-screen rectangle from its output position + `tile_position_in_view` + `window_offset_in_tile`. On the minimize traffic light the app captures that rectangle with `grim -g` **before** parking and records the path. **Verified live 2026-09-17:** minimizing the dev Files window wrote `{window: 27, workspace: 4, app_id: "org.rmac.Files", title: "Computer — Finder", thumbnail: ".../thumbnails/27.png"}` and the 1375×900 PNG is the Files window. `cargo test -p rmac-compositor` ✔ 18. **Stage 2 done (`8f757fe`+`3758c22`):** parked windows no longer count as running — `dock.rs` splits them out via a `parking_workspaces` filter and builds `MinimizedItem`s (id, app id, title, catalog icon, thumbnail path). `EntryId::Minimized(WindowId)` + `Entry.miniature` carry the tile through `ShelfContent::project`, which prepends them to the right group so Trash stays rightmost; the four accessibility matches handle them (restore action, no menu). `rmac-dock-system` gained `Activation::RestoreWindow`, `Operation::Restore`, `Outcome::RestoreRequested`, a defaulted `Backend::restore_window`, and `prepare_minimized_activation` (fails closed for stale tiles); `SystemBackend::restore_window` reads the origin from `ParkingStore`, forgets it, and sends niri `RestoreWindow`. The Dock bin grows its geometry by the tile count, renders each tile (thumbnail image, app-icon badge bottom-right at 20 px, tooltip, magnify) before Trash, and dispatches `ActivateEntry(Minimized)` on click. `cargo test -p rmac-dock` ✔ 55, `-p rmac-dock-system` ✔ 35. **Verified live 2026-09-17:** with a dev Files window parked, the Dock showed it as a thumbnail tile (own snapshot + 20 px app-icon badge) at the right end of the shelf and its normal app icon correctly had **no running dot**; clicking the tile moved the window back to its recorded origin workspace, emptied `parking.json`, removed the tile, and restored the running dot — `target/evidence/phase4/dock_min{2,3}.png`. **Remaining:** the per-window "minimize into app icon" setting (4.15) currently defaults to showing tiles; a minimized tile has no context menu yet; and the capture still shells out to `grim` — a first-party screencopy replaces it later. (`cargo test -p rmac-dock-system` has one **pre-existing** failure unrelated to this work: `special_activation_resolves_the_current_private_path_at_prepare_time` expects a Downloads place, but `project_special_items` has been Trash-only; it fails identically on the prior commit.)
- [~] **4.12 Autohide.** Hidden: shelf slides off-edge 250 ms; reveal when pointer touches the edge for 0 ms (Tahoe default delay), hide 500 ms after leaving. Menu open prevents hiding. Setting in Desktop & Dock: "Automatically hide and show the Dock". ⌥⌘D toggles autohide. (Verified live 2026-09-17: with `dock.autohide` in `shell.json` the shelf is absent over the wallpaper, and moving the pointer to the bottom edge reveals it — `target/evidence/phase4/autohide-{hidden,shown}.png`. Animation timing and the ⌥⌘D toggle still to confirm; defaults restored after the check.)
- [ ] **4.13 Keyboard.** ⌃F3 (Full Keyboard Access "Move focus to the Dock") focuses the first tile with focus ring; ←/→ move; Return/Space activate; ↑ opens context menu; Escape leaves.
- [ ] **4.14 Multi-display.** Dock lives on the display where the pointer last touched the Dock edge (macOS behavior with separate Spaces), defaulting to primary; `outputs` setting keeps `All` option for users who prefer per-output Docks.
- [ ] **4.15 Desktop & Dock settings pane** shows exactly (Phase 7.2 renders it): Size slider (Small…Large), Magnification switch + slider (Off…Max), Position on screen (Left/Bottom/Right), Minimize windows using (Genie Effect/Scale Effect), Double-click a window's title bar to (Fill/Minimize/Do Nothing), Minimize windows into application icon, Automatically hide and show the Dock, Animate opening applications, Show indicators for open applications, Show suggested and recent apps in Dock.

**Phase 4 exit checklist**
- [ ] Screenshot pairs: idle, hover tooltip, magnification, context menu (running/not running), Trash full/empty, stack grid, autohide revealed.
- [ ] Launch bounce stops on window map; failed launch shows alert.
- [ ] Reorder, remove, add, drag-open, drag-trash all persist across `systemctl --user restart rmac-dock`.
- [x] Dock idle CPU 0% and no redraw when pointer is outside (render counter from `RMAC_DOCK_RENDER_COUNT_DIR` stays flat for 60 s). **Verified live 2026-09-18:** with `RMAC_DOCK_RENDER_COUNT_DIR=/run/user/1000/rmac-dock-renders` set on the dev Dock unit, the counter held at **3** for a full 60 s with the pointer away from the Dock (no idle redraw).

---

## 8. Phase 5: overlays (Spotlight, Control Center, Notifications, OSD, Apps, switcher, Force Quit, screenshots)

**Goal:** every transient system surface is a layer-shell surface in `shell/`, opens on the focused output, never enters window MRU, dismisses correctly, and looks like Tahoe.
**Read first:** `docs/launcher.md`, `docs/quick-settings.md`, `docs/notifications.md`, `docs/focus.md`, `docs/global-shortcuts.md`, `crates/rmac-launcher-runtime`, `crates/rmac-quick-settings`, `crates/rmac-notifications-runtime`, `crates/rmac-notifications-linux/src/banner.rs`, `crates/rmac-shell-invocation*`, `crates/rmac-shell-activation-runtime`.

**Common overlay contract (applies to 5.1–5.8):**
- New bin per surface in `shell/bins/` reusing the existing GPUI-free runtime crate; the old stable-GPUI app crate (`launcher-app`, `quick-settings-app`, `notification-center-app`, `app-drawer`) is kept only until its replacement passes, then removed from packaging and the niri `window-rule`s for `org.rmac.Launcher`, `org.rmac.QuickSettings`, `org.rmac.NotificationCenter` are deleted from `shell.kdl`.
- Layer: `overlay` for Spotlight/Force Quit/screenshot/switcher, `top` for Control Center/NC/banners/OSD. Keyboard interactivity: `on-demand` while open (exclusive for Spotlight/switcher/Force Quit), `none` when closed. Surface is unmapped (not just transparent) when closed.
- Invocation goes through the existing action-scoped sockets (`rmac-shell-invocation`), so shortcut, menu bar icon, and CLI all toggle the same instance.
- Dismiss: Escape; click outside (full-output transparent input catcher on the same layer while open, which forwards nothing); invoking again; focus moving to an app via keyboard shortcut. Returning focus: restore the previously focused window.
- Open on the output that has keyboard focus (niri `focused-output`).
- niri `layer-rule` per namespace (`rmac-spotlight`, `rmac-control-center`, `rmac-notification-center`, `rmac-notification-banners` (existing name in `surfaces.rs`), `rmac-apps`, `rmac-switcher`) with `geometry-corner-radius` = token and `background-effect { blur true; }` — no `shadow` from niri for menus that draw their own.

### 5.1 Search (Spotlight)

**Measured 2026-09-18:** the bar is **642 × 59**, pill radius ≈ 29.5, horizontally centred with its
top edge at **23.2 % of output height**. Typing shows an **inline completion inside the bar**
("terminal.app — Open") with the target's glyph in a rounded square at the right end; a result list
appears only for multi-result queries. Build the inline completion first.

```
                   ╭──────────────────────────────────────────────╮   ← 642 wide, 59 high, radius 29.5
                   │ ⌕  Search                             [A][▢][⌘][▤] │   ← trailing: Apps, Files, Actions, Clipboard filters (Tahoe)
                   ╰──────────────────────────────────────────────╯
                   ╭──────────────────────────────────────────────╮
                   │ TOP HIT                                        │
                   │ ▣ Terminal                         Application │ ← 44 px rows, selected = accent fill radius 10
                   │ APPLICATIONS                                   │
                   │ ▣ Text Editor                                  │
                   │ FILES                                          │
                   │ ▢ termination-notes.md        ~/Documents      │
                   │ SYSTEM SETTINGS                                │
                   │ ⚙ Keyboard Shortcuts                           │
                   ╰──────────────────────────────────────────────╯
```

- Position: horizontally centered, top edge at 22% of output height (Tahoe). Field font 22 Regular, placeholder `label.placeholder` **"Search"**.
- Empty state (no query): bar only, plus a row of up to 8 recent app icons (60 px) under it if "Show recent" is enabled.
- Results panel appears below the bar as a separate glass card (gap 8), max 10 visible rows, scroll after. Section headers `caption` `label.tertiary`.
- Row: icon 28, title `body`, trailing kind or path `subheadline label.secondary`. Selected: accent fill, white text.
- Filters (Tahoe "browse" buttons): ⌘1 Applications, ⌘2 Files, ⌘3 Actions, ⌘4 Clipboard (Clipboard only if clipboard history is enabled with consent; default off → hide button).
- Keys: ↑/↓ move; Return opens; ⌘Return reveal in Files; ⌘C copy path/value; ⌘I Get Info (Files); Tab moves focus to filters; Escape clears query, second Escape closes. Typing while closed does nothing (no global type-to-search).
- Providers (existing): apps, settings panes/controls (deep link), files/recents, calculator. Add unit/currency conversions only with offline rates (currency: omit unless a real source is configured). Calculator shows result as the top hit row with "= 4" `title2`, Return copies.
- Actions provider: "Lock Screen", "Sleep", "Empty Trash", "New Note", "New Terminal Window", "Toggle Dark Mode", "Toggle Wi-Fi" — each maps to a real command.
- Timing: visible ≤ 80 ms after shortcut (p95), first results ≤ 150 ms for apps/settings, files stream in without reordering rows above the selection.
- The current category-pill "app browser" UI in `launcher-app` is removed; browsing apps is the Apps surface (5.5).

### 5.2 Control Center

```
╭────────────────────────────────────╮  width 340 (S), anchored 6 px below menu bar, right edge 10 px inset
│ ╭─────────────────╮ ╭─────────────╮ │
│ │ (ᯤ) Wi-Fi       │ │   ▶ Now     │ │  Connectivity module (2×2): Wi-Fi, Bluetooth, (VPN), (Airplane hidden)
│ │     Home        │ │   Playing   │ │  each: 36 px circle toggle (accent when on, fill.control when off) + title + subtitle
│ │ (ᛒ) Bluetooth   │ │             │ │
│ │     On          │ ╰─────────────╯ │
│ ╰─────────────────╯ ╭─────────────╮ │
│ ╭────────╮╭───────╮ │ ☾ Focus     │ │
│ │ ◐ Dark │ │ ▭ Mir │ ╰─────────────╯ │  small 1×1 modules: Appearance toggle, Screen Mirroring(Displays)
│ ╰────────╯╰───────╯                  │
│ ╭──────────────────────────────────╮ │
│ │ Display   ☀ ════════════○────     │ │  thick slider module (full width)
│ ╰──────────────────────────────────╯ │
│ ╭──────────────────────────────────╮ │
│ │ Sound     🔈 ═════════○──────  ›  │ │  › opens output device list inline
│ ╰──────────────────────────────────╯ │
│                     Edit Controls   │  footer text button
╰────────────────────────────────────╯
```

**Measured 2026-09-18:** panel width **287**, top edge ≈ 34 px below the menu bar, right inset ≈ 20.
The real layout is a **single column of mixed-size modules**, not the 2×2 grid drawn above: Wi-Fi
pill, Bluetooth pill, AirDrop pill (rmac: omit — no AirDrop), Now Playing card beside them, then a
row of **circular** buttons (appearance, screen mirroring, Focus), then full-width **Display** and
**Sound** slider modules, then **Edit Controls** bottom-right. Pills use radius ≈ 26 with a filled
circular glyph badge. Rebuild the ASCII sketch above to match `settings-appearance-dark.png` and
`control-center-dark.png` in `target/evidence/reference-mac/`.

- Panel: `material.popover`, radius 20 (R), padding 12, module gap 10. Modules: `surface` `FFFFFF 59` light / `FFFFFF 14` dark over the glass, radius 18.
- Circular toggle: 36 px, on = `accent` + white glyph; off = `fill.control` + `label.primary` glyph; pending = spinner inside; unavailable authority = module hidden (not grayed).
- Clicking the **circle** toggles; clicking the **title area** expands the module in place into a detail list (Wi-Fi networks, Bluetooth devices, Focus modes, Sound outputs) with a back chevron header, `motion.standard` height morph.
- Brightness module only if a backlight device exists (`rmac-display`); Keyboard Brightness only if `kbd_backlight` exists; Battery module only on battery hardware.
- Now Playing: MPRIS artwork 48, title/artist, prev/play/next; hidden when no player.
- **Edit Controls** (Tahoe customization): enters edit mode: modules jiggle-free outline, "–" remove badges, a Controls Gallery sheet below lists addable controls (Wi-Fi, Bluetooth, Focus, Appearance, Displays, Sound, Now Playing, Brightness, Keyboard Brightness, Battery, Screen Recording (opens 5.8), Timer (if Clock app exists), Accessibility Shortcuts, Stage Manager **omitted**); drag to reorder; sizes 1×1, 2×1, 2×2 where the control supports. Persist layout in shell settings `control_center.layout`.
- Existing `QuickSettingsView` card UI with a mute `Toggle` is replaced entirely. Mute lives on the speaker glyph inside the Sound slider (click glyph toggles mute).
- Keys: Tab/arrows move between modules, Space toggles, Return expands, Escape collapses or closes.

### 5.3 Notification Center and widgets

- Panel anchored top-right below menu bar, width 356, full output height minus menu bar and 10 px; no background panel — **individual glass cards** float over the wallpaper (Tahoe), `material.hud` cards radius 18.
- Top: notifications grouped by app (stack of up to 3 visible offset cards, "Show Less"/"Clear" header buttons on expand). Each card: app icon 20 + app name `subheadline label.secondary` + time `footnote`, title `headline`, body `callout` up to 4 lines, optional attachment thumbnail 40 right. Hover shows `xmark` clear button at top-left corner and `⋯` Options menu (Mute for 1 hour / Mute for Today / Turn Off Notifications… / Settings…).
- Below: widgets **only with real providers**: Calendar-free "Date" widget (today's date/weekday, no events unless a calendar provider exists), Weather omitted, Battery widget (devices incl. Bluetooth battery), System Monitor mini (CPU/memory) *(may defer)*. "Edit Widgets" button at bottom.
- Opens by clicking clock or two-finger swipe left from the right trackpad edge (if niri exposes gesture; else *may defer*); ⌥-click clock toggles Do Not Disturb Focus.
- "Clear All" appears as `×` on hover over the group header.

### 5.4 Banners and alerts (pixels for `banner.rs`)

- Bin `shell/bins/rmac-banners` hosts the existing `rmac-notifications-linux` banner session as a layer surface anchored top-right, 10 px below menu bar, 10 px right inset, width 356.
- Banner card = NC card design; enters `motion.banner`; stacks newest on top, max 3 visible, older collapse into the group.
- **Banner** style auto-dismisses after 5 s (pointer hover pauses). **Alert** style stays until acted on and shows action buttons stacked on hover ("Options" menu for > 2 actions).
- Click body → default action + focus app; drag right → dismiss to NC; Reply-type actions show inline text field.
- Focus active → suppressed per Focus policy (existing), counted in NC. Screen locked → nothing on screen unless "Show previews: Always".
- Sound: default rmac original sound (not Apple's), respects "Play sound for notifications".
- a11y: announce title/body via live region (existing presenter `Announcement`).

### 5.5 Apps (Tahoe Apps / Launchpad replacement)

**Corrected from the 2026-09-18 capture (`apps-grid-dark.png`): this is a floating panel, not a full-screen Launchpad.**
- Floating glass panel ≈ **840 × 570**, centred, `material.popover`, radius `radius.large`; the desktop behind is untouched (no full-screen blur, no scrim). Opens from the Dock "Apps" tile, F4, or the pinch gesture.
- Top row: search field with a grid glyph and placeholder **"Applications"**, plus a `⋯` menu at the right (Sort by Name / Sort by Category, Show in Files).
- Below it: **category chips** — Productivity & Finance · Utilities · Developer Tools · Creativity · Social · Entertainment · Other — from desktop-entry categories, hiding empty groups. (These are the pills task 5.1 removes from Spotlight; they live here.)
- Grid: **7 columns**, icons ≈ 56 with one-line labels beneath, vertical scrolling — **no pages, no page dots**.
- Drag an icon onto the Dock pins it; right-click → Open, Keep in Dock, Show in Files, (Uninstall: only if the package authority supports removal with confirmation; otherwise omit).
- Type anything → search field focuses and filters live; Return launches first match; Escape clears/closes; clicking empty space closes.
- Existing list/grid view toggle and "Reveal in Finder" wording removed; "Show in Files".

### 5.6 OSD (volume/brightness/keyboard backlight/mute)

- [~] Keep existing layer surface (R: 304×74, radius 28, hold 1600 ms) but restyle to Tahoe: horizontal capsule under the menu bar on the right side (top-right, 10 px below bar, aligned with Control Center column), content: glyph 22 + title `headline` ("Display"/"Sound"/"Keyboard") + thick slider 16 high, output device name `subheadline` for sound. (**Verified live 2026-09-17:** running the dev `osd volume-up` shows the rounded HUD capsule near the top-right with the real output name "Built-in Audio Analog Stereo", speaker glyphs, a thick slider, and tick dots — `target/evidence/phase5/osd.png`. Still to do: exact title/step behavior, brightness/keyboard variants, fine-adjust, and in-place repeat.)
- Repeated key presses update in place (no re-animation); ⌥⇧+key = quarter steps (fine adjust) via `rmac-osd` step size.

### 5.7 App switcher (⌘Tab) and window cycling (⌘`)

- New bin `rmac-switcher` (overlay layer, exclusive keyboard). Holding Super, Tab advances; ⇧Tab back; release Super activates; Escape cancels; Q quits selected app; H hides it; ↑/↓ shows App Exposé for the selected app.
- Look: centered glass panel, radius 22, icons 96 with 12 px gaps, selected app gets `FFFFFF 33` rounded 20 plate, name `body` under the selected icon.
- Order: most-recently-used **applications** (not windows), including hidden/minimized apps. Replace niri `recent-windows` binds `Mod+Tab` in `shell.kdl` with the rmac switcher; keep `Mod+grave` (window cycling within app) through niri `recent-windows filter="app-id"`.

### 5.8 Force Quit, screenshots, logout dialog hosts

- **Force Quit (⌥⌘⎋):** 420×400 window-like overlay: title "Force Quit Applications", list of running apps with icons (hung apps show "(not responding)" in red when niri reports unresponsive or the app misses 5 s ping), "Force Quit" button (default, destructive confirmation alert "Do you want to force “X” to quit?"), SIGTERM then SIGKILL after 3 s via `rmac-app-launch` process tracking. Files row says "Relaunch".
- **Screenshots:** ⇧⌘3 full output; ⇧⌘4 region (crosshair with pixel size readout; Space toggles window mode with highlighted window); ⇧⌘5 toolbar at bottom center (Capture Entire Screen / Window / Selection / Record Entire Screen / Record Selection / Options: Save to Desktop|Documents|Clipboard, Timer None/5/10, Show Floating Thumbnail, Show Mouse Pointer). Use niri's built-in `screenshot`, `screenshot-window`, `screenshot-screen` actions where they suffice, and the xdg ScreenCast portal + PipeWire + GStreamer for recording. Floating thumbnail bottom-right 5 s, click opens preview, drag out to drop file. Filename `Screenshot 2026-09-17 at 3.45.12 PM.png` (existing `screenshot-path` pattern). Menu bar shows stop-recording button (■ in circle) while recording + purple privacy dot.
- **Logout/Restart/Shutdown dialog:** Phase 6.

**Phase 5 exit checklist**
- [ ] None of the overlays appear in niri window list, MRU, or overview.
- [ ] Each overlay opens on the focused output of a two-monitor setup.
- [ ] Screenshot pairs for Spotlight (empty/typing/calc), Control Center (collapsed/expanded Wi-Fi/edit mode), NC (grouped), banner, Apps page, switcher, OSD, Force Quit, ⇧⌘5 toolbar.
- [ ] Spotlight p95 visible latency ≤ 80 ms, Control Center ≤ 100 ms (measured with the Phase 8 harness).
- [ ] `notify-send "Hello" "World"` from a terminal shows a banner within 200 ms and lands in NC.

---

## 9. Phase 6: lock screen, login, power

**Goal:** one authentication per lock/login, Tahoe lock screen look, real power actions with unsaved-work safety.
**Read first:** `docs/secure-lock.md`, `docs/secure-lock-recovery.md`, `docs/decisions/0004-secure-lock-boundary.md`, `docs/decisions/0005-rmac-lock-provider-state-machine.md`, `docs/rmac-pam-wrapper-audit.md`, `crates/rmac-lock-provider-linux/src/{paint,text_renderer,surface}.rs`, `packaging/rmac-session/greeter/*`.

- [ ] **6.1 Lock screen visual.** Rendered by `rmac-lock-provider-linux` (ext-session-lock). Layout at any resolution:
  - Background: the current wallpaper of that output, Gaussian-blurred (radius 30 logical) and darkened `00000033`, computed once per lock (not per frame). If wallpaper unavailable: `1C1C1E`.
  - Top center at 12% height: date `lock.date` (e.g. "Wednesday 17 September"), below it time `lock.time` 96 Semibold, white `FFFFFFE6`.
  - Bottom center at 80% height: user avatar 64 circle (AccountsService icon or initials on `system.gray`), full name `headline` white, password field 200×30 radius 15 `FFFFFF33` fill, placeholder "Enter Password", caret white, dots `•` for characters, trailing `→` circle button appears when non-empty.
  - Wrong password: field shakes horizontally 3× 8 px over 400 ms (Reduce Motion: none), text clears, below it "Incorrect password" `callout` white 80%.
  - Caps Lock on: ⇪ glyph inside field (existing `caps_lock.rs`).
  - Bottom-right: keyboard layout short name (if > 1 layout), accessibility glyph (opens on-screen keyboard if available), battery glyph on laptops.
  - Lock message (Settings ▸ Lock Screen ▸ "Show message when locked") at bottom above avatar.
  - Displays other than the focused one show background + time only.
- [ ] **6.2 One password.** Suspend path: logind `PrepareForSleep` → lock surface mapped **before** suspend; resume shows the same surface; GDM never shows in between. Verify no double prompt through: lock → suspend → resume → unlock; lid close; idle lock; ⌃⌘Q; wrong password ×3 then right.
- [ ] **6.3 Greeter branding (GDM).** Keep GDM as the login authority. Provide only the existing gschema override (logo, background `rmac-aurora.svg`). Do not replace GDM for 1.0.
- [ ] **6.4 Session dialogs.** Restart…/Shut Down…/Log Out… open a centered alert (§5.11) on the focused output: icon (restart/power/logout glyph 56), title "Are you sure you want to shut down your computer now?", message "If you do nothing, the computer will shut down automatically in 60 seconds.", checkbox "Reopen windows when logging back in" (only if session restore exists; else omit), buttons Cancel / **Shut Down** default with live countdown. Holding ⌥ while choosing the menu item skips the dialog. Before acting: ask every running first-party app over its AppMenu D-Bus for unsaved documents; if any → the app raises its own save sheet and the shutdown is cancelled with message "Shut Down was interrupted because Notes has unsaved changes." Use logind `PowerOff`/`Reboot`/session `Terminate` only after all clients allow; respect logind inhibitors and show their `Why` text.
- [ ] **6.5 Idle and display sleep.** Settings ▸ Lock Screen: "Start Screen Saver when inactive" (Never for 1.0; no screen saver) ; "Turn display off on battery when inactive" (1–180 min, default 2 min); "Turn display off on power adapter when inactive" (default 10 min); "Require password after screen saver begins or display is turned off" (Immediately default, 5 s, 1 min, 5 min, 15 min, 1 h, 8 h). Implement with `ext-idle-notify-v1` + niri `power-off-monitors` + lock provider.

**Phase 6 exit checklist**
- [ ] Recorded journey: lock, wrong password, unlock once, suspend/resume, lid close, idle display-off → one prompt each time.
- [ ] Shut Down with an unsaved note is interrupted with the correct message; without unsaved work it powers off.
- [ ] GNOME recovery session and TTY recovery unaffected.

---

## 10. Phase 7: first-party applications

**Goal:** each app's daily journey passes on Ubuntu with real services and Tahoe-level UI.
**Order:** 7.1 Files → 7.2 System Settings → 7.3 Terminal → 7.4 Notes → 7.5 Text Editor → 7.6 System Monitor.
**Common app contract (every app, check each box per app):**

- [ ] Opens centered at last size (first run: size given below), unified toolbar with traffic lights, sidebar material where it has a sidebar.
- [ ] Exports complete menus via `org.rmac.AppMenu2` (app menu per 3.3 + the menus listed per app). Every shortcut listed works with `super-` and the `ctrl-` alias (FD-4).
- [ ] Window menu per 2.5. Help menu: search field (searches the app's menu items and highlights them) + "{App} Help" opening `docs/user-guide.md#<app>` in the browser.
- [ ] Settings… ⌘, opens the app settings window (tabbed toolbar, fixed width 540) if the app has settings.
- [ ] Light/dark/high-contrast/reduced-transparency; accent color live; text scale; 200% scale.
- [ ] Keyboard-only complete; accessibility names on every control.
- [ ] Empty, loading, error states designed (§5.15), no raw error strings; errors say what happened + one next action.
- [ ] Autosave/restore of window frame, sidebar width, selection, scroll position.
- [ ] Startup ≤ 500 ms warm (Files/Terminal ≤ 900 ms); idle CPU 0% and no redraw when unchanged.

### 7.1 Files (Finder)

**First run size** 920×560. **Read first:** `docs/places.md`, `crates/finder/src/view*`, `trash_store.rs`, `operation_journal.rs`, `undo_journal.rs`, `conflict.rs`, `quick_look.rs`.

**Window layout**
```
╭──────────────────────────────────────────────────────────────────────────────╮
│ ● ● ●   │ ‹ ›  Documents                 [▦ ☰ ⫴ ▭] [⊞▾] [⇪] [🏷] [⋯]  ⌕ Search │ 52 toolbar (glass groups)
│─────────┤──────────────────────────────────────────────────────────────────────│
│Favorites│                                                                      │
│ ⌂ jacob │   [icon]      [icon]      [icon]                                     │
│ ▭ Desktop│  Budget.ods  Photos     notes.md                                   │
│ ▤ Docume│                                                                      │
│ ↓ Downlo│                                                                      │
│ ▣ Applic│                                                                      │
│ ⏲ Recent│                                                                      │
│Locations│                                                                      │
│ 💽 Disk  │                                                                      │
│ ⏏ USB   │                                                                      │
│ 🗑 Trash │                                                                      │
│Tags     │──────────────────────────────────────────────────────────────────────│
│ ● Red   │ Macintosh HD › Users › jacob › Documents          (path bar, optional)│
│ ● Orange│ 12 items, 180.4 GB available                     (status bar, optional)│
╰─────────┴──────────────────────────────────────────────────────────────────────╯
 sidebar 180 default (140–320), material.sidebar
```

**Sidebar sections (exact):** Favorites: Recents, Applications, Desktop, Documents, Downloads, home folder (user name) — user can drag folders in/out and reorder. Locations: this computer's root volume (named from `hostnamectl --pretty` or "Computer"), mounted volumes with ⏏ eject buttons on hover, Network (only if GVfs network browsing available). Tags: Red, Orange, Yellow, Green, Blue, Purple, Gray, All Tags…. Hidden items configurable in Settings ▸ Sidebar.

**Toolbar items (left→right):** Back/Forward group (chevrons; press-and-hold shows history menu) · title (folder name + icon, ⌘-click title shows path menu) · View segmented group (Icons ⌘1, List ⌘2, Columns ⌘3, Gallery ⌘4) · Group By pop-up (None/Name/Kind/Application/Date Last Opened/Date Added/Date Modified/Date Created/Size/Tags) · Share (only installed real targets: e.g. email client via `xdg-email`, else hide) · Tags pop-up · Action `⋯` menu (same as context menu) · Search field (expands from icon to 200 px on click).

**Views**
- **Icons:** icon size 64 default (16–512 slider in View Options ⌘J), grid spacing, text 12 below, label up to 2 lines with middle truncation, selection: icon gets `selection.unfocused` rounded 6 plate, name gets accent capsule. Rubber-band select. Arrange By/Sort By from View Options. Thumbnails via `rmac-thumbnails`.
- **List:** columns Name (with disclosure triangles for inline expansion), Date Modified, Size, Kind; optional Date Created, Date Last Opened, Date Added, Version, Comments, Tags. Row 24, alternating rows, header sort, column reorder/resize, right-click header for columns. ⌘→ expands, ⌘← collapses, ⌥-click triangle expands recursively.
- **Columns:** each column 220 min, resize handle `||` at column bottom; selecting a file shows preview column (large thumbnail 256, name `title3`, kind/size/created/modified/last opened, "More…"). → enters folder, ← goes up.
- **Gallery:** large preview on top (fit), filmstrip thumbnails 64 below, metadata sidebar right (⇧⌘P toggles Preview pane), quick actions row (Rotate Left, Markup — omit if not implemented; show only real actions).

**Selection and keys:** Return = rename (inline field, name selected without extension); ⌘O / ⌘↓ open; ⌘↑ enclosing folder; ⌘[ ⌘] back/forward; Space = Quick Look; ⌘I Get Info; ⌘D duplicate; ⌘⌫ move to Trash (with undo ⌘Z); ⌥⌘⌫ delete immediately (confirmation alert); ⇧⌘N new folder; ⌃⌘N new folder with selection; ⌘C/⌘V copy/paste; ⌥⌘V move; ⌃⌘A make alias (symlink named "X alias"); ⌘E eject; ⇧⌘G Go to Folder sheet with autocomplete; ⇧⌘H home; ⇧⌘D Desktop; ⌥⌘L Downloads; ⇧⌘O Documents; ⇧⌘A Applications; ⇧⌘F Recents; ⌘T new tab; ⌘N new window; ⌘F search; ⌘J View Options; ⇧⌘. show hidden files; type-ahead select.

**Context menu (file):** Open · Open With › (default app ✓, other apps, Other…) · Move to Trash · ─ · Get Info · Rename · Compress "X" · Duplicate · Make Alias · Quick Look · ─ · Copy · Share… (if real) · ─ · Tags (color dots row + Tags…) · ─ · Show in Enclosing Folder (only in search/recents) · Open in Terminal (Services-equivalent, under a "Quick Actions ›" submenu) . Background context menu: New Folder · Get Info · ─ · Import… (omit) · Paste Item · ─ · Show View Options · Use Groups · Sort By › · ─ · Open in Terminal.

**Menus:** Files (app menu: About Files, Settings…, Empty Trash… ⇧⌘⌫, Hide/Show, no Quit) · File (New Files Window ⌘N, New Folder ⇧⌘N, New Folder with Selection ⌃⌘N, New Tab ⌘T, Open ⌘O, Open With ›, Close Window ⌘W, Get Info ⌘I, Rename, Compress, Duplicate ⌘D, Make Alias ⌃⌘A, Quick Look ⌘Y, Print ⌘P (if printable), Show Original ⌘R (symlink), Add to Sidebar ⌃⌘T, Move to Trash ⌘⌫, Eject ⌘E, Find ⌘F) · Edit (Undo/Redo with action names e.g. "Undo Move of 3 Items", Cut disabled for files (⌥⌘V moves instead), Copy, Paste, Select All, Show Clipboard) · View (as Icons/List/Columns/Gallery, Use Groups ⌃⌘0, Sort By ›, Clean Up, Hide/Show Tab Bar ⇧⌘T, Show All Tabs, Hide/Show Path Bar ⌥⌘P, Hide/Show Status Bar ⌘/, Hide/Show Sidebar ⌃⌘S, Show Preview ⇧⌘P, Hide Toolbar ⌥⌘T, Customize Toolbar…, Show View Options ⌘J, Enter Full Screen fn F) · Go (Back, Forward, Enclosing Folder, Recents, Documents, Desktop, Downloads, Home, Computer ⇧⌘C, Network ⇧⌘K (if available), Applications, Utilities ⇧⌘U (omit), Recent Folders ›, Go to Folder… ⇧⌘G, Connect to Server… ⌘K (GVfs smb/sftp/webdav URL sheet, real mount)) · Window · Help.

**File operations UI:** copy/move > 1 s shows a progress window (title "Copying 3 items to Documents", bar, "12.4 MB of 1.2 GB — About 30 seconds", ✕ cancel, pause if supported). Multiple ops stack in one window. Conflict sheet: "An item named “a.txt” already exists in this location. Do you want to replace it with the one you're moving?" buttons Keep Both · Stop · **Replace**, checkbox "Apply to all". Folder merge shows Merge when ⌥ held. Errors: "The operation can't be completed because you don't have permission to access “X”." with OK. Low space: "“Y” can't be copied because there isn't enough space. 1.2 GB needed." Existing journals already provide recovery; surface "Resume interrupted copy of 3 items?" alert at next launch.

**Get Info window** (240 wide, disclosure sections): header icon 32 + name + size + modified; General (Kind, Size "12,345 bytes (16 KB on disk)", Where, Created, Modified, Stationery pad omitted, Locked checkbox = immutable attr only if permitted else omit); More Info (dimensions/duration from metadata); Name & Extension (editable, Hide extension); Comments (xattr `user.xdg.comment`); Open with (pop-up + "Change All…"); Preview (thumbnail); Sharing & Permissions (owner/group/others rows with Read & Write/Read only/Write only/No Access pop-ups, lock button → polkit for foreign files).

**Tags:** stored as xattr `user.xdg.tags` (comma separated). Colors fixed as §4.3. Show dots after file names in all views.

**Quick Look (Space):** floating panel 60% of output, `material.hud` title bar with file name, "Open with X" button, share (if real), fullscreen; renders images, PDF (first page via `pdftoppm`/poppler if installed; otherwise icon + metadata), text/code (mono, syntax colors none in 1.0), audio/video (GStreamer if available; else metadata). ←/→ moves between selected items. Space/Escape closes.

**Search:** typing in search field searches "This Computer" (default) or current folder (scope bar under toolbar: `Search: This Computer | "Documents"`), results list view with "Kind" and "Date Last Opened". Tokens: `kind:pdf`, `name:`, date filters via ＋ criteria row. Uses `rmac-search`. Save search → saved smart folder in sidebar.

**Trash window:** toolbar button "Empty" top-right; context menu "Put Back" for items with origin data (existing trash store), "Delete Immediately…".

**Desktop icons** *(may defer to after 1.0 if niri layer-surface desktop is not feasible)*: show Desktop folder items + mounted volumes on the wallpaper layer, Finder semantics.

**Files exit checks:** copy 2 GB folder with conflict → keep both → undo; cancel mid-copy; eject USB while copying (error + partial cleanup); Trash → Put Back; Quick Look image/PDF/text; tag a file red and find it via sidebar tag; Connect to Server sftp; screenshots of all four views light/dark.

### 7.2 System Settings

**First run size** 715×620 (min 715×450; macOS Settings fixed-width-ish). Sidebar 248 (R, keep).
**Read first:** `docs/system-settings-audit.md`, `docs/settings-guide.md`, `crates/system-settings/src/navigation.rs`, `controller/*`.

**Window layout**
```
╭───────────────────────────────────────────────────────────────╮
│ ● ● ●                     │ ‹ ›  Wi-Fi                          │ toolbar 52: title in detail pane
│ ⌕ Search                  │                                     │
│ ╭──╮ Jacob Samas          │  ╭───────────────────────────────╮  │
│ │JS│ Local Account        │  │ (ᯤ) Wi-Fi                 [●] │  │ grouped rows (surface.grouped.row, radius 12)
│ ╰──╯                      │  ╰───────────────────────────────╯  │
│ ▣ Wi-Fi                   │  Known Network                      │ section header: callout semibold label.secondary, 20 px inset
│ ▣ Bluetooth               │  ╭───────────────────────────────╮  │
│ ▣ Network                 │  │ Home        🔒 ᯤ  ✓        (⋯) │  │ row 36 (S) with trailing controls
│ ▣ Battery                 │  ╰───────────────────────────────╯  │
│ ─                         │  Other Networks                     │
│ ▣ General                 │  ╭───────────────────────────────╮  │
│ ▣ Accessibility           │  │ Office      🔒 ᯤ               │  │
│ ...                       │  │ Other…                          │  │
│                           │  ╰───────────────────────────────╯  │
│                           │                      [Advanced…] [?] │ footer buttons right-aligned, ? = help
╰───────────────────────────┴─────────────────────────────────────╯
```

- Sidebar row: 20 px colored squircle icon (white glyph on color from §4.3), `body` label, row 28; groups separated by 10 px space (no lines). Account header row 52: avatar 40 + name `headline` + "Local Account" `subheadline`. Selecting it opens Users & Groups.
- Detail pane: background `surface.grouped.background`, content max width 480 centered, grouped boxes radius 12 with 0.5 px `separator` border, rows separated by inset lines (inset 12 from leading text), row min height 36, title `body` left, control right (mini switch, pop-up, slider, value text + chevron for subpages). Row descriptions `subheadline label.secondary` under the title.
- Subpages push with a slide from right 250 ms, Back ⌘[ returns. Search: sidebar filters matching panes and shows a dropdown of matching controls; choosing one navigates and **highlights the row with an accent 2 px ring pulsing twice**.
- Every mutating row: pending spinner in place of control, error inline under the row in `danger`, readback after success (R architecture exists; enforce visually).
- Risky changes (display resolution/scale/arrangement, network DNS/proxy): "Keep these display settings?" alert with 15 s countdown, **Revert** default.

**Sidebar order (exact; hide panes whose authority is absent):**
1. Account header
2. Wi-Fi · Bluetooth · Network · VPN *(only if a VPN profile exists or after "Add VPN Configuration…")* · Battery *(laptop)* / Energy *(desktop; power profiles + sleep)*
3. General · Accessibility · Appearance · Menu Bar *(new, Phase 3 settings)* · Spotlight *(rename visible to "Search")* · Desktop & Dock · Displays · Wallpaper
4. Notifications · Sound · Focus
5. Lock Screen · Privacy & Security · Login Password *(new: change password via AccountsService/passwd with PAM; polkit)* · Users & Groups *(new: AccountsService list, add/remove standard/admin with polkit, automatic login off by default)*
6. Keyboard · Mouse *(if mouse present)* · Trackpad *(if touchpad present)* · Printers & Scanners *(new: CUPS via `rmac-print-linux`; list printers, add via IPP Everywhere discovery, default printer, open queue)*

**General subpages (exact order):** About · Software Update · Storage · ─ · Date & Time · Language & Region · Login Items & Extensions · Sharing · ─ · Startup Disk *(omit)* · Transfer or Reset *(omit for 1.0)*. Move the existing top-level Date & Time, Language & Region, Login Items, Sharing panes under General (routes keep working: `rmac-settings://date-time` still deep-links).

**Pane specs (only real controls; each row lists control type):**
- **Wi-Fi:** Wi-Fi switch · Known Network section · Other Networks (live scan, "Other…" sheet: Network Name, Security pop-up, Password) · "Ask to join networks" pop-up (Off/Notify/Ask) *(only if NM supports; else omit)* · footer Advanced… (Known Networks list with Auto-Join switch and Remove) · Details sheet per network (TCP/IP: Configure IPv4 Automatic/Manual, IP, Router; DNS servers list +/−; Proxies (Auto/Manual/None); Hardware MAC address; "Forget This Network…" destructive).
- **Bluetooth:** switch · "My Devices" (name, battery, Connected/Not Connected, ⓘ → Rename/Disconnect/Forget) · "Nearby Devices" (discovering spinner, Connect button → pairing sheet showing passkey comparison) · footer "Advanced…" omitted unless needed.
- **Network:** services list (Ethernet, Wi-Fi, VPN, Thunderbolt Bridge omit) with status dot (green/yellow/red) · ⓘ Details per service (same TCP/IP/DNS/Proxy tabs) · `⋯` pop-up: Add Service Order… (only if NM priority works), Add VPN Configuration › (WireGuard, OpenVPN, IPsec if plugins installed) · Firewall row → Privacy & Security firewall *(if ufw present; else omit)*.
- **Battery:** Low Power Mode pop-up (Never/Always/Only on Battery/Only on Power Adapter → power-profiles-daemon policy), Battery Health row "Normal/Service Recommended" + capacity % (UPower energy-full/energy-full-design), usage chart last 24 h / 10 days *(only if UPower history exists; else omit chart, never fake)*, Options… (Optimize video streaming omit; "Prevent automatic sleeping on power adapter when display is off", "Wake for network access" omit unless supported).
- **General ▸ About:** device name (editable via hostnamed), Chip/CPU, Memory, Graphics, Serial (hidden until "Show"), OS & rmac version, Displays, Storage row, System Report… (opens System Monitor "System Information" tab *(may defer)*), Regulatory omit, Licenses (opens third-party license list).
- **General ▸ Software Update:** "rmac 1.0.1 is available" card with Update Now / release notes; Automatic updates ⓘ (Check for updates, Download new updates when available, Install security responses) via `rmac-updates-linux`; APT packages summary "12 other updates" with "Open Software Center" if present.
- **General ▸ Storage:** colored capacity bar by category **only from real measurement** (Applications=/usr+flatpak, Documents=$HOME/Documents, Downloads, Trash, System=rest) computed on a worker with progress; Recommendations: Empty Trash Automatically (30 days via rmac trash store), Reduce Clutter (opens Files sorted by size); per-category ⓘ list.
- **General ▸ Date & Time:** Set time and date automatically (timedated NTP) · Source (display) · Time zone: Set automatically (geoclue, only if consent) · Closest city pop-up search · 24-hour time switch · Show 24-hour time on Lock Screen.
- **General ▸ Language & Region:** Preferred Languages list (+/−, drag order, restart-required banner), Region, Calendar (Gregorian), Temperature, Measurement system, First day of week, Date format, Number format, Live preview row; Applications section (per-app language) omit.
- **General ▸ Login Items & Extensions:** "Open at Login" list (+/− with app picker), "Allow in the Background" list (systemd user units/XDG autostart with switches).
- **General ▸ Sharing:** Content & Media: File Sharing (Samba if installed), Remote Login (SSH, needs polkit), Remote Management/Screen Sharing (gnome-remote-desktop if installed); Local hostname editable. Rows absent when the service is not installed.
- **Accessibility:** Vision: Screen reader (Orca switch + shortcut), Zoom (niri has no zoom → omit unless implemented), Display (Increase contrast, Reduce transparency, Reduce motion, Pointer size slider, Differentiate without color, Text size → global TextScale), Spoken Content *(omit)*. Hearing: Audio (Flash the screen when an alert sound occurs), Captions omit. Motor: Keyboard (Sticky keys, Slow keys, Full Keyboard Access switch), Pointer Control (mouse keys *if* supported). General: Shortcut (⌥⌘F5 panel listing enabled toggles).
- **Appearance (macOS 27 order, measured):** Appearance tiles Light/Dark/Auto (picture previews) · **Liquid Glass** live preview + intensity slider · Theme → **Colour** (Multicolour + 8 dots) · **Text highlight colour** pop-up (Automatic) · **Icon & widget style** tiles (Default/Dark/Clear/Tinted) · Folder color pop-up *(Files feature)*, Sidebar icon size (Small/Medium/Large), Allow wallpaper tinting in windows switch *(omit until material implements it)*, Show scroll bars (Automatically/When scrolling/Always), Click in the scroll bar to (Jump to next page / Jump to spot).
- **Menu Bar (new):** Automatically hide and show the menu bar pop-up, Show menu bar background switch, Recent documents/applications/servers count pop-up, "Menu Bar Controls" list: each control with Show in Menu Bar pop-up (Always/When Active/Never), Clock Options… sheet (3.7 fields), Allow in the Menu Bar list of third-party SNI items with switches.
- **Search (Spotlight):** Search results categories with switches (Applications, Files, Folders, System Settings, Calculator, Actions, Clipboard history + consent text), "Help Apple improve…" omitted, Search Privacy… sheet (excluded paths +/−, existing), Include removable volumes switch (existing).
- **Desktop & Dock:** exactly 4.15 fields, then Desktop & Stage Manager: Show items On Desktop (if desktop icons implemented), Click wallpaper to reveal desktop pop-up, Stage Manager **omitted**; Widgets omitted; Default web browser pop-up (xdg-settings); Windows: Prefer tabs when opening documents (Always/In Full Screen/Never — first-party apps honor), Ask to keep changes when closing documents, Close windows when quitting an application, Drag windows to screen edges to tile, Drag windows to menu bar to fill screen, Hold ⌥ key while dragging windows to tile, Tiled windows have margins; Mission Control: Automatically rearrange Spaces based on most recent use (omit unless niri can reorder workspaces), When switching to an application, switch to a Space with open windows, Group windows by application, Displays have separate Spaces (read-only on) ; Shortcuts… sheet; Hot Corners… sheet (4 pop-ups: – / Mission Control / Application Windows / Desktop / Notification Center / Launchpad→Apps / Quick Note (if Notes) / Lock Screen / Put Display to Sleep / Start Screen Saver omitted) — implement hot corners in `rmac-dock`'s pointer barrier watcher or a dedicated 1 px layer surface per corner.
- **Displays:** arrangement canvas (drag rectangles; only when >1 output), per-display: name, "Use as" (Main display / Extended / Mirror if supported), Resolution tiles "Larger Text … More Space" (5 scale presets from available scales) + "Show all resolutions" list, Refresh rate pop-up, Rotation pop-up, Brightness slider (backlight only), Automatically adjust brightness (ambient sensor only), Night Shift… sheet (schedule, temperature slider via `wlsunset`/gammastep only if installed — else omit). Keep/Revert 15 s alert for resolution/scale/arrangement/rotation (existing logic).
- **Wallpaper:** current wallpaper preview 200×125 + name + fit pop-up (Fill/Fit/Stretch/Center/Tile) + "Show on all Spaces" (always on; hide) ; galleries: rmac Wallpapers (original assets, light/dark dynamic pairs), Colors (solid swatches + custom color picker), Your Photos (Add Folder or Photo… via portal file chooser). Per-display pop-up when multiple outputs.
- **Notifications:** Notification Center: Show previews (Always/When Unlocked/Never), Summarize omitted; Allow notifications when the display is sleeping, when the screen is locked, when mirroring or sharing the display; Application Notifications list (icon, name, subtitle "Banners, Sounds, Badges") → subpage: Allow notifications, alert style picture tiles (None/Banners/Alerts), Show notifications on Lock Screen, Show in Notification Center, Badge application icon, Play sound for notifications, Show previews pop-up, Notification grouping pop-up (Automatic/By Application/Off).
- **Sound:** Sound Effects: Alert sound list (original rmac sounds) with preview, Play sound on startup (omit), Play user interface sound effects, Play feedback when volume is changed, Alert volume slider; Output & Input segmented: device table (Name, Type), Output volume slider + Mute, Balance slider (if device has 2 channels); Input volume slider + live level meter (PipeWire peak).
- **Focus:** modes list (Do Not Disturb, Sleep, Personal, Work + "Add Focus…" with icon/color picker) → subpage: Allowed People (omit, no contacts provider) / Allowed Apps list, Set a Schedule (time/app triggers), Focus filters omitted; Share Focus status omitted.
- **Lock Screen:** 6.5 fields + Show large clock (On Lock Screen/Always/Never — Always omitted), Show user name and photo, Show password hints (omit), Show message when screen is locked + Set…, Login window shows (List of users / Name and password) via GDM config with polkit.
- **Privacy & Security:** Privacy list: Location Services (geoclue app permissions), Camera, Microphone, Screen & System Audio Recording, Files & Folders, Accessibility, Input Monitoring (portal permission store tables with per-app switches and "Reset" — label "Revoke saved permission", never claims universal revocation), Security: Allow applications from (App Store→"Software Center & known developers"/Anywhere — omit if not enforceable), FileVault → "Disk Encryption: On/Off (LUKS)" read-only status, Lockdown Mode omit, Firewall row (ufw if present), Developer Tools omit, Advanced… (Require an administrator password to access system-wide settings — polkit default read-only info).
- **Keyboard:** Key repeat rate slider (Off…Fast), Delay until repeat slider (Long…Short) via niri input config write + reload, Adjust keyboard brightness in low light (omit), Keyboard navigation switch (Full Keyboard Access), Keyboard Shortcuts… sheet (sidebar: Launchpad & Dock→Apps & Dock, Display, Mission Control, Keyboard, Input Sources, Screenshots, Presenter Overlay omit, Services omit, Spotlight→Search, Accessibility, App Shortcuts, Function Keys, Modifier Keys) listing §13 shortcuts with editable key recorders and conflict warnings; Text Input: Input Sources Edit… (layouts via niri xkb, IBus/Fcitx engines if installed), Text Replacements… (store for rmac apps), Dictation omit.
- **Mouse:** Tracking speed slider, Natural scrolling switch, Secondary click pop-up (Click right side/left side), Double-click speed omit unless supported, Scrolling speed slider.
- **Trackpad:** tabs Point & Click (Tracking speed, Click pressure omit, Force Click omit, Look up & data detectors omit, Secondary click pop-up (Click with two fingers/bottom right corner/bottom left), Tap to click), Scroll & Zoom (Natural scrolling, Zoom in or out (pinch) only if supported, Smart zoom omit, Rotate omit), More Gestures (Swipe between pages, Swipe between full-screen applications (3/4 fingers → niri workspace gestures), Notification Center (two fingers from right edge, may defer), Mission Control (swipe up), App Exposé (swipe down), Launchpad→Apps (pinch), Show Desktop (spread)) — each only when implemented; each row has a looped demo illustration (original SVG animation, static under Reduce Motion).
- **Users & Groups (new):** current user row (avatar edit via portal file picker, admin badge), other users list, "Add User…" sheet (Name, Account name, Password, Verify, Type Standard/Administrator) via AccountsService + polkit, ⓘ per user: Allow user to administer this computer, Delete User… with home handling options; Guest User omit; Automatic login pop-up (Off default, polkit).
- **Printers & Scanners:** printer list with status, "+ Add Printer, Scanner, or Fax…" sheet (discovered IPP devices, Use: driverless), Default printer pop-up, Default paper size pop-up, ⓘ Options & Supplies (queue name, location, "Printer Queue…" opens queue window: jobs list, Pause/Resume/Delete). Scanners omit unless SANE present.

**Settings exit checks:** each pane on a laptop and desktop profile; toggle every control and verify external tool reflects it (`nmcli`, `bluetoothctl`, `wpctl`, `powerprofilesctl`, `timedatectl`, `localectl`, `niri msg outputs`); change externally → UI updates ≤ 1 s; search "night" → Displays ▸ Night Shift row highlighted (or "No Results" if omitted); all panes screenshot light/dark.

### 7.3 Terminal

**First run size** 80×24 cells + chrome (compute from font metrics). **Read first:** `crates/terminal/src/*`, `docs/shortcuts.md`.

- **Chrome:** unified titlebar 28 (titlebar-only window; not 52), title center: `{cwd basename} — {process} — {cols}×{rows}` (Terminal default title components; configurable). Tabs: when > 1 tab, tab bar 26 under titlebar, tabs equal width, active tab `surface.window`, inactive `surface.chrome`, close ✕ on hover left side, `+` at right. Remove `TITLE_BAR_HEIGHT 34`/`TAB_BAR_HEIGHT 32` literals → tokens.
- **Profiles (Settings… ⌘, → Profiles tab):** sidebar list of original rmac profiles (Basic default, Midnight, Paper, Grass (green on black), Amber, Ocean, Sepia, Solarized-style Light/Dark with original values); each has Text tab: Background color + opacity + blur, Font (JetBrains Mono 12), Text color, Bold text color, Selection color, Cursor (Block/Underline/Vertical Bar, Blink), ANSI 16 colors; Window tab: title components, columns/rows; Tab tab: title; Shell tab: When the shell exits (Close if clean/Close window/Don't close), Ask before closing (Always/Never/Only if processes other than login shell and: screen, tmux); Keyboard tab: Use Option as Meta key; Advanced: Terminfo `xterm-256color`, bell (visual/audible/badge Dock icon).
- **General tab:** On startup open New window with profile, New windows open with (Default Profile / Same Profile), New windows open with (Same working directory / Default working directory), New tabs open with same.
- **Menus:** Terminal app menu; Shell (New Window ⌘N, New Tab ⌘T, New Command… ⇧⌘N, New Remote Connection omit, Import/Export profile, Close Window ⇧⌘W, Close Tab ⌘W, Use Settings as Default, Print ⌘P (visible text), Export Text As… ⌘S); Edit (Undo omit, Cut disabled, Copy ⌘C, Copy Special › Copy with Styles, Paste ⌘V, Paste Escaped Text ⌃⌘V, Paste Selection ⇧⌘V (primary selection), Select All ⌘A, Clear to Previous Mark ⌘L (uses shell-integration marks), Clear Scrollback ⌘K, Clear Screen ⌥⌘K; Find › Find ⌘F, Find Next ⌘G, Find Previous ⇧⌘G, Use Selection for Find ⌘E, Jump to Selection ⌘J; Marks › Mark ⌘U, Jump to Previous Mark ⌘↑, Jump to Next Mark ⌘↓; Emoji & Symbols ⌃⌘Space); View (Show/Hide Tab Bar ⇧⌘T, Show All Tabs ⇧⌘\\, Hide/Show Marks, Scroll to Top ⌘Home, Scroll to Bottom ⌘End, Page Up ⌘PgUp, Page Down ⌘PgDn, Line Up ⌥⌘PgUp, Line Down ⌥⌘PgDn, Bigger ⌘+, Smaller ⌘−, Default Font Size ⌘0, Enter Full Screen); Window (standard + Select Next Tab ⌃Tab/⇧⌘], Select Previous Tab ⌃⇧Tab/⇧⌘[, Move Tab to New Window, Merge All Windows, tabs ⌘1…⌘9); Help.
- **Find bar:** slides under titlebar 30 high: search field, match count "3 of 12", ‹ › buttons, Done; highlights all matches (`FIND_HL` → token `system.yellow` @ 60%).
- **Behavior:** super-key commands only (FD-4); Ctrl/Alt go to the PTY. ⌘-click URLs and file paths (OSC 8 hyperlinks exist) open with `xdg-open`/Files. Right-click menu: Copy, Paste, Open URL/Reveal in Files (if hovered link/path), Look Up omit, Clear Scrollback, Inspector omit, Show/Hide marks. Drag a file into terminal inserts shell-quoted path. Close confirmation sheet per profile rule: "Do you want to terminate running processes in this window? Closing this window will terminate the running processes: vim, ssh." [Cancel] [**Terminate**].
- **Dock badge:** bell in background tab → badge count on Terminal Dock icon (if enabled).
- **Exit checks:** vim, htop, tmux, ssh, `less` with mouse, IME input (Hindi via IBus), emoji width, 256-color and truecolor test script, resize reflow, 100k line scrollback memory ≤ 150 MB, paste of 1 MB text with bracketed paste confirmation for multi-line (existing `paste.rs`), `yes` flood stays responsive.

### 7.4 Notes

**First run size** 1000×640. **Read first:** `crates/notes/src/*`, `crates/rmac-notes-*`.

```
╭────────────────────────────────────────────────────────────────────────────────────╮
│ ● ● ●  [▯]         │ [☰ ▦]  [🗑]         │ [✎] [Aa] [☑] [▦] [📎] [🔗]   ⌕ Search    │ toolbar 52 across 3 panes
│ Folders (210, R)   │ Notes list (310, R) │ Editor                                  │
│ On My Computer     │ Pinned              │ 17 September 2026 at 3:45 PM (centered) │
│  ▤ Notes        12 │ ╭─────────────────╮ │                                         │
│  ▤ Work          4 │ │Shopping list    │ │ Shopping list                           │ title = first line (title2 → Title style 24 bold)
│  ▤ Recently Del. 2 │ │3:45 PM  Milk, eg│ │ ☑ Milk                                  │
│ Tags               │ ╰─────────────────╯ │ ☐ Eggs                                  │
│  #recipes          │ Today               │                                         │
│                    │  …                  │                                         │
│ [+ New Folder]     │                     │                                         │
╰────────────────────────────────────────────────────────────────────────────────────╯
```

- **Folders sidebar:** section "On My Computer" (no iCloud), folders with counts `label.tertiary`, Quick Notes omit, Shared omit, "Recently Deleted" (30-day app trash, existing), Tags section with `#tag` chips (All Tags, individual tags, multi-select filter). "+ New Folder" bottom-left 28 high. Smart Folders: "New Smart Folder…" with tag/date/attachment filters (only after tags work).
- **Notes list:** grouped by Pinned, Today, Yesterday, Previous 7 Days, Previous 30 Days, month names; row 64: title `headline` 1 line, second line `subheadline`: time/date `label.secondary` + first body line `label.tertiary`, attachment thumbnail 40 right when present; selected row radius 8 `notes.selection` (R token) when list focused else `selection.unfocused`. Gallery view (⌘2 toggles): cards 160×190 with preview. Swipe/right-click: Pin, Move to ›, Delete, Duplicate, Share omit, Lock omit (unless encrypted notes implemented), Export as PDF….
- **Editor:** content width max 720 centered with 24 px side padding; date header `subheadline label.tertiary` centered, first line becomes note title. Format (Aa) popover: paragraph styles Title (24 bold), Heading (18 bold), Subheading (15 semibold), Body (13), Monostyled (JetBrains Mono 12, `fill.control` background), Bulleted List, Dashed List, Numbered List, Block Quote; character styles Bold ⌘B, Italic ⌘I, Underline ⌘U, Strikethrough ⇧⌘X, highlight colors (5 system colors). Checklist ⇧⌘L with round checkboxes (checked = `notes.accent` fill + white check) and "Move checked items to bottom" option. Table ⌥⌘T (rows/cols add via hover handles) **only if storage supports it; else omit button**. Attachments 📎: images inline (max width content, rounded 8), files as 64 px icon cards (Quick Look on Space). Links: ⌘K adds link; auto-detect URLs. Collapsible headings (chevron on hover).
  Store format stays the existing storage model; any unsupported style is simply not offered.
- **Toolbar (left→right in editor pane):** Compose ✎ (⌘N), Format Aa, Checklist ☑, Table ▦ (if supported), Attachment 📎 (portal picker), Link, Share omit, Lock omit, Search field (searches all notes, results highlight matches in list and editor with `system.yellow` @ 50%, existing `search_highlight.rs`).
- **Menus:** File (New Note ⌘N, New Folder ⇧⌘N, Close ⌘W, Import to Notes… (Markdown/plain text via portal), Export as PDF…, Export as Markdown…, Print… ⌘P, Pin Note, Duplicate Note ⌘D, Delete), Edit (Undo/Redo, Cut/Copy/Paste, Paste and Match Style ⌥⇧⌘V, Delete, Select All, Find ›, Spelling and Grammar › (only if spellchecker exists), Substitutions › Smart Quotes/Dashes/Links, Attach File… ⇧⌘A, Add Link ⌘K), Format (Title ⇧⌘T, Heading ⇧⌘H, Subheading ⇧⌘J, Body ⇧⌘B, Monostyled ⇧⌘M, Bulleted/Dashed/Numbered List ⇧⌘7/8/9, Checklist ⇧⌘L, Mark as Checked ⇧⌘U, More › Move List Item Up/Down ⌃⌘↑/↓, Font › Bold/Italic/Underline/Strikethrough/Bigger/Smaller, Text › Align Left/Center/Right, Indentation › Increase ⌘]/Decrease ⌘[), View (as List ⌘1, as Gallery ⌘2, Sort By ›, Group By Date, Show/Hide Folders ⌥⌘S, Show/Hide Note Count, Attachments Browser ⌘3 omit, Zoom In/Out/Actual Size, Enter Full Screen), Window, Help.
- **Autosave:** every edit coalesced ≤ 500 ms to storage (existing transactional store); crash recovery surfaces "Recovered 1 note edit" inline banner with Review (existing `edit_recovery_controller.rs`).
- **Exit checks:** create/pin/move/delete/recover from Recently Deleted; checklist reorder; image attach and Quick Look; export PDF opens; kill -9 while typing → no data loss on relaunch; search across 5k notes ≤ 100 ms.

### 7.5 Text Editor (TextEdit)

**First run:** on launch with no document, show the **Open panel sheet style window** (TextEdit behavior) with "New Document" button bottom-left; setting "Show Open panel on launch" in Settings (default on). **Read first:** `crates/text-editor/src/*`.

- **Window:** titlebar 28 (titlebar-only), title = file name with proxy icon (drag the icon to Files/Terminal; ⌘-click title → path menu), " — Edited" suffix `label.secondary` when dirty; clicking title shows rename/move popover (Name, Tags, Where, Locked). Ruler/format bar below titlebar **only in rich text mode** (36 high): paragraph style pop-up, font family pop-up, typeface pop-up, size combo, color well, highlight well, B I U, alignment segmented (left/center/right/justify), line spacing pop-up, list bullets pop-up.
- **Plain text mode (⇧⌘T toggles, confirm when converting rich→plain):** Inter 13 by default, like TextEdit's system font. The font is changeable in Settings; wrap to window (Format ▸ Wrap to Window / Wrap to Page).
- **Settings:** New Document tab: Format (Rich text / Plain text), Window size (width 90 chars, height 30 lines), Font (Plain text font, Rich text font with Change… buttons), Properties (author etc. omit), Options (Check spelling as you type, Check grammar omit, Smart copy/paste, Smart quotes, Smart dashes, Smart links, Text replacement). Open and Save tab: When Opening a File (Display HTML files as HTML code instead of formatted text, Display RTF files as RTF code), When Saving a File (Add ".txt" extension to plain text files), Plain Text File Encoding (Opening: Automatic; Saving: UTF-8), line endings (LF default; preserve existing).
- **Menus:** File (New ⌘N, Open… ⌘O, Open Recent ›, Close ⌘W, Save… ⌘S, Duplicate ⇧⌘S, Rename…, Move To…, Revert To › Last Saved / Browse All Versions (omit unless versions exist), Export as PDF…, Show Properties ⌥⌘P omit, Page Setup… ⇧⌘P, Print… ⌘P), Edit (Undo/Redo, Cut/Copy/Paste, Paste and Match Style, Delete, Complete ⌥⎋ omit, Select All, Insert › Page Break/Line Break, Find › Find ⌘F, Find and Replace ⌥⌘F, Find Next ⌘G, Find Previous ⇧⌘G, Use Selection for Find ⌘E, Jump to Selection ⌘J, Spelling and Grammar ›, Substitutions ›, Transformations › Make Upper Case/Lower Case/Capitalize, Speech omit, Emoji & Symbols ⌃⌘Space), Format (Font › Show Fonts ⌘T, Bold ⌘B, Italic ⌘I, Underline ⌘U, Bigger ⌘+, Smaller ⌘−, Show Colors ⇧⌘C; Text › Align Left ⌘{, Center ⌘|, Justify, Align Right ⌘}, Writing Direction omit, Show Ruler ⌘R, Copy Ruler ⌃⌘C, Paste Ruler ⌃⌘V; Make Plain Text ⇧⌘T; Prevent Editing; Wrap to Page / Wrap to Window; Allow Hyphenation omit; List…; Table… omit), View (Actual Size ⌘0, Zoom In ⇧⌘., Zoom Out ⇧⌘,, Show/Hide Tab Bar, Enter Full Screen), Window, Help.
- **Find & Replace bar:** under titlebar, 2 rows (Find field with options pop-up: Ignore Case ✓, Wrapping Around ✓, Contains/Starts With/Full Word; Replace field; buttons "All", "Replace", "Replace & Find", ‹ › , Done).
- **Documents:** open via portal; encoding detection (UTF-8/UTF-16/Latin-1 with "Reinterpret as…" alert); external change on disk → sheet "The file has been changed by another application." [Keep Editing / **Revert**]; autosave-in-place to recovery store (existing `recovery.rs`), relaunch restores windows with unsaved changes; close dirty doc → sheet "Do you want to save the changes made to the document “X”?" [Delete (⌘⌫)] [Cancel] [**Save**].
- **Exit checks:** open 50 MB log file responsive (lazy layout), UTF-16 file round-trip, CRLF preserved, RTF bold/italic round-trip (existing `rtf.rs` — bounded, documented), print to PDF, crash recovery.

### 7.6 System Monitor (Activity Monitor)

**First run size** 900×600. **Read first:** `crates/activity-monitor/src/*`.

- **Toolbar:** title "System Monitor" + subtitle "All Processes" (from View filter) · ⓧ Stop process (sheet: "Are you sure you want to quit this process?" [Cancel] [Force Quit] [**Quit**]) · ⓘ Inspect (⌘I) · `⋯` action menu (Sample Process omit, Run Spindump omit, Run system diagnostics omit → only real: Send Signal to Process…) · segmented tabs **CPU | Memory | Energy | Disk | Network** (Energy only if a truthful per-process power estimate exists — RAPL-based totals are allowed at the bottom panel but per-process "Energy Impact" is **omitted** unless measured; Tab list then CPU | Memory | Disk | Network) · Search field (filters by name/user/PID).
- **Table:** row 20 compact, alternating, columns per tab:
  - CPU: Process Name (icon 16 + name), % CPU, CPU Time, Threads, Idle Wake Ups (from `/proc/<pid>/sched` nr_voluntary_switches delta only if accurate; else omit), % GPU omit, GPU Time omit, PID, User.
  - Memory: Process Name, Memory, Threads, Ports omit, PID, User (Compressed Memory omit unless zram per-process exists).
  - Disk: Process Name, Bytes Written, Bytes Read, Kind (64-bit), PID, User (from `/proc/<pid>/io`, permission-limited rows show "—").
  - Network: Process Name, Sent Bytes, Rcvd Bytes, Sent Packets, Rcvd Packets, PID, User — only if a per-process source exists (eBPF/nethogs-like needs privileges → **omit columns**, show totals panel only).
- **Bottom panel (per tab), 120 high, three columns:**
  - CPU: System %, User %, Idle % with colored labels; "CPU Load" live graph (red system, blue user) 60 s; Threads, Processes counts.
  - Memory: "Memory Pressure" graph (green/yellow/red from PSI `/proc/pressure/memory`), Physical Memory, Memory Used (App Memory, Wired → "Kernel", Compressed → "zram" if present), Cached Files, Swap Used.
  - Disk: Reads in, Writes out, Data read, Data written, graph with IO/data toggle.
  - Network: Packets in/out, Data received/sent, graph packets/data toggle.
- **View menu:** Columns ›, Dock Icon › (Show Application Icon / Show Network Usage / Show Disk Activity / Show Memory Usage / Show CPU Usage / Show CPU History — Dock icon live graph requires Dock badge image API; *may defer*), Show CPU History ⌘3 (floating window with per-core bars), Show CPU Usage ⌘2 (floating), Show GPU History omit, All Processes / All Processes, Hierarchically / My Processes / System Processes / Other User Processes / Active Processes / Inactive Processes / Windowed Processes / Selected Processes / Applications in last 12 hours omit, Filter Processes ⌥⌘F, Inspect Process ⌘I, Quit Process ⌥⌘Q, Send Signal to Process…, Update Frequency › Very often (1 sec) / Often (2 sec) / Normally (5 sec, default) / Less often … , Clear CPU History ⌘K, Enter Full Screen.
- **Inspector window (⌘I):** header name + PID + parent process link + user + % CPU + Recent hangs omit; tabs Memory (Real/Virtual memory size, Shared memory), Statistics (Threads, Faults, Context switches), Open Files and Ports (from `/proc/<pid>/fd`, permission-limited). Buttons: Sample omit, Quit.
- **Hierarchical view:** disclosure triangles by parent PID.
- **Exit checks:** numbers match `top`/`free`/`iotop` within sampling tolerance; quit and force quit a user process; permission-denied process shows correct alert "You don't have permission to quit “X”." with Authenticate… via polkit `pkexec kill` *(may defer)*; idle CPU of System Monitor itself < 1% at Normally frequency; sampling stops when window hidden/minimized.

- [~] **7.8 Wording and button-order pass (FEEL_SPEC.md §D.10).** Added `scripts/check-wording.py`, which scans product Rust for the strings the spec forbids (`Error:`, `Failed to`, `Warning:`, and user text containing `/home/` or `dbus`), stripping inline `#[cfg(test)]` modules and ignoring D-Bus protocol constants/Debug shapes. Fixed the first real user-facing hits: Text Editor's alert titles now read **"The file could not be opened." / "The file could not be saved."** instead of "Failed to …" (`cargo test -p rmac-text-editor` ✔ 37). The rules are documented in `CONTRIBUTING.md`. The scanner now reports **0** and is wired into CI (`python3 scripts/check-wording.py --fail`, next to the token gate) — the `dbus` pattern is case-sensitive lowercase so D-Bus object paths (`/org/freedesktop/DBus`) are not treated as user text. **Remaining:** expand the check to button order, curly quotes, and the error shape (what/why/what-to-do) across the remaining apps; the current gate covers the forbidden substrings only.

**Phase 7 exit checklist**
- [ ] Every app passes its exit checks and the common app contract.
- [ ] Screenshot pairs for each app's main window light/dark, one sheet, one menu.
- [ ] `docs/user-guide.md` sections updated to match the built behavior exactly.

---

## 11. Phase 8: accessibility, performance, resilience, security

**Goal:** the product is usable by everyone, fast on the reference PC, and survives failure.
**Read first:** `docs/accessibility.md`, `docs/accessibility-release-audit.md`, `docs/performance-baseline.md`, `docs/performance-release-audit.md`, `docs/chaos-soak.md`, `docs/security-release-review.md`, `scripts/performance-budgets.json`, `scripts/accessibility-audit.json`, `scripts/chaos-soak.json`.

- [ ] **8.1 AT-SPI for every shell surface.** Using the upstream GPUI accessibility path proven by `shell/probes/a11y`: menu bar (`menubar`/`menu`/`menuitem`), Dock (`toolbar` of `button`s with names "Terminal, running"), Spotlight (`dialog` + `combobox` + `listbox`), Control Center (`dialog` with `switch`/`slider`), NC (`list` of `article`), banners (`alert` live region), switcher (`listbox`). Extend the three `assert_*_accessibility.py` scripts and add ones for the new bins.
- [ ] **8.2 Apps on stable GPUI.** Where 0.2.2 cannot expose AT-SPI, record each gap in `docs/known-limitations.md` and in the ADR from 0.1 as the Phase 10.3 migration driver. Do not claim Orca support for those apps.
- [ ] **8.3 Orca journeys.** With Orca on: log in, open Spotlight, launch Terminal, open Control Center and toggle Wi-Fi, read a notification, open Files and rename a file, lock and unlock. Record pass/fail per step in `docs/accessibility-release-audit.md`.
- [ ] **8.4 Keyboard-only journeys.** Same journeys with no pointer (Full Keyboard Access on).
- [ ] **8.5 Scale and preferences matrix.** 100%, 125%, 150%, 200% × light/dark × Increase Contrast × Reduce Transparency × Reduce Motion: screenshot every shell surface + every app main window; no clipped text, no overlaps, no blur where reduced transparency is on, no motion where reduced motion is on.
- [~] **8.6 Performance harness.** (FEEL_SPEC.md §D.12 idle-silence audit done: every `Duration::from_secs(1)` in the shell/runtime crates is a reconnect/retry backoff after an authority failure (`rmac-dock-runtime`, `rmac-audio`, `rmac-bluetooth`, portal, focus, gtk-settings, locale), not an idle poll; the menu-bar clock is minute-aligned and there is no per-second redraw timer. The latency/idle **budgets still need measurement**.) Extend `scripts/measure-baseline.py` to Linux with: session-ready time (GDM auth → all shell units READY), Spotlight shortcut→first frame, Control Center click→first frame, menu open latency, Dock hover→tooltip frame, app cold/warm launch, idle CPU and wakeups per shell process over 10 min (`pidstat -w`), RSS after 8 h. Budgets (from `PLAN_V2.md` §6.4 plus these): session ready ≤ 3 s; overlay open ≤ 80–100 ms p95; interaction ≤ 50 ms p95; 60 Hz animations ≥ 99% frames ≤ 16.67 ms; each shell process idle CPU ≤ 0.1%, wakeups ≤ 1/s; total shell RSS ≤ 350 MB (S, revise only with evidence).
- [ ] **8.7 Resilience drills** (`scripts/chaos-soak.json`): kill each shell unit, restart NetworkManager/BlueZ/PipeWire/UPower, corrupt `shell-settings` JSON (falls back to last-known-good with a notification), fill disk to 1 GiB free (writes fail gracefully with alerts, no data loss), suspend/resume ×20, hotplug external display ×20, 8-hour soak with synthetic notifications and app launches. All pass or have filed defects fixed.
- [ ] **8.8 Security review.** Update `docs/security-release-review.md`: polkit actions used, lock provider PAM path, secret agent, D-Bus interfaces exposed by rmac (AppMenu2, invocation sockets) with sender checks, log redaction test (grep journal for a test Wi-Fi password, note body, notification text → must be absent), `cargo deny check` clean in both workspaces.

**Phase 8 exit checklist**
- [ ] Accessibility audit, performance audit, chaos/soak, and security review each name the exact commit and hardware and pass.

---

## 12. Phase 9: install, update, remove, release

**Read first:** `docs/install.md`, `docs/native-packaging.md`, `docs/ubuntu-session-packaging.md`, `docs/update-trust.md`, `docs/keyring-packaging.md`, `docs/update-and-remove.md`, `docs/alpha-contributor-build.md`, `docs/beta-cohort.md`, `docs/one-dot-zero-candidate.md`, `packaging/apt/*`.

- [ ] **9.1 Packages.** `rmac-session`, `rmac-shell` (all `shell/bins`), `rmac-apps` (seven apps), `rmac-keyring`; amd64 first, arm64 before 1.0. Depends include niri (≥ documented version), `fonts-inter`, `fonts-jetbrains-mono`, `xdg-desktop-portal`, `xdg-desktop-portal-gnome` (file chooser) or `-gtk`, `pipewire`, `wireplumber`, `network-manager`, `bluez`, `upower`, `power-profiles-daemon`, `accountsservice`, `cups` (Recommends), `orca` (Recommends).
- [ ] **9.2 Signed APT repository** with the existing trust/keyring design; publish script dry-run then real publish when the owner provides the signing key (**OWNER**: key custody).
- [ ] **9.3 Lifecycle proof on a clean Ubuntu 26.04 VM and the reference PC:** install → GDM shows "rmac" → login → complete desktop; reinstall same version; upgrade to next candidate while logged into GNOME and while logged into rmac (deferred activation, no mixed binaries); interrupt upgrade (kill apt) → recover with `apt -f install`; rollback to previous version; remove (user data kept); purge (all rmac system files gone, user data untouched unless opted); GNOME logs in at every step.
- [ ] **9.4 First-login experience.** First rmac login shows a one-time **Welcome** window (560×460): page 1 "Welcome to rmac" + Appearance choice (Light/Dark/Auto tiles); page 2 Dock and Search tips with keyboard shortcuts (⌘Space, ⌘Tab, ⌃↑); page 3 Privacy: what stays local, no telemetry; "Get Started". Never shown again (flag in shell settings).
- [ ] **9.5 Public release materials.** README rewrite: one-sentence value proposition, 3 screenshots (desktop, Spotlight+Control Center, Files), 2–5 min demo video link, honest status, install guide link, hardware support table, known limitations, license, contributing, security, sponsor links (**OWNER**: sponsor accounts). No Apple logos, no "macOS clone" wording implying affiliation: use "macOS-inspired".
- [ ] **9.6 Demo video script** (record on reference PC, 1080p60): boot → GDM → rmac login (5 s) · menu bar + Dock tour (15 s) · ⌘Space "terminal" launch (10 s) · Files: icon/list/column views, Quick Look, copy with progress (40 s) · Control Center Wi-Fi/Bluetooth/volume with OSD (20 s) · notification banner → NC (15 s) · System Settings Appearance dark mode switch live (15 s) · Mission Control + Spaces (15 s) · Notes create checklist (15 s) · lock/unlock (10 s) · closing card: "Open source, MIT, runs on Ubuntu 26.04" (5 s).

**12.3 Evidence protocol (used by every visual task).**
- Capture on Ubuntu with `grim -o <output> target/evidence/<phase>/<task>/<surface>-<light|dark>-<scale>.png`.
- Capture on Mac with `screencapture -x` (window: `-l <windowid>`), same logical size.
- Compose side-by-side with `python3 scripts/compose-evidence.py a.png b.png out.png` (create the script in task 1.1 if missing: Pillow, horizontal concat, labels "rmac @ <commit>" / "macOS <version>").
- In the ticked checkbox line write: `evidence: target/evidence/<path> (not committed)` and one sentence of remaining differences, or "no visible difference".

**Phase 9 exit checklist**
- [ ] Every item of `GOAL.md` Definition of Done D2, D3, D11, D13 has evidence at one candidate version.

---

## 13. Phase 10 (after 1.0, optional) and the master shortcut table

### Phase 10 — optional

- [ ] 10.1 Calculator (basic/scientific/programmer, history tape, Spotlight provider reuse).
- [ ] 10.2 Preview (images + PDF via poppler, markup, rotate, export) and Quick Look sharing.
- [ ] 10.3 Migrate apps to the shell's GPUI rev once `gpui-component` (or an rmac replacement) supports it; remove the stable 0.2.2 line; unify `rmac-ui` and `rmac-shell-ui`.
- [ ] 10.4 Opt-in global ⌘ remapping for third-party apps via `keyd` profile (Super+C→Ctrl+C in non-terminal apps), off by default, with per-app exclusions.
- [ ] 10.5 Clock app with world clock/alarms/timers (+ Control Center Timer control).
- [ ] 10.6 Desktop icons layer (if deferred in 7.1).
- [ ] 10.7 Genie minimize effect (needs compositor shader support in niri).

### Master shortcut table (global; "⌘" = Super; implement exactly)

| Shortcut | Action | Owner |
|---|---|---|
| ⌘Space | Toggle Search | shortcut broker → `rmac-spotlight` |
| ⌥⌘Space | New Files window with search focused | Files |
| ⌘Tab / ⇧⌘Tab | App switcher | `rmac-switcher` |
| ⌘` | Next window of current app | niri recent-windows filter app-id |
| ⌃↑ / F3 | Mission Control | niri overview |
| ⌃↓ | App Exposé (fallback: switcher window list) | shell |
| ⌃← / ⌃→ | Previous/next Space | niri |
| ⌃1…⌃9 | Go to Space N | niri |
| F4 | Apps | `rmac-apps` |
| F11 (fn F11) | Show Desktop | shell `ShowDesktop` |
| ⌥⌘D | Toggle Dock autohide | Dock |
| ⌃F2 | Focus menu bar | menubar |
| ⌃F3 | Focus Dock | Dock |
| ⌃⌘Q | Lock Screen | lock provider |
| ⇧⌘Q | Log Out… | session dialog |
| ⌥⇧⌘Q | Log Out immediately | session |
| ⌥⌘⎋ | Force Quit Applications | `rmac-forcequit` |
| ⇧⌘3 / ⇧⌘4 / ⇧⌘5 | Screenshot screen / selection / toolbar | screenshot overlay |
| ⌃⇧⌘3 / ⌃⇧⌘4 | Screenshot to clipboard | screenshot overlay |
| ⌃⌘F | Toggle full screen (focused window) | niri (replace existing `Mod+Ctrl+F` bind, same keys) |
| ⌃⌥← / → / ↑ / ↓ | Tile left/right/top/bottom | shell tiling (Tahoe uses fn⌃; PC keyboards lack reliable fn) |
| ⌃⌥F | Fill | shell tiling |
| ⌃⌥C | Center | shell tiling |
| ⌃⌥R | Return to previous size | shell tiling |
| ⌃⌘Space | Emoji & Symbols picker | shell (*may defer*: IBus emoji) |
| Volume/Brightness/Mute keys | OSD + change | `rmac-osd` (exists) |
| ⌥⇧ + volume/brightness key | Fine (quarter) step | `rmac-osd` |
| ⌥ + volume key | Open Sound settings | `rmac-osd` |
| ⌥ + brightness key | Open Displays settings | `rmac-osd` |
| Power button (short) | Sleep (setting) | logind config via package |

Per-app shortcuts are listed in §10 per app. Standard app-level shortcuts for all rmac apps: ⌘N ⌘O ⌘S ⇧⌘S ⌘W ⌥⌘W ⌘Q ⌘H ⌥⌘H ⌘M ⌘, ⌘Z ⇧⌘Z ⌘X ⌘C ⌘V ⌥⇧⌘V ⌘A ⌘F ⌘G ⇧⌘G ⌘E ⌘J ⌘P ⇧⌘P ⌘T ⇧⌘] ⇧⌘[ ⌘+ ⌘− ⌘0 ⌃⌘F ⌘? (Help search).

Update `crates/rmac-shortcuts/src/model.rs` (currently only launcher + lock), `packaging/rmac-session/shortcuts-fallback.kdl`, `packaging/rmac-session/shell.kdl`, `docs/shortcuts.md`, and Settings ▸ Keyboard ▸ Keyboard Shortcuts to this table in one task inside Phase 5 (task **5.9**, add it to the Phase 5 list when you start Phase 5):

- [ ] **5.9 Global shortcut table implemented** exactly as above, conflicts detected in Settings, `docs/shortcuts.md` regenerated from `rmac-shortcuts` model (write a small generator test that fails if the doc and model differ).

---

## 14. Definition of Done

rmac is complete when **all** boxes below are checked against one versioned candidate (these are `GOAL.md` D1–D13 made concrete):

- [ ] **D1 Framework:** ADR 0006 merged; `shell/` and apps build at pinned revisions; known gaps listed.
- [ ] **D2 Install:** clean Ubuntu 26.04 installs from the signed repo following `docs/install.md` only.
- [ ] **D3 First login:** GDM → rmac → full desktop, Welcome shown once, no terminal step, no duplicate components.
- [ ] **D4 Daily journeys:** Phase 7 exit checks for all six apps pass on the reference PC.
- [ ] **D5 Shell parity:** Phases 2–6 exit checklists pass with evidence pairs.
- [ ] **D6 Settings:** every visible Settings control authoritative, persistent, failure-aware, reversible where risky; omitted items documented in `docs/system-settings-audit.md`.
- [ ] **D7 Accessibility:** Phase 8.1–8.5 pass (with documented app gaps from 8.2 narrowed in public claims).
- [ ] **D8 Performance:** Phase 8.6 budgets met on named hardware.
- [ ] **D9 Resilience:** Phase 8.7 drills pass.
- [ ] **D10 Security & provenance:** Phase 8.8 pass; asset provenance file lists every icon/wallpaper/sound/font with license; no Apple assets.
- [ ] **D11 Lifecycle:** Phase 9.3 pass.
- [ ] **D12 Hardware:** `docs/hardware-support.md` lists exactly the machines with evidence; claims narrowed to match.
- [ ] **D13 Public release:** Phase 9.5–9.6 materials match the candidate build.

---

## 15. Quick index: where things live

| Need | Path |
|---|---|
| Tokens (after Phase 0) | `crates/rmac-design/src/lib.rs` |
| App UI components | `crates/rmac-ui/src/{controls,components,feedback,chrome,window}.rs` |
| Shell UI components | `shell/crates/rmac-shell-ui/` |
| Menu bar host | `shell/bins/rmac-menubar/` (was `experiments/gpui-upstream-lab/src/bin/top_bar.rs`) |
| Menu bar model/labels/a11y | `crates/rmac-top-bar/`, `crates/rmac-top-bar-runtime/` |
| App menu export | `crates/rmac-app-menu/` |
| Dock host / model / runtime / system | `shell/bins/rmac-dock/`, `crates/rmac-dock*/` |
| Shell settings schema + migration | `crates/rmac-shell-settings/src/{model,migration}.rs` |
| Compositor actions (niri) | `crates/rmac-compositor/src/actions.rs`, `crates/rmac-compositor-niri/src/` |
| niri session policy | `packaging/rmac-session/shell.kdl`, `config.kdl`, `shortcuts-fallback.kdl` |
| Session supervisor | `crates/rmac-session/`, `docs/session-supervisor.md`, `scripts/linux/install-session-units.sh` |
| Spotlight runtime/providers | `crates/rmac-launcher*/`, `crates/rmac-search/` |
| Control Center model/mutations | `crates/rmac-quick-settings*/` |
| Notifications authority / banners / store | `crates/rmac-notifications*/` |
| OSD | `crates/rmac-osd/`, `shell/bins/rmac-osd/` |
| Lock screen | `crates/rmac-lock-provider-linux/` |
| Wallpaper | `crates/rmac-wallpaper*/`, `shell/bins/rmac-wallpaper/` |
| Network / Bluetooth / Audio / Power / Display | `crates/rmac-network/`, `rmac-bluetooth/`, `rmac-audio/`, `rmac-power/`, `rmac-display/` |
| Files app | `crates/finder/` |
| Settings app | `crates/system-settings/` |
| Terminal / Notes / Text Editor / System Monitor | `crates/terminal/`, `crates/notes/` + `crates/rmac-notes-*`, `crates/text-editor/`, `crates/activity-monitor/` |
| Packaging | `packaging/`, `scripts/linux/build-native-packages.py`, `scripts/linux/native_package_contract.py` |
| Release gates | `scripts/run-release-contract-checks.py`, `scripts/*-candidate.json`, `scripts/linux/run-reference-gates.sh` |
| CI | `.github/workflows/ci.yml` |

---

*End of spec. Start at §0.1 step 1.*
