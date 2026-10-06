# Speed round 7 — 2026-10-06: what people do

Rounds 1–6 made launching apps and opening panels fast. This round measures
what people do once something is open (scrolling, typing, Files, window
actions), fixes the slowest cases and finishes SPEED-02.

Branch `op/speed-7` on `integ` 9e80a63c. The method is the same as rounds 1–6:
a private nested niri on the GPU path at 1920×1080 on the reference laptop,
never the owner's session. The laptop was quiet before every sweep (`ps`
showed nothing above 8 % CPU) and each sweep held the shared build lock, so
no build could start during it. Medians are reported.

## The harness

`scripts/behavior/run_speed_sweep.py --interactions` (or `--only NAME`) runs
the new scenarios in `scripts/behavior/speed_interactions.py`. All timings
come from the traced process's own `RMAC_FRAME_TRACE`, so each process uses
one clock:

- **Frame cost** runs from a frame's `frame_callback` to its `present`. That
  covers GPUI's layout, prepaint and paint plus the GPU submit, measured
  against the 16.7 ms budget. The target is that ≥ 99 % of frames fit.
- **Key echo** runs from a key press's `input` row to the next `present`.
  A press is an input that comes more than 45 ms after the previous one, so
  releases and modifier rows are never counted as key presses. The target
  is p95 < 16 ms.
- **Settled** is the last `present` of the burst that follows an input. The
  burst ends at the first quiet gap, so a caret that starts blinking later
  is not counted.

The fixtures are a 2,000-item folder, 200 PNGs, 500 notes plus one 200 KB
note (written by the new `rmac-notes-storage` example `seed_speed_fixture`),
10,000 Mail messages (`RMAC_MAIL_FIXTURE_MESSAGES`), 1 MB plain and RTF
documents, and 20,000 lines of Terminal scrollback.

The frame trace now also records `focus_in` and `focus_out`. Each app launch
reports `focus_after_first_frame_ms` and `presents_after_focus`, so the sweep
shows directly whether focusing a window costs a frame.

Binaries: `iterate` profile for the interaction sweeps, built from this
branch. For each fix, "before" is the same tree without that fix. Settings
(SPEED-02) uses the release profile (fat LTO).

## Changes

1. **Files icon view builds only the visible rows** (`e90422ce`). Before,
   an ungrouped icon grid laid out and painted every tile on every frame.
   Now it builds the rows in the viewport plus two on each side, with
   spacers that keep the scroll height. Grouped grids are unchanged.
2. **The Notes list builds only the visible rows** (`b3c6263d`). With 500
   notes, every scroll frame and every keystroke in a note spent about 80 ms
   laying out the whole list. A plain list (no search, no gallery, no tag
   rows, which vary in height) now builds the rows in the viewport plus four
   on each side.
3. **SPEED-02: a new window is drawn focused from its first frame**
   (`5cc6e065`, `f4b6144e`):
   - A focusable xdg toplevel now starts active. Lulo's niri focuses every
     new toplevel, but 75–210 ms after its first frame.
   - A keyboard enter that does not change the active state no longer wakes
     a frame or calls GPUI's activation handler.
   - A modifier event whose state the window has already seen is dropped.
     niri follows every keyboard enter with the unchanged modifier state,
     and GPUI redrew the window for it.
   - A window presumed active falls back to inactive in two cases: a
     configure arrives after mapping without the activated state, or
     another window of the same app gets the keyboard.
   - `RMAC_GPUI_PRESUME_ACTIVE=0` restores the old behaviour, so one binary
     can measure both sides.
4. **Harness, fixtures and trace rows** (`63fe7e38`, `2646f738`, `e0a6292b`, `8fdedacd`,
   `8dbde028`, `f4f4e3d2`, `62f20d6c`, `a7a26816`, `6f709536`).

## Results

| Interaction | Target | Before | After | |
|---|---|---|---|---|
| Files icon view, 2,000 items, scroll | ≥ 99 % ≤ 16.7 ms | 70.6 %, p95 116 ms, 10 stalls | **100 %**, p95 7 ms | fixed |
| Notes list, 500 notes, scroll | ≥ 99 % | 69.0 %, p95 84 ms, 13 stalls | **98.6–100 %**, p95 8–11 ms | fixed |
| Notes, typing in a 200 KB note | echo p95 < 16 ms | p50 250 ms, **p95 592 ms** | p50 7 ms, p95 25 ms | much better, not met |
| Files list view, 2,000 items, scroll | ≥ 99 % | 91–96 % (p95 17 ms) | 91–98 % (p95 15–18 ms) | not met |
| Mail list, 10,000 messages, scroll | ≥ 99 % | — (no fixture) | 95.3–96.9 %, p95 16 ms | not met |
| Settings panes scroll (Accessibility, Privacy, Keyboard) | ≥ 99 % | 99.3–100 % | 99.3 % | met |
| Terminal scrollback (20,000 lines) | ≥ 99 % | 100 %, p95 3 ms | 100 % | met |
| Terminal typing | echo p95 < 16 ms | 3.7 ms | 3.5 ms | met |
| Text Editor plain, 1 MB | echo p95 < 16 ms | 9–13 ms (97.7 % of frames) | 9–13 ms | met (echo) |
| Text Editor rich, 1 MB RTF | echo p95 < 16 ms | p50 5.5 ms, p95 101 ms | p50 5–7 ms, p95 9–96 ms | met in 1 of 2 sweeps (SPEED-13) |
| Spotlight typing | echo p95 < 16 ms; results < 50 ms | echo p95 34 ms; results 30 ms | same | results met; echo not met |
| Files: Quick Look open | < 100 ms | 24–25 ms | 24 ms | met |
| Files: open 2,000-item folder | first rows < 100 ms | — | first frame 5–20 ms; last frame 139–287 ms; worst frame 21–74 ms | first frame met; settling not |
| Files: 200 thumbnails | progressive, no stalls | inconclusive (see below) | | |
| ⌘Tab switch, key to new app's frame | — | 253 ms | 251–275 ms | not met (SPEED-11) |
| Minimise / restore | — | — | 61 ms / 47–59 ms | |
| Settings edge-resize drag | ≥ 99 % | 54–62 %, p95 32–47 ms | 64–79 %, p95 35–42 ms | not met (SPEED-12) |
| Mission Control animation | ≥ 99 % | 100 %, p95 12–13 ms | 97–100 % | met in 2 of 3 sweeps |
| **Settings settled (SPEED-02, release)** | **< 300 ms** | 374, 448, 386, 431 ms | 452, 433 (presumed active); 394, 448 ms (+ modifiers) | **not met** |

The raw reports are in `speed-round-7-2026-10-06/`:

- `interactions-before.json`: this branch before the Files and Notes fixes.
- `interactions-files-icon-fixed.json`: Files fixed, Notes still before.
- `interactions-notes-fixed.json`: Notes fixed.
- `interactions-final.json`: everything fixed.
- `settings-*.json`: the SPEED-02 release A/B.

## SPEED-02: what is left

These are release builds with 5 launches per sweep. The sides alternated:
before, after, before, after. Before means `RMAC_GPUI_PRESUME_ACTIVE=0`
with the same binary.

| | Before | Drawn active | Drawn active + modifier filter |
|---|---|---|---|
| First frame | 241, 281, 234, 288 ms | 258, 257 ms | 232, 317 ms |
| Settled | 374, 448, 386, 431 ms | 452, 433 ms | 394, 448 ms |
| niri focuses the window after its first frame | 73–126 ms | 88–212 ms | 73–131 ms |
| Frames presented after the focus | 1 | 1 | 1 |

A new window's first frame is now drawn active, as on the Mac, so the
traffic lights and sidebar selection no longer change colour about 100 ms
after the window appears. A keyboard enter no longer reaches GPUI as an
activation or a modifier event.

Settled time did not change, though: niri's activation is still followed by
one more frame. The trace now records configures and size changes, and it
shows the order of events:

1. `focus_in`.
2. The activation `configure`, with no size change.
3. About 30 ms later, a `draw_start` and a `present`.

At that point nothing GPUI can see is dirty: the app gets no activation
callback and no input. The next suspect is the renderer redrawing the
whole scene after a suboptimal swapchain. Mesa reports one when niri changes
the surface's dmabuf feedback, and niri may do that when it focuses the
window (`force_render_after_recovery` in
`shell/compat/gpui_linux/src/linux/wayland/window.rs`). The trace now has
a `force_render` row there; the next launch sweep shows whether that frame
is the one. Settled is 390–450 ms, so the 300 ms target is still not met.

## Why the rest is not met

- **Files list scroll (91–98 %).** Each scroll frame re-renders the whole
  Files window, including the sidebar, toolbar and visible rows, at about
  14–15 ms of CPU per frame (iterate profile). The rows are already
  windowed. Next: draw the sidebar and toolbar as cached views, as Settings
  does (`f8c77f5d`). A scroll then re-renders only the list and the window
  shell.
- **Mail list (95–97 %).** It already uses `uniform_list`, but every scroll
  frame re-renders the whole Mail root view at about 15 ms. The next step is
  the same as for Files.
- **Notes typing (p95 25 ms).** Typing is now steady at about 7 ms. A few
  keys take 20–25 ms. That is not traced yet.
- **Text Editor rich (p95 9–96 ms).** Typing is steady at 4–9 ms per key.
  In one sweep the first three keystrokes after opening a 1 MB RTF took
  100–260 ms, but the second sweep did not reproduce that (p95 9 ms). Most
  sweeps also show one frame of about 42 ms. The suspected cause is the 2 s
  recovery draft, which is built on the UI thread (`schedule_autosave`).
  That has not been confirmed.
- **Spotlight echo (p95 34 ms).** Most keys echo in about 1 ms. The slow
  ones come at a steady 30 fps cadence while results stream in, which
  matches GPUI's 33 ms frame throttle for inactive windows, so the panel
  seems not to count as active while `img()` icons load. The new
  `focus_in` trace rows show this the next time the launcher is built. In
  addition, the first key typed straight after reopening the panel can
  arrive before its window exists.
- **⌘Tab (≈ 250 ms).** The switcher deliberately waits `RELEASE_GRACE`
  (80 ms) after its surface gets the keyboard before it treats ⌘ as
  released (`shell/bins/rmac-app-switcher/src/main.rs`). Add the spawn of
  the `next` client and the surface's keyboard activation, and a quick tap
  costs about 250 ms. This is an accepted trade-off. It was tuned for
  correctness on the low-spec laptop and is left alone here.
- **Settings resize drag (54–79 %).** Every configure relays out the whole
  Settings window (p95 32–47 ms iterate).
- **Thumbnails.** In the scenario, switching the 200-PNG folder to icon view
  produced only two frames. The thumbnails apparently landed in one batch,
  or were not generated for synthetic PNGs, so the scenario does not yet
  show the progressive path. It needs a check against Files'
  thumbnail-size filter.
