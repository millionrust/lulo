# Release process

This is the runbook for `.github/workflows/release.yml` and
`.github/workflows/rollout.yml`. Read [Update trust](update-trust.md) first;
this document is the day-to-day operator's guide to the pipeline that
implements it, not a restatement of the trust design itself.

## What exists today

Tagging `vX.Y.Z` always:

- builds `rmac-apps` and `rmac-session` on amd64 (a real `ubuntu:26.04`
  container, since GitHub has no hosted Ubuntu 26.04 image yet -- see
  "Runner decisions" below);
- builds the same on arm64 **only if** the `RMAC_HAS_ARM64_RUNNER`
  repository variable is `true` (nothing is configured today, so this job
  is skipped, not failed);
- builds Lulo OS's `niri` and `xwayland-satellite` packages, their complete
  source packages, and their SBOMs (see "Third-party packages: niri and
  xwayland-satellite" below);
- generates an SBOM with `cargo-cyclonedx` (version pinned in
  `release.yml`'s `RMAC_CARGO_CYCLONEDX_VERSION`);
- attests build provenance with `actions/attest-build-provenance`; and
- attaches the `.deb` files, the niri/xwayland-satellite source packages,
  `SHA256SUMS`, and the SBOMs to the GitHub Release, creating it if
  needed. Any `~` in a file name (rmac's pre-release versions) becomes `.`
  first, because GitHub rewrites it in asset names.

None of that needs a secret. That release bundle is also what
`scripts/linux/install.sh --from-release <tag>` and `--from-dir` install
from today, ahead of the signed APT repository below (see
[Install](install.md) "Install from a GitHub Release (the Beta path)" and
[Beta clean-VM checklist](beta-clean-vm-checklist.md)).

Signing and publishing the APT repository -- `stage-apt-snapshot.py`,
clearsigning `InRelease`, `publish-apt-snapshot.py`, and
`actions/deploy-pages` -- is fully wired in `release.yml`'s
`apt-repository` job and in `rollout.yml`, but both stay off
(`vars.RMAC_SOURCE_PACKAGING_READY != 'true'`) until the two gaps in
"What the owner still has to do" are closed. Building the keyring packages
(`keyring` job in `release.yml`) is separately gated on
`vars.RMAC_ARCHIVE_SIGNING_FINGERPRINT` being set, and then fails if the
`RMAC_ARCHIVE_PUBLIC_KEYRING_B64` secret is missing (a job-level `if`
cannot read secrets).

## What the owner still has to do

1. **Decide who holds the signing keys** (`update-trust.md` "Decisions
   needed" is still unchecked). Generate the offline primary key and a
   bounded-lifetime online signing subkey, per `update-trust.md` "Signing
   and rotation". Nothing in this repository, and no workflow here, ever
   generates a key -- that stays a deliberate, offline, human action.

2. **Build a real Debian source package for `rmac`.** `update-trust.md`
   requires a genuine source offer bound to the exact `Cargo.lock` and
   commit for every binary publication; a promise to add it later is
   explicitly not acceptable. Nothing in this repository builds one today
   -- `build-native-packages.py` deliberately packages prebuilt ELF
   binaries without a `dpkg-buildpackage` source flow, and there is no
   `debian/` source-packaging tree for the Rust workspace. Until a real
   builder exists (producing a `.dsc`, source tarball, `.buildinfo`, and
   `.changes` for package `rmac`, ideally as a new job in `release.yml`
   feeding a `rmac-source` artifact), `apt-repository` and `rollout` must
   stay off. Do not flip `RMAC_SOURCE_PACKAGING_READY` to `true` before
   this exists.

3. **Configure the secrets and variables**, once 1 and 2 are done:
   - Repository/environment variable `RMAC_ARCHIVE_SIGNING_FINGERPRINT`
     (the subkey's fingerprint -- not secret, published in the README,
     `docs/install.md`, and every Release).
   - Environment secret `RMAC_APT_SIGNING_SUBKEY` in the `apt-signing`
     GitHub Environment (an ASCII-armored export of the signing subkey
     only, never the offline primary key).
   - Repository secret `RMAC_ARCHIVE_PUBLIC_KEYRING_B64` (the exported
     public keyring, base64-encoded) so the `keyring` job can build
     `rmac-archive-keyring`.
   - Repository variable `RMAC_SOURCE_PACKAGING_READY = true`, last, once
     everything above is in place.

4. **Configure the `apt-signing` GitHub Environment** (Settings ->
   Environments) to require a reviewer's approval before a job using it
   runs. Both `apt-repository` and `rollout` reference this environment, so
   every publish and every rollout step waits for that approval.

5. **Enable GitHub Pages** for this repository with source "GitHub Actions"
   (Settings -> Pages), so `actions/deploy-pages` has somewhere to deploy
   to.

6. Publish the archive fingerprint in the README, `docs/install.md`, and
   each Release, and replace `install.sh`'s and `uninstall.sh`'s TODO
   fingerprint placeholder with the real value.

7. If arm64 releases are wanted, provision a
   `[self-hosted, linux, arm64]` runner and set
   `RMAC_HAS_ARM64_RUNNER = true`.

## Third-party packages: niri and xwayland-satellite

`rmac-session` needs niri and xwayland-satellite, and neither is in the
Ubuntu 26.04 archive (the reference laptop got them from the third-party
`avengemedia/danklinux` PPA: `niri 26.04ppa3`, `xwayland-satellite
0.8.2ppa1`). So Lulo OS builds and ships the exact releases rmac is tested
against, unmodified, in every GitHub Release.

### What is pinned

`packaging/third-party/upstreams.json` pins each upstream release by tag,
commit, and tarball SHA-256, and (once recorded) the SHA-256 of the
`cargo vendor` tarball:

| Package | Tag | Commit | Upstream tarball SHA-256 | Licence |
|---|---|---|---|---|
| niri | `v26.04` | `8ed0da44d974c32c6877d2f4630c314da0717ecb` | `134c602d8e0d53413a52d6cd58f9ce7e79a07d03288ee0a51ba1abd5db1b1ad9` | GPL-3.0-or-later |
| xwayland-satellite | `v0.8.2` | `8d135d3b2854b30fd01ea6cd6c27e523dd50a839` | `cb50bb6948582d5ec3aa511d2d66ad622989bb14bef94e3bb81bae8b64c120b1` | MPL-2.0 (embeds Open Sans, OFL-1.1) |

The tarballs are GitHub's `archive/refs/tags/<tag>.tar.gz`; the build also
checks the commit that GitHub records inside each tarball (`git
get-tar-commit-id`). niri's repository moved from `YaLTeR/niri` to
`niri-wm/niri`; the pin uses the new home.

### Package names and versions (the choice, and why)

The packages keep the upstream names, `niri` and `xwayland-satellite`, with
the Debian revision `0luloN`: `niri 26.04-0lulo1`, `xwayland-satellite
0.8.2-0lulo1`. `rmac-session` depends on `niri (>= 26.04)` and
`xwayland-satellite (>= 0.8.2)`. Under dpkg's ordering:

- `26.04 < 26.04-0lulo1`, so our build satisfies `rmac-session`;
- `26.04-0lulo1 < 26.04ppa3`, so on a machine that already has the PPA,
  apt keeps the PPA build (no forced downgrade; it satisfies
  `rmac-session` too). `install.sh` detects this, keeps it, and prints the
  `apt-get install --allow-downgrades` command for anyone who wants to
  switch;
- `26.04-0lulo1 < 26.04-1`, `< 26.04-0ubuntu1`, and `< 26.04-0.1`, so a
  future official Debian or Ubuntu package upgrades over ours cleanly, with
  no `Conflicts`/`Replaces` dance and no leftover `lulo-*` package;
- the next Lulo build of a newer upstream (`26.08-0lulo1`) sorts above
  `26.04ppa3`, so PPA users move onto it by an ordinary upgrade (unless the
  PPA has meanwhile published its own, higher `26.08ppaN`).

A renamed package (`lulo-niri` with `Provides`/`Conflicts: niri`) was
rejected: both install `/usr/bin/niri`, so it must conflict with the PPA
package and with any future official one, and apt would never replace it
with the official package on its own. A `~lulo1` suffix was rejected too:
`26.04-0~lulo1` sorts *below* `26.04`, so it would need a `(>= 26.04~)`
relation, and GitHub rewrites `~` in Release asset names.
`scripts/test_third_party_packages.py` asserts every ordering above.

### What the packages install

`niri` installs what upstream's own packaging does (and the PPA package
does): `/usr/bin/niri`, `/usr/bin/niri-session`, the `niri.service` and
`niri-shutdown.target` user units, `niri.desktop`, `niri-portals.conf`, and
bash/fish/zsh completions. rmac relies on `niri-session` (which
`rmac-wayland-session` runs as `/usr/bin/niri-session -l`), `niri.service`,
and `niri-shutdown.target` (which `niri-session` starts on exit).
`niri --version` reports `26.04 (8ed0da4)`. `xwayland-satellite` installs
the binary and its man page; niri starts it on demand. `Depends` come from
`dpkg-shlibdeps`, plus the libraries niri opens with `dlopen`
(`libegl1`, `libegl-mesa0`, `libwayland-server0`) and `xwayland`.

### How it is built

`scripts/linux/build-niri-packages.sh`, both in CI and on the laptop:

1. downloads each tarball and refuses it unless its SHA-256 and embedded
   commit match the pin;
2. runs `cargo vendor --locked` (the only networked step) and packs
   `vendor/` into a deterministic `<name>_<version>.orig-vendor.tar.xz`
   (sorted, fixed mtime/owner, single-threaded xz). Once
   `vendor_sha256` is recorded in `upstreams.json`, a different vendor
   tarball stops the build;
3. adds `packaging/third-party/<name>/debian` and a generated
   `debian/dependency-licenses.txt` (every vendored crate's licence and
   notice files; installed as `/usr/share/doc/<name>/LICENSE.dependencies`)
   and runs `dpkg-buildpackage -us -uc -sa`. `debian/rules` is hand-written
   (dpkg-dev only, no debhelper) and compiles with `cargo build --frozen`
   against the vendored sources, the repository's pinned Rust 1.95.0,
   `--remap-path-prefix`, and `SOURCE_DATE_EPOCH` from `debian/changelog`;
4. writes a CycloneDX SBOM per package and architecture
   (`<name>_<version>_<arch>.cdx.json`: every `Cargo.lock` entry, the
   upstream commit and tarball hash, the vendor tarball hash, and the hash
   of every produced file) and a `SHA256SUMS`.

The source package (`.dsc`, `.orig.tar.gz`, `.orig-vendor.tar.xz`,
`.debian.tar.xz`) plus `.buildinfo` and `.changes` is the complete
corresponding source for each binary. It is the GPL-3.0 and MPL-2.0 source
offer, published beside the binaries in the same Release, as
`update-trust.md` "Source and license obligations" requires; `dpkg-source
-x <name>_<version>.dsc` reproduces the build tree.

In `release.yml`, `build-third-party-amd64` runs the script in the same
`ubuntu:26.04` container and non-root `builder` pattern as `build-amd64`
(`--build-deps system`, with the Build-Depends installed by apt), and
`attach-release` does not publish until it succeeds. `build-third-party-arm64`
is gated on `RMAC_HAS_ARM64_RUNNER` like `build-arm64`; the source package
is published once, from amd64.

### Building them on the reference laptop

No sudo and no Docker. Uses the existing `~/.cargo` toolchain, one work
directory (`~/rmac-niri-build`, with a single reused cargo target
directory inside it), and refuses to start with under 25 GiB free. The
laptop lacks the `-dev` packages, so the script's `user-sysroot` mode
fetches them with `apt-get download` (no root) and unpacks them under
`~/rmac-niri-build/sysroot` (about 50 MiB). Run it when nothing else is
compiling:

```sh
cd ~/<an up-to-date checkout of this branch>
df -h /home
bash scripts/linux/build-niri-packages.sh --jobs 2
```

It prints the output directory (`~/rmac-niri-build/packages-<timestamp>`)
and each vendor tarball's SHA-256. Expect a long first build (niri uses
thin LTO; 2 jobs keep the 6.7 GB machine out of swap). Then:

- record the two printed vendor SHA-256 values as `vendor_sha256` in
  `packaging/third-party/upstreams.json` and commit them, so CI must
  reproduce the same vendored sources;
- inspect without installing: `dpkg-deb -I` and `dpkg-deb -c` on each
  `.deb` (check `Depends`, the file list above, and that nothing lands
  outside `/usr`), `lintian` if available;
- to try one, it is `sudo apt-get install --allow-downgrades
  ./niri_26.04-0lulo1_amd64.deb ./xwayland-satellite_0.8.2-0lulo1_amd64.deb`
  (the PPA build is newer), and `sudo apt-get install niri=26.04ppa3
  xwayland-satellite=0.8.2ppa1` goes back. Both need the owner's sudo.

To combine them with a native rmac package set for `install.sh --from-dir`,
copy both directories' `.deb` files into one directory and run `sha256sum
-- *.deb > SHA256SUMS` there.

### Updating to a new upstream release

Change the tag, commit, tarball URL/SHA-256, directory, and
`upstream_version` in `upstreams.json`; set `vendor_sha256` to `null`; add a
`debian/changelog` entry (`<version>-0lulo1`); update
`UPSTREAM_SHORT_COMMIT` in niri's `debian/rules` and the file list if
upstream's packaging changed; raise the floors in
`native_package_contract.py`; run the laptop build, record the vendor hash,
and re-run the tests. A rebuild of the same upstream bumps `0luloN`.

### Known gaps

- **Not yet built anywhere.** Neither the CI job nor the laptop path has
  run; expect to debug the first run (in particular the laptop's
  user-sysroot pkg-config rewrite and `bindgen`'s view of it).
- **Reproducibility is designed for, not proven.** Unlike rmac's own
  packages there is no second independent build compared byte for byte.
- **The signed APT repository does not carry them yet.**
  `stage-apt-snapshot.py` only knows `rmac` and `rmac-archive-keyring`
  sources; it must learn these two before `apt-repository` is enabled.
- **arm64** needs the same self-hosted runner as rmac's arm64 packages.

## Runner decisions

**amd64**: built inside a real `ubuntu:26.04` container on a
`ubuntu-latest` host, rather than directly on the host OS, so the build
uses the target distribution's actual glibc and toolchain ABI.
`build-native-inputs.sh` and `check-native-reproducibility.sh` both refuse
to run as root, so the job creates a non-root `builder` user and runs the
Rust/package steps through `sudo -u builder`. **This has not been
validated against a real GitHub Actions run** -- in particular, whether the
default `ubuntu-latest` runner's free disk space is enough for two
side-by-side Rust dependency graphs (the root workspace and `shell/`,
per ADR 0013) has not been checked. If a release build fails on disk space,
move to a larger-disk hosted runner or a self-hosted one before assuming
the build logic itself is wrong.

**arm64**: no GitHub-hosted Ubuntu 26.04 arm64 image exists, and this
repository has no self-hosted runner configured, so the job is defined
(same container approach) but gated behind `RMAC_HAS_ARM64_RUNNER` and
skipped by default. `attach-release` still runs (and still publishes an
amd64-only Release) when arm64 is skipped; `apt-repository` and `rollout`
do not, since a two-architecture APT repository needs both.

## Tagging a release

1. Make sure `dev` is what you want to ship, then fast-forward or merge
   into the branch the tag will point at.
2. `git tag vX.Y.Z` (matching the workspace version in the root
   `Cargo.toml`) and `git push origin vX.Y.Z`.
3. Watch the `Release` workflow run. `dependency-policy`, `build-amd64`,
   and `sbom` need no approval. `apt-repository` (once enabled) will pause
   for the `apt-signing` environment's required reviewer approval --
   approve it from the run's page only after checking the staged snapshot
   looks right.
4. Once `attach-release` finishes, check the Release page: `.deb` files
   (including `niri` and `xwayland-satellite`), their `.dsc`,
   `.orig.tar.gz`, `.orig-vendor.tar.xz`, `.debian.tar.xz`, `.buildinfo`,
   and `.changes`, `SHA256SUMS`, the SBOMs, and a provenance attestation
   should all be attached.

## Tagging a pre-release (Alpha/Beta/RC)

A tag whose name is not exactly `vX.Y.Z` (for example `v0.9.0-beta.1`,
matching a workspace version of `0.9.0-beta.1`) is a pre-release. The
`attach-release` job detects the suffix and passes `--prerelease` to
`gh release create`/`gh release edit`, so it publishes as a GitHub
pre-release rather than "Latest" -- readers of the Releases page see it
correctly labeled, and it never gets picked up by a tool that only follows
"Latest". Everything else about the workflow is unchanged: `build-amd64`,
`sbom`, and `attach-release` need no secrets and no approval, so a Beta tag
produces its `.deb` files, `SHA256SUMS`, the SBOM, and a provenance
attestation the same way a final release does. `apt-repository` and
`keyring` stay off regardless (see "What exists today" above) until the
owner's signing-key and source-package decisions are made -- a Beta
pre-release is a GitHub Release with a manual install guide
([docs/install.md](install.md)), not an APT repository entry.

## Approving the signing environment

Every run of a job with `environment: apt-signing` (both `release.yml`'s
`apt-repository` and `rollout.yml`) stops and waits in GitHub's "Review
deployments" UI until an authorized reviewer approves it. Before approving:

- confirm the tag/run is one you expect (no unexpected commit range);
- for a rollout step, confirm the requested phase makes sense (a scheduled
  tick should only ever be the next step forward; a manual dispatch should
  match what was actually agreed).

Rejecting the deployment stops the job before it imports any key material.

## Halting a rollout

Run `rollout.yml` manually (Actions -> Rollout -> Run workflow) with
`phase: "0"`. `next-rollout-phase.py` allows an explicit `0` immediately,
regardless of how long the current phase has been live. A scheduled tick
never resumes a halted (0%) rollout on its own -- resuming needs another
manual dispatch with a real phase.

## Emergency higher-version path

APT versions and Release snapshots only move forward
(`update-trust.md` "Staged rollout and rollback"). There is no downgrade,
pin-above-1000, forced revert, or replayed older `Release` file. To ship an
emergency fix:

1. Fix the issue and bump the workspace version (or just the Debian
   revision) so the new build's version compares higher than the affected
   one.
2. Tag and push as normal (`git tag vX.Y.Z+1`). Security fixes should
   publish at `--phase 100` directly rather than starting the normal
   10/25/50/100 phasing -- this is not yet wired as a `release.yml` input;
   until it is, edit the `--phase` argument in the `apt-repository` job for
   that run, or extend the workflow with a `workflow_dispatch` override
   before relying on this in practice.
3. The previous signed snapshot and package hashes stay retained
   (`publish-apt-snapshot.py` keeps at least three) for diagnosis; automatic
   clients only ever see the higher-version revert.

## Known gaps and risks (read before your first real run)

- **Untested runner plumbing.** The non-root-build-user-in-a-container
  pattern in `build-amd64`/`build-arm64`/`keyring` has not been exercised
  against a real GitHub Actions run. Expect to debug it on the first tag
  push.
- **Container disk space.** See "Runner decisions" above.
- **No `rmac` source-package builder.** This is the hard blocker on
  `apt-repository`/`rollout` ever running; see item 2 above.
- **`rollout.yml`'s republish step is a stub.** The phase-decision logic
  (`next-rollout-phase.py`) is real and tested; the actual
  stage/sign/promote/deploy call is left as a documented `TODO` that
  mirrors `release.yml`'s `apt-repository` job, because it depends on how
  the release bundle ends up shaped once the source-package builder exists.
  Wire it up before relying on scheduled rollout steps.
- **Pages has no history.** `actions/deploy-pages` deploys an artifact, not
  a `gh-pages` branch, so nothing but the live site itself remembers what
  was previously published. `apt-repository` mirrors the current live site
  with `wget` before promoting so `publish-apt-snapshot.py` can compare
  against it and keep retained snapshots; if the mirror step or the Pages
  size limit (1 GB) ever drops old snapshots, treat that as an incident,
  not routine behavior.
- **`cargo-cyclonedx` is pinned to a specific version** for reproducible
  SBOMs; bump `RMAC_CARGO_CYCLONEDX_VERSION` deliberately, not implicitly.
- **Key custody is still an open decision** (`update-trust.md` "Decisions
  needed"): who holds the offline primary key, and how it is stored, is
  not decided by this document or by any workflow here.
