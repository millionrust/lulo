# Storage authority

`rmac-mounts` owns the F11 boundary between System Settings and mounted-volume
state. The Storage pane is deliberately narrower than macOS Storage until Linux
can provide equally authoritative category measurements: it shows real mounted
volumes and capacity, warns about low space, and offers a safe route into Files.
It never invents a “Macintosh HD”, estimates categories from filenames, or
deletes data based on a heuristic.

## Mounted-volume snapshot

The system volume is presented as `System Volume` and measured directly at `/`.
On Linux, additional visible mounts come from the current process mount
namespace in `/proc/self/mountinfo`. Only descendants of `/media`, `/run/media`,
and `/mnt`, plus the current user's GVfs mount directory, are admitted. Internal
procfs, sysfs, container, and implementation mounts remain hidden.

The mount table read is limited to 4 MiB and must be valid UTF-8. Mount IDs must
be decimal, escaped fields must use the kernel's documented mount encoding, and
the result is deduplicated by exact mount point. More than 256 unique visible
mounts fails the complete snapshot instead of silently hiding a volume. Visible
names are control-character-free and limited to 256 bytes. GVfs identities do
not expose remote host or account fields: an available share label is shown,
otherwise the pane uses `Remote Volume`.

Each mount retains a private opaque namespace identity separate from its name
and path. That identity is never rendered. A mount or unmount is only a refresh
hint; all displayed state is rebuilt from a complete snapshot.

## Capacity and low-space state

Every volume is measured independently with `statvfs`. Total bytes use the
filesystem fragment size when available, available bytes use the unprivileged
`f_bavail` value, and arithmetic saturates before entering UI state. One
disconnected, inaccessible, or unsupported volume gets its own capacity error
without hiding healthy volumes. Available space can never exceed total space,
and progress bars are clamped between zero and one.

A volume is marked low on space when fewer than 5 GB are available or fewer
than five percent of its capacity remains. The warning recommends reviewing
large personal files and application caches, but does not claim that any item
is safe to delete.

## Live refresh and safe review

Linux exposes `/proc/self/mounts` as a pollable file: mount-namespace changes
produce a priority event. `rmac-mounts` waits for that kernel event on a blocking
worker with a five-second close check and sends a bounded hint to System
Settings. The UI coalesces event bursts, retains one pending read through a
manual refresh, and generation-checks results so a stale live read cannot
overwrite the manual snapshot. Watcher failure is separate from capacity state,
so last-known-good volumes remain visible.

`Review in Files` is non-destructive. Before opening anything, rmac rereads the
mount namespace and requires the exact opaque identity, path, and ejectable
class to match the selected row. The desktop portal then opens the revalidated
directory by file descriptor where supported. A stale or replaced mount fails
and triggers a fresh snapshot; visible labels are not used as authority.

Unmount, eject, category cleanup, cache deletion, and Trash-emptying controls
remain absent. Those actions require exact ownership, confirmation, bounded
completion, authoritative readback, and a recovery story; a generic shell
command or irreversible “clean” button is not an acceptable substitute.

The live watcher follows the Linux
[`/proc/pid/mounts` poll contract](https://man7.org/linux/man-pages/man5/proc_pid_mounts.5.html),
while snapshot parsing follows the documented
[`/proc/pid/mountinfo` format](https://man7.org/linux/man-pages/man5/proc_pid_mountinfo.5.html).

## Evidence and remaining gate

Focused tests cover kernel field decoding, malformed escape and ID rejection,
visible-mount filtering, exact deduplication, remote-identity redaction,
control-character and size bounds, maximum-volume refusal, saturating capacity
math, low-space classification, and live root-capacity relationships. System
Settings coverage proves that mount-event snapshots cannot cross initial load
or manual-refresh generations. The Linux-target build proves the `POLLPRI`
watch adapter and bounded worker compile for the supported platform.

F11 remains unchecked until the Ubuntu/niri reference PC proves:

- the system disk, USB storage, SD cards, encrypted media, network shares, GVfs
  mounts, bind mounts under admitted roots, and names containing spaces or
  non-ASCII characters;
- live mount/unmount/eject, rapid reconnect, stale Review selection, namespace
  changes, inaccessible and disconnected capacity, watcher failure, and
  suspend/resume behavior;
- ext4, Btrfs, FAT, exFAT, NTFS, network, read-only, nearly full, reserved-block,
  zero-capacity, and very large filesystem measurements against trusted system
  tools;
- portal success, denial, cancellation, missing Files integration, and proof
  that remote account/host identity and private paths never enter visible
  errors;
- a reviewed reversible cleanup design before any category or deletion action
  is added; and
- keyboard-only operation, focus visibility, 100–200% scaling, contrast,
  reduced motion, Orca names/state announcements, bounded refresh latency, and
  idle wakeups.
