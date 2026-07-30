# rmac 1.0 release candidate

The 1.0 candidate is promoted only after two distinct consecutive candidate
builds pass the complete supported matrix. One good run, one machine, or an
aggregate score cannot satisfy this gate. The exact contract is
`scripts/one-dot-zero-candidate.json`.

## Two-build proof

Each build must independently:

- pass all five H8 stations (amd64 Intel/AMD/NVIDIA, second AMD form factor,
  and arm64);
- record at least 20 attempts for every product journey and at least 95%
  passing for each journey, so a strong journey cannot hide a weak one;
- record zero crashes in desktop session, text document, Notes library,
  Terminal session, and file management;
- pass every I4 performance budget and bind its complete raw evidence archive
  by SHA-256; and
- use one internally consistent signed artifact set from its exact revision.

The two revisions must be distinct and their candidate evidence must be
consecutive in the release ledger. If either build fails, fixes begin a new
two-build sequence.

## Release contents and gates

The final inventory includes source, amd64/arm64 application and session Debian
packages, the supported Text Editor Flatpak bundle, signed APT metadata,
checksums, SPDX JSON SBOM, and build provenance. All 16 release checks,
accessibility/visual/performance/chaos/security evidence, current docs and
limitations, update trust/rollback, install/upgrade/uninstall, safe mode,
stock-GNOME recovery, and lock recovery must pass with zero release blockers.

## Candidate summary

Generate a pending summary from the second revision:

```sh
python3 scripts/verify-one-dot-zero-candidate.py \
  --print-template \
  --version 1.0.0-rc.1 \
  --previous-revision 1111111111111111111111111111111111111111 \
  --revision "$(git rev-parse HEAD)" \
  > /absolute/review/path/one-dot-zero-candidate.json
```

Populate only reviewed artifact hashes/sizes, build-evidence hashes, station
and journey results, performance, recovery, limitations, and release checks.
Then verify with the same identities:

```sh
python3 scripts/verify-one-dot-zero-candidate.py \
  --evidence /absolute/review/path/one-dot-zero-candidate.json \
  --version 1.0.0-rc.1 \
  --previous-revision 1111111111111111111111111111111111111111 \
  --revision "$(git rev-parse HEAD)"
```

Missing/extra/stale artifacts, repeated revisions, fewer than five stations,
any journey below 95%, a top-five crash, an unmet performance/recovery check,
stale limitations, or any release blocker fails closed. Canonical JSON cannot
replace signatures, raw evidence, elapsed tests, native hardware, installed
packages, rollback, recovery, or the reviewed release ledger.
