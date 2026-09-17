# Global shortcuts

`rmac-shortcuts` provides one stable shortcut vocabulary for shell consumers
and two Linux backends: the versioned XDG GlobalShortcuts portal and an
explicit niri include fallback. It never grabs input devices, uses X11 key
grabs, or listens to arbitrary keystrokes.

## Shortcut vocabulary

The initial shell actions are launcher, application drawer, Notification
Center, Quick Settings, and lock. IDs contain only lowercase ASCII letters,
digits, dots, and hyphens. Descriptions and triggers are bounded and reject
control characters; IDs and triggers must be unique.

Portal preferences use the freedesktop shortcuts syntax (`LOGO`, `CTRL`, and
XKB key identifiers joined by `+`). The niri representation is separate and
uses its documented `Mod+Key` syntax. This prevents accidental string reuse
between two different protocols.

## Portal backend

On Linux the broker connects to
`org.freedesktop.portal.GlobalShortcuts`, reads its `version` property, and
requires v1 for session creation, binding, and activation signals. It reports
whether v2's configuration UI is available. Signal subscriptions are opened
before session creation so activation/change events cannot be lost between
binding and watch setup.

The portal owns user consent and the actual trigger choices. The broker
publishes returned human-readable trigger descriptions and reconnects when the
portal or its backend restarts. Backend state is atomically written to
`$XDG_RUNTIME_DIR/rmac/shortcuts-status.json`.

`rmac-shortcuts` exposes a typed reader for that atomic snapshot. System
Settings uses it to report whether the active session selected the portal or
requires the niri fallback, while displaying both stable trigger forms. The
snapshot is diagnostic authority only: Settings can refresh it but cannot
invent, bind, or silently switch a shortcut backend.

When the live portal reports version 2 or newer, Settings exposes Configure.
The action sends one versioned, bounded request to the broker-owned
`$XDG_RUNTIME_DIR/rmac/shortcut-broker-control.sock`; it does not create a
competing portal session. The broker calls `ConfigureShortcuts` with its exact
live session handle and returns a matching request acknowledgement. The
runtime directory is mode 0700, the broker and ephemeral reply sockets are
mode 0600, request IDs must match, reads time out after three seconds, and
identity-checked cleanup never removes a replacement socket. Unsupported
portal versions and portal/request failures stay distinct internally, and
accepted requests are coalesced for two seconds so repeated clicks cannot
create a portal-dialog burst, while Settings presents a path-free failure. The
portal remains responsible for the configuration UI, consent, and final
bindings; its API configures every shortcut in the session rather than only
Spotlight.

Activated IDs pass through `rmac-shortcut-dispatch`, which accepts only the
compiled allowlist. A normal shell ID becomes a small JSON message on its own
`$XDG_RUNTIME_DIR/rmac/shortcut-<allowlisted-id>.sock`. Each separately
supervised surface binds only its compiled action, validates the message again,
and turns accepted datagrams into monotonically sequenced typed activations.
This prevents one surface from consuming another surface's shortcut and keeps
crash domains independent. Runtime directories are mode 0700; non-socket path
collisions and a second live owner fail closed, while a stale socket is removed
before binding. Lock is the deliberate exception: it starts the
fixed `rmac-lock.service` directly and waits for the lock readiness transaction,
so shell availability cannot turn a security action into a dropped event. No
shortcut is converted into a shell command.

The supervised launcher, Apps, Quick Settings, and Notification Center
panel processes bind their own endpoints, signal systemd readiness only after
the socket exists, and order the broker after all four ready units. Each keeps
at most one on-demand GPUI surface and treats a repeated activation as
dismissal. Apps retains a separate standalone mode for development and
performance measurement; only its explicit service mode owns the shortcut.
The notification daemon remains a separate D-Bus authority and cannot consume
the panel shortcut. Every compiled ordinary shell action therefore has a live,
crash-isolated consumer before the broker accepts activations.

## Explicit niri fallback

The development installer always generates
`$XDG_CONFIG_HOME/rmac/niri-shortcuts.kdl` using an atomic write. It does not
edit the user's niri configuration. Enable the printed `include` line only
when `shortcuts-status.json` says `fallback-required`; remove or disable it
when the portal backend becomes available, so there is exactly one shortcut
owner.

Generated bindings use `repeat=false`, expose titles in niri's hotkey overlay,
and call the dispatcher with separate `spawn` arguments. They never use
`spawn-sh`, `sh -c`, interpolation, or user-provided command text. niri 26.04
supports included KDL files and live validation/reload. Only the lock binding
uses `allow-when-locked=true`, matching niri's documented dead-locker recovery
path; ordinary shell actions remain unavailable while locked.

## Verification

Tests prove the default IDs and both trigger forms are unique and valid,
reject malformed input and relative dispatchers, prove action-scoped endpoints,
prove the configuration request/acknowledgement identity and ephemeral-socket
cleanup, and inspect the generated KDL for one shell-free dispatch per
shortcut. The real portal implementation, including the broker-owned v2
configuration call, is cross-compiled through the Linux Rust target on the
development host. Final evidence still requires the Linux PC to record portal
version/consent/configuration, each activation, fallback validation, portal
restart, niri reload, and keyboard layout behavior.

Primary contracts:

- <https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.GlobalShortcuts.html>
- <https://specifications.freedesktop.org/shortcuts/latest/>
- <https://specifications.freedesktop.org/global-shortcuts-spec/latest/>
- <https://github.com/YaLTeR/niri/wiki/Configuration%3A-Key-Bindings>
- <https://github.com/YaLTeR/niri/wiki/Configuration%3A-Include>
