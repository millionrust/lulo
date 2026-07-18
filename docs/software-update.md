# Software Update authority

`rmac-updates` and `rmac-updates-linux` own the F10 boundary between System
Settings and the operating system's package authority. The pane provides the
familiar check, review, install, progress, and restart flow expected from a
desktop updater, while PackageKit and its Ubuntu APT backend remain the only
source of package truth. The UI never constructs package-manager shell
commands, parses localized command output, or treats a method return as proof
that package state changed.

## Authoritative update snapshot

The Linux adapter opens the system bus, reads PackageKit's advertised `Roles`,
creates a transaction with `CreateTransaction`, subscribes before starting the
operation, and invokes `GetUpdates` with the numeric `FILTER_NONE` bitfield.
Startup may reuse metadata up to one hour old; Check Again and every mutation
recovery request a zero cache age. The result is accepted only after a successful
`Finished` signal.

Package IDs must contain exactly four semicolon-separated fields, fit within
1,024 bytes, and contain no control characters. Display text is normalized and
bounded. At most 512 unique updates enter UI state. An additional unique result
marks the snapshot truncated and disables installation rather than silently
installing items the person could not review. Security, critical, blocked, and
normal priority values use the current numeric PackageKit enum contract.
Blocked packages remain visible but are never requested for installation.

Installation is offered only when the complete snapshot is not truncated, at
least one non-blocked update exists, and PackageKit advertises the
`UPDATE_PACKAGES` role. Missing capability is a visible limitation, not a
button that fails later.

## Trusted plan and confirmation

Selecting Install All performs another authoritative update read and then a
trusted-only simulation of `UpdatePackages`. The transaction combines
`ONLY_TRUSTED` and `SIMULATE`; rmac never retries without `ONLY_TRUSTED`. The
simulation captures the exact requested updates, additional installs,
reinstalls, removals, replacements, downgrades, and strongest restart signal.
It rejects unknown actions, untrusted-package signals, malformed IDs, empty
plans, and plans above 1,024 unique changes.

No package mutation has started when the confirmation sheet appears. The sheet
shows requested and dependency counts, calls out removals/replacements and
downgrades, previews destructive package names and versions, reports the
expected restart class, and warns that interactive authorization may appear.
Cancel simply discards the plan.

Confirmation is not permission to apply a stale plan. Immediately before the
real transaction, the adapter refreshes the update set, requires the exact
sorted requested IDs to match, reruns the trusted simulation, and requires the
entire typed plan to match. Any repository, dependency, version, action,
summary, or restart change returns to review instead of being applied under an
older confirmation.

## Installation, progress, and cancellation

The real `UpdatePackages` call retains `ONLY_TRUSTED` and enables PackageKit's
interactive hint so the system polkit agent can request authorization. rmac
does not collect or store administrator credentials. Authorization denial or
cancellation, signing failures, untrusted packages, licences, required media,
package-database changes, low disk space, backend errors, and timeouts become
bounded, privacy-safe error classes; arbitrary backend detail never enters the
UI.

PackageKit owns download and installation. The adapter listens on the exact
transaction object path for both transaction signals and standard D-Bus
`PropertiesChanged` progress notifications. It rereads `Status`, `Percentage`,
`AllowCancel`, and `RemainingTime`; invalid percentages become indeterminate.
Package signals provide the current bounded package name and restart signals
are retained. The Cancel button is enabled only while PackageKit says the
transaction can be cancelled. A request is sent at most once and the UI keeps
the transaction active until PackageKit reports completion.

Checks have a 60-second bound, simulations a five-minute bound, and installation
a four-hour total bound. Ten minutes without any transaction signal triggers a
safe cancellation attempt only when PackageKit advertises `AllowCancel`; rmac
continues owning a non-cancellable transaction until it finishes or reaches the
total bound. These bounds prevent a disappeared service from leaving Settings
permanently busy; they do not claim that a failed or cancelled transaction made
no partial changes.

## Live changes and recovery

Root `UpdatesChanged`, `RepoListChanged`, and `RestartSchedule` signals are
refresh hints. PackageKit owner reappearance also forces a complete read; its
normal idle daemon exit is not presented as an outage. System Settings
coalesces bursts, keeps one pending refresh through manual checks and update
transactions, and generation-checks stream results so a read started before a
mutation cannot overwrite its recovery state.

After every installation result—success, failure, cancellation, or timeout—the
pane takes an independent zero-cache-age snapshot. A successful install is
reported only alongside authoritative remaining-update state. If recovery
fails, the old snapshot is cleared so it cannot enable a retry against unknown
package state. A successful recovery may replace state after a failed
transaction, but the original failure remains visible. While installation is
active, closing System Settings is refused; preparation can be cancelled
without starting package changes.

The adapter follows PackageKit's current
[`Transaction` D-Bus interface](https://github.com/PackageKit/PackageKit/blob/main/src/org.freedesktop.PackageKit.Transaction.xml),
its [numeric enum definitions](https://github.com/PackageKit/PackageKit/blob/main/lib/packagekit-glib2/pk-enum.h),
and the daemon's current
[`PropertiesChanged` implementation](https://github.com/PackageKit/PackageKit/blob/main/src/pk-transaction.c).

## Evidence and remaining gate

Focused domain tests cover numeric classification, bounded and exact package
identity, deduplication, truncation, trusted simulation, destructive plan
classification, unknown percentage handling, cancellation state, restart
strength, privacy-safe errors, and bounded installation results. Adapter tests
pin the current bitfields, progress notification names, and normal PackageKit
idle-exit behavior. System Settings tests prove that live snapshots cannot
cross initial loading or an update transaction generation.

F10 remains unchecked until the Ubuntu/niri reference PC proves:

- fresh and cached reads through the APT backend, no-update, security, critical,
  blocked, malformed, oversized, repository-change, and daemon-unavailable
  states;
- simulation and confirmation of dependency installs, reinstalls, removals,
  replacements, downgrades, restart classes, and a plan that changes between
  confirmation and execution;
- successful interactive authorization, denial, dismissal, and cancellation,
  with no credential handling inside rmac;
- download and install progress with determinate and indeterminate percentages,
  remaining time, every cancellable/non-cancellable phase, package-manager lock
  contention, backend restart, and Settings close attempts;
- network loss, insufficient space, invalid/missing signatures, untrusted
  packages, licence/media requirements, backend conflicts, partial failure,
  cancellation, timeout, recovery failure, and exact remaining-update readback;
- application, session, system, and security restart reporting followed by the
  required sign-out or reboot; and
- keyboard-only operation, focus visibility, 100–200% scaling, contrast,
  reduced motion, Orca names/state announcements, bounded latency, and idle
  wakeups.
