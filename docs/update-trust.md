# Update trust and repository operations

H7 uses APT's native authenticated-repository chain rather than inventing an
rmac downloader or treating HTTPS as package authentication. PackageKit
remains the System Settings transaction authority described in
`software-update.md`; the repository supplies the signed APT truth consumed by
its Ubuntu backend.

The machine-readable contract is
`packaging/apt/update-trust.json`. Validate it and the client templates with:

```sh
python3 scripts/linux/verify-update-trust.py
```

## Client boundary

The release package must render `packaging/apt/rmac.sources.in` with the final
HTTPS repository URI and install it as
`/etc/apt/sources.list.d/rmac.sources`. It enables both `deb` and `deb-src`,
pins Ubuntu 26.04's `resolute` suite and `main` component, checks amd64 and
arm64, keeps `Check-Valid-Until` enabled, and accepts signatures only through
`/usr/share/keyrings/rmac-archive-keyring.gpg`.

The binary OpenPGP keyring is owned by a narrow
`rmac-archive-keyring` package. It is never copied to the global trusted
keyrings and no setup instruction uses `apt-key`, `trusted=yes`, an insecure
repository exception, or disabled expiry. `rmac.pref` gives `rmac-apps`,
`rmac-session`, and the keyring package the normal priority 500 from the rmac
origin -- as it does `niri` and `xwayland-satellite`, Lulo OS's own builds of
rmac-session's compositor dependencies, which the Ubuntu archive lacks -- and
gives every other package from that origin priority -1, so the repository can
never install or replace an unrelated Ubuntu package such as `sudo` or
`openssh-server`.

The reproducible binary/source package boundary and rotation build procedure
are specified in [Archive keyring packaging](keyring-packaging.md). It installs
the unarmored public keyring with APT's supported `.gpg` suffix and rejects
secret material before standard Debian package assembly.

APT authenticates the signed Release metadata and the strong hashes it carries;
this establishes archive origin, not that arbitrary package code is safe.
Publishing authority therefore remains isolated from build authority, and H2
archive verification remains mandatory before upload.

## Release metadata

Every publication provides one clearsigned `dists/resolute/InRelease`.
Detached-only `Release.gpg` publication is rejected. Its cleartext Release
record must contain:

- exact Origin `rmac`, Label `rmac`, Suite `stable`, Codename `resolute`,
  Component `main`, and Architectures `amd64 arm64`;
- a monotonically increasing Date and `X-Rmac-Snapshot`;
- `Valid-Until` between six and 48 hours after Date, with at most five minutes
  of accepted future clock skew;
- `Acquire-By-Hash: yes`, with current and retained previous by-hash indices;
- SHA-256 and SHA-512 for every binary/source index and no MD5/SHA-1 release
  chain;
- `Signed-By` fingerprints authorizing the next release's exact signer set,
  comma-separated as apt-secure(8) requires. The archive publishes its
  offline PRIMARY fingerprint, so any valid signing subkey of it is accepted
  and a subkey renewal needs no `Signed-By` transition.

Publication is atomic: upload immutable pool objects and by-hash indices first,
then canonical indices, and make `InRelease` visible last. At least three
complete snapshots remain available for investigation and recovery. A
publisher never overwrites an existing pool object with different bytes.

### Atomic promotion boundary

`scripts/linux/publish-apt-snapshot.py` is the filesystem publisher for a
prepared archive. It does not build packages, generate source offers, create
keys, or sign with an ambient/default key. The isolated signing job prepares an
absolute staging directory containing exactly:

- `dists/resolute/InRelease`;
- `dists/resolute/rmac-publication.json`;
- uncompressed and deterministic-gzip `Packages` for amd64 and arm64;
- uncompressed and deterministic-gzip `Sources`;
- SHA-256 and SHA-512 by-hash copies beside every canonical index; and
- the exact immutable `pool/` binary, `.dsc`, source-tar, `.buildinfo`, and
  `.changes` objects named by the publication manifest.

The manifest binds one increasing UTC snapshot ID, product revision, Date,
Valid-Until, one or two rotation fingerprints, every file's size/SHA-256/
SHA-512, and explicit successful package, reproducibility, source-offer, and
license gates. The clearsigned Release body must hash the manifest and the six
canonical indices, use the exact rmac/resolute/main identity, advertise
amd64/arm64 plus Acquire-By-Hash, and contain no weak hash section.
The amd64 index, and the arm64 index once arm64 is built, must name exactly
`niri`, `rmac-apps`, `rmac-archive-keyring`, `rmac-session`, and
`xwayland-satellite`; an architecture that is not built names only the
Architecture: all keyring, so its clients see no rmac candidate rather than a
partial set. Every paragraph carries the binary's own control fields (Depends,
Description, ...) read from the `.deb`, binds its pool object with both strong
hashes, and uses one allowed consistent phase; keyring delivery is always
100%. The source index must name exactly `niri`, `rmac`,
`rmac-archive-keyring`, and `xwayland-satellite`, with matching strong-hash
`.dsc` and source-tar inventories. Matching `.buildinfo` and `.changes` objects
remain directly bound by the signed publication manifest.

Promote only from the isolated release host, using the package-managed public
keyring that corresponds to the intended client keyring:

```sh
python3 scripts/linux/publish-apt-snapshot.py \
  --staging-dir /absolute/path/to/prepared-repository \
  --repository-dir /absolute/path/to/published-repository \
  --keyring /absolute/path/to/rmac-archive-keyring.gpg
```

The publisher verifies exactly one valid OpenPGP signature with `gpgv` in an
empty home, accepts an exact authorized primary or signing-subkey fingerprint,
checks the live clock/validity window, and compares both Date and snapshot
against the currently published signed metadata. It rejects links, extras,
missing source/build artifacts, altered by-hash objects, false gate state,
weak/extra Release fields, unauthorized or multiple signatures, and an
immutable pool collision.

It first retains a complete metadata snapshot, then installs immutable pool and
by-hash objects, atomically replaces canonical indices and the signed
publication manifest, and atomically replaces `InRelease` last. A crash before
that final replacement leaves the previous signed by-hash view usable; a crash
after it leaves the complete new view visible. Readback and directory `fsync`
bound each visible replacement. The publisher retains at least three metadata
snapshots and refuses a promotion whose projected writes would cross the
15 GiB free-space floor.

### Stateless publication from GitHub Releases

GitHub Pages keeps no history -- `actions/deploy-pages` replaces the whole site
with one artifact, and a site without directory listings cannot be mirrored --
so the publisher's state never lives on Pages (SR-12). Instead:

- every tagged release attaches `apt-inputs-<tag>.tar`, the exact package sets
  the repository is built from (rmac's binaries per architecture, the `rmac`
  source package, Lulo OS's niri and xwayland-satellite binaries and source
  packages, and the keyring packages), covered by the Release's `SHA256SUMS`
  and its build-provenance attestation;
- every publication attaches `apt-snapshot-<snapshot>.tar` to the Release whose
  packages it serves: the exact signed metadata snapshot the publisher retains
  (`InRelease`, the publication manifest, the indices and their by-hash
  copies) plus an unsigned `rmac-snapshot.json` sidecar naming the Release each
  pool object came from and when the current phase began. A Release that
  carries a snapshot bundle is sealed: `attach-release` refuses to replace its
  assets.

Each run of `scripts/linux/publish-apt-repository.sh` (release.yml's
`apt-repository`, rollout.yml) rebuilds the published repository from those
authoritative inputs with `scripts/linux/apt-publication.py collect`: it takes
the newest three snapshot bundles, verifies each `InRelease` with `gpgv`
against the keyring from the target Release's own `rmac-archive-keyring`
package (whose primary fingerprints must equal the reviewed pin in
`packaging/apt/archive-key.json`), checks every metadata file and the indices
against the signed manifest and the sidecar against both, and re-fetches every
pool object those snapshots name from the named Release's `apt-inputs` tar,
itself verified against that Release's `SHA256SUMS` and attestation (signer
workflow `release.yml`). The result is the previous publication byte for byte,
so `publish-apt-snapshot.py`'s monotonic Date/snapshot, immutable-pool, and
retention checks run against real state. Any verification failure, a missing
asset, or a snapshot with no reachable pool object stops the job. No snapshot
bundle at all is refused too, unless the repository variable
`RMAC_APT_FIRST_PUBLICATION` explicitly allows the very first publication; the
flag is itself refused once a history exists.

`stage-apt-snapshot.py` then carries every already-published source-package
version forward byte for byte from that rebuilt pool (a rebuilt
`niri 26.04+lulo1-1` with different bytes never replaces the published one),
refuses any package version lower than the published one and any architecture
that would disappear, and refuses a reused pool path (such as an orig tarball)
with different bytes. The Pages site is the promoted `dists/` and `pool/` plus
`install.sh`, `uninstall.sh`, the bootstrap keyring package, and the armored
public keyring; the new bundle is uploaded to its Release before Pages
changes, so the next run always sees whatever might be live.

The unsigned sidecar can only point at where bytes are fetched from (every
object is checked against the signed manifest) and at when the phase began.
Rewriting it needs repository write access, which could equally change the
workflows.

## Signing and rotation

The archive has an offline OpenPGP primary key (ed25519, certify-only) and a
bounded-lifetime online signing subkey, created by
`scripts/release/create-archive-key.sh` on the owner's machine. Signing happens
in an isolated job after the package, source, license, and repository
verification gates pass. CI receives only the subkey: `sign-apt-release.sh`
refuses a secret whose primary is not a stub.

Two GitHub environments hold that subkey. `apt-signing` has a required
reviewer and signs publications of new packages (release.yml). `apt-refresh`
has no reviewer, is restricted to the default branch, and signs rollout.yml's
phase steps and signature refreshes, which `stage-apt-snapshot.py
--rollout-only` limits to exactly the already-published pool. The split is
forced by `Valid-Until`: metadata expires at most 48 hours after its Date, so
the repository must be re-signed unattended at least that often or every
client's `apt update` fails.

Normal rotation is overlap-first:

1. create and certify the replacement signing key offline;
2. publish a keyring package containing old and new public material, signed by
   the currently trusted signer;
3. publish an old-signed `InRelease` whose `Signed-By` transition admits both;
4. wait at least 30 days and prove the keyring update reached supported hosts;
5. sign with the new key while retaining both public keys;
6. publish a new-signed transition naming only the new signer;
7. remove the old public key in a later keyring update.

Compromise stops publication immediately. Revoke the affected key offline,
publish the revocation and replacement through a separately reviewed recovery
channel, and require explicit administrator recovery where no uncompromised
trusted signer remains. Never weaken APT authentication to make recovery
appear automatic.

## Staged rollout and rollback

Normal versions begin at `Phased-Update-Percentage: 10`, then advance through
25, 50, and 100 only after at least 24 hours of reviewed install, crash,
rollback, and reference-machine evidence per step. Setting the percentage to
zero halts a rollout. APT derives eligibility locally from package/version and
machine ID, so rmac sends no rollout telemetry or machine identity to the
repository.

Security fixes are published at 100%; they are not deliberately withheld by
rmac phasing. System Settings must preserve PackageKit/APT's authoritative
held-back result rather than offering a bypass.

APT versions and Release snapshots only move forward. An emergency rollback is
a reviewed revert built as a higher Debian version, never a pin above 1000,
forced downgrade, replaced pool object, or replayed older Release file. The
previous signed snapshot and package hashes are retained for diagnosis and
manual recovery, but automatic clients receive the higher-version revert.

## Source and license obligations

Every binary publication has matching `deb-src` metadata. The repository keeps
the `.dsc`, complete source tar material, `.buildinfo`, and `.changes` beside
the binary lifetime, all bound by SHA-256 and SHA-512. Source publication binds
the exact `Cargo.lock`, commit, Debian packaging, generated-source procedure,
and H2 binary hashes.

The `rmac` source package (`scripts/linux/build-rmac-source-package.sh`,
`packaging/rmac-source/debian`) is a 3.0 (quilt) package: `git archive` of the
release commit, one deterministic `cargo vendor` tarball covering both cargo
workspaces, and a `debian/` whose rules run the release build procedure
offline. release.yml's `rmac-source-rebuild` job unpacks it with
`dpkg-source -x`, checks its Build-Depends, rebuilds `rmac-apps` and
`rmac-session` with cargo's network disabled, and verifies them; the
repository is not signed unless that passes. The rebuilt binaries are not
required to be byte-identical to the published ones (build paths differ); the
published binaries' own reproducibility is the two-assembly check.

The source and binary packages include the MIT license text and reviewed
copyright inventory. Dependency licenses must pass the existing dependency
policy before publishing. A release is withheld if corresponding source,
rebuild instructions, license text, build information, or notices are missing;
a promise to add them later is not an acceptable source offer.

## Acceptance gate

H7 is complete only after the release environment provides real key
fingerprints and proves:

- `gpgv` accepts exactly one authorized signature using only the installed
  rmac keyring, and rejects unknown, revoked, expired, duplicate, and malformed
  signatures;
- the atomic publisher rejects altered manifests/indices/by-hash objects,
  immutable pool collisions, non-monotonic Date/snapshot state, expired/future
  metadata, and incomplete binary/source publication inventories;
- APT rejects expired/future/replayed metadata, altered indices/packages,
  missing by-hash objects, Release identity changes, and insecure fallback;
- old-to-overlap-to-new key rotation works on installed and offline hosts;
- normal 10/25/50/100 phasing, zero-percent halt, 100% security delivery, and
  no-telemetry client eligibility behave as specified;
- a higher-version emergency revert succeeds while downgrades and snapshot
  replay remain rejected;
- amd64, arm64, and keyring packages have matching signed source artifacts,
  reproducible-build records, licenses, notices, and exact archive hashes; and
- clean install, update, interrupted update, expiry, mirror lag, rotation,
  rollback, uninstall, and stock-GNOME recovery journeys pass in disposable
  Ubuntu 26.04 VMs.

The design follows Debian's
[`apt-secure(8)`](https://manpages.debian.org/testing/apt/apt-secure.8.en.html),
[`sources.list(5)`](https://manpages.debian.org/unstable/apt/sources.list.5.en.html),
and
[repository-format](https://wiki.debian.org/DebianRepository/Format)
contracts, plus Ubuntu's
[phased-update behavior](https://documentation.ubuntu.com/project/how-ubuntu-is-made/concepts/phased-updates/).
