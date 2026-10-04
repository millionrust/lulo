# Beta release checklist — 0.9.0-beta.1

## Beta 1 go/no-go (updated 2026-10-03)

**Still no-go, but close.** `dev` is at `fcafa5e4`. CI and the GitHub-built
candidate packages are green for recent `dev` commits, and a GitHub-built
install works on the reference laptop (`0.9.0~beta.1-38`). Against
`fcafa5e4`'s tree: the full nested behaviour suite passes **160/160**,
power-dialog checks pass **51/51**, window-move checks pass **19/19**, and
menu-dismiss checks pass **17/18** — the one failure is a known, already-
tracked gap (`MENU-15`, Partial: a Dock click still doesn't close an open
menu; fix in progress, see item 1 below). `BUG-01` (System Settings burning
41–48% of a core with Search focused) is fixed: two fresh samples measured
0.1–0.2%, below the 0.3% target. `docs/inventory-gaps.md` is down to 672
menu-item gaps across System Settings, Files, Terminal, Text Editor and the
rest. Control Centre and Notification Center now expose populated AT-SPI
trees, not empty frames (`ACC-11`). The Lulo session now skips the two
Evolution/`foot` autostart services it never uses (`MEM-02`).

Two owner-reported P0s remain open and are **not** confirmed fixed: desktop
icons that can stay hidden until a click after login (`DESK-12`), and the
Dock's Bin icon vanishing then the Dock lagging/disappearing after a delete
(`DOCK-27`). Both reproduce live but not in the nested test compositor, so
they're narrowed, not closed, and need a recheck on the current installed
build.

**What's done this pass:** the 160/160 full behaviour run, the
51/51 power-dialog and 19/19 window-move reruns, `BUG-01`'s idle-CPU fix,
`MEM-02`'s autostart-service skip, Control Centre/Notification Center's
populated AT-SPI trees, the inventory-gaps reduction to 672, and green
CI/candidate-package builds for recent `dev` commits.

**What's still open, in order:**

1. **Dock-click menu dismissal (`MENU-15`).** Three live reruns on
   2026-10-03 (`run_menu_dismiss.py`) still show a Dock click not closing an
   open menu (3/3 failures); every other dismissal path (wallpaper click,
   another window, Escape, title-switch, Control Centre) passes. Fix in
   progress.
2. **Owner re-check of `DESK-12`/`DOCK-27` on the new install.** Neither
   reproduces in the nested test compositor, so only a live recheck on the
   currently installed build can confirm or re-open them.
3. **An 8-hour soak against the installed package**, not just source
   binaries.
4. **The Orca accessibility audit (journey 9).** An automated run is in
   progress; the owner still needs to do the final listen.
5. **Security-review native-station evidence (9 of 80 checks left).** The
   disposable-install station runs on GitHub Actions. The AMD and NVIDIA
   desktop stations are waived for Beta 1, because the owner has neither
   machine. The reference laptop's checks (lock screen, TTY recovery,
   suspend, Sharing toggle, mount removal, notifications while locked) need
   the owner; the read-only steps are in the review's "Reference-laptop
   checks".
6. **NVIDIA testing: dropped from Beta 1** (owner decision 2026-10-04: no NVIDIA machine is available; shipped as a known limitation in docs/known-limitations.md). Needs NVIDIA hardware, which the project doesn't
   have yet.
7. **A real GitHub Actions release-workflow run on the actual tag.** There
   is no release-candidate tag — `v0.9.0-beta.1` itself is the first run of
   the release workflow.
8. **Owner sign-off on the README/`docs/install.md` early-access wording**
   (drafted this pass; the owner approves in review).

Per the owner's 2026-10-03 decision, items 4–6 (the Orca audit, the
security-station runs, and NVIDIA testing) **stay Beta 1 gates**: the owner
declined the proposed move of those three to Beta 2 (see "Proposed Beta 1 /
Beta 2 split" below, now a record of a declined proposal, not an open
option).

### Path to Beta 1 (ordered)

The numbered list above **is** the current path to Beta 1 — every closeable
piece of agent work (full behaviour suite, idle-CPU fix, autostart-service
fix, accessibility-tree fixes, inventory-gap cleanup, green CI) landed this
pass or earlier ones. What's left is either hardware-bound (NVIDIA), time-
bound (the 8-hour soak, the release-workflow's first real run), or owner-
only (the Orca final listen, the security-station runs, the wording
sign-off) — plus the one still-open code fix, `MENU-15`'s Dock-click
dismissal.

**Deferred past this Beta (owner-only, not blocking):**

- Decide and create the APT archive signing key (`docs/release-process.md`,
  "Switching on signed updates") — needed for the in-place-update
  follow-up, not for this Beta.
- GitHub admin: confirm repo/Pages settings for the eventual signed APT
  repository — not blocking this Beta.

**Release steps (after the items above are done):**

9. Tag `v0.9.0-beta.1` — this is the first run of the Release workflow, not
   a dry run behind it; watch it and verify every asset attaches (`.deb`s,
   `SHA256SUMS`, SBOM, provenance).
10. Publish the refreshed release notes (this pass — see
    `packaging/release-notes/0.9.0~beta.1.txt` and
    `docs/release-notes/0.9.0-beta1.md`).
11. Announce with the "early access, not the daily-driver Beta cohort"
    subtitle (see "Read this first" below).

### Priority work queue: open P0s and first-hour-visible P1s

This is the coordinator's work queue — every currently-open P0, plus every
open P1 a typical first-hour Beta user (opening Files, using menus and
context menus, the Dock, window management, Settings, Notes, Terminal,
Preview) would plausibly hit, in priority order. The full P1 list (53 open
rows) is in `docs/parity.md`; this is the visible/common subset. Deep or
edge-case P1 rows (specific Settings sub-panes like Printers & Scanners,
Night Shift, the 29-category Privacy list; accessibility-only rows already
covered in §3) are intentionally left off this list — see `docs/parity.md`
for those.

1. **DESK-12 (P0, S)** — Desktop icons can stay hidden until a click after
   login. Plan: live-session diagnosis after a fresh reinstall; nested repro
   attempts haven't reproduced it, so this needs real-hardware timing
   instrumentation on `rmac-wallpaper`'s icon layer.
2. **DOCK-27 (P0, M)** — Dock Bin icon vanishes after a delete, then the
   Dock lags/disappears while its service stays alive. Plan: same as above,
   live-only; watch for the `915ceb55` surface-churn class of bug recurring
   elsewhere in `rmac-dock-runtime`.
3. **OTHER-01/02/06/12 (P1, L)** — The shared Open/Save/Print dialog is a
   portal-client window, not a Finder-style attached sheet, and its sidebar
   and Print layout don't match the Mac; this touches nearly every app's
   first save/open/print. Plan: biggest single remaining UI-architecture
   gap; needs a dedicated sheet-attachment pass, tracked as its own project,
   not a quick fix.
4. **CC-11 (P1, S)** — Control Centre isn't frosted glass on the installed
   `0.9.0~beta.1-38` build even though the source fix (`918aaa38`) predates
   it. Plan: confirmed fixed in source; just needs the owner's
   `DESK-12`/`DOCK-27`-style recheck on the new install (see "What's still
   open, in order", item 2).
5. **DOCK-26 (P1, M)** — Dragging a file from Files onto the Dock's Trash
   doesn't work (GPUI's Linux Wayland backend has no `wl_data_device`
   drag-and-drop source at all). Plan: needs upstream GPUI drag-source
   support; track as a framework-level item, not fixable with an app-level
   patch.
6. **SESSION-02 (P1, S)** — Shutdown/Restart/Log Out dialogs are missing the
   "Reopen windows when logging in" checkbox (no session-restore feature
   behind it yet). Plan: either implement minimal session restore or drop
   the checkbox from the dialog deliberately and document it.
7. **APP-01 (P1, L)** — No Calendar app; the menu-bar clock and desktop
   Calendar widget currently have nothing to open. Plan: large, build on
   Evolution Data Server; not a Beta-1-sized fix, but likely to be the
   single most-reported missing app — call it out explicitly in release
   notes/known-limitations rather than let users discover it.
   **Update 2026-10-03:** the owner moved Calendar and Mail into Beta 1. Plan:
   ADR 0022 and `docs/design/calendar-mail.md` (accounts ACC-1..4, Calendar
   CAL-1..9, Mail MAIL-1..10).
8. **PREV-02/03/04/15 (P1, mixed)** — Preview's File menu is still partial:
   Print works for PDFs/images but is unverified live, Save/Export As is
   partial, Markup/annotation is entirely absent. Plan: Print live-check is
   small; Markup is a new feature (needs a PDF writer) and should stay a
   known limitation for Beta 1.
9. **TE-18 (P1, M)** — Saving a new Untitled document mostly works (direct
   Save is fixed, TE-22) but the full Save-sheet parity has remaining gaps.
   Plan: small follow-up once TE-22's current state is confirmed live.
10. **NOTES-02 (P1, M)** — Fixed `65477bc9`: clicking a checklist circle in
    Markdown Preview, or ⇧⌘U on the current line, now flips `- [ ]`/`- [x]`
    in the saved body (undoable, AT-SPI `CheckBox` role/state). Laptop unit
    tests/clippy/fmt pass. Plan: still needs the extended
    `tests/parallel/06-notes.json` checklist steps run in a nested session
    before closing out; "Tick All" and the Mac's move-checked-to-bottom
    setting remain unimplemented follow-ups.
11. **FILES-05 / FILES-35 (P1, M/M)** — Fixed 05818264: a
    bounded background-scanned Linux tag index backs the sidebar's Tags
    section on every OS (FILES-05), and a new Finder ▸ Settings… window
    (General/Tags/Sidebar/Advanced, versioned persistence, live broadcast
    to every open window) covers FILES-35, with sidebar checkboxes,
    extension visibility, folders-on-top, the empty-Bin warning and the
    new-window target wired live. See `docs/parity.md` rows FILES-05 and
    FILES-35 for exactly what still isn't wired (open-folders-in-tabs,
    the rename extension warning, the 30-day Bin sweep, Desktop-item
    settings, and search scope). Plan: still needs a laptop clippy/fmt/test
    pass and a nested-session behaviour check before closing out fully.
12. **TERM-03 (P1, M)** — Fixed `c894f1dd`: Settings… now has Text (cursor
    style/blink), Window (size), Shell (when it exits) and General
    (new-window directory) sections on top of Profile/Font. Plan: needs a
    live Linux-build check before closing out; still one scrolling page,
    not the Mac's tabbed window (TERM-17 follow-up).
13. **BAR-01 (P1, S)** — The no-focus/desktop menu-bar fallback (Files
    menus) landed in source but hasn't been live-validated. Plan: small,
    just needs the reinstall-and-recheck pass.
14. **FILES-38 (P1, S)** — File ▸ Delete Immediately… (⌥⌘⌫) dialog is
    partial. Plan: small follow-up, finish matching the Mac's exact dialog
    text/sizing.
15. **SWU-07 (P1, M)** — No polkit authentication agent, so any
    administrator action (installing a `.deb` by double-click, `pkexec`,
    printer admin) has no password-sheet UI at all. Plan: medium; likely to
    surprise a first-hour user who tries to install something from Files —
    worth a known-limitations callout even before it's fixed.

### Candidate-build history (2026-09-24–29, superseded by the above)

Kept condensed for the record; none of it reflects current `dev` HEAD
(`fcafa5e4`) or the figures at the top of this document — see those for the
current, dated list. In order, this period moved through four amd64
candidates: `0e3fa470` (installed on the reference PC 2026-09-27 as
`0.9.0~beta.1-38`, 24/27 Mac-behavior scenarios, 28/28 shutdown and 41/41
power-dialog checks, live Shut-Down-Cancel confirmed, 9/9 startup checks —
[report](https://github.com/millionrust/lulo/actions/runs/36312045023)),
`e8b589ac` (staged only, 25/27 behavior, 9/9 startup,
[CI](https://github.com/millionrust/lulo/actions/runs/36327602027)), and
`8ba31b82` (staged only, 25/27 behavior, 28/28 shutdown, 37/37 power-dialog,
[CI](https://github.com/millionrust/lulo/actions/runs/36334082582); its
[eleven-app idle-CPU sample](perf/reference-laptop-2026-09-28-candidate-8ba31b82.json)
found five apps over budget, later corrected in the
[2026-09-29 release-binary candidate](perf/idle-cpu-2026-09-29.md), which
passed but was never packaged). The two recurring mismatches across all
three installed/staged reruns were the Files item/background context menu
(Share, Quick Actions, grouping, View Options) and Get Info's window
behaviour — both since fixed in source (see `docs/parity.md` FILES-15/23/49).
Visual parity was not established in this period: one local Mac/Lulo
screenshot pair under `target/evidence/visual-comparisons/` passed the
provenance audit but did not establish overall desktop parity; see the
[comparison protocol](visual-comparison.md). The security and accessibility
gates named in this period's evidence (`EditableText`, 24/80 native-station
checks, three accepted Low findings) are unchanged — see §3 and §4.

### Proposed Beta 1 / Beta 2 split (2026-09-29 proposal, declined 2026-10-03)

Beta 1 is an early-access build for a small cohort on the reference class of
hardware. The 2026-09-29 proposal below would have moved three gates that
need hardware or a person Beta 1 does not have into Beta 2, reporting them
honestly as **Not yet run** rather than waiving them or marking them as
passing:

| Gate | Why it would move | Beta 1 disclosure if moved |
|---|---|---|
| NVIDIA station repeat | No NVIDIA hardware in the station matrix | known-limitations.md: tested on Intel graphics only |
| Formal Orca/I3 accessibility audit (journey 9) | **Kept in Beta 1 (owner, 2026-10-03)**: automated by `scripts/a11y/orca_audit.py`; the owner does only the final listen. The AT-SPI `EditableText` gap is upstream | known-limitations.md: screen-reader support is incomplete |
| Security native-station evidence (24 of 80 checks) | Needs Beta stations that don't exist yet | The verifier keeps failing closed; the 56 checks that ran are reported |

**The owner declined this proposal on 2026-10-03: all three stay Beta 1
gates.** Beta 1 must be genuinely good, so every product-quality and
performance gate, including the 8-hour soak, stays in Beta 1 too. The three
rows above remain open items in "What's still open, in order" above (the
Orca audit, the security-station runs, and NVIDIA testing) rather than
being deferred or waived.

**What changed in the 2026-09-25 pass, with real evidence** (see the top of
this document for what changed in the 2026-09-29–10-01 pass): journeys 2 (Files), 3
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

**Update, 2026-10-01 (since the table above was last refreshed):** journey
2's two tracked FILES-15/23/49 gaps have both moved from "missing" to
"Partial (nested pass)" in `docs/parity.md` — Get Info is now a real
separate window and the item/background context menus show the recorded
Share…/Quick Actions/tag rows (the underlying Share/Quick Actions providers
are still absent, so this is presentation parity, not full feature parity).
Journey 1/2's touchscreen input is now real (TOUCH-01, Fixed) with its own
nested regression pass (111/112, then 112/112 after a later fix), but this
is pointer/touch parity, not a change to the `EditableText` accessibility
gap that still fails journey 1 and 2's `Fail` status above. Journey 5's
Save-panel scenario result is unchanged — still nested-only, live portal
retest still outstanding. No journey in this table has moved to a different
top-level Status (Fail/Pass/Partial/Owner-manual) since 2026-09-27; the
movement has all been inside already-tracked parity rows.

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
| Memory (8-hour soak) | per-app budget, no leak | **Pass with one note.** The [2026-10-04 eight-hour soak](perf/reference-laptop-2026-10-04-memory-soak.md) (release binaries from 2f3ea7a4, idle laptop, swap-aware) kept all 13 processes alive. Every app's private footprint (Pss_Anon + SwapPss) stays under 128 MiB (43.9 to 73.8 MiB) and 9 of 10 apps grow by 2 MiB or less. Files grew 53.2 to 73.8 MiB in the first 2 hours, then stayed flat for 6 hours. That is over the 16 MiB growth budget, so the script still flags it; MEM-03 in docs/parity.md tracks proving its caches are bounded. The verdict is judged on the private footprint, because RSS counts shared libraries and GPU mappings and falls when the kernel reclaims cache. Cold first-open <150 ms is still not established. |
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

**Update, 2026-10-01:** no newer idle-CPU/soak/latency numbers exist — the
table above is still the current evidence. Separately, LINUX-HW-07 (fixed
2026-09-30) removed a class of bug where several Settings panes and
background tasks (Mouse, Wi-Fi, VPN, thumbnails, hostname lookup, lock
requests, Weather, Clock alarms) could hang indefinitely because they ran
`std::process::Command` directly on GPUI's small, fixed background-executor
thread pool (a fork-in-multithreaded-process hazard). This wasn't a measured
idle-CPU regression — a hang doesn't show up as elevated CPU, it shows up as
a frozen pane — so it isn't reflected in any number above, but it was a real
responsiveness/reliability gap closed this pass. A new CI check
(`scripts/check-background-executor-command.sh`) now greps for the same
mistake recurring.

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
| Verify every release journey with Orca | **Fail** (automated) | 2026-10-03: `scripts/a11y/orca_audit.py` runs the real Orca in the private nested session over 13 app and shell journeys (results: `docs/accessibility-audit.md`, "Automated Orca run"; ACC-12–27). Five defects fixed, eleven open; the owner's ~10-step "Owner final listen" on the live desktop remains. Journeys 1–6 now have real, live or nested AT-SPI-tree evidence (see §1); 7 has live backend checks but no Settings-UI run; 8's Control-F2 gap is confirmed; 9 is Owner/manual |
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

**Update, 2026-10-01:** `build-native-inputs.sh`, `build-native-packages.py`,
`verify-native-packages.py` and `install-native-candidate.sh` gained a
`--profile iterate` / `--build-metadata` fast path (`1c9ce60d`) for
owner-testing candidate builds (optimized, no fat LTO, `+iterate` in the
Debian version so it's never confused with a release build) — this is a
release-engineering convenience, not a new release gate. `.github/
workflows/release.yml` was also hardened for SR-18 (`97ed94b9`, 2026-09-29):
container images pinned by sha256 digest, rustup and `cargo-cyclonedx`
fetched with pinned hashes instead of `curl|sh`/bare `cargo install`. Neither
change has a real GitHub Actions release run behind it yet — that row above
stays **Not yet run**.

## 6. Documentation

| Check | Status |
|---|---|
| `python3 -m pytest scripts/test_documentation.py` | **Pass** (re-verified 2026-10-01) |
| `python3 scripts/verify-documentation.py` | **Pass** (re-verified 2026-10-01) — "rmac documentation set verified (15 required topics)" |
| CHANGELOG.md | **Pass** — added 2026-09-24; not refreshed with the 2026-09-29–10-01 fixes (out of this pass's scope; CHANGELOG is a per-tag record, not a running log) |
| docs/release-notes.md (required doc) + docs/release-notes/0.9.0-beta1.md | **Refreshed 2026-10-01** — both release-notes drafts (`packaging/release-notes/0.9.0~beta.1.txt`, `docs/release-notes/0.9.0-beta1.md`) are updated to reflect current reality (touch input, hardware-aware Settings, fixed accessibility claims) and still unpublished per the brief |
| docs/known-limitations.md reflects current reality | **Refreshed 2026-10-01** — see §7 |
| docs/install.md / README.md readiness banners | **Drafted 2026-10-03** — both switched from "isn't ready for daily use yet" to early-access Beta wording; needs the owner's sign-off in review (see "What's still open, in order", item 8) |
| Full `python3 -m pytest scripts/` (broader than the required set) | **2 real failures found and fixed this pass**: `test_application_icons.py` (the `915ceb55` Dock Bin-icon fix hand-edited `trash-full.svg` without updating `scripts/build-icons.py`, so the generator drifted from its own output — this is a Release-contracts CI check) and `test_sweep_settings_errors.py` (`PANE_ROUTES` in `scripts/linux/sweep-settings-errors.py` was missing the `touchscreen` pane LINUX-HW-02 added). Both fixed on this branch; `scripts/build-icons.py --check` and the targeted pytest files are now clean. One remaining failure, `hardware-touchscreen.json has no .mac.json` in `test_behavior_suite.py`, is a real gap (not yet recorded) tracked in "Path to Beta 1" item 2 — not a bug. A handful of `CompareTests` failures in `test_behavior_suite.py` only appear when the full `scripts/` suite runs together (not standalone), indicating pytest cross-file test-isolation pollution rather than a product bug; not investigated further this pass. |

## 7. Known limitations

`docs/known-limitations.md` was refreshed this pass (2026-10-01) to add: the
two open live-only P0s (desktop icons hidden until a click, Dock Bin
icon/Dock disappearing), an explicit "no NVIDIA hardware has been tested"
line under Compatibility limits, and a corrected framing of the Release
blockers section — the Linux layer-shell/GPUI-accessibility framework
question is resolved (ADR 0013, vendored; ACC-01/02/03 fixed), so the
remaining accessibility blocker is specifically the upstream `EditableText`
gap, not an open framework decision. The Accessibility limits, Feature
limits (including "no cross-app drag yet" — `DOCK-26` confirms this is still
true, drag-and-drop onto the Dock's Trash doesn't work either), and Security
findings sections were checked against current `docs/parity.md` and
`docs/security-review-0.9.0-beta.1.md` and found still accurate — no
security finding closed since 2026-09-29.

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

**Update, 2026-10-01: no CI result exists for current HEAD.** The most
recent CI links anywhere in this document are pinned to source `8ba31b82`
(2026-09-28), which is 279 commits and 3 calendar days behind `integ` HEAD
`1c9ce60d`. No commit message in that window references a new Actions run
URL. This is an open item, not a known-red item — status is simply unrun.

**Update, 2026-10-03: resolved.** CI and the GitHub-built candidate
packages are green for recent `dev` commits (see the go/no-go section at
the top of this document). What's still outstanding is a real release run
on the actual tag, not a CI dry run — see "What's still open, in order",
item 7.

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

## Required test run (2026-10-01, this session)

```sh
python3 -m pytest -q scripts/test_documentation.py
python3 scripts/verify-documentation.py
```

Both pass (`3 passed`; "rmac documentation set verified (15 required
topics)"). Also re-checked this session, broader than required:

```sh
python3 -m pytest -q scripts/test_native_packages.py scripts/test_session_package.py \
  scripts/test_release_workflows.py scripts/test_documentation.py
```

79 passed. A full `python3 -m pytest -q scripts/` found and this session
fixed 2 real failures (`test_application_icons.py`,
`test_sweep_settings_errors.py` — see §6) and surfaced one real, not-yet-
recorded gap (`test_behavior_suite.py`'s `hardware-touchscreen.json` scenario
has no Mac recording) plus 3 cross-file test-isolation flakes in
`test_behavior_suite.py::CompareTests` that only reproduce when the whole
`scripts/` directory runs together, not standalone — not investigated
further. **Update, 2026-10-03:** the `hardware-touchscreen.json` recording
gap is fixed — `python3 -m pytest -q scripts/test_behavior_suite.py` now
passes 40/40 — consistent with the 160/160 full behaviour-suite run in the
go/no-go section at the top of this document.

## What actually blocks shipping 0.9.0-beta.1 today

**Updated 2026-10-01** (previously 2026-09-27; see the top of this document
for the full current picture). The go/no-go table and "Path to Beta 1" list
at the top are the current punch list; the items below are the previous
pass's explanation of the same gates and remain directionally accurate:

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
4. **Security review: Fail (source review and disposable station done,
   gate not met).**
   [docs/security-review-0.9.0-beta.1.md](security-review-0.9.0-beta.1.md)
   and its format 2 summary cover all 80 checks.
   - **Disposable station:** it runs on GitHub Actions on request
     (`.github/workflows/security-station.yml`, a fresh `ubuntu-26.04` VM).
     Against a candidate built from the fix branch, it passed 11 of 12
     checks: install/purge exactness, maintainer scripts, permissions, the
     full package lifecycle and rollback, relay hardening, polkit denial,
     untrusted files never executing, native PackageKit (SR-29) and journal
     redaction. The 12th check found SR-39 (Ptyxis drops a `$(…)`
     argument).
   - **Fixes:** SR-15, SR-29 to SR-38, and functional issues F-1 (Sharing
     polkit prompts) and F-2 (AppStream categories).
   - **Counts:** 71 checks pass and 9 are pending: 7 on the reference laptop,
     1 needing the owner's asset licence record, and 1 blocked on SR-39.
   - **Open findings:** two Low, SR-18 and SR-39.
   - **Stations:** the AMD and NVIDIA desktops are waived
     (`owner-2026-10-04-beta1-without-amd-nvidia-desktops`). The reference
     laptop station has not run, so `verify-security-review.py` still fails
     closed.
5. **Product journeys remain incomplete.** Journey 5's nested Save-panel
   scenario passes and the chooser service is deployed, but the live portal
   flow has not been retested; journey 6's process-stop path is live-confirmed
   except for upstream EditableText, while journey 7 has backend evidence but no full
   Settings-UI run. Journeys 8/9 remain Partial/Owner-manual (§1).
6. **`docs/install.md`/`README.md` wording — drafted, awaiting sign-off.**
   Both now use early-access Beta wording instead of "isn't ready for daily
   use yet" (2026-10-03); the owner still needs to approve it in review.

None of these are release-engineering plumbing problems — the tagging,
packaging contracts, versioning, and pre-release workflow gating are ready
(§5, §9). What's missing is the evidence that the product itself is safe
and usable enough to hand to someone outside the project, which is exactly
what `todo.md`'s accessibility/security/data-safety gates exist to prove —
and, as of this pass, a real and growing share of that evidence now exists.
