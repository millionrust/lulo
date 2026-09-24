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
* `View > Quick Look` did not open any visible window or layer-shell
  surface. Cause: the script launched `/usr/bin/rmac-files`, a package
  build from 2026-09-20 that predates the Quick Look panel
  (`strings` finds no `org.rmac.QuickLook` in it). The current build
  (`~/rmac-dev-bin/rmac-files`), driven the same way (focus the window,
  then the `finder::SelectAll` and `finder::QuickLook` menu actions over
  `org.rmac.AppMenu1`), opens a floating `org.rmac.QuickLook` window.
  The script now tests `~/rmac-dev-bin/rmac-files` when it exists
  (`--files-exec` overrides it) and prints the binary it chose.
* Copy and Move did not work on Linux: the pasteboard's Linux module was
  a no-op, so Copy never wrote anything a Paste in another window could
  read. Files now writes `text/uri-list` (copy) or
  `x-special/gnome-copied-files` (cut) through wl-clipboard, which keeps
  serving the selection after the source window closes, and Paste reads
  either one back (plus KDE's `application/x-kde-cutselection`). This
  needs the `wl-clipboard` package, which `rmac-apps` now depends on;
  without it Copy and Paste show an error naming the package.
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

`scripts/linux/run-journey-terminal.py` exercises journey 3 ("Open Terminal,
run a command, scroll, select and copy text, paste it, and manage tabs")
against `rmac-terminal` on the reference laptop, the same way
`run-journey-launch.py`/`run-journey-files.py` do: AT-SPI (`pyatspi`) and
niri IPC only, no keyboard or pointer injector.

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

  'exec 9>/tmp/lulo-journey.lock; flock -w 900 9 && \
   python3 /tmp/run-journey-terminal.py --output /tmp/journey-terminal-report.json'
```

The last real run against the reference laptop (2026-09-24, before this
pass) found the terminal's *content* surface with real accessibility gaps
against todo.md's "no pointer-only controls" gate: the AT-SPI tree for a real
`rmac-terminal` window exposed exactly 4 button nodes (a profile-picker
button plus 3 unnamed tab-strip buttons) and no text/entry node for the grid
at all.

* **The terminal grid published no accessible text, caret, or selection.**
  `crates/terminal/src/accessibility.rs` fully implements and unit-tests
  `TerminalAccessibilitySnapshot`/`project_visible_terminal`, but that module
  was compiled only into the crate's own library target
  (`crates/terminal/src/lib.rs`) — nothing in the running `rmac-terminal`
  binary (`src/main.rs`'s own module tree) referenced it, so it had no effect
  on the live accessibility tree. `run_command`, `scroll`, and the read-back
  half of `select`/`copy` could not be driven or verified over AT-SPI because
  there was no node to find.
* **The tab strip, close buttons, new-tab button, and profile-picker rows
  used only `.id(...)`, never `.role()`/`.aria_label()`**
  (`crates/terminal/src/controller/renderer/chrome.rs`), so they were
  reachable only as unnamed, structurally-countable AT-SPI "button" nodes —
  enough to count tabs opening and closing, but not to identify one by name
  or read back which tab is active.

This pass fixes both, following the same `.role()`/`.aria_label()` pattern
`shell/bins/rmac-dock/src/main.rs` and `crates/rmac-ui` already use:

* The terminal body div now carries `.id("terminal-grid")`,
  `.role(Role::Terminal)`, and `.a11y_synthetic_children(...)`
  (`crates/terminal/src/controller/renderer/accessibility.rs`, new) that
  publishes the active tab's visible-grid projection as a synthetic
  `Role::TextRun` child with character lengths and a `TextSelection` for the
  caret/selection, exactly the pattern GPUI's own `_accessibility` guide
  documents for a text surface with no per-cell child elements. The
  projection is cached per tab for 120 ms so a fast-scrolling command (`yes`,
  a build log) cannot turn every `cx.notify()` into a full grid re-walk;
  idle windows never call this path at all (nothing calls `cx.notify()`
  without real output/input — see `TerminalView::new`'s redraw channel).
  `rmac_terminal::accessibility` is reached from the binary's own module
  tree the same way `crates/finder/src/view/accessibility.rs` reaches
  `rmac_finder::accessibility` — through the package's implicit lib
  dependency, not a duplicated `mod accessibility;` in `main.rs`.
* Each tab is now `Role::Tab` named with its title, `aria_selected` for the
  active tab, and its close button is named "Close tab `<title>`"; the tab
  track is `Role::TabList`; the new-tab button is named "New Tab"; the
  profile-picker panel is `Role::Menu` and each row is a named, selectable
  `Role::MenuItem`. All of these already had a working AT-SPI `click` action
  via `.on_click(...)`, which GPUI registers automatically.

**Still a gap, not fixed here:** the pinned `accesskit_unix` AT-SPI bridge
does not implement `org.a11y.atspi.EditableText` at all (see
`docs/known-limitations.md` and Spotlight's write-up above), so even with a
real text/caret node, a command still cannot be *typed* into the terminal
without a keyboard injector. Unlike Spotlight's search field or Notes' text
fields below, this pass deliberately does not wire an
`on_a11y_action(AccessibleAction::SetValue, ...)` handler onto the terminal
grid: a screen reader's "set value" action is a single blind
whole-value replace, which is the wrong model for a live shell (it bypasses
line editing, job control, and the exact bytes a shell expects) — an honest
limitation is preferable to a simulated one. **This fix has not yet been
re-verified live**; it needs a fresh `run-journey-terminal.py` run against a
rebuilt `rmac-terminal` binary to confirm the grid, tabs, and picker now
appear with the names and text described above.

`scripts/test_journey_terminal.py` unit-tests the script's own pure logic
(`python3 -m pytest scripts/test_journey_terminal.py`, no live session
required) and does not exercise the live AT-SPI/niri orchestration.

`scripts/linux/run-journey-notes.py` exercises journey 4 ("Create, search
and edit a note, and recover it after a crash") against `rmac-notes` on the
reference laptop, the same way the other journey scripts do.

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

### Journey 5 -- text file through the portal

`scripts/linux/run-journey-launch.py`'s AT-SPI/niri pattern extends to
journey 5 ("Open, edit and save a text file through the portal without
losing content") as `scripts/linux/run-journey-textfile.py`. It creates a
disposable fixture under `~/Documents/lulo-journey-5-<random>/`, drives Text
Editor's exported File menu (`crates/rmac-app-menu`, rendered by
`rmac-top-bar` as real AT-SPI `menu`/`menu item` nodes) to open and save
through whichever `org.freedesktop.portal.FileChooser` backend answers
(`crates/rmac-file-chooser`, ADR 0012, or a GTK/GNOME fallback -- the chooser
is driven generically, by name match, so the script doesn't care which one
opened), and always removes the fixture folder afterward. It launches Text
Editor with `gtk-launch org.rmac.TextEditor [path]` rather than a hard-coded
binary path: on the reference laptop `/usr/bin/rmac-text-editor` is a stale
packaged build, while the user's own desktop entry
(`~/.local/share/applications/org.rmac.TextEditor.desktop`, ahead of
`/usr/share/applications` in the XDG lookup order) points at the current
build (`~/.local/libexec/rmac/rmac-text-editor` -> `~/rmac-dev-bin/`).
`scripts/linux/run-journey-monitor.py` does the same for System Monitor.
`scripts/linux/run-journey-launch.py` (journey 1) still spawns
`/usr/bin/rmac-text-editor`/`/usr/bin/rmac-notes` directly and should
probably be updated the same way -- out of scope for this change.

Two systemic gaps limit what is exercisable on the reference laptop today,
found by directly probing the live AT-SPI tree (not inferred):

* **The portal panel is not deployed there.** `systemctl --user status
  rmac-file-chooser.service` reports the unit does not exist, and
  `rmac-portals.conf` still reads `default=gnome;gtk;*` with no
  `org.freedesktop.impl.portal.FileChooser` override. Clicking File > Open
  or File > Save As... over AT-SPI genuinely activates the menu item (the
  click succeeds), but no dialog window opens at all -- not even a GTK
  fallback -- and Text Editor shows no error either. The script waits a
  bounded time for a new AT-SPI application to appear, records the gap
  precisely when none does, and falls back to opening the fixture with a
  direct launch (`fallback_spawn`, the same pattern as journey 1's
  Dock/Spotlight fallback) so the rest of the journey can still be measured;
  ADR 0012's backend exists in the repository but is simply not installed on
  this host yet.
* **No AT-SPI Text or EditableText anywhere.** `queryText()` and
  `queryEditableText()` both raise on every `InputState`-backed entry this
  script has probed, including Text Editor's own document body -- not just
  Spotlight's query field (run-journey-launch.py documented that gap for
  Spotlight alone; this script confirms it is systemic). The
  encoding/line-ending picker button does expose a `click` action, but
  invoking it never opens its dropdown menu over AT-SPI either. With no
  keyboard or pointer injector installed on the reference laptop, there is
  no accessible way to dirty the document buffer at all, so a plain File >
  Save (a no-op on a clean buffer, `crates/text-editor/src/view/saving.rs:
  37-42`) can only be checked as "did nothing," never as a real write.

What the script *can* and does verify for real, without typing anything:

* on-disk content is unchanged by merely opening, or by a no-op Save
  (SHA-256 compared at every step);
* Save As -- which always writes regardless of the dirty flag
  (`crates/text-editor/src/view/saving.rs:75-140`) -- is driven through the
  portal to a sibling folder and the resulting copy is verified
  byte-identical to the original;
* external-change detection needs no typing at all: Text Editor watches its
  open document's directory (`crates/text-editor/src/view/lifecycle.rs:
  17-26`) and shows an always-visible "This document changed outside Text
  Editor..." banner with a real, clickable "Review..." control the moment
  the file changes underneath it (`crates/text-editor/src/view/render.rs:
  191-207`). The script edits the file directly on disk while it's open,
  waits for that banner, opens the Conflict dialog via Review..., dismisses
  it with Cancel, and confirms the file on disk still holds exactly the
  externally-written bytes;
* the SIGKILL-during-save race is exercised for real by targeting Save As
  (not Save, which the buffer can never dirty): the script re-saves a 24 MiB
  fixture over itself through the same Save-As UI path, polls for
  `rmac_storage::atomic_write`'s sibling `.{name}.tmp-<pid>-<seq>` file to
  appear as proof the write is in flight, and SIGKILLs the editor at that
  instant, then asserts the destination is either the complete original
  bytes or entirely absent -- never truncated. This is best-effort ("if
  feasible" per the brief): on a laptop where the portal doesn't open at
  all, the write is never triggered either, so the step reports "not
  conclusively exercised" rather than a false pass.

`scripts/test_journey_textfile.py` unit-tests the pure fixture-naming,
hashing, atomic-write-temp-file-pattern, JSON-parsing, and report-building
logic with `python3 -m pytest scripts/test_journey_textfile.py` (no live
session required).

### Journey 6 -- inspect and stop a process, with confirmation

`scripts/linux/run-journey-monitor.py` exercises journey 6 ("Inspect
resource use and safely stop a process, with confirmation") against System
Monitor (`crates/activity-monitor`, binary `rmac-system-monitor`). It starts
a single disposable `sleep 600` it owns -- identified throughout by the
exact PID this script spawned, never by a fuzzy name match, so it can never
act on any other process -- then tries to find and stop it exactly as a
real user would, through the search field, the process list, and the
Quit/Force Quit confirmation flow.

On the reference laptop as of this writing this is entirely blocked, and the
script proves it precisely rather than reporting a false pass:

* **The process list has zero AT-SPI semantic representation.** A live dump
  of `rmac-system-monitor`'s whole AT-SPI tree at its default 960x640 size
  found no table, row, or cell for any process, ever -- one run's tree
  contained 14 nodes total (1 application, 1 frame, 11 chrome buttons, and 1
  unlabelled search entry); a later run against the same desktop-entry
  ("gtk-launch org.rmac.SystemMonitor", resolving through
  `~/.local/share/applications` to the current dev build) found the window's
  entire AT-SPI tree collapsed to its 3 unlabelled title-bar buttons only --
  no tabs, no toolbar, no Quit/Force Quit, no search entry either. Neither
  run ever saw a table/row/cell. `crates/activity-monitor/src/accessibility.rs`
  already defines the right projection for process rows
  (`project_process_table`, `ProcessTableAccessibilitySnapshot`,
  `project_process_action_dialog` -- accessibility.rs:149,241, each with its
  own passing unit tests), but grep confirms zero call sites for any of it
  outside those unit tests: the live table renders each row as a plain
  `div()` with no AccessKit wiring
  (`crates/activity-monitor/src/process_table.rs:363-389`), so none of the
  modelled semantics ever reaches AT-SPI. The coordinator should re-check
  whether the currently staged dev build has a broader accessibility
  regression beyond the process table -- `scripts/linux/run-journey-monitor.py`'s
  `check_quit_controls_exist` step reports exactly what a given run finds,
  rather than assuming either shape.
* **The search field cannot be typed into either**, for the same systemic
  reason as journey 5's document body: `queryEditableText()` raises.

Because selecting a row is a hard prerequisite for the confirmation dialog
and for Quit/Force Quit, there is no separate accessible path left to fall
back to (unlike journey 1's Dock/Spotlight fallback, there is nothing
downstream of "select a process" that a different mechanism could still
reach). The script deliberately does **not** click Quit/Force Quit, or the
top bar's "Process" menu, when it cannot first confirm its own disposable
process is selected: Quit/Force Quit act on whatever `selected_pid` a mouse
click or keyboard table-navigation last set
(`crates/activity-monitor/src/process_table.rs:343-365`,
`crates/activity-monitor/src/view.rs:165-176`), which this script cannot
observe or control over AT-SPI -- and the reference laptop's session is
shared with other automated agents, so a row could already be selected by
someone else. Invoking Quit blind could therefore signal an unrelated
process, which this journey must never do. It reports the blocked steps
precisely instead, and always kills its own marker process directly (never
through the app's UI) in a `finally` block, so no stray `sleep` survives a
run regardless of how the journey went.

`scripts/test_journey_monitor.py` unit-tests the pure JSON-parsing,
environment-discovery, process-identity, and report-building logic with
`python3 -m pytest scripts/test_journey_monitor.py` (no live session
required).

  'exec 9>/tmp/lulo-journey.lock; flock -w 900 9 && \
   python3 /tmp/run-journey-notes.py --output /tmp/journey-notes-report.json'
```

The last real run against the reference laptop (2026-09-24, before this
pass) found two compounding accessibility gaps severe enough that the script
deliberately stops short of creating, editing, or deleting any note — doing
so blind, with no way to identify or find the note again afterward, could
leave unremovable clutter in the reference user's real Notes library:

* **No text-entry surface exposed AT-SPI Text or EditableText.** A fresh
  `rmac-notes` window's four `entry`-roled nodes (search, title, tags, body)
  reported `interfaces=['Accessible', 'Component']` only — no name, no
  readable value, and (unlike Spotlight's search field) no
  `on_a11y_action(AccessibleAction::SetValue/ReplaceSelectedText, ...)`
  handler at all.
* **The entire note list and folder sidebar were absent from the AT-SPI tree
  outright, not merely unnamed** (`crates/notes/src/note_navigation.rs`'s
  folder rows and note rows, both plain `div().id(...).on_click(...)` with
  no `.role()`/`.aria_label()`, via `crates/notes/src/presentation.rs`'s
  `folder_row`). A live tree dump found exactly one frame with 20 flat
  children (16 unnamed/inert buttons, 2 named view-toggle buttons, and the 4
  `entry` nodes) and no container, list, or row nodes at all.

This pass fixes both, following the same pattern the terminal fix above and
`crates/launcher-app`'s Spotlight field already use:

* Folder rows (`presentation::folder_row`) and note rows
  (`note_navigation::render_note_list`) are now `Role::ListItem` with
  `aria_selected` for the current selection. Since GPUI's `div()` exposes no
  AccessKit `description` property, each row folds what would have been a
  description into its accessible name — "All Notes, 12 notes",
  "Pinned, Groceries, Yesterday, milk eggs bread" — the same way this
  audit's own Toast fix combined a title and message into one `aria_label`
  when no separate mechanism existed. Their containers (the folder sidebar
  and the note-list scroll view) are now `Role::List` with a name ("Folders",
  "Notes"/"Results"). All rows already had a working `click` action via
  `.on_click(...)`.
* The search, title, tags, and body fields are each wrapped in a
  `Role::TextInput`/`Role::SearchInput` node with an `aria_label` and
  `aria_value` set to the field's current text, and
  `on_a11y_action(AccessibleAction::SetValue/ReplaceSelectedText, ...)`
  handlers that replace the value and then run the same edit-scheduling or
  search-dispatch path a keystroke takes — mirroring
  `crates/launcher-app/src/view/render.rs`'s Spotlight field exactly. The
  title/tags/body handlers no-op while the field is read-only (Recently
  Deleted, Markdown preview, or no worker), matching the field's own
  `.disabled(...)` state.

**Still a gap, not fixed here:** as with Terminal, the pinned `accesskit_unix`
does not implement `org.a11y.atspi.EditableText`, so a real screen-reader
user still cannot *type* into these fields without a keyboard injector today
— the `SetValue`/`ReplaceSelectedText` wiring above is there for when that
upstream gap closes, not a present-day substitute for it. Folder/note rows
also do not yet expose the pin state as a distinct AccessKit property (folded
into the name instead, for the same reason as the description above).
**This fix has not yet been re-verified live**; it needs a fresh
`run-journey-notes.py` run against a rebuilt `rmac-notes` binary, and even
then the script's own safety reasoning above means it will still stop short
of creating a note — a full live confirmation that a title can be set and
read back needs a manual Orca pass, not just this script.

`scripts/test_journey_notes.py` unit-tests the script's own pure logic
(`python3 -m pytest scripts/test_journey_notes.py`, no live session
required) and does not exercise the live AT-SPI/niri orchestration.

### Journey 6 — System Monitor's process table (fix landed this session)

Journey 6 ("Inspect resource use and safely stop a process, with
confirmation"; `crates/activity-monitor`, binary `rmac-system-monitor`) has
its own live acceptance script, `scripts/linux/run-journey-monitor.py`,
developed on a sibling branch; this branch's own history does not yet include
it, so it is referenced here by name rather than reproduced. That script's
last real run found System Monitor's process table entirely unreachable over
AT-SPI (a live tree dump found no table, row, or cell for any process, and a
search field exposing neither `Text` nor `EditableText`), for exactly the
pattern this suite already documented twice — Terminal's orphaned
`accessibility.rs` (journey 3) and Notes' absent note-row semantics (journey
4): `crates/activity-monitor/src/accessibility.rs` fully implemented and
unit-tested `project_process_table`, `project_process_action_dialog`, and
`project_live_feedback` (accessibility.rs:149–306), but nothing in
`process_table.rs`/`view.rs` called any of it. Unlike Terminal/Notes, the
root cause here was not that the module was uncompiled — it was that
`gpui_component::table::{TableState, TableDelegate}` (the virtualized table
API `ProcessTableDelegate` implements, distinct from the same crate's
declarative `Table`/`TableRow`/`TableCell` builder that
`docs/accessibility-audit.md`'s "Table" row describes as Pass) sets **no**
AT-SPI role anywhere on its own: the container, header, rows, and cells are
entirely the `TableDelegate` implementation's responsibility, and
`ProcessTableDelegate`'s `render_tr`/`render_td` were plain, roleless
`div()`s.

This session wired the existing projection into the live table rather than
building a new one:

* `ProcessTableDelegate` now caches a `ProcessTableAccessibilitySnapshot`
  (`accessible`), recomputed by `refresh_accessible()` on every data
  refresh/filter/sort/column-visibility change (`apply_view`) and on every
  selection change (`set_selected_pid`, now the only place `selected_pid` is
  written) — bounded by the same 300-row/11-column limits
  `project_process_table` already enforces, and never recomputed from a
  per-frame render path.
* Each row (`render_tr`) is `Role::Row` with `aria_selected` and an
  `aria_label` combining the process name, PID, %CPU, and memory;
  `AccessibleAction::Click`/`Focus` select the row through the same
  `set_selected_pid` + `set_selected_row` path the mouse handlers use (the
  mouse handlers were refactored to share it too, in a new `select_row`
  helper).
* The header row and cells (`render_header`/`render_th`) get `Role::Row` /
  `Role::ColumnHeader` with the column's display name; the table's own
  container (`view/render.rs`) gets `Role::Table`, an `aria_label`, and
  `aria_row_count`/`aria_column_count`.
* The toolbar search field (`view/render/chrome.rs`) is wrapped the way
  `crates/launcher-app`'s Spotlight query field already is:
  `Role::TextInput`, `aria_label("Search")`, `aria_value` mirroring the live
  query, and `AccessibleAction::SetValue`/`ReplaceSelectedText` routed to a
  new `MonitorView::set_search_from_assistive_technology`.
* The five toolbar tabs (CPU/Memory/Energy/Disk/Network) get `Role::Tab` +
  `aria_label` + `aria_selected`.
* The Quit/Inspect/Columns toolbar buttons are icon-only, so
  `gpui_component::button::Button`'s `aria_label` — settable only via
  `.label()`, which would also draw visible text — could not name them
  directly; they get an outer accessible wrapper (`accessible_icon_button`)
  carrying the name and an `AccessibleAction::Click` that runs the identical
  state change the existing mouse `on_click` already runs. Quit/Inspect
  already no-op when nothing is selected, so no separate "disabled" signal
  was added beyond the inner button's existing focus/click gating.
* The process-ended/failed banner now gets `Role::Alert` with a combined
  title+message `aria_label`, matching `rmac-ui`'s `Toast`. The confirmation
  dialog already had `Role::AlertDialog` and named buttons via `rmac_ui::alert`
  (`docs/accessibility-audit.md`'s "Fixes applied" #4); its logic — what
  Quit/Force Quit act on, and the confirmation requirement — was not touched.

**Left as a Gap, not fixed here**: individual table cells (`render_td`)
carry no `Role::Cell`; the outer wrapper around the three icon-only toolbar
buttons necessarily nests a second, unnamed `Role::Button` node (the inner
button's own) inside the named wrapper, which an AT client may present as
two adjacent button-shaped entries instead of one; and the column-chooser
popover's checkbox-like rows are unchanged.

None of this was live-verified with AT-SPI or Orca in this session — no
laptop access was used or needed for a code-level wiring fix. The
coordinator should re-run `scripts/linux/run-journey-monitor.py` against a
laptop build containing this change: its `check_quit_controls_exist` and row
introspection should now find a real `Role::Table`/`Role::Row` tree instead
of the previously reported empty one, and the search field should report a
real `Role::TextInput`. The script's own safety behavior — never invoking
Quit/Force Quit unless it can independently confirm its own disposable
process is selected — is unchanged by this fix and should still hold: the
`AccessibleAction::Click` this session added lets the script select a row by
PID and observe `aria_selected` flip, but the script's existing caution
about a shared laptop session choosing what to click is a script-side
decision this fix does not alter.
