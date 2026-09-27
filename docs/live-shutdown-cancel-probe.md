# Live Shut Down confirmation probe

`scripts/linux/probe_live_shutdown_cancel.py` observes the installed top bar on
its active local Wayland session using AT-SPI. It opens the system menu,
activates the `Shut Down…` row to reveal the confirmation, reports the visible
`Cancel` and `Shut Down` controls, then activates **Cancel** and verifies that
the confirmation closes.

The probe uses AT-SPI `click` actions only. Its only permitted activations are
the top-bar menu button, the `Shut Down…` menu row, and the confirmation's
`Cancel` button. It never activates the `Shut Down` confirmation button and
contains no power, restart, suspend, logout, pointer injection, keyboard
injection, or service mutation call. Since opening this confirmation starts
the product's 60-second countdown, it cancels immediately and retries Cancel
in cleanup if an error occurs after opening the dialog.

It refuses to interact unless it runs as the fixed expected user (`jacob`) and discovers exactly one active, local, seat0 Wayland session for
that user, with `wayland-1`, the matching `/run/user/<uid>` bus, and the
corresponding Wayland socket. The session checks use read-only `loginctl` and
`systemctl --user show-environment` queries.
It also refuses to change an already open system menu or confirmation.

On the reference laptop, run:

```sh
ssh -o BatchMode=yes jacob@192.168.18.52 'python3 -' \
  < scripts/linux/probe_live_shutdown_cancel.py
```

A successful run records that AT-SPI could open and cancel the installed
confirmation. It does not test physical pointer activation or the final power
action; the probe deliberately cannot perform either.
The installed pyatspi bridge may emit cache/signature warnings on stderr even
when the probe's state checks pass; use the exit status and state lines as the
result.
