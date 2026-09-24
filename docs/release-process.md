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
- generates an SBOM with `cargo-cyclonedx` (version pinned in
  `release.yml`'s `RMAC_CARGO_CYCLONEDX_VERSION`);
- attests build provenance with `actions/attest-build-provenance`; and
- attaches the `.deb` files, `SHA256SUMS`, and the SBOM to the GitHub
  Release, creating it if needed.

None of that needs a secret.

Signing and publishing the APT repository -- `stage-apt-snapshot.py`,
clearsigning `InRelease`, `publish-apt-snapshot.py`, and
`actions/deploy-pages` -- is fully wired in `release.yml`'s
`apt-repository` job and in `rollout.yml`, but both stay off
(`vars.RMAC_SOURCE_PACKAGING_READY != 'true'`) until the two gaps in
"What the owner still has to do" are closed. Building the keyring packages
(`keyring` job in `release.yml`) is separately gated on
`secrets.RMAC_ARCHIVE_PUBLIC_KEYRING_B64` existing.

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
4. Once `attach-release` finishes, check the Release page: `.deb` files,
   `SHA256SUMS`, the SBOM, and a provenance attestation should all be
   attached.

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
