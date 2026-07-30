# Contributor Alpha build

The Alpha is a build for contributors on disposable supported test machines.
It is not a daily-driver recommendation, general support promise, or permission
to install on a primary account. The exact publish contract is
`scripts/alpha-candidate.json`.

## Candidate contents

One candidate revision produces a source archive, matching `rmac-apps` and
`rmac-session` amd64 Debian packages, SHA-256 inventory, SPDX JSON SBOM, and
bounded build provenance. Artifacts must come from the clean reproducible build
and be verified before signing or distribution; files from different revisions
must never be combined.

The Alpha runs only on the amd64 Intel laptop and amd64 AMD desktop H8 classes.
Stock Ubuntu/GNOME remains installed and tested as the recovery session.

## Safety floor

The candidate cannot publish with a known data-loss, privilege-escalation, or
session-lockout defect. The framework accessibility/layer-shell evidence,
broader hardware coverage, general support, and signed public APT channel may
remain explicit Alpha limitations only when they do not cross that safety
floor.

All 18 candidate checks must pass: clean/reproducible source, dependency
policy, package/signature/checksum/SBOM integrity, clean install and session
login/logout, stock-GNOME recovery, upgrade/rollback/uninstall, both Alpha H8
stations, ten journeys, accessibility status disclosure, reviewed limitations
and release notes, privacy-safe issue intake, and zero open safety defects.

## Preparing evidence

Generate the pending summary outside the repository:

```sh
python3 scripts/verify-alpha-candidate.py \
  --print-template \
  --version 0.1.0-alpha.1 \
  --revision "$(git rev-parse HEAD)" \
  > /absolute/review/path/alpha-candidate.json
```

Record each canonical artifact's SHA-256 and byte size after independent
verification. Change a check/station to `pass` and a limitation to `disclosed`
only after its evidence is reviewed. Verify the final summary:

```sh
python3 scripts/verify-alpha-candidate.py \
  --evidence /absolute/review/path/alpha-candidate.json \
  --version 0.1.0-alpha.1 \
  --revision "$(git rev-parse HEAD)"
```

The summary requires zero release blockers. A syntactically valid summary does
not replace signatures, packages, clean-machine runs, raw evidence, or human
review.

## Reporting problems

Use the structured **Alpha bug report** issue form. It requests the exact
revision, public H8 class, GPU/driver family, niri/session, scale/refresh,
portal context, affected journey, reproduction, recovery, safety impact, and
an optional minimal redacted log excerpt. It explicitly rejects personal
machine dumps.

Report suspected security, privacy, privilege, credential, update-trust, or
lock-boundary problems privately through the repository Security tab, following
[the security policy](../SECURITY.md).
