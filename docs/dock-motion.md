# Dock magnification and visibility

`rmac-dock::motion` defines the framework-neutral behavior behind D6. It does
not own a pointer, timer, animation frame, output, or layer surface; the future
view translates platform events into deterministic inputs and schedules only
the deadline returned by the model.

## Magnification

Each icon has a stable base center. Pointer distance from those stable centers
produces a bounded smoothstep scale within one influence radius. After scales
are known, the nearest slot remains anchored and expanded icon widths push
neighbors outward with the configured gap.

Rendered centers never feed back into the next scale calculation. Identical
pointer/configuration input therefore returns byte-for-byte identical geometry
instead of oscillating around a moving hit target. Tests cover symmetry,
maximum scale, finite/range validation, gap preservation, deterministic replay,
and the unscaled/reduced-motion path.

The layout is one-dimensional and applies equally to bottom, left, and right
placement; the renderer maps its axis to screen coordinates. Reduced motion
sets every scale to one rather than replacing magnification with a different
spatial effect.

## Autohide

Pointer leave enters a 500 ms waiting state and returns one absolute deadline.
That state is still visually visible and requests no animation/frame. At the
deadline, the shelf hides only if autohide/fullscreen is still effective, the
pointer is outside, and overview is closed.

A hidden shelf requires 150 ms of continuous reveal-edge pressure. Leaving the
pressure edge resets the dwell. Pointer entry reveals immediately. Overview
forces the Dock visible; fullscreen applies hide policy even when ordinary
autohide is off. Ending fullscreen restores the configured policy. The direct
niri adapter and `rmac-dock-runtime` now carry typed initial/live overview state
into the renderer boundary without focus or geometry inference.

State changes and visual hidden-boundary changes are reported separately.
Reduced motion keeps identical timing/visibility semantics but marks visual
transitions non-animated. The UI must cancel obsolete scheduled deadlines and
must not create a polling or unconditional frame loop.

This model is not D6 completion. Niri 26.4 does not expose real fullscreen state
on its IPC `Window`, so the project does not guess it; real fullscreen behavior
must come from niri's layer-shell stacking and reference-PC proof. Layer-surface
pressure behavior, pointer capture, autohide reservation policy, live reduced-
motion wiring, 60/120 Hz frame evidence, combined-shell idle measurements, and
multi-output hardware validation remain pending.
