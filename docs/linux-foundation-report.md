# Linux foundation evidence bundle

A1–A4 produce several kinds of evidence: environment/preflight captures, two
stable GPUI reports, session/portal/service observations, soak traces, and the
23-result upstream report. `scripts/verify-foundation-evidence.py` turns their
reviewed conclusions into one privacy-safe, revision-bound input for A5.

The bundle contains exactly:

- `gnome-platform-lab.txt` — all ten exercisable stable probes pass and both
  known stable API blockers are confirmed;
- `niri-platform-lab.txt` — the same exact report on the same architecture;
- `a4-upstream-report.txt` — all 23 pinned-upstream results pass; and
- `foundation-summary.json` — 16 A1/A3 checks, both four-hour durations, exact
  report hashes, product/upstream revisions, and five hashes representing the
  separately reviewed supporting evidence.

No hostname, username, session ID, socket, bus address/peer, output identity,
device model/serial/address, path, environment value, typed/clipboard/document
content, screenshot, recording, log body, or raw report is added to the
summary.

## Assemble on the reference PC

Run the GNOME and niri procedures in
[Linux reference bring-up](linux-reference-bringup.md), copy the platform-lab
reports into an absolute ignored evidence directory, and complete the upstream
A4 report. The two stable reports must say `os=linux`, use the same supported
architecture, pass all ten probes, and confirm—not pass—the two missing stable
API capabilities.

Create the pending summary after the three report files exist:

```sh
bundle="$PWD/target/linux-evidence/foundation-candidate"
python3 scripts/verify-foundation-evidence.py \
  --print-summary \
  --bundle-dir "$bundle" \
  --revision "$(git rev-parse HEAD)" \
  > "$bundle/foundation-summary.json"
```

Review the raw GNOME, stable-lab, niri, and upstream evidence separately.
Record their bounded archive/directory SHA-256 values under the five exact
`supporting_evidence_sha256` keys. Change A1/A3 results to `pass` only after
review, and record actual elapsed seconds for both soaks.

Verify the exact bundle:

```sh
python3 scripts/verify-foundation-evidence.py \
  --bundle-dir "$bundle" \
  --revision "$(git rev-parse HEAD)"
```

Missing/extra/linked files, wrong OS/architecture/revision, failed or pending
probe, unconfirmed stable blocker, incomplete A4 observation, altered report,
short soak, missing supporting hash, or relaxed A1/A3 result fails closed.

The bundle is an A5 input, not the framework decision. Hashes do not prove that
observations are true; maintainers still review the underlying evidence and
choose one path allowed by ADR 0001. Product migration remains forbidden until
that decision is committed.
