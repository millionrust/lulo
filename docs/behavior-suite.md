# Behaviour-parity suite

## Interaction probes

`scripts/interaction/` is a sibling suite to `scripts/inventory` (which
diffs menu *item lists*) and the rest of this file (which diffs one app's
reaction to a fixed script): it diffs transient UI's *reaction to input* -
closing, hovering, switching, focus - which neither of those can see. It
exists because the owner found two gaps by hand that no menu-item inventory
could ever catch: top-bar menus and Control Centre not closing on an
outside click, and Control Centre's sliders not growing on hover.

`scripts/interaction/surfaces.py` declares each surface (a menu, a popover,
a context menu, …) and `probes.py` declares each probe (outside-click,
Escape, re-click the same title, switch to a neighbour, hover) with the
facts it records - booleans only, like this suite's own facts, never a
screenshot. `mac_probe.py` records the Mac side (holds `/tmp/mac-gui.lock`,
the coordinator's mkdir protocol, never flock, which this Mac has none of);
`lulo_probe.py` records the Lulo side, in nested niri with the shipped
`shell.kdl` for shell chrome (a top-bar menu, Control Centre) or plain
nested Sway for one app (reusing `run_lulo.py`'s `Nested`/isolation either
way). `diff.py` compares the two recordings per probe and writes
`docs/interaction-gaps.md`; a missing recording or an unmeasured fact is
reported as "not yet probed", never a silent pass or a fabricated gap.

```sh
python3 scripts/interaction/mac_probe.py --all              # the owner's Mac
python3 scripts/interaction/lulo_probe.py --bin-dir /usr/bin \
  --bin-dir /usr/libexec/rmac --shell-bin-dir /usr/bin --all  # the laptop
python3 scripts/interaction/diff.py
```

`--explore` on `lulo_probe.py` dumps the AT-SPI tree for a surface instead
of probing it (the Mac side has no such mode; its AX ground truth was read
live with short ad hoc AppleScript/JXA snippets under the GUI lock, the
same way `record_mac.py`'s own scenarios were written).

Three surfaces are recorded both ways as of 2026-10-02 (the Apple/Lulo menu
and Files' File menu in the top bar, and Control Centre), proving both
owner-reported gaps plus one more found along the way (see `docs/parity.md`
BAR-10, CC-14): both menus genuinely don't close on an outside click or
Escape, and the second click on an already-open menu title toggles it
closed on Lulo but is a no-op on the Mac. Control Centre's own
outside-click/Escape close reliably enough to measure (Escape was flaky:
~2 of 3 runs stayed open); its sliders' hover reaction could not be
measured on Lulo this pass - `rmac-quick-settings` exposes no AT-SPI
children for its content in the nested-shell harness even after a 25 s
wait, so those two facts are `null` rather than a guess. Everything else in
`surfaces.py`/`probes.py` (status menus, Spotlight, the Dock, Files'
context menu, dialogs, lists, tab focus, arrow keys, type-to-select,
press-and-hold) is declared for the matrix but has no driver yet.

## System Settings View menu

`scripts/behavior/run_lulo.py --check-settings-view-menu` launches one Settings
instance in its private compositor and activates Appearance, Wallpaper and
About through the published application-menu D-Bus endpoint. It checks that
each command opens the named pane; no input reaches the live session.

```sh
python3 scripts/behavior/run_lulo.py \
  --bin-dir ~/rmac-wt/target/iterate --check-settings-view-menu
```

## Terminal profiles

`docs/behavior-pending/terminal/profile-settings.json` describes the Settings
shortcut. It remains pending until a real Mac recording can provide its
`.mac.json` expectation. On Lulo, the private nested-compositor check opens
Settings, checks all 12 Mac profile names in the accessibility tree, then
selects Clear Dark and checks that the choice was saved:

```sh
python3 scripts/behavior/run_lulo.py --bin-dir ~/rmac-wt/target/iterate --check-terminal-profiles
```

## Spotlight latency

`scripts/behavior/run_cold_surfaces.py` is the private nested-niri Spotlight
performance scenario. It starts one resident launcher before dispatching
through niri and `rmac-shortcut-dispatch`, checks the first and later GPUI
frames against a 150 ms limit, and types `cal` through the nested virtual
keyboard. The echoed field and cached results must reach a frame within
50 ms of the input-change event. It also verifies the launcher stays resident
after dismissal and records its five-second memory sample. Other shell
surfaces retain their lifecycle checks.

```sh
python3 scripts/behavior/run_cold_surfaces.py \
  --bin-dir ~/rmac-wt/target/iterate \
  --output /tmp/lulo-cold-surfaces.json --resident-settle 5
```

The cross-platform JSON scenario harness has no Spotlight app target, and
its Mac recorder marks Spotlight input unsafe for the owner's live session;
this private runner covers the interaction and timing on Lulo.

Notes scenarios marked `"lulo_only": true` use a neighbouring `.lulo.json`
expectation because recording a new note on the owner's Mac could sync to
iCloud. `run_lulo.py` plays them only inside its private compositor and
temporary home, alongside the recorded Mac comparisons for other apps.

## Cross-app file drag

`scripts/behavior/run_file_drag.py` starts the shipped Dock and wallpaper in
private nested niri, opens one Files window, and drags two fixture files with
the virtual pointer: one to the Dock Bin and one to the Desktop. It checks
that each source disappeared and the corresponding Trash or Desktop item
appeared. The runner holds `/tmp/lulo-journey.lock` and uses disposable XDG
directories.

```sh
python3 scripts/behavior/run_file_drag.py --bin-dir ~/rmac-wt/target/iterate
```

## Menu dismissal on an outside click

`scripts/behavior/run_menu_dismiss.py` starts the shipped top-bar, Dock,
wallpaper and Control Center in a private nested niri and opens the Lulo
menu, a status menu, and Control Center in turn, then checks that each
closes on a real pointer click on the Dock, the wallpaper (inside and below
the bar's own `MENU_SURFACE_HEIGHT` band), another app's window (a dummy
`foot` window), and on Escape; it also checks that clicking a different
top-bar title switches menus instead of just closing, and that opening a
menu alongside Control Center and clicking the wallpaper closes both
(MENU-15, `docs/parity.md`). Control Center's dismissal is checked with
`grim` + a pixel-difference crop over its corner, since it is a layer-shell
popover with no niri "window" entry and no accessible control labels in this
build to search for by name. `--bin-dir` must hold `top-bar`, `dock`,
`wallpaper`, `rmac-quick-settings` and `rmac-shortcut-dispatch`.

```sh
python3 scripts/behavior/run_menu_dismiss.py \
  --niri /usr/bin/niri --bin-dir ~/rmac-wt/target/iterate
```

## Monkey testing

`scripts/behavior/monkey.py` is a seeded random ("monkey") tester, not a
fixed scenario: it drives one app, or the shell (Dock, menu bar, Spotlight,
Control Centre, Mission Control, no document window), with a long stream of
weighted-random actions — accessible-tree clicks, keyboard shortcuts drawn
from `tests/inventory/lulo/<App>.json`, typed text, window move/resize/
minimise/zoom/close/new, and file operations confined to a private sandbox
(text, a PDF, a PNG, folders, a long name, a unicode name, a 0-byte file and
a large file) — and watches for a crash, a hang, an error dialog, runaway
CPU, memory growth (`Pss_Anon+SwapPss`), a stuck window, and journal
warnings. On a finding it freezes the seed, the action log, the app's
stderr/stdout tail and a screenshot into `--findings-dir` (outside the repo
by default: never committed), verifies that the full log reproduces, then
binary-searches the shortest reproducing prefix against a fresh private
HOME. The report includes a working `--replay` command and optional
`--replay-count` for the prefix. Normal Quit and last-window close restart
the app and are recorded as replayable actions. Idle CPU sampling starts
15 seconds after launch so startup work is not classified as idle. It reuses
`run_window_move.py`'s nested Sway+niri+shell bootstrap and `run_lulo.py`'s AT-SPI helpers, and takes
`/tmp/lulo-journey.lock` itself like every other runner here.
Settings and System Monitor random clicks are confined to navigation; the
System Monitor's Quit Process shortcut is excluded. Terminal text omits
newlines, so random typing cannot submit a shell command. Print shortcuts
are skipped because the private headless compositor has no printer portal.

```sh
python3 scripts/behavior/monkey.py --bin-dir ~/lulo-monkey-bins \
  --niri /usr/bin/niri --app files --duration 1800 \
  --findings-dir ~/lulo-monkey-findings
```

`--seed` is printed if omitted, so any run can be reproduced. A triaged
finding gets a `BUG-*` row in `docs/parity.md`'s "Bugs found by monkey
testing" section; a fixed one gets a regression test in `tests/behavior` or
a unit test next to the fix, cited from that row.

## Parallel visual journeys

`tests/parallel/` contains ten first-hour journeys that use the same JSON
steps on macOS and Lulo. Each input step has a following `shot`. Actions are
`launch`, `click` (exact accessible name), `key`, `type`, `wait`, and
`drag_window`. `setup.files` and `setup.fixtures` create disposable content;
`$SANDBOX` in typed text resolves to that run's disposable folder. A shot
with `"scope": "full"` captures shell controls and menus. The Lulo runner
always uses headless Sway with nested niri, Dock and top bar, including for
ordinary app journeys.

```sh
python3 scripts/parallel/run_mac.py --output /tmp/rmac-parallel-mac
# On the laptop, from a worktree with the same commit:
python3 scripts/parallel/run_lulo_journey.py \
  --bin-dir ~/rmac-release/inputs-20260929T1945 \
  --output ~/rmac-coord/parallel-lulo
# After copying the laptop output to this Mac:
python3 scripts/parallel/compare.py --mac /tmp/rmac-parallel-mac \
  --lulo /tmp/rmac-parallel-lulo --output /tmp/rmac-parallel-compare
```

The Lulo runner holds `/tmp/lulo-journey.lock`; the Mac runner holds the
directory `/tmp/mac-gui.lock`. Both always release their lock in `finally`.

To benchmark Settings › Storage without reading the owner's home, run the
private nested runner against an installed or freshly built binary:

```sh
python3 scripts/behavior/run_lulo.py --bin-dir ~/rmac-wt/target/iterate --benchmark-storage 200000
```

It creates 200,000 empty files under a temporary `HOME/Documents`, keeps one
Settings instance open, and reports time to the capacity label, first
category, completion, and Settings process CPU time on first open, then
time to the category row and CPU use over two seconds on reopen. The runner
tracks the Refresh button's vertical position because the Storage card's
text is not exported through AT-SPI. It also samples Settings CPU use for one
idle second after five seconds of settling. It
removes the synthetic tree and temporary XDG directories when it exits.
Mac input is gated by the same AX ownership check as `record_mac.py`. Notes,
Dock/window management, status menus and Spotlight are marked Mac-unsafe in
their journey definitions; they run on Lulo only. Mac Terminal accepts only
`cd` into the sandbox, `echo` and `ls`. If an owner's Settings, Calculator or
Terminal is already running, the Mac runner stops that journey. The comparison
includes failures and skips, so a missing image never silently becomes a
parity pass. Text Editor Save is confirmed only after the sheet's Where
control names the disposable sandbox; Return in the sheet is otherwise
blocked.

Response timing samples target-region images after each input and reports
both first change and the last change followed by 300 ms of stability. The
Mac uses Quartz images for polling at a requested 45 Hz and `screencapture`
for final shots. Lulo uses a persistent wlroots screencopy connection for
timing and `grim` for final PNGs, both in its private compositor. Each result
records the achieved sample rate, which may be below the requested rate.
Outputs and screenshots belong only under `/tmp` or `~/rmac-coord`, outside
the repository.

Finds places where Lulo *behaves* differently from the Mac, without anyone testing by hand. A
scenario is data. The Mac recorder plays it on the owner's Mac and saves what macOS did. The Lulo
runner plays the same scenario inside a private nested compositor and diffs the two.

For a new Lulo behavior that cannot be recorded on the owner's busy Mac,
`<name>.lulo.json` supplies a local contract using the same observation
format. The runner prefers a `.mac.json` recording when one exists. The System
Monitor Find Next journey uses a local contract and runs in the full suite.

| Piece | Where | Runs on |
|---|---|---|
| Scenarios | `tests/behavior/<area>/<name>.json` | — |
| Mac expectations | `tests/behavior/<area>/<name>.mac.json` (words and numbers only) | written by the recorder |
| Lulo-only expectations | `tests/behavior/<area>/<name>.lulo.json` (for scenarios marked `lulo_only`) | recorded in the private nested compositor |
| Recorder | `scripts/behavior/record_mac.py` (+ `mac_observe.js`, `mac_click.py`) | the owner's Mac |
| Runner | `scripts/behavior/run_lulo.py` (+ `wlinput.py`) | the laptop, or CI's `behavior-parity` job |
| Comparator | `scripts/behavior/compare.py`, rules in `scripts/behavior/scenario.py` | anywhere |

For packaged apps without a recorded interaction scenario, run the separate
startup check on Lulo:

```sh
python3 scripts/linux/smoke-app-launches.py --bin-dir target/release \
  --output /tmp/lulo-app-smoke.json
```

It uses a private D-Bus session, headless Sway and disposable app data, then
reports whether each app launches and exposes an accessible surface. It does
not test interaction or visual parity; the JSON records a SHA-256 for each
tested binary so mixed dev builds are visible.

## Add a scenario

The folder-size Get Info scenario is staged at
`docs/behavior-pending/files/get-info-folder-size.json`. Its `info` fact checks
whether the focused Info window exposes a byte size and item count through
accessibility. Move it into `tests/behavior/files/`, then run the Mac recorder
and keep the recorded `.mac.json` alongside it. The 2026-09-28 Mac recording
attempt stopped before opening a scenario window because System Events returned
`-10827`; the behavior suite requires a real Mac recording, so this pending
scenario is not part of CI yet.

1. Write `tests/behavior/<area>/<name>.json`. `area` is `files`, `text-editor`, `settings`,
   `calculator`, `preview`, `notes` or `desktop`.

   ```json
   {
     "title": "New Folder (⇧⌘N) leaves the new folder's whole name selected for editing",
     "app": "files",
     "setup": {"files": {"report.txt": "text", "Projects/": null}},
     "launch": {"folder": "."},
     "steps": [
       {"key": "cmd-2"},
       {"key": "cmd-shift-n"},
       {"observe": "created", "facts": ["focus", "files"]}
     ]
   }
   ```

   - **setup**: `files` writes disposable content into the scenario's sandbox folder (as above);
     `config` writes a fixture under `XDG_CONFIG_HOME` (e.g. `{"rmac/weather.json": "..."}`);
     `state` writes one under `XDG_STATE_HOME` the same way, for apps that persist settings there
     (e.g. Files' sidebar favourites, `rmac/files/settings.json`) and whose preconditions a later
     scenario needs already set rather than toggled through the UI.
   - **launch**: `{"folder": "."}` or `{"reveal": "report.txt"}` for Files;
     `{"file": "guide.pdf"}` for Preview, with that file created in the
     scenario's sandbox. The other apps start with no arguments. Text Editor
     starts with one Untitled document.
   - **Steps**:
     - `key`: a chord like `cmd-shift-n` or `⇧⌘N`. ⌘ is Super on Lulo (ADR 0017), ⌥ is Alt,
       ⌃ is Control.
     - `type`: ASCII text.
     - `wait`: seconds.
     - `select` or `context`: click or right-click the item with that name. After `context: "background"`, `select` can activate a named context-menu item. For Files, `context: "background"` right-clicks an empty point in the list viewport.
     - `select` also takes `modifiers` (a list including `"shift"` and/or `"cmd"`) and `double` (bool) for a
       real shift-click, command-click or double-click, instead of the plain Finder "select" Apple Event
       (which only sets selection state and cannot extend a range, toggle an item, or open a folder). The
       Mac recorder locates the named item on screen and clicks it through Quartz, the same way `context`
       already does for right-clicks. The Lulo runner mirrors this: `run_lulo.py`'s `click_item` passes
       `count` (2 for `double`) and `modifiers` through to `wlinput.py`'s pointer `click(..., count=,
       modifiers=)`, which holds the virtual keyboard's shift/cmd key down around the pointer click. A
       plain `select` with neither field still sends exactly one unmodified left click.
     - `focus_desktop`.
     - `observe`: records facts.
     - Any step can take `settle` (seconds to wait after it; the default is 0.8).
     - `menu` (a menu-bar path) works on the Mac only for now: the nested runner has no top bar.
   - **facts**:
     - `focus`: the focused element's role, value, selection, selected_text and selected_all.
     - `windows`: count, front title and titles.
     - `dialog`: whether one is present, its title, texts, buttons in reading order and its default
       button.
     - `menu`: the open menu's items, with ✓ and [disabled].
     - `selection`: the selected item names.
     - `files`: the entries under the sandbox.
     - `tabs`.
     - `display`: Calculator.
   - **tolerance**: `{"<observation>.<fact>.<field>": rule}`. The rules are `exact`, `set`,
     `text` (normalizes quotes, ellipses and spacing), `role-class`, `subset`, `present`, `count`
     and `ignore`. The defaults are in `DEFAULT_RULES`. Pixel sizes are never recorded.
   - **omit**: fields the recorder must not save. Use it for anything that depends on the owner's
     machine, such as a new Finder window's home folder or the Go to Folder path. `menu_until`
     cuts a menu after an item, which drops the owner's own Services.
2. Record it on the Mac. The recorder holds `/tmp/lulo-mac-gui.lock` and uses a
   `/tmp/lulo-behavior/…/sandbox` folder:

   ```sh
   python3 scripts/behavior/record_mac.py files/new-folder --dry-run   # look first
   python3 scripts/behavior/record_mac.py files/new-folder             # writes .mac.json
   ```

3. Run it on Lulo. Do this on the laptop with binaries built from the
   branch under test. Build them with
   `cargo build --profile iterate --bins -p rmac-finder -p rmac-text-editor
   -p rmac-system-settings -p rmac-calculator -p rmac-preview
   -p rmac-file-chooser`, plus the shell's
   `wallpaper` binary. Don't use `--bin`: it restricts every `-p` to that one binary, which leaves
   the others stale. The runner acquires `/tmp/lulo-journey.lock` itself; do not
   wrap it in another lock on that file.

   ```sh
   python3 scripts/behavior/run_lulo.py --bin-dir $CARGO_TARGET_DIR/iterate \
     --shell-bin-dir $CARGO_TARGET_DIR/iterate --output /tmp/behavior.json files/new-folder
   python3 scripts/behavior/compare.py /tmp/behavior.json --emit-parity-rows
   ```

   `--explore --explore-steps N` prints the app's AT-SPI tree after the first N steps. Use it when
   a fact reads `nothing` on Lulo. `--emit-parity-rows` proposes `docs/parity.md` rows, which cite
   `behavior:<area>/<name>`. Copy the real ones into parity.md; the tool never edits it.

   For the Files context-menu flyouts, run the private interaction check against freshly built
   binaries. It opens View and Sort By, selects Columns and Size, then verifies that Size becomes
   the checked sort option:

   ```sh
   python3 scripts/behavior/run_lulo.py --bin-dir $CARGO_TARGET_DIR/iterate \
     --check-context-submenus
   ```

   The tag-swatch check needs `grim`. It verifies all seven visible colors and
   accessible names, then applies and removes a Red tag through the menu and
   checks the file's `user.rmac.tag` attribute:

   ```sh
   python3 scripts/behavior/run_lulo.py --bin-dir $CARGO_TARGET_DIR/iterate \
     --check-file-tag-swatches
   ```

## Safety

**Mac**

- Before every key press, the recorder checks two things: the frontmost app is the scenario's
  app, and the focused window is one the scenario opened. Otherwise it stops.
- It never quits an app that was already running. It closes only the windows and documents it
  opened, without saving.
- TextEdit autosaves an edited Untitled document to iCloud. The recorder moves only such copies,
  created during the run, to the Bin.
- The Desktop scenario needs Finder to have no open windows. It removes its folder with
  `rmdir`, which only removes an empty folder.
- Calculator must not be running when the scenario starts. System Settings is reused if it is
  open and left open.
- Nothing destructive outside the sandbox is ever confirmed.

**Lulo**

- `run_lulo.py` re-executes itself under `dbus-run-session` with a temporary HOME,
  XDG_RUNTIME_DIR and every XDG_* directory, and `GSETTINGS_BACKEND=memory`. Each scenario gets
  a fresh HOME.
- The private bus can start only three services: the AT-SPI bus, `xdg-desktop-portal`, and the
  `rmac-file-chooser` from `--bin-dir`, which is what opens Save panels. The installed rmac
  services never start. At the end the runner stops every process still using its runtime
  directory. `text-editor/save-untitled` now exercises the attached in-window sheet and needs
  only `rmac-text-editor`. The chooser binary is needed when a scenario opens the portal through
  Where ▸ Other…. On a cold D-Bus activation, the chooser can take several seconds to register
  its AT-SPI window.
- It starts its own headless Sway and holds `wayland-0`/`wayland-1`'s lock files so its socket
  is never named `wayland-1`.
- It injects input only through `wlinput.py`. That script is a pure-Python virtual keyboard and
  pointer, and it refuses `WAYLAND_DISPLAY=wayland-1`, any `/run/user/*` runtime directory, and
  any environment without `RMAC_BEHAVIOR_NESTED=1`.
- One app instance runs at a time. It is stopped by PID and waited for.

## Why headless Sway, not nested niri

Scenarios test app behaviour: focus, selection, dialogs, files. They do not test window
management. Sway's headless backend needs no GPU or seat, offers the wlroots virtual-keyboard and
virtual-pointer protocols this suite injects through, and is what CI's nested smoke test already
uses. niri has no headless backend and no virtual-input protocols. A scenario that needs
niri-specific behaviour should say so in its title and stay a live AT-SPI journey
(`docs/journey-suite.md`).

The one exception is minimising, which only niri can do (the parking workspace, the
`WindowMinimizeRequested` event, the ⌘M bind; ADR 0021). `scripts/behavior/run_niri_minimize.py`
runs niri *nested inside* the headless Sway, with the shipped `shell.kdl` and this branch's Dock
and Mission Control service. Keys go to Sway, because a virtual keyboard on niri itself bypasses
niri's binds, and niri reads Mod as Alt when it is nested. It uses `run_lulo.py`'s isolation, and
it checks a GTK window's own minimise, the Dock tile and restore, ⌘M, and the clean-up when a
parked window closes. Pass `--calculator` to include an rmac app. Checks 1 and 2 need Lulo's
patched niri (`--niri`, 26.04+lulo1-2).

```sh
python3 scripts/behavior/run_niri_minimize.py --niri ~/rmac-niri-build/target/release/niri \
  --bin-dir $CARGO_TARGET_DIR/iterate [--calculator $CARGO_TARGET_DIR/iterate/rmac-calculator]
```

Title-bar movement runs in nested niri as well, because Sway does not exercise GPUI's
`xdg_toplevel.move` requests. The runner uses the shipped `shell.kdl`, drags Calculator and
Settings plus a GTK window with the virtual pointer, and checks niri's reported positions. It
also captures Settings before any input and after two idle seconds, checks that its first-map
geometry contains no wallpaper band, and checks that the wallpaper and Dock repaint a preexisting
Desktop icon, file changes, and full/empty Bin states without input. It then opens, dismisses,
and reopens Quick Settings through its private shortcut socket, checking that the endpoint remains
owned after the popover closes. `--frame-only` runs those
capture checks at 1920×1080 without injecting any input; PNGs stay outside the repository with
`--keep`. `--geometry-only` captures Settings before and after its automatic screen-fit resize
at 1280×900, also without input. The runner also has an `assert_double_click_zoom` check for the default Zoom action (SET-33): the first
double-click should fill the working area without going under the Dock's exclusive zone, and a
second should restore the window's previous size. On the merged branch, three
clean nested runs each passed 15/18 checks: dragging and left-edge resizing
worked, but Settings stayed at its old frame after the double-click, so Zoom,
screen fit and Dock clearance failed. Instrumentation confirmed clicks 1 and 2
reach Settings' GPUI handler and its niri resize request returns success. The
same resize commands sent later from the runner do change the frame. SET-33
therefore remains partial; see the runner's `assert_double_click_zoom` note.
Floating edge resizing for other windows remains open in WIN-10:

```sh
python3 scripts/behavior/run_window_move.py --niri ~/rmac-niri-build/target/release/niri \
  --bin-dir $CARGO_TARGET_DIR/iterate
python3 scripts/behavior/run_window_move.py --bin-dir $CARGO_TARGET_DIR/iterate \
  --frame-only --keep
python3 scripts/behavior/run_window_move.py --bin-dir $CARGO_TARGET_DIR/iterate \
  --geometry-only --keep
```

`run_terminal_close.py` covers the red traffic light on a real `rmac-terminal`
window (the owner's report: it did nothing). It runs the same nested-niri
skeleton as `run_niri_minimize.py` so AT-SPI (`org.a11y.Bus`) and the shipped
`shell.kdl` are available, then drives the close button through AT-SPI's own
`Action.doAction` — no synthetic pointer/keyboard input into the app itself.
It checks: closing with no running foreground job closes the window with no
review (`RequestClose` reaching its handler regardless of what currently has
keyboard focus was the actual bug — see TERM-14/18, docs/parity.md); closing
with `sleep 30` running shows the "Do you want to terminate running processes
in this window?" review; Cancel leaves the window open and the job running;
a second close plus Terminate closes the window and ends the job.

```sh
python3 scripts/behavior/run_terminal_close.py --niri ~/rmac-niri-build/target/release/niri \
  --bin-dir $CARGO_TARGET_DIR/iterate
```

Pending, unrecorded scenarios live in `docs/behavior-pending/` until a Mac
recording exists: `settings/hardware-touchscreen.json`, which is Lulo-only
because no Mac has a touchscreen (run it with `run_lulo.py --explore`), and:

- `settings/storage-refresh.json`: System Settings nests Storage under
  General, not at the sidebar's top level, and the 2026-10-01 recording
  attempt found that General's own row list ("About", "Storage", …) exposes
  no AX-discoverable name at all through System Events on macOS 26.2 (no
  `AXTitle`, no `AXDescription`, no reachable `AXStaticText` child) — only
  measured pixel offsets would find "Storage", which this suite's
  accessible-name click (`record_mac.py`'s `select`) deliberately never
  does. Record it once a reliable accessible path to that row is found.
- `text-editor/save-untitled-other.json` and `save-panel-desktop.json`: both
  assume the Save sheet's "Where" pop-up has a "Documents" entry and an
  "Other…" item that opens the full browser. The 2026-10-01 recording
  attempt found that real macOS 26.2's "Where" pop-up has no "Other…" row,
  and reaches the full browser through a disclosure triangle instead
  (`AXDescription` "show more options", not "Show Details") — recorded as
  OTHER-14. Lulo's `save_sheet.rs` now matches: the Where pop-up's
  "Other…" item is gone, and a `save-sheet-where-disclosure` button next to
  it (aria-label "Show More Options") opens the same full-chooser flow.
  Both `.json` steps still select the old "Other…"/"Show Details" controls
  and so need re-recording against the new control on each platform before
  they can move out of `docs/behavior-pending/`; that recording pass still
  needs a real Mac (for `save-untitled-other.json`) or the laptop's own
  recorder (for `save-panel-desktop.json`), neither of which this pass
  drives.
- `files/drag-to-dock-and-desktop.json`: `scripts/behavior/record_mac.py`
  and `mac_click.py` only ever post a click (mouse-down immediately followed
  by mouse-up); neither has a drag primitive (mouse-down, several
  mouse-dragged moves, mouse-up), so this scenario cannot be recorded until
  one is added. Driving a real drag-and-drop blind on the owner's live Mac
  (Dock bin, Desktop) without that primitive is not attempted here.

Scenarios whose expected results were hand-written task contracts, not Mac
recordings, were kept out of `tests/behavior/` so that every enforced
expectation comes from a real Mac. `text-editor/close-unsaved-delete`,
`close-unsaved-save`, `open-panel-desktop`, `save-go-to-folder` and
`save-with-find-focus` are now recorded (their `.contract.json` files are
gone); `save-untitled-other` and `save-panel-desktop` remain in
`docs/behavior-pending/` with their `.contract.json`, for the reason above.
To promote a pending scenario, record it with
`scripts/behavior/record_mac.py` and move it back.
