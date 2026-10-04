# Frame timing -- 2026-10-05

The first run of the new frame-timing harness
(`scripts/behavior/run_frame_timing.py`), filling in docs/beta-checklist.md's
two performance rows that had no harness at all: "Input to visible response
p95 <= 50 ms" and the 60/120 Hz animation frame budget.

## What ran

`iterate`-profile binaries (`cargo build --profile iterate`; no `release`
build existed in the shared laptop target at the time of this run, so this
is the weaker of the two profiles the task allows -- rebuild with
`--release` and rerun for a release-profile number before using this as a
ship gate). Built and measured on the reference laptop,
`jacob@192.168.18.52`, under the shared `/tmp/lulo-cargo.lock` and the
shared `CARGO_TARGET_DIR=~/rmac-wt/target`:

```
cargo build --profile iterate -p rmac-text-editor -p rmac-finder -p rmac-quick-settings-app -p rmac-shortcuts
cd shell && cargo build --profile iterate -p rmac-shell-mission-control --features wayland
```

The harness drives four scenarios inside a **private nested niri session**
(never the owner's live session -- headless Sway hosting a nested niri with
the shipped `packaging/rmac-session/shell.kdl`, input injected only into
that Sway via `wlinput.py`, exactly as `scripts/behavior/run_niri_minimize.py`
and `scripts/interaction/lulo_probe.py`'s "shell" harness already do):

- typing in Text Editor (``The quick brown fox...`` x3, 0.03 s/key);
- scrolling Files' list (300-file sandbox, 40 discrete wheel ticks);
- opening Control Centre (`rmac-shortcut-dispatch quick-settings`) and
  closing it with Escape;
- opening Mission Control (niri's shipped Ctrl+Up bind, reaching the
  resident `mission-control --service`) and closing it with Escape.

**GPU path:** Text Editor, Files, Quick Settings and the Mission Control
service ran with no `VK_ICD_FILENAMES` override, so each picked up the
laptop's real Vulkan driver -- the same one GPUI's `WgpuRenderer` uses live.
Only the nested niri's own compositing onto the outer headless Sway output,
and Sway itself, were forced to software (`LIBGL_ALWAYS_SOFTWARE` /
`WLR_RENDERER=pixman`); that layer hosts the session, it is not what is
measured.

## How it's measured

The vendored GPUI Linux backend
(`shell/compat/gpui_linux/src/linux/wayland/frame_trace.rs`) records, only
when `RMAC_FRAME_TRACE=<path>` is set (zero cost otherwise -- a single
relaxed atomic load per call site when unset):

- `frame_callback` -- the compositor's `wl_callback::Done`, i.e. the start
  of `WaylandWindow::frame`;
- `draw_start` / `present` / `draw_skip` -- around `WaylandWindow::draw`'s
  call into the renderer;
- `input` -- the start of `WaylandWindow::handle_input`, covering every
  keyboard, pointer and scroll event GPUI's window receives.

`run_frame_timing.py` then computes, per scenario and overall:

- **input-to-present p95**: for every `input` timestamp, the latency to the
  first `present` that follows it;
- **frame-budget share**: the fraction of `draw_start` -> `present` spans at
  or under 16.7 ms (60 Hz) and 8.3 ms (120 Hz).

Caveat worth keeping in mind when reading these numbers: the vblank pacing
a traced window's `frame_callback` rides on is the outer headless Sway's
synthetic headless-output clock, not the laptop panel's real refresh rate.
The frame-budget share is therefore "GPU+CPU per-frame cost against the
16.7 / 8.3 ms budgets" -- a real, honest number for that cost -- not a
live-hardware vsync measurement. Opening Control Centre and Mission Control
goes through a socket/IPC call, not a Wayland input event, so only their
*closing* Escape keypress contributes an `input` sample; both scenarios'
`input_count` is correspondingly small (2 and 3).

## Results

| Scenario | Frames | Inputs | Input->present p95 | <=16.7 ms (60 Hz) | <=8.3 ms (120 Hz) | Worst frame |
|---|---:|---:|---:|---:|---:|---:|
| Text Editor typing | 139 | 275 | 31.5 ms | 99.3% | 98.6% | 18.9 ms |
| Files scrolling | 44 | 42 | 48.6 ms | 97.7% | 97.7% | 37.0 ms |
| Control Centre open/close | 6 | 2 | 12.0 ms | 83.3% | 66.7% | 23.6 ms |
| Mission Control open/close | 28 | 3 | 25.1 ms | 100.0% | 96.4% | 14.7 ms |
| **Overall** | 217 | 322 | **41.7 ms** | **98.6%** | **97.2%** | 37.0 ms |

Full machine-readable report: [report.json](frame-timing-2026-10-05/report.json).

## Verdict against the checklist budgets

- **Input to visible response p95 <= 50 ms: Pass (41.7 ms overall)**, but
  close to the edge -- Files scrolling alone is 48.6 ms, within 1.4 ms of
  the budget.
- **>= 99% of frames within the 60 Hz (16.7 ms) budget: Fail (98.6%
  overall)**, just short of the line.
- **>= 95% of frames within the 120 Hz (8.3 ms) budget: Pass (97.2%
  overall)**.

Worst offender: **Control Centre's open/close animation**, at 83.3% within
the 60 Hz budget and 66.7% within the 120 Hz budget -- but its sample is
only 6 frames (one open, one close), so treat that figure as a weak signal,
not a confident measurement. Across the larger samples, **Files scrolling**
is the more reliable worst offender: the single worst frame overall
(37.0 ms, more than double the 60 Hz budget) and the highest input-to-present
p95 (48.6 ms). Per the brief, this pass does not chase that regression with
an optimisation; it is left for whoever picks up Files' scroll performance
next, with this harness ready to confirm a fix.

## Next evidence

- Rerun with `--release` binaries once a release build exists in the shared
  target, for a ship-gate number.
- A larger Control Centre/Mission Control sample (more open/close cycles)
  would firm up those two scenarios' thin frame counts.
- This nested-session run cannot measure the real panel's vsync pacing; a
  live-session, read-only trace (never injecting input into the owner's
  session) would be the next fidelity step if the owner wants one.
