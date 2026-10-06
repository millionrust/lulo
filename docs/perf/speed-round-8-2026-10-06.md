# Speed round 8 — 2026-10-06: lists, ⌘Tab and what the trace shows

Round 7 left five items open: the Files list and Mail list scrolls, Spotlight's
key echo, ⌘Tab, Settings settling and its resize drag, and the first keys in a
1 MB RTF. This round fixes the ones whose cause was in our code and traces the
others to their cause.

Branch `op/speed-8` on `integ` 77d9111c. The method is the same as round 7: a
private nested niri on the GPU path at 1920×1080 on the reference laptop,
never the owner's session, `run_speed_sweep.py --only …` with `--repeat 3`.
`ps` showed nothing above 8 % CPU before each sweep, and each sweep held the
build lock. "Before" is `integ` built with the `iterate` profile; "after" is
this branch, same profile, same laptop, back to back. Settings settling is
also measured with the release profile (fat LTO).

## Harness changes

- `cmd-tab` now also reports the switcher's own first frame.
- New `cmd-tab-hold`: hold ⌘ (Super; niri's Mod is Alt in the nested
  session, so the bind fires on Alt-Tab and Super is held as well), time the
  Tab stroke to the panel's first full-size frame, then time releasing ⌘ to
  the chosen app's next frame.
- Scroll and resize scenarios split each frame into render (GPUI's render,
  layout, prepaint and paint: `frame_callback` → `draw_start`) and submit
  (`draw_start` → `present`).
- Typing scenarios report the first three key echoes (`first_keys_ms`) and
  `echo_after_frame_callback_p95_ms`, the echo minus any wait for the
  compositor's next frame callback.
- App launches report `draw_skips` and `force_renders`.
- The frame trace now starts with `monotonic_origin` (its zero on
  CLOCK_MONOTONIC), so rows from several processes line up; it also names the
  window on every focus change (`focus_window:<app id>:<surface>`), and
  records `idle_inactive` for a frame an inactive window did not draw and
  `appearance_changed` for a whole-window repaint the platform asked for.

## Changes

1. **Files: sidebar, toolbar and content area are cached views** (SPEED-08).
   A wheel scroll notified `FinderView`, so every frame rebuilt and laid out
   the whole window. The list and icon grid now notify only the content view;
   the root shell re-renders and the sidebar and toolbar reuse their last
   layout and paint. Every `cx.notify()` on `FinderView` still repaints all
   three. They draw uncached while an assistive technology is connected, as
   Settings does.
2. **Mail: the conversation list is a cached view and builds its rows once
   per change** (SPEED-09). Each frame used to clone five strings for each of
   10,000 messages before `uniform_list` picked the visible rows. The list
   view now keeps the rows until `MailView` notifies.
3. **⌘Tab** (SPEED-11). The release grace after the switcher gets the
   keyboard is 16 ms instead of 80 ms. The trace shows niri's modifier state
   arriving 0.1 ms after the keyboard enter. When the modifier state shows ⌘
   still held, the panel grows to full size at once instead of after the
   120 ms reveal delay. A quick tap still never shows the panel.
   Tried and dropped: sending the activation before closing the switcher
   surface (so the keyboard goes straight to the chosen app, not back to the
   previous one first). niri then focused the chosen app 107 ms after the
   commit instead of 50 ms, so the switch took 181 ms
   (`interactions-after-2.json`).
4. **Settings: no whole-window repaint for repeated window capabilities**
   (SPEED-02). niri repeats `wm_capabilities` with later configures, its
   activation configure included, and the backend called GPUI's appearance
   handler for every one, which re-renders the whole window. That was the
   frame about 20–30 ms after focus. Unchanged capabilities and unchanged
   decoration modes no longer repaint. The new `force_render` count was 0 in
   every launch, before and after, so a suboptimal swapchain was not the
   cause.
5. **Text Editor: the recovery draft is serialised off the UI thread**
   (SPEED-13). The 2 s autosave wrote 1 MB of RTF on the UI thread in the
   middle of typing. It now takes a snapshot of the document (its paragraphs
   are shared) and writes the RTF on the background executor.

## Results

Iterate profile unless noted. Before is `interactions-before-{1,2}.json`.
After is `interactions-after-{1,3}.json` and the Files, Mail, TE and resize
rows of `interactions-after-2.json`.

| Interaction | Target | Before | After | |
|---|---|---|---|---|
| Files list view, 2,000 items, scroll | ≥ 99 % ≤ 16.7 ms | 92.7 %, 98.2 % (render p50 11.5 ms, p95 15–18 ms) | **100 %** (render p50 7.7 ms, frame p95 11.9 ms) | fixed |
| Files icon view, 2,000 items, scroll | ≥ 99 % | 100 % (round 7) | 100 %, p95 5.6 ms | holds |
| Mail list, 10,000 messages, scroll | ≥ 99 % | 96.9 %, 98.5 % (render p50 11.6 ms) | **100 %**, 100 % (render p50 4.5 ms, frame p95 5.6–7.1 ms) | fixed |
| Spotlight typing, echo p95 | < 16 ms | 33.5–37.3 ms | 33.2–40.9 ms | not met; cause found (below) |
| ⌘Tab quick tap, key to new app's frame | < 100 ms | 248–249 ms | **137–141 ms** | better, not met |
| ⌘Tab quick tap, switcher's first frame | — | 44 ms | 43–50 ms | |
| ⌘Tab held, Tab to full-size panel | < 50 ms | 165–166 ms | **124–131 ms** | better, not met |
| ⌘Tab held, release ⌘ to new app's frame | — | 58 ms | 54–58 ms | |
| Settings settled (iterate) | < 300 ms | 239–507 ms; 1 frame after focus in 4 of 6 | 246–375 ms; 1 frame after focus in 4 of 6 | see release row |
| Settings settled (release, 5 launches) | < 300 ms | 374–448 ms (round 7) | 260, 280, 350, 372, 382 ms (median 350); 1 frame after focus in 4 of 5 | better, not met |
| Settings edge-resize drag | ≥ 95 % | 25 % (8 frames) | 50–55 % (10–11 frames), render p50 7.7 ms, p95 32–39 ms | not met |
| Text Editor rich 1 MB, echo p95 | < 16 ms | 13.9 ms; first keys 31, 7, 9 ms | **11.1–11.5 ms**; first keys 25, 7–9, 5–11 ms | met |

## What is left, and why

- **Spotlight echo.** It is not GPUI's 30 fps throttle for inactive windows.
  The new trace rows show the launcher surface getting the keyboard
  (`focus_window:org.rmac.Launcher`) and never skipping a frame while active.
  The slow key is always the second one. The first key brings in results,
  and the launcher grows its layer surface to show them. The surface then
  waits for niri's configure for the new size, and niri sends no frame
  callback until then, so the second key waits 26–33 ms for it. Keys after
  that echo in about 1 ms. Next: open the launcher at its full results
  height (transparent below the field, with the input region limited to
  what is drawn), so typing never resizes the surface. The launcher has no
  compositor blur or shadow rule, so the transparent area shows nothing.
- **⌘Tab.** On the cross-process timeline (`monotonic_origin`), a quick tap
  costs: spawning the `next` client 12–14 ms; opening the switcher surface
  to its first frame 28–33 ms (7 ms renderer, about 22 ms niri's map
  configure); niri giving it the keyboard about 20 ms later; the 16 ms grace;
  then about 50 ms until the chosen app has focus and draws. The held case
  pays the map, the keyboard and then a resize round trip (about 59 ms
  before niri's next frame callback). To go below 100 ms and 50 ms the
  switcher has to stop creating a surface per ⌘Tab: keep one hidden surface
  mapped at full size and re-show it with `set_mapped` (as Mission Control
  does) and request its map configure ahead of time.
- **Settings settling.** The repeated-capabilities repaint is gone, but one
  frame still follows the activation configure in most launches. With no
  `appearance_changed` or `force_render` row in front of it, the remaining
  candidate is pointer and keyboard enter events (hover and focus refreshes
  in GPUI). The iterate sweeps vary a lot (239–507 ms before), so the release row is the result: median 350 ms
  (`settings-after-release.json`), down from 386–452 ms.
- **Settings resize drag.** Every resize is a GPUI refresh, which re-renders
  cached views too, so each step lays out the whole Settings window: 32–39 ms
  at p95 with the iterate profile. During the drag the nested niri also held
  frame callbacks back for up to 300 ms, so each sweep scores only 8–11
  frames. Neither is fixed here.
- **Text Editor first keys.** Neither sweep reproduced round 7's 100–260 ms.
  The first key costs 25–31 ms, all of it in GPUI's render (20 ms between the
  frame callback and `draw_start`); later keys take 5–9 ms. The autosave frame
  (about 42 ms in round 7) is gone.
