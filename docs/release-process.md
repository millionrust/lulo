# Release process

This is the runbook for `.github/workflows/release.yml` and
`.github/workflows/rollout.yml`. Read [Update trust](update-trust.md) first;
this document is the day-to-day operator's guide to the pipeline that
implements it, not a restatement of the trust design itself.

## What a tag does

Tagging `vX.Y.Z` (or a pre-release such as `v0.9.0-beta.1`):

- builds `rmac-apps` and `rmac-session` on amd64 (a real `ubuntu:26.04`
  container, since GitHub has no hosted Ubuntu 26.04 image yet -- see
  "Runner decisions" below), twice, and requires byte-identical packages;
- builds the same on arm64 **only if** the `RMAC_HAS_ARM64_RUNNER`
  repository variable is `true` (nothing is configured today, so this job
  is skipped, not failed);
- builds Lulo OS's `niri` and `xwayland-satellite` packages, their complete
  source packages, and their SBOMs (see "Third-party packages: niri and
  xwayland-satellite" below);
- builds the `rmac` Debian source package (`rmac-source`: `git archive` of
  the commit plus one `cargo vendor` of both workspaces, and
  `packaging/rmac-source/debian`), then proves it in `rmac-source-rebuild`
  by unpacking it with `dpkg-source -x` and rebuilding both binary packages
  with cargo's network disabled;
- builds the `rmac-archive-keyring` packages from the committed public
  keyring, once the archive key exists (the `keyring` job, gated on the
  `RMAC_ARCHIVE_SIGNING_FINGERPRINT` variable);
- generates an SBOM with `cargo-cyclonedx` (version pinned in
  `release.yml`'s `RMAC_CARGO_CYCLONEDX_VERSION`);
- attaches the `.deb` files, the niri/xwayland-satellite source packages,
  `apt-inputs-<tag>.tar` (the exact, unrenamed package sets the APT
  repository is built from, including the `rmac` source package and the
  keyring packages), `SHA256SUMS`, and the SBOMs to the GitHub Release, and
  attests build provenance for all of them. Any `~` in a file name (rmac's
  pre-release versions) becomes `.` first, because GitHub rewrites it in
  asset names; the `rmac` source package keeps its real names inside
  `apt-inputs-<tag>.tar`. A Release that the APT repository already serves
  is sealed: re-running `attach-release` for it fails rather than replace
  its assets;
- once signed updates are switched on (below), `apt-repository` rebuilds
  the published repository from the Releases, stages the new release at
  `Phased-Update-Percentage: 10`, waits for the `apt-signing` reviewer,
  signs, promotes, records `apt-snapshot-<id>.tar` on the Release, and
  `apt-pages` deploys it to GitHub Pages.

`rollout.yml` then runs every 6 hours: it re-signs the repository before
its 48-hour `Valid-Until` gets close and steps the phase 10 -> 25 -> 50 ->
100, at least 24 hours apart, through the unattended `apt-refresh`
environment. It can never add a package (`--rollout-only`).

Everything up to and including `attach-release` needs no secret. That
release bundle is also what `scripts/linux/install.sh --from-release <tag>`
and `--from-dir` install from (see [Install](install.md)), with no
automatic updates. The default `install.sh` path is the signed repository.

## Switching on signed updates

This is the owner's one-time setup. Everything else is already wired; until
step 9, `apt-repository` and `rollout` are skipped. You need `gh` logged in
as a repository admin (`gh auth status`) and GnuPG 2.2 or newer on the Mac
(`brew install gnupg`). Run the commands from an up-to-date checkout of
`master` that contains this pipeline.

1. **Create the archive key** (ideally offline, on an encrypted volume):

   ```sh
   scripts/release/create-archive-key.sh
   ```

   Give the archive e-mail address, keep the default one-year subkey
   lifetime, choose two *different* passphrases of at least 16 characters
   (primary key, backup archive), and type `create`. It works only in a
   throwaway `GNUPGHOME`, uploads nothing, and writes to
   `./lulo-archive-key-<date>/`:
   `archive-keyring.asc` / `rmac-archive-keyring.gpg` (public),
   `primary-fingerprint.txt`, `RMAC_APT_SIGNING_SUBKEY.asc` (the signing
   subkey only, no passphrase, primary stripped -- self-checked), and
   `primary-key-backup.tar.gpg` (the primary secret key, its revocation
   certificate, and restore/renew instructions, encrypted with the backup
   passphrase). It also writes the pin into the repository:
   `packaging/apt/archive-keyring.asc`, `packaging/apt/archive-key.json`, and
   the `RMAC_ARCHIVE_KEYRING_FINGERPRINT` line of `scripts/linux/install.sh`.

2. **Store the offline key.** Copy `primary-key-backup.tar.gpg` to two
   offline media kept in different places, then delete the local copy. Keep
   the two passphrases apart (password manager and paper). Note the subkey
   expiry printed by the script and set a reminder one month before it
   (renewal: README in the backup; the release after a renewal must ship the
   new `archive-keyring.asc`).

3. **Create the two signing environments.** `apt-signing` (new packages)
   requires your approval and only deploys from `v*` tags; `apt-refresh`
   (rollout steps and signature refreshes, unattended) only runs from
   `master`:

   ```sh
   repo=millionrust/lulo
   me="$(gh api user --jq .id)"
   printf '{"reviewers":[{"type":"User","id":%s}],"deployment_branch_policy":{"protected_branches":false,"custom_branch_policies":true}}' "$me" \
     | gh api -X PUT "repos/$repo/environments/apt-signing" --input -
   gh api -X POST "repos/$repo/environments/apt-signing/deployment-branch-policies" -f name='v*' -f type=tag
   printf '{"deployment_branch_policy":{"protected_branches":false,"custom_branch_policies":true}}' \
     | gh api -X PUT "repos/$repo/environments/apt-refresh" --input -
   gh api -X POST "repos/$repo/environments/apt-refresh/deployment-branch-policies" -f name=master -f type=branch
   ```

4. **Add the signing subkey and the fingerprint**, then destroy the subkey
   export:

   ```sh
   key_dir=./lulo-archive-key-<date>     # the directory step 1 printed
   gh secret set RMAC_APT_SIGNING_SUBKEY --repo "$repo" --env apt-signing < "$key_dir/RMAC_APT_SIGNING_SUBKEY.asc"
   gh secret set RMAC_APT_SIGNING_SUBKEY --repo "$repo" --env apt-refresh < "$key_dir/RMAC_APT_SIGNING_SUBKEY.asc"
   gh variable set RMAC_ARCHIVE_SIGNING_FINGERPRINT --repo "$repo" --body "$(cat "$key_dir/primary-fingerprint.txt")"
   rm -P "$key_dir/RMAC_APT_SIGNING_SUBKEY.asc"
   ```

   The variable is the PRIMARY fingerprint (not secret; it is also in
   `install.sh`). No public-keyring secret is needed any more: the keyring
   package is built from the committed `packaging/apt/archive-keyring.asc`.

5. **Enable GitHub Pages** with source "GitHub Actions", and let its
   `github-pages` environment accept deployments from `v*` tags and
   `master`:

   ```sh
   gh api -X POST "repos/$repo/pages" -f build_type=workflow \
     || gh api -X PUT "repos/$repo/pages" -f build_type=workflow
   printf '{"deployment_branch_policy":{"protected_branches":false,"custom_branch_policies":true}}' \
     | gh api -X PUT "repos/$repo/environments/github-pages" --input -
   gh api -X POST "repos/$repo/environments/github-pages/deployment-branch-policies" -f name=master -f type=branch
   gh api -X POST "repos/$repo/environments/github-pages/deployment-branch-policies" -f name='v*' -f type=tag
   ```

6. **Commit the pin** and get it onto `master` (the tag must contain it,
   and the scheduled rollout runs from `master`):

   ```sh
   git add packaging/apt/archive-keyring.asc packaging/apt/archive-key.json scripts/linux/install.sh
   git commit -m "Pin the Lulo OS archive signing key"
   python3 -m pytest -q scripts/test_install_uninstall.py scripts/test_apt_publication.py
   git push origin HEAD:master
   ```

   The key must be older than the release commit: the keyring package build
   refuses a key created after `SOURCE_DATE_EPOCH`.

7. **Optionally** set `RMAC_HAS_ARM64_RUNNER=true` once a
   `[self-hosted, linux, arm64]` runner exists. Until then the repository
   serves arm64 machines only the keyring.

8. **Allow the first publication, once:**

   ```sh
   gh variable set RMAC_APT_FIRST_PUBLICATION --repo "$repo" --body true
   ```

9. **Switch publishing on and tag the Beta:**

   ```sh
   gh variable set RMAC_APT_PUBLISHING_ENABLED --repo "$repo" --body true
   git tag v0.9.0-beta.1 && git push origin v0.9.0-beta.1
   ```

   Watch the Release run. When `apt-repository` waits for review, open the
   run, check the tag and commit are the ones you meant, and approve
   `apt-signing`. When `apt-pages` has deployed:

   ```sh
   gh variable delete RMAC_APT_FIRST_PUBLICATION --repo "$repo"
   curl -fsSI https://millionrust.github.io/lulo/dists/resolute/InRelease
   ```

   Leaving the first-publication variable set makes the next publication
   fail on purpose.

10. **Check a client** on a disposable Ubuntu 26.04 machine:
    `curl -fsSL https://millionrust.github.io/lulo/install.sh | sh`, log in
    to Lulo OS, then after the next release confirm that
    `systemctl --user start rmac-update-check.service` prepares the update
    and a restart installs it ([Software Update](software-update.md)).

Keep the repository active: GitHub disables scheduled workflows in a public
repository after 60 days without activity, and without `rollout.yml` the
repository's signatures expire 48 hours later and every client's
`apt update` reports an error for it (re-enable the workflow and run it by
hand with the current phase to recover).

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
a `+luloN` upstream-version suffix: `niri 26.04+lulo1`, `xwayland-satellite
0.8.2+lulo1` (the Debian `+dfsg`/`+repack` pattern for a locally-built
variant of an upstream release, not a `-revision` after a hyphen).

An earlier design used a `-0luloN` Debian revision (`niri 26.04-0lulo1`)
instead, reasoning that it needed to sort *below* the danklinux PPA's
`26.04ppa3` so apt would never force a downgrade on a machine that already
had the PPA build. That was a bug, not a feature: dpkg's version comparison
splits a hyphenated version at the *last* hyphen and compares the
upstream-version part first; `26.04-0lulo1`'s upstream-version part is the
bare `26.04`, and a bare `26.04` always sorts below `26.04ppa3` no matter
what comes after the hyphen. So our own archive's niri could never be
`apt upgrade`d to on a machine that already had the PPA's build -- which,
in practice, is most of them, including the owner's laptop -- and pinning
our version above the floor `rmac-session` required would have been a
downgrade from the PPA's perspective.

`+luloN` fixes this by putting the marker *inside* the upstream-version part
that dpkg compares first, so our build outranks the PPA unconditionally:

- **Letters sort below everything except themselves and `~`.** Comparing
  `26.04` against `26.04ppa3`, dpkg matches the common `26.04` and then
  compares what follows: nothing (end of string) against `ppa3`. End of
  string sorts below a letter, so plain `26.04` already loses to
  `26.04ppa3` -- confirming the bug above. Comparing `26.04+lulo1` against
  `26.04ppa3` the same way: `+lulo1` (starts with `+`, a non-letter) against
  `ppa3` (starts with `p`, a letter). Letters sort *below* non-letters other
  than `~`, so `ppa3` < `+lulo1` and `26.04ppa3` < `26.04+lulo1` --
  regardless of the PPA's own build number, since the comparison is decided
  by `p` vs `+` before either string's digits are ever reached. The same
  argument beats a bare Debian/Ubuntu revision (`26.04-1`, `26.04ubuntu1`):
  a hyphenated `26.04-1`'s upstream-version part is bare `26.04`, which
  loses to `26.04+lulo1` the same way plain `26.04` did above; `ubuntu1`
  starts with a letter, so it loses to `+lulo1` the same way `ppa3` did.
- **A future upstream release still wins.** `26.04+lulo1` vs `26.05`: dpkg
  compares the digit runs first (`26` = `26`, then `04` vs `05`), and `04 <
  05` decides it before the `+lulo1` suffix is ever compared. Any higher
  upstream version -- ours or anyone else's -- sorts above every `26.04*`
  build.
- **A rebuild of the same upstream bumps the counter**, exactly as the old
  `0luloN` scheme did (`26.04+lulo1 < 26.04+lulo2`, ordinary digit
  comparison once the shared `+lulo` prefix matches).

Verified directly with `dpkg --compare-versions` on the reference laptop
(`ssh jacob@192.168.18.52`):

```
$ dpkg --compare-versions 26.04+lulo1 gt 26.04ppa3 && echo yes
yes
$ dpkg --compare-versions 26.04+lulo1 gt 26.04-1 && echo yes
yes
$ dpkg --compare-versions 26.04+lulo1 gt 26.04ubuntu1 && echo yes
yes
$ dpkg --compare-versions 26.04+lulo1 lt 26.05 && echo yes
yes
```

Two alternatives were rejected:

- **`26.04ppa3+lulo1`** (embedding the exact PPA version being beaten) is
  fragile: it only outranks `26.04ppa3` specifically. `dpkg --compare-versions
  26.04ppa3+lulo1 lt 26.04ppa4` is true -- the digit comparison of `ppa3`
  against `ppa4` decides the ordering before `+lulo1` is ever reached, so
  the very next PPA point release would sort above ours again. `+luloN`
  alone never has this problem, because the letters-vs-`+` comparison that
  decides it happens before either side's trailing digits.
- **An epoch** (`1:26.04`) would also sort above the PPA unconditionally and
  is the standard fallback when nothing in the version string itself can be
  made to compare correctly. It was avoided here because it is permanent
  and highly visible (`apt policy niri`, `dpkg -l`, every future changelog
  entry needs it too) for a problem a plain upstream-version suffix already
  solves without one, and because bumping an epoch later, if some other
  future ordering problem ever needs it, is easy -- removing one, once
  users have it recorded in `dpkg`'s status file, is not.

A renamed package (`lulo-niri` with `Provides`/`Conflicts: niri`) was
rejected too: both install `/usr/bin/niri`, so it must conflict with the PPA
package and with any future official one, and apt would never replace it
with the official package on its own. A `~lulo1` suffix was rejected for a
different reason: `26.04~lulo1` sorts *below* `26.04` (`~` sorts below
everything, even end of string), the opposite of what is needed here, and
GitHub also rewrites `~` in Release asset names.
`scripts/test_third_party_packages.py` asserts every ordering above.

`rmac-session`'s `Depends` on niri and xwayland-satellite name the exact
pinned `+luloN` version, not just the bare upstream version (see "What the
packages install" and `native_package_contract.py`): installing or
upgrading `rmac-session` therefore requires a niri that is at least Lulo
OS's own build, which the PPA's `26.04ppaN` never satisfies. Combined with
`packaging/apt/rmac.pref` pinning our niri at the same priority (500) any
other archive gets by default, this is what actually gets a user's machine
onto our build rather than merely allowing it.

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
- to try one, it is `sudo apt-get install
  ./niri_26.04+lulo1_amd64.deb ./xwayland-satellite_0.8.2+lulo1_amd64.deb`
  (Lulo OS's build sorts above the PPA's, so this is an ordinary install,
  not a downgrade), and `sudo apt-get install --allow-downgrades
  niri=26.04ppa3 xwayland-satellite=0.8.2ppa1` goes back. Both need the
  owner's sudo.

To combine them with a native rmac package set for `install.sh --from-dir`,
copy both directories' `.deb` files into one directory and run `sha256sum
-- *.deb > SHA256SUMS` there.

### Updating to a new upstream release

Change the tag, commit, tarball URL/SHA-256, directory, and
`upstream_version` in `upstreams.json`; set `vendor_sha256` to `null`; add a
`debian/changelog` entry (`<version>+lulo1`); update `UPSTREAM_SHORT_COMMIT`
in niri's `debian/rules` and the file list if upstream's packaging changed;
run the laptop build, record the vendor hash, and re-run the tests.
`rmac-session`'s `Depends` floors (`native_package_contract.py`) are read
straight from `upstreams.json`'s pins, so they never need a separate manual
bump. A rebuild of the same upstream bumps `+luloN`.

### Known gaps

- **Not yet built anywhere.** Neither the CI job nor the laptop path has
  run; expect to debug the first run (in particular the laptop's
  user-sysroot pkg-config rewrite and `bindgen`'s view of it).
- **Reproducibility is designed for, not proven.** Unlike rmac's own
  packages there is no second independent build compared byte for byte.
- **In the APT repository** they are published like rmac's own packages,
  pinned to priority 500 by `rmac.pref`. A rebuilt `niri` with an unchanged
  version is never republished: the published bytes are carried forward
  (the pool is immutable), so a new niri build only reaches clients with a
  new `+luloN` revision or upstream version.
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
   into `master`.
2. `git tag vX.Y.Z` (matching the workspace version in the root
   `Cargo.toml`) and `git push origin vX.Y.Z`. The version must be higher
   than the published one: the stager refuses a version that goes
   backwards, and refuses a reused orig tarball name with new bytes (a
   Debian-revision-only bump must not change the source).
3. Watch the `Release` workflow run. Nothing before `apt-repository` needs
   approval. `apt-repository` pauses for the `apt-signing` reviewer --
   approve it only after checking the tag and commit range.
4. Once `attach-release` finishes, check the Release page: `.deb` files
   (including `niri` and `xwayland-satellite`), their `.dsc`,
   `.orig.tar.gz`, `.orig-vendor.tar.xz`, `.debian.tar.xz`, `.buildinfo`,
   and `.changes`, `apt-inputs-<tag>.tar`, `SHA256SUMS`, the SBOMs, and a
   provenance attestation should all be attached, and after publication an
   `apt-snapshot-<id>.tar`.

## Tagging a pre-release (Alpha/Beta/RC)

A tag whose name is not exactly `vX.Y.Z` (for example `v0.9.0-beta.1`,
matching a workspace version of `0.9.0-beta.1`) is a pre-release. The
`attach-release` job passes `--prerelease` to `gh release create`/`gh
release edit`, so it publishes as a GitHub pre-release rather than
"Latest". Everything else is the same, including APT publication: the
Debian version `0.9.0~beta.1-38` sorts below `0.9.0-38`, so the final
release later upgrades Beta machines normally.

## Approving the signing environment

Every run of `apt-repository` stops in GitHub's "Review deployments" UI
until you approve `apt-signing`. Before approving, confirm the tag and run
are ones you expect (no unexpected commit range). Rejecting stops the job
before it touches key material.

`rollout.yml` signs through `apt-refresh`, which has no reviewer because the
repository must be re-signed at least every 48 hours. It can only republish
the already-published pool (`stage-apt-snapshot.py --rollout-only`): a phase
change or a fresh Date and `Valid-Until`, never a new package.

## Halting a rollout

Run `rollout.yml` manually (Actions -> Rollout -> Run workflow) with
`phase: "0"`. A halt is allowed immediately, regardless of how long the
current phase has been live. A scheduled tick never resumes a halted (0%)
rollout on its own -- it only keeps re-signing it at 0% -- so resuming needs
another manual run with a real phase. A manual run can never move the phase
backwards except to 0.

## Emergency higher-version path

APT versions and Release snapshots only move forward
(`update-trust.md` "Staged rollout and rollback"). There is no downgrade,
pin-above-1000, forced revert, or replayed older `Release` file. To ship an
emergency fix:

1. Fix the issue and bump the workspace version (or just
   `DEBIAN_REVISION` in `native_package_contract.py`) so the new build's
   version compares higher than the affected one.
2. Tag and push as normal. The release publishes at 10%; for a security
   fix, run `rollout.yml` by hand with `phase: "100"` right after
   `apt-pages` deploys.
3. The previous three signed snapshots stay retained (their bundles remain
   on their Releases) for diagnosis; automatic clients only ever see the
   higher-version revert.

## Known gaps and risks (read before your first real run)

- **None of this has run on GitHub yet.** The container build pattern, the
  `rmac-source` vendoring, the offline rebuild, the publication job, and the
  Pages deployment are exercised by tests and by local runs (a real APT
  client accepting a staged, subkey-signed repository, including phasing,
  was checked on the reference laptop), not by a real Actions run. Expect to
  debug the first tag.
- **Disk.** `build-native-inputs.sh` refuses to start below 25 GiB free,
  both in `build-amd64` and inside `rmac-source-rebuild`; hosted runners may
  not have that. The publication jobs delete preinstalled toolchains to stay
  above `publish-apt-snapshot.py`'s 15 GiB floor.
- **Pages size.** Pages sites are limited to about 1 GB. The site holds the
  pool of the three retained snapshots: normally one release, two for a few
  days after a new one. The `rmac` vendor tarball is the largest object; if
  the site outgrows the limit, filter platform-only crates from the vendor
  tarball or move to another static host.
- **Signature freshness depends on the schedule.** If `rollout.yml` stops
  (disabled schedule, lost secret, failing job), clients start failing
  `apt update` for this repository 48 hours after the last signature.
- **Subkey expiry.** The signing subkey expires (default one year); renew it
  offline and release the new public keyring before it lapses.
- **Unsigned sidecar.** `rmac-snapshot.json` is not signed; it can only
  choose where verified bytes are fetched from and when a phase began.
- **`cargo-cyclonedx` is pinned to a specific version** for reproducible
  SBOMs; bump `RMAC_CARGO_CYCLONEDX_VERSION` deliberately, not implicitly.
- **Runner and supply-chain pinning** (SR-18): rustup is installed by
  `curl | sh`, the `ubuntu:26.04` container is pinned by tag.
