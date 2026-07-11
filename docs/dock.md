# Dock application model

`rmac-dock` is the framework-neutral foundation for the D4 Dock. It combines
the versioned shell settings, installed application catalog, and complete niri
snapshot into a deterministic item list and typed activation result.

The interaction target is familiar to a MacBook user without copying Apple
branding: pinned applications stay spatially stable, running applications are
obvious, the active app has a reliable indicator, urgent state is visible, and
clicking never creates a fake local running state while Linux is still
launching or focusing the real window.

## Ordering and identity

- Pinned applications appear first in their persisted order; duplicate IDs are
  removed after case-insensitive `.desktop` normalization.
- All niri windows with the same normalized app ID form one Dock item.
- Unpinned running applications follow pinned items, most recently focused
  first with stable name/ID tie-breakers.
- Focused windows lead each app's window list, followed by focus timestamp.
- Windows without a reliable compositor app ID are omitted rather than assigned
  an invented application identity.
- An uninstalled pinned application remains a truthful unavailable item so a
  later context menu can offer removal; it cannot be launched.

Desktop-entry ID and compositor app-ID matching is intentionally conservative.
`StartupWMClass`/alias metadata still needs to be added to `rmac-apps` for
applications whose Wayland identity differs from their desktop-file ID.

## Primary click

- An installed app with no windows returns its exact shell-free `LaunchSpec`.
- A running background app focuses its most recently focused window.
- Repeated click with `cycle-windows` focuses the next recent window and is a
  no-op when only one window exists.
- `do-nothing` is an explicit no-op.
- `hide-application` reports unavailable because niri exposes no truthful
  application-hide operation; the Dock does not simulate hiding.

The launcher/compositor adapter must execute the result and wait for catalog or
niri events to change the item state. It must never optimistically mark an app
running or focused.

`rmac-dock-system` now provides that adapter. Application launches use the
exact parsed `LaunchSpec` on a blocking executor, preserving program and
argument boundaries without a shell. Window activation sends the exact stable
window ID and caller-owned activation ID directly through
`rmac-compositor-niri`. A successful result is only a launch/focus receipt;
visible running and active state still waits for authoritative events.

Launch I/O and niri unavailable/transport/protocol/rejected/unsupported errors
remain typed with the operation and application/window identity. Unavailable
and explicit no-op model outcomes never touch either platform service.

## Outputs

The model creates candidates only for enabled compositor outputs. `all` returns
every enabled stable output ID, `named` requires that exact enabled output, and
`primary` requires a separate authoritative primary ID. Missing authority
produces no primary surface instead of guessing from connector order.

The current slice is not D4 completion. The application/compositor runtime,
layer-shell view, icons/assets, pointer/keyboard semantics, hotplug execution,
persistence UI, niri/reference-PC evidence, and performance gates remain
pending.
