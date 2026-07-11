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

## Context actions and pins

The context model exposes a launch-new action when the catalog provides an
exact launch specification, plus show and close actions for every real niri
window. Window title, focused state, and urgency remain attached to their
stable IDs. It intentionally exposes no process-wide **Quit**: closing known
windows is truthful, while sending signals or guessing process ownership is
not equivalent to an application quit contract.

Pinned items expose unpin and only the reorder directions that can change their
position. Unpinned running apps expose pin. The pure pin reducer matches IDs
using the same desktop normalization, preserves exact stored IDs, treats a
duplicate pin or edge move as a no-op, and rejects moving or unpinning an app
that is not currently pinned. Drag reorder uses an explicit destination index,
bounded to the current pin list, and moves one exact stored ID without
reconstructing the rest of the order.

`rmac-dock-system` rereads the latest shell settings, applies one pin command,
atomically saves only when the list changed, then rereads before returning a
receipt. Tests prove unrelated Dock and provider settings survive. The visible
Dock still waits for the settings watcher instead of applying that receipt
optimistically. A future single-writer shell-settings service should serialize
simultaneous writes across processes; this slice does not claim that E-phase
authority is complete.

## Live runtime

`rmac-dock-runtime` watches the installed-application catalog, the versioned
shell settings, and the direct niri event stream. The catalog watcher is
established before initial discovery, and its bounded signal channel coalesces
filesystem bursts. Failed setup/discovery and settings watchers retry without
discarding their last-known-good values; niri reconnect remains owned by the
compositor adapter.

The first Dock snapshot is withheld until all three sources are either healthy
or explicitly unavailable. This prevents a flash of default pins or an empty
running-app shelf during ordinary startup. Later health-only changes remain
available to diagnostics but do not request a Dock frame. Catalog, settings,
focus/urgency/window, and output-hotplug changes rebuild the authoritative
model and enabled-output candidates without polling.

## Outputs

The model creates candidates only for enabled compositor outputs. `all` returns
every enabled stable output ID, `named` requires that exact enabled output, and
`primary` requires a separate authoritative primary ID. Missing authority
produces no primary surface instead of guessing from connector order.

The current slice is not D4 completion. The primary-output authority,
layer-shell view, icons/assets, pointer/keyboard semantics, surface hotplug
execution, persistence UI, niri/reference-PC evidence, and performance gates
remain pending.
