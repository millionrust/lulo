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
Dock launch, Spotlight launch, window appearance and warm launch-to-window
timing against the todo.md performance budget (p95 ≤ 500 ms for simple apps),
focus, app switching by `niri msg action focus-window`, and closing through
each app's own top-bar Quit menu item — plus a `gaps` list. "Log in" checks
that a real graphical rmac session is already active rather than performing a
full logout/login, so the shared reference session is never disturbed; the
destructive full GDM login/logout journey is
[`docs/session-journey-evidence.md`](session-journey-evidence.md).

As of this writing, neither the Dock nor Spotlight can actually be activated
this way: Dock icons expose only the AT-SPI Accessible/Component interfaces
(no `click` action), and Spotlight's search field exposes neither `Text` nor
`EditableText` (so a query cannot be typed) with no D-Bus query/activate
entry point to fall back to. Both are real accessibility gaps against
todo.md's "no pointer-only controls" gate, reported as failed steps rather
than hidden. To still measure the rest of the journey, the script then
launches the target application directly (the same installed command a Dock
or Spotlight activation would run), clearly labeled `fallback_spawn` in the
report.

`scripts/test_journey_launch.py` unit-tests the script's pure JSON-parsing,
environment-discovery, and report-building logic and runs anywhere with
`python3 -m pytest scripts/test_journey_launch.py` (no pyatspi, niri, or live
session required); it does not exercise the live AT-SPI/niri orchestration,
which only runs on the reference laptop.
