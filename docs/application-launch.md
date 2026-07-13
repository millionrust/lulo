# Application launch and activation

`rmac-app-launch` is the shared execution route for App Drawer, Spotlight, and
Dock. It preserves one parsed `rmac_apps::LaunchSpec` and never turns desktop
entry text into a shell command.

## Supported niri session

When `$NIRI_SOCKET` is present, a command launch is converted to an exact argv
and sent as niri IPC `Action::Spawn`. Niri launches that child with an XDG
activation token, allowing a compliant Wayland application to request focus
even under strict focus-stealing prevention, across workspaces, or over a
fullscreen window. rmac does not mint a token from an unrelated Wayland
connection: GPUI 0.2.2 does not expose the initiating input serial, seat, and
surface needed for a strong token request.

The compositor-neutral `SpawnCommand` permits at most 256 arguments, 32 KiB per
argument, and 128 KiB total. It rejects an empty program and interior NULs.
Its `Debug` representation contains only argument count and total byte count;
private paths and application parameters do not enter default diagnostics.

Desktop entries with a working directory use the Ubuntu base system's
`/usr/bin/env --chdir <directory> -- <program> ...` form. This remains a direct
`exec`, preserves the compositor-provided activation token, and introduces no
shell expansion. Terminal entries retain exact program/argument boundaries
behind the selected terminal's `-e` argument.

## Fallback and failures

On macOS, a non-niri desktop, a missing niri socket, or a transient niri
transport failure, the shared route uses the existing direct process spawn.
The receipt distinguishes compositor activation from direct fallback and
contains a process ID only when rmac created the process itself.

A compositor rejection is never bypassed with direct spawning. Protocol errors
also remain failures rather than silently changing launch authority. Default
errors expose only an actionable class such as missing executable, permission
denied, compositor rejection, or protocol failure; command arguments are
redacted.

## Remaining evidence

The Ubuntu/niri reference gate must enable niri's strict new-window focus
policy, then launch native Wayland, XWayland, terminal, working-directory, and
desktop-action entries from App Drawer, Spotlight, and Dock. Evidence must show
that token-aware clients focus correctly, rejected launches do not fall back,
and ordinary GNOME/macOS development launches retain their direct behavior.
