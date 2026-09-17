# Ubuntu session packaging

The repository now owns a package-grade **integration payload** for an rmac
Wayland session. It is intentionally not a live-root installer and it is not
yet a release package. The payload is the bounded input for the future Ubuntu
native package and VM install/upgrade/uninstall gates.

## Session boundary

`rmac.desktop` is an additional GDM Wayland session. It does not replace,
rename, select, or remove the distribution's Ubuntu/GNOME session:

1. GDM launches `/usr/libexec/rmac/rmac-wayland-session`.
2. The wrapper selects `XDG_CURRENT_DESKTOP=rmac:niri`, provisions a private
   `$XDG_CONFIG_HOME/rmac/niri/config.kdl` entry point when it is missing,
   exports that exact path as `NIRI_CONFIG`, and executes the distribution's
   `/usr/bin/niri-session` lifecycle as a child. The ordinary user niri config
   is neither loaded nor changed by the rmac login.
3. It waits at most 30 seconds for `niri.service`, `NIRI_SOCKET`, Wayland
   session type, the exact desktop identities, and the selected rmac config in
   the user manager.
4. Only then does it provision missing user-owned lock defaults and start
   `rmac-session.target`.
5. Logout, startup failure, or a termination signal stops only the two rmac
   targets and returns the exact session result to GDM.

This preserves upstream niri's systemd graphical-session and portal setup.
The package does not copy or modify niri's launcher, service, shutdown target,
or portal defaults. The contract follows the niri 26.04
[packaging](https://niri-wm.github.io/niri/Packaging-niri.html),
[integration](https://niri-wm.github.io/niri/Integrating-niri.html), and
[systemd session](https://niri-wm.github.io/niri/Example-systemd-Setup.html)
guidance. `DesktopNames=rmac;niri` follows the
[Desktop Entry specification](https://specifications.freedesktop.org/desktop-entry/latest-single/).

## Safe mode and recovery

Before niri starts, the wrapper checks the existing
`$XDG_STATE_HOME/rmac/session/safe-mode.json` marker. When present it uses
`XDG_CURRENT_DESKTOP=niri`, so rmac portal selection and optional shell
surfaces are not activated, then starts `rmac-safe-mode.target`. The secure
lock coordinator and idle policy remain supervised as documented in
`docs/session-supervisor.md`.

Safe mode never changes the GDM session inventory. If rmac or niri cannot
start, sign out or return to GDM and select the stock Ubuntu/GNOME Wayland
session. The installed-host verifier refuses to pass unless it finds a
separate session whose `DesktopNames` contains the exact `GNOME` token.

The package carries immutable swaylock and lock-policy defaults under
`/usr/share/rmac/session`. First start copies only missing files into
`$XDG_CONFIG_HOME/rmac` with private permissions. An existing file—including
an older user choice—is never overwritten by session startup.

The user-owned niri entry point contains only an include of the immutable
`/usr/share/rmac/niri/shell.kdl`. Package upgrades can therefore update rmac
window rules and desktop bindings without rewriting user data. System Settings
may place its isolated display include first and input include last in the
entry point. The initial shell policy suppresses niri's default Waybar startup,
provides the rmac session shortcuts and hardware volume/brightness keys, and
opens Spotlight, Apps, Quick Settings, and Notification Center as sized
floating surfaces rather than scrolling-layout application windows. Persistent
safe mode removes `NIRI_CONFIG` before niri starts and remains independent of
this shell policy.

## Staging a native package payload

The assembler accepts only an absolute, empty `DESTDIR`. It refuses `/`, so it
cannot be used as a privileged ad-hoc installer:

```sh
destdir="$(mktemp -d)"
python3 scripts/linux/stage-session-package.py --destdir "$destdir"
python3 scripts/linux/verify-session-package.py --root "$destdir"
```

It builds in a sibling temporary directory and publishes the complete tree
with one rename. The payload contains:

- the additional GDM session entry and bounded session wrapper;
- package-path systemd user units rendered from the reviewed development
  units;
- rmac notification and Focus D-Bus activation;
- rmac portal metadata and desktop-specific backend selection;
- immutable lock defaults and the original MIT license;
- an upgradable niri shell policy, private user entry-point seed, and
  shell-free shortcut fragment;
- a deterministic manifest of every package-owned file, mode, and SHA-256.

The payload does **not** include binaries. The future native package set must
install the manifest's `/usr/libexec/rmac` executables from the matching rmac
build and declare the exact Ubuntu runtime dependencies. Keeping integration
separate lets the package manager reject a partial version combination.

The dedicated rmac session includes its package-owned shortcut fragment so it
is usable immediately and does not depend on edits to another niri session.
Outside the rmac session, include the fragment only when the runtime reports
that the GlobalShortcuts portal is unavailable. A Rust test proves the shipped
fragment remains byte-for-byte derived from the typed shortcut domain.

## Installed-host preflight

After a real package manager installs the complete package set on the Ubuntu
reference PC, run this read-only gate:

```sh
python3 scripts/linux/verify-session-package.py \
  --root / \
  --installed-host
```

The gate rechecks the manifest hashes and modes, required niri/system/rmac
executables, the exact rmac session contract, package-path units, and the
independent GNOME Wayland recovery session. It does not read home directories,
user data, session logs, hostnames, or hardware identity.

## Upgrade, rollback, and uninstall ownership

The native package manager—not this staging tool—must own `/usr` mutation,
dependency ordering, interruption recovery, and rollback. Every integration
path is under an rmac-specific name. There are no package claims under:

- `/etc/gdm`, `/var/lib/AccountsService`, or GNOME/Ubuntu session paths;
- `$HOME`, XDG config, data, cache, state, or runtime directories;
- niri's package-owned launcher, units, configuration, or portal file.

Uninstall removes only package-owned immutable files and leaves user documents,
application data, preferences, and the safe-mode marker intact. A later
explicit purge/export flow may offer reviewed user-data removal, but package
uninstall must never infer consent to delete it.

The binary packages deliberately contain no `preinst`, `postinst`, `prerm`,
`postrm`, trigger, or conffile hooks. Their exact control inventory is only
`DEBIAN/control`, so install, upgrade, rollback, remove, purge, and interrupted
configuration remain ordinary dpkg/APT transactions with no hidden mutation of
the live session or home directories.

The destructive lifecycle runner is restricted to an Ubuntu 26.04 VM with the
exact `/run/rmac-disposable-vm` marker. It requires an older baseline package
set and a newer candidate package set:

```sh
printf 'rmac-package-lifecycle-v1\n' | \
  sudo tee /run/rmac-disposable-vm >/dev/null
sudo python3 scripts/linux/run-package-lifecycle.py \
  --baseline /absolute/path/to/baseline \
  --candidate /absolute/path/to/candidate \
  --evidence /absolute/empty/lifecycle-evidence
```

The runner verifies both package sets, installs the baseline, upgrades to the
candidate, intentionally stops a rollback after `dpkg --unpack`, configures the
matching rollback set, removes and purges it, reinstalls the candidate, and
purges again. Every transition rechecks the 15 GiB floor, five synthetic
user-data classes, exact installed versions/binary hashes, package-owned path
removal, and the independent GNOME Wayland session. It emits only a bounded
version/step/pass report and deletes its fixed synthetic test user on success.

H4–H6 remain incomplete until this automation and the GDM login/logout,
crash-loop recovery, and stock-GNOME recovery journeys produce reviewed
evidence on clean Ubuntu 26.04 VMs and the reference PC.

The exact five-login operator procedure and its privacy-safe evidence boundary
are in [GDM and recovery journey evidence](session-journey-evidence.md). The
session wrapper explicitly stops the shared supervisor, idle locker, and lock
coordinator after niri exits, in addition to both rmac targets, so none of those
services leak into the following GNOME recovery login.
