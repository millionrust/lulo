# ADR 0028 — Touch input: gestures in the GPUI backend, touch targets in the UI

- Status: accepted
- Date: 2026-10-14
- Follows: ADR 0013 (vendored gpui_linux), ADR 0026 (vendored GPUI core),
  ADR 0027 (hardware profile)
- Branch: `op/touch-input`

## Context

The owner's EliteBook has an Atmel touchscreen, and Lulo should adapt to each
PC's hardware (ADR 0027). GPUI has no touch events at all. TOUCH-01 had taught
the vendored Wayland backend to bind `wl_touch` and turn one contact into a
click, a long-press right click, a scroll with the touchpad's momentum, or a
title-bar move, but:

- a second finger cancelled everything, so there was no two-finger scroll or
  pinch;
- the backend could not tell a slider or a Dock tile from a scrolling list, so
  dragging them with a finger scrolled instead (only Files had a window-wide
  heuristic);
- the scroll direction was inverted: a finger moving up scrolled the content
  down, the opposite of every touchscreen;
- small targets (traffic lights, menu bar items) stayed pointer-sized;
- niri mapped touches to whichever output came first in the layout, so with an
  external display on the left the panel's touches landed on the wrong screen;
- nothing could prove any of it: no nested compositor here injects touch.

## Decision

### Gestures live in the backend, as pointer events GPUI already understands

`shell/compat/gpui_linux/src/linux/wayland/touch_gesture.rs` is a pure
recogniser (no Wayland types, unit-tested); `client.rs` feeds it `wl_touch`
events and delivers its results:

| Touch | Becomes |
| --- | --- |
| Tap (under 8 px of movement) | Left `MouseDown`/`MouseUp`; a quick second tap is a double click |
| Rest 500 ms without moving | Right click (context menu) |
| One-finger swipe | `ScrollWheel` with Started/Moved/Ended phases, content following the finger, then the touchpad's kinetic momentum |
| Move on a `touch_drag` element or a title bar | Held left button: `MouseDown`, `MouseMove`s, `MouseUp`; a title bar starts niri's move with the touch serial |
| Rest 160 ms, then move, on an `on_drag` element | The same held-button drag (a quick swipe over it still scrolls) |
| Two fingers moving together (midpoint past 10 px) | `ScrollWheel` at the midpoint, with momentum after lifting |
| Two fingers spreading or closing (spread past 8 %) | `Pinch` with the change in scale since the last event, as the touchpad pinch reports it |
| Quick two-finger tap (under 350 ms) | Right click at the midpoint |
| Three or more fingers | Nothing, until every finger lifts |

A second finger joins the first only on the same window and before the first
has started dragging; once a two-finger gesture is a scroll or a pinch it stays
one. The other finger is ignored after the first lifts.

### GPUI core tells the backend what is under the finger

Two small additions to the vendored core (`shell/compat/gpui/src/rmac_touch.rs`):

- `TouchDragMode::{Immediate, AfterHold}`. `InteractiveElement::touch_drag()`
  marks an element (and everything inside it) as finger-draggable at once; any
  element with `on_drag` gets `AfterHold` automatically. Painting records these
  hitboxes beside the window-control ones (and replays them for cached views);
  the platform asks through `PlatformWindow::on_hit_test_touch_drag`, a default
  no-op for other backends. `Immediate` anywhere under the finger wins, so a
  slider's own `on_drag` thumb does not make it wait.
- `last_input_was_touch()`: the backend sets it on a touch down and clears it
  on a pointer button, axis or a real pointer move (a re-sent position after a
  window change does not count). Each window refreshes on its next event when
  it changes.

Marked `touch_drag`: rmac-ui's `Slider` (System Settings and every app), the
Control Centre Display and Sound sliders, Dock tiles and the Dock separator.
Items that use `on_drag` (Files rows and sidebar, Launchpad icons, Clock's list) drag after the short
rest.

### Touch targets grow only while touch is in use

`rmac_shell_ui::touch` (re-exported as `rmac_ui::touch`): `touch_targets()` is
`last_input_was_touch()` and the hardware profile's `touch_targets` (a
touchscreen; without a profile in the process, the stored one; without either,
the touch itself is the proof). `hit_size` grows a hit box by `TOUCH_SLOP`
(6 px) on each side and takes it back with an equal negative margin, so nothing
moves and nothing is drawn differently. The traffic lights reach into half the
gap beside them (3.5 px); menu bar status items and Spotlight reach to the
bar's top and bottom edges. A mouse move puts everything back.

### niri

Touch already reaches windows and layer-shell surfaces (Dock, menu bar,
Control Centre, Spotlight, Launchpad, Notification Centre, Mission Control)
through niri's `contents_under`, and an `xdg_toplevel.move` with a touch serial
starts niri's touch move grab. `packaging/rmac-session/shell.kdl` now maps
touch to `eDP-1`, the laptop panel, so touches stay on it whatever the output
layout; niri falls back to the first output when there is no `eDP-1`, and
System Settings' managed input include replaces the whole `touch` block when
the user picks another output.

### Proof without a touchscreen

`scripts/behavior/run_touch_input.py` starts a private headless GNOME Shell
(Mutter) on its own session bus with a temporary HOME and runtime dir, and
injects touches through Mutter's RemoteDesktop D-Bus API into
`shell/probes/touch`, a full-screen GPUI window on this backend whose regions
log what each touch became. It is the only compositor available here that
delivers real `wl_touch` events without a device (niri nested, Sway and cage
have no virtual touch protocol; `/dev/uinput` is root's). Layer-shell surfaces
cannot run under Mutter, but they share the backend code path the probe
exercises. It needs gnome-shell, pipewire and wireplumber, so it runs on the
reference laptop, not in CI; CI runs the recogniser's unit tests
(`cargo test -p gpui_linux --lib touch` in shell/).

## Not done

- Touch-specific feedback (press highlights without hover), edge swipes for
  Notification Centre or Mission Control, and on-screen keyboard
  auto-show are separate items.
- Pen input arrives as `wl_touch` only if the compositor sends it that way;
  tablet-v2 is not handled.
- End-to-end runs on the EliteBook's own panel wait for the owner.

## Consequences

- Any GPUI surface gets touch through the backend; new surfaces only add
  `.touch_drag()` where a finger should drag rather than scroll.
- Two small, additive patches to the vendored GPUI core and one `PartialEq` on
  `TouchPhase`; a GPUI bump must re-apply them (ADR 0026).
