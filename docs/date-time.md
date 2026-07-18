# Date & Time authority

System Settings presents a macOS-like Date & Time pane without maintaining a
second clock preference. On the supported Linux session, `rmac-time-linux`
reads and mutates the system authority at
`org.freedesktop.timedate1`; `rmac-time` keeps validation and transaction
semantics independent of D-Bus and GPUI.

The interface contract follows the upstream
[`org.freedesktop.timedate1`](https://www.freedesktop.org/software/systemd/man/latest/org.freedesktop.timedate1.html)
specification. A complete snapshot reads `Timezone`, `LocalRTC`, `CanNTP`,
`NTP`, `NTPSynchronized`, and `TimeUSec`, then obtains the installed inventory
through `ListTimezones`. The inventory is syntax-validated, sorted,
deduplicated, and bounded to 1,024 display values. The complete untruncated
inventory must still contain the current zone or the read fails closed.

## Mutations and readback

Time-zone changes accept only an exact installed zone. Automatic-time changes
are offered only when timedated advertises `CanNTP`. Both operations perform a
fresh preflight read, skip an already-satisfied request, use timedated's
interactive authorization path, and require an exact complete readback before
the UI reports success.

Manual clock editing is available only while automatic time is off. Input is a
canonical absolute timestamp in `YYYY-MM-DD HH:MM:SS +/-HH:MM` form. Requiring
the numeric UTC offset makes repeated or skipped local times around a daylight-
saving transition unambiguous. Control characters, non-canonical forms,
pre-1970 values, and values after 2261 (outside the conservative kernel timer
range used by live change detection) are rejected before D-Bus.

The pane shows a confirmation sheet because a clock jump can affect
certificate validity, scheduled work, and file timestamps. Confirmation takes
a fresh snapshot, refuses a concurrent automatic-time state, calls absolute
`SetTime` with interactive authorization, and requires the returned `TimeUSec`
to match the requested instant plus post-mutation readback time within a five-
second scheduling tolerance. Time spent in interactive authorization is not
misclassified as clock drift. Per timedated's contract, manual setting also
updates the hardware clock. The RTC's UTC/local mode remains read-only in rmac;
UTC is the recommended Linux configuration.

## Live state and recovery

Timedated property signals and well-known-name owner changes drive coalesced
complete snapshots. D-Bus properties do not signal a discontinuous clock jump,
so Linux also arms a nonblocking `CLOCK_REALTIME` timerfd with
`TFD_TIMER_ABSTIME | TFD_TIMER_CANCEL_ON_SET`; `ECANCELED` triggers a refresh
after an external clock change. The watcher reconnects after timedated loss and
reports a separate live-update failure while Settings retains the last known-
good snapshot.

Each manual refresh or mutation advances a generation. A stream read may
replace visible state only if its generation is still current and no load or
mutation is active; otherwise one pending refresh is retained. The visible
minute-resolution clock redraws at most twice per minute and only while the
Date & Time pane is open. It does not use a frame loop.

Errors exposed to the UI are bounded and control-character-free. D-Bus details
are used only to classify authorization denial/cancellation; raw bus messages,
paths, and peer details are never shown or persisted.

## Linux acceptance matrix

F12 remains unchecked until the Ubuntu/niri reference PC proves:

- NTP unavailable, off, enabling, synchronizing, synchronized, disabling, and
  exact post-mutation mismatch states;
- installed time-zone validation, a bounded/truncated inventory, successful
  change, authorization denial/cancellation, and external `timedatectl` change;
- successful manual clock setting, confirmation cancellation, authorization
  denial/cancellation, automatic-time conflict, and readback mismatch;
- external forward/backward clock jumps detected through timerfd, timedated
  stop/restart, suspend/resume, and last-known-good recovery;
- repeated and skipped daylight-saving local times using explicit offsets, and
  read-only UTC/local RTC presentation;
- keyboard-only editing, Enter/Escape confirmation, focus restoration, 100–200%
  scaling, contrast modes, Orca behavior, and the documented idle-wakeup bound.

Tests that change the real clock must run only on an isolated reference system
with recovery access. Record privacy-safe results; never commit hostnames,
users, addresses, paths, or raw authorization diagnostics.
