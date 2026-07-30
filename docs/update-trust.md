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
`/usr/share/keyrings/rmac-archive-keyring.pgp`.

The binary OpenPGP keyring is owned by a narrow
`rmac-archive-keyring` package. It is never copied to the global trusted
keyrings and no setup instruction uses `apt-key`, `trusted=yes`, an insecure
repository exception, or disabled expiry. `rmac.pref` names only
`rmac-apps`, `rmac-session`, and the keyring package, preventing the repository
from replacing unrelated Ubuntu packages.

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
- `Signed-By` fingerprints authorizing the next release's exact signer set.

Publication is atomic: upload immutable pool objects and by-hash indices first,
then canonical indices, and make `InRelease` visible last. At least three
complete snapshots remain available for investigation and recovery. A
publisher never overwrites an existing pool object with different bytes.

## Signing and rotation

The archive has an offline OpenPGP primary key and a bounded-lifetime online
signing subkey. Signing happens in an isolated release job after the package,
source, license, and repository verification gates pass. CI receives no
primary-key material.

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
