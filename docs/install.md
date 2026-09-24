# Install Lulo OS

> Lulo OS is not released for general installation yet.

The repository contains the applications, session integration payload, and
native package assembly contract, but the signed APT repository and clean-PC
install gates are still pending. Do not install Lulo OS on a primary machine or
replace the stock Ubuntu desktop. Use a separate test account on a disposable
Ubuntu 26.04 machine and keep the Ubuntu/GNOME Wayland session available.

## Before you start

The current target is Ubuntu 26.04 with a Vulkan-capable GPU and niri. Check
the provisional [hardware support](hardware-support.md) first. Keep at least
25 GiB free before a release build and stop if the data volume approaches
15 GiB free.

Clone the repository on the disposable Ubuntu machine, then validate and apply
the guarded reference-PC preparation. It installs the exact development and
evidence dependencies, the pinned Rust toolchain, cargo-deny, Debian packaging,
Flatpak, and freedesktop validator tools without building Lulo OS or removing
GNOME:

```sh
bash scripts/linux/prepare-reference-pc.sh --check
bash scripts/linux/prepare-reference-pc.sh --execute
```

Reboot if Ubuntu requests it, log into the untouched GNOME Wayland session,
and run the read-only preflight from
[Linux reference PC bring-up](linux-reference-bringup.md) before compiling.

## Install from the APT repository

> This is the eventual normal path once the release pipeline in
> [Release process](release-process.md) has real signing keys. Today
> `install.sh` refuses to run: its pinned archive fingerprint is still an
> unfilled placeholder awaiting the key (see "Decisions needed" in
> [Update trust](update-trust.md)). Nothing below works yet, but this is
> the exact flow that will replace source builds for everyday testers.

One line, on a disposable Ubuntu 26.04 amd64 or arm64 machine:

```sh
curl -fsSL https://millionrust.github.io/lulo/install.sh | sh
```

That script (`scripts/linux/install.sh` in this repository) does exactly the
following, and nothing else:

1. Refuses to run as root; it calls `sudo` itself only for the steps that
   need it.
2. Checks that the machine reports Ubuntu 26.04 on amd64 or arm64.
3. Downloads `rmac-archive-keyring-latest.deb` over HTTPS, unpacks it
   without installing it, and verifies with `gpg` that its keyring holds
   exactly one primary key, the archive fingerprint pinned in the script --
   HTTPS transport security is never treated as package authentication
   (see [Update trust](update-trust.md)).
4. Copies only that verified keyring file to
   `/usr/share/keyrings/rmac-archive-keyring.gpg` (the unsigned bootstrap
   `.deb` is never given to `dpkg`, so none of its maintainer scripts run)
   and writes `/etc/apt/sources.list.d/rmac.sources` and
   `/etc/apt/preferences.d/rmac.pref` (the rendered
   `packaging/apt/rmac.sources.in` / `packaging/apt/rmac.pref`, pinned to
   `rmac-apps`, `rmac-session`, and the keyring package only).
5. Runs `apt update && apt install rmac-archive-keyring rmac-session`: the
   keyring package now comes from the signed repository and takes over the
   keyring file, and `rmac-session` pulls in `rmac-apps`.

It never touches the GNOME session: no session default changes, no GDM
restart. The same steps written out by hand:

```sh
# 1. Download and verify the keyring, then install it.
curl -fsSL -o rmac-archive-keyring.deb \
  https://millionrust.github.io/lulo/rmac-archive-keyring-latest.deb
dpkg-deb --fsys-tarfile rmac-archive-keyring.deb \
  | tar -xO ./usr/share/keyrings/rmac-archive-keyring.gpg > rmac-archive-keyring.gpg
gpg --show-keys --with-colons rmac-archive-keyring.gpg \
  | grep -E '^(pub|fpr)'   # exactly one pub line; compare its fpr by hand
                           # against the README/Release notes
sudo install -o root -g root -m 0644 rmac-archive-keyring.gpg \
  /usr/share/keyrings/rmac-archive-keyring.gpg

# 2. Add the repository and its pin.
sudo tee /etc/apt/sources.list.d/rmac.sources >/dev/null <<'EOF'
Types: deb deb-src
URIs: https://millionrust.github.io/lulo/
Suites: resolute
Components: main
Architectures: amd64 arm64
Signed-By: /usr/share/keyrings/rmac-archive-keyring.gpg
Check-Valid-Until: yes
EOF
sudo tee /etc/apt/preferences.d/rmac.pref >/dev/null <<'EOF'
Package: rmac-apps rmac-archive-keyring rmac-session
Pin: release o=rmac,n=resolute,c=main
Pin-Priority: 500

Package: *
Pin: release o=rmac
Pin-Priority: -1
EOF

# 3. Install.
sudo apt update
sudo apt install rmac-archive-keyring rmac-session
```

Log out and choose Lulo OS on the login screen. Keep Ubuntu/GNOME as the
recovery session.

To remove rmac later:

```sh
curl -fsSL https://millionrust.github.io/lulo/uninstall.sh | sh
```

or by hand: `sudo apt purge rmac-session rmac-apps rmac-archive-keyring`
then remove `/etc/apt/sources.list.d/rmac.sources` and
`/etc/apt/preferences.d/rmac.pref`.

### Keeping rmac up to date

Once installed, `rmac-update-check.timer` asks PackageKit for updates once a
day and shows a notification when any exist; it never installs, downloads,
or removes anything itself. Reviewing and applying updates happens in
System Settings (see [Software Update](software-update.md)), which drives
PackageKit directly. PackageKit installs on the next restart ("offline
updates"), so a running session is never replaced mid-session. An optional
"install updates automatically" setting for `origin=rmac` security updates
is still on the todo list and not implemented yet.

## Install from a GitHub Release (the Beta path)

Until the signed APT repository above exists, a tagged release still
publishes `rmac-apps` and `rmac-session` `.deb` files, Lulo OS's own
`niri` and `xwayland-satellite` `.deb` files with their source packages, a
`SHA256SUMS`, SBOMs, and a build-provenance attestation to its GitHub
Release page (see
[Release process](release-process.md) "What exists today"). `install.sh`
and `uninstall.sh` can use that release directly, on a disposable Ubuntu
26.04 amd64 or arm64 machine, without ever touching APT repository
configuration:

```sh
curl -fsSL -O https://raw.githubusercontent.com/millionrust/lulo/main/scripts/linux/install.sh
sh install.sh --from-release vX.Y.Z
```

This downloads `rmac-apps_*_<arch>.deb`, `rmac-session_*_<arch>.deb`,
`niri_*_<arch>.deb`, `xwayland-satellite_*_<arch>.deb`, and `SHA256SUMS`
from that release tag, verifies the package files against `SHA256SUMS`,
verifies `gh attestation verify`'s build-provenance check for each (skipped
with a note if `gh` is not installed -- this is a defense in depth on top of
the checksum check, not a replacement for it), then runs `sudo apt-get
install` on the local files so their remaining `Depends` still resolve from
the machine's ordinary Ubuntu archive. If a newer `niri` or
`xwayland-satellite` is already installed (for example the danklinux PPA's
`26.04ppa3`), it is kept -- it already satisfies `rmac-session` -- and the
script prints the `apt-get install --allow-downgrades` command that switches
to the Lulo OS build. If you already
downloaded or copied the `.deb` files and `SHA256SUMS` yourself (for
example with `gh release download`, or from `build-native-inputs.sh` +
`check-native-reproducibility.sh`'s output), skip the download instead:

```sh
sh install.sh --from-dir /absolute/path/to/native-package-set
```

Reverse either one the same way as the APT-repository path:

```sh
curl -fsSL -O https://raw.githubusercontent.com/millionrust/lulo/main/scripts/linux/uninstall.sh
sh uninstall.sh
```

`uninstall.sh` does not need to know which path you installed from: it
purges whichever of `rmac-session`, `rmac-apps`, and `rmac-archive-keyring`
dpkg actually knows about (a `--from-release`/`--from-dir` install never
registers `rmac-archive-keyring` at all, since it never adds the rmac APT
repository) and removes the repository configuration files if present.

> **niri and xwayland-satellite:** neither is in the Ubuntu archive for
> `resolute` (26.04), so Lulo OS ships its own builds of the exact releases
> rmac is tested against (niri 26.04, xwayland-satellite 0.8.2) in every
> GitHub Release, and `install.sh` installs them together with rmac.
> `rmac-session` depends on `niri (>= 26.04)` and
> `xwayland-satellite (>= 0.8.2)`, so a PPA or a future official Ubuntu
> package satisfies it equally; see [Release process](release-process.md)
> "Third-party packages: niri and xwayland-satellite". A `--from-dir` set
> without them only works if apt can find them elsewhere. `uninstall.sh`
> leaves them installed (Ubuntu/GNOME never depended on them); remove them
> with `sudo apt-get purge niri xwayland-satellite` if nothing else needs
> them.

For a full manual pass on a disposable VM (install, first login, upgrade,
uninstall, and confirming Ubuntu/GNOME survives), follow
[Beta clean-VM checklist](beta-clean-vm-checklist.md), or run
`scripts/linux/run-package-lifecycle.py --print-checklist` to print its
path.

## Build one application

Build only what you need while developing:

```sh
cargo build --locked -p rmac-text-editor
cargo run --locked -p rmac-text-editor
```

Other application packages are listed in the [README](../README.md#applications).
Use the normal `target` directory so dependencies are reused. A release build
is appropriate only for performance or package evidence.

## Development session

The home-directory development installer is documented in
[Session supervisor](session-supervisor.md). It builds the current session
components and installs user-owned units under XDG locations; it does not
install a system package or alter `/usr`.

Before launching the session, run the read-only Linux preflight:

```sh
scripts/linux/run-reference-gates.sh --session niri --preflight-only
```

Enable the generated niri shortcut include only when the shortcut status says
`fallback-required`. The GlobalShortcuts portal is otherwise the sole shortcut
owner.

## Native packages

The native package assembler is for clean build/VM evidence. It must never be
pointed at `/` or used as a root installer. The resulting packages are not a
public release until signing, APT trust, clean install/upgrade/rollback, and
hardware gates pass. The native flow builds exactly the 18 reviewed executables
once and performs two independently verified byte-identical package assemblies:

```sh
mkdir -p "${PWD}/target"
bash scripts/linux/build-native-inputs.sh \
  --output "${PWD}/target/native-package-inputs"
bash scripts/linux/check-native-reproducibility.sh \
  --binary-dir "${PWD}/target/native-package-inputs" \
  --output "${PWD}/target/native-$(dpkg --print-architecture)-reproducibility" \
  --architecture "$(dpkg --print-architecture)" \
  --source-date-epoch "$(git log -1 --format=%ct)"
```

Run that flow natively on amd64 and arm64; see
[Native packaging](native-packaging.md) for verification, installation, and
evidence requirements.

On the dedicated reference PC, install one of the two byte-identical results
through the guarded APT handoff. Run the read-only check first from the
untouched GNOME Wayland session:

```sh
candidate_dir="${PWD}/target/native-$(dpkg --print-architecture)-reproducibility/run-a"
bash scripts/linux/install-native-candidate.sh \
  --check --directory "${candidate_dir}"
```

The check verifies Ubuntu 26.04, the native architecture, 25 GiB headroom, the
exact package inventory, and a separate stock GNOME Wayland recovery entry.
If it passes, authorize exactly one installation attempt and repeat it in
execute mode:

```sh
printf 'rmac-reference-pc-install-v1\n' | \
  sudo tee /run/rmac-reference-pc >/dev/null
bash scripts/linux/install-native-candidate.sh \
  --execute --directory "${candidate_dir}"
```

The installer consumes the one-attempt marker, asks APT to install exactly
`rmac-apps` and `rmac-session` with package removals forbidden, verifies both
installed versions and the complete Lulo OS/GNOME session boundary, then reloads
only the current user's systemd unit inventory. It never selects the new
session, restarts GDM, edits niri configuration, or reads user data. Sign out
normally and choose **Lulo OS** in GDM; keep **Ubuntu** available for recovery.

## Flatpak candidate

Text Editor is the sole current sandbox candidate. Prepare its runtime and
source cache while online, then perform the provenance-bound no-download build:

```sh
bash scripts/linux/build-flatpak-candidate.sh --prepare-online
bash scripts/linux/build-flatpak-candidate.sh --build-offline
```

Installation is deliberately separate because it begins the portal, permission,
recovery, scaling, input, and accessibility review. See
[Flatpak packaging](flatpak-packaging.md).

## First login

At GDM, choose the separate Lulo OS session. Do not remove Ubuntu/GNOME. Confirm
that the desktop, top bar, Dock, shortcuts, Settings, sound, network, lock, and
logout work before placing any test data in the account. If startup fails,
return to GDM and choose Ubuntu/GNOME, then follow
[Troubleshooting and recovery](troubleshooting.md).
