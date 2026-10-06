# Speed round 9 — 2026-10-06: Spotlight, ⌘Tab, resizing and settling

Branch `op/speed-9` on `integ` dcef1e00. The method is the same as rounds 7
and 8: a private nested niri at 1920×1080 on the reference laptop, never the
owner's session. `ps` showed nothing above 8 % CPU before each sweep, and each
sweep held the build lock. "Before" is `integ` and "after" is this branch,
both built with the `iterate` profile on the same laptop, back to back.
Settings settling is also measured with a release build (fat LTO). The raw
reports are in `speed-round-9-2026-10-06/`.

## Harness

- New `resize-drag-long`: niri's own interactive resize (Mod + right-drag),
  three seconds each way, with configures and resizes counted. The old
  `resize-drag` pressed exactly on the window edge, which often missed.
- Key echo now finds key-downs exactly from the new `draw_for_key` row. The
  45 ms gap rule misread a key release that arrived late as the next press,
  which made some fast echoes look like 30–50 ms ones.
- `wlinput.drag` can hold modifiers.
- New trace rows: `draw_resized`, `draw_for_key`, `renderer_resized`, and app
  marks through the new `gpui_linux::trace_mark` (the App Switcher records
  `switcher_reshown`, `switcher_commit`, `switcher_hidden` and
  `switcher_activated`).

## Changes

1. **Spotlight opens at its full height and never resizes while typing**
   (SPEED-10). The layer surface is created at the expanded height. Below the
   bar it is transparent, and while the bar is compact the input region
   covers only the bar, so presses below it reach the outside-click catcher
   exactly as before. The launcher's glass is drawn by the client (there is
   no compositor blur rule), so it looks the same as before. Typing no longer
   resizes the surface, so no key waits for niri's configure.
2. **A key press draws its echo at once** (`gpui_linux`). It used to wait
   for the frame callback of the frame before it. In a window that was still
   drawing (Spotlight streaming results), that cost up to a whole compositor
   frame, which is about 30 ms in the nested session. The window now draws
   right after the key, as a parked window always did. A window with nothing
   new to show draws nothing.
3. **A new size is drawn at once** (`gpui_linux`). niri holds a window's
   frame callbacks while it waits for the buffer that answers its resize
   configure. A window that had just drawn therefore never drew the new size
   until the transaction timed out.
4. **⌘Tab keeps its switcher surface** (SPEED-11). A switcher that was
   never revealed (a quick tap) is unmapped instead of destroyed, and it
   requests its next configure at once. The next ⌘Tab maps it again, as
   Mission Control does (round 6), with no new surface, swapchain or map
   round trip. The release grace is 4 ms (it was 16 ms). The trace shows
   niri's modifier state 0.1 ms after the keyboard enter. A revealed
   switcher still closes: shrinking it back to 1 × 1 rebuilds its swapchain,
   and niri then took about 500 ms to give the re-mapped surface the
   keyboard. The catalog re-read after a switch now waits 1 s so it does not
   compete with the chosen app's frame. The `next` client is still spawned
   by niri's bind (about 14 ms); niri can only run a command for a bind.
5. **Settings focuses its search field before the first frame** (SPEED-02).
   It used to focus the field while rendering the first frame. GPUI then
   refreshes the whole window after that frame, and niri drew that refresh
   only when it activated the window, about 90 ms later. A newly focused
   field also draws its caret in the frame that focuses it (vendored
   `BlinkCursor`), instead of in one more frame. Settled time improved
   (iterate median 470 → 391 ms back to back), but one frame still follows
   the activation in most launches, so this was not the whole cause.

## Results

| Interaction | Target | Before | After | |
|---|---|---|---|---|
| Spotlight key echo p95 | < 16 ms | 32.9–34.3 ms | **5.3–7.1 ms** | met |
| Spotlight results | < 50 ms | 31 ms | 30 ms | met |
| App Switcher panel open (`next` → switcher frame) | < 100 ms | 35–84 ms (median 76) | 4–72 ms (median 10; 72 for the first open) | |
| ⌘Tab quick tap, key → new app's frame | < 100 ms | 140–182 ms (median 150) | 101–106 ms (median 104; 162 for the first) | almost met |
| ⌘Tab held, Tab → full panel | < 50 ms | 124–175 ms (median 129) | 113–160 ms (median 128) | not met |
| ⌘Tab held, release → new app's frame | — | 58–71 ms | 44–54 ms | |
| Settings interactive resize | ≥ 95 % | 58 % of 12 frames (8 configures in 6 s) | 42–55 % of 11–12 frames | not met (cause below) |
| Settings settled, iterate, 5 launches each, back to back | < 300 ms | 388–493 ms (median 470); frame after focus in 5 of 5 | 314–510 ms (median 391); 5 of 5 | better, not met |
| Settings settled, release, 2 × 5 launches | < 300 ms | 260–382 ms, median 350 (round 8) | 267–368 ms (median 306), then 305–484 ms (median 411); frame after focus in 1 of 5, then 5 of 5 | not met; noisy |

The owner's live session started on the laptop at 21:43 (niri-session plus
Files), during the Settings sweeps. Those runs are noisier than the
interaction sweeps, which finished before it started.

## Why the rest is not met

- **Settings settling.** Focusing the search field before the first frame
  did not remove the frame after activation. The trace still shows no frame
  callback between the first present and niri's activation, about 90 ms
  later. That frame then renders something that became dirty after the
  first frame (20–30 ms of render). The remaining candidates are the
  background snapshot loads that land in that window and the focus
  listeners GPUI runs after the first frame. The next step is to add a
  trace mark when a view is notified.

- **Settings resize.** The new trace row `renderer_resized` shows that
  `update_drawable_size` takes 300–470 ms on each resize configure. The
  swapchain rebuild (the Vulkan WSI in `wgpu`'s `surface.configure`)
  blocks until niri replies. niri is itself waiting for this window's buffer
  at the new size, so every step lasts until niri's transaction gives up.
  GPUI's render is not the problem: render p50 is 7 ms. The fix is not to
  rebuild the swapchain on every configure. Render into a swapchain sized in
  steps larger than the window and crop it with `wp_viewport`, or rebuild it
  only after the drag ends. That is a renderer change for the next round.
- **⌘Tab held.** Revealing the panel resizes the surface, which pays the
  same swapchain rebuild (about 58 ms), plus niri's focus about 20 ms after
  the map. Next: create the switcher's swapchain at the panel size from the
  start and show it at 1 × 1 through `wp_viewport` until it is revealed.
  The reveal then only changes the viewport.
- **⌘Tab quick tap.** The tap now costs: spawning `next` (14 ms), mapping
  the kept surface and drawing it (3 ms), niri giving it the keyboard
  (about 23 ms, one nested-niri frame), the 4 ms grace, then about 50 ms
  for niri to hand the keyboard back and process the focus request. Each of
  those last steps is one frame of the nested niri, which runs at about
  30 Hz here. At 60 Hz they should take half as long.
