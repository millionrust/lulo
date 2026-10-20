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
  (Amended 2026-10-07, `op/win-shell`: once a retry has fired, the window
  arms no other until it draws again. With several inactive windows in one
  process, as in the Lulo layer, each retry's message re-checked the other
  parked windows and their idle frames armed retries of their own, about 40
  a second.)
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
  (`frame drew`/`frame idle`). Since ADR 0023 phase 2e it also prints
  `gpui_windows cpu: <pool|timer> <spawn site> <100 ns> at <ms> ms`, the
  CPU each thread-pool task used: WARP, the software rasteriser on a
  machine without a GPU, runs its workers on the same system thread pool,
  and this is how `launch_smoke.py` tells the app's own pool work from
  WARP's.

`scripts/windows/launch_smoke.py` turns both on, prints each app's phases,
groups the wake-ups inside its idle window by source and also names the
threads that used CPU there (Toolhelp32 and `GetThreadTimes`, with the thread
description Rust sets from a thread's name), and writes everything to
`--results`. `scripts/windows/idle_gate.py` reads that file and fails when any
app but Terminal used more than one tick; it is the one blocking step of the
otherwise non-blocking `windows` CI job.

### Window frame

`src/rmac_window_frame.rs` (ADR 0023, "Window chrome: Lulo's own corners and shadow")
implements the core's `PlatformWindow::set_frame_style`: a rounded
`IDCompositionRectangleClip` on the swap chain's visual (`directx_renderer.rs`, applied
again on resize and after a device loss) and an owned, layered, click-through tool window
for the shadow and hairline, updated from `WM_WINDOWPOSCHANGED` and `WM_ACTIVATE` only.

### Smooth scrolling

`src/rmac_smooth_scroll.rs` (WIN-OS-95). Upstream handed every `WM_MOUSEWHEEL`
to GPUI as `ScrollDelta::Lines` the moment it arrived: a mouse notch jumped
three lines in one frame, and a precision touchpad's legacy wheel messages
(small deltas at about 125 Hz) landed two in one 60 Hz frame and three in
another, so the scroll stepped unevenly and stopped dead when the fingers
lifted. Wheel input is now queued in logical pixels (24 px a line) and handed
out once per frame, timed by the vblank that woke it:

- a notch eases out over about 120 ms (time constant 37 ms); a notch the other
  way cancels the rest;
- a stream of high-resolution deltas is handed out at its own velocity over
  the last 100 ms, with a slow correction toward one frame of backlog, and
  glides on when it stops with `gpui_linux`'s momentum (decay 325 ms, from the
  last 60 ms of motion; ADR 0013). A click or a new Direct Manipulation
  contact stops a glide.

Direct Manipulation's pixel pan and system inertia (upstream's
`direct_manipulation.rs`) are kept, but its per-frame steps are evened out
(`PanSmoother`, WIN-OS-96): Direct Manipulation moves the content by whatever
touchpad reports landed before each frame's `Update`, so on the owner's ELAN
touchpad one frame stepped 300 px and the next 2 px. Each frame now hands out
the pan's velocity over the last four frames, timed by the vblank, with a slow
correction toward 12 ms of backlog, and drains the rest when the gesture ends;
the distance is exactly what Direct Manipulation reported. The scroll trace's
`dm=` field is the raw pan, `dy=` what was dispatched. While a glide runs the window keeps
its vblanks; it parks as before when it ends. `RMAC_GPUI_SCROLL_TRACE=1`
prints every wheel message and frame; `RMAC_GPUI_SMOOTH_SCROLL=0` restores
upstream's behaviour for measurements. `scripts/windows/scroll_smoke.py`
(run by `launch_smoke.py` with Text Editor) injects a touchpad-like stream and
three notches and checks that the scroll moves on every frame, without frame
gaps above 1.5 vsync intervals (on a hardware GPU; CI's WARP only reports
them), and keeps moving after the input stops.

Measured on the owner's test laptop (AMD Radeon, 60 Hz, CI debug build), the
same injected input before (`RMAC_GPUI_SMOOTH_SCROLL=0`) and after, two runs
each: touchpad-like stream, per-frame step 18–27 px (CV 0.12–0.14) before,
15.6–22.6 px (CV 0.06–0.07) after; glide after the input stops, 0 frames
before, 83–88 frames (about 1.4 s) after; a notch, one 72 px jump before,
11–12 eased frames after; mean frame interval 16.7 ms both, max gap
18.8–19.0 ms after in the last two runs (an earlier run had one 25 ms gap
followed by a frame that handed out nothing; a late frame now steps by its own
time instead, not yet re-measured on the laptop).

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

## Text (branch `op/win-visual`)

Lulo's windows on Windows read heavier and softer than the same windows on
Lulo OS (owner, 2026-10-13). Both draw Inter from the same OTF files with
the same gamma (1.8), contrast (1.0 grayscale, 0.5 subpixel) and shaders;
the DirectWrite parameters on the owner's laptop were read with a probe and
match `gpui_wgpu`'s defaults. What differed was the rasterizer: DirectWrite
drew Inter's CFF outlines with its own hinting and antialiasing, where
`gpui_wgpu` renders them with swash, hinted.

- `direct_write.rs` now rasterizes the glyphs of the fonts the app added
  itself (`add_fonts`: Lulo's Inter and JetBrains Mono) with the same swash
  calls `CosmicTextSystem` makes on Lulo OS: hinted, the same subpixel
  offsets, grayscale for a transparent window and LCD subpixel coverage for
  an opaque one (the window decides, as on Lulo OS). A DirectWrite face is
  matched to its swash face by its `head` table; a face DirectWrite
  simulates stays DirectWrite's, as do system fallbacks and emoji.
  DirectWrite still shapes and lays out every line.
- Fonts added from `'static` bytes are lent to DirectWrite with an owner
  instead of copied: each face had cost a private copy of its file in
  every process (about 600 KB an Inter face; 2.4 MB for the four faces
  shipped before this branch).
- The CI window scenes (`shell-scenes.yml`) put Calculator and Text Editor
  from the same commit side by side (mean difference about 1 in 255, the
  title bar band under 3). On the owner's laptop, against the same apps
  in a nested Lulo OS at scale 1, Text Editor's toolbar text now differs
  by a mean of 2.2 in 255 (DirectWrite: 6.0) and Files' sidebar by 5.7 to
  8.3 (DirectWrite: 11.6 to 13.9; the rest is the words Windows' Files uses
  and icons a debug build does not embed).
- `RMAC_TEXT_TRACE=1` reports on stderr which faces swash took over.

## Launch on a real PC (branch `op/win-launch-speed`, WIN-OS-99)

On Ruban's laptop (Ryzen 3 7320U, Radeon, Windows 11, release builds) the
owner measured 0.73 to 1.26 s from a click to an app's window, against the
Mac's 0.3 to 0.5 s. `scripts/windows/launch_times.ps1` splits a launch into
the time before the process exists and the time from process creation to
the first frame on screen, both cold (the build copied to a new folder) and
warm (the launch after). For beta.2: 500 to 690 ms before the process
existed when cold, about 5 ms warm; then 290 to 560 ms to the first frame.

Where the time went, and what changed:

- **Microsoft Defender** scans an exe it has not cached before
  `CreateProcess` returns: `New-MpPerformanceRecording` showed a 460 ms
  `OnOpen` scan of Calculator, and a fresh copy of an exe waited 400 to
  600 ms where the next launch waited 5. Its cache lets an exe go within
  minutes, and the scan grows with the file (63 ms for 0.7 MB, 460 for 19,
  640 for 32). Opening the file, mapping it as an image or reading it does
  not fill the cache; creating its process does. Without changing the PC's
  Defender settings and without code signing (which costs money), two
  things help:
  - Smaller exes: Lulo's fonts (4.8 MB) are files beside the exes, mapped
    and shared by every process (`rmac_ui`'s `fonts_windows`); GPUI's
    `image` no longer builds the default codecs (an AV1 encoder, OpenEXR,
    DDS, HDR, QOI, TGA, rayon) nothing decodes; and one `resvg` replaces two.
    Calculator went from 19.8 to 13.9 MB, Mail from 27.0 to 21.0, Settings
    from 28.6 to 22.4, lulo-shell from 33.7 to 27.1.
  - Warming: when an app is about to be opened (the pointer reaches its
    Dock tile or Apps icon, or it becomes Spotlight's selection), the shell
    creates its process suspended, so none of its code runs, and ends it
    (`rmac_app_launch::prewarm`, at most once per app per two minutes). That
    is when Defender scans it; the click a moment later waited 6 to 9 ms
    instead of 480 to 570, even 30 s on. Nothing runs at idle.
- **DirectWrite's system font collection** was asked for with
  `checkForUpdates`, which reads through every installed font: 106 to 345
  ms on this laptop (251 families), 1.5 ms without. It no longer is, and a
  family that is not installed asks for the check once per process.
- **Console hosts:** Mail, Calendar, System Monitor, Player and Archive
  Utility were console programs, so Windows started a `conhost` for each
  before `main` (about 210 ms of Mail's and Calendar's launches). They are
  GUI programs now, as the other apps were.
- **Direct3D devices** (60 to 70 ms, the GPU driver) are still made on their
  own thread, but the first window takes them, not DirectWrite: the
  platform, GPUI and the app's own set-up run meanwhile. The text system
  gets its GPU state (only colour glyphs use it) when the devices arrive.
- **System Monitor** sampled every process twice before its first frame
  (once building its view, once for the window's own first activation).
- **The first frame:** a window is cloaked (`DWMWA_CLOAK`) when it is shown
  and uncloaked by the frame that first draws it, with its Mac shadow held
  back until then (WIN-OS-16); a one-shot timer uncloaks it after 750 ms if
  nothing drew.

`launch_smoke.py --launch-budget-ms` fails an app whose first frame is on
screen later than the budget after its process started; CI uses 900 ms for
its debug builds on a software renderer. `RMAC_GPUI_STARTUP_TRACE` now
also marks `crt_init` (the first Lulo code, before `main`), rmac-ui's
`init_application`, menu and window-option steps, `window_cloaked`,
`directx_devices_attached` and `first_frame_shown`; an app can add its own
with `rmac_ui::startup_mark`.

## Consequences

- A GPUI bump now also means re-importing `gpui_windows` and re-applying the
  rmac commits, as for `gpui_linux` and `gpui_wgpu` (ADR 0013).
- The changes are worth offering to Zed once they have run on the owner's
  laptop for a while; the owner decides when anything is published.
- No GPL code is involved: GPUI is Apache-2.0.
