# Behaviour-parity suite

## Fake hardware

The private nested session (headless Sway or nested niri, every runner
below) has no real NetworkManager, BlueZ, UPower or backlight, so Control
Centre, the top-bar status menus (Wi-Fi, Bluetooth, Sound, Battery) and the
matching System Settings panes all ran against empty/unavailable data —
state the real laptop, with real hardware, never produces. That gap is why
a real Control Centre crash (2026-10-03) never reproduced in any nested
test.

`scripts/behavior/fake_hardware.py` closes it: `fake_hardware.start(work)`
starts a *private* D-Bus system bus (never the real one — only
`DBUS_SYSTEM_BUS_ADDRESS` in the env dict handed to the nested apps points
at it) and loads three python-dbusmock templates onto it — NetworkManager
(three Wi-Fi networks, one connected), BlueZ (one adapter, one paired and
connected device) and UPower (an 80%, discharging battery) — plus a
scratch tree shaped like `/sys` (`LULO_FAKE_SYS_ROOT`) so
`system-settings::hardware::current()` and `rmac-osd`'s backlight reader,
which read real sysfs paths by default, detect the same devices when
pointed at it.

It is wired into `run_lulo.py`, `run_menu_dismiss.py`, `run_power_dialogs.py`
and `run_lulo_journey.py`, on by default; pass `--no-fake-hardware` to any
of them to go back to the old empty-hardware session. python3-dbusmock is
an Ubuntu package (installed in CI's `scenarios` and `checks` jobs); where
it is not installed — e.g. this repo's reference laptop, which has no sudo
access for an agent to install it — `start()` logs a warning and returns
`None`, so every caller falls back to today's behaviour instead of failing
the run.

The same bus also carries the repository's own templates in
`tests/dbusmock/`: AccountsService (`accounts_service.py`: the account
running the session, under its real UID, as an administrator, plus one
standard user) and cups-pk-helper (`cups_pk_helper.py`: driverless printers
to discover, every change recorded). `fake_cupsd.py` serves one idle printer
and one waiting job on a scratch socket that `CUPS_SERVER` points the apps
at. The Lulo-only scenarios `settings/users-new-user-sheet`,
`settings/login-password-change-sheet` and `settings/printers-add-sheet`
depend on them, so they need python3-dbusmock (CI); on a machine without it
they fail at the disabled Add User…/Change… button rather than pass on empty
data. The Rust integration tests in `crates/rmac-users-linux/tests` and
`crates/rmac-printers-linux/tests` start the same templates on their own
private bus (`tests/dbusmock/private_bus.rs`); CI's Linux jobs install
python3-dbusmock and set `RMAC_REQUIRE_DBUSMOCK=1`, so a missing mock fails
there instead of skipping.

`scripts/behavior/fake_audio.py` fakes the sound server the same way for
`run_menu_dismiss.py`: it puts `pw-dump` and `wpctl` stand-ins on the nested
session's PATH (rmac-audio runs both by name) over a JSON graph with two
outputs, Lulo Speakers (default) and Lulo HDMI Display. `wpctl set-default`,
`set-volume` and `set-mute` change the graph and wake every `pw-dump
--monitor` through its own FIFO, so Control Centre's Sound view lists real
outputs and switching one is checked end to end. It needs no package, so it
also runs on the reference laptop; that runner also fakes the backlight there
when python3-dbusmock is missing.

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
python3 scripts/interaction/interaction_diff.py
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
~2 of 3 runs stayed open); the original 2026-10-02 run could not measure
its sliders because `rmac-quick-settings` exposed no AT-SPI children.
The current nested probe asserts a populated panel and Wi-Fi detail view,
then targets accessible sliders for hover captures. The 2026-10-03
Display capture found no hover change (INT-008); Sound was unavailable in
the private session and remains unmeasured. After the hovers, the probe sweeps the pointer
across the whole open panel and sets every enabled slider through AT-SPI;
it fails if Control Centre panics (CC-16).

As of 2026-10-03, `lulo_probe.py` also records the Lulo side of six more
surfaces: Spotlight/the launcher and Notification Centre (shell harness,
shortcut-driven, outside-click/Escape), the Dock (hover, right-click/item
menu), and three surfaces via a new "app" harness that launches one app
directly in `run_lulo.Nested`'s headless Sway with no shell - the Files
background context menu (outside-click/Escape), Text Editor's
unsaved-changes alert (Escape, Tab focus), and the Settings sidebar list
(hover, right-click, scroll, Tab focus). None of these has a Mac recording
yet (this agent never drives the owner's live Mac GUI), so each surface's
`status` is `"lulo-only"`, not `"automated"`: `interaction_diff.py` reports
every one of their probes as "no recording" rather than a pass, exactly as
it already does for a genuinely missing recording on an "automated"
surface. Several facts were flaky run to run in the same live session
(Spotlight's outside-click, the Dock's hover/right-click, Notification
Centre's Escape) - recorded as whatever one run observed, the same
treatment Control Centre's own flaky Escape probe already gets, not
smoothed into a single answer. One probe (the Dock's right-click) could
not reliably be driven at all: a synthetic secondary pointer click does
not reach a Dock item in this harness (a left-click sanity check on
another tile also failed to activate it, and AT-SPI exposes no named
secondary action to fall back to the way `click_node()` does for the top
bar), so it is recorded as unmeasured rather than a false "no menu".
Status menus, dialogs beyond Text Editor's, arrow keys, type-to-select,
double-click and press-and-hold are still declared for the matrix but
have no driver yet.

## System Settings View menu

`scripts/behavior/run_lulo.py --check-settings-view-menu` launches one Settings
instance in its private compositor and activates Appearance, Wallpaper and
About through the published application-menu D-Bus endpoint. It checks that
each command opens the named pane; no input reaches the live session.

```sh
python3 scripts/behavior/run_lulo.py \
  --bin-dir ~/rmac-wt/target/iterate --check-settings-view-menu
```

## Calendar and Mail (pending)

ACC-2's non-UI GOA wire contract is exercised by
`scripts/behavior/run_goa_private_bus.sh` on the reference laptop. It launches
one fake GOA service on a disposable session bus with temporary XDG directories
and runs the ignored `private_goa` integration test. The test covers account
enumeration, AddAccount, service toggles, token/password requests, and Remove;
it never connects to the owner's session bus or changes a real account.
An optional ignored `real_goa` test reads the laptop's live ObjectManager
without printing identities or changing accounts; run it only with
`RMAC_READ_ONLY_GOA_TEST=1` under the shared build lock:

```sh
exec 8>/tmp/lulo-cargo.lock; flock 8
export CARGO_TARGET_DIR="$HOME/rmac-wt/target"
RMAC_READ_ONLY_GOA_TEST=1 cargo test -p rmac-accounts-linux --profile iterate \
  --test real_goa -- --ignored --exact real_goa_object_manager_read_only
```

ACC-3 adds `tests/behavior/settings/internet-accounts-add-sheet.json` as a
Lulo-only, cancel-before-sign-in scenario. It has no Mac recording yet. The
`scripts/assert_internet_accounts_accessibility.py` AT-SPI check opens the
same sheet and asserts six named provider actions and an accessible email
field, then cancels; it never creates an account.

`docs/behavior-pending/calendar/` and `docs/behavior-pending/mail/` remain
stubs for the apps planned in ADR 0022. They move to `tests/behavior` once the app exists,
`scenario.APPS` lists `calendar`/`mail`, and a Mac recording exists. Mac
recordings use only a local "On My Mac" calendar and mailbox: never send mail,
sign in, or confirm a deletion on the owner's Mac. The Lulo side runs against
in-tree fixture IMAP/SMTP/CalDAV servers and a fake goa-daemon on a private bus.

MAIL-5 has two Lulo-only fixture scenarios in `tests/behavior/mail/` for
marking a message read and toggling conversation grouping. MAIL-7 adds four
more: flagging (`mail/flag-message`), Junk (`mail/move-to-junk`), delete with
undo (`mail/delete-and-undo`), and the toolbar/⌘F search field filtering the
list (`mail/search-mailbox`). MAIL-8 adds a seventh, adding a signature in
Mail ▸ Settings ▸ Signatures (`mail/settings-signature`). The six fixture-list
scenarios compare against `mail_messages` (conversation/unread/flagged counts
over the fixture's ten messages), driven by the app's own keybindings rather
than `menu_action` where one exists. Mac recordings against a local mailbox
remain pending until the Mac can be measured; `docs/behavior-pending/mail/`
keeps the speculative, Mac-recording-shaped versions of scenarios written
before the app existed until then. Search against a real account — local
index first, then IMAP `SEARCH` on the server — remains open behind
MAIL-2/MAIL-4; this milestone's search is the fixture-backed list filter in
`rmac-mail::MailState`, proven against FTS5 token parsing and escaping by
`rmac-mail-storage`'s own unit tests.

MAIL-2's protocol behaviour is exercised by the local TLS IMAP fixture in
`crates/rmac-mail-imap/src/tests.rs`: it checks authentication, QRESYNC,
CONDSTORE, SPECIAL-USE, UID MOVE/EXPUNGE, message literals and IDLE. A JSON
live-account mail scenarios remain pending until MAIL-4 connects the app to
the runtime.

MAIL-6's compose window (reply/reply-all/forward quoting, To/Cc/Bcc address
completion, attachments, local Drafts autosave, send via the Outbox) is
covered by unit tests in `crates/mail/src/compose.rs`,
`crates/rmac-mail-mime/src/lib.rs` (attachment content-type guessing) and
`crates/rmac-mail-storage/src/tests.rs` (`allocate_local_uid`,
`clear_attachments`, reusing a draft's UID across autosaves, and discarding
it with `remove_server_uid`). `docs/behavior-pending/mail/attach-file.json`
and `compose-draft-autosave.json` stay pending: the behaviour harness has no
FACTS entry yet for compose attachments or the local Drafts mailbox, and a
Mac recording is still needed.

## Terminal profiles

`docs/behavior-pending/terminal/profile-settings.json` describes the Settings
shortcut. It remains pending until a real Mac recording can provide its
`.mac.json` expectation. On Lulo, the private nested-compositor check opens
Settings, checks all 12 Mac profile names in the accessibility tree, then
selects Clear Dark and checks that the choice was saved:

```sh
python3 scripts/behavior/run_lulo.py --bin-dir ~/rmac-wt/target/iterate --check-terminal-profiles
```

## Desktop icons paint without input

`scripts/behavior/run_desktop_first_paint.py` (runtime suite `desktop-paint`)
runs the desktop in nested niri at scale 2 while a small floating terminal
holds keyboard focus, as an app does in a real session. With no input before
each capture it checks the folder and document icons at startup, the first
folder added while the desktop sits idle, and four picture previews added one
by one; then the shell's `img-paint` probe (a never-focused layer surface,
like the Dock and the menu bar) shows swatches whose assets load 0–120 ms
after the frame that requested them. Both idle main threads must stay asleep
(at most 10 context switches in 10 s).

It starts with the login case: the owner's desktop (the folder `hi` and four
full-screen screenshots at their saved positions) with nothing else open, at
`--scale` (the reference laptop's niri picks 1.25 for its 14-inch 1080p
panel). Rounds after the first also drop the 20 frames after the desktop's
first one (`RMAC_GPUI_TEST_UNPRESENTED_DRAWS`). On the laptop, niri's
scan-out dmabuf feedback makes Mesa's swapchain suboptimal and that frame is
never presented, so the folder icon must still appear without input.
`--fixture-dir` copies the real screenshots read-only, `--companions` starts
the menu bar and Dock alongside, and `--gpu` renders on the machine's GPU
instead of llvmpipe/lavapipe. Captures wait until the desktop process has
been idle for 1.5 s, not just for `--settle`. On a busy CI runner the folder
artwork takes about 5 s to rasterise, so the earlier fixed 5 s wait captured
the desktop before its icons loaded. Those startup checks only passed
because the CI gradient wallpaper's own blue pixels counted as folder blue.

```sh
python3 scripts/behavior/run_desktop_first_paint.py --bin-dir shell/target/iterate \
  --probe shell/target/iterate/img-paint --capture-dir /tmp/lulo-desktop-paint
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

## Lulo Intelligence's Spotlight rows, with the real model

`scripts/behavior/run_spotlight_intents.py` is the private nested-niri
scenario for ADR 0024 phase 1's "Lulo can do this" rows. By default it
stubs the model (`RMAC_INTELLIGENCE_ENGINE=fixture`, CI's only mode: there
is no model file there) and checks the off/on, arm/confirm and idle-exit
behaviour with one fixed query.

`--real-model` instead runs the *installed* binaries (`--bin-dir
/usr/libexec/rmac`) with `RMAC_INTELLIGENCE_ENGINE` unset, so
`rmac-intelligence-service` loads the real `llama.cpp` engine and the
owner's own downloaded, checksum-verified Qwen3.5-0.8B model. It types
twenty-two realistic requests into Spotlight one at a time — correct
phrasings, two typos, three requests the closed intent list cannot do, and
two plain single-word searches that must never reach the model at all
(`worth_asking` needs two or more words) — and records each row's exact
text and the latency from the last keystroke to the row appearing. It
confirms two actions end to end, in the nested session only: a second
Return switching the nested theme store to Dark, and a Return starting a
real 10-minute timer in the nested Clock store. With `RMAC_FRAME_TRACE` on
the launcher process only (never the whole session: sharing one path would
have each process's `File::create` truncate what an earlier one wrote), it
also measures keystroke-to-presented-frame latency while the model answers
in the background, and that the service leaves within its idle timeout
with the session otherwise quiet.

The owner's model is linked into the private session read-only, by a
directory-level symlink from this run's own `$XDG_DATA_HOME/lulo/
intelligence/models` to the owner's real one. `rmac_intelligence::verify`
reads the `<sha256>.gguf` file with `symlink_metadata` and the `.verified`
stamp with `O_NOFOLLOW`, both of which refuse a *leaf* symlink outright, so
only the parent directory may be one; every other path component,
including that one, is still resolved normally, so the owner's real files
are seen exactly as the service already sees them. Nothing under the
owner's `HOME` is ever written: the owner's own fetcher already wrote a
`.verified` stamp that matches the file's (size, mtime, inode) identity, so
the service only reads it here, never recomputes or rewrites it.

```sh
python3 scripts/behavior/run_spotlight_intents.py --real-model \
  --bin-dir /usr/libexec/rmac
```

Measured on the reference laptop (2026-10-08, idle otherwise confirmed —
`ps -eo pcpu,comm | awk '$1+0>30'` empty, as ADR 0024's own phase 0
methodology requires): cold (first-ever prefix evaluation) 15.1 s, matching
the ADR's own phase 1 "16.5 s through the service" figure; warm end-to-end
(last keystroke to the row showing) p50 1.3 s, min 1.1 s, max 1.6 s across
15 rows. 20 of 22 requests showed the row the request actually calls for,
including both typo'd ones that parsed correctly ("trun on drak mode",
confirmed end to end to Dark) and the file search, which free-text-matched
by prefix. Two were genuinely wrong, reproducibly across repeated clean
runs — not this harness's own bug, and not CPU contention from another
build on the shared laptop (checked and ruled out after one run was
discarded for exactly that, following the ADR's own precedent): "opn
notse" showed no row at all (the typo was too severe for the pre-fine-tuning
0.8B model), and "remind me to call mum at 5" showed "Start a 5-Minute
Timer" — the system prompt already says a reminder at a clock time is
`none`, but the small model still read "5" as a duration. Both are inside
the base-model accuracy ADR 0024 §6 documents and scopes to phase 2's
fine-tuning, not a regression here. Keystroke-to-presented-frame latency
while typing stayed fast (p50 20 ms, p95 44 ms over ~714 traced keystrokes);
24 of them, clustering one per request roughly 2–3 s after that request's
window focused (not while actively typing, and not scaling with the 15 s
cold load, whose own slow frame was only 566 ms), took 400–620 ms to
present — most likely the "Lulo Intelligence" results section being
inserted once the row arrives, not the model blocking the UI thread. It is
bounded and infrequent rather than a stutter while typing, but is noted
here for whoever next touches the launcher's result-list layout. The
service exited within its idle timeout every time, and the private
session's own processes (by `XDG_RUNTIME_DIR`) cost a handful of clock
ticks over the following 2 s — the idle compositor's own redraw, not a
live intelligence worker.

This mode never exercises the real `rmac-intelligence.service` systemd
unit: the private bus activates the service directly from its own
`org.rmac.Intelligence1.service` (`Exec=`, no `SystemdService=`), the same
way the fixture pass above always has. A caller-check failure that is
specific to the unit's sandboxing (hardening directives, AppArmor) would
not reproduce here even though the caller check's own logic — same uid,
`/proc/<pid>/exe` under `/usr/libexec/rmac` — is exercised for real (the
binaries run from their installed path, not copied into a private
directory the way the fixture pass's own-directory bypass does it).

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

`scripts/behavior/run_menu_dismiss.py` starts the shipped shell services in
a private nested niri and checks the first Lulo menu open without a runner
warm-up. It exercises outside-click and Escape dismissal for the Lulo menu,
Wi-Fi, Bluetooth, Sound, the Dock context menu, Spotlight, Apps, Control
Center, and Notification Center. It also checks title re-click and switching,
wallpaper clicks on both sides of the top bar's surface boundary, a click on
another window, and a Dock click. `--only focus-return` checks that Esc on
the Lulo menu and on the Wi-Fi menu hands the keyboard back to the window
the menu opened over (UIA-14). See
`tests/behavior/shell/menu-dismissal.md` for the scenario. Control Center's
appearance is checked with a screenshot crop; the other layer popovers are
checked through niri's layer list. `--bin-dir` must hold `top-bar`, `dock`,
`wallpaper`, `rmac-quick-settings`, `rmac-launcher`, `rmac-app-drawer`,
`rmac-notification-center-panel`, and `rmac-shortcut-dispatch`.

```sh
python3 scripts/behavior/run_menu_dismiss.py \
  --niri /usr/bin/niri --bin-dir ~/rmac-wt/target/iterate
```

## Tall menu-bar menus

`scripts/behavior/run_menu_scroll.py` opens Files' File menu in private
nested sessions at 1920x1080 and 1280x720, both at scale 1.25. It checks
that a menu that fits is drawn whole with nothing under it, and that a
taller one stops 5 pt above the screen bottom and scrolls with the keyboard,
the wheel and its scroll arrows. See `tests/behavior/shell/menu-scroll.md`.

```sh
python3 scripts/behavior/run_menu_scroll.py --bin-dir ~/rmac-wt/target/iterate
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
always uses headless Sway with nested niri, Dock, top bar and the resident
shell services (Spotlight, Control Centre, app switcher), including for
ordinary app journeys.

Lulo shots come from niri's own composition (`niri msg action
screenshot-screen`), not from the parent Sway output: nested niri's winit
backend can stop presenting new frames to Sway for seconds after a window
maps, which once made Files look frozen after Quick Look. Shots carry
assertions and a journey **fails** when one does not hold: `expect_change`
(on by default after every action — the screen must change by more than a
caret blink), `expect_window` (focused app id), `expect_mapped` /
`expect_unmapped`, `expect_files` / `expect_no_files` (sandbox globs),
`expect_text` (AT-SPI text of a named accessible) and `expect_accessible`
(names that must be showing). Any runner error also fails the journey.
`--full-too` and `--dump-a11y` save a full-screen shot and every named
accessible's extents beside each shot for review.

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

Dock icon sharpness (DOCK-30) uses the same nested niri with no input at all.
`scripts/behavior/run_dock_sharpness.py` starts only the Dock at `--scale`, optionally with a
read-only copy of a `shell.json` (pinned apps, tile size), captures the output and scores each
tile's mean absolute Laplacian. The blurry d7fa75c9 Dock scored a median of 4.9 at scale 1, the
fixed one 18.6; `--min-sharpness 12` makes that a pass/fail check.

```sh
python3 scripts/behavior/run_dock_sharpness.py --dock $CARGO_TARGET_DIR/iterate/dock \
  --scale 1.25 --min-sharpness 12 [--shell-json COPY_OF_SHELL_JSON] [--output DIR]
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

Mission Control keeps its overlay surface unmapped between opens (SPEED-03).
`run_mission_control_reopen.py` uses `run_speed_sweep.py`'s nested niri, opens
and closes Mission Control over a Calculator window `--opens` times, and checks
each round:

- The open changes the screen.
- After close, grim's capture matches the one taken before the open.
- A typed digit and a click reach Calculator, and the hidden overlay gets no
  input.
- The hidden overlay presents nothing and the service stays idle.

```sh
python3 scripts/behavior/run_mission_control_reopen.py --bin-dir DIR_WITH_MISSION_CONTROL \
  --bin-dir DIR_WITH_RMAC_CALCULATOR --json-output /tmp/mc-reopen.json --opens 5
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
