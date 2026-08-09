#!/usr/bin/env bash
set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
evidence_dir="$repo_root/target/linux-evidence/$timestamp"
run_upstream_smoke=false
run_performance=false
run_resilience=false
preflight_only=false
expected_desktop=gnome
minimum_kib=$((15 * 1024 * 1024))
build_minimum_kib=$((25 * 1024 * 1024))
storage_blocked=false
summary_file=""

usage() {
  echo "usage: $0 [--session gnome|niri|any] [--preflight-only] [--with-upstream-smoke] [--with-performance] [--with-resilience]" >&2
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --with-upstream-smoke) run_upstream_smoke=true ;;
    --with-performance) run_performance=true ;;
    --with-resilience) run_resilience=true ;;
    --preflight-only) preflight_only=true ;;
    --session)
      shift
      if [[ $# -eq 0 ]] || [[ ! "$1" =~ ^(gnome|niri|any)$ ]]; then
        usage
        exit 2
      fi
      expected_desktop=$1
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      usage
      exit 2
      ;;
  esac
  shift
done

if [[ "$run_resilience" == true && "$expected_desktop" != niri ]]; then
  echo "--with-resilience requires --session niri" >&2
  exit 2
fi

if ! command -v python3 >/dev/null 2>&1; then
  echo "reference preflight requires python3" >&2
  exit 2
fi

preflight_args=(
  --repo-root "$repo_root"
  --expected-desktop "$expected_desktop"
  --minimum-free-gib 25
  --require-command appstreamcli
  --require-command bash
  --require-command busctl
  --require-command cargo
  --require-command cargo-deny
  --require-command desktop-file-validate
  --require-command git
  --require-command python3
  --require-command rustc
  --require-command systemctl
  --require-command timeout
  --require-command vulkaninfo
  --require-command wayland-info
)
if [[ "$expected_desktop" == niri ]]; then
  preflight_args+=(--require-command niri)
fi
if [[ "$run_upstream_smoke" == true ]]; then
  preflight_args+=(
    --require-command dbus-run-session
    --require-command jq
    --require-command sway
  )
fi
if [[ "$run_resilience" == true ]]; then
  preflight_args+=(--require-command jq)
fi

if [[ "$preflight_only" == true ]]; then
  exec python3 "$repo_root/scripts/linux/reference-preflight.py" "${preflight_args[@]}"
fi

mkdir -p "$evidence_dir"
summary_file="$evidence_dir/gate-summary.tsv"
{
  printf 'schema\t1\n'
  printf 'git_commit\t%s\n' "$(git -C "$repo_root" rev-parse HEAD)"
  printf 'expected_desktop\t%s\n' "$expected_desktop"
  printf 'upstream_smoke_requested\t%s\n' "$run_upstream_smoke"
  printf 'performance_requested\t%s\n' "$run_performance"
  printf 'resilience_requested\t%s\n' "$run_resilience"
  printf 'gate\tstatus\texit_status\n'
} >"$summary_file"

record_gate() {
  local name=$1
  local status=$2
  local exit_status=$3
  printf '%s\t%s\t%s\n' "$name" "$status" "$exit_status" >>"$summary_file"
}

if ! python3 "$repo_root/scripts/linux/reference-preflight.py" "${preflight_args[@]}" \
  2>&1 | tee "$evidence_dir/preflight.log"; then
  record_gate preflight fail "${PIPESTATUS[0]}"
  printf 'overall\tfail\n' >>"$summary_file"
  echo "reference gates stopped before evidence collection" >&2
  echo "reviewable summary: $summary_file" >&2
  exit 3
fi
record_gate preflight pass 0

failed=0
if bash "$repo_root/scripts/linux/collect-reference-evidence.sh" "$evidence_dir"; then
  record_gate evidence-collection pass 0
else
  evidence_status=$?
  record_gate evidence-collection fail "$evidence_status"
  failed=1
fi

available_kib() {
  df -Pk "$repo_root" | awk 'NR == 2 { print $4 }'
}

run_gate() {
  local name=$1
  local required_kib=$2
  shift
  shift
  if [[ "$storage_blocked" == true ]]; then
    echo "$name skipped because the storage floor was reached" \
      | tee "$evidence_dir/$name.log" >&2
    record_gate "$name" skipped-storage "-"
    return
  fi
  local available
  available=$(available_kib)
  if (( available < required_kib )); then
    echo "$name stopped: ${available} KiB free; ${required_kib} KiB required" \
      | tee "$evidence_dir/$name.log" >&2
    record_gate "$name" fail-storage "-"
    failed=1
    storage_blocked=true
    return
  fi
  echo
  echo "==> $name"
  "$@" 2>&1 | tee "$evidence_dir/$name.log"
  local status=${PIPESTATUS[0]}
  if [[ $status -ne 0 ]]; then
    echo "$name failed with status $status" >&2
    failed=1
  fi
  available=$(available_kib)
  if (( available < minimum_kib )); then
    echo "storage floor reached after $name: ${available} KiB free; ${minimum_kib} KiB required" \
      | tee -a "$evidence_dir/$name.log" >&2
    failed=1
    storage_blocked=true
  fi
  if [[ $status -ne 0 ]]; then
    record_gate "$name" fail "$status"
  elif [[ "$storage_blocked" == true ]]; then
    record_gate "$name" fail-storage 0
  else
    record_gate "$name" pass 0
  fi
}

cd "$repo_root"
run_gate format "$minimum_kib" cargo fmt --all -- --check
run_gate desktop-metadata "$minimum_kib" \
  bash scripts/linux/check-desktop-metadata.sh
run_gate release-contracts "$minimum_kib" \
  /usr/bin/python3 scripts/run-release-contract-checks.py
run_gate shared-controls "$minimum_kib" bash scripts/check-shared-controls.sh
run_gate clippy "$build_minimum_kib" \
  cargo clippy --locked --workspace --lib --bins --tests -- -D warnings
run_gate product-journeys "$build_minimum_kib" \
  /usr/bin/python3 scripts/run-journey-suite.py \
  --output "$evidence_dir/journey-results.json" --fail-fast
run_gate dependency-policy "$minimum_kib" \
  cargo deny --locked --log-level error check --hide-inclusion-graph

if [[ "$run_upstream_smoke" == true ]]; then
  run_gate upstream-wayland-smoke "$build_minimum_kib" dbus-run-session -- \
    bash experiments/gpui-upstream-lab/scripts/nested-wayland-smoke.sh
fi

if [[ "$run_performance" == true ]]; then
  run_gate performance "$build_minimum_kib" python3 scripts/measure-baseline.py \
    --output "$evidence_dir/performance.json"
fi

if [[ "$run_resilience" == true ]]; then
  run_gate live-shell-resilience "$minimum_kib" \
    bash scripts/linux/run-live-shell-resilience.sh --execute
fi

echo
if [[ $failed -eq 0 ]]; then
  printf 'overall\tpass\n' >>"$summary_file"
  echo "all requested Linux reference gates passed; evidence: $evidence_dir"
else
  printf 'overall\tfail\n' >>"$summary_file"
  echo "one or more Linux reference gates failed; evidence: $evidence_dir" >&2
fi
echo "reviewable summary: $summary_file"
exit "$failed"
