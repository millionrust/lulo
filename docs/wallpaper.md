# Wallpaper and desktop foundation

`rmac-wallpaper` is the compositor- and framework-neutral D9 contract. The
session-owned `rmac-shell-settings` snapshot remains the choice authority; a
render process does not infer wallpaper state from its own view or from a
desktop portal response.

## Sources and original default

The safe default is `builtin:rmac-aurora`: original rmac procedural-gradient
metadata with four sRGB palette colors. It is not an Apple wallpaper, a traced
asset, or a redistributed bitmap. A future renderer generates it directly at
the output size, avoiding decode, scaling, and licensing ambiguity.

User sources accept only that built-in ID, a hostless local `file:///` URI, or
a normalized absolute path. Remote schemes, relative paths, parent traversal,
unknown built-ins, and hosted file URIs are rejected during settings validation
and again at the wallpaper domain boundary.

`rmac-wallpaper-system` then opens the selected local path once. It requires a
nonempty regular file no larger than 512 MiB, recognizes PNG, JPEG, or WebP by
magic bytes rather than extension, rewinds the validated handle, and transfers
that handle to the decoder. This avoids reopening a potentially replaced path.
Default errors and `Debug` output redact the private path.

Resolution is per output. A missing, empty, oversized, unreadable, or unknown
custom file records only a typed output/error kind and substitutes Aurora on
that output; already resolved peers remain present.

## Per-output and hotplug behavior

Each enabled compositor output with valid logical geometry receives exactly
one deterministic background-surface plan. A named output selection overrides
the default; otherwise the default applies. An invalid selection falls back to
Aurora only on that output and records a typed issue without embedding the
source value. Disabled or unplugged outputs produce no surface. Replugging the
same stable output ID restores its persisted selection.

Fill uses uniform cover/crop, Fit uses uniform contain/letterbox, Stretch fills
both axes, Center preserves one image pixel per physical output pixel, and Tile
repeats at that same natural scale. Geometry is computed in logical coordinates
using the output scale and rejects zero, non-finite, or invalid inputs.

## Live session runtime

`rmac-wallpaper-runtime` watches the direct niri output authority and the
versioned shell-settings store. It waits until both have resolved as healthy or
explicitly unavailable before its first publication. A transient disconnect or
settings read failure changes source health but preserves the last-known-good
output plan and wallpaper choices.

Only a changed plan or selected-file event produces a render update. File
validation, decode, procedural rasterization, and fit layout run on the blocking
pool before publication; the renderer receives ready RGBA data. A health-only
update does not reopen a file, decode content, or request a frame. Watch setup
for the settings authority retries after a bounded delay, filesystem bursts
remain coalesced by the authority watcher, and closing the consumer ends the
runtime cleanly. Runtime debug/error formatting redacts source details.

The decoder enables only PNG, JPEG, and WebP. It rejects zero or over-16,384
axes, more than 40 megapixels, and codec allocations above the RGBA pixel bound.
Custom images are fingerprinted by redacted canonical identity, size,
modification time, and format, then shared across outputs in a 256 MiB LRU.
Aurora is cached at target physical size. Eviction leaves live renderer handles
valid. Exact-file native events (including existing symlink targets) explicitly
invalidate matching entries and force re-rasterization; there is no polling.

## Motion

Wallpaper changes use a 300 ms smoothstep crossfade capped at two seconds. The
transition asks for presentation callbacks only while opacity can still change;
the stable state has no timer or frame loop. If a new wallpaper arrives during
a fade, the renderer captures the frozen current composite once and fades from
that capture to the newest target. Stale captures are ignored and multiple
rapid replacements update the pending target without accumulating layers.

Reduced motion makes every replacement immediate. Enabling it during a fade or
while capture is pending settles directly on the newest target.

## Portal ownership

The XDG Wallpaper portal version 1 is a mutation journey for sandboxed apps,
not readable desktop state, and has no output or fit fields. The versioned rmac
session store therefore remains authoritative. rmac Settings writes that store
directly and never calls its own portal.

ADR 0003 specifies a future rmac desktop backend: accept a bounded app identity
and hostless local portal URI, always preview, validate through the same decoder,
then atomically import and apply one whole-desktop Fill choice. Remote fetching
and lock-screen/both requests fail until their real authorities exist. The
backend and installer integration remain pending.

The Wayland background layer surface and executable, portal backend, System
Settings previews, and Linux hotplug/frame-time evidence remain pending. D9 is
therefore not complete.
