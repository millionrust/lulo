#!/usr/bin/env bash
set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
evidence_dir="$repo_root/target/linux-evidence/$timestamp"
run_upstream_smoke=false
run_performance=false
preflight_only=false
expected_desktop=gnome
minimum_kib=$((15 * 1024 * 1024))
build_minimum_kib=$((25 * 1024 * 1024))
storage_blocked=false

usage() {
  echo "usage: $0 [--session gnome|niri|any] [--preflight-only] [--with-upstream-smoke] [--with-performance]" >&2
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --with-upstream-smoke) run_upstream_smoke=true ;;
    --with-performance) run_performance=true ;;
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

if ! command -v python3 >/dev/null 2>&1; then
  echo "reference preflight requires python3" >&2
  exit 2
fi

preflight_args=(
  --repo-root "$repo_root"
  --expected-desktop "$expected_desktop"
  --minimum-free-gib 25
  --require-command bash
  --require-command busctl
  --require-command cargo
  --require-command cargo-deny
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

if [[ "$preflight_only" == true ]]; then
  exec python3 "$repo_root/scripts/linux/reference-preflight.py" "${preflight_args[@]}"
fi

mkdir -p "$evidence_dir"
if ! python3 "$repo_root/scripts/linux/reference-preflight.py" "${preflight_args[@]}" \
  2>&1 | tee "$evidence_dir/preflight.log"; then
  echo "reference gates stopped before evidence collection" >&2
  exit 3
fi
bash "$repo_root/scripts/linux/collect-reference-evidence.sh" "$evidence_dir"

failed=0
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
    return
  fi
  local available
  available=$(available_kib)
  if (( available < required_kib )); then
    echo "$name stopped: ${available} KiB free; ${required_kib} KiB required" \
      | tee "$evidence_dir/$name.log" >&2
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
}

cd "$repo_root"
run_gate format "$minimum_kib" cargo fmt --all -- --check
run_gate harness-tests "$minimum_kib" \
  /usr/bin/python3 -m unittest \
  scripts/test_measure_baseline.py \
  scripts/test_application_icons.py \
  scripts/test_reference_preflight.py \
  scripts/test_session_package.py \
  experiments/gpui-upstream-lab/scripts/test_a4_report.py
run_gate shared-controls "$minimum_kib" bash scripts/check-shared-controls.sh
run_gate clippy "$build_minimum_kib" \
  cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
run_gate tests "$build_minimum_kib" cargo test --locked --workspace --all-features
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

echo
if [[ $failed -eq 0 ]]; then
  echo "all requested Linux reference gates passed; evidence: $evidence_dir"
else
  echo "one or more Linux reference gates failed; evidence: $evidence_dir" >&2
fi
exit "$failed"
