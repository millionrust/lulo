# Dock behaviour: macOS 26.2 against rmac (2026-09-23)

The Dock's look was already matched. This pass covers what it does. Every
number below comes from the owner's Mac (macOS 26.2, dark mode, Dock at the
defaults: 64 pt tiles, magnification off, auto-hide off, recent apps on,
minimise with Genie into the right-hand section). No Dock preference was
left changed. Auto-hide was switched on with ⌥⌘D and straight back off.

## How it was measured

- **Structure.** System Events → process "Dock" → list 1 → UI elements
  gave each item's frame and subrole. `AXIsApplicationRunning` told running
  apps from closed ones.
- **Menus.** `perform action "AXShowMenu"` opened each menu. The item tree
  was dumped with names, enabled state and mark characters, then
  screenshotted with and without Option held.
- **Motion.** `screencapture -v` recorded 60 fps video, regions only. Frames
  were extracted with their timestamps (ffmpeg `showinfo`). Frame-to-frame
  difference boxes gave when things started and stopped moving. A column
  scan of the icon's top edge gave the bounce curve.
- **Pointer.** A small CGEvent tool (`move`/`down`/`drag`/`up`) produced
  drags, run from Terminal, which holds the Accessibility grant.
- **Apps used.** Only Calculator was launched, minimised, restored and quit
  (by me). Its window was closed with ⌘W. Nothing else was opened or quit.

## Geometry

The item pitch is 68 pt. Each AX item frame is 68 × 84 and starts at the
shelf's top. The list frame was 1166 × 84 for 17 items plus 2 separators.
Separators are 30 pt AX items.

The owner's Dock is laid out as: kept apps │ recent apps (3 Chrome entries
plus a Handoff tile) │ Bin. It has no folder stacks, so **stacks were not
recorded**.

## Measured behaviour

| Behaviour | macOS 26.2 (measured) |
|---|---|
| Click a closed kept app | The tile bounces while the app starts and a dot appears. **One bounce is 680 ms. The tile lifts 18 pt, a parabola 4p(1−p).** At 17 % of the period it was 10/18 of the way up, at 81 % it was 12/18. A warm launch bounces exactly once: the tile finishes the bounce that is in progress when the app is up. |
| App launched from elsewhere (`open -a`) | It is added to the recent section at the right-hand end. The shelf grows and the new tile grows in and bounces over 0.47 s (5.45 → 5.92 s). The oldest recent app that is not running shrinks away at the same time. |
| Click the frontmost app | Nothing happens. No change was recorded in the 1.5 s after the click. |
| Click a running app with no windows | It opens a new window. After ⌘W, Calculator quit; the next click relaunched it and bounced once (0.68 s). |
| Click an app whose windows are all minimised | The newest minimised window comes back. |
| Hover | The name label appears **in the first frame after the pointer arrives**: no delay and no fade. It is a dark capsule with a small pointer, centred above the tile. Moving to the next tile moves the label in the next frame. **Tiles show no hover highlight.** |
| Press | The icon darkens (the same dim as a tile whose menu is open). The label stays up. |
| Drag a kept app | Once the pointer moves, the icon lifts and follows it freely; the label disappears. The slot stays open. **A neighbour slides into the gap (about 270 ms) only after the pointer passes that neighbour's centre.** Dropping settles the icon into its slot in about 250 ms. |
| Drag a recent app | Its slot closes at once: the shelf shrinks over 0.25 s. |
| Drag off the Dock | **A "Remove" label appears above the icon when the pointer is about 96 pt (1.5 tiles) above the shelf.** It fades in over about 0.1 s. Releasing there fades out the icon and the label over about 200 ms. No poof sprite showed up at 60 fps. Releasing back in the Dock restores the slot (0.25 s). |
| Minimise (⌘M) | A Genie animation of about 0.5 s runs into a new tile between the second separator and the Bin, while the shelf widens. The tile is a window thumbnail with the app's icon as a badge. |
| Click the minimised tile | The window comes back with the reverse Genie (about 0.5 s). The tile slides out and the shelf shrinks. |
| Recent apps | Up to 3 apps that are not kept in the Dock. Running ones have a dot. **Quit apps stay, without a dot** (Calculator stayed after it quit). New apps join at the right-hand end. The oldest non-running app is evicted. |
| Running app menu (Terminal) | The windows come first, and the current window is ticked, with a window glyph. Then a separator, the app's own items (New Window, New Window with Profile ▸, …), a separator, **Options ▸** (Keep in Dock ✓, Open at Login, Show in Finder), a separator, then Show All Windows, Hide and Quit. **With Option held: Hide Others and Force Quit** take the place of Hide and Quit. |
| Closed kept app menu (FaceTime) | The app's static items, a separator, Options ▸ (Remove from Dock, Open at Login, Show in Finder), a separator, then Open. With Option held, Open becomes Force Quit. |
| Finder menu | Windows (ticked), New Finder Window, New Smart Folder, Find…, Go to Folder…, Connect to Server…, recent folders, Show All Windows, Hide. There are no Options and no Quit. |
| Bin menu | Open, a separator, then **Empty Bin, which is disabled while the Bin is empty**. The labels are en_GB; rmac says Trash. |
| Apps (Launchpad) menu | Remove from Dock, a separator, Show Apps. |
| Auto-hide (⌥⌘D) | **The Dock starts sliding out one frame (33 ms) after the pointer leaves and slides for about 190 ms.** With the pointer at the screen edge it **comes back after 200 ms and slides in over about 180 ms**. The glass shelf moves with it. |
| Attention and badges | Not triggered: no harmless app could ask for attention or badge on demand. The documented behaviour is continuous bouncing until the app is activated, and a red count badge. |
| Two-finger scroll → App Exposé | Not recorded: rmac has no per-app Exposé to match. |

## rmac before and after this pass

| Behaviour | Mac | rmac before | rmac now |
|---|---|---|---|
| Click a closed app | Launch and bounce | Launched, no bounce | ✓ Bounce, measured curve (`crates/rmac-dock/src/bounce.rs`) |
| Click a background app | Brings every window forward | Focused one window | ✓ `Activation::FocusApplication` focuses back to front (`crates/rmac-dock/src/dock.rs`, `crates/rmac-dock-system/src/execution.rs`) |
| Click the frontmost app | Nothing | Cycled windows by default | ✓ Default is Do Nothing; Cycle Windows is an option |
| All windows minimised | Restores the newest | Launched again | ✓ `Activation::RestoreWindow` |
| Launch from elsewhere | One bounce | None | ✓ One bounce |
| Attention | Continuous bounce | A red dot | ✓ Bounce from niri urgency or LauncherEntry `urgent` |
| Badges and progress | Count badge and progress bar | None | ✓ From `com.canonical.Unity.LauncherEntry` (`crates/rmac-dock/src/badges.rs`). Geometry not yet measured |
| Hover | Label at once, no highlight | Label, plus a fade to 88 % | ✓ No hover fade |
| Press | Icon darkens | Nothing | ✓ Dim overlay while pressed |
| Label during drag and menu | Hidden | Shown | ✓ Hidden |
| Menu order | Windows, commands, Options ▸, Open/Quit | Open first, flat list | ✓ Mac order with an Options submenu (`crates/rmac-dock/src/menu.rs`) |
| Keep in Dock | Ticked when running and kept; Remove from Dock when closed | "Remove from Dock" always | ✓ |
| Open at Login | In Options | Missing | ✓ XDG autostart through `rmac-login-items-linux` |
| Option → Force Quit | Relabels live | Only worked on click | ✓ Relabels from pointer-event modifiers (the Dock has no keyboard focus, so it updates on the next pointer move) |
| Show All Windows, Hide, Hide Others | Present | Missing | ✗ Omitted: niri has no app hiding or per-app Exposé |
| Bin menu | Open, then Empty Bin (disabled when empty) | "Open Trash" only | ✓ Open, then Empty Trash with a review step and a confirmation alert. The alert text is the Mac's; its geometry was not measured |
| Drag reorder | Icon follows the pointer, slot stays open, neighbour moves past its centre with a 270 ms slide | Instant swap along the axis, thresholds taken from a different layout | ✓ `crates/rmac-dock/src/reorder.rs` fed the drawn centres |
| Drag off to remove | Remove at 1.5 tiles, 200 ms fade | Missing | ✓ Kept apps only; recent apps cannot be dragged out yet |
| Drop a file on an app | Opens it with the app | Missing | ✓ For apps declaring MIME types, through `gio launch` |
| Drop on the Bin | Moves the items to the Bin | Missing | ✓ Through the `trash` crate |
| Drop an app on the Dock | Kept at the drop point | Missing | ◐ Kept at the end of the kept apps. rmac Files cannot start a drag yet (GPUI) |
| Recent apps | 3, quit apps stay without a dot | Running only | ✓ `crates/rmac-dock/src/recents.rs`, persisted in `$XDG_STATE_HOME/rmac/dock-recents` |
| Insert and remove animation | Grow or shrink over 0.47 s | Instant | ✗ The shelf material is a separate layer surface that is reopened when its size changes |
| Minimise into the Dock | Genie into a thumbnail tile | Tile with a thumbnail, no animation | ◐ Genie needs compositor support |
| Auto-hide | 0 ms hide delay, 200 ms reveal dwell, 190 ms slide | 500 ms delay, instant reveal, icons faded, shelf material left on screen | ✓ Measured timings, slide, and the material hides too |
| Stacks | Fan, grid or list | None | ✗ Not in the owner's Dock and not recorded |
