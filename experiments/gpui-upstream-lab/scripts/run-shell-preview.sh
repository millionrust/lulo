#!/usr/bin/env bash
set -euo pipefail

lab_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target_dir="${CARGO_TARGET_DIR:-$lab_dir/target}"
if [[ "$target_dir" != /* ]]; then
  target_dir="$lab_dir/$target_dir"
fi

build=true
if [[ "${1:-}" == "--no-build" ]]; then
  build=false
  shift
fi
if [[ $# -ne 0 ]]; then
  echo "usage: scripts/run-shell-preview.sh [--no-build]" >&2
  exit 2
fi

if [[ "$(uname -s)" != "Linux" || "${XDG_SESSION_TYPE:-}" != "wayland" ]]; then
  echo "the rmac shell preview requires a Linux Wayland session" >&2
  exit 2
fi
if [[ -z "${WAYLAND_DISPLAY:-}" || -z "${XDG_RUNTIME_DIR:-}" ]]; then
  echo "WAYLAND_DISPLAY and XDG_RUNTIME_DIR must come from the active desktop session" >&2
  exit 2
fi

available_kib() {
  df -Pk "$lab_dir" | awk 'NR == 2 { print $4 }'
}

require_free_gib() {
  local phase=$1
  local required_gib=$2
  local required_kib=$((required_gib * 1024 * 1024))
  if (( $(available_kib) < required_kib )); then
    echo "$phase requires at least ${required_gib} GiB free" >&2
    exit 1
  fi
}

cd "$lab_dir"
if [[ "$build" == true ]]; then
  require_free_gib "building the rmac shell preview" 25
  cargo build --locked --jobs "${CARGO_BUILD_JOBS:-2}" --features wayland \
    --bin wallpaper --bin top-bar --bin dock
  require_free_gib "launching the rmac shell preview" 15
fi

for binary in wallpaper top-bar dock; do
  if [[ ! -x "$target_dir/debug/$binary" ]]; then
    echo "$target_dir/debug/$binary is missing; rerun without --no-build" >&2
    exit 1
  fi
done

declare -a component_names=(wallpaper top-bar dock)
declare -a component_pids=()

cleanup() {
  local status=$?
  trap - EXIT INT TERM
  for pid in "${component_pids[@]:-}"; do
    if kill -0 "$pid" 2>/dev/null; then
      kill "$pid" 2>/dev/null || true
    fi
  done
  for pid in "${component_pids[@]:-}"; do
    wait "$pid" 2>/dev/null || true
  done
  exit "$status"
}
trap cleanup EXIT INT TERM

for component in "${component_names[@]}"; do
  "$target_dir/debug/$component" &
  component_pids+=("$!")
done

sleep 1
for index in "${!component_pids[@]}"; do
  if ! kill -0 "${component_pids[$index]}" 2>/dev/null; then
    wait "${component_pids[$index]}" || status=$?
    echo "${component_names[$index]} exited during rmac shell startup" >&2
    exit "${status:-1}"
  fi
done

echo "rmac shell preview is running: wallpaper + menu bar + Dock"
echo "Press Ctrl+C in this terminal to stop it."

while sleep 1; do
  for index in "${!component_pids[@]}"; do
    if ! kill -0 "${component_pids[$index]}" 2>/dev/null; then
      wait "${component_pids[$index]}" || status=$?
      echo "${component_names[$index]} stopped; closing the shell preview" >&2
      exit "${status:-1}"
    fi
  done
done
