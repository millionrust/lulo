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

## Physical surface lifecycle

`rmac-wallpaper-runtime::surfaces` owns the framework-neutral boundary between
render updates and real background windows. It accepts at most 32 requested
outputs and validates each complete plan/raster transaction before changing
desired state: output identities must be unique and matched, geometry and scale
must agree with the plan, layout must be exactly reproducible, and decoded RGBA
dimensions, pixels, and byte length must remain within the decoder bounds. A
malformed, duplicate, unexpected, or oversized update leaves the previous
accepted state untouched.

Rasterization remains independently fallible per output. A requested output
missing from a partial raster update retains its previous accepted frame and is
reported as stale; a new output without any accepted frame is reported as
unavailable. Other outputs can still update. Disconnecting an output removes
its old background before a replacement is created, while presenting a new
frame on a connected output preserves that surface's stable physical identity.

The registry emits only one acknowledged Create, Present, or Remove command at
a time. Applied state changes only after the platform adapter reports success.
A newer desired revision supersedes pending work by reconciling immediately
after its acknowledgement; stale command completions and safe cancellations
cannot mutate newer state. Failure blocks only the still-current operation for
that output, leaves its applied surface unchanged, and does not prevent peers
from converging. Explicit retry or a changed desired frame clears that failure.
Health-only runtime updates never create surface work.

## Desktop accessibility contract

Every accepted raster now retains one path-free actual source—its built-in ID
or `UserFile`—plus whether a plan, resolve, or decode failure substituted the
Aurora fallback. The surface registry verifies that metadata against the exact
plan and raster issue set before admission, and source/fallback changes count
as real presentation changes even when an image cache handle is reused.

`rmac-wallpaper-runtime::accessibility` consumes the applied session snapshot,
not desired wallpaper settings. It produces one named “Desktop” root and one
named image node for each real applied background surface in stable order.
Aurora and user images remain distinguishable without exposing a filename;
Fill/Fit/Stretch/Center/Tile, fallback, retained-stale-frame, and failed-update
state remain explicit. Every desktop surface is passive, has one-item reading
order, and has no keyboard focus target.

The projection accepts at most 32 surfaces and 512-byte private output
identities. It verifies sorted unique requested/desired/unavailable/stale/
applied/failure sets, exact unavailable derivation, host ownership, pending and
applied raster validity, and output/raster identity agreement. Output IDs,
paths, pixels, and source-health details never enter the semantic snapshot or
its diagnostics. Defining this tree does not claim that the current framework
can export it to AT-SPI.

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
portable `rmac-wallpaper-portal` transaction now stages the frontend-provided
document from one validated handle under an exclusive private-directory lease,
decodes the frozen bytes before confirmation, and exposes that decoded image to
the future preview window. Consent creates or reuses an exact content-addressed
private import, reloads the latest shell document, preserves unrelated policy,
sets one default Fill source, clears output overrides, saves, and verifies
readback. Decline and cancellation never touch settings. Failure cleanup and
startup recovery remove only recognized unreferenced imports, while uncertain
or currently referenced content is retained. Response mapping distinguishes
success, cancellation, and other failure.

The same crate now exposes the exact authenticated backend method on a
dedicated service connection. Only the current unique owner of
`org.freedesktop.portal.Desktop` may call it. Each call installs the standard
backend Request object at the supplied handle, and `Close()` cancels the live
interaction before a durable commit can begin. At most eight requests are live;
decode preparation is serialized; retained preview pixels share a 256 MiB
budget; and source URI plus parent-window inputs are bounded. Every admitted
image is sent through a private, path-free preview event even when the caller
sets `show-preview=false`. The decision capability is one-shot, stale replies
are inert, a dropped preview consumer fails closed, and the Request object is
removed when the method returns. Missing `set-on` selects background, while
lock-screen/both remain explicit non-success until the secure lock authority
can apply them truthfully.

Presentation consumes ordered Open and terminal Close events. The terminal
event retains the request's admission lease until consumed, so a stalled UI
stops new requests instead of losing a dismissal. The portable presenter shows
one modal at a time and queues the rest FIFO. It validates exact RGBA bounds,
uses the shared Fill geometry for a 16:9 desktop preview, discloses the whole-
desktop/per-output consequence, and keeps source paths out of semantics and
diagnostics. Set Wallpaper receives default focus; Tab/reverse Tab, arrow keys,
Enter, Space, Escape, pointer activation, and window close are explicit. Once a
decision is delivered the dialog becomes resolving and stays noninteractive
until terminal Close; frontend cancellation removes a visible or queued dialog
without inventing consent.

System Settings now reads this authority, offers default and stable-output
overrides, and follows the direct niri output stream so unplugged choices remain
explicit and reappear by ID. Its local-file chooser is portal-mediated, and a
choice must pass the same bounded magic-byte, size, dimension, and decode gates
before persistence. The preview uses the runtime layout contract for Fill, Fit,
Stretch, Center, and Tile, and mutations reread authority plus retain one-step
wallpaper rollback. It keeps one exact selected-file watcher alive, invalidates
stale generations on target/settings changes, and regenerates through the same
bounded decoder when the file is replaced or edited.

The Wayland adapter that executes this lifecycle as background layer surfaces,
the supervised wallpaper executable and GPUI mandatory-preview window, export
of the now-defined passive Desktop/image semantics, backend installation
assets/selection, and Linux portal/hotplug/Orca/frame-time evidence remain
pending. D9 is therefore not complete.
