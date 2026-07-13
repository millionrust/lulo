# Display layout authority

`rmac-display` owns the F7 display transaction boundary for the niri session.
System Settings renders its typed snapshot and never treats a local control as
proof that the compositor changed.

The user outcome is familiar from macOS: connected displays have stable names,
the arrangement is visible, one enabled display is Main, and risky mode, scale,
rotation, or position changes must be kept within 15 seconds. The implementation
remains Linux-native and does not claim capabilities niri does not provide.

## Live state and identity

Linux reads `niri msg --json outputs`. The map key is the stable niri output
name used for IPC and configuration; the connector, make/model label, serial,
advertised modes, physical size, logical rectangle, scale, and transform remain
separate typed fields. Mutations retain the complete selected output and reread
the compositor immediately before sending an argument-separated command. A
changed connector, serial, or friendly hardware identity stops the operation.

Every successful command requires a bounded fresh readback within three
seconds. Command stdout and stderr are drained concurrently, retained only up
to 4 MiB each, and the process is stopped after ten seconds. Unknown transforms
survive discovery but are not offered as writable values.

The direct niri event stream has no dedicated output-change event. Snapshot,
output replacement, workspace replacement, and `ConfigLoaded` events are
therefore narrow refresh hints. Settings coalesces those hints, retains one
pending refresh through a mutation or confirmation, and resamples the complete
display snapshot. Window churn does not trigger a display read.

## Keep and Revert

Before a risky mutation, `rmac-display` captures a fresh complete baseline. The
successful result includes both that baseline and the authoritative post-change
snapshot. System Settings starts a 15-second countdown only after readback.

Revert restores mode, scale, transform, and position for every enabled output,
not merely the control that was clicked. It first proves that the connected
hardware is identical and that the current complete layout still equals the
post-change snapshot owned by this confirmation. An external layout change
therefore stops restoration instead of being overwritten. Commands are applied
in phases and the complete baseline must be observed again within three
seconds.

Keep serializes the complete live layout and saves it only if a fresh snapshot
still matches. Save failure leaves the prior persistent configuration in place
and restarts the confirmation countdown when the transient state is still
eligible for restoration.

## Persistent niri configuration

The authority is the existing absolute `$NIRI_CONFIG`, or
`$XDG_CONFIG_HOME/niri/config.kdl` with the normal home fallback. rmac refuses
symbolic links and configurations larger than 2 MiB. It owns only the adjacent
`.rmac-displays.kdl`; a fixed ownership header prevents adoption of an unrelated
file.

On first save, the main configuration receives one first-position relative
include. Existing main-config bytes otherwise remain untouched. The first
position is intentional because niri resolves the first matching output block.
The managed file contains only bounded, unique output blocks. Connected blocks
are replaced from the complete confirmed layout, while disconnected blocks are
preserved so a docked display does not lose its saved configuration. Exactly
one connected block receives `focus-at-startup`, which is the rmac Main Display
authority consumed by Settings and the Dock.

Before either live file changes, candidate main and managed files are written
beside the real configuration and checked with `niri --config … validate`.
Atomic replacement preserves file permissions and directory durability. The
previous managed file is retained as `.last-good`. If enabling or compositor
readback fails, both files are restored when possible; rollback failure is
reported explicitly instead of claiming success.

## Arrangement and mirroring

Settings shows proportional logical display rectangles and offers edge-aligned
Left, Right, Above, and Below placement relative to Main. Position changes use
the same timed complete-layout transaction as mode, scale, and rotation.

niri currently exposes extended output placement but no native mirror layout
mode, so the snapshot reports mirroring unsupported. Settings explains that
`wl-mirror` can mirror content without misrepresenting it as compositor-owned
display mirroring.

## Evidence and remaining gate

Unit coverage proves bounded parsing, exact non-overlapping layouts, stale mode
rejection, hardware-identity comparison, complete-layout ownership comparison,
managed-file isolation, offline-output preservation, unique Main selection,
bounded helper output, Dock Main-output readiness, and display refresh hints.

F7 remains unchecked until the Ubuntu/niri reference PC proves multi-monitor
startup and persistence, dock/undock and hotplug, lid and suspend/resume,
fractional scale, rotation, mode failure, countdown expiry, explicit Keep and
Revert, concurrent external edits, niri restart, keyboard operation, scaling,
and accessibility. Native mirroring remains an explicit compositor capability
limit rather than a hidden gate.
