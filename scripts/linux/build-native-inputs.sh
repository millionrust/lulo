#!/usr/bin/env bash
# SPDX-License-Identifier: MIT

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
target_dir="$repo_root/target"
lab_dir="$repo_root/shell"
lab_target_dir="$lab_dir/target"
minimum_kib=$((15 * 1024 * 1024))
build_minimum_kib=$((25 * 1024 * 1024))
cargo_jobs=${CARGO_BUILD_JOBS:-1}

usage() {
  echo "usage: $0 --output /absolute/new/directory" >&2
}

if [[ $# -ne 2 || "$1" != --output ]]; then
  usage
  exit 2
fi
output=$2

fail() {
  echo "native input build refused: $*" >&2
  exit 1
}

[[ "$cargo_jobs" =~ ^[1-9][0-9]*$ ]] \
  || fail "CARGO_BUILD_JOBS must be a positive decimal integer"

available_kib() {
  df -Pk "$repo_root" | awk 'NR == 2 { print $4 }'
}

require_space() {
  local required_kib=$1
  local phase=$2
  local available
  available="$(available_kib)"
  if [[ ! "$available" =~ ^[0-9]+$ ]] || (( available < required_kib )); then
    fail "$phase requires at least $((required_kib / 1024 / 1024)) GiB free"
  fi
}

[[ "$(uname -s)" == Linux ]] || fail "Linux is required"
[[ ${EUID} -ne 0 ]] || fail "run as the build user, not root"
command -v cargo >/dev/null 2>&1 || fail "cargo is required"
command -v dpkg >/dev/null 2>&1 || fail "dpkg is required"
command -v python3 >/dev/null 2>&1 || fail "python3 is required"
[[ "$output" == /* && "$output" != / ]] \
  || fail "output must be an absolute non-root path"
[[ ! -e "$output" && ! -L "$output" ]] || fail "output must not already exist"
output_parent="$(dirname "$output")"
[[ -d "$output_parent" && ! -L "$output_parent" ]] \
  || fail "output parent must be an existing ordinary directory"

architecture="$(dpkg --print-architecture)"
[[ "$architecture" == amd64 || "$architecture" == arm64 ]] \
  || fail "only native amd64 and arm64 builders are supported"
require_space "$build_minimum_kib" "release build"

inventory="$(python3 -I -c '
import sys
sys.path.insert(0, sys.argv[1])
from native_package_contract import ALL_BINARIES
print("\n".join(ALL_BINARIES))
' "$repo_root/scripts/linux")" || fail "native package inventory could not be loaded"
mapfile -t binary_names <<<"$inventory"
[[ ${#binary_names[@]} -eq 23 ]] || fail "native package inventory is not exact"

# Reuse the repository's one normal target graph even if the caller exports a
# different Cargo target directory.
export CARGO_TARGET_DIR="$target_dir"
(
  cd "$repo_root"
  cargo build --locked --release --jobs "$cargo_jobs" \
    -p rmac-app-drawer --bin rmac-app-drawer \
    -p rmac-finder --bin rmac-files \
    -p rmac-notes --bin rmac-notes \
    -p rmac-activity-monitor --bin rmac-system-monitor \
    -p rmac-system-settings --bin rmac-system-settings \
    -p rmac-terminal --bin rmac-terminal \
    -p rmac-text-editor --bin rmac-text-editor \
    -p rmac-session --bin rmac-session-supervisor \
    -p rmac-launcher-app --bin rmac-launcher \
    -p rmac-quick-settings-app --bin rmac-quick-settings \
    -p rmac-notification-center-app --bin rmac-notification-center-panel \
    -p rmac-notifications-linux --bin rmac-notification-center \
    -p rmac-focus-linux --bin rmac-focus-service \
    -p rmac-shortcuts \
      --bin rmac-shortcut-broker \
      --bin rmac-shortcut-dispatch \
      --bin rmac-locker \
      --bin rmac-lock-coordinator \
      --bin rmac-idle-locker \
    -p rmac-lock-provider-linux --features provider \
      --bin rmac-lock-provider
)
(
  cd "$lab_dir"
  CARGO_TARGET_DIR="$lab_target_dir" cargo build --locked --release \
    --jobs "$cargo_jobs" \
    --features wayland --bin wallpaper --bin top-bar --bin dock --bin osd
)

binary_source() {
  case "$1" in
    rmac-wallpaper) printf '%s\n' "$lab_target_dir/release/wallpaper" ;;
    rmac-top-bar) printf '%s\n' "$lab_target_dir/release/top-bar" ;;
    rmac-dock) printf '%s\n' "$lab_target_dir/release/dock" ;;
    rmac-osd) printf '%s\n' "$lab_target_dir/release/osd" ;;
    *) printf '%s\n' "$target_dir/release/$1" ;;
  esac
}

require_space "$minimum_kib" "completed release build"
total_kib=0
for name in "${binary_names[@]}"; do
  source_path="$(binary_source "$name")"
  [[ -f "$source_path" && ! -L "$source_path" && -x "$source_path" ]] \
    || fail "Cargo did not produce the required executable: $name"
  size="$(stat -c '%s' "$source_path")"
  [[ "$size" =~ ^[0-9]+$ && "$size" -gt 0 ]] \
    || fail "Cargo produced an invalid executable: $name"
  total_kib=$((total_kib + (size + 1023) / 1024))
done

available="$(available_kib)"
if (( available - total_kib < minimum_kib )); then
  fail "staging the exact binaries would cross the 15 GiB storage floor"
fi

staging="$(mktemp -d "$output_parent/.rmac-native-inputs.XXXXXX")"
trap 'rm -rf "${staging:-}"' EXIT HUP INT TERM
for name in "${binary_names[@]}"; do
  install -m 0755 "$(binary_source "$name")" "$staging/$name"
done

for name in "${binary_names[@]}"; do
  [[ -f "$staging/$name" && ! -L "$staging/$name" && -x "$staging/$name" ]] \
    || fail "staged executable is invalid: $name"
done
actual_count="$(find "$staging" -mindepth 1 -maxdepth 1 -type f | wc -l)"
[[ "$actual_count" -eq ${#binary_names[@]} ]] \
  || fail "staged binary inventory is not exact"

mv "$staging" "$output"
trap - EXIT HUP INT TERM
require_space "$minimum_kib" "completed native input staging"

echo "Prepared ${#binary_names[@]} native $architecture package inputs."
echo "Next: python3 scripts/linux/build-native-packages.py --binary-dir \"$output\" --output /absolute/empty/output --architecture $architecture"
