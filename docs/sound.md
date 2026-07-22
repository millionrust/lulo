# Sound authority

System Settings treats the user's PipeWire graph and WirePlumber policy as the
Linux authority for audio devices, defaults, volume, mute, ports, and device
profiles. It does not keep a private audio preference document or expose
unimplemented alert-sound preferences. The notification authority may submit
only policy-approved E2 cues through the bounded playback path below.

## Notification cue playback

`rmac-audio` plays notification cues with the official `pw-play` client and
marks the PipeWire stream as `Playback`/`Notification`. The default is an
original 420 ms rmac two-tone cue synthesized into mono 48 kHz PCM WAV; it does
not copy a platform vendor's sound. Portal custom cues remain limited to the
already structurally validated Ogg Opus, Ogg Vorbis, and PCM WAV formats. The
audio boundary independently checks their 2 MiB bound and format signature
before starting a process.

Playback bytes are written into a sealed, non-executable `memfd`, rewound, and
given to `pw-play` as its seekable standard input through `/proc/self/fd/0`.
There is no shell, command interpolation, temporary pathname, captured media
diagnostic, or durable copy. Standard output/error are discarded, process
errors expose classifications only, cancellation kills the child, and a hard
15-second wall-clock deadline kills and reaps it even though the earlier media
validator also checked declared duration.

The E2 `SoundPlayer` has one cancellation-safe admission. A concurrent cue
returns an explicit busy result instead of spawning another decoder/player
during a flood. The eventual layer-surface host must submit emitted cues through
this player off its sole runtime-event receiver task and show playback failure
separately from banner/action state. Ubuntu reference evidence records
`pw-play`, PipeWire, WirePlumber, and libsndfile versions; real sink routing,
mute/volume interaction, all three custom codecs, missing-tool behavior,
cancellation, and the deadline remain reference-PC gates.

## Current snapshot and mutations

`rmac-audio` runs blocking audio queries and mutations off the UI thread. The
current Linux adapter uses argument-separated official `wpctl` operations for
the default sink/source level, mute, device inventory, and default-node change.
The inventory comes from the tab-separated machine-readable `wpctl list`
contract: numeric object ID, exact `node.name`, media class, and default marker.
The private node name is retained only to revalidate a selected node against a
fresh graph; it is omitted from `Debug` output and never persisted. The numeric
ID is treated as current-graph identity rather than a stable hardware ID.

`pw-dump --no-colors` supplies friendly descriptions and advertised capability
metadata. rmac correlates a Node only when its object ID, exact private
`node.name`, and media class agree with the machine list; an invalid or
unavailable dump cannot replace that authoritative inventory. Both standard
output and standard error are drained to process completion while retaining at
most 4 MiB each, so a large graph cannot deadlock the child or grow memory
without bound. The dump is held in memory only long enough to extract bounded
typed state. If it is missing, invalid, oversized, or ambiguous, base audio
state remains available while profile and port configuration reports a separate
temporary failure.

Volume reads target the exact advertised default node IDs. Default-device
mutation re-reads the selected ID and private node name before issuing
`wpctl set-default`, polls the machine-readable inventory for at most three
seconds, and returns a complete snapshot only after the same identity is
advertised as default. Settings consumes that verified snapshot directly;
command completion alone is never presented as success. Volume and mute
mutations likewise end in a complete readback.

## Device profiles and ports

`rmac-audio` accepts profiles only from `Audio/Device` `EnumProfile` parameters
with one exact current `Profile`. It bounds and deduplicates indices and private
names, preserves PipeWire's available/unavailable/unknown state, and redacts
private device and profile names from `Debug`. A selection retains the current
device ID plus private `device.name`, revalidates both with a fresh graph,
rejects unavailable or stale profiles, calls the official argument-separated
`wpctl set-profile`, and requires the same profile to become current within
three seconds plus a complete snapshot readback.

Ports come from the device's `EnumRoute` and current `Route` parameters. A route
is attached to a sink or source only when its direction, active profile,
`device.id`, `card.profile.device`, node ID, and private node name all agree.
Duplicate indices, stale-profile Route records, unavailable routes, reused IDs,
and ambiguous device identities are not writable. Selection uses the exact
node and route index with `wpctl set-route`, then requires the same complete
private association and active Route readback. The domain exposes this state
without persisting any private PipeWire path or hardware identifier. System
Settings renders advertised output/input ports and hardware profiles with their
active and unavailable states, disables every choice while another mutation is
busy, and consumes the verified service snapshot directly after selection. A
profile may legitimately leave no default sink or source; Settings then hides
that direction's volume/mute controls, and Quick Settings marks Sound
unavailable instead of sending a mutation to a nonexistent default.

## Stereo balance

System Settings shows a macOS-style L/R balance control for the default output
only when the current PipeWire node advertises a writable `channelVolumes`
array and an exact two-channel `FL`/`FR` map. Read-only nodes, mono and surround
maps, ambiguous parameter records, malformed values, and effectively silent
channels do not expose the control. This keeps the absence of a safe mutation
contract visible instead of guessing how a device maps its channels.

Every balance change revalidates the exact current node and private route
association, rereads its channel order and volumes, and preserves the louder
channel while attenuating the other; rmac never amplifies a channel to create
balance. It sends an argument-separated `pw-cli set-param` `Props` update,
polls the exact balance for at most three seconds, and returns success only
after a complete snapshot contains the requested value. The current session is
authoritative; rmac does not claim or maintain separate balance persistence.

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

F6 remains open until the Ubuntu/niri reference PC proves:

- PipeWire and WirePlumber stop/restart, missing tools, monitor failure,
  external changes, Bluetooth/USB/HDMI hotplug, and suspend/resume behavior;
- keyboard, 100–200% scaling, contrast, Orca, slider latency, event coalescing,
  idle wakeups, and combined shell behavior.

The implementation follows the official
[`pw-mon` interface](https://pipewire.pages.freedesktop.org/pipewire/page_man_pw-mon_1.html),
[`pw-dump` interface](https://pipewire.pages.freedesktop.org/pipewire/page_man_pw-dump_1.html),
[`pw-cli` interface](https://pipewire.pages.freedesktop.org/pipewire/page_man_pw-cli_1.html),
[`pw-play` interface](https://pipewire.pages.freedesktop.org/pipewire/page_man_pw-cat_1.html),
[libsndfile format support](https://libsndfile.github.io/libsndfile/formats.html),
and [WirePlumber `wpctl` interface](https://pipewire.pages.freedesktop.org/wireplumber/tools/wpctl.html).
