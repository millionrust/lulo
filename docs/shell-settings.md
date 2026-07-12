# Shell settings authority

`rmac-shell-settings` is the durable, compositor-independent authority shared
by the top bar, Dock, launcher, wallpaper service, notification/Focus service,
and shell-owned Settings panes. It contains no GPUI, Wayland, niri, D-Bus, or
platform command code.

## Schema ownership

The current v3 document stores:

- ordered, unique desktop application IDs pinned to the Dock;
- Dock edge, all/primary/named-output scope, autohide, magnification and scale,
  reserved-space behavior, and repeated-click behavior;
- locale/12-hour/24-hour clock format plus date, seconds, and workspace labels;
- independent visibility for meaningful network, VPN, Bluetooth, sound, power,
  battery-percentage, notification, and Focus indicators;
- default and per-output wallpaper source plus fit policy; sources are the
  original `builtin:rmac-aurora`, a hostless local `file:///` URI, or a
  normalized absolute path;
- legacy Focus selection fields retained only for migration while
  `rmac-focus-store` owns live modes, schedules, and manual expiration;
- per-provider enablement and explicit private-content/network permissions;
- bounded normalized Spotlight exclusions and removable-media search opt-in.

The store persists policy and user choices, not service results. For example,
it does not claim a wallpaper was decoded, a provider is currently available,
or a legacy Focus preference was successfully applied. Live Focus mutations go
through the Focus authority; runtime authorities publish their own state and
errors.

## Durability contract

Linux resolves the primary file to `$XDG_CONFIG_HOME/rmac/shell.json`, falling
back to `$HOME/.config/rmac/shell.json`; macOS development uses
`~/Library/Application Support/rmac/shell.json`.

Every save validates the complete snapshot before touching disk. The store
then serializes one versioned document and atomically replaces the
`shell.json.last-good` sibling before replacing the primary. If the primary
write fails, the previous last-known-good bytes are restored. Atomic writes are
adjacent, flushed, renamed, and followed by a parent-directory flush through
`rmac-storage`.

Loading prefers the primary. A missing or malformed primary recovers the
last-known-good document and reports that recovery in the returned snapshot;
it never silently claims the primary was healthy. If both copies fail, the
typed error preserves the operation, path, I/O classification, and both
failure details.

## Migration and validation

The v1 migration preserves pinned apps, Dock edge/autohide/magnification,
wallpaper source, and selected Focus mode. The v2 migration adds Spotlight
scope defaults. Both validate the result and immediately rewrite primary and
last-known-good documents as v3. Unknown fields in the current version are
tolerated; unknown document versions are rejected rather than guessed.

Validation rejects duplicate/empty/control-character IDs, more than 128 pinned
apps, invalid named outputs or remote/relative wallpaper sources, non-finite or
out-of-range Dock magnification, invalid Spotlight exclusions, and an
expiration attached to disabled Focus. Provider policies default to local,
non-private, non-network access until the user or a trusted migration grants
more.

## Multi-process updates

The watcher observes only replacement of the primary file, ignores the
last-known-good sibling, and uses a bounded one-item channel so filesystem
bursts coalesce. On notification, consumers reload the complete authoritative
snapshot; they do not merge partial filesystem events into local UI state.

Tests cover v3 round trips, v1/v2 migration/rewrite, unknown fields and versions,
corrupt-primary recovery, validation, watcher filtering, and injected primary
write failure with last-known-good rollback.
