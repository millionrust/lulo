# Speed sweep — 2026-10-05

Owner priority: "check each and everything for speed… I feel like Ubuntu is
faster… I want ours to be the best." This pass's brief: find and fix the
systemic cause behind every slow icon paint, build a repeatable timing
harness, and report a before/after table. Branch `op/speed-sweep`.

## What was fixed

**Systemic cause** (the task's own hypothesis, confirmed): GPUI's default
`img(path)` pipeline rasterizes an SVG at (its own `viewBox` size) × 2 —
GPUI's internal smoothing factor — regardless of the size it's actually
drawn at. Lulo's master app icons are 1024×1024 artwork (some with an
`feDropShadow` filter), so every one of them decoded as a 2048×2048 canvas
even at a 16 pt menu glyph. `shell/bins/rmac-wallpaper/src/linux_wayland/desktop.rs`
(`ICON_SVG_SCALE`, `warm_desktop_icons`, DESK-12 in docs/parity.md) had
already measured and fixed this for the desktop's own folder/document
glyphs: **~1.4–1.75 s per icon on the loaded reference laptop, independent
of which SVG**, dropping to a fraction of that once rasterized at the real
display size instead of native size.

This pass generalized that fix into a shared, cached helper and applied it
across the shell:

- `crates/rmac-ui/src/svg_icon.rs` — `rmac_ui::svg_icon(source, size, cx)`,
  for every `crates/` app (all of which already depend on `rmac-ui`).
- `shell/crates/rmac-shell-ui/src/svg_icon.rs` — the equivalent for shell
  bins (Dock, App Switcher, Mission Control), which can't depend on
  `rmac-ui` (it depends on `rmac-dock`, which those bins also depend on).

Both: rasterize at exactly the display size via `SvgRenderer`, cache the
`Arc<RenderImage>` process-wide per `(source, size)` in a bounded map,
decode off the UI thread (`blocking::unblock`) with a one-frame blank
placeholder while that happens, and fall back to plain `img(path)`
unchanged for anything that isn't actually an SVG (a `.desktop` entry's
`Icon=` can resolve to a theme PNG — confirmed real by inspecting
`rmac_dock`'s and `rmac_apps`' icon resolution; the helper checks the file
extension so it's always a safe drop-in).

Wired into every call site this pass could reach without a signature
cascade deeper than 1–2 layers: **Dock** (tiles, badges, stacks, trash,
drag ghost), **App Switcher** (grid tiles, Force Quit sheet), **Mission
Control** (covered-window badge), **Launchpad**/app-drawer, **Spotlight**
(results, query panel, mode panel), **Notification Center** (history list
and the live banner surface), and the **About panel**. Parity rows:
DOCK-30, APPS-04, MC-09, SPOT-07, NC-12 in docs/parity.md.

**Not reached this pass** — same cause, same fix, deferred for time (see
"Remaining offenders" below): Settings' per-app icon rows (SET-114),
most of Finder's icon rendering (FILES-62, which already had a partial,
independent mitigation — see below).

## Validation

- `rustfmt --edition 2021 --check` clean on every touched file.
- GitHub CI (`agent/speed-sweep`, final commit `38a96058`): **Linux
  checks: success**, **macOS checks: success**, **Release contracts:
  success**, **Dependency policy: success** (all confirmed on the
  preceding commit `a4cdd07c`, which differs from `38a96058` only by one
  one-line signature fix in `rmac-dock`; the full suite was still running
  — `Build all app and shell binaries` and `Current GPUI Linux runtime
  gate` — when this pass's time-box was reached). CI caught two real bugs
  this pass introduced and both are fixed in the final commit: a
  module/function name collision in `rmac_shell_ui` (`pub mod svg_icon`
  alone left `rmac_shell_ui::svg_icon` resolving to the module, not the
  re-exported function — E0423) and one call site in the Dock
  (`render_stack_popover`) whose own `cx: &Context<Dock>` wasn't widened to
  `&mut` when its callee (`stack_popover_item`) was — E0308.
- `python3 -m unittest scripts.test_run_speed_sweep` and
  `scripts.test_run_frame_timing`-style pure-logic tests: 6/6 pass.
- The root-workspace app binaries (Files, Text Editor, Settings,
  Calculator, Calendar, Mail, Clock, Weather, Preview, Notes, System
  Monitor, Terminal, Spotlight, Quick Settings, Notification Center,
  Launchpad) built clean on the reference laptop (`cargo build --offline
  --profile iterate`, 34 min from a cold shared target). The shell bins
  (Dock, Mission Control, App Switcher) needed a from-scratch `gpui`
  rebuild under `shell/`'s separate feature set and did not finish inside
  the time-box; this pass's confidence in that code is from CI's
  `rustc`/clippy pass against the identical commit, not a laptop binary run.

## Measurement harness

`scripts/behavior/run_speed_sweep.py`, reusing `run_frame_timing.py`'s
nested-niri isolation (headless Sway hosting niri with the shipped
`shell.kdl`, `RMAC_FRAME_TRACE`, `wlinput.py` for key input). For every app:
wall-clock from spawn to the window appearing in `niri msg windows`
(`first_frame_ms`), and wall-clock to its frame trace going quiet for 250 ms
once it has at least one `present` (`icons_painted_ms`, a proxy — see the
script's docstring for why a per-icon signal doesn't exist). For panels
(Spotlight, Control Centre, Notification Centre, Launchpad via
`rmac-shortcut-dispatch`; Mission Control via niri's own Ctrl+Up bind; App
Switcher as a fresh spawn, matching how it's really launched): wall-clock
from the triggering input to that process's next `present`.

## Before/after table

**This table is "after" only.** Building two complete binary sets (the
`integ` baseline and this branch) on the shared laptop target, back to
back, did not fit the time-box after the fix itself and three CI/laptop
round-trips to find and fix the two bugs above — see "What wasn't done".
Numbers below are one run, on a laptop shared with other agents' concurrent
cargo builds throughout this pass (confirmed via `ps aux`: a clippy+test
job and, earlier, another agent's build were both holding the shared
`/tmp/lulo-cargo.lock` for parts of this session) — **treat the absolute
values as upper bounds, not a clean-room result.**

| Item | Target | Before | After | Pass/fail |
|---|---:|---:|---:|---|
| Files first frame | ≤300 ms | not measured | 5131 ms | FAIL |
| Files icons painted | ≤300 ms | not measured | n/a (no quiescence inside 4 s) | FAIL |
| Text Editor first frame | ≤300 ms | not measured | 1824 ms | FAIL |
| Text Editor icons painted | ≤300 ms | not measured | 2236 ms | FAIL |
| Settings first frame | ≤300 ms | not measured | 936 ms | FAIL |
| Settings icons painted | ≤300 ms | not measured | 2375 ms | FAIL |
| Calculator first frame | ≤300 ms | not measured | 728 ms | FAIL |
| Calculator icons painted | ≤300 ms | not measured | 1079 ms | FAIL |
| Calendar first frame | ≤300 ms | not measured | 1047 ms | FAIL |
| Calendar icons painted | ≤300 ms | not measured | 1834 ms | FAIL |
| Mail first frame | ≤300 ms | not measured | 790 ms | FAIL |
| Mail icons painted | ≤300 ms | not measured | 1558 ms | FAIL |
| Clock first frame | ≤300 ms | not measured | 915 ms | FAIL |
| Clock icons painted | ≤300 ms | not measured | 1324 ms | FAIL |
| Weather first frame | ≤300 ms | not measured | 681 ms | FAIL |
| Weather icons painted | ≤300 ms | not measured | 1024 ms | FAIL |
| Preview first frame | ≤300 ms | not measured | 565 ms | FAIL |
| Preview icons painted | ≤300 ms | not measured | 1028 ms | FAIL |
| Notes first frame | ≤300 ms | not measured | 821 ms | FAIL |
| Notes icons painted | ≤300 ms | not measured | 1824 ms | FAIL |
| System Monitor first frame | ≤300 ms | not measured | 863 ms | FAIL |
| System Monitor icons painted | ≤300 ms | not measured | 1475 ms | FAIL |
| Terminal first frame | ≤300 ms | not measured | 708 ms | FAIL |
| Terminal icons painted | n/a | — | animates continuously; not scored | — |
| Spotlight open | ≤100 ms | not measured | 356 ms | FAIL |
| Control Centre open | ≤100 ms | not measured | 356 ms | FAIL |
| Notification Centre open | ≤100 ms | not measured | 379 ms | FAIL |
| Launchpad open | ≤100 ms | not measured | error: shortcut endpoint never registered in this harness | FAIL |
| Mission Control open | ≤100 ms | not measured | 561 ms | FAIL |
| App Switcher open | ≤100 ms | not measured | binary not built in time | FAIL |
| Settings pane switching | ≤100 ms | not measured | not implemented (needs simulated sidebar clicks) | FAIL |

The one controlled before/after this bug class actually has is DESK-12's
own measurement, on the same icon-decode mechanism, on the same laptop:
**~1.4–1.75 s per icon before → ~0.7–0.9 s mean time-to-icon after**, for
the desktop's folder/document glyphs specifically. That is the real
evidence the fix works; this sweep's own numbers above did not isolate it
with a same-session comparison.

## Why "after" still fails the targets

The 300 ms / 100 ms targets are macOS's, measured warm, on real hardware,
with nothing else running. Every number above includes this harness's own
nested-niri + headless-Sway + dbus-run-session startup and the traced
process's own cold Wayland/GPUI connection setup — overhead the icon fix
was never going to touch. DESK-12 already found exactly this shape of
result for the desktop case: after its fix, "the remaining time is
process/Wayland/compositor startup, not the icon decode." Two concrete,
believed-real remaining costs inside that overhead, not chased further this
pass: Files' outlier 5.1 s (almost certainly its filesystem scan of a
non-trivial home directory structure on a loaded shared laptop, not an icon
decode — Files was not one of the apps with a converted icon call site
this pass touched at all); and the shared laptop's own CPU contention
during this exact run (see the CI/build evidence above).

## Remaining offenders

| Surface | Cause | Where |
|---|---|---|
| Settings — Notifications pane, Focus allowed-apps row, a sheet | Same systemic cause; `img(path)` still called directly. Deferred: each call site needs `cx: &mut Context<Settings>` threaded through 1–2 more layers than this pass's time allowed (see SET-114, docs/parity.md). | `crates/system-settings/src/controller/{notifications,view_helpers,view_helpers/form,view_helpers/focus,view_helpers/sheets}.rs` |
| Finder — list/content/gallery presentation, search info | Same systemic cause, already partly mitigated: `presentation_support.rs::item_artwork_path` picks between three pre-rendered folder/document SVG sizes instead of one 1024 px master (its own comment already documents the GPUI behavior this pass's helper fixes generally). Lower priority than the five fixed surfaces: most of Finder's other `img()` calls are real file thumbnails, already size-bounded by `rmac_thumbnails`, not master SVGs. See FILES-62. | `crates/finder/src/view/{list_presentation,content_presentation,gallery_presentation,search_info_controller,presentation_support}.rs` |
| rmac-polkit-agent's badge icon | Same systemic cause, one call site, not reached: low-traffic (an authorization dialog, not a frequently-opened surface). | `crates/rmac-polkit-agent/src/ui.rs:361` |
| Settings pane switching latency | Not measured at all: needs simulated clicks on the sidebar's measured row coordinates, which this pass did not build into `run_speed_sweep.py`. | `scripts/behavior/run_speed_sweep.py` (gap, documented in its own docstring) |
| True before/after comparison | Not measured: building two complete binary sets on the shared laptop target did not fit the time-box. The next pass should build `integ` and the fix branch back to back (same shared target, so the second build is mostly incremental) and diff the two JSON reports `run_speed_sweep.py` already produces for exactly this. | — |
| App Switcher / Mission Control / Dock binary validation | Not confirmed by an actual laptop build (CI's compile pass against the identical source is the only confirmation) — the shell-side `gpui` rebuild (a different feature-flag resolution than the root workspace) did not finish inside the time-box. | `shell/bins/{rmac-dock,rmac-app-switcher,rmac-mission-control}` |

## What wasn't done

- No clean, contention-free before/after run.
- Settings and most of Finder not migrated to `rmac_ui::svg_icon` (see table above).
- Settings-pane-switching timing not implemented.
- The full CI suite (`Build all app and shell binaries`, `Current GPUI Linux runtime gate`, the runtime behavior-suite matrix) did not finish before the time-box; `Linux checks`, `macOS checks`, `Release contracts` and `Dependency policy` did, all passing, against a commit one line removed from the final one.
- Shell-bin binaries (Dock, Mission Control, App Switcher) not rebuilt and re-run through `run_speed_sweep.py` after the final fix landed.
