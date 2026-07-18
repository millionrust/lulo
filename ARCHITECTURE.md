# Architecture

## Current system

rmac is a Cargo workspace with seven application binaries, a platform lab, and
shared domain, service, storage, portal, and UI crates. Several application
roots still combine domain state, platform access, persistence, and rendering
in their `main.rs`; completed vertical slices are moving platform behavior
behind typed crates without a workspace-wide rewrite.

```text
application binary
  -> rmac-ui
  -> rmac-editor (Notes and Text Editor)
  -> GPUI / gpui-component
  -> direct filesystem, subprocess, sysinfo, PTY, or macOS FFI access
```

This shape was effective for proving the applications, but it is not the target
Linux architecture. Large application files and direct macOS command usage make
platform work, fault testing, and accessibility review unnecessarily coupled.

## Target dependency direction

```text
apps and shell surfaces
        |
        v
domain models and service interfaces <-> rmac-ui
        |
        v
storage, portals, Linux/macOS adapters, compositor adapter
        |
        v
filesystem, D-Bus, Wayland, OS APIs
```

Dependencies point downward. Domain crates do not import GPUI, D-Bus, Wayland,
or platform FFI. UI code renders domain snapshots and sends typed commands.
Adapters translate external events into domain events.

`rmac-notes-store` is the first G2 extraction from the Notes application root.
It contains only path-independent stable identities, bounded authoritative
folder/note/attachment records, cross-record invariants, and a canonical
versioned binary codec. It imports no GPUI, filesystem, portal, or async
runtime. The forthcoming journal/storage adapter must validate and encode a
complete candidate through this crate before changing durable state; the Notes
view will eventually consume accepted snapshots rather than scan paths itself.
`rmac-notes-storage` is the lower durable adapter. It serializes mutations per
store instance, exact-preflights the loaded primary bytes, verifies a private
write-ahead journal, atomically replaces and rereads the primary, refreshes the
last-known-good copy, and removes the journal only after both durable records
match. Startup uses the journal's previous SHA-256 identity and complete
candidate to roll back a merely prepared save or finish backup/cleanup after an
already committed primary. Ambiguous or malformed journals are preserved and
block further writes rather than being guessed away. The application runtime
still needs to enforce the single-writer process boundary before integration.

`rmac-compositor` owns compositor-independent outputs, workspaces, windows,
layer surfaces, focus, activation, urgency, snapshots, and incremental events.
It depends only on serialization crates. The direct niri adapter is a lower
layer and must tolerate the source event stream's non-atomic cross-collection
ordering and future JSON additions.

`rmac-compositor-niri` is that lower layer. It connects directly to
`$NIRI_SOCKET`, opens the event stream before querying outputs and layer
surfaces on separate sockets, and publishes one coherent domain snapshot only
after niri's complete initial workspace and window events arrive. It then
forwards incremental domain events. Socket loss produces explicit connection
states and bounded reconnect backoff; a replacement stream always rebuilds
state from scratch. The adapter inspects each JSON envelope before typed
deserialization so a future event variant remains an `Event::Unknown` instead
of taking down the shell.
The same adapter exposes neutral typed actions and capabilities. Every action
uses an explicit stable target and an independent socket; `Handled` is command
acceptance, while the event stream remains the authority for visible state.

`rmac-session` defines typed health snapshots and the persistent safe-mode
marker for systemd user-supervised shell processes. Top bar, Dock, launcher,
the notification authority, Notification Center panel, and wallpaper are
independent service units with bounded restart rates. A small supervisor
publishes runtime health, records restart budget exhaustion, and moves the
session to a supervisor-only diagnostic target
until the user explicitly clears safe mode. Session startup imports only a
fixed allowlist of Wayland/D-Bus routing variables before starting the target;
the complete login environment is never copied into the user manager.

`rmac-shortcuts` owns stable shell shortcut IDs and keeps backend syntax out of
consumers. On Linux its broker checks the GlobalShortcuts portal version,
creates one consent-bound session, publishes the bound trigger descriptions,
and reconnects if the portal disappears. Activations pass through an
allowlisted Unix-datagram dispatcher. When the portal is unavailable, the
installer generates an explicit niri 26.04 include using direct `spawn`
arguments—never a shell—and the user opts into that fallback instead of running
both backends simultaneously.

The intended crate map and migration phases are specified in `PLAN_V2.md`.

## UI and update model

`rmac-component-gallery` is the executable design-system contract. Its typed
inventory lives in `rmac-ui`, renders every planned shared control state at
logical 100%, 150%, and 200% previews, and records the keyboard journey B7 must
implement. Native Linux scaling and accessibility evidence remains gated by the
reference-PC framework reports; deterministic preview scale is never presented
as compositor evidence.

The first B7 control boundary owns semantic Button roles, keyboard-focusable
binary/mixed Toggle behavior, and Slider orientation/state exports. Product
crates do not import upstream Switch or Slider types; dialog actions and the
Activity Monitor action buttons also use the shared Button implementation.
TextField and SearchField preserve the entity-backed editor model behind the
same boundary; SearchField enables a clear action by default.
Tabs owns single-selection routing and bounded arrow/Home/End keyboard
navigation; Activity Monitor is its first product consumer.
List and Tree share explicit ready/empty/loading/stale/unavailable/error
surfaces. Their rows own focus and selection; TreeRow adds indentation and
Left/Right expansion. System Settings categories use the shared ListRow.
Table preserves the pinned virtualized delegate/state implementation behind an
rmac-owned API, including sorting, selection, keyboard movement, scrolling,
loading, empty content, and row events. Activity Monitor is the active consumer.
Tooltip, Progress, EmptyState, and Toast own shared feedback styling and state
roles. System Settings uses the latter three for loading, empty Wi-Fi results,
and dismissible service/persistence failures.
The shared Button supports text, icons, compact sizes, selected state, and
dropdown triggers. Terminal uses it for the profile control; Terminal, Finder,
and App Drawer use shared text/search fields exclusively.
All seven product apps are prohibited from importing upstream Button, Input,
Switch, Slider, or Table modules by `scripts/check-shared-controls.sh`, which is
run in CI and the Linux reference gate. Alerts form a tab group, and context
menus use focusable rows plus a menu-scoped Escape binding.

- GPUI entities own view state and subscriptions.
- Render methods must not perform filesystem access, subprocess work, or D-Bus
  calls.
- Long-running work is cancellable and publishes progress.
- External services push events when possible.
- A redraw is requested only after visible state changes or while an animation
  is active.
- Shared controls live in `rmac-ui` and own their keyboard, focus, and accessible
  semantics.

## Platform boundaries

Linux product code will use typed services for application discovery, portals,
file operations, monitoring, network, Bluetooth, power, audio, and compositor
state. The first Linux implementations use freedesktop specifications,
NetworkManager, BlueZ, UPower, PipeWire/WirePlumber, and niri IPC.

macOS implementations remain optional development adapters. Platform commands
and FFI cannot leak into application state or render modules.

`rmac-system-info` owns the privacy-safe About snapshot and hostname service
boundary. Linux reads systemd-hostnamed over the system D-Bus, validates static
hostnames before mutation, requests interactive polkit authorization, and
returns a refreshed authoritative snapshot. Its diagnostic projection excludes
host/user identity, serials, machine IDs, network addresses, and paths. The
System Settings renderer only holds this typed snapshot and dispatches work to
the background executor; a deterministic `Service` fake covers consumer tests.

Software updates are split between platform-neutral `rmac-updates` and the
PackageKit adapter in `rmac-updates-linux`. The domain crate owns bounded update
records, classification, failure kinds, transaction collection, and an
asynchronous fakeable source. The Linux adapter opens one system-D-Bus
transaction, subscribes before requesting results, tolerates unknown signals,
and has a fixed timeout. System Settings retains the last successful snapshot
when a refresh fails. Installation remains outside this read-only boundary
until progress, cancellation, polkit, restart, and recovery states are complete.

`rmac-mounts` also owns storage-capacity discovery. It combines the system
volume with user-visible removable/network mounts and calls `statvfs` directly
for each one. Capacity failures remain attached to their volume rather than
failing the entire snapshot. System Settings performs discovery off the UI
thread, preserves the last good list on refresh failure, and does not infer
categories or safe-to-delete files from raw filesystem totals.

Date and time follow the same split boundary: `rmac-time` owns snapshots,
timezone validation, bounded inventory handling, typed failures, and a fake
service; `rmac-time-linux` owns systemd-timedated D-Bus and interactive polkit
calls. Mutations return a newly sampled snapshot rather than optimistic local
state. System Settings dispatches blocking D-Bus work to its background
executor and retains the previous snapshot when authorization or mutation
fails. A bounded event adapter coalesces property changes, refreshes after
timedated reappearance, reconnects after bus loss, and ignores the daemon's
expected idle shutdown.

Appearance is split deliberately: `rmac-appearance` owns the platform-neutral
snapshot, capabilities, event reducer, source trait, and deterministic fake;
`rmac-appearance-portal` reads standardized host preferences and follows the
XDG Settings portal signal stream. The portal is read-only. `rmac-theme` is the
separate writable authority: it resolves explicit or automatic preferences
against the host snapshot, persists a versioned primary and last-known-good
copy atomically, and emits bounded filesystem change events for other
processes. System Settings must use this authority instead of local view state.

`rmac-ui::theme` derives semantic colors, typography, spacing, radii, focus,
elevation, and motion from the resolved appearance. It guarantees accent-safe
foreground selection and supplies light, dark, increased-contrast, and
reduced-motion variants. The legacy `rmac_ui::mac` accessors resolve through
the default light token set only as a source-compatibility bridge while apps
move to live tokens.

The shared boot path now starts a non-blocking appearance runtime. It resolves
portal state plus `rmac-theme` preferences after the first frame, consumes
reconnecting portal events and bounded preference-file events, changes the
gpui-component mode, and refreshes windows only when resolved tokens change.
Terminal is the first representative consumer: its application chrome follows
live tokens while terminal color profiles remain explicit user content.
Finder and App Drawer also consume semantic backgrounds, controls, selection,
drag targets, and notice colors; file tags and application icons remain
content-owned colors rather than being recolored as chrome.
Notes and Text Editor consume the same live document surfaces and notice
tokens; note-yellow semantics and colors embedded in RTF content remain
content-owned.
Activity Monitor and System Settings complete the current application rollout:
their structural surfaces and interaction feedback follow live tokens, while
charts, pressure/status signals, category tiles, and device-state colors remain
semantic data. All seven apps now enter through the shared event-driven theme
runtime.

Shell preferences have their own GPUI- and compositor-free authority in
`rmac-shell-settings`. Its versioned snapshot covers pinned applications, Dock
placement/output/autohide/magnification behavior, clock and meaningful
indicators, default and per-output wallpaper selection, current Focus choice,
per-provider privacy/network policy, and Spotlight exclusions/removable-media
opt-in. Exclusions are bounded, unique, normalized absolute paths; shell text
and URI forms never enter the search boundary. Separate shell processes watch
one XDG configuration file and refresh from authority after coalesced change
events. Writes atomically replace both the primary and last-known-good
documents; schema migration and corrupt-primary recovery happen below every UI
surface.

Live shell chrome has a separate read boundary. `rmac-shell-status` reduces
focused compositor identity and complete network, VPN, Bluetooth, audio, power,
Focus, and notification inputs into one visibility-aware projection; duplicate
or hidden changes do not request a frame. `rmac-shell-status-linux` sits below
it and subscribes to NetworkManager, BlueZ, UPower/power-profile D-Bus paths and
the PipeWire object monitor. It publishes coalesced refresh hints rather than
claiming signal payloads are authoritative state. Service snapshot reads remain
off the render path, and transport loss is explicit instead of silently leaving
menu-bar indicators stale. `rmac-shell-runtime` combines those hints with niri
and shell-settings streams, keeps the last known good status through transient
source failures, and publishes source health separately. A health-only update
is available to diagnostics but is explicitly marked as not requiring a shell
frame. The same publication carries full Quick Settings snapshots and a
separate redraw flag. A dead source disables its mutation surface while the
compact top bar retains its last-known-good indicator.

Notification admission is a composed authority, not a view decision.
`rmac-notifications-linux` loads the private `rmac-notifications-store` Center,
resolves per-app enabled/banner/sound/history/urgent policy, and asks the live
`org.rmac.Focus1` authority to apply the active mode and exact allow list before
posting to the shared protocol server. It reuses one session-bus connection;
Focus loss fails normal banners closed while retaining policy-allowed history.
The bounded runtime event stream carries the exact posted snapshot and persists
validated Center changes off D-Bus dispatch, retaining expired history and
removing dismissed, withdrawn, or action-closed records without a lookup race.
Its private `org.rmac.NotificationCenter1` interface publishes only unread and
urgent indicator state. The shell subscribes before reading, retains
last-known-good state through reconnects, and never receives notification
content through this status boundary. A separate bounded, authenticated history
projection feeds only the on-demand Notification Center panel; its client
revalidates wire content and keeps payloads out of diagnostics. The panel groups
records, acknowledges read state, and routes clear and per-app policy changes
back to the same single-writer interface rather than touching storage. That
interface also owns app-policy projection, and callers receive refreshed state
plus explicit persistence errors. Action disclosure is joined against the exact
live reducer record by ID, source, update time, and complete action set; the wire
carries only a validated visible label and original button position, never the
private action ID or target. Persisted-only records expose no actions, their IDs
are reserved across service restart, and invocation returns through the
existing `ServiceHandle` transport transaction.
The independently supervised panel process owns the action-scoped
`notification-center` shortcut endpoint, reports readiness only after its Unix
socket is bound, and creates at most one GPUI window. Dismissal drops the window
and its bounded content watchers without terminating the endpoint; repeated
activation toggles that same surface. The panel unit is deliberately separate
from the D-Bus notification authority so presentation failure cannot interrupt
admission, actions, or retained history.

Quick Settings consumes the same typed service snapshots through
`rmac-quick-settings`. Its framework-neutral transaction model validates
capabilities, permits only one in-flight mutation per control, retains the
authoritative value while work is pending, and accepts success only alongside
a refreshed authority snapshot. Stale task completions cannot overwrite newer
state, and failures preserve last-known-good values with user-visible detail.
The same domain owns the one-popover/multi-output lifecycle, available-control
focus order, dismissal reasons, and commands derived from authoritative values;
it does not import GPUI or Wayland input types.
`rmac-quick-settings-system` is the blocking adapter above that boundary. It
maps validated commands to NetworkManager, BlueZ, PipeWire/WirePlumber, and
power-profiles, then rereads only the affected authority. Focus explicitly
uses the session-owned `org.rmac.Focus1` command authority and never writes
legacy shell preferences. The supervised `rmac-quick-settings-app` owns the
action-scoped shortcut endpoint and creates at most one GPUI popover. Each open
surface starts a bounded `rmac-shell-runtime` subscription, runs the system
adapter off its render executor, and drops all hardware watchers when dismissed.
The stable GPUI line cannot map the popup to a proven niri output/seat, so final
layer-shell placement and invoker focus restoration remain adapter gates.

`org.rmac.Focus1` is also the single writer for the full Focus configuration.
Its bounded whole-config wire contract preserves modes, app allow lists,
urgent behavior, and schedules; replacement validates the complete graph,
persists atomically, reevaluates current policy, and emits a separate
configuration-change signal. Settings clients subscribe before reading and
never compose partial file writes.

The Dock begins with a compositor- and catalog-backed domain in `rmac-dock`.
It preserves configured pinned order, groups niri windows by normalized desktop
identity, orders unpinned running apps by recent focus, and derives launch,
focus, cycle, no-op, or unavailable outcomes without mutating view-local state.
Output-scope resolution requires an explicit primary-output authority and never
invents a surface on a disabled or missing display. Layer-shell rendering,
pointer dynamics, and process execution remain adapters above this model.
`rmac-dock-system` executes those typed outcomes: desktop-entry `LaunchSpec`
values go through shared `rmac-app-launch` off the render thread. App Drawer,
Spotlight, and Dock therefore use the same direct niri `Spawn` action and XDG
activation-token path, with redacted bounded argv and a typed direct fallback.
Window focus goes directly to the niri socket with a request ID.
Receipts never update the Dock model; only later catalog/compositor events do.
Its context boundary exposes per-window focus/close and current-settings-based
pin mutations. Pin writes preserve unrelated shell settings and reread the
atomic authority before returning; there is no fabricated process-wide Quit.
App Drawer's explicit supervised mode owns the action-scoped `app-drawer`
shortcut endpoint and reports readiness only after binding. It constructs its
catalog watcher and GPUI entity only while its single window exists; repeated
activation dismisses that window without stopping the endpoint. Its standalone
mode remains a normal measurable product app and never competes for the socket.
`rmac-dock-runtime` establishes the app-directory watcher before discovery and
combines its coalesced changes with reconnecting niri events and versioned
shell-settings events. It waits until every source is healthy or explicitly
unavailable before the first publication, retains last-known-good catalog and
settings state, and separates diagnostic-only changes from Dock redraws.
Dock direct manipulation stays in `rmac-dock::motion`: stable-slot
magnification produces non-overlapping one-dimensional geometry, while a
deadline/pressure state machine owns autohide, overview, fullscreen, and
reduced-motion policy without a timer or frame loop of its own.

Dock places begin in the platform-neutral `rmac-places` boundary. It resolves
the XDG Downloads value with a deliberately restricted grammar—HOME or an
absolute literal only—and models filesystem presence plus Trash availability,
emptiness, and item count. It never evaluates shell text, enumerates mounts, or
permanently deletes content; those responsibilities belong to its system
adapter and an explicit destructive confirmation path.

`rmac-places-system` supplies that adapter with partial-failure reports. It
reads XDG configuration, checks place availability, opens Downloads through
the desktop portal boundary, and delegates home/mounted-volume Trash behavior
to the freedesktop implementation. Empty Trash requires a module-private
confirmation value and rereads the item count after the purge.

Launcher and Spotlight policy begins in `rmac-launcher`. Provider descriptors
declare private-content and network requirements before work is scheduled.
Each query cancels the previous generation; stale or identity/category-spoofed
batches are rejected. Action type must also match the declared category, and
any file-path action requires a private-content descriptor. The domain owns
deterministic scoring, category caps,
stable keyboard selection, and exact primary/alternate actions without
importing GPUI, filesystem search, portals, or process execution.
`rmac-launcher-providers` supplies the first local adapters. It preserves exact
desktop-entry launch specifications, maps Settings keywords to stable pane IDs,
passes cancellation into bounded filename/recent-document searches, and
evaluates a deliberately small arithmetic grammar. Its application catalog is
an atomically shared revision: installed-app rescans replace every provider
clone together, while identical discoveries cause no query churn. File results
are declared
private before admission and retain an explicit Reveal alternate; providers
run behind an exact-descriptor check so an unadmitted adapter receives no
query. Filename traversal prunes excluded roots before descent and stays on the
root filesystem unless removable media was explicitly enabled. Recent records
must still exist, remain in scope, and cannot bypass exclusions through a
symlink alias. `rmac-launcher-system` is the separate execution boundary. It
validates the typed action again, launches the preserved application
specification off the UI executor, and opens/reveals files through
`rmac-portal`. Settings
navigation and clipboard writes are delegated to the live overlay surface,
where the GPUI context exists. Receipts contain only an activation ID and
outcome kind; default errors redact paths, copied text, and backend detail.
Application results likewise preserve launch as primary and declare Show
Application as a distinct alternate. That action reveals the desktop-entry or
bundle source through the portal and returns only an `ApplicationRevealed`
outcome, never repurposing a private document action or exposing its path in a
receipt.
`rmac-launcher-runtime` captures provider descriptors once, dispatches admitted
providers concurrently on the blocking pool, and cancels the shared request if
the overlay receiver closes. Its coordinator exposes immediate query-focus
intent before scheduling search, progressive loading/results/degraded/empty
states, private-safe live announcements, single-flight activation, and stale
completion rejection. Policy changes restart the current generation so a
newly denied private provider cannot keep publishing into an open overlay. It
consumes only the stable `launcher` activation from `rmac-shortcuts`; fresh
timestamps toggle the overlay, while repeats, replays, deactivation, and other
shortcut IDs are ignored.

The runtime establishes the installed-app watcher before its first discovery,
so an install during startup is observed by the bounded change channel. A real
new revision cancels and reissues only the current open query. Discovery or
watch failure retains last-known-good app results, exposes degraded catalog
health without diagnostic paths in the overlay snapshot, and retries with a
bounded delay; stale/replayed revisions do nothing.

Wallpaper begins in the GPUI- and Wayland-free `rmac-wallpaper` domain. It
parses only the original `builtin:rmac-aurora` source, hostless local file URIs,
or normalized absolute paths; invalid output-specific sources fall back on that
output without blanking peers. Enabled compositor outputs deterministically
produce one background-surface plan each, so unplug removes only that surface
and replug restores the persisted output choice. Fill, Fit, Stretch, Center,
and Tile geometry is explicit in logical coordinates at the output scale.
`rmac-wallpaper-system` opens local files once, requires a bounded nonempty
regular file, recognizes PNG/JPEG/WebP magic, rewinds and retains the validated
handle for decoding, and redacts paths from default errors and `Debug`. The
built-in default is renderer-owned procedural metadata and an original rmac
palette; no Apple or third-party wallpaper bitmap is bundled.

`rmac-wallpaper-image` applies strict 16,384-pixel axis, 40-megapixel, and codec
allocation limits before RGBA expansion. It rasterizes Aurora deterministically
at output physical size, decodes each custom file once across output scales,
and retains decoded images in a 256 MiB LRU whose eviction never invalidates a
live renderer `Arc`. Its native exact-file watcher invalidates metadata keys and
forces re-rasterization on change or replacement without polling. Wallpaper
builds enable only PNG, JPEG, and WebP codecs; thumbnail-specific formats stay
isolated in `rmac-thumbnails`.

`rmac-wallpaper-runtime` combines the reconnecting niri output stream and
versioned shell-settings watcher. It waits for both sources to resolve before
the first publication, retains last-known-good outputs and choices through
source failure, and builds a replacement plan only when visible state changes.
Changed plans and selected-file events are resolved, decoded, and laid out on
the blocking pool, then published as ready RGBA surfaces. Health-only changes
publish diagnostics without reopening files, decoding images, or requesting a
wallpaper frame. Default runtime `Debug` and errors redact source details and
file paths.

Wallpaper motion is an event-driven crossfade state in
`rmac-wallpaper::transition`. Its smoothstep is bounded to two seconds and asks
for frames only while opacity can change. Replacing an active target freezes
the current blend and requests one renderer capture before beginning the next
fade, so rapid changes do not jump or accumulate unbounded layers. Reduced
motion presents immediately, including when enabled mid-transition.

The session settings store—not the XDG Wallpaper portal—is the readable
wallpaper authority. Portal v1 is a sandboxed-app mutation API with no state,
output, or fit model. rmac will eventually implement its desktop backend for
confirmed local background requests; rmac Settings never consumes its own
portal. Remote fetching and lock-screen targets remain explicit failures until
an isolated importer and E5 secure-lock authority exist. See ADR 0003.

## Persistence

The target persistence contract is:

- XDG config/data/cache/state locations on Linux;
- versioned serde formats;
- temporary sibling write, flush, and atomic replacement;
- last-known-good recovery for settings;
- application-specific migrations;
- typed, user-visible errors for failed saves;
- no silent failure for data-changing operations.

The current prototype does not yet meet this contract everywhere; see the Phase
0 inventory.

## Safety boundaries

- Finder treats copy, move, trash, restore, and delete as journaled operations,
  not one-off UI callbacks.
- System settings use service-owned authorization or narrowly scoped D-Bus and
  polkit APIs, never arbitrary root commands.
- Desktop-entry command expansion follows the freedesktop specification and
  never passes through a shell.
- Logs exclude secrets and user document contents by default.

## Migration rule

Migrate one vertical slice at a time. Introduce a service and its fake, port one
consumer, verify behavior, then remove the old direct platform path. Avoid a
workspace-wide rewrite.
