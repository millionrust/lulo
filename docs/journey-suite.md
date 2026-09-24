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
