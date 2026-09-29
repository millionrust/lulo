# Live Shut Down menu-row probe

`scripts/linux/probe_live_shutdown_cancel.py` retains its old filename for
existing invocations. It now opens the installed top bar's system menu over
AT-SPI, verifies that the `Shut Down…` row exists, and closes the menu. It
never activates that row or opens the confirmation.

The old probe opened the confirmation and then clicked Cancel. That is not a
safe automated live test: the confirmation starts a 60-second countdown whose
default action shuts down the computer. If AT-SPI or the probe fails after
opening it, cleanup cannot guarantee Cancel. Confirmation and completion
behavior belong in the private nested suites with a fake `systemctl`.

The probe may activate only the top-bar menu toggle. It refuses to interact
unless it runs as the expected user (`jacob`) and discovers exactly one active,
local, seat0 Wayland session for that user, with `wayland-1` and the matching
user bus and Wayland socket. It also leaves an already open menu or
confirmation untouched.

On the reference laptop, run:

```sh
ssh -o BatchMode=yes jacob@192.168.18.52 'python3 -' \
  < scripts/linux/probe_live_shutdown_cancel.py
```

A pass establishes only that AT-SPI can open the system menu and find its Shut
Down row. It does not test the confirmation's pointer behavior or actual
poweroff. The installed pyatspi bridge may emit cache/signature warnings on
stderr; use the exit status and state lines as the result.
