# Beta release checklist — 0.9.0-beta.1

## Beta 1 go/no-go (updated 2026-09-25, evening pass)

**Not ready today.** The three worst accessibility bugs (Terminal/Notes/Files
having no accessible content at all) are now genuinely fixed and
live-reconfirmed against a fresh `dev` build on the reference laptop this
pass — that is real progress, not a projection. What is still open:

| # | Blocker | Owner/agent | Size |
|---|---|---|---|
| 1 | CI is red on `dev` (run [36151133746](https://github.com/millionrust/lulo/actions/runs/36151133746)): a macOS-only compile error in `rmac-updates-linux` (`FLAG_ONLY_DOWNLOAD`/`FLAG_ONLY_TRUSTED` are `#[cfg(target_os = "linux")]`-gated but used unconditionally in `crates/rmac-updates-linux/src/transaction.rs:14`) and a `-D warnings` dead-code failure in `crates/preview/src/view.rs:319` (`window_generation`, `print_busy` unused under `--all-targets`). Two other causes (a stale `rustfmt` diff and a missing Dock icon in the test whitelist) are fixed in this pass's commits. | agent (needs cargo + macOS to validate) | S–M |
| 2 | Journey 5 (Text Editor via the portal): Save on an Untitled document still doesn't present a proper Save panel (`panel.dialog.present` false, no Cancel/Save buttons) — confirmed this pass with a fresh nested run, 3/4 scenarios pass. `rmac-file-chooser` is still not deployed on the reference laptop's live session either. | agent | M |
| 3 | Journey 6 (System Monitor): the shared `rmac_ui::Table` viewport bound (ACC-10) is fixed and live-confirmed 2026-09-26 — off-screen rows now get a synthetic AccessKit node, and the full accept flow reaches select → Quit → confirmation dialog → Cancel → process survives (8/10 steps pass; `run-journey-monitor.py`'s stale "Process menu" name was also fixed to the real "View menu"). Two gaps remain: the search field still has no AT-SPI `EditableText` (pinned `accesskit_unix` gap, not fixable here), and a newly found, unrelated bug (MON-15, P0): re-opening View ▸ Quit Process after Cancel does not reopen the confirmation dialog, so the disposable process is never actually terminated — reproduced in two independent live runs, root cause not found (see `docs/parity.md` MON-15). | agent | S–M |
| 4 | Journey 8 (keyboard only): Control-F2 (menu-bar keyboard focus) is now implemented (`6f38352e`) — bind, dispatch endpoint, invisible focus surface, title highlight/open/Esc/letter-jump, AT-SPI focus for the highlight. Compiles, passes `clippy -D warnings`, `niri validate`, and new unit tests. Not yet confirmed live or with a real keypress — needs either a visible nested-niri window on the reference laptop or the owner's Mac with Full Keyboard Access on to record a behaviour-suite scenario; this pass did neither rather than disturb the owner's live session or Mac settings unasked. | agent (S, live/nested confirmation only) | S |
| 5 | Journey 9 (Orca at 200%): no formal Orca pass exists; this pass only re-confirmed the AT-SPI tree stays intact at compositor scale 2 in the nested runner. A real Orca run needs the owner (screen reader must not be enabled by an agent). | **owner** | — |
| 6 | Accessibility: `accesskit_unix` still has no `EditableText` implementation at all (upstream), so no AT-SPI client can type without a physical/virtual keyboard, in Terminal, Notes, Files' search/rename fields, or Spotlight. Not fixable in this repo. | upstream | — |
| 7 | No real packaging install/remove/upgrade run on a clean VM or the laptop, and no exercised GitHub Actions release run. Not re-verified this pass (out of scope of this pass's evidence-gathering; still exactly as `docs/release-process.md` describes). | agent | L |
| 8 | Security review: 3 open findings, all Low and accepted for Beta with a documented risk and mitigation (SR-15, SR-18, SR-29; see `docs/security-review-0.9.0-beta.1.md` "Beta decision" and `docs/known-limitations.md`). Nothing Critical, High or Medium is open; SR-17 and the new SR-28 were fixed on 2026-09-25. 24 of 80 checks still need native-station evidence and no Beta station has run, so `verify-security-review.py` still fails closed. | agent (stations: owner) | L |
| 9 | `docs/install.md`/`README.md` still say the product isn't ready to install. | **owner** | S |

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
| 4 | Notes: create, search, edit, recover after crash | `scripts/linux/run-journey-notes.py`, `scripts/assert_notes_accessibility.py` | **Re-run 2026-09-25.** `run-content-accessibility.sh ... notes` exits 0 live: "2 folder items (1 selected), 1 note items (1 selected), named Search/Title/Body/Tags entries with Text, Title focus via `grabFocus`, caret 8" — all against the real Notes library's temp-XDG-isolated copy, never the owner's real data. A nested probe additionally confirmed the folder ("Folders") and note ("Notes") list boxes carry named `list item`s ("All Notes, 0 notes", "Recently Deleted, 0 notes") and the Search field's AT-SPI `Text` interface now works (`queryText()` returns a real value instead of raising). Note-content typed-round-trip (title/body) was not independently re-confirmed this pass — the toolbar's icon-only buttons (New Note, etc.) are still unnamed over AT-SPI (ACC-08, tracked separately), which blocked identifying "New Note" reliably in the nested probe. | **Fail** — ACC-02 (no text entry surface, no note list/sidebar) is fixed and live-reconfirmed; the remaining gaps are the same upstream `EditableText` limitation as journey 1, plus ACC-08 (unnamed toolbar buttons, tracked separately) |
| 5 | Open/edit/save a text file through the portal | `scripts/linux/run-journey-textfile.py`, `scripts/behavior/run_lulo.py text-editor/*` | **Re-run 2026-09-25** in the nested compositor against a fresh `rmac-text-editor` build: 3/4 scenarios pass (`find`, `new-document`, `close-unsaved`); `save-untitled` still fails — the Save panel doesn't present properly (`panel.dialog.present` false, no Cancel/Save buttons, and the focused field still shows the typed body text instead of the filename with it selected). `rmac-file-chooser` remains undeployed on the reference laptop's live session (`systemctl --user status rmac-file-chooser.service`: unit does not exist), so the portal-driven live run still falls back to `fallback_spawn` for Open/Save As, per `docs/journey-suite.md`. | **Fail** — Save-panel-on-Untitled is a real, reconfirmed functional bug (not just an accessibility gap), plus the file-chooser deployment gap |
| 6 | Inspect resource use, safely stop a process | `scripts/linux/run-journey-monitor.py` | **Re-run 2026-09-26** live against a fresh `rmac-system-monitor` build (`5decc694`, branch `table-a11y`), its own disposable `sleep` process, one instance, temp desktop entry removed afterward, ~/rmac-*-wt worktree removed afterward: 8/10 steps pass. The `rmac_ui::Table` viewport-bound blocker (ACC-10 in `docs/parity.md`) is fixed: off-screen rows now get a synthetic AccessKit node with the same `"{name} (PID {pid})"` identity, row index/count and selected state as a painted row, and a Click/Focus action that scrolls the row into view and selects it. The accept flow now reaches select → STATE_SELECTED verified → Quit → confirmation dialog appears → Cancel → process survives. `quit_controls_exist` also now passes: a live AT-SPI dump found Quit Process/Force Quit Process… live in the top bar’s **View** menu, not a "Process" menu — `run-journey-monitor.py` was checking the wrong menu name, fixed in the same commit. Two gaps remain: the search field still has no AT-SPI `EditableText` (pinned `accesskit_unix` gap, not fixable here), and a newly found, unrelated P0 bug (MON-15 in `docs/parity.md`): re-opening View ▸ Quit Process after Cancel does not reopen the confirmation dialog, so the disposable process was never actually terminated in either of two independent live runs — root cause not found; not caused by the Table fix (only reachable because of it). | **Fail** (2 gaps; was 4) — the process-table viewport blocker and the Process/View menu-name issue are both resolved and live-confirmed; MON-15 (Quit does not survive a Cancel-then-retry) now blocks the accept flow’s last step |
| 7 | Wi-Fi, Bluetooth, audio output, battery | none (code trace + read-only live checks) | `docs/journey-7-trace.md`'s source trace stands. This pass added **read-only live queries** on the laptop's real services (no toggling, no connecting): `nmcli device status`/`nmcli radio` show Wi-Fi connected and radios enabled; `bluetoothctl show` shows the controller powered on with real UUIDs; `wpctl status` shows PipeWire's real Analog Stereo sink/source (the laptop has no `pactl`/`pipewire-pulse` compat layer active, so Settings' audio backend must be the native PipeWire path, not a PulseAudio shim — matches `docs/settings-backend-audit.md`); `upower -i` on `battery_BAT0` returns real battery state (95%, fully-charged, real voltage/energy). All four backends are real, live, queryable services, not stubs. | **Not yet run** as a full live Settings-UI acceptance test — the backends themselves are confirmed real and live this pass |
| 8 | Journeys 1–7, keyboard only | none | No dedicated script. The Dock is keyboard-reachable via Control-F3; **Control-F2 (menu bar keyboard focus) is now implemented** (`6f38352e`): niri's `Ctrl+F2` bind spawns `rmac-shortcut-dispatch menu-bar-focus`, the same command-endpoint mechanism as the power key, to a socket the menu bar watches; the bar takes the keyboard through its own invisible 1×1 overlay surface (`MenuKeyboard`), the same technique ⌃F3 already uses for the Dock, since the bar's own layer surface only takes keyboard on a click. Left/Right moves the highlighted title, Down/Return opens it, Esc backs out one level at a time, and typing a letter jumps to a title; `MenuKeyboard`'s single accessible node carries AccessKit focus for the highlighted title and, once a menu opens, the highlighted row. Verified this pass by compiling, `cargo clippy -p rmac-shell-menubar --features wayland -- -D warnings`, `niri validate` on `packaging/rmac-session/shell.kdl`, and new unit tests (`rmac-shortcuts`: 34/34 pass; `rmac-session`: 26/26 pass, both including new ⌃F2-specific tests) — **not yet verified live or with a real keypress**: doing that needs either a visible nested-niri window on the reference laptop's actual screen (niri has no headless backend to test a keybinding plus layer-shell keyboard-interactivity in isolation the way the Sway-headless behaviour suite tests apps) or toggling Full Keyboard Access on the owner's own Mac to record `tests/behavior/keyboard/ctrl-f2.json`; this pass avoided both rather than disturb the owner's live session or change their physical Mac's system settings without asking first. This pass's nested behaviour-suite reruns used only the isolated virtual keyboard (no mouse for typing/shortcut steps) and confirmed arrow-key list navigation (`list-arrows`), Return-to-rename, and ⌘-shortcut dispatch (Select All, Undo, New Folder, New Tab) all work by keyboard once an app has focus. | **Fail** — Control-F2 is implemented but not yet live/Orca-verified; within-app keyboard operability is well evidenced; reaching the menu bar by keyboard is code-complete and compile/unit-verified but needs a live or nested confirmation pass before this journey can close |
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
| Warm launch to interactive | p95 ≤ 500 ms (900 ms Files/Terminal) | **Measured under build contention** — Notes, System Monitor, System Settings over; re-run with `rustc` at 0 |
| Idle CPU | ≤ 0.3%/app, ≤ 1% shell combined | **Re-measured 2026-09-25** (read-only, `top -b -d 10 -n 6`, 60 s, on the live session's real shell processes): top bar averaged 0.27% (samples 0.0–0.6%), Dock 0.0–0.1%, wallpaper 0.0–0.3%, OSD/launcher/Mission Control/screenshot/shortcut-broker/focus-service/clipboard/file-chooser all 0.0% — combined well under the 1% shell budget, a large improvement on 2026-09-24's 2.78%. This held even while another build was saturating the laptop (load average ~4, 98% system CPU) — a harder test than idle. Per-app numbers (System Settings 24.95%, Files 5.32%, System Monitor 3.13%, Clock 1.58% from 2026-09-24) were **not re-measured this pass** — avoided launching extra app instances on a laptop shared with other agents; still needs a clean re-run |
| Idle wake-ups | none while nothing changes | **Not re-measured this pass** (still the 2026-09-24 number: ~4/s per visible layer window, top bar 24/s, shortcut broker 23/s) — the near-zero CPU result above is consistent with a fix, but a wake-up count needs `strace -c -e sendmsg`, not just `top`, and the laptop was busy with a concurrent build for the whole window this pass had available |
| Input to visible response | p95 ≤ 50 ms | **Not yet run** — no frame-timing harness exists yet |
| 60/120 Hz animation frame budget | ≥ 99% / ≥ 95% within budget | **Not yet run** — `docs/performance-baseline.md` notes no per-frame trace is available yet |
| Memory (8-hour soak) | per-app budget, no leak | **Not yet run** |
| Repeat on an NVIDIA system | required before Beta | **Not yet run** — no NVIDIA station in the matrix yet |

Evidence: [docs/perf/reference-laptop-2026-09-24.md](perf/reference-laptop-2026-09-24.md)
is the first real run (system audit, 2026-09-24); see
[docs/system-audit-2026-09-24.md](system-audit-2026-09-24.md). This pass adds
a 2026-09-25 read-only re-measurement of shell idle CPU only (see the Idle
CPU row above) — command: `top -b -d 10 -n 6 -p <shell PIDs>` against the
live session's own `rmac-dock`/`rmac-top-bar`/`rmac-wallpaper`/etc. processes,
no changes made. **Status: Fail** — idle wake-ups and per-app idle CPU are
unconfirmed this pass and were last measured over budget; shell idle CPU
itself now measures well under budget.

## 3. Accessibility gates (todo.md)

| Gate | Status | Evidence |
|---|---|---|
| Stable identity/role/name/state/actions on every `rmac-ui` control | **Fail** (partial) | `docs/accessibility-audit.md`: Toggle/Checkbox/Radio/dialogs/menus/toasts/traffic lights fixed this pass; `PopUpButton`, `ContextMenu` items, `ListRow`/`TreeRow` still wrap `gpui_component::Button` and are stuck at `Role::Button` until either an upstream `gpui-component`/`gpui-kit` change or a rewrite off `Button` (Blocked, not Gap, per that doc) |
| Correct tab order, visible focus ring | **Fail** (partial) | Same doc: a shared 3pt focus ring now exists and is used on the rewritten controls; everything still wrapping `Button` keeps a hardcoded 1.5px ring it cannot override (Blocked) |
| Full keyboard operation, no pointer-only controls | **Fail** (much improved) | Terminal, Notes and Files' content surfaces were the sharpest evidence of this gate's worst failures — as of 2026-09-25 those are fixed and live-reconfirmed (ACC-01/02/03; journeys 2–4 above). The gate still fails: Control-F2 (menu-bar keyboard focus, journey 8), `accesskit_unix`'s missing `EditableText` (journeys 1–4), Notes' unnamed toolbar buttons (ACC-08), and `PopUpButton`/`ContextMenu`/`ListRow`/`TreeRow` stuck at `Role::Button` (Blocked, row above) |
| Announcements for async status/errors | **Fail** (partial) | Toast fixed this pass; `EmptyState`'s error variant and list Loading/Empty/Error messages still have no role (Gap) |
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
Notes and Files' search/rename fields identically), Control-F2, the formal
Orca/I3 audit, and the `Role::Button`-wrapped controls still Blocked on an
upstream/rewrite decision.

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

## 8. CI

**Checked directly this pass** via `ssh jacob@192.168.18.52 'gh run list -R
millionrust/lulo --branch dev --limit 3'` (the laptop has `gh` auth). The
`dev` HEAD at the start of this pass (`3ca78f24`, run
[36151133746](https://github.com/millionrust/lulo/actions/runs/36151133746))
is **red**:

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
`rustfmt`/`pytest` too. The two macOS compile errors are real, current, and
**not fixed** — they need an agent with cargo and a macOS build target.
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

Updated 2026-09-25 (evening pass) with real evidence gathered on the
reference laptop this session — see the go/no-go table at the top of this
document for the current, short punch list. In order of severity:

1. **CI is red on `dev`.** Two of the four causes found this pass are fixed
   in this pass's commits (a stale `rustfmt` diff, a missing Dock icon in
   the test inventory whitelist); two macOS-only compile errors remain open
   and need an agent with cargo and a macOS target (§8).
2. **Accessibility (never waived) — much improved, not clear.** Terminal,
   Notes and Files no longer have the "zero accessible content" bugs that
   made this the top blocker as of 2026-09-24: ACC-01/02/03 are fixed and
   live-reconfirmed on the reference laptop this pass (§1, §3). What remains
   release-blocking: the upstream `accesskit_unix` `EditableText` gap (not
   fixable here), Control-F2 (journey 8), the formal Orca/I3 audit (journey
   9, owner-only), and Notes' unnamed toolbar buttons (ACC-08).
3. **No real packaging install/remove/upgrade run** on a clean VM or the
   reference laptop, and no exercised GitHub Actions release run — not
   re-verified this pass; the pipeline is well-gated and unit-tested but has
   never produced a real `.deb` on real CI infrastructure.
4. **Performance: partially re-measured.** Shell idle CPU is now confirmed
   well under budget on the live laptop (§2), even under concurrent-build
   load. Idle wake-ups and per-app idle CPU (System Settings, Files, System
   Monitor, Clock) were not re-measured this pass and were last recorded
   over budget.
5. **Security review: Fail (source review done, gate not met).**
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
6. **Journey 5 has a real, reconfirmed functional bug** (Save-panel-on-
   Untitled doesn't present properly); journey 6's headline AT-SPI blackout
   is fixed but its full accept flow is unexercised; journey 7 has live
   backend evidence but no Settings-UI run; journeys 8/9 remain Fail/
   Owner-manual (§1).
7. **`docs/install.md`/`README.md` still say the product isn't ready to
   install** — an owner-level messaging decision, not a mechanical fix.

None of these are release-engineering plumbing problems — the tagging,
packaging contracts, versioning, and pre-release workflow gating are ready
(§5, §9). What's missing is the evidence that the product itself is safe
and usable enough to hand to someone outside the project, which is exactly
what `todo.md`'s accessibility/security/data-safety gates exist to prove —
and, as of this pass, a real and growing share of that evidence now exists.
