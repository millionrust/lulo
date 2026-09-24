# Automated product journeys

I1 maps the ten product journeys in `GOAL.md` to package-scoped fixture suites.
The exact map is `scripts/journey-suite.json`; the runner never expands it to a
workspace-wide or all-features Cargo command.

List the journeys without building:

```sh
python3 scripts/run-journey-suite.py --list
```

Run one journey:

```sh
python3 scripts/run-journey-suite.py \
  --journey 5 \
  --output /absolute/path/journey-results.json
```

Run all ten sequentially and retain completed passes across an interrupted
invocation:

```sh
python3 scripts/run-journey-suite.py \
  --output /absolute/path/journey-results.json \
  --resume
```

The runner requires a clean tracked worktree and 25 GiB free, executes one
command at a time in the normal target directory, uses `--locked`, publishes a
bounded privacy-safe result after every journey, and never stores command
output, paths, environment values or fixture data. A failed command marks only
its journey failed; `--fail-fast` stops immediately when that is preferable.

The package tests cover domain behavior, failure/recovery, persistence,
keyboard reducers and fake service states. They do not prove Wayland
placement, real services, hardware, Orca, IME, scaling or end-to-end visual
behavior. Those remain A, H8 and I2–I5 evidence gates. I1 is accepted only when
all fixture suites pass on the exact clean Linux candidate and their coverage
is paired with the required reference-hardware journeys rather than presented
as a replacement for them.

## Live reference-hardware acceptance tests

`scripts/linux/run-journey-launch.py` exercises journey 1 ("Log in, launch an
app from the Dock or Spotlight, switch apps, and close it") against the real
session on the reference laptop, over AT-SPI (`pyatspi`) and niri IPC only —
there is no keyboard or pointer injector installed there. Copy it to the
laptop and run it in the live session:

```sh
scp scripts/linux/run-journey-launch.py jacob@<reference-pc>:/tmp/
ssh jacob@<reference-pc> \
  'python3 /tmp/run-journey-launch.py --output /tmp/journey-launch-report.json'
```

It publishes a privacy-safe JSON report (no screenshots, no window titles,
no paths under a home directory) with one entry per step — session liveness,
Dock launch, Spotlight launch, window appearance, focus, app switching by
`niri msg action focus-window`, and closing through each app's own top-bar
Quit menu item — plus a `gaps` list. Each launched app's `performance` entry
reports both `mapped_ms` (when niri considers the window present) and
`interactive_ms` (when the app's own `RMAC_BENCHMARK_READY_FILE` marker says
its real content, not a loading placeholder, is on screen — see
`crates/rmac-ui`'s `mark_content_ready`), and evaluates todo.md's warm
launch-to-interactive budget (p95 ≤ 500 ms for simple apps) against
`interactive_ms` whenever it is available. A real Dock/Spotlight activation
cannot be instrumented with that env var, so `interactive_ms` is `null` for
an `accessible_ui` launch and the verdict falls back to `mapped_ms`; a
`fallback_spawn` launch (used whenever the Dock/Spotlight accessible actions
above aren't available) gets both. "Log in" checks
that a real graphical rmac session is already active rather than performing a
full logout/login, so the shared reference session is never disturbed; the
destructive full GDM login/logout journey is
[`docs/session-journey-evidence.md`](session-journey-evidence.md).

Both surfaces had real accessibility gaps against todo.md's "no pointer-only
controls" gate, tracked as the `dock_launch` and `spotlight_launch` steps:

* Dock icons exposed only the AT-SPI Accessible/Component interfaces (no
  `click` action) -- fixed: each launchable tile now wires AccessKit's Click
  action to the same activation path a mouse click uses.
* Spotlight's search field exposed neither `Text` nor `EditableText` -- partly
  fixed: it now reports a real Entry role with its value over `Text`, and
  each result row is a real Button, but the pinned `accesskit_unix` AT-SPI
  bridge does not implement `org.a11y.atspi.EditableText` at all, so a query
  still cannot be typed without a keyboard injector (an upstream dependency
  gap, not an rmac one).

The script always drives both surfaces through live AT-SPI introspection, so
it reports exactly what the deployed binary exposes rather than assuming a
fix is live; until the fixed binaries are deployed to the reference laptop,
both steps still fail there. Whenever either step fails, the script then
launches the target application directly (the same installed command a Dock
or Spotlight activation would run), clearly labeled `fallback_spawn` in the
report, so the rest of the journey is still measured.

`scripts/test_journey_launch.py` unit-tests the script's pure JSON-parsing,
environment-discovery, and report-building logic and runs anywhere with
`python3 -m pytest scripts/test_journey_launch.py` (no pyatspi, niri, or live
session required); it does not exercise the live AT-SPI/niri orchestration,
which only runs on the reference laptop.

`scripts/linux/run-journey-files.py` exercises journey 2 ("Find a file,
preview it, copy, move, rename and trash it, and undo a destructive
operation") against Files (`crates/finder`, binary `rmac-files`) on the
reference laptop, the same way run-journey-launch.py exercises journey 1:
AT-SPI (`pyatspi`) and niri IPC only, no keyboard or pointer injector. It
creates a disposable folder under `~/Documents/lulo-journey-2-<random>`,
never touches anything outside it, and always removes it (and any Trash
entries it created) even on failure:

```sh
scp scripts/linux/run-journey-files.py jacob@<reference-pc>:/tmp/
ssh jacob@<reference-pc> \
  'python3 /tmp/run-journey-files.py --output /tmp/journey-files-report.json'
```

Because other agents may also be driving the reference laptop's screen, wrap
the run in the shared lock so only one UI-driving run happens at a time:

```sh
ssh jacob@<reference-pc> \
  'exec 9>/tmp/lulo-journey.lock; flock -w 900 9 && \
   python3 /tmp/run-journey-files.py --output /tmp/journey-files-report.json'
```

The last real run against the reference laptop (2026-09-24) found Files
considerably less accessible than journey 1's Dock/Spotlight surfaces:

* **No accessible name anywhere in a Files window itself.** Every toolbar
  button, the search/path entry, and the icon-size slider are exposed over
  AT-SPI with an empty name (`grep -rn "aria_label\|\.role(\|
  on_a11y_action" crates/finder/src` returns nothing, unlike
  `shell/bins/rmac-dock/src/main.rs`, which uses that exact API for its own
  tiles). Worse, **no file, folder, or sidebar row is exposed at all** -- a
  Files window's AT-SPI frame has only its toolbar controls as children.
  This blocks "find a file" (`find_file` step) and any per-item selection
  entirely; the script falls back to whole-folder Edit > Select All against
  a single-item folder to get a definite target for the rest of the
  journey.
* The shell top bar's per-app menu for Files *is* fully accessible --
  "File menu" (New Folder, New Tab, Close Tab, Move to Trash, Get Info),
  "Edit menu" (Undo, Cut, Copy, Paste, Select All), "View menu" (view
  modes, sort, Show Hidden Files, Quick Look), "Go menu" (Back, Forward,
  Enclosing Folder, Home, Applications, Downloads, Trash) -- but there is
  no "Rename" item anywhere, and no "Quit Files" item in "Files menu"
  either (unlike journey 1's Text Editor/Notes). Renaming is otherwise
  inline-only (`crates/finder/src/view/rename_controller.rs`), needing both
  row-level selection (unavailable) and a Return keypress (no keyboard
  injector); `File > Get Info`'s dialog also exposes no editable content
  over AT-SPI, only a Close button. The `rename` step fails for real, live,
  every run.
* Selection-scoped top-bar menu commands (Select All, Move to Trash, Undo,
  Quick Look) silently no-op -- no error, no dialog, nothing on disk --
  unless the target Files window's AT-SPI frame is given focus first via
  the `Component` interface's `grabFocus()`; the script always does this
  immediately before such a click.
* `View > Quick Look` does not open any visible window or layer-shell
  surface on this build even with a real selection and a focused window
  (`crates/finder/src/view/quick_look_controller/controller.rs:33`); the
  `preview` step fails for real.
* **Copy and Move do not work on Linux at all.** Both menu actions are
  reached and clicked correctly, but nothing is ever pasted:
  `crates/finder/src/pasteboard.rs`'s `#[cfg(not(target_os = "macos"))]`
  module (lines 62-70) stubs `write_file_urls`, `read_file_urls`, and
  `clear_file_urls` to complete no-ops, so Copy never writes anything a
  Paste could read back, on this window or any other. This is a real
  product gap, not flaky automation -- the `copy`/`move` steps fail for
  real, every run, once the script deliberately never keeps two Files
  windows open at the same time (see below).
* Two simultaneously open `rmac-files` processes were observed to register
  only one `org.rmac.Files.Menu` D-Bus name between them (`dbus-send
  ... org.freedesktop.DBus.ListNames` shows a single entry with two Files
  windows open), which can make it unpredictable which window a top-bar
  menu click actually reaches. The script therefore always fully closes a
  source Files window before opening a destination one, rather than
  keeping both open.
* Trash and Undo work correctly once a window has AT-SPI focus: `File >
  Move to Trash` moves the file for real (verified against
  `~/.local/share/Trash/{files,info}`, matched to this run by the
  `.trashinfo`'s `Path=` line rather than by name, so it never confuses
  another user's or run's Trash entries with this one), and `Edit > Undo`
  restores it and removes the matching Trash entry. These two steps pass
  reliably.

`scripts/test_journey_files.py` unit-tests the script's pure JSON-parsing,
environment-discovery, Trash-entry-matching, and report-building logic and
runs anywhere with `python3 -m pytest scripts/test_journey_files.py` (no
pyatspi, niri, or live session required); it does not exercise the live
AT-SPI/niri orchestration, which only runs on the reference laptop.

### Journey 3 — Terminal

`scripts/linux/run-journey-terminal.py` exercises journey 3 ("Open Terminal,
run a command, scroll, select, copy and paste, and manage tabs";
`crates/terminal`, package `rmac-terminal`) the same way, over AT-SPI and
niri IPC only:

```sh
scp scripts/linux/run-journey-terminal.py jacob@<reference-pc>:/tmp/
ssh jacob@<reference-pc> \
  'python3 /tmp/run-journey-terminal.py --output /tmp/journey-terminal-report.json'
```

The top bar's per-app menus for Terminal (AT-SPI application `rmac-top-bar`:
"Terminal menu", "Shell menu" for New/Close/Next/Previous Tab, "Edit menu"
for Copy/Paste/Select All, "View menu") are real, named AT-SPI buttons and
menu items and are live-confirmed to dispatch the terminal's own GPUI actions
end-to-end. But the terminal's own content surface has real, live-confirmed
gaps against todo.md's "no pointer-only controls" gate:

* The terminal grid publishes no AT-SPI text, caret, or selection at all.
  `crates/terminal/src/accessibility.rs` fully implements and unit-tests
  `TerminalAccessibilitySnapshot`/`project_visible_terminal`, but that module
  is compiled only into an orphaned library target
  (`crates/terminal/src/lib.rs:1-3`) that the running `rmac-terminal` binary
  never compiles (`crates/terminal/src/main.rs`'s module list omits `mod
  accessibility;`) or calls (`grep -rn "rmac_terminal::"` across the repo:
  zero hits) — independently, `crates/terminal/src/controller/renderer*.rs`
  and `chrome.rs` never call `.role()`/`.aria_label()` anywhere. **Typing a
  command is therefore impossible over AT-SPI on this build**: there is no
  accessible text-entry surface to type into, no clipboard CLI installed on
  the reference laptop to preload input externally (`wl-copy`/`wl-paste`/
  `xclip`/`xsel`/`wtype`/`ydotool`/`dotool` all confirmed absent live), and
  this compounds the separately-documented upstream gap that the pinned
  `accesskit_unix` AT-SPI bridge does not implement `EditableText` at all.
  This is the `run_command` step and the headline finding of this script;
  `scroll` fails for the same root cause (no accessible viewport node to
  scroll or read).
* The tab strip (`crates/terminal/src/controller/renderer/chrome.rs:65-183`)
  gives its tab rows, close buttons, and "+" new-tab button no accessible
  name — but, live-confirmed, GPUI/accesskit still publishes them as
  unnamed AT-SPI "button" nodes with a working `click` action (unlike Notes'
  note rows, which are pruned entirely — see journey 4 below). The script
  therefore verifies tab open/close by counting unnamed clickable buttons
  before and after each Shell-menu action (each tab contributes exactly one
  row and one close button), rather than by name.

`select_all`/`copy`/`paste` click the Edit menu's real items over AT-SPI —
the dispatch itself is verified, but with no accessible text/selection state
on the terminal and no clipboard-reading tool on the reference laptop, their
functional effect cannot be independently confirmed; the report says so
explicitly rather than claiming a pass it cannot back up.

`scripts/test_journey_terminal.py` unit-tests the script's pure JSON-parsing,
environment-discovery, budget, and tab-count-delta logic and runs anywhere
with `python3 -m pytest scripts/test_journey_terminal.py`.

### Journey 4 — Notes

`scripts/linux/run-journey-notes.py` exercises journey 4 ("Create, search and
edit a note, and recover it after a crash"; `crates/notes`, package
`rmac-notes`) against the real Notes library on the reference laptop:

```sh
scp scripts/linux/run-journey-notes.py jacob@<reference-pc>:/tmp/
ssh jacob@<reference-pc> \
  'python3 /tmp/run-journey-notes.py --output /tmp/journey-notes-report.json'
```

This script deliberately stops before creating, editing, trashing, or
deleting any note — including a throwaway test note — because two
independent, live-confirmed accessibility gaps make it impossible to do so
without risking the reference laptop's real Notes data, which the shared-
laptop brief for this suite requires never be read or modified:

* **No text entry surface at all.** The search field and the new-note
  title/tags/body fields expose neither AT-SPI `Text` nor `EditableText` —
  live-confirmed: their AT-SPI interfaces are `['Accessible', 'Component']`
  only, not even a readable value. Notes wires no
  `on_a11y_action(AccessibleAction::SetValue/ReplaceSelectedText, ...)`
  handler either, unlike Spotlight's search field
  (`crates/launcher-app/src/view/render.rs:351-360`). A title, tag, search
  query, or body cannot be read or typed over AT-SPI on this build — so this
  script cannot even give a test note a unique, identifiable name.
* **The note list and folder sidebar are absent from the AT-SPI tree, not
  merely unnamed.** Live-confirmed twice (~2 s apart) against a freshly
  launched `rmac-notes`: its AT-SPI tree contains exactly one frame with 20
  flat children (16 `button` nodes and 4 `entry` nodes) and no additional
  container, list, or row node of any kind — no note row, no "Recently
  Deleted" (`crates/notes/src/note_navigation.rs:95-104`), nothing. This is a
  clear regression relative to Terminal's own unnamed-but-present tab
  buttons (see journey 3 above); this script cannot state the exact
  mechanism from source alone (full accesskit pruning vs. a responsive
  layout collapsing the sidebar at the window's default size) and flags this
  as an open question rather than asserting an unverified root cause.
  Without any accessible node for a note or folder row, a note cannot be
  selected, opened, or deleted over AT-SPI on this build.

Because a test note created here could not be given an identifiable title
and could not be reselected afterward to clean it up, this script does not
create one — doing so would risk leaving permanent, unremovable clutter in
the reference user's real Notes library. It instead verifies session
liveness, launch, window timing/focus, the confirmed absence of the note-
list/sidebar surface (`note_list_reachable`), and read-only presence checks
of the Notes-specific top-bar menus (File/Edit/Format — opened and inspected,
no item ever invoked), then quits the app the same way journey 1 and 3 do.
This is the same "an honest limitation beats simulated system behaviour"
principle from todo.md, applied one gap earlier than journey 3's stop,
because here even the identify-and-clean-up precondition cannot be met.

`scripts/test_journey_notes.py` unit-tests the script's pure JSON-parsing,
environment-discovery, budget, and AT-SPI-node-classification logic and runs
anywhere with `python3 -m pytest scripts/test_journey_notes.py`.
