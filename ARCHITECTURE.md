# Architecture

## Current system

rmac is a Cargo workspace with seven binary crates and two shared libraries.
Each application owns a GPUI root view and currently combines domain state,
platform access, persistence, and rendering in its `main.rs`.

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

The intended crate map and migration phases are specified in `PLAN_V2.md`.

## UI and update model

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

