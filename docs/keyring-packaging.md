# Archive keyring packaging

`rmac-archive-keyring` is the narrow package-managed trust anchor for the rmac
APT source. It installs one unarmored public OpenPGP keyring at
`/usr/share/keyrings/rmac-archive-keyring.gpg`; it contains no secret key,
maintainer script, global APT trust entry, source configuration, or package
pin. The separate source and preference packages consume that path.

APT documents `.gpg` as the supported extension for an unarmored keyring and
`/usr/share/keyrings` as the location for keyrings managed by packages. The
client template therefore uses that exact filename rather than the earlier
development-only `.pgp` suffix. See Debian's
[`apt-secure(8)`](https://manpages.debian.org/testing/apt/apt-secure.8.en.html)
and
[`sources.list(5)`](https://manpages.debian.org/testing/apt/sources.list.5.en.html).

## Build boundary

Build on a native Ubuntu 26.04 amd64 or arm64 host with `dpkg-dev`, `debhelper`,
and GnuPG installed. The builder:

- accepts one absolute public-key export and one or two exact sorted primary
  fingerprints;
- rejects links, oversized input, secret key packets, missing/unrequested
  primaries, and keys without a valid signing primary or subkey;
- re-exports only minimal public material through an isolated temporary GnuPG
  home;
- requires the requested architecture to equal the native dpkg database;
- creates a rootless `3.0 (quilt)` source tree with exact MIT licensing,
  `Rules-Requires-Root: no`, no maintainer hooks, and deterministic xz;
- runs standard `dpkg-buildpackage --build=source,all` unsigned, so `.dsc`,
  `.buildinfo`, and `.changes` describe the real Debian build environment
  rather than a fabricated one; and
- verifies the package payload, strong hashes, source extraction, build/upload
  records, installed path, and source/binary keyring equality before publishing
  the output directory.

The archive publisher later authenticates these reviewed artifacts through its
signed publication manifest and `InRelease`. The build job never receives the
offline archive primary secret or the online repository-signing secret.

Choose the source revision timestamp and build:

```sh
epoch="$(git log -1 --format=%ct)"
mkdir -p "$PWD/artifacts/keyring-amd64"
python3 scripts/linux/build-keyring-packages.py \
  --keyring /absolute/private-build-input/rmac-archive-keyring.gpg \
  --fingerprint FULL_UPPERCASE_PRIMARY_FINGERPRINT \
  --build-architecture amd64 \
  --source-date-epoch "$epoch" \
  --output "$PWD/artifacts/keyring-amd64"
```

During an overlap rotation, pass the two sorted primary fingerprints by
repeating `--fingerprint`. Public key material is not secret, but the input
still belongs in the isolated release workspace rather than the repository:
the committed project must never imply that a development key is production
authority.

## Exact output

For workspace version `0.1.0`, the output contains exactly:

```text
SHA256SUMS
keyring-packages.json
rmac-archive-keyring_0.1.0-1.debian.tar.xz
rmac-archive-keyring_0.1.0-1.dsc
rmac-archive-keyring_0.1.0-1_all.buildinfo
rmac-archive-keyring_0.1.0-1_all.changes
rmac-archive-keyring_0.1.0-1_all.deb
rmac-archive-keyring_0.1.0.orig.tar.xz
```

The manifest binds the canonical public-key bytes/fingerprints, installed path,
source date, native build architecture, exact Debian tool versions, and
SHA-256/SHA-512/size/role for all six package artifacts. `SHA256SUMS` covers
the same exact artifact inventory.

Verify a transferred build:

```sh
python3 scripts/linux/verify-keyring-packages.py \
  --directory "$PWD/artifacts/keyring-amd64" \
  --build-architecture amd64
```

The verifier also invokes `dpkg-source --require-strong-checksums -x` in a
temporary directory. Debian defines the required real-environment fields in
[`deb-buildinfo(5)`](https://manpages.debian.org/testing/dpkg-dev/deb-buildinfo.5.en.html)
and the complete upload inventory in
[`deb-changes(5)`](https://manpages.debian.org/testing/dpkg-dev/deb-changes.5.en.html).

## Acceptance

Build twice with identical inputs, epoch, architecture, and package database;
all eight output files must be byte-identical. Repeat on arm64. Then install
the `.deb` on clean Ubuntu, verify mode `0644` and the exact fingerprint
inventory with GnuPG, prove that APT accepts only the intended signed rmac
repository, upgrade through an overlap keyring, and remove the package without
touching unrelated system trust. Real key custody, signed repository
publication, rotation reach, expiry/revocation, and VM update evidence remain
H7 acceptance requirements.
