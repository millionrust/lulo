# Invited daily-driver Beta cohort

The Beta is available only after the contributor Alpha gates pass and the four
GOAL safety classes have zero open defects: data loss, privilege escalation,
session lockout, and critical accessibility. It is an invited test cohort, not
an unrestricted public daily-driver recommendation.

## Cohort design

The exact contract is `scripts/beta-candidate.json`:

- at least 20 consenting participants for at least 14 complete days;
- at least 280 participant-days, increasing with any participant count above
  20;
- at least eight participants on the amd64 Intel laptop class, eight on the
  amd64 AMD desktop class, and four on the amd64 NVIDIA desktop class;
- at least 20 attempts for every one of the ten product journeys with at least
  a 90% pass rate per journey; and
- all 15 promotion checks, three H8 Beta stations, release audits, signed
  artifacts/update rollback, recovery, limitations, and privacy review passing.

Participant identity is not part of the canonical evidence. Enrollment,
consent, withdrawal, contact, and raw reports live in the private cohort
system. The committed/reviewed summary contains only aggregate counts and
canonical results.

## Daily practice

Participants use synthetic or disposable nonessential data, retain the stock
Ubuntu/GNOME recovery session, install only the signed Beta artifact set, and
apply updates through the staged channel. Each day samples normal app/shell
use, the assigned product journeys, suspend/resume where supported, lock,
network/device changes, and recovery.

Stop the cohort immediately for suspected data loss, privilege escalation,
lockout, credential exposure, update-trust failure, or critical accessibility
failure. Security/privacy reports use the private policy; all other problems
use the structured issue form after redaction.

## Promotion evidence

Generate a pending summary:

```sh
python3 scripts/verify-beta-candidate.py \
  --print-template \
  --version 0.1.0-beta.1 \
  --revision "$(git rev-parse HEAD)" \
  > /absolute/review/path/beta-candidate.json
```

After the full cohort and gate review, verify:

```sh
python3 scripts/verify-beta-candidate.py \
  --evidence /absolute/review/path/beta-candidate.json \
  --version 0.1.0-beta.1 \
  --revision "$(git rev-parse HEAD)"
```

Missing participants/days/stations/journeys, a journey below 90%, a pending
check, any open safety defect, a release blocker, a stale source hash, or an
invalid version/revision fails closed. Aggregate JSON cannot replace consent,
elapsed cohort time, raw issue review, native station evidence, signed
artifacts, or update/recovery testing.
