# ADR 0029 — Edge-drag tiling through one more niri event

- **Status:** accepted 2026-10-20.
- **Amends:** ADR 0021 (niri carries Lulo patches that report, rmac decides).
- **Scope:** `packaging/third-party/niri/debian/patches/0003-ipc-report-window-move-edges.patch`,
  `crates/rmac-compositor-niri/src/edge_drag.rs`, `shell/bins/rmac-mission-control`
  (`edge_drag`, `EdgePreview`, the Arrange commands), `crates/rmac-ui/src/chrome.rs` (`zoom_menu`).

## Context

On macOS 26 a window dragged to a screen edge tiles there when it is let go: the left or right
edge gives a half, a corner a quarter, the top edge the whole screen. A frosted outline shows the
tile while the pointer rests at the edge. Lulo already had the tiles (Window ▸ Move & Resize, the
green button, 🌐⌃ + arrow, WIN-01) but no edge drag (docs/parity.md DESK-03).

A window drag is niri's interactive move. Nothing outside niri sees the pointer during it: the
client only asked for the move, and niri's event stream reports positions only after layout.

## Decision

1. **niri reports where the drag is, and nothing else**, the way ADR 0021 reports minimise
   requests. Patch `0003-ipc-report-window-move-edges.patch` (about 140 lines) tracks, in niri's
   move grab, which edge or corner the pointer is at (2 px from an edge; within 48 px of a corner
   along it) and sends two transient event-stream events: `WindowMoveEdgeChanged { id, edge }`
   when that changes, and `WindowMoveEnded { id, edge }` on release. `edge` is `left`, `right`,
   `top`, `bottom`, the four corners, or null. niri still leaves the window where it was dropped.

2. **Lulo's window service decides and acts.** `rmac-mission-control` already runs the tiles. It
   maps an edge to a tile (`EdgeTile`: top → Fill, sides → halves, corners → quarters; the bottom
   edge, which holds the Dock, tiles nothing, as on the Mac), shows `EdgePreview` (a
   `rmac-tile-preview` layer surface on the Top layer, no input, no keyboard) while the pointer is
   at an edge, and on release records the tile history (so 🌐⌃R returns the window) and applies
   the same floating-frame action as Move & Resize.

3. **The green button's Fill & Arrange row** gains the Mac's three Arrange layouts: Left & Right,
   Top & Bottom, Quarters. The focused window takes the first region and the app's other windows
   on its Space the rest, most recently used first (`model::arrange`); minimised windows stay put.

## Consequences

- An unpatched niri (a developer's nested session) sends neither event; edge drags then do
  nothing, as before. Nothing polls.
- The preview is drawn over windows, translucent, rather than under the dragged window as macOS
  does: a layer surface cannot sit between two toplevels in niri.
- The patch stays small enough to rebase onto each niri release, like 0001 and 0002.
