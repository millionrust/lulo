# Session supervisor

The rmac shell uses systemd user units as separate crash domains. A failed Dock
or top bar can restart without terminating the compositor, other shell
surfaces, or the user's applications. `rmac-session` supplies the typed health
model, safe-mode state, command-line supervisor, unit assets, and development
installer.

## Startup and environment

Run `scripts/linux/install-session-units.sh` once from the repository. It
builds the release supervisor, installs it under
`~/.local/libexec/rmac/`, installs the unit files under the XDG systemd user
directory, and installs `~/.local/bin/rmac-session-start`.
It also builds the notification and Focus services, installs the notification
portal descriptor and desktop-specific backend selection, and installs D-Bus
activation files for both authorities.

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

Top bar, Dock, launcher, notification center, Focus authority, wallpaper, and
the global shortcut broker each have their own service. They use
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

Automatic reference evidence collects only unit state properties and typed
health, not journal message bodies, because application logs may contain
private paths or content. Review journal excerpts before sharing them.

## Secure locker exception

`rmac-lock.service` is intentionally not a normal restart-budgeted shell
component. It starts only on a lock request, becomes ready only after swaylock's
compositor-confirmed readiness handshake, and restarts without a start-limit on
failure because niri remains fail-closed when the locker disappears. Sending it
through the ordinary safe-mode failure budget could strand an already locked
session without an authentication provider. See `docs/secure-lock.md` and ADR
0004 for the security and recovery boundary.

The companion `rmac-lock-coordinator.service` is required by both normal and
safe-mode targets. It restarts without a start limit, becomes ready only after
its logind signal subscriptions and sleep inhibitor exist, and keeps lock and
lock-before-sleep behavior independent of optional shell surfaces.

Both targets also want `rmac-idle-lock.service`. It reads only the validated
timeout policy and supervises swayidle with a fixed lock command and unbounded
crash restart. Disabling the timeout keeps the policy service alive without
claiming an idle lock; it does not weaken manual or pre-sleep locking.

The coordinator owns the session-bus `org.rmac.LockScreen1` settings authority
before reporting ready. It is intentionally not D-Bus activated: the normal or
safe session target must establish the logind subscriptions and inhibitor at
the same time as the policy API. Timeout writes atomically restart only the idle
service, never the coordinator or active locker.

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
