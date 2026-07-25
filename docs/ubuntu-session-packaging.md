# Ubuntu session packaging

The repository now owns a package-grade **integration payload** for an rmac
Wayland session. It is intentionally not a live-root installer and it is not
yet a release package. The payload is the bounded input for the future Ubuntu
native package and VM install/upgrade/uninstall gates.

## Session boundary

`rmac.desktop` is an additional GDM Wayland session. It does not replace,
rename, select, or remove the distribution's Ubuntu/GNOME session:

1. GDM launches `/usr/libexec/rmac/rmac-wayland-session`.
2. The wrapper selects `XDG_CURRENT_DESKTOP=rmac:niri` and executes the
   distribution's `/usr/bin/niri-session` lifecycle as a child.
3. It waits at most 30 seconds for `niri.service`, `NIRI_SOCKET`, Wayland
   session type, and the exact desktop identities in the user manager.
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
- an optional, shell-free niri shortcut fallback fragment;
- a deterministic manifest of every package-owned file, mode, and SHA-256.

The payload does **not** include binaries. The future native package set must
install the manifest's `/usr/libexec/rmac` executables from the matching rmac
build and declare the exact Ubuntu runtime dependencies. Keeping integration
separate lets the package manager reject a partial version combination.

The shortcut fragment is deliberately not enabled automatically. Include
`/usr/share/rmac/niri/shortcuts-fallback.kdl` only when the runtime reports
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

H4–H6 remain incomplete until a native Debian package, maintainer-script
policy, clean-VM install/upgrade/rollback/uninstall automation, interrupted
transaction evidence, GDM login/logout evidence, crash-loop recovery, and the
stock-GNOME recovery journey all pass on the Ubuntu 26.04 reference PC.
