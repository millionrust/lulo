# About and system identity authority

`rmac-system-info` owns the F9 boundary between System Settings and host
identity. The About pane presents a familiar concise hardware/software summary,
but it does not invent Mac model names, expose stable private identifiers, or
treat a successful method call as proof that the computer name changed.

## Authoritative facts

On Linux, the current, static, and pretty hostnames plus kernel name and release
come from `org.freedesktop.hostname1`. If that service is unavailable, the
read-only fallback is the kernel hostname and `/etc/hostname`; mutation remains
disabled. Operating-system identity comes from `/etc/os-release`, processor and
memory facts from procfs, manufacturer and model from DMI sysfs, and graphics
facts from `/sys/class/drm/card*/device`.

Graphics enumeration accepts only exact `card` plus decimal-number entries. It
reports the driver and public PCI vendor/device IDs with a small friendly vendor
mapping, sorts and deduplicates at most 16 devices, and never reads a PCI
address, serial number, machine ID, or kernel device identity. Session and
desktop labels come from the inherited XDG session environment. On macOS the
development build uses bounded `sw_vers`, `sysctl`, `scutil`, and
`system_profiler` reads for equivalent non-private facts while correctly
leaving Linux hostname administration unavailable.

Every file read has a byte limit and rejects invalid UTF-8. Every helper process
has null standard input, concurrent bounded standard-output and standard-error
drains, and a five-second timeout with termination. Individual facts are
trimmed, length-bounded, and rejected if they contain control characters. Slow
reads remain on the background executor.

## Hostname transaction and live refresh

The editable value is the Linux static hostname. Input is limited to 63 ASCII
letters, numbers, and hyphens, cannot begin or end with a hyphen, and is
normalized to lowercase before mutation. The service calls
`SetStaticHostname(value, true)` so systemd can request interactive polkit
authorization. Success requires a fresh complete snapshot whose
`StaticHostname` exactly equals the normalized request. Denial, cancellation,
service loss, mutation failure, or mismatched readback preserves the previous
UI snapshot and keeps the editor open with a specific error.

A bounded system-bus stream watches hostname properties and the
`org.freedesktop.hostname1` owner. Property changes and service recovery
coalesce into complete snapshot reads. Service loss is a separate dismissible
live-update error while last-known-good facts remain visible. System Settings
keeps one pending refresh and generation-checks stream results so a read started
before a hostname transaction cannot overwrite that transaction's authoritative
readback. The manual Refresh action independently resamples all About facts.

The behavior follows the official systemd
[`org.freedesktop.hostname1` contract](https://www.freedesktop.org/software/systemd/man/latest/org.freedesktop.hostname1.html).

## Privacy-safe report

Copy System Report includes only operating system, kernel, architecture,
manufacturer, model, processor, memory, graphics, session type, and desktop.
It deliberately excludes every hostname, account name, username, machine ID,
serial number, network address, and filesystem path. The report revalidates
even publicly constructed snapshot fields before adding them, so newline or
control-character injection cannot add disguised fields.

## Evidence and remaining gate

Focused tests prove hostname syntax and normalization, exact readback,
last-known-good failure behavior, report redaction and injection rejection,
Linux fact parsing, DRM-card admission, graphics labels, and hostname service
loss/recovery classification. System Settings coverage proves that stream reads
cannot cross initial loading or hostname mutation generations.

F9 remains unchecked until the Ubuntu/niri reference PC proves:

- successful, cancelled, and denied interactive hostname authorization, exact
  readback, invalid input, and preserved state after failure;
- hostname changes made outside rmac, hostnamed loss/recovery, event bursts,
  manual refresh, and interaction during an in-flight mutation;
- truthful OS/kernel/architecture, Intel/AMD/NVIDIA or virtual graphics,
  processor, memory, manufacturer/model, niri session, and desktop facts on the
  available hardware;
- clipboard inspection showing no hostname, username, machine ID, serial,
  address, or path; and
- keyboard-only editing/copying, focus visibility, 100–200% scaling, contrast,
  reduced motion, Orca names/state, bounded latency, and idle wakeups.
