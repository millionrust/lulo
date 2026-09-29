# Beta release checklist — 0.9.0-beta.1

## Beta 1 go/no-go (updated 2026-09-27)

**No-go under the current release gates.**
[Main CI run 36312045023](https://github.com/millionrust/lulo/actions/runs/36312045023)
and [quality CI run 36312045008](https://github.com/millionrust/lulo/actions/runs/36312045008)
succeeded on source `0e3fa470`. An amd64 package set built from that exact
commit was installed on the reference PC on 2026-09-27. The 27-scenario nested
behavior rerun on installed binaries matched 24 Mac recordings; its three mismatches are the Files item menu
(Share/tags/Quick Actions), Get Info window behavior, and the new background
Get Info scenario (remaining menu rows plus that same window behavior). The new
Preview PDF Find scenario passes in the full release-binary rerun.
CI success does not establish release readiness:
these product, accessibility, security, and packaging gates remain open.

The reference PC now has the `0e3fa470` `rmac-apps` and `rmac-session`
`0.9.0~beta.1-38` package set. The installed Files, Preview, and top-bar
binary hashes match their staged package contents. The installed binaries
passed 28/28 nested shutdown checks and 41/41 power-dialog checks with fake
`systemctl`. On the live installed session, AT-SPI opened the Lulo menu,
activated Shut Down…, found its confirmation, and clicked Cancel; the dialog
closed without a power request. Actual pointer activation and poweroff remain
unverified. The repeatable
[`probe_live_shutdown_cancel.py`](../scripts/linux/probe_live_shutdown_cancel.py)
also passed on the active session: it opened the same dialog, activated only
Cancel, verified closure, and closed the menu. The isolated startup smoke passed
9/9 apps against the installed binaries, proving startup readiness only. A
private Terminal typed-command roundtrip also passed: the marker appeared in
both the echoed command and its executed output. The
sequential run and binary hashes are in `/tmp/lulo-installed-report-20260927`
on the reference PC; `scripts/behavior/run_installed_suite.py` reproduces it.

The `dev` source now has separate non-modal Files Get Info windows and a
persistent seven-colour item-menu tag control. Focused Files tests and a
private Get Info scenario passed. A private UI run against the `8e31d9dc`
release binary also selected Blue, observed the `user.rmac.tag=blue` attribute
and checked menu row, then cleared it by clicking the row again. These commits
are newer than the installed package set, and the installed 24/27 behavior
result remains the valid installed-binary result.
The staged candidate selected by `./install-lulo.sh` now contains these source
changes. It has not been installed on the reference PC.

The previous amd64 candidate with `rmac-apps` and `rmac-session` built from source
`e8b589ac` and the pinned niri/xwayland-satellite packages remains staged at
`~/rmac-release/packages-install10-e8b589ac` on the reference PC. Its four
Debian package SHA-256 checks and native package verifier passed.
[Main CI run 36327602027](https://github.com/millionrust/lulo/actions/runs/36327602027)
and [quality run 36327602104](https://github.com/millionrust/lulo/actions/runs/36327602104)
passed for the exact source. The candidate is staged but **not installed**;
the installed-build results above still refer to `0e3fa470`. The exact
`e8b589ac` release binaries passed a private
[27-scenario rerun](behavior-results/source-e8b589ac-27.json) at 25/27:
Get Info now matches its Mac behavior; the item and background context menus
remain mismatches. No scenario that passed on the installed 24/27 run
regressed. The candidate also passed
[9/9 private startup checks](behavior-results/source-e8b589ac-startup-smoke.json),
including App Drawer and Preview through their proper launch paths. Its top bar
passed
45/45 scoped unit tests and 37/37 private menu/power-dialog checks. The latter
includes a regression that focuses Notes then Files without a Files menu
publisher: the installed top bar lacked File/Edit/View/Go, while the candidate
shows all four.

The current candidate is the 38-binary amd64 set built from exact source
`8ba31b82`, with the same pinned niri/xwayland-satellite packages, at
`~/rmac-release/packages-install11-8ba31b82`. Its four SHA-256 checks and
native package verifier pass; `~/install-lulo.sh` and its installer checkout
are pinned to this set. It is **not installed**: the active reference session
still runs `0e3fa470`. [Main CI](https://github.com/millionrust/lulo/actions/runs/36334082582)
and [quality CI](https://github.com/millionrust/lulo/actions/runs/36334082589)
passed for `8ba31b82`. The exact binaries pass
[9/9 startup checks](behavior-results/source-8ba31b82-startup-smoke.json),
[25/27 Mac behavior scenarios](behavior-results/source-8ba31b82-27.json),
37/37 private power-dialog checks, and 28/28 private shutdown checks with a
fake `systemctl`. Focused private Files checks also pass for View and Sort By
flyouts, checked sorting, all seven visible tag swatches, and applying and
removing a tag xattr. The two behavior mismatches remain the Files item and
background menus: Share, Quick Actions, grouping, View Options, and iPhone
import lack their underlying services or presentation model. Text Editor's
stale parent redraw observer was removed, but one paired 30-second private
[probe](perf/reference-laptop-2026-09-27-text-editor-parent-invalidation.md)
changed idle CPU from 40.5% to 39.1%, an inconclusive difference; the idle
performance gate is still open. A sequential
[eleven-app candidate sample](perf/reference-laptop-2026-09-28-candidate-8ba31b82.json)
against the staged binaries finds five apps above the 0.3% idle CPU target in
private software-rendered Sway: Text Editor 40.53%, Weather 39.93%, Clock
32.43%, System Monitor 19.63%, and Files 0.80%. Each is one 30-second idle
window, not an installed-desktop percentile; the result confirms the gate
needs further work and an installed rerun. The later 2026-09-29
[release-binary candidate](perf/idle-cpu-2026-09-29.md) passes the corrected
five-app idle budgets in the live session, but has not been packaged.
Neither fake power checks nor private UI
checks prove real host poweroff or full visual parity.

Visual parity is not established by the behavior or startup passes. One local
Mac/Lulo screenshot-pair record under `target/evidence/visual-comparisons/`
documents the installed build's missing Files menus; its originals differ in
scale, wallpaper and desktop state. It passes the provenance audit but does
not establish overall desktop parity. The
[comparison protocol](visual-comparison.md) and
`scripts/audit-visual-comparisons.py` can validate future pairs and their
provenance, but a human must still review their visible differences.

| # | Blocker | Owner/agent | Size |
|---|---|---|---|
| 1 | Journey 5 Save-panel behavior passes in the nested run with `rmac-file-chooser` present. The earlier failure was a cold-start timing issue: the scenario observed after 1.5 s and sent Escape before the D-Bus-activated chooser's AT-SPI window was ready. It now waits 4 s before observing. `run_lulo.py` also fails preflight when the helper is missing. The chooser service is now deployed, but the live portal journey has not been retested. | agent | M |
| 2 | Accessibility remains a release gate: AT-SPI `EditableText` is absent upstream; ACC-08 and role/name/action gaps remain; Control-F2 is nested-confirmed but not Orca-confirmed; and no owner-run Orca/I3 audit exists. See journeys 1–5 and 8–9 below. | agent / owner / upstream | L |
| 3 | The amd64 package set from `0e3fa470` was installed on the reference PC using `~/install-lulo.sh`. The native pair and pinned compositor packages pass `verify-native-packages.py`, all four SHA-256 checks pass, and installed binaries pass 9/9 startup, 41/41 power-dialog, and 28/28 nested shutdown checks; 24/27 selected behaviors match the Mac recordings. The GNOME-only installer succeeded on the reference PC; a clean Ubuntu 26.04 install/upgrade/uninstall run and GitHub Actions Release workflow remain unexercised. Green dev CI does not exercise that release pipeline. | agent / owner VM | L |
| 4 | Security gate remains **Fail**. Three accepted Low findings remain (SR-15, SR-18, SR-29), and 24 of 80 checks need native-station evidence; no Beta station has run. The verifier must continue to fail closed until its evidence requirements are met. | agent / owner stations | L |
| 5 | Nine apps passed the earlier warm-launch budget. The five idle CPU offenders now pass in a live, sequential 60-second [release-binary candidate run](perf/idle-cpu-2026-09-29.md), with the corrected 2.5% System Monitor and 0.5% shell budgets. That candidate is not installed; the old package remains above idle limits. Frame pacing, input response, soak memory, and NVIDIA results remain unmeasured. | agent | M |
| 6 | Release-facing install/readiness language still needs the owner's decision before publication so README and install guidance match the early-access Beta scope. | **owner** | S |

### Proposed Beta 1 / Beta 2 split (2026-09-29, needs the owner's yes)

Beta 1 is an early-access build for a small cohort on the reference class of
hardware. These gates need hardware, time or a person that Beta 1 does not
have. The proposal is to report them honestly as **Not yet run** and make them
**Beta 2 gates**, not to waive them or mark them as passing:

| Gate | Why it moves | Beta 1 disclosure |
|---|---|---|
| Memory 8-hour soak | Needs an unattended overnight run on the reference PC | known-limitations.md: long-session memory is unmeasured |
| NVIDIA station repeat | No NVIDIA hardware in the station matrix | known-limitations.md: tested on Intel graphics only |
| Formal Orca/I3 accessibility audit (journey 9) | Only the owner may enable Orca; the AT-SPI `EditableText` gap is upstream | known-limitations.md: screen-reader support is incomplete |
| Security native-station evidence (24 of 80 checks) | Needs Beta stations that don't exist yet | The verifier keeps failing closed; the 56 checks that ran are reported |

Everything else in the table above stays a Beta 1 gate.

**What changed this pass, with real evidence:** journeys 2 (Files), 3
(Terminal) and 4 (Notes) — see the updated journey table below — went from
"no accessible content surface at all" to fully working live, both via the
repo's own `scripts/assert_{terminal,notes,files}_accessibility.py` acceptance
scripts (run live against a same-day `dev` build, `exit 0` on all three) and
a fresh nested-compositor run with real typed keystrokes (14/16 Files
behaviour scenarios pass, including two rename scenarios that were
completely blocked before). Idle CPU for the shell surfaces (Dock, top bar,
wallpaper, OSD, etc.) also measured near zero (0.0–0.6%, well under budget)
even while another build was saturating the laptop's CPU — a big
improvement over the 2026-09-24 numbers, though per-app idle CPU (System
Settings, Files, System Monitor, Clock) was not independently re-measured
this pass to avoid running extra app instances on a laptop shared with other
agents.

This is the release-blocking list for shipping **0.9.0-beta.1** as a GitHub
Release: `.deb` packages (`rmac-apps`, `rmac-session`, Lulo OS's `niri`
and `xwayland-satellite` builds with their source packages, and the keyring
package when a signing key exists) plus `SHA256SUMS`, an SBOM, build
provenance, and a manual install guide. The signed APT repository comes
after this Beta (it needs the owner's signing-key decision and a real Debian
source package; see [docs/release-process.md](release-process.md)).

Status key: **Pass**, **Fail**, **Not yet run**, **Waived — reason**. Per
`todo.md`'s own direction ("Cut scope, never gates"), data-safety,
accessibility, and security gates are never marked Waived here, even under
release pressure — only Pass, Fail, or Not yet run.

## Read this first: two different things are called "Beta"

This repository already defines a much larger "Beta" gate:
[docs/beta-cohort.md](beta-cohort.md) and `scripts/beta-candidate.json`
describe an **invited daily-driver cohort** — at least 20 participants for
14 days (280 participant-days), a 90%+ pass rate on 20 attempts of each of
the ten product journeys, three certified H8 hardware stations, all 15
promotion checks, and zero open defects across four safety classes (data
loss, privilege escalation, session lockout, critical accessibility). That
gate is nowhere close to met (see below) and this checklist does not claim
otherwise.

**0.9.0-beta.1, as scoped for this release, is a much smaller thing**: a
downloadable early-access build for people willing to run pre-release
software on a disposable machine, published so real users outside the
project can start reporting problems. Calling it "Beta" in the GitHub
Release matches the owner's naming decision, but it should not be read or
announced as satisfying `docs/beta-cohort.md`'s cohort gate — that gate, and
the H8 hardware matrix and 1.0-candidate gate above it, still apply before
any wider or "recommended" claim is made. Consider a subtitle on the
release itself (e.g. "0.9.0-beta.1 — early access, not the daily-driver
Beta cohort") if that distinction needs to survive outside this document.

## 1. Product journeys (todo.md)

| # | Journey | Script | Last result | Status |
|---|---|---|---|---|
| 1 | Log in, launch from Dock/Spotlight, switch, close | `scripts/linux/run-journey-launch.py` | Ran live on the reference laptop. Dock tiles and Spotlight rows were fixed to expose a real AT-SPI `click`/`Text` surface (see `docs/journey-suite.md`). Spotlight's search field still can't be **typed into** over AT-SPI: the pinned `accesskit_unix` bridge has no `EditableText` implementation at all (upstream gap, not fixable here). | **Fail** — one real, live, unfixable-in-repo accessibility gap (no-keyboard-injector query typing); everything else in journey 1 passes |
| 2 | Find, preview, copy, move, rename, trash a file; undo | `scripts/linux/run-journey-files.py`, `scripts/assert_files_accessibility.py`, `scripts/behavior/run_lulo.py files/*` | **Re-run 2026-09-25** against a fresh `dev` build (`3ca78f24` + this pass's fixes, built package-scoped on the laptop). `scripts/linux/run-content-accessibility.sh ~/rmac-wt/target/iterate files` — live session, one instance, temp XDG dirs, no input injection — exits 0: "3 items with kind descriptions and click/selection, 8 sidebar places, click selects, Rename opens a focused Name entry (selection [(0, 5)]), setCaretOffset moves the caret, refocusing the row ends the rename." Separately, the nested-compositor behaviour suite (real typed keystrokes via the isolated Wayland injector, never the live session) ran all 16 Files scenarios against the same build: **14/16 pass**, including `rename-file`, `rename-folder`, `new-folder-name` and `undo-rename` — all previously blocked live by the missing injector. The 2 fails are already-tracked, non-blocking parity gaps: `context-menu` is missing Share…/Quick Actions items (FILES-23/49) and `get-info` opens a modal card instead of a separate window (FILES-15). | **Fail** — ACC-03 (no accessible file list, no working rename path) is fixed and live-reconfirmed; the one remaining gap is the same upstream `accesskit_unix` `EditableText` limitation as journey 1 (real keyboard/pointer use is unaffected); FILES-15/23/49 are tracked, non-blocking |
| 3 | Terminal: run, scroll, select, copy/paste, tabs | `scripts/linux/run-journey-terminal.py`, `scripts/assert_terminal_accessibility.py` | **Re-run 2026-09-25.** `run-content-accessibility.sh ... terminal` exits 0 live: "62 characters over 24 lines, caret 41 after the prompt, line navigation returns single lines." Separately, a nested-compositor probe launched a fresh `rmac-terminal`, clicked the grid, typed `echo LULO_PROBE_OK` through the isolated virtual keyboard, and read the result back over AT-SPI `Text`: the grid's text included the typed command and its real output. The terminal grid now publishes real text/caret/selection (ACC-01); the tab strip is named per `docs/journey-suite.md`. | **Fail** — ACC-01 (no accessible text-entry surface) is fixed and live-reconfirmed with a real typed round-trip; the one remaining gap is the same upstream `EditableText` limitation as journey 1 (Terminal intentionally doesn't wire `SetValue` either, since a blind whole-buffer replace is the wrong model for a shell) |
| 4 | Notes: create, search, edit, recover after crash | `scripts/linux/run-journey-notes.py`, `scripts/assert_notes_accessibility.py` | **Re-run 2026-09-27.** `run-content-accessibility.sh ... notes` exits 0 against a disposable Notes library: named New Note Click creates a selected note; New Folder has a named Click action; Checklist, Add Photo… and Move Note… are named, but their Click actions were absent in one post-create run and present in another, so stable activation is unverified; in-note Find's Previous match, Next match and Done actions were activated live. Search/Title/Body/Tags expose named Text entries with Title focus and caret. A title/body typed round-trip and crash recovery have not been independently re-confirmed in this pass; More, View Options and Folder Actions remain unnamed (ACC-08). | **Fail** — the content surface and some toolbar and Find accessibility actions pass, but upstream `EditableText`, remaining ACC-08 controls, and the full recovery journey remain open |
| 5 | Open/edit/save a text file through the portal | `scripts/linux/run-journey-textfile.py`, `scripts/behavior/run_lulo.py text-editor/*` | **Re-run 2026-09-27:** the nested `save-untitled` scenario passes 1/1 against installed binaries with the `rmac-file-chooser` backend. It observes a focused `Untitled` name selected `[0,8]`, Cancel/Save buttons, and Escape dismissal (`/tmp/save-untitled-20260927.json` on Lulo). The previous failure came from observing after 1.5 s and sending Escape before the D-Bus-activated chooser's AT-SPI window was ready; the scenario now waits 4 s. The chooser appears as a separate, blank-titled window in nested Sway; visual/parent-window parity is not covered by this assertion. The chooser service is deployed after reinstall, but the live portal journey has not been rerun. | **Not yet confirmed live** — nested product behavior passes with its backend; live Open/Save As remains untested |
| 6 | Inspect resource use, safely stop a process | `scripts/linux/run-journey-monitor.py` | **Re-run 2026-09-26** live against a fresh `rmac-system-monitor` build (`5decc694`, branch `table-a11y`), using its own disposable process. The table fix makes off-screen rows accessible and selectable; menu lookup was corrected from Process to View. A later live rerun also verified select → Quit → confirmation → Cancel → Quit again → confirm ends the process. MON-15 is fixed by registering the menu target (`3e75d3d8`); the behavior suite covers reopening Quit and Force Quit confirmations after Cancel. | **Pass (9/10), 2026-09-26.** The only remaining failure is `search_field_editable`, due to the upstream `accesskit_unix` EditableText gap. |
| 7 | Wi-Fi, Bluetooth, audio output, battery | none (code trace + read-only live checks) | `docs/journey-7-trace.md`'s source trace stands. This pass added **read-only live queries** on the laptop's real services (no toggling, no connecting): `nmcli device status`/`nmcli radio` show Wi-Fi connected and radios enabled; `bluetoothctl show` shows the controller powered on with real UUIDs; `wpctl status` shows PipeWire's real Analog Stereo sink/source (the laptop has no `pactl`/`pipewire-pulse` compat layer active, so Settings' audio backend must be the native PipeWire path, not a PulseAudio shim — matches `docs/settings-backend-audit.md`); `upower -i` on `battery_BAT0` returns real battery state (95%, fully-charged, real voltage/energy). All four backends are real, live, queryable services, not stubs. | **Not yet run** as a full live Settings-UI acceptance test — the backends themselves are confirmed real and live this pass |
| 8 | Journeys 1–7, keyboard only | nested Ctrl-F2 probe, within-app keyboard scenarios | **Nested-confirmed 2026-09-26:** with the current niri config and isolated virtual keyboard, Ctrl-F2 focused the menu bar, Right selected Files, Down opened its visible menu, another Down focused Empty Trash… without activating it, and two Escapes closed the menu and exited keyboard mode. AT-SPI focus and screenshots agreed; 8/8 focused checks passed. The Dock is keyboard-reachable via Control-F3, and within-app arrow, Return and ⌘ shortcut scenarios are exercised separately. Letter typeahead, Return activation, complete journeys 1–7 by keyboard, and the owner-only Orca pass remain unverified. | **Partial** — menu-bar entry/navigation works in nested niri; full keyboard-only journeys and Orca remain open |
| 9 | Core of 1–7 with Orca at 200% | none | No dedicated script; Orca was **not enabled** this pass (per the brief, only the owner may do that). The formal I3 accessibility-release audit (`docs/accessibility-release-audit.md`, `scripts/accessibility-audit.json`) requires 442 explicit Orca observations; no evidence file exists. This pass did verify, in the nested compositor only, that the AT-SPI tree survives a compositor output scale of 2 (`swaymsg -t get_outputs` reports `scale: 2.0`; a freshly launched Files window still exposed its full 27-node tree with named sidebar items at that scale) — a narrow, positive signal that scaling doesn't collapse the accessibility tree, not a substitute for a real Orca pass. | **Owner/manual** — steps: on the reference laptop, enable Orca and 200% text scaling, then walk journeys 1–7 by ear, logging each surface against `scripts/accessibility-audit.json`'s observation list; write the result to `docs/accessibility-release-audit.md` |

The package-scoped automated fixture suite (`scripts/run-journey-suite.py`,
`docs/journey-suite.md`'s "I1", 10 journeys mapped to Cargo package tests)
was not run in this session — it needs `cargo test`, which this session
does not have. The pure-Python logic tests for the four live scripts above
(`test_journey_launch.py`, `test_journey_files.py`, `test_journey_terminal.py`,
`test_journey_notes.py`) do pass, but they only cover the scripts' own
JSON/parsing logic, not a live run.

## 2. Performance budgets (todo.md)

| Metric | Budget | Status |
|---|---|---|
| Warm launch to interactive | p95 ≤ 500 ms (900 ms Files/Terminal) | **Pass for nine measured apps** on the 2026-09-27 `8ae3a9eb` release build: all used the ready-file marker, with p95 from 150.1 ms (Text Editor) to 290.7 ms (Notes). App Drawer and Preview now have single private first-frame samples of 282.4 ms and 328.7 ms on the installed `0e3fa470` binaries, but no five-launch p95. See [release-binary report](perf/reference-laptop-2026-09-27-release.md). |
| Idle CPU | ≤ 0.3%/normal app, ≤ 2.5% System Monitor, ≤ 0.5% shell combined | **Pass on the 2026-09-29 release-binary candidate:** Text Editor 0.000%, Clock 0.050%, System Monitor 2.017%, Files 0.033%, Weather 0.000%; shell services combined 0.200%. Each app ran alone for 60 seconds with private XDG directories. The candidate is not yet installed as a package. See [before/after report](perf/idle-cpu-2026-09-29.md). |
| Idle wake-ups | none while nothing changes | **Static app timers fixed; proxy improved:** Text Editor 9.050 → 0.000 context switches/s, Weather 8.450 → 0.000, Files 0.633 → 0.050, Clock 5.267 → 0.267, System Monitor 3.800 → 1.367; shell combined 2.150 → 1.300. Clock advances at minute boundaries and System Monitor samples visible metrics every five seconds. Context switches are a wake-up proxy, not exact frame counts. See [before/after report](perf/idle-cpu-2026-09-29.md). |
| Input to visible response | p95 ≤ 50 ms | **Not yet run** — no frame-timing harness exists yet |
| 60/120 Hz animation frame budget | ≥ 99% / ≥ 95% within budget | **Not yet run** — `docs/performance-baseline.md` notes no per-frame trace is available yet |
| Memory (8-hour soak) | per-app budget, no leak | **Not yet run** — proposed Beta 2 gate (see the split above) |
| Repeat on an NVIDIA system | required before Beta | **Not yet run** — no NVIDIA station in the matrix yet; proposed Beta 2 gate |

Earlier evidence: [system audit](perf/reference-laptop-2026-09-24.md),
[Clock before/after](perf/reference-laptop-2026-09-26-clock.md), and
[system-audit notes](system-audit-2026-09-24.md). The current
[release-binary run](perf/reference-laptop-2026-09-27-release.md) used a
temporary HOME and XDG directories per app, no Cargo contention, and one app
at a time. Its marker and XDG writes are outside Files' watched HOME and parent;
the earlier layout inflated Files' result. The newer
[candidate run](perf/idle-cpu-2026-09-29.md) passes the corrected idle CPU
budgets; it has not yet been installed. Frame pacing, input response, soak
memory, and NVIDIA results remain unmeasured.

## 3. Accessibility gates (todo.md)

| Gate | Status | Evidence |
|---|---|---|
| Stable identity/role/name/state/actions on every `rmac-ui` control | **Fail** (partial) | `docs/accessibility-audit.md`, “Component × criterion” and “What still needs gpui-kit/gpui-component work”: context-menu items and list/tree rows now expose roles and names; the unused plain `PopUpButton` variant remains a code-level role gap. The audit marks this as a Gap, not an upstream Blocked issue; runtime assistive-technology review remains open. |
| Correct tab order, visible focus ring | **Fail** (partial) | Same doc: a shared 3pt focus ring now exists and is used on the rewritten controls; everything still wrapping `Button` keeps a hardcoded 1.5px ring it cannot override (Blocked) |
| Full keyboard operation, no pointer-only controls | **Fail** (much improved) | Terminal, Notes and Files' content surfaces are fixed and live-reconfirmed (ACC-01/02/03; journeys 2–4 above). Remaining gaps include `accesskit_unix`'s missing `EditableText`, the Popover trigger's mouse-only activation, Notes' unnamed toolbar buttons (ACC-08), and journey 8 keyboard checks that remain incomplete (letter typeahead, Return activation, and full keyboard-only journeys). See `docs/accessibility-audit.md`, “PopUpButton” and “What still needs gpui-kit/gpui-component work”; these are code-level findings, not Orca verification. |
| Announcements for async status/errors | **Fail** (partial) | `docs/accessibility-audit.md`, “Component × criterion”: Toast, EmptyState errors, and List/Tree state messages have code-level announcement roles. `TextField` inline validation errors still lack a live announcement; runtime screen-reader review remains open. |
| No clipping at 200% | **Not yet run** (S) | `docs/accessibility-audit.md`: static read-through only, no rendered check at 200%. This pass separately confirmed (nested compositor only) that a compositor output scale of 2 doesn't collapse the AT-SPI tree — a Files window still exposed all 27 nodes with named items — but that isn't a visual-clipping check |
| Usable high-contrast colours | **Pass** (token level) | Theme contrast-ratio tests pass; no shared component hardcodes a raw color |
| Reduced motion from the Settings portal | **Pass** (plumbing) / **Not yet run** (exercised) | Portal → theme wiring is tested end to end, but no shared component currently animates anything, so the gate has nothing live to violate yet |
| Verify every release journey with Orca | **Fail** | Orca itself has not been run against any journey (only the owner may enable it). Journeys 1–6 now have real, live or nested AT-SPI-tree evidence (see §1); 7 has live backend checks but no Settings-UI run; 8's Control-F2 gap is confirmed; 9 is Owner/manual |
| Formal I3 accessibility-release audit (442 observations) | **Not yet run** | `docs/accessibility-release-audit.md` / `scripts/verify-accessibility-audit.py`; no evidence file committed |

**This is the release-blocking category.** `todo.md` states accessibility
gates are never waived. As of 2026-09-25, Terminal, Notes and Files no
longer ship with zero accessible content — ACC-01/02/03 are fixed and
live-reconfirmed on the reference laptop, both via the repo's own
`scripts/assert_*_accessibility.py` acceptance scripts (live session, exit 0
on all three) and a nested-compositor run with real typed keystrokes (Files:
14/16 behaviour scenarios, including working rename). What remains
release-blocking in this category: the upstream `accesskit_unix`
`EditableText` gap (not fixable in this repo, affects Spotlight, Terminal,
Notes and Files' search/rename fields identically), mouse-only Popover
triggers, the plain `PopUpButton` role gap, Notes' unnamed toolbar buttons,
the unfinished keyboard-only checks in journey 8, and the formal Orca/I3
audit.

## 4. Code rules (todo.md)

| Rule | Status | Evidence |
|---|---|---|
| No destructive-operation error dropped with `let _ = ...` | **Pass** (fixed) | `docs/code-rules-audit.md`: 5 real violations found and fixed (window/Finder/media-player state saves now log; clipboard payload delete and an unconfirmed permanent-delete path now refuse instead of silently dropping/deleting) |
| Persisted data: versioned serde, temp-sibling + fsync + atomic rename via `rmac-storage` | **Fail** (partial) | Same doc: 5 low-severity settings/cache writers (`rmac-screenshot`, `rmac-compositor` parking cache, `rmac-gtk-settings` stub writer, `rmac-dock-runtime` recents, `rmac-clipboard-linux`) still hand-roll their own write path instead of `rmac-storage`; none touch high-value user content, but none are fixed yet (needs a Linux `cargo check` this session didn't have) |
| User-visible failures: typed errors with a recovery action | **Pass** | No violations found in the crates audited |
| Logs never include secrets or document contents | **Pass** | No violations found |
| Domain crates never import GPUI/Wayland/D-Bus/platform FFI | **Pass** (with 2 naming nits) | `rmac-app-menu`/`rmac-osd` carry `zbus` without a `-linux`/`-system` suffix — functionally fine, a naming-convention note only |
| Files safety rules (symlinks, conflict policy, cancel, Trash-first, zip-slip) | **Pass** (1 bug found and fixed) | `docs/code-rules-audit.md` Rule 6: `DeleteItem`'s unconfirmed permanent-delete path (dead code, nothing bound to it) now refuses instead of deleting |
| Never parse human-readable CLI output on Linux | **Pass** (fixed) | `docs/code-rules-audit.md` Rule 7: UFW/Samba/PipeWire status now read structured state; full repo grep found no remaining violation on the Linux build |

This audit's scope explicitly excluded `crates/system-settings/**`,
`shell/bins/rmac-wallpaper/**`, `shell/crates/rmac-shell-ui/**`, and the
network/Bluetooth/audio/power/top-bar/quick-settings crates (other agents
were actively editing them) — those are **not yet audited** against these
rules at all.

## 5. Packaging: install / remove / upgrade

| Check | Status | Evidence |
|---|---|---|
| Native package contract tests | **Pass** | `python3 -m pytest scripts/test_native_packages.py scripts/test_session_package.py scripts/test_application_package.py scripts/test_keyring_packages.py` all pass in this session |
| Release workflow structural checks | **Pass** | `python3 -m pytest scripts/test_release_workflows.py` passes; `release.yml` now tags pre-release builds correctly (this pass) and needs no APT-signing secrets to build/attach `.deb`s, `SHA256SUMS`, the SBOM, and provenance |
| Real install/remove/upgrade on a clean Ubuntu 26.04 VM or the reference laptop | **Not yet run** | Needs an actual `apt install ./rmac-apps_*.deb ./rmac-session_*.deb`, a second install as an upgrade, and `apt purge`, on a real machine — this session had no cargo and no laptop UI access for a packaging run |
| GDM session-selection, crash-loop safe mode, TTY repair, recovery, GNOME-fallback journey (H4/H6) | **Not yet run** | `docs/session-journey-evidence.md` is a runbook; no evidence file from an actual run is committed |
| Real GitHub Actions release run (untested runner plumbing) | **Not yet run** | `docs/release-process.md` "Known gaps": the non-root-build-user-in-a-container pattern and container disk space have never been exercised against a real GitHub Actions run — expect to debug the first tag push |
| Reproducible build (two independent byte-identical package assemblies) | **Not yet run** (this session) | Contract exists and is unit-tested (`check-native-reproducibility.sh`); not executed here (no cargo) |
| Signed APT repository install/upgrade/rollback | **Not applicable to this Beta by design** | Explicitly deferred to after Beta per the owner's scope decision; `apt-repository`/`keyring` jobs stay off until the signing-key and source-package decisions in `docs/release-process.md` are made |

## 6. Documentation

| Check | Status |
|---|---|
| `python3 -m pytest scripts/test_documentation.py` | **Pass** |
| `python3 scripts/verify-documentation.py` | **Pass** — "rmac documentation set verified (15 required topics)" |
| CHANGELOG.md | **Pass** — added this pass |
| docs/release-notes.md (required doc) + docs/release-notes/0.9.0-beta1.md | **Pass** — added/updated this pass |
| docs/known-limitations.md reflects current reality | **Pass**, but see §7 — it already names the release blockers honestly and should stay accurate as the gaps above close |
| docs/install.md / README.md readiness banners | **Needs an owner decision** — both currently say the product "is not released for general installation yet" / "isn't ready for daily use yet." Shipping 0.9.0-beta.1 as a GitHub Release means someone outside the project can now install it; those banners should be reworded (early-access Beta, not "not released yet") once the owner confirms this release is going out, rather than left contradicting the Release page. Not changed in this pass since it's a product-readiness claim, not a mechanical doc-set fix. |

## 7. Known limitations

`docs/known-limitations.md` is current and, cross-checked against the live
journey evidence above, accurate. It already names the Dock/menu-bar
keyboard gaps and the AT-SPI `EditableText` upstream gap; as of 2026-09-25
that gap description is no longer stale — Terminal, Notes and Files'
previously undocumented "no accessible content at all" failures are now
fixed (ACC-01/02/03, §1/§3), so `EditableText` really is the residual gap
those sections describe, not an understatement of a bigger problem. The
placeholder top-bar mark and the absence of a signed APT repository are
also still named and accurate.

## 8. CI (historical 2026-09-24 failure snapshot)

The preceding `dev` commit `a1150511` passed
[CI](https://github.com/millionrust/lulo/actions/runs/36263486076) and
[quality gates](https://github.com/millionrust/lulo/actions/runs/36263485988).
The old result was checked via `ssh jacob@192.168.18.52 'gh run list -R
millionrust/lulo --branch dev --limit 3'` (the laptop has `gh` auth). The
`dev` HEAD at the start of this pass (`3ca78f24`, run
[36151133746](https://github.com/millionrust/lulo/actions/runs/36151133746))
was **red**:

| Job | Result | Cause | Fixed this pass? |
|---|---|---|---|
| Dependency policy | success | — | — |
| Linux checks | failure | `rustfmt --check` diff in `crates/text-editor/src/view/startup.rs` | **Yes** — reformatted, `rustfmt --edition 2021 --check` now clean |
| Current GPUI Linux runtime gate | failure | same `rustfmt` diff | **Yes** |
| Linux checks (Ubuntu 26.04, non-blocking) | failure | same `rustfmt` diff | **Yes** (non-blocking anyway) |
| Release contracts | failure | `scripts/test_application_icons.py`: `crates/rmac-dock/assets/icons/stack-item-document.svg` (shipped, referenced by `shell/bins/rmac-dock/src/main.rs:4581` and the session-package staging/verify scripts) was never added to the `DOCK_ICONS` inventory whitelist | **Yes** — added to the whitelist; `python3 -m pytest scripts/test_application_icons.py` now 4/4 |
| macOS checks | failure | `cargo clippy --workspace --all-targets --all-features -D warnings` on macOS: (1) `crates/rmac-updates-linux/src/transaction.rs:14` imports `FLAG_ONLY_DOWNLOAD`/`FLAG_ONLY_TRUSTED`, which are `#[cfg(any(target_os = "linux", test))]`-gated in `api.rs`, so they don't exist when checking on macOS outside `test`; (2) `crates/preview/src/view.rs:319` has two fields (`window_generation`, `print_busy`) unused under `--all-targets`' test-binary build, hit by `-D dead-code` | **No** — needs cargo + a macOS target to fix and validate safely; not attempted blind |
| Mac behaviour parity (non-blocking) | failure | 6/23 behaviour scenarios match the Mac — already tracked (FILES-43–49, TE-19–20, ACC-09 in `docs/parity.md`), explicitly non-blocking | not applicable |

This pass's two fixes are committed on this branch (`beta-checklist`) and
validated locally (`rustfmt --edition 2021 --check
crates/text-editor/src/view/startup.rs`; `python3 -m pytest
scripts/test_application_icons.py`, 4 passed) — per `AGENTS.md`, no cargo was
run on the Mac; both fixes were cross-checked against the laptop's own
`rustfmt`/`pytest` too. The two macOS compile errors described here were
fixed in later commits and the current macOS job passes.
`linux-2604` (the Ubuntu 26.04 hosted-runner trial) is still explicitly
non-blocking per `todo.md`.

## 9. Versioning and release engineering (this pass)

- Both workspace manifests (`Cargo.toml`, `shell/Cargo.toml`) now version
  `0.9.0-beta.1` (Cargo/semver pre-release syntax, so `0.9.0` final sorts
  higher under Cargo's own ordering).
- `scripts/linux/native_package_contract.py` and
  `scripts/linux/keyring_package_contract.py` translate that into the
  Debian tilde convention (`0.9.0~beta.1`) for the actual `.deb` `Version`
  field, so `dpkg`/APT also order a Beta below its eventual final release;
  `scripts/linux/run-package-lifecycle.py`'s manifest-version check was
  updated to accept the same pattern.
- The 13 app AppStream `metainfo.xml` files carry a `0.9.0-beta.1` release
  entry; `scripts/linux/verify-application-package.py`'s expected value
  matches.
- `release.yml`'s `attach-release` job now publishes any tag with a
  pre-release suffix (e.g. `v0.9.0-beta.1`) as a GitHub pre-release.
- Every pinned-version Python test that reads the real repository was
  updated and passes (see the command below).

## Required test run (this session)

```sh
python3 -m pytest -q scripts/test_native_packages.py scripts/test_session_package.py \
  scripts/test_release_workflows.py scripts/test_documentation.py
python3 scripts/verify-documentation.py
```

All pass. (Also re-checked, not in the required list but touched by the
versioning changes: `scripts/test_keyring_packages.py`,
`scripts/test_application_package.py`, `scripts/test_alpha_candidate.py`,
`scripts/test_beta_candidate.py`, `scripts/test_one_dot_zero_candidate.py`,
`scripts/test_package_lifecycle.py` — all pass.)

## What actually blocks shipping 0.9.0-beta.1 today

Updated 2026-09-27. The go/no-go table at the top is the current punch list;
the items below explain the remaining gates in more detail:

1. **Accessibility (never waived) — much improved, not clear.** Terminal,
   Notes and Files no longer have the "zero accessible content" bugs that
   made this the top blocker as of 2026-09-24: ACC-01/02/03 are fixed and
   live-reconfirmed on the reference laptop this pass (§1, §3). What remains
   release-blocking: the upstream `accesskit_unix` `EditableText` gap,
   pointer-only Popover triggers, incomplete journey 8 keyboard coverage (letter typeahead, Return activation, and full keyboard-only journeys),
   Notes' unnamed toolbar buttons (ACC-08), and the formal Orca/I3 audit
   (journey 9, owner-only). Current component-level roles and remaining
   announcement gaps are recorded in `docs/accessibility-audit.md`,
   “Component × criterion” and “What still needs gpui-kit/gpui-component work.”
2. **No clean-VM package lifecycle run or exercised release workflow.**
   Local amd64 candidate packages pass their artifact and native-pair checks,
   but the install/upgrade/uninstall path and GitHub Actions Release workflow
   still need end-to-end evidence.
3. **Performance: the new idle candidate is not installed.** Nine apps pass
   the earlier warm-launch budgets. Files, System Monitor, Text Editor,
   Clock, and Weather now pass their idle CPU budgets in the live
   [2026-09-29 release-binary run](perf/idle-cpu-2026-09-29.md), but the
   installed package is older. Input latency, frame pacing, soak memory and
   NVIDIA remain open.
4. **Security review: Fail (source review done, gate not met).**
   [docs/security-review-0.9.0-beta.1.md](security-review-0.9.0-beta.1.md)
   and its canonical summary `docs/security-review-0.9.0-beta.1.json` cover
   all 80 checks of `scripts/security-review.json`. 26 findings are fixed
   (9 during the review, 17 after it, including every High and Medium). 3
   remain open, all Low, each accepted for Beta with a risk statement and a
   mitigation (SR-15 build paths in locally built binaries, SR-18 release
   build inputs pinned by tag, SR-29 automatic updates not simulated for
   removals). A fresh pass on 2026-09-25 over the new root-run and
   privileged code (system-sleep hook, power-key inhibitor, rmac-process,
   the update flow, the shared D-Bus connection) found SR-28 (fixed) and
   SR-29. 56 checks pass on source review; 24 need native station
   evidence. None of the Beta stations has been run, so
   `verify-security-review.py` fails closed, as it should.
5. **Product journeys remain incomplete.** Journey 5's nested Save-panel
   scenario passes and the chooser service is deployed, but the live portal
   flow has not been retested; journey 6's process-stop path is live-confirmed
   except for upstream EditableText, while journey 7 has backend evidence but no full
   Settings-UI run. Journeys 8/9 remain Partial/Owner-manual (§1).
6. **`docs/install.md`/`README.md` still say the product isn't ready to
   install** — an owner-level messaging decision, not a mechanical fix.

None of these are release-engineering plumbing problems — the tagging,
packaging contracts, versioning, and pre-release workflow gating are ready
(§5, §9). What's missing is the evidence that the product itself is safe
and usable enough to hand to someone outside the project, which is exactly
what `todo.md`'s accessibility/security/data-safety gates exist to prove —
and, as of this pass, a real and growing share of that evidence now exists.
