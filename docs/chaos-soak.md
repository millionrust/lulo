# Chaos and soak release gate

I5 is the recovery and longevity gate for the complete rmac session. The exact
contract is `scripts/chaos-soak.json`. It binds 14 fault scenarios, two real
soaks, all ten product journeys, the H8 station inventory, and the exact
candidate revision into 136 reviewed results.

The fault pass covers desktop-service restarts, niri restart, individual shell
and supervisor crashes, isolated low disk, malformed niri and rmac
configuration, mount disappearance, display and peripheral hotplug,
suspend/resume, and PackageKit update backend, network, and interrupted-install
failures. Every injection must show a truthful degraded state, bounded
operation, safe retained/empty state, authoritative recovery, no crash loop,
no data loss, and privacy-safe diagnostics.

Validate the contract without building Rust:

```sh
python3 scripts/verify-chaos-soak.py
```

## Safety boundary

Never perform this gate on a machine or account containing the user's real
data. Snapshot the disposable station first and use synthetic documents,
accounts, notifications, networks, mounts, and updates. Update installation
interruption is allowed only on a disposable station with tested recovery.

The repository/data volume must retain at least 15 GiB free at all times. The
low-disk scenario must use a separate disposable filesystem with an explicit
size limit; it must never fill the host, repository, home directory, Cargo
cache, or normal `target` directory. Stop immediately if the host approaches
the floor. Do not use Docker volumes or duplicate the repository.

Malformed-config scenarios edit copies or the project's atomic managed file
through its supported mutation boundary. Keep a known-good backup and prove
validation/refusal or exact rollback. Do not corrupt the user's live niri
configuration.

## Evidence procedure

Use the full packaged Ubuntu 26.04 rmac/niri session at a clean revision. Run
the 14 fault scenarios once before each soak and repeat relevant faults during
the run. At every checkpoint record only elapsed duration, component
active/degraded state, bounded restart counts, aggregate memory/wakeup/log
figures, and canonical scenario outcomes. Keep raw journal, trace, device,
process, path, network, and content data in ignored local evidence.

The eight-hour run uses checkpoints at least every five minutes, for at least
96 checkpoints. The seven-day run uses checkpoints at least every 15 minutes,
for at least 672 checkpoints. A missed interval may be explained in raw
evidence, but the minimum checkpoint total and wall duration remain mandatory.
After each run, restart the session cleanly and execute all ten product
journeys. Passing requires no crash loop, lost data, stuck operation, missing
surface, unbounded log growth, or I4 idle/memory regression.

Generate one pending result file per required H8 station outside the
repository:

```sh
python3 scripts/verify-chaos-soak.py \
  --print-template \
  --station amd64-intel-laptop \
  --revision "$(git rev-parse HEAD)" \
  > /absolute/review/path/amd64-intel-laptop.json
```

Change results and both run records to `pass` only after reviewing their raw
evidence. Verify the exact release-tier directory:

```sh
python3 scripts/verify-chaos-soak.py \
  --evidence-dir /absolute/review/path \
  --tier alpha \
  --revision "$(git rev-parse HEAD)"
```

Missing, extra, stale, reordered, skipped, blocked, short-duration, or
under-sampled evidence fails closed. The canonical JSON proves only that the
reviewed inventory is complete; it cannot replace the native fault injection,
elapsed wall time, recovery observation, or raw evidence.
