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
