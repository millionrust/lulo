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
shared `rmac-app-launch` route, preserving exact program/argument boundaries
without a shell and using niri IPC spawn for XDG activation. Window activation
sends the exact stable window ID and caller-owned activation ID directly through
`rmac-compositor-niri`. A successful result is only a launch/focus receipt;
visible running and active state still waits for authoritative events.

Receipts distinguish compositor activation from direct fallback without
inventing a child process ID for compositor-owned spawn. Launch I/O and
non-fallback niri protocol/rejected errors remain typed with the operation and
application/window identity. Unavailable, transport, and unsupported spawn
errors use the documented direct fallback. Explicit no-op model outcomes never
touch either platform service.

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

## Context menus and the More stack

`rmac-dock::menu` projects the accepted application menu and crowded-output
stack into bounded renderer rows without re-resolving actions. Application
menus keep New Window, the ordered real-window list, Keep/Remove from Dock, and
only the reorder directions that can act. A window row carries its exact focus
action plus an exact secondary close action; focused and urgent state are
announced independently. The More stack preserves the hidden application order,
keeps unavailable entries visible but out of keyboard selection, and activates
the exact typed application identity through the current authoritative model.

Menus select the first enabled row on open. Up/Down wrap across enabled rows,
Home/End move to the first/last enabled row, Return invokes the primary action,
alternate activation invokes a separately exposed secondary action, and Escape
dismisses. Activation closes before asynchronous execution. Both dismissal and
activation return the exact invoking application or More identity so the view
can restore Dock focus. Pointer selection cannot mutate a closed session.

Visible labels are capped at 96 Unicode characters. Window titles remain
visible where the user asked for the menu but labels, accessibility strings,
launch specifications, and window titles are redacted from Debug output.
Malformed overflow/pin projections and menus above 512 rows fail explicitly
instead of panicking or silently dropping actions. The renderer must expose
sections, menu roles, checked/urgent state, the close accessibility action, and
focus restoration exactly as described.

## Busy state and failure feedback

`rmac-dock-system::interaction` gives the future surface one target-scoped
action state for primary activation, window menu actions, pins, places, and
Trash. Starting an action exposes an exact busy target and rejects a duplicate
for that target while allowing unrelated entries to proceed. Completion,
cancellation, retry, and feedback dismissal all bind to a monotonic ticket;
late completion from a cancelled surface cannot clear newer work or create a
stale error. Success removes busy state but never changes the Dock model—the
catalog, niri, settings, and places watchers remain authoritative.

Failures retain only the public target, intended operation, and a semantic
reason such as permission denied, unavailable service, lost connection,
rejected, unsupported, or failed. Raw backend details, commands, paths, and
window titles never enter the presentation snapshot. The fallback accessible
message is therefore safe for a toast/status node, and feedback is explicitly
dismissible. State is bounded to 64 simultaneous targets and the 16 newest
feedback records, published chronologically; retry clears the target's previous
feedback. The real renderer still needs to connect tickets to its async action
tasks, show busy/error visuals, and restore menu focus.

## Live runtime

`rmac-dock-runtime` watches the installed-application catalog, the versioned
shell settings, the direct niri event stream, the persisted Main Display
authority exposed by `rmac-display`, and effective appearance resolved from the
subscribed Settings portal through the watched writable rmac theme store. The
catalog watcher is
established before initial discovery, and its bounded signal channel coalesces
filesystem bursts. Failed setup/discovery and settings watchers retry without
discarding their last-known-good values; niri reconnect remains owned by the
compositor adapter.

The first Dock snapshot is withheld until every always-required source,
including appearance and user places, is either healthy or explicitly
unavailable. Primary-output scope additionally waits until display authority
has resolved. This prevents a flash of default pins, an empty running-app
shelf, animated motion against an already-known reduced-motion preference, or
a surface on a guessed output during ordinary startup. Later health-only changes remain
available to diagnostics but do not request a Dock frame. Catalog, settings,
focus/urgency/window, and output-hotplug changes rebuild the authoritative
model and enabled-output candidates without polling. The exact initial/live
niri overview state crosses the same coherent snapshot and requests a Dock
frame when it changes, ready for each output's D6 visibility machine.
Effective reduced motion crosses that boundary as one boolean. Portal or theme
watch failures preserve its last-known-good value and update only redacted
source health; a real preference change requests one Dock frame.

## Renderer content and original assets

The same snapshot publishes two explicit groups: applications, then places.
The renderer inserts the familiar Dock separator only when both groups are
present. Every entry has a stable typed identity, visible label, path-free
accessible label, enabled state, exact active/running indicator, urgent state,
icon source, and an authoritative badge where one exists. A running app remains
actionable even if its desktop entry disappears; a missing non-running pin
remains visibly unavailable. Accessible labels announce active/running state,
attention, exact window count, unavailable state, and exact Trash count without
disclosing window titles, commands, or local paths.

Desktop-entry icons retain their resolved Linux icon file. Apps without one use
an original embedded rmac application tile. Files, Downloads, empty Trash, full
Trash, and the crowded-Dock More stack also use original self-contained 64-unit
SVG assets embedded in the crate, so session startup cannot race an
installation path. The full Trash icon and badge appear only from an
authoritative positive count; empty and unknown state never invent a badge.
External icon paths have a redacted debug representation. The future renderer
remains responsible for bounded decoding and accessible button semantics.

## Crowded outputs

Fitting is chosen from content and output width before hover. The Dock first
keeps every item and reduces its 48-pixel icon, normal gap, and influence radius
proportionally, never below the 36-pixel primary-shell hit target. A conservative
worst-case magnification envelope is reserved during this decision, so pointer
movement cannot change the selected base size or make the shelf overflow.
The renderer caches that immutable layout plan until content, output geometry,
or policy changes; each pointer frame performs only the stable-center geometry
projection and never repeats fitting or clones the hidden application stack.

If every application still cannot fit, the leading application order and every
place remain fixed while the hidden application tail moves behind one typed
More stack. The stack retains the complete entries in order for a future
keyboard/Orca-accessible popover and aggregates the exact hidden count plus
running, active, and urgent state. It is a real hit-test identity with an
original icon, not an ellipsis painted over unreachable items. Only an output
too narrow for the More stack and mandatory places at the 36-pixel floor fails
with an explicit required/available extent.

Each coherent runtime snapshot also contains a renderer-ready `surface_plan`.
For every selected output it fixes the edge, valid logical axis length, scale,
base and maximum thickness, exclusive-zone reservation, hidden-edge reveal
sensor, overview state, magnification policy, motion policy, and non-keyboard-
interactive layer-shell behavior. The renderer must apply that plan rather than
re-deriving settings or output policy. Changes such as moving the Dock from the
bottom to the left, enabling autohide, changing magnification, or disabling
reservation request a frame even when the application items and output IDs are
unchanged. Invalid magnification policy is an explicit plan error rather than a
clamped or partly rendered Dock.

## Outputs

The model creates candidates only for enabled compositor outputs with finite,
positive logical geometry and scale. `all` returns every valid stable output
ID, `named` requires that exact valid output, and `primary` uses the single
persisted `focus-at-startup` owner from the validated rmac display include.
Output/configuration refresh hints resample that authority off-thread. A failed
resample retains the last-known Main ID, reports separate display-source health,
and never guesses from connector order.

The current slice is not D4 completion. The layer-shell view, bounded external
icon decoding, pointer/keyboard semantics, surface hotplug execution,
persistence UI, niri/reference-PC evidence, and performance gates remain
pending.
