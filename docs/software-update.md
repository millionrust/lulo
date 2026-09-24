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

## Automatic Lulo OS updates

Lulo OS's own packages update themselves without a visit to System Settings.
The rmac-session package ships `rmac-update-check.timer` and
`rmac-update-check.service` as user units; `rmac-session.target` wants the
timer. The timer fires once a day (`OnCalendar=daily`, spread over an hour by
`RandomizedDelaySec=1h`), catches up after the machine was off
(`Persistent=true`), and also fires 15 minutes after the user manager starts
(`OnStartupSec=15min`), so a laptop that is only on briefly still gets checked.
The service runs `/usr/libexec/rmac/rmac-update-check` once and exits, at
`Nice=10` and idle I/O priority, with a two-hour start timeout for large
downloads. Nothing stays resident between runs.

The program is Python 3 and uses PackageKit's own client library through
GObject introspection (`gir1.2-packagekitglib-1.0`, `python3-gi`). It never runs
a package-manager command and never parses command output. Each run:

1. Creates a non-interactive, background PackageKit client
   (`set_interactive(False)`, `set_background(True)`), refreshes the metadata
   with `refresh_cache(False)` and reads `get_updates` with the `NONE`
   filter. Blocked updates are ignored.
2. Splits the updates by exact package name. `rmac-apps`, `rmac-session`,
   `rmac-archive-keyring`, `niri` and `xwayland-satellite` are Lulo OS
   packages; everything else is an ordinary update.
3. Prepares the Lulo OS updates as a PackageKit offline update:
   `update_packages` with `ONLY_TRUSTED | ONLY_DOWNLOAD` for exactly those
   package IDs, then `offline_trigger(REBOOT)`. The transaction always keeps
   `ONLY_TRUSTED`; an unsigned or untrusted package makes the download fail
   instead of being accepted. If PackageKit's prepared update
   (`offline_get_prepared_ids`) already holds exactly the same IDs and the
   pending action is already reboot, nothing is downloaded or triggered
   again. If the prepared IDs match but the trigger was cancelled, the run
   triggers again without downloading.
4. Sends at most one notification over `org.freedesktop.Notifications`
   (app name "Software Update", icon `software-update-available`): "Lulo OS
   updates are ready — they will be installed the next time you restart."
   when the offline update is prepared, and "N other updates are available —
   open System Settings to review and install them." for the remaining
   updates. Both sentences share one notification when both apply. A run
   with no updates sends nothing.

On the next restart systemd enters `system-update.target` because PackageKit
created the `/system-update` link, and `packagekit-offline-update.service`
(`/usr/libexec/pk-offline-update`) installs the prepared packages before the
desktop starts, then reboots again. The install itself runs as root under
systemd; the user's session is never replaced mid-session.

**Authorization.** A download-only update needs no authorization:
`pk_transaction_obtain_authorization()` in PackageKit's
[`src/pk-transaction.c`](https://github.com/PackageKit/PackageKit/blob/v1.3.4/src/pk-transaction.c)
returns early ("No authentication required") when the cached transaction flags
contain `ONLY_DOWNLOAD` or `SIMULATE`, so the `system-update` polkit action
(`auth_admin_keep` on Ubuntu 26.04) is never consulted. When that
download-only `UpdatePackages` finishes, `pk_transaction_offline_finished()`
records its package IDs as the prepared update. Triggering uses the
`org.freedesktop.packagekit.trigger-offline-update` action, which Ubuntu
26.04's policy grants to the active user without a prompt. The program never
asks for interaction. If a site policy makes PackageKit refuse anyway
(`not-authorized`, `failed-auth` or `declined-interaction`), the Lulo OS
updates are counted with the others in the "open System Settings"
notification, where the normal interactive flow applies.

**Errors.** If PackageKit or the system bus is unavailable, the metadata
refresh fails (for example expired or unsigned repository metadata), or the
download or trigger fails, the program writes one bounded line to the user
journal, such as `rmac-update-check: refresh failed: gpg-failure`, and exits
non-zero. `systemctl --user status rmac-update-check` then shows the failed
run. The line carries only the PackageKit error class, never the GError
message, which can contain URLs, paths or package names. A download or
trigger failure still sends the fallback notification, counting the Lulo OS
updates with the others, and the unit still fails. That way the person can
act and the failure stays visible. A missing notification service is logged
and is not a failure. A missing PackageKit client library exits with status 2.

**Opting out.** `systemctl --user disable --now rmac-update-check.timer` turns
off both the daily notification and the automatic preparation. Updates remain
available in System Settings > Software Update.

**Phasing.** PackageKit's APT backend reports as updates what libapt marks
for upgrade in its dependency cache. Neither the backend nor this program
reads `Phased-Update-Percentage`. It is **unverified** whether PackageKit's
APT backend holds back phased Ubuntu updates the way `apt upgrade` does.
This matters only for the "other updates" count: Lulo OS's own repository
does not publish phased updates, so the prepared Lulo OS set is unaffected.

**Replacing a prepared update.** PackageKit records the IDs of the latest
download-only `UpdatePackages` as the prepared update. If another tool has
already prepared a different offline update, a new Lulo OS preparation takes
its place. System Settings installs online, and we know of no stock Ubuntu
26.04 component that prepares PackageKit offline updates in the background,
so nothing on Lulo OS should compete for the prepared update today. Whether PackageKit 1.3.4 replaces or
merges an existing prepared list is unverified.

### What the reference PC still has to prove

`scripts/test_update_check.py` runs the program against a fake `gi` package
that records every PackageKit and D-Bus call. On the reference PC, a
read-only run through the real `PackageKitGlib` (with refresh, download,
trigger and notification stubbed) confirmed the method signatures, enum
values, error domains and the `only-trusted;only-download` bitfield. The
automatic path is not proven until the reference PC completes a real cycle:

- a Lulo OS package update is published to the signed repository; the timer
  (or `systemctl --user start rmac-update-check.service`) refreshes, downloads
  and triggers without any polkit prompt; `offline_get_prepared_ids` lists
  exactly the Lulo OS IDs and the "ready" notification appears once;
- a second run with no new version performs no download and no trigger;
- a restart enters `system-update.target`, `pk-offline-update` installs the
  packages, the machine reboots into the updated Lulo OS session, and
  `dpkg-query` shows the new versions;
- expired or unsigned repository metadata, a network loss during download and
  a refused trigger each leave the unit failed with one journal line, and
  never schedule a partial update;
- with the notification centre stopped, the run still succeeds; and
- `systemctl --user disable --now rmac-update-check.timer` stops both the
  notification and the preparation.
