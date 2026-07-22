# ADR 0003: Session-owned wallpaper with an rmac portal backend

- Status: Accepted
- Date: 2026-07-12

## Context

The XDG Wallpaper portal version 1 lets sandboxed applications request a
desktop-background change. Its frontend accepts `SetWallpaperURI` for non-file
URIs and `SetWallpaperFile` for a local file descriptor, with `show-preview`
and `set-on=background|lockscreen|both` options. It does not expose readable
wallpaper state, output identity, fit policy, or per-output choices. The desktop
backend receives one URI plus the requesting app identity and options.

Sources:

- [Wallpaper portal frontend](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Wallpaper.html)
- [Wallpaper portal backend](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.impl.portal.Wallpaper.html)
- [Desktop-specific portal backend selection](https://flatpak.github.io/xdg-desktop-portal/docs/portals.conf.html)

## Decision

`rmac-shell-settings` remains the sole readable wallpaper-choice authority for
the rmac session. System Settings writes it directly through its typed atomic
store. The wallpaper process watches that authority and niri outputs. Neither
component calls `org.freedesktop.portal.Wallpaper`, because consuming its own
mutation portal would lose per-output/fit information and create an authority
loop.

For sandboxed third-party applications, rmac will provide
`org.freedesktop.impl.portal.Wallpaper` as a desktop backend and select it for
that interface in the installed rmac `portals.conf`. The backend request is an
ingress command, never a second state store:

1. Accept only a bounded valid app ID and a hostless local file URI already
   made available by the portal frontend.
2. Reject remote URI fetching in the wallpaper service. A future isolated
   importer may add it with explicit network policy; until then the request
   fails truthfully.
3. Always show an rmac preview/confirmation, even when `show-preview=false`.
4. Validate and decode through the same bounded PNG/JPEG/WebP pipeline before
   persistence.
5. On consent, import an rmac-owned durable copy, set the session default to
   Fill, clear per-output overrides, and atomically commit once. Portal v1 has
   only whole-desktop semantics, so retaining hidden overrides would make the
   accepted request appear ineffective.
6. Reject `lockscreen` and `both` until E5 supplies a real secure lock-screen
   wallpaper authority. Never report success for a cosmetic or nonexistent
   lock surface.
7. Cancellation or failed validation changes no settings and leaves no imported
   orphan.

The `rmac-wallpaper::portal` model is the framework-neutral admission contract.
`rmac-wallpaper-portal` now owns the durable transaction beneath the eventual
D-Bus adapter. It takes an exclusive private-directory lease, stages the
already frontend-authorized local document from one validated open handle,
decodes that frozen copy through the shared bounds, and returns decoded pixels
for the mandatory preview. Acceptance creates or reuses a verified
content-addressed private copy, reloads the latest shell document, changes only
wallpaper policy to one default Fill choice with no output overrides, saves and
rereads the authority, and preserves unrelated shell settings. Decline and
cancel never read settings. Failed validation, failed persistence, dropped
requests, and startup recovery remove only recognized unreferenced transaction
files; a file that current settings may reference is retained. Portal response
codes map success, cancellation, and other failure explicitly.

`rmac-wallpaper-portal` also owns the authenticated asynchronous backend
boundary. Its dedicated service builder exports the exact
`org.freedesktop.impl.portal.Wallpaper.SetWallpaperURI` method and accepts calls
only from the current unique owner of `org.freedesktop.portal.Desktop`. It
exports `org.freedesktop.impl.portal.Request` at the frontend-provided handle
for the interaction lifetime, and `Close()` races preview completion through
one idempotent cancellation state. A preview decision wins exactly once before
the durable commit begins; duplicate, stale, and replayed decisions are inert.

Admission is capped at eight live interactions, source URIs and parent handles
are bounded, preparation is serialized, and retained decoded previews share a
256 MiB budget. The renderer-facing event contains app identity, parent handle,
format, byte count, and decoded pixels but no source URI or staging path. It is
published even when `show-preview=false`; a missing preview consumer fails with
response 2 and drops the transaction. Unknown options are ignored for forward
compatibility, missing `set-on` means background in rmac, and wrong types or
unknown `set-on` values are rejected. The service uses a separate
`org.freedesktop.impl.portal.desktop.rmac.wallpaper` bus name so future image/UI
failures cannot take down the notification backend.

The preview stream is an ordered Open/Close lifecycle rather than a fire-and-
forget image queue. Each terminal Close retains its admission lease until the
UI consumes or drops it, so a stalled preview process applies backpressure and
cannot lose the event that dismisses a visible dialog. Frontend `Close()`, a
user decision, a dropped request future, and an internal failure all converge
on that same terminal event.

The framework-neutral presenter serializes the eight admitted interactions
into one focused modal and a FIFO queue. It renders the proposed image with the
same exact Fill geometry, discloses that the change affects every display and
replaces per-display choices, and exposes stable dialog/preview/button labels.
Set Wallpaper is the default focus; Tab, reverse Tab, left/right, Enter, Space,
Escape, explicit Cancel, and window close have deterministic behavior. A
decision enters a disabled resolving phase until terminal Close, preventing a
second activation or exposing the next request during a durable commit.

The GPUI preview/confirmation window, supervised executable, installed backend
descriptor, and `rmac-portals.conf` selection entry remain future
implementation work. Installation must not advertise this backend before the
real consent UI drains the mandatory preview stream.

## Consequences

- The rendered desktop and Settings pane always agree because they read one
  versioned authority.
- Sandboxed apps retain the standard portal journey once the backend ships.
- rmac-specific per-output and fit features stay available without inventing
  nonstandard portal keys.
- Remote wallpaper requests and lock-screen targets fail explicitly for now.
- Portal interoperability is not complete until the backend is installed and
  exercised against `xdg-desktop-portal` on the Linux reference machine.

## Rejected alternatives

- Calling the portal from rmac System Settings: authority loop and information
  loss.
- Treating the portal as readable state: version 1 provides no read method.
- Silently applying only the default while retaining output overrides: accepted
  request may produce no visible change.
- Claiming `both` succeeded before secure lock integration: false security UI.
