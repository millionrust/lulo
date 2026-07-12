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

The existing `rmac-wallpaper::portal` model is the framework-neutral admission
contract. The D-Bus backend, preview UI, durable importer, response mapping,
installed backend descriptor, and `rmac-portals.conf` entry remain future
implementation work.

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
