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
notification center, and wallpaper are independent service units with bounded
restart rates. A small supervisor publishes runtime health, records restart
budget exhaustion, and moves the session to a supervisor-only diagnostic target
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
and per-provider privacy/network policy. Separate shell processes watch one XDG
configuration file and refresh from authority after coalesced change events.
Writes atomically replace both the primary and last-known-good documents;
schema migration and corrupt-primary recovery happen below every UI surface.

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
frame.

Quick Settings consumes the same typed service snapshots through
`rmac-quick-settings`. Its framework-neutral transaction model validates
capabilities, permits only one in-flight mutation per control, retains the
authoritative value while work is pending, and accepts success only alongside
a refreshed authority snapshot. Stale task completions cannot overwrite newer
state, and failures preserve last-known-good values with user-visible detail.
`rmac-quick-settings-system` is the blocking adapter above that boundary. It
maps validated commands to NetworkManager, BlueZ, PipeWire/WirePlumber,
power-profiles, and shell-settings operations, then rereads only the affected
authority. The GPUI popover must run this adapter off its render executor.

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
