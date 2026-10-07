# ADR 0025 — Carry a patched copy of GPUI's Windows backend

- Status: accepted
- Date: 2026-10-07
- Follows: ADR 0013 (patched `gpui_linux` and `gpui_wgpu`), ADR 0023 (Lulo on
  Windows)
- Branch: `op/gpui-windows-idle`

## Context

ADR 0023 runs Lulo's apps on Windows through GPUI's own Windows backend,
`gpui_windows`. CI's `windows` job measured every app, Calculator included, at
6 to 13 scheduler ticks of CPU over 20 s with no input after a 10 s settle
(about 0.5 to 1 % of a core; run 37596770109), and 375 to 530 ms from process
start to a visible window. On Lulo OS both are near zero idle and about 0.1 s,
because ADR 0013's `gpui_linux` parks idle windows. The `op/win-polish` audit
had already found rmac's own app code idle on Windows (WIN-OS-11), so the cost
sat in upstream code this repository could not change.

### Where the backend comes from

- `gpui_platform::current_platform` picks `gpui_windows::WindowsPlatform` on
  `target_os = "windows"` (`crates/gpui_platform/src/gpui_platform.rs`); the
  dependency is `[target.'cfg(target_os = "windows")'.dependencies]` in
  `gpui_platform`, so no other target ever builds it.
- Both lockfiles pinned it as `gpui_windows 0.1.0` from
  `git+https://github.com/zed-industries/zed.git#76c93968da5b8b8809bdd72e4ad9e7d0e946bad0`,
  the same Zed revision as `gpui`, `gpui_linux` and `gpui_wgpu`.

### What woke an idle app

`RMAC_GPUI_WAKE_TRACE=1` (below) logs every wake-up of the process. Over the
20 s idle window of CI run 37602160420 (the unmodified import plus the traces)
every app showed exactly one source, three lines per vblank:

| App | `vsync tick` | `message 0x000f` (`WM_PAINT`) | `frame idle` | Ticks |
|---|---|---|---|---|
| Calculator | 956 | 956 | 956 | 6.01 |
| Notes | 942 | 942 | 942 | 7.01 |
| Text Editor | 955 | 955 | 955 | 2.00 |
| Preview | 942 | 942 | 942 | 4.01 |
| Clock | 984 | 984 | 984 | 6.01 |
| Weather | 990 | 990 | 990 | 9.01 |
| Terminal | 1006 | 1006 | 1006 | 12.02 |

Nothing else woke them: no main-thread task, no thread-pool task or timer, no
other message. The cause is `WindowsPlatform::begin_vsync_thread`: a
`VSyncProvider` thread loops on `DwmFlush` (falling back to a sleep of one
refresh interval when DWM returns early, which is why CI's headless desktop
ran at about 48 rather than 60 a second) and calls
`RedrawWindow(RDW_INVALIDATE)` on every window after every vblank. Each window
then takes a `WM_PAINT`, `draw_window` runs GPUI's frame callback, GPUI finds
nothing dirty and draws nothing. Two threads woke per vblank for the life of
the process.

### Where launch time went

`RMAC_GPUI_STARTUP_TRACE=1` (below) marks each start-up phase from the moment
Windows created the process. In run 37602160420 (debug builds, as CI builds
them), Calculator, the slowest:

| Phase | At (ms) |
|---|---|
| `platform_new` (entering `WindowsPlatform::new`) | 12.7 |
| `directx_devices` | 77.3 |
| `direct_write_text_system` | 179.0 |
| `open_window` | 192.7 |
| `renderer_new` → `renderer_ready` | 193.7 → 359.0 |
| `window_shown` | 381.5 |
| `first_present` | 478.2 |

The other apps were warmer (devices about 7 ms, DirectWrite about 35 ms) but
every one spent 155 to 175 ms in `DirectXRenderer::new`. Upstream's `build.rs`
compiles the HLSL shaders with `fxc.exe` only for release builds; debug builds
compile all eighteen entry points at run time, on every launch, with
`D3DCompileFromFile` and optimisations off. DirectWrite's emoji pipeline did
the same, which was most of its 35 ms.

## Decision

Vendor `crates/gpui_windows` from zed-industries/zed at
`76c93968da5b8b8809bdd72e4ad9e7d0e946bad0` into `shell/compat/gpui_windows`
(Apache-2.0; `LICENSE-APACHE` kept beside it) and use it through
`[patch."https://github.com/zed-industries/zed.git"]` in both the application
workspace and the shell workspace, exactly as ADR 0013 did for `gpui_linux`
and `gpui_wgpu`.

- The import is one commit with the upstream sources unmodified; only
  `Cargo.toml` is rewritten, replacing workspace inheritance with the versions
  and the `windows` feature list from Zed's root manifest at that revision, and
  dropping `[lints] workspace = true`.
- rmac's changes are separate commits on top, each marked `rmac:` in the code,
  so a GPUI bump can re-import and re-apply them (`git log --
  shell/compat/gpui_windows`).
- Because the crate only builds on Windows, the Linux and macOS dependency
  graphs are unchanged. Each lockfile loses just the package's `source` line,
  and the Flatpak Cargo sources drop its git copy, as for the other path
  crates. `cargo deny` needs no change: the crate is Apache-2.0 and all of its
  dependencies were already in the graph.

### Idle frame loop

`src/rmac_frame_loop.rs` ports ADR 0013's parked loop:

- A window asks for vblanks only while its frames draw. A process-wide
  `VsyncDemand` (a mutex-guarded set of windows plus a condition variable)
  holds the windows that want the next one. The vsync thread blocks on it
  while it is empty, waits for DWM only when some window wants a frame, and
  invalidates only those windows.
- After two frames in a row that draw nothing (`PlatformWindow::draw` not
  called, no forced render pending, no touchpad gesture running) the window
  *parks* and leaves the demand.
- A parked window is re-checked after every message the main loop dispatches
  and after every batch of main-thread tasks (`run_foreground_task`, which a
  modal loop such as a native file dialog also runs). GPUI marks a window
  dirty only on the main thread, inside a task, a timer or a platform callback,
  and each of those arrives as a message there, so no change is missed. A check
  that finds nothing dirty draws and presents nothing. Input that dirties a
  parked window therefore draws in the same message, without waiting for a
  vblank, and the window then asks for vblanks again until it is idle.
- Inactive windows: GPUI skips an inactive window's frame when next-frame
  callbacks are queued and less than 33.3 ms passed since the last frame it
  ran (a finished `img()` load repaints that way). Such a frame (drew nothing,
  inactive, within 33.3 ms of the previous one) arms one 35 ms `WM_TIMER`
  whose frame GPUI cannot skip. It never re-arms itself, so an idle window
  takes at most one extra wake-up. This is ADR 0013's 2026-10-04 amendment.
- Minimising takes GPUI's frame callback away (upstream) and now also leaves
  the demand; restoring rejoins it. A destroyed window leaves it too.
- Direct Manipulation (touchpad pan and pinch) runs in manual-update mode and
  only advances when a frame calls `update`, so a touchpad contact
  (`DM_POINTERHITTEST`) wakes the loop and frames keep running while the
  viewport is `RUNNING` or `INERTIA`.
- Device loss: the vsync thread still checks the Direct3D device after every
  wait. A failed swap-chain resize, which sets `invalidate_devices`, now also
  wakes it. A device lost while every window is parked is found on the next
  frame anything draws.
- Known gaps: an occluded but not minimised window that animates keeps drawing
  (DirectComposition swap chains do not report `DXGI_STATUS_OCCLUDED`); and,
  as on Linux, a `Window::on_next_frame` callback queued by a frame that drew
  nothing waits for the next message rather than the next vblank.

### Start-up

- Every build embeds `fxc` output, as release builds already did, so debug
  and release start the same way. `GPUI_WINDOWS_RUNTIME_SHADERS=1` at build
  time brings back upstream's run-time compile for shader work.
- Debug builds no longer turn on the Direct3D debug layer whenever it is
  installed; `RMAC_GPUI_D3D_DEBUG=1` asks for it.
- `WindowsPlatform::new` makes the Direct3D devices (which loads the GPU
  driver) on their own thread while OLE starts and DirectWrite loads the
  system font collection on the main thread. DirectWrite takes the devices
  only for its GPU state, after the collection is loaded; a thread that
  cannot start falls back to making them inline.

### Tracing

`src/rmac_trace.rs` adds two opt-in traces, each read once, costing one load
per call site when off:

- `RMAC_GPUI_STARTUP_TRACE=1`: `gpui_windows startup: <phase> at <ms> ms`,
  timed from process creation (`GetProcessTimes`), for `platform_new`,
  `ole_initialized`, `system_fonts_loaded`, `directx_devices`,
  `direct_write_text_system`, `platform_ready`, `run`, `finished_launching`,
  `open_window`, `create_window`, `renderer_new`, `renderer_ready`,
  `window_created`, `window_shown`, `window_opened`, `first_frame_drawn` and
  `first_present`.
- `RMAC_GPUI_WAKE_TRACE=1`: `gpui_windows wake: <source> <detail> at <ms> ms`
  for each message the main loop retrieves (`message 0x....`), each
  main-thread task (`task <spawn site>`), each thread-pool task and timer
  (`pool`/`timer <spawn site>`), each vsync tick and each frame
  (`frame drew`/`frame idle`).

`scripts/windows/launch_smoke.py` turns both on, prints each app's phases,
groups the wake-ups inside its idle window by source and also names the
threads that used CPU there (Toolhelp32 and `GetThreadTimes`, with the thread
description Rust sets from a thread's name), and writes everything to
`--results`. `scripts/windows/idle_gate.py` reads that file and fails when any
app but Terminal used more than one tick; it is the one blocking step of the
otherwise non-blocking `windows` CI job.

## Results

CI `windows-latest`, debug builds, 20 s idle window after a 10 s settle.

Before: CI run 37602160420 (the unmodified import plus the traces; the
original measurement, run 37596770109, gave the same 6 to 13 ticks and 375 to
530 ms). After: run 37606858660 (`efd4c37d`). Launch time is
`launch_smoke.py`'s process start to a visible window, polled every 20 ms.

| App | Idle ticks before → after | Idle wake-ups before → after | Launch ms before → after | `window_shown` → `first_present` after (ms) |
|---|---|---|---|---|
| Calculator | 6.01 → 0.00 | 2,868 → 0 | 375 → 234 | 228 → 425 |
| Notes | 7.01 → 0.00 | 2,826 → 0 | 250 → 94 | 102 → 215 |
| Text Editor | 2.00 → 0.00 | 2,865 → 0 | 250 → 94 | 98 → 205 |
| Preview | 4.01 → 0.00 | 2,826 → 0 | 266 → 93 | 104 → 170 |
| Clock | 6.01 → 0.00 | 2,952 → 0 | 266 → 94 | 99 → 186 |
| Weather | 9.01 → 0.00 | 2,970 → 0 | 266 → 109 | 105 → 183 |
| Terminal | 12.02 → 1.00 | 3,018 → 0 | 266 → 94 | 100 → 274 |

- Calculator runs first, on a cold runner. In this run its `CreateWindowExW`
  alone took 130 ms before `WM_NCCREATE` reached GPUI (`create_window` 71 ms,
  `renderer_new` 201 ms); in the previous run (37604982444) it took 0.2 ms
  and Calculator was visible at 109 ms. That is the first window a fresh
  desktop session makes, not GPUI.
- The apps are now visible about 95 ms after process start, against the
  ≤ 150 ms target: about 17 ms before `main` runs (loading a large debug
  executable), 8 ms for the Direct3D devices on their thread, about 28 ms for
  DirectWrite's system font collection, 15 ms of app start-up, 5 to 15 ms of
  window and renderer creation and 15 to 20 ms for `SetWindowPlacement`
  (the first `WM_SIZE` resizes the swap chain).
- The remaining idle CPU is not GPUI's: Terminal's 1.00 tick was its
  `notify-rs windows loop` thread. notify 7's Windows backend waits with a
  100 ms timeout, so every notify watcher wakes its own thread ten times a
  second; in the previous run that charged Text Editor 2.00 ticks with no
  GPUI wake-up. Text Editor now makes its watcher only once a document has
  a folder to watch. The final run (37610904494) charged Calculator and
  Terminal one tick each to the same thread: every rmac-ui app watched
  `~/.config/rmac` for System Settings' theme file, which exists only on
  Lulo OS and was found on Windows only because CI's shell sets `HOME`.
  rmac-ui no longer starts that watcher on Windows. Clock's alarm-state
  watcher still polls (WIN-OS-15).
- The window is visible 70 to 175 ms before its first frame is presented
  (WIN-OS-16).

## Consequences

- A GPUI bump now also means re-importing `gpui_windows` and re-applying the
  rmac commits, as for `gpui_linux` and `gpui_wgpu` (ADR 0013).
- The changes are worth offering to Zed once they have run on the owner's
  laptop for a while; the owner decides when anything is published.
- No GPL code is involved: GPUI is Apache-2.0.
