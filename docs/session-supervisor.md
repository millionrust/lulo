# Session supervisor

The rmac shell uses systemd user units as separate crash domains. A failed Dock
or top bar can restart without terminating the compositor, other shell
surfaces, or the user's applications. `rmac-session` supplies the typed health
model, safe-mode state, command-line supervisor, unit assets, and development
installer.

## Startup and environment

Run `scripts/linux/install-session-units.sh` once from the repository. It
builds the release supervisor, launcher, supervised App Drawer, on-demand Quick
Settings and Notification Center panel services, and launcher-routed System
Settings, installs them under
`~/.local/libexec/rmac/`, installs the unit files under the XDG systemd user
directory, and installs `~/.local/bin/rmac-session-start`.
The launcher, App Drawer, Quick Settings, and Notification Center panel
services bind their separate action-scoped runtime sockets before the shortcut
broker starts, so
the first consented activation has an owner and one surface crash cannot
consume another action. The panel service is distinct from the D-Bus
notification authority: closing its window leaves its shortcut endpoint alive,
while restarting it cannot take down notification admission or retained
history.
The installer also
builds the notification and Focus services, installs the notification
portal descriptor and desktop-specific backend selection, and installs D-Bus
activation files for both authorities.

The selected current-upstream GPUI wallpaper, menu bar, and Dock are still
framework-gated candidates, so the ordinary development installer does not
silently mix them into the stable product build. On the Ubuntu reference PC,
install all three into their already supervised unit paths with one guarded
handoff:

```sh
bash scripts/linux/install-upstream-shell-candidate.sh --check
bash scripts/linux/install-upstream-shell-candidate.sh --execute
```

The handoff requires the exact committed GPUI revision, a clean tracked
worktree, Ubuntu 26.04, 25 GiB before building, and the 15 GiB absolute floor.
It atomically installs the three executables and a revision manifest without
altering public packages or GNOME. If the rmac target is already active, it
restarts only those three units. Otherwise the next normal
`rmac-session-start` starts the entire supervised desktop together. A manually
launched preview must be stopped first so two bars, Docks, or wallpapers cannot
claim the same session. The handoff never clears safe mode automatically: when
a previous component failure marker exists, it prints the bounded diagnostics
and recovery commands for the user to review before returning to the normal
target.

That command remains a development installer. Native packaging stages the
separate, immutable integration payload described in
`docs/ubuntu-session-packaging.md`. Its GDM wrapper delegates compositor
lifecycle to the distribution's `niri-session`, waits for real niri readiness,
and starts these same targets with package-path units. It never installs
directly into `/usr` and never alters the stock GNOME recovery session.

The start command must run from niri after the graphical session environment is
available. It imports only `WAYLAND_DISPLAY`, `DISPLAY`, `XAUTHORITY`, desktop
and session identity, `XDG_RUNTIME_DIR`, the D-Bus address, and `NIRI_SOCKET`
into the systemd user manager and D-Bus activation environment. It never
imports the entire process environment, `PATH`, tokens, agent sockets, or
arbitrary secrets. It then starts the normal target unless a persistent
safe-mode marker exists.

For a normal rmac shell start, the command prepends `rmac` to
`XDG_CURRENT_DESKTOP`, imports that exact value, starts the shell services, and
uses `try-restart` on an already-running portal frontend so notification
selection is adopted. It preserves niri as the secondary desktop identity.
Safe mode neither changes the desktop identity nor restarts the portal.

systemd's environment.d generator is also valid for stable user-manager
configuration, but graphical values created by the live niri session still
need to be imported before these services start.

## Crash and restart policy

Top bar, Dock, launcher, App Drawer, Quick Settings, the notification authority,
Notification Center panel, Focus authority, wallpaper, and the global shortcut
broker each have their own service. They use
`Restart=on-failure`, a
one-second restart delay, and at most four starts in a 60-second interval. They
are `PartOf` the normal rmac target; one component is not `RequiredBy` another.
The unit conditions keep future D-phase services inactive—not failed—until
their executables are installed.

Every terminal component failure invokes the failure observer with a fixed,
allowlisted unit identity. Three observed restarts or systemd's
`start-limit-hit` result atomically writes
`$XDG_STATE_HOME/rmac/session/safe-mode.json`, stops the normal target, and
starts `rmac-safe-mode.target`. Safe mode retains only the health supervisor in
this foundation slice, preventing an endless graphical crash loop while
preserving diagnostics. It remains active across session restarts until the
user runs:

```sh
~/.local/libexec/rmac/rmac-session-supervisor clear-safe-mode
```

That command removes the marker, resets only the allowlisted component failure
states, stops the safe target, and restarts the normal target.

For privacy-safe diagnostics, last-known-good shell settings restoration, and
same-user TTY steps, follow [Session recovery](session-recovery.md).

## Health and logs

The supervisor queries stable systemd properties for every component and
atomically publishes `$XDG_RUNTIME_DIR/rmac/session-health.json` every five
seconds. The snapshot records load/active/sub states, result, restart count,
main PID, exit status, observation time, and safe-mode cause. Missing component
binaries therefore remain distinguishable from crashed processes. Run
`rmac-session-supervisor status` for the same JSON on demand.

systemd connects service stdout/stderr to the user journal. Inspect one unit
with:

```sh
journalctl --user -u rmac-dock.service -b --no-pager
```

`rmac-session-supervisor diagnostics` emits the bounded shareable report:
allowlisted unit identity and typed health/restart facts, safe-mode state, and a
content-free settings recovery status. It excludes PIDs, paths, configuration,
environment, and journal bodies because application logs may contain private
content. `status` remains the local operator view and is not the shareable
report. Review any journal excerpts separately before sharing them.

## Secure locker exception

`rmac-lock.service` is intentionally not a normal restart-budgeted shell
component. It starts only on a lock request, becomes ready only after swaylock's
compositor-confirmed readiness handshake, and restarts without a start-limit on
failure because niri remains fail-closed when the locker disappears. Sending it
through the ordinary safe-mode failure budget could strand an already locked
session without an authentication provider. See `docs/secure-lock.md` and ADR
0004 for the security boundary, and `docs/secure-lock-recovery.md` for the
same-user TTY and destructive last-resort procedures.

The companion `rmac-lock-coordinator.service` is required by both normal and
safe-mode targets. It restarts without a start limit, becomes ready only after
its logind signal subscriptions and sleep inhibitor exist, and keeps lock and
lock-before-sleep behavior independent of optional shell surfaces.

Both targets also want `rmac-idle-lock.service`. It reads only the validated
timeout policy and supervises swayidle with fixed lock and capability-gated
suspend commands plus unbounded crash restart. Disabling both timeouts keeps the
policy service alive; it does not weaken manual or pre-sleep locking.

The coordinator owns the session-bus `org.rmac.LockScreen1` settings authority
before reporting ready. It is intentionally not D-Bus activated: the normal or
safe session target must establish the logind subscriptions and inhibitor at
the same time as the policy API. Timeout writes atomically restart only the idle
service, never the coordinator or active locker. Automatic suspend is routed
back through the coordinator so its logind capability check and pre-sleep lock
ordering remain authoritative.

Custom-provider recovery experiments do not alter this installed topology.
Their units live under `crates/rmac-lock-provider-linux/evidence`, have no
`[Install]` section, use an `rmac-lock-*-evidence` namespace, and are copied only
by the explicit evidence installer. The normal installer neither builds the
feature-gated provider nor references those units. The recovery gate runs them
against a nested compositor and retains the installed `rmac-lock.service` as
the production swaylock path. The custom evidence unit also sets a ten-second
systemd watchdog with `SIGKILL`; its provider arms the manager-derived heartbeat
only after compositor-confirmed readiness. The recovery gate proves both a
watchdog replacement for a stopped event loop and an ordinary crash restart
before testing authenticated swaylock fallback.

## Verification boundary

Cross-platform tests parse healthy, inactive, malformed, and mismatched
systemctl output; reject non-rmac unit injection; prove restart-budget
exhaustion persists and clears safe mode; and inspect every component unit for
the bounded restart and independent failure-handler contract. Shell scripts
are syntax-checked on every gate. Real systemd activation, process restart,
safe-mode transition, and journal evidence remain mandatory on the Linux
reference PC.

Upstream contracts:

- <https://www.freedesktop.org/software/systemd/man/latest/systemd.service.html>
- <https://www.freedesktop.org/software/systemd/man/latest/systemd.unit.html>
- <https://www.freedesktop.org/software/systemd/man/latest/systemctl.html>
- <https://www.freedesktop.org/software/systemd/man/latest/environment.d.html>
- <https://www.freedesktop.org/software/systemd/man/latest/journalctl.html>
