# Input configuration authority

`rmac-input` owns the F8 keyboard, mouse, and trackpad transaction boundary for
the niri session. System Settings presents effective configuration and live
Linux input inventory; it does not treat a local toggle, a command exit, or a
kernel event number as proof of durable hardware configuration.

The intended experience is familiar from macOS: connected devices are visible,
keyboard repeat and Num Lock are understandable, and mouse and trackpad
controls react immediately. The authority remains Linux-native and preserves
niri settings that rmac does not expose.

## Effective niri configuration

The authority is the existing absolute `$NIRI_CONFIG`, or
`$XDG_CONFIG_HOME/niri/config.kdl` with the normal home fallback. An empty
`NIRI_CONFIG` is ignored. `/etc/niri/config.kdl` can be read but is never
rewritten; Settings asks the user to create a user configuration first.

Every source is parsed as KDL v1, matching niri rather than accepting ambiguous
KDL v2 syntax. The reader recursively expands top-level relative, absolute, and
`~/` includes at their exact positions. Optional missing files are retained as
graph dependencies so their appearance during a save is detected. The graph is
bounded to niri's ten-level recursion limit, 64 loaded files, 2 MiB per file,
and 8 MiB total. Recursive includes, malformed include nodes, duplicate input
blocks in one file, invalid UTF-8, and oversized graphs fail closed.

Availability requires a successful JSON `Version` request through the running
compositor's `$NIRI_SOCKET`, not merely an installed `niri` executable. When the
session is available, the current complete config is also checked by bounded
`niri --config … validate` before its values are presented as effective.

Keyboard sections merge across the graph. Repeat delay, repeat rate, and
explicit true or false Num Lock flags therefore preserve their effective
meaning. Mouse and touchpad sections replace earlier sections as complete
device-type configurations. An empty `xkb {}` follows `systemd-localed`; an
explicit empty `options ""` remains an override. XKB rules, layout, model,
variant, and keymap file are read for authority reporting but are not moved into
the rmac-owned file.

These rules follow niri's current [include semantics](https://github.com/niri-wm/niri/wiki/Configuration%3A-Include)
and [input contract](https://github.com/niri-wm/niri/wiki/Configuration%3A-Input).

## Isolated write ownership

rmac owns only the adjacent `.rmac-input.kdl`, identified by an exact fixed
header, and one exact final main-file node:

```kdl
include ".rmac-input.kdl"
```

An alias, optional form, nested reference, duplicate reference, non-final
reference, child block, foreign file, or symbolic link makes the graph
read-only. The main configuration and every loaded or missing optional graph
dependency are sampled before validation and again immediately before writing.
Any concurrent edit or newly appearing optional include stops the transaction.

The managed file may own only one argument-free `input` block containing one
keyboard, mouse, and touchpad section each. Its keyboard section is restricted
to repeat delay, repeat rate, and Num Lock. When rmac first changes a mouse or
touchpad value, it clones the complete effective section before replacing the
supported nodes. Unknown current settings such as scroll method, scroll factor,
or click method therefore remain intact. An explicit `off` remains off and the
corresponding controls are disabled instead of silently enabling that device
type.

The complete candidate main and managed files are written beside the live
configuration and checked with bounded `niri --config … validate`. Managed
output is forced to KDL v1. The previous managed file is retained as
`.rmac-input.kdl.last-good`; live files use atomic replacement. Enabling the
include, parsing the fresh graph, or complete settings readback failure triggers
rollback, and rollback failure is reported explicitly.

Immediately before live writes, rmac opens a bounded direct niri event stream,
observes its initial successful `ConfigLoaded` state, and checks the graph once
more. The transaction must then receive a new `ConfigLoaded { failed: false }`
within three seconds. A failed reload, timeout, disconnected compositor, or a
runtime-switched config path that is not watching these files restores the
previous files instead of claiming adoption. Standard output and standard error
are drained concurrently up to 4 MiB and the validator is stopped after ten
seconds; IPC event lines have the same size bound.

## Live device state

On Linux, the inventory enumerates bounded `/sys/class/input/event*` entries.
The kernel event name is treated as ephemeral and is never persisted. Device
type comes from udev `ID_INPUT_*` capability properties when available, with a
conservative name fallback. rmac reads the display name but not the udev serial
or kernel `uniq` field. Settings lists keyboards, mice, trackpads, pointing
sticks, trackballs, tablets, touchscreens, and unclassified devices without
claiming that every type shares the same writable settings.

A bounded watcher treats `/dev/input` changes only as refresh hints and always
resamples the full snapshot. Only successful niri `ConfigLoaded` events refresh
the complete include graph. Settings permits one input refresh in flight,
retains one pending refresh through a mutation, and generation-checks results so
stale hotplug or config reads cannot replace newer transaction readback.
Watcher failure is shown separately while the last known-good snapshot remains
visible.

Current niri applies settings to every device of a type and explicitly does not
support configuring an individual device. System Settings says so rather than
using unstable `eventN` numbers as fake identities. Pointing sticks and
trackballs are inventoried separately and are not mislabeled as mice.

## Evidence and remaining gate

Focused unit coverage proves KDL v1 flags and strings, keyboard merge and
pointing-section replacement, empty versus explicit XKB state, positional and
optional include traversal, cycle and malformed-include rejection, managed
ownership and exact-final-include rules, preservation of unknown pointer nodes,
source-concurrency rejection, bounded command output, value ranges, and udev
classification. System Settings coverage proves that only `ConfigLoaded`
refreshes input configuration and that stream results cannot cross mutation
generations.

F8 remains unchecked until the Ubuntu/niri reference PC proves:

- built-in, USB, Bluetooth, docked, and removed keyboard/mouse/trackpad state,
  plus pointing-stick, trackball, tablet, and touchscreen inventory where
  available;
- nested relative, absolute, home, optional, cyclic, duplicate, aliased,
  malformed, missing, oversized, and concurrently edited include graphs;
- empty localed XKB, explicit layouts/options, Num Lock false, complete
  pointing-section replacement, explicit `off`, and preservation of settings
  not exposed by rmac;
- main and managed symlink refusal, foreign ownership, validator failure,
  atomic save, last-good creation, readback mismatch, and successful and failed
  rollback;
- hotplug bursts, niri reload/restart, watcher outage/recovery, suspend/resume,
  and interaction during an input mutation; and
- keyboard-only operation, focus visibility, 100–200% scaling, contrast,
  reduced motion, Orca names/state, bounded latency, and idle wakeups.

Individual-device overrides remain an explicit upstream capability gap rather
than an unverified implementation claim.
