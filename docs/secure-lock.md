# Secure session lock

The accepted boundary is documented in
`docs/decisions/0004-secure-lock-boundary.md`. This file records the concrete
integration and its remaining proof rather than treating visual similarity as
security evidence.

## Current path

The global `lock` shortcut bypasses the general shell-event socket and runs the
fixed `systemctl --user start rmac-lock.service` request without a shell. The
unit starts `rmac-locker`, which launches `/usr/bin/swaylock` with the installed
absolute config path and captures its dedicated readiness descriptor on stdout.
Only the exact newline handshake advances the unit to ready. `systemd-notify`
then completes the `Type=notify` start transaction, so a successful shortcut
return means niri has already hidden security-sensitive content.

The locker remains a foreground child in the same systemd control group. A
normal zero exit clears logind's locked hint. A signal or nonzero exit fails the
unit; systemd retries after one second without a start-limit ceiling. niri's
protocol behavior keeps the session locked during that gap. The generated niri
fallback adds `allow-when-locked=true` only to the lock shortcut for manual
recovery.

The development installer requires `/usr/bin/swaylock`, installs the supervisor
and unit, and writes the default rmac swaylock config only when the user has no
existing config. Production packaging must depend on the Ubuntu swaylock build
with PAM support rather than installing a setuid or vendored binary.

The supervisor accepts only an absolute, regular UTF-8 config of at most 64 KiB.
It rejects `daemonize`, `ready-fd`, and nested `config` keys so appearance
customization cannot detach swaylock from its control group or counterfeit the
readiness channel.

## Not complete yet

- logind `Lock()` signal listener;
- delay `sleep` inhibitor and lock-readiness ordering before suspend/hibernate;
- idle timeout and lid-close policy authority;
- notification preview filtering and lock wallpaper authority;
- PAM password, wrong-password, cancellation, and supported MFA evidence;
- output add/remove, scaling, rotation, suspend/resume, and GPU-reset evidence;
- killed-locker automatic recovery and the documented TTY/manual recovery path;
- accessibility and keyboard-layout evidence on the Linux reference PC.

Until those are proven, E5 and E10 remain unchecked and System Settings must not
show controls that imply the missing behavior exists.
