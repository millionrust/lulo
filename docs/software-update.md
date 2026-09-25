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

The pane now downloads updates and installs them at restart (see "Update
Now: download, then Restart Now" below). The plan contract in this section
and the next applies to that path too; only the final transaction differs.

Preparing an update performs another authoritative update read and then a
trusted-only simulation of `UpdatePackages`. The transaction combines
`ONLY_TRUSTED` and `SIMULATE`; rmac never retries without `ONLY_TRUSTED`. The
simulation captures the exact requested updates, additional installs,
reinstalls, removals, replacements, downgrades, and strongest restart signal.
It rejects unknown actions, untrusted-package signals, malformed IDs, empty
plans, and plans above 1,024 unique changes.

No package mutation has started when the confirmation sheet appears. The pane
shows it only for a plan that removes, replaces or downgrades packages; it
gives the counts, previews the destructive package names and versions, and
Cancel simply discards the plan.

Confirmation is not permission to apply a stale plan. Immediately before the
real transaction, the adapter refreshes the update set, requires the exact
sorted requested IDs to match, reruns the trusted simulation, and requires the
entire typed plan to match. Any repository, dependency, version, action,
summary, or restart change returns to review instead of being applied under an
older confirmation.

## Installation, progress, and cancellation

The real `UpdatePackages` call retains `ONLY_TRUSTED`. The pane's download
adds `ONLY_DOWNLOAD` and no interactive hint, since it needs no
authorization; the adapter's online install still sets the interactive hint,
but Lulo OS runs no polkit agent (see "Authorization"). rmac does not collect
or store administrator credentials. Authorization denial or
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

## The Software Update pane (the Mac's model)

System Settings > General > Software Update follows macOS 26.2's pane,
measured on the owner's Mac (design-lab/software-update.html):

- **Lulo OS as one item.** Updates to `rmac-apps`, `rmac-session`,
  `rmac-archive-keyring`, `niri` and `xwayland-satellite`
  (`rmac_updates::LULO_OS_PACKAGES`, the same list `rmac-update-check`
  uses) form a single card: "Lulo OS 0.9.1" over "0.9.1 — 42.3 MB", an
  **Update Now** button, and ⓘ (More Info). The version comes from
  `rmac-session` (else `rmac-apps`) without its epoch or Debian revision
  (`0.9.0~beta.2-1` reads "0.9.0 Beta 2"); a component-only update is
  "Lulo OS Update". The size is the sum of PackageKit's `GetDetails`
  `download-size` (else `size`) for those packages; one unknown size hides
  the total rather than show a partial one. The release notes are drawn
  inline under the header, as the Mac draws its upgrade's notes, followed by
  "Once downloaded, this update will be installed when you restart."
- **Other updates are separate and secondary.** Every other installable
  update is one "Other Updates" card under an **Also Available** heading
  ("firefox, libc6 and 12 more…"), with its own Update Now and ⓘ. This is
  the Mac's split: its OS release is the primary card and everything else
  (point releases, Command Line Tools) is "Other Updates". Folding Ubuntu's
  updates into "Lulo OS" was rejected: they are released on Ubuntu's
  schedule, not ours, can be numerous, and a Lulo OS version must name
  exactly what Lulo OS shipped. When there is no Lulo OS update, Other
  Updates is the first card and the heading is dropped.
- **More Info sheet** (either ⓘ): "Updates are available for your
  computer", a table with a tick box, name, version and size per item (Lulo
  OS first, then each other update), and the selected item's details
  ("Lulo OS 0.9.1 — Restart Required" and its notes; for an Ubuntu package,
  its kind and summary). Cancel is on the left, Update Now on the right,
  and Update Now prepares exactly the ticked items.
- **Installed** shows the running system, and **Automatic Updates** shows
  On or Off with ⓘ for the switches (below). The pane checks for updates
  each time it opens, like the Mac; there is no Check Again button.
- **Update Tonight** (the Mac's scheduled overnight install) is not offered
  (SWU-05).

### Update Now: download, then Restart Now

Update Now never installs packages under the running session:

1. `prepare_selection` refreshes the update set, resolves the selection
   (`LULO_OS_ITEM` and/or package IDs) and adds every package already
   prepared for restart that is still offered, because PackageKit keeps a
   single prepared list and each download-only transaction replaces it.
   Blocked updates are never requested, and a truncated set cannot be
   prepared. It then simulates that exact set with `ONLY_TRUSTED |
   SIMULATE`.
2. A plan that removes, replaces or downgrades packages stops at a
   confirmation that names them ("This update removes or replaces
   software"). Any other plan goes straight on, as the Mac's Update Now does.
3. `prepare_offline` refreshes and re-simulates again. If any requested
   package disappeared or the dependency plan changed, it returns to review.
   Otherwise it runs `UpdatePackages` with `ONLY_TRUSTED | ONLY_DOWNLOAD`
   for exactly the plan's IDs, with the same progress, cancellation and time
   bounds as before, then calls `org.freedesktop.PackageKit.Offline.Trigger
   ("reboot")`.
4. The card's button becomes **Restart Now** and its subtitle "Ready to
   install — restart to finish" once `Offline.UpdateTriggered` is true and
   `Offline.GetPrepared()` holds every one of the item's packages. The pane
   learns this from the Offline interface's `PropertiesChanged` signal,
   which the existing PackageKit watch now also subscribes to (no polling).
5. Restart Now sends the `restart-to-update` dispatch
   (`rmac_shortcuts::power_key::RESTART_TO_UPDATE_SHORTCUT`). The menu bar
   runs the same quit-all path as its Restart…: every window is asked to
   close, an app that refuses (unsaved work) cancels the restart with a
   notice, and otherwise it runs `systemctl reboot`. systemd then enters
   `system-update.target`, `pk-offline-update` installs the prepared
   packages, and the computer restarts into the updated system.

The older reviewed online install (`rmac_updates_linux::install`, the
`UpdatePackages` without `ONLY_DOWNLOAD`) is kept in the adapter but no
longer used by the pane.

### Release notes

Lulo OS release notes come from the archive, signed. A release's notes file
(`packaging/release-notes/<version>.txt`, see
[Release process](release-process.md#release-notes)) becomes
`rmac-session`'s `Lulo-Release-Notes` control field, which the publication
copies into the `Packages` index that the clearsigned `InRelease` binds by
SHA-256 and SHA-512. APT moves an index into `/var/lib/apt/lists` only after
verifying it against that signature. System Settings
(`rmac_updates::lulo_release_notes`) reads only
`*_dists_resolute_main_binary-<arch>_Packages` files whose sibling
`*_dists_resolute_InRelease` names `Origin: rmac` and `Label: rmac`, finds
`rmac-session` at exactly the offered version, and decodes the field
(bounded, no control characters): the first paragraph is the lead, `# `
lines are bold section headings. Nothing is downloaded and no command runs.
Without a notes field (a component-only update, or a release published
before notes existed), the card lists the packages and versions instead.

Ubuntu's packages have no notes here: PackageKit's APT backend fetches
Ubuntu changelogs over the network in `GetUpdateDetail`, and the More Info
sheet shows each package's summary instead.

## Automatic Updates

The ⓘ beside Automatic Updates opens the Mac's "Automatically" sheet with
macOS 26.2's three switches (the Mac no longer has "Check for updates" or an
App Store switch there; checking always happens):

| Switch | Key in `software-update.conf` | Effect on the daily run |
|---|---|---|
| Download new updates when available | `download-updates` | Download every installable update in the background (`ONLY_TRUSTED \| ONLY_DOWNLOAD`). Off: download and schedule nothing, only notify. |
| Install Lulo OS updates | `install-lulo-os` | Schedule the Lulo OS packages for the next restart. |
| Install system data files and security updates | `install-security` | Schedule Ubuntu updates PackageKit marks security or critical for the next restart. |

The two install switches are greyed out, and have no effect, while
downloading is off, as on the Mac. The row reads On while downloading is on,
Off otherwise. A switch applies and saves the moment it changes; Done only
closes the sheet.

The switches live in `$XDG_CONFIG_HOME/rmac/software-update.conf` (else
`~/.config/…`): `key=true|false` lines, written atomically with mode 0600
(`rmac_updates::AutomaticUpdates`). A missing file, unknown key or bad value
means the default, on, like a new Mac. Nothing watches the file:
`rmac-update-check` reads it on each run.

### The daily run

The rmac-session package ships `rmac-update-check.timer` and
`rmac-update-check.service` as user units; `rmac-session.target` wants the
timer. The timer fires once a day (`OnCalendar=daily`, spread over an hour by
`RandomizedDelaySec=1h`), catches up after the machine was off
(`Persistent=true`), and also fires 15 minutes after the user manager starts
(`OnStartupSec=15min`). The service runs `/usr/libexec/rmac/rmac-update-check`
once and exits, at `Nice=10` and idle I/O priority, with a two-hour start
timeout for large downloads. Nothing stays resident, and nothing else polls.

The program is Python 3 and uses PackageKit's own client library through
GObject introspection (`gir1.2-packagekitglib-1.0`, `python3-gi`). It never
runs a package-manager command and never parses command output. Each run:

1. Reads the switches, creates a non-interactive background PackageKit
   client, refreshes the metadata, and reads `get_updates`. Blocked updates
   are ignored.
2. Splits the updates into Lulo OS packages (by exact name), security
   updates (`SECURITY` or `CRITICAL`, not Lulo OS), and the rest. The
   automatic set is the Lulo OS packages if Install Lulo OS updates is on,
   plus the security updates if Install system data files and security
   updates is on; it is empty while downloading is off.
3. If downloading is on: first downloads everything outside the automatic
   set (a cache for a later Update Now), then prepares the automatic set as
   PackageKit's offline update. That download comes last so that it becomes
   the prepared list, and it includes anything an earlier Update Now
   scheduled that is still offered, so a scheduled update is never
   replaced. It then triggers the update for reboot. If the prepared list
   already holds the automatic set and a reboot is pending, nothing is
   downloaded or triggered again; if an update is scheduled and nothing is
   automatic, the cache download is skipped so it cannot replace it.
4. Writes `$XDG_STATE_HOME/rmac/software-update-status` (`version=1`,
   `updates=<items>`, `restart-required=0|1`), the count the menu bar shows
   (below).
5. Sends at most one notification (app name "Software Update", icon
   `software-update-available`): "Lulo OS updates are ready — they will be
   installed the next time you restart." (or "Security updates are ready…")
   when it scheduled something, and "N other updates are available — open
   System Settings to review and install them." for the rest.

**Errors.** If PackageKit or the system bus is unavailable, the metadata
refresh fails (for example expired or unsigned repository metadata), or a
download or the trigger fails, the program writes one bounded line to the
user journal, such as `rmac-update-check: refresh failed: gpg-failure`, and
exits non-zero, so `systemctl --user status rmac-update-check` shows the
failed run. The line carries only the PackageKit error class, never the
GError message, which can contain URLs, paths or package names. A failed
preparation still sends the fallback notification, counting the updates for
review. A missing notification service is logged and is not a failure. A
missing PackageKit client library exits with status 2.

**Opting out.** Turning Download new updates off stops all background
downloads and scheduling; the daily check and its notification remain, as on
the Mac. `systemctl --user disable --now rmac-update-check.timer` stops the
check too.

**Phasing.** PackageKit's APT backend reports as updates what libapt marks
for upgrade in its dependency cache; neither it nor this program reads
`Phased-Update-Percentage`. It is **unverified** whether PackageKit's APT
backend holds back phased Ubuntu updates the way `apt upgrade` does. Lulo
OS's own repository phases its releases too (release-process.md), so this
also decides how quickly a new Lulo OS reaches each machine.

## The menu bar count

Each Settings snapshot and each daily run writes the status file above. The
count is the number of cards the pane shows: one for a Lulo OS update and one
for any other updates, the way the Mac's single "Other Updates" row counts
once. The menu bar reads the file when the Lulo menu opens (no watch, no
poll) and draws the Mac's capsule after the item: "System Settings…  1
update" (BAR-06).

## Authorization

Measured on the reference PC (Ubuntu 26.04, polkit 127, PackageKit 1.3.4)
on 2026-09-25:

- **No polkit authentication agent runs in a Lulo OS session.** Only
  `polkitd` is running; nothing registers
  `org.freedesktop.PolicyKit1.AuthenticationAgent`. Any action that returns
  `auth_admin` or `auth_admin_keep` is therefore refused, not prompted.
- **PackageKit's implicit policy** (`pkaction --verbose`):
  `org.freedesktop.packagekit.system-update` is `auth_admin_keep` for the
  active session, `trigger-offline-update` is `yes` for active and inactive
  sessions, and `package-install` is `auth_admin_keep`.
- **Ubuntu's rule lets administrators update without a password.**
  `/usr/share/polkit-1/rules.d/org.freedesktop.packagekit.rules` returns
  `YES` for `system-update`, `trigger-offline-update` and
  `trigger-offline-upgrade` when the subject is active, local, and in the
  `sudo` (or `wheel`) group. `pkcheck --process <niri's PID>` for the owner
  (in `sudo`) returns `yes` for both.
- **User services count as the graphical session.** polkit attributes a
  process in the user's systemd manager (the Dock's `app.slice` unit, niri's
  `session.slice` unit, and a `systemd-run --user` unit standing in for
  `rmac-update-check.service`) to the user's display session: `pkcheck`
  returned `yes` for all three while the Lulo session was active, and
  `auth_admin` during the moment the session was being restarted.
- **Download-only needs nothing.** `pk_transaction_obtain_authorization()`
  returns early for `ONLY_DOWNLOAD` and `SIMULATE`, and metadata refresh
  (`system-sources-refresh`) is `yes`.

So the pane's path, a download-only transaction plus the offline trigger,
works without any password for every active local user, administrator or
not, and so does the timer while the user is logged in. That is why
Software Update installs at restart rather than online: it needs no agent,
and nothing is replaced under the running session. An online install (the
old path) would work silently for administrators but would be refused for a
standard user.

A standard user can therefore install already-published, trusted updates at
the next restart without an administrator. That is PackageKit's and
Ubuntu's own policy (`trigger-offline-update` is `yes`); Lulo OS adds no
polkit rule. The packages still pass `ONLY_TRUSTED`, so only packages the
configured archives signed can be installed.

### An authentication agent: the next step (not built)

Other privileged actions still have no password prompt in Lulo OS:
`package-install` (installing a package file), CUPS administration outside
`lpadmin`, `pkexec`, udisks actions for non-administrators, and more
(SWU-07). The design for a minimal Mac-style agent, `rmac-polkit-agent`:

- **Registration.** A session user service
  (`rmac-polkit-agent.service`, `PartOf=rmac-session.target`) calls
  `org.freedesktop.PolicyKit1.Authority.RegisterAuthenticationAgentWithOptions`
  for the `unix-session` subject of the graphical session, the locale, and
  an object path, then exports `org.freedesktop.PolicyKit1.AuthenticationAgent`
  (`BeginAuthentication(action_id, message, icon_name, details, cookie,
  identities)`, `CancelAuthentication(cookie)`) with zbus. Pure D-Bus, no
  GObject: `libpolkit-agent-1` would need GObject subclassing through FFI
  for `PolkitAgentListener`, which is most of the complexity.
- **Caller check.** As with the Wi-Fi and Bluetooth agents (SR-22), resolve
  `org.freedesktop.PolicyKit1`'s unique name at registration and reject
  every call from any other sender.
- **Authentication.** polkit 127 on Ubuntu runs `polkit-agent-helper-1`
  as a socket-activated root service (`polkit-agent-helper.socket`,
  `/run/polkit/agent-helper.socket`; the helper binary is no longer setuid).
  The agent connects to the socket, writes the identity's user name and the
  cookie, and then speaks the helper's line protocol:
  `PAM_PROMPT_ECHO_OFF <prompt>` → the password line, `PAM_TEXT_INFO` /
  `PAM_ERROR_MSG` → shown, `SUCCESS` / `FAILURE` → done. The helper itself
  answers polkitd (`AuthenticationAgentResponse2`); the agent never does, so
  it never sees whether a cookie is valid. The exact socket handshake must
  be checked against polkit 127's `polkitagentsession.c` before building it.
- **The sheet.** A layer-shell overlay drawn like the Mac's authorization
  dialog: the lock glyph, "<App> wants to make changes.", "Enter your
  password to allow this.", the user name (the first `unix-user` identity
  in the `sudo` group; a picker when there are several), a password field,
  Cancel and OK. It names the action from polkit's `message`, never from
  the caller's `details` alone.
- **Password handling**, following the lock provider
  (`crates/rmac-lock-provider-linux`, security review SR-08/SR-09 and
  "secret-lifetime-and-zeroization-reviewed"): a fixed-capacity
  `SecretInput` zeroized on every edit and on drop, a full field ignores
  input instead of failing, one copy written straight from that buffer to
  the socket followed by `explicit_bzero`, no `String`/`CString` of the
  secret, `LimitCORE=0` and `MemoryDenyWriteExecute=yes` on the unit,
  `panic = "abort"`, no logging of prompts' answers, and the field cleared
  on cancel, failure, timeout and `CancelAuthentication`.
- **Abuse limits.** One sheet at a time (queue others), a visible
  requesting-app name from the subject's PID via `/proc` (bounded, never
  trusted for the decision), cancel after 5 minutes, and three failures
  end the request as polkit's own agents do.

It needs a security review of its own before it ships.

## What the reference PC still has to prove

`scripts/test_update_check.py` runs `rmac-update-check` against a fake `gi`
package that records every PackageKit and D-Bus call, including the
Automatic Updates switches, the download order, and the status file.
`rmac-updates` tests cover the Lulo OS grouping, sizes, versions, selection
resolution, readiness, release-notes decoding and origin filtering, and the
two files. None of this has run against a real Lulo OS release yet. It is not
proven until the reference PC completes a real cycle:

- a Lulo OS release with a notes file is published to the signed
  repository; Software Update shows "Lulo OS <version>" with its size and
  notes, and the menu bar shows "1 update";
- Update Now downloads with progress, without any polkit prompt, and the
  button becomes Restart Now; `Offline.GetPrepared` lists exactly the
  selected packages plus anything already scheduled;
- Restart Now asks each app to quit, an app with unsaved work cancels it,
  and otherwise `system-update.target` installs the packages and
  `dpkg-query` shows the new versions after the second restart;
- the timer with each combination of the three switches downloads,
  schedules and notifies as specified, never replacing an update scheduled
  from the pane; a second run with nothing new downloads nothing;
- a plan with a removal stops at the confirmation;
- expired or unsigned repository metadata, a network loss during download
  and a refused trigger each leave one journal line and never schedule a
  partial update; and
- keyboard-only operation of both sheets, Orca names, 100–200% scaling,
  and idle wakeups with the pane open.
