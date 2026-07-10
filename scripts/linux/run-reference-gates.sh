#!/usr/bin/env bash
set -uo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
evidence_dir="$repo_root/target/linux-evidence/$timestamp"
run_upstream_smoke=false
run_performance=false

usage() {
  echo "usage: $0 [--with-upstream-smoke] [--with-performance]" >&2
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --with-upstream-smoke) run_upstream_smoke=true ;;
    --with-performance) run_performance=true ;;
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

mkdir -p "$evidence_dir"
bash "$repo_root/scripts/linux/collect-reference-evidence.sh" "$evidence_dir"

failed=0
run_gate() {
  local name=$1
  shift
  echo
  echo "==> $name"
  "$@" 2>&1 | tee "$evidence_dir/$name.log"
  local status=${PIPESTATUS[0]}
  if [[ $status -ne 0 ]]; then
    echo "$name failed with status $status" >&2
    failed=1
  fi
}

cd "$repo_root"
run_gate format cargo fmt --all -- --check
run_gate clippy cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
run_gate tests cargo test --locked --workspace --all-features
if command -v cargo-deny >/dev/null 2>&1; then
  run_gate dependency-policy cargo deny --locked --log-level error check --hide-inclusion-graph
else
  echo "cargo-deny is not installed; see docs/linux-reference-bringup.md" | tee "$evidence_dir/dependency-policy.log" >&2
  failed=1
fi

if [[ "$run_upstream_smoke" == true ]]; then
  run_gate upstream-wayland-smoke dbus-run-session -- \
    bash experiments/gpui-upstream-lab/scripts/nested-wayland-smoke.sh
fi

if [[ "$run_performance" == true ]]; then
  run_gate performance python3 scripts/measure-baseline.py \
    --output "$evidence_dir/performance.json"
fi

echo
if [[ $failed -eq 0 ]]; then
  echo "all requested Linux reference gates passed; evidence: $evidence_dir"
else
  echo "one or more Linux reference gates failed; evidence: $evidence_dir" >&2
fi
exit "$failed"
