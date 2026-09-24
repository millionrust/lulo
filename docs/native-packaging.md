# Native development packages

H2 defines a rootless, binary-only Debian packaging boundary for the trusted
rmac applications and niri session. Packaging never invokes Cargo, guesses a
cross-architecture dependency set, installs into the live root, or mutates user
data.

The package set is intentionally split:

| Package | Executable destination | Executables | Role |
| --- | --- | ---: | --- |
| `rmac-apps` | `/usr/bin` | 7 | Files, Terminal, Notes, Text Editor, System Monitor, Applications, and Settings plus their desktop metadata |
| `rmac-session` | `/usr/libexec/rmac` | 13 | Session supervision, launcher and panels, notification/Focus services, shortcuts, and the accepted swaylock coordination boundary |

`rmac-session` depends on the exact matching `rmac-apps` version. Apps
and Settings appear in both payloads because the ordinary applications launch
from `/usr/bin`, while their supervised session modes use the immutable
`/usr/libexec/rmac` boundary. The currently gated Top Bar, Dock, and Wallpaper
units do not receive invented executables; their existing
`ConditionFileIsExecutable` checks keep them inactive until the real
layer-surface binaries land after the framework decision.

## Build contract

Build on the same architecture as the package:

- Ubuntu 26.04 amd64 produces `amd64`;
- Ubuntu 26.04 arm64 produces `arm64`;
- `dpkg --print-architecture` must exactly match `--architecture`;
- `dpkg`, `dpkg-deb`, and `dpkg-shlibdeps` must be installed (`dpkg-dev`
  provides the last tool);
- the input directory must be absolute and contain exactly the 18 named,
  executable, regular ELF64 files;
- `SOURCE_DATE_EPOCH` must be explicit canonical decimal seconds;
- the output must be an empty absolute directory.

The architecture match is deliberate. `dpkg-shlibdeps` resolves each linked
SONAME through the native installed package symbols/shlibs database, so using
an amd64 database to claim arm64 dependencies would not be truthful. The tool
fails on missing dependency information and does not use
`--ignore-missing-info`.

Prepare the exact binaries with the guarded native builder. It accepts only a
new absolute output path, checks 25 GiB before the single locked release build,
reuses the repository's normal target graph, calculates the staging-copy size
before writing, preserves the 15 GiB floor, and publishes all 18 inputs
atomically:

```bash
binary_dir="${PWD}/target/native-package-inputs"
mkdir -p "$(dirname "${binary_dir}")"
bash scripts/linux/build-native-inputs.sh --output "${binary_dir}"
```

Choose a stable timestamp, such as the source revision timestamp, and build:

```bash
export SOURCE_DATE_EPOCH="$(git log -1 --format=%ct)"
mkdir -p "${PWD}/artifacts"
python3 scripts/linux/build-native-packages.py \
  --binary-dir "${binary_dir}" \
  --output "${PWD}/artifacts/native-amd64" \
  --architecture amd64
```

Use `arm64` and a different empty output directory on the native arm64 builder.
The assembler consumes the prebuilt files and never performs a hidden Rust
build. The input builder is the only command in this flow that invokes Cargo.

## Exact dependencies

Shared-library dependencies are generated from every packaged ELF with
`dpkg-shlibdeps -O`, argument-separated and with `LD_LIBRARY_PATH` removed.
The generated relations are retained separately in the publication manifest.
They are combined with reviewed command/service dependencies:

- `rmac-apps`: BlueZ, the D-Bus user session, the `fonts-inter` and
  `fonts-jetbrains-mono` UI fonts, GLib command tools, NetworkManager,
  PackageKit tools, PipeWire tools, power profiles, UPower, WirePlumber, XDG
  portals, and XDG utilities;
- `rmac-session`: the exact apps package, coreutils, the D-Bus user session,
  the `fonts-inter` and `fonts-jetbrains-mono` UI fonts, an `awk`
  implementation, niri, swayidle, swaylock, systemd, the portal frontend, and
  the GNOME and GTK portal backends;
- GDM is a recommendation rather than a hard dependency so the packages remain
  inspectable on non-GDM development hosts. H4 installation acceptance still
  requires GDM and a separate stock GNOME Wayland recovery entry.

The relation parser rejects unsupported syntax, control characters, shell
syntax, duplicate records, and noncanonical ordering. No command uses a shell.

## Reproducibility and publication

Every payload path is staged through the already verified application/session
assemblers. The native boundary then:

1. validates and fingerprints every ELF input;
2. copies only the exact package inventory;
3. derives shared-library dependencies from the native package database;
4. writes one canonical `DEBIAN/control` file and no maintainer scripts;
5. applies `SOURCE_DATE_EPOCH` to all files and directories;
6. builds root-owned, uniformly compressed xz archives with one compressor
   thread;
7. verifies both archives by raw extraction before atomically publishing them.

`dpkg-deb` documents that `SOURCE_DATE_EPOCH` controls the ar timestamp and
clamps tar entry mtimes, while `--root-owner-group` provides the rootless
ownership boundary. See the official
[`dpkg-deb(1)` documentation](https://manpages.debian.org/bookworm/dpkg/dpkg-deb.1.en.html)
and
[`dpkg-shlibdeps(1)` documentation](https://manpages.debian.org/testing/dpkg-dev/dpkg-shlibdeps.1.en.html).

The output directory contains exactly:

```text
SHA256SUMS
native-packages.json
rmac-apps_<version>_<architecture>.deb
rmac-session_<version>_<architecture>.deb
```

`native-packages.json` binds the architecture, Debian version,
`SOURCE_DATE_EPOCH`, archive hashes/sizes, static and generated dependencies,
and the path/hash/size of every binary. `SHA256SUMS` contains only the two
archives. Verify a transferred set without installing it:

```bash
python3 scripts/linux/verify-native-packages.py \
  --directory "${PWD}/artifacts/native-amd64" \
  --architecture amd64
```

The verifier rejects extra files, links, altered hashes, wrong controls,
unexpected payload paths, wrong modes, malformed ELF files, architecture
mismatches, dependency drift, and changes to either underlying immutable
manifest.

For the H2 acceptance run, package the same binary inputs twice with the same
toolchain, dependency database, and epoch through the guarded reproducibility
driver:

```bash
epoch="$(git log -1 --format=%ct)"
python_arch="$(dpkg --print-architecture)"
mkdir -p "${PWD}/target"
bash scripts/linux/check-native-reproducibility.sh \
  --binary-dir "${binary_dir}" \
  --output "${PWD}/target/native-${python_arch}-reproducibility" \
  --architecture "${python_arch}" \
  --source-date-epoch "${epoch}"
```

The driver requires a clean tracked tree, runs and independently verifies both
package sets, compares all four files byte for byte, and emits a bounded summary
binding the commit, architecture, epoch, filenames, and hashes. Repeat on amd64
and arm64, install both packages on clean Ubuntu 26.04 machines, and record
reviewed dependency, launch, portal, and uninstall-preservation evidence.
Package installation, upgrade, rollback, purge/export, repository signing, and
the full GDM journey remain H4–H7 gates.

For the dedicated reference-PC installation, use one verified result directory
without copying archives out of the set:

```bash
candidate_dir="${PWD}/target/native-$(dpkg --print-architecture)-reproducibility/run-a"
bash scripts/linux/install-native-candidate.sh \
  --check --directory "${candidate_dir}"
printf 'rmac-reference-pc-install-v1\n' | \
  sudo tee /run/rmac-reference-pc >/dev/null
bash scripts/linux/install-native-candidate.sh \
  --execute --directory "${candidate_dir}"
```

This is an explicitly authorized local-candidate install, not repository trust
or release signing. It runs only from the untouched GNOME Wayland session on
Ubuntu 26.04, verifies the exact native package set before mutation, forbids
APT removals, consumes its `/run` marker before invoking APT, checks the 15 GiB
post-install floor, verifies the exact installed versions and package-owned
session content, and proves that a separate stock GNOME Wayland entry remains.
It does not perform upgrade, rollback, remove, purge, or user-data cleanup.

### Third-party candidates

`--directory` may also hold the exact, pinned `niri` and `xwayland-satellite`
`.deb`s built by `scripts/linux/build-niri-packages.sh`
(`niri_<upstream_version>+lulo<N>_<arch>.deb` and the matching
`xwayland-satellite`, versions from `packaging/third-party/upstreams.json`):
copy `build-niri-packages.sh`'s output `.deb`s in beside the `rmac-apps`/
`rmac-session` pair and regenerate `SHA256SUMS` over all four (`sha256sum --
*.deb > SHA256SUMS`). `verify-native-packages.py` then accepts either the
rmac pair alone or that pair plus exactly this third-party pair -- never one
of the two third-party packages without the other, and never a version other
than the one pinned in `upstreams.json` -- and `install-native-candidate.sh`
installs whichever inventory is present in one `apt-get install`. This is how
the reference PC (which already has the danklinux PPA's niri) ends up running
Lulo OS's own build instead: see [Release process](release-process.md)
"Package names and versions" for why Lulo OS's `+luloN` version now sorts
above the PPA's.
