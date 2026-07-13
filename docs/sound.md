# Sound authority

System Settings treats the user's PipeWire graph and WirePlumber policy as the
Linux authority for audio devices, defaults, volume, and mute. It does not keep
a private audio preference document or present alert sounds, interface effects,
or balance controls that have no reviewed session authority.

## Current snapshot and mutations

`rmac-audio` runs blocking audio queries and mutations off the UI thread. The
current Linux adapter uses argument-separated official `wpctl` operations for
the default sink/source level, mute, device inventory, and default-node change.
The inventory comes from the tab-separated machine-readable `wpctl list`
contract: numeric object ID, exact `node.name`, media class, and default marker.
The private node name is retained only to revalidate a selected node against a
fresh graph; it is omitted from `Debug` output and never persisted. The numeric
ID is treated as current-graph identity rather than a stable hardware ID.

`pw-dump --no-colors` supplies best-effort friendly descriptions only. rmac
accepts descriptions solely from PipeWire Node objects whose media class is
`Audio/Sink` or `Audio/Source`; an invalid or unavailable dump cannot replace
the authoritative `wpctl list` inventory. Both standard output and standard
error are drained to process completion while retaining at most 4 MiB each, so
a large graph cannot deadlock the child or grow memory without bound. The dump
is held in memory only long enough to extract bounded labels.

Volume reads target the exact advertised default node IDs. Default-device
mutation re-reads the selected ID and private node name before issuing
`wpctl set-default`, polls the machine-readable inventory for at most three
seconds, and returns a complete snapshot only after the same identity is
advertised as default. Settings consumes that verified snapshot directly;
command completion alone is never presented as success. Volume and mute
mutations likewise end in a complete readback.

## Live changes and recovery

On Linux, the audio watcher starts the official `pw-mon --color=never` monitor
with no shell, null input, bounded captured output, and kill-on-drop ownership.
It treats monitor output only as a graph-change hint and always rereads the
complete audio snapshot. A 75 ms quiet period coalesces event bursts, including
hotplug and volume changes, into one refresh on a capacity-one channel. Reads
use a fixed 8 KiB buffer instead of accumulating monitor text, and a 250 ms
maximum coalescing window prevents a continuously changing graph from starving
refresh.

Monitor startup failure or exit emits one non-droppable unavailable event and
retries after one second. Recovery emits one non-droppable changed event before
normal coalescing resumes. System Settings keeps the last known-good audio
snapshot and reports the live-stream outage separately.

Manual refresh and every audio mutation increment a generation. A live snapshot
is accepted only if its captured generation is still current and no load or
mutation is active. Events arriving during a mutation set one pending refresh,
which runs after the mutation finishes; an external change or service recovery
cannot be silently lost or overwrite newer readback.

## Explicitly remaining

F6 remains open until rmac adds and the Ubuntu/niri reference PC proves:

- advertised device profiles and routes with exact identity, availability,
  mutation, authoritative readback, hotplug, and rollback-safe failure states;
- per-channel balance only where a real channel map supports it;
- PipeWire and WirePlumber stop/restart, missing tools, monitor failure,
  external changes, Bluetooth/USB/HDMI hotplug, and suspend/resume behavior;
- keyboard, 100–200% scaling, contrast, Orca, slider latency, event coalescing,
  idle wakeups, and combined shell behavior.

The implementation follows the official
[`pw-mon` interface](https://pipewire.pages.freedesktop.org/pipewire/page_man_pw-mon_1.html),
[`pw-dump` interface](https://pipewire.pages.freedesktop.org/pipewire/page_man_pw-dump_1.html),
and [WirePlumber `wpctl` interface](https://pipewire.pages.freedesktop.org/wireplumber/tools/wpctl.html).
