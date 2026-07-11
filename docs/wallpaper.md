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

The Wayland background layer surface, decoder/cache/invalidation runtime,
portal interoperability decision, transitions/reduced-motion behavior,
System Settings previews, and Linux hotplug/frame-time evidence remain pending.
D9 is therefore not complete.
