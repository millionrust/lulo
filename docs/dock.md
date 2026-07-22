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

`rmac-dock::drag` turns that reducer into direct manipulation without feeding
magnified geometry back into its thresholds. A drag starts only from a visible
pinned application, uses the retained layout plan's resting centers, and does
not activate until movement crosses four logical pixels. Below that threshold
the exact original primary click is returned. During a real drag the preview
order is cached and borrowed on unchanged pointer frames; dropping back on the
source is a no-op, while cancellation creates no action. Special items,
unpinned running apps, hidden entries, malformed layouts, nonfinite input, and
pin sets above the persisted 128-item bound fail explicitly.

`rmac-dock-system` rereads the latest shell settings, applies one pin command,
atomically saves only when the list changed, then rereads before returning a
receipt. Tests prove unrelated Dock and provider settings survive. The visible
Dock still waits for the settings watcher instead of applying that receipt
optimistically. A future single-writer shell-settings service should serialize
simultaneous writes across processes; this slice does not claim that E-phase
authority is complete.

A completed drag carries the exact full pin order the user manipulated, not
only a destination index. Dispatch compares it with the newest coherent model
before ticketing, and the settings worker rereads the store and compares it
again immediately before applying `MoveTo`. A changed order becomes rejected
feedback instead of reinterpreting the drop against different neighbors. The
successful receipt still does not reorder the visible model optimistically.

## Context menus and the More stack

`rmac-dock::menu` projects the accepted application menu and crowded-output
stack into bounded renderer rows without re-resolving actions. Application
menus keep New Window, the ordered real-window list, Keep/Remove from Dock, and
only the reorder directions that can act. A window row carries its exact focus
action plus an exact secondary close action; focused and urgent state are
announced independently. Files, Downloads, and Trash menus expose a typed Open
row without copying a private directory path. Only an available,
authoritatively nonempty Trash adds an **Empty Trash…** row in a distinct
destructive section; its accessible label announces the exact item count and
that confirmation is required. The More stack preserves the hidden application
order, keeps unavailable entries visible but out of keyboard selection, and
activates the exact typed application identity through the current
authoritative model.

Menus select the first enabled row on open. Up/Down wrap across enabled rows,
Home/End move to the first/last enabled row, Return invokes the primary action,
alternate activation invokes a separately exposed secondary action, and Escape
dismisses. Activation closes before asynchronous execution. Both dismissal and
activation return the exact invoking application or More identity so the view
can restore Dock focus. Pointer selection cannot mutate a closed session.

Visible labels are capped at 96 Unicode characters. Window titles remain
visible where the user asked for the menu but labels, accessibility strings,
launch specifications, and window titles are redacted from Debug output.
Malformed overflow, pin, or special-item projections and menus above 512 rows
fail explicitly instead of panicking or silently dropping actions. The
renderer must expose sections, menu roles, checked/urgent/destructive state,
the close accessibility action, and focus restoration exactly as described.

## Busy state and failure feedback

`rmac-dock-system::interaction` gives the future surface one target-scoped
action state for primary activation, window menu actions, pins, places, and
Trash. Starting an action exposes an exact busy target and rejects a duplicate
for that target while allowing unrelated entries to proceed. Completion,
cancellation, retry, and feedback dismissal all bind to a monotonic ticket;
late completion from a cancelled surface cannot clear newer work or create a
stale error. Success removes busy state but never changes the Dock model—the
catalog, niri, settings, and places watchers remain authoritative.

`rmac-dock-system::dispatch` now closes the gap between an accepted menu row
or shelf entry and those asynchronous executors. It revalidates every
application intent against the newest coherent model before issuing a ticket:
launch-new adopts the current catalog launch specification, a focus or close
action requires its exact window ID to still exist, and pin/unpin/reorder
requires the same command to remain valid instead of silently becoming its
opposite. Files and Downloads likewise resolve their newest private directory
authority only during preparation, while Trash resolves its current desktop
URI action; no private path enters the menu intent or Debug output. A focused
single-window activation remains an explicit no-op, malformed entry identities
fail before ticketing, and stale or unavailable actions become private-safe
feedback without touching an inappropriate platform service.

Direct reorder intents enter the same target-scoped action path. Only an intent
whose complete accepted pin order still matches the newest model becomes a
prepared update; it then uses the dedicated checked settings transaction above.
This keeps drag preview responsive while treating persistence as authoritative
asynchronous work.

The prepared action owns its exact target and operation as it enters the busy
state. Its pending value then owns the monotonic ticket throughout asynchronous
execution, and only its completion can finish that ticket. This prevents a
menu that was open during catalog, compositor, or settings changes from
executing stale authority, while preserving the rule that successful receipts
never mutate the visible model directly.

Empty Trash is a two-ticket destructive workflow. Dispatch first requires the
menu's expected count to still match the newest model, then a blocking review
ticket enumerates and binds the exact path-free Trash identities. A successful
review exposes only the item count for the confirmation sheet; declining
consumes it and creates no deletion capability. An affirmative response alone
creates a confirmed value that can start the second Empty Trash ticket.
Deletion still revalidates the bound identities, leaves later additions alone,
fails closed if a reviewed item vanished, and publishes only the authoritative
remaining count. Review and deletion failures use the same bounded semantic
feedback state; no private identity enters presentation or Debug output.

Failures retain only the public target, intended operation, and a semantic
reason such as permission denied, unavailable service, lost connection,
rejected, unsupported, or failed. Raw backend details, commands, paths, and
window titles never enter the presentation snapshot. The fallback accessible
message is therefore safe for a toast/status node, and feedback is explicitly
dismissible. State is bounded to 64 simultaneous targets and the 16 newest
feedback records, published chronologically; retry clears the target's previous
feedback. The real renderer still needs to invoke this dispatch boundary from
its shelf and menu nodes, show busy/error visuals, and restore menu focus.

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
must use `rmac-dock-system::icons` from a worker and remains responsible for
accessible button semantics. The decoder reads at most 4 MiB, detects content
rather than trusting the extension, and accepts PNG or a deliberately bounded
SVG subset. PNG dimensions, pixels, decoder allocation, and final output are
capped. SVG parsing caps depth, elements, attributes, coordinates, and output;
disables DTDs, processing instructions, scripts, event attributes, external or
embedded images, filters, and expansion-heavy `use`; and installs resolvers
that cannot read files or URLs. Both formats become a centered, exact-size,
non-premultiplied RGBA8 square no larger than 512 pixels. Invisible, malformed,
oversized, unsupported (including legacy XPM), and unsafe files return typed,
path-free failures so the renderer can use the embedded application tile.

The shared decoder cache serializes misses, verifies device/inode/mtime/length
before and after decoding, coalesces concurrent requests, and evicts by both a
32 MiB default byte budget and a 256-entry ceiling. Exact-path invalidation and
automatic metadata-key replacement prevent theme changes from retaining old
pixels; cache diagnostics expose only counts and bytes.

`rmac-dock-runtime::icons` owns the off-UI execution contract. One batch may
contain at most 512 distinct application/physical-size keys and at most 32 MiB
of final pixels, while two-slot command and event queues make backpressure
explicit. A dedicated serial worker uses the shared cache and returns a typed
decoded image or fallback reason for every key. Request generations cancel old
work between files, and the renderer-side session accepts completion only when
the generation, cardinality, and complete key order still match. Late,
cancelled, duplicate, oversized, and malformed work therefore cannot replace
newer Dock artwork. Worker commands also provide exact-path and whole-cache
invalidation without placing paths in event or Debug output.

The same runtime can hold one native watcher over only the selected source
files, their current symlink targets, and direct parent directories. Relevant
file changes and conservative backend-overflow notifications enter a one-slot
path-free channel, so a burst requests one complete current-generation reload
instead of one decode per filesystem event. Access and unrelated sibling
events are ignored. Partial setup reports only an unavailable-directory count;
backend failures report only `Failed`. After any change the consumer rebuilds
the watcher to follow atomic replacements or a new symlink target. While that
replacement generation is pending, `IconSession` retains the matching last
accepted results; cancellation restores them and exact completion replaces
them. Theme updates therefore neither flash every app to a generic tile nor
allow old completion to overwrite newer artwork.

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

`rmac-dock-runtime::surfaces::Registry` is the required ownership boundary for
applying those plans. It accepts the complete runtime snapshot, retains at most
32 desired outputs, rejects duplicate or invalid plans without discarding the
last accepted state, and issues one monotonic command at a time. Disconnected
outputs are removed before replacements are created; policy changes
reconfigure the existing logical surface identity. A newer plan may arrive
while an older command is pending—the acknowledged platform result is recorded
first, then the next command converges directly to the newest description.

Applied state changes only after the renderer acknowledges a real create,
reconfigure, or remove. Cancellation is legal only when no platform state was
touched, and a stale completion cannot mutate newer pending work. A failed
command blocks only its output while other outputs continue; explicit retry or
a changed desired description unblocks it, preventing an automatic failure
loop. The renderer's command adapter must report `Failed` only when its applied
state is known unchanged; uncertain partial platform work must be reconciled by
closing that surface before retry. This registry specifies hotplug ownership,
but the upstream-GPUI layer-surface command adapter remains pending A5.

## Outputs

The model creates candidates only for enabled compositor outputs with finite,
positive logical geometry and scale. `all` returns every valid stable output
ID, `named` requires that exact valid output, and `primary` uses the single
persisted `focus-at-startup` owner from the validated rmac display include.
Output/configuration refresh hints resample that authority off-thread. A failed
resample retains the last-known Main ID, reports separate display-source health,
and never guesses from connector order.

The current slice is not D4 completion. The layer-shell view, connection of the
icon worker/watcher to retained renderer nodes, pointer/keyboard semantics,
real surface-command execution,
persistence UI, niri/reference-PC evidence, and performance gates remain
pending.
