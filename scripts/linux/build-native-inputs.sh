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

profile=release

usage() {
  echo "usage: $0 --output /absolute/new/directory [--profile release|iterate] [--minimum-free-gib N]" >&2
}

output=
while [[ $# -gt 0 ]]; do
  case "$1" in
    --output)
      [[ $# -ge 2 ]] || { usage; exit 2; }
      output=$2
      shift 2
      ;;
    --profile)
      [[ $# -ge 2 ]] || { usage; exit 2; }
      profile=$2
      shift 2
      ;;
    --minimum-free-gib)
      [[ $# -ge 2 && "$2" =~ ^[1-9][0-9]*$ ]] || { usage; exit 2; }
      # CI's disposable runner has a smaller disk than the reference laptop.
      # The normal local floor stays 25 GiB unless explicitly overridden.
      build_minimum_kib=$(( $2 * 1024 * 1024 ))
      minimum_kib=$build_minimum_kib
      shift 2
      ;;
    *)
      usage
      exit 2
      ;;
  esac
done
[[ -n "$output" ]] || { usage; exit 2; }

fail() {
  echo "native input build refused: $*" >&2
  exit 1
}

# Cargo's output directory for a profile is the profile's own name (the
# built-in "release" profile is the only case matching its own name by
# coincidence). Only known profiles are accepted so a typo never silently
# reads stale binaries from the wrong target subdirectory.
case "$profile" in
  release|iterate) ;;
  *) fail "unsupported cargo profile: $profile (see [profile.iterate] in Cargo.toml)" ;;
esac

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
# [profile.iterate] (Cargo.toml) leaves `strip = false` so a plain
# `cargo build --profile iterate` keeps debug symbols for relinking and
# debugging on the reference PC. A candidate's *staged copy* is stripped
# below instead, right before packaging, so the shared target directory's
# binaries (and anyone debugging them directly) are never touched.
[[ "$profile" == release ]] || command -v strip >/dev/null 2>&1 \
  || fail "strip is required to stage a $profile candidate build"
[[ "$output" == /* && "$output" != / ]] \
  || fail "output must be an absolute non-root path"
[[ ! -e "$output" && ! -L "$output" ]] || fail "output must not already exist"
output_parent="$(dirname "$output")"
[[ -d "$output_parent" && ! -L "$output_parent" ]] \
  || fail "output parent must be an existing ordinary directory"

architecture="$(dpkg --print-architecture)"
[[ "$architecture" == amd64 || "$architecture" == arm64 ]] \
  || fail "only native amd64 and arm64 builders are supported"
require_space "$build_minimum_kib" "$profile build"

inventory="$(python3 -I -c '
import sys
sys.path.insert(0, sys.argv[1])
from native_package_contract import ALL_BINARIES
print("\n".join(ALL_BINARIES))
' "$repo_root/scripts/linux")" || fail "native package inventory could not be loaded"
mapfile -t binary_names <<<"$inventory"
# The contract's tests pin the exact list; here it only has to be non-empty
# and free of duplicates, so adding a program never needs a count edit.
[[ ${#binary_names[@]} -gt 0 ]] || fail "native package inventory is empty"
[[ "$(printf '%s\n' "${binary_names[@]}" | sort -u | wc -l)" -eq ${#binary_names[@]} ]] \
  || fail "native package inventory has duplicates"

# Reuse the repository's one normal target graph even if the caller exports a
# different Cargo target directory.
export CARGO_TARGET_DIR="$target_dir"
# Keep compiler diagnostics and panic locations independent of the builder's
# checkout and Cargo cache paths. Cargo's encoded form takes precedence over
# RUSTFLAGS, so extend whichever form the caller already supplied.
cargo_home="${CARGO_HOME:-$HOME/.cargo}"
if [[ ${CARGO_ENCODED_RUSTFLAGS+x} ]]; then
  unit_separator=$'\x1f'
  export CARGO_ENCODED_RUSTFLAGS="${CARGO_ENCODED_RUSTFLAGS:+${CARGO_ENCODED_RUSTFLAGS}${unit_separator}}--remap-path-prefix=$repo_root=/rmac${unit_separator}--remap-path-prefix=$cargo_home=/cargo"
else
  export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }--remap-path-prefix=$repo_root=/rmac --remap-path-prefix=$cargo_home=/cargo"
fi
(
  cd "$repo_root"
  cargo build --locked --profile "$profile" --jobs "$cargo_jobs" \
    -p rmac-app-drawer --bin rmac-app-drawer \
    -p rmac-archive-utility --bin rmac-archive-utility \
    -p rmac-calculator --bin rmac-calculator \
    -p rmac-calendar --bin rmac-calendar \
    -p rmac-mail --bin rmac-mail \
    -p rmac-clock --bin rmac-clock \
    -p rmac-weather --bin rmac-weather \
    -p rmac-player --bin rmac-player \
    -p rmac-finder --bin rmac-files \
    -p rmac-notes --bin rmac-notes \
    -p rmac-preview --bin rmac-preview \
    -p rmac-activity-monitor --bin rmac-system-monitor \
    -p rmac-system-settings --bin rmac-system-settings \
    -p rmac-terminal --bin rmac-terminal \
    -p rmac-text-editor --bin rmac-text-editor \
    -p rmac-session --bin rmac-session-supervisor \
    -p rmac-launcher-app --bin rmac-launcher \
    -p rmac-quick-settings-app --bin rmac-quick-settings \
    -p rmac-notification-center-app --bin rmac-notification-center-panel \
    -p rmac-notification-center-app --bin rmac-notification-center \
    -p rmac-focus-linux --bin rmac-focus-service \
    -p rmac-clipboard-linux --bin rmac-clipboard-service \
    -p rmac-file-chooser --bin rmac-file-chooser \
    -p rmac-shortcuts \
      --bin rmac-shortcut-broker \
      --bin rmac-shortcut-dispatch \
      --bin rmac-locker \
      --bin rmac-lock-coordinator \
      --bin rmac-idle-locker \
    -p rmac-lock-provider-linux --features provider \
      --bin rmac-lock-provider \
    -p rmac-sound --bin rmac-sound \
    -p rmac-media --bin rmac-media \
    -p rmac-keyboard --bin rmac-mac-keyboard \
    -p rmac-setup-assistant --bin rmac-setup-assistant
)
(
  cd "$lab_dir"
  CARGO_TARGET_DIR="$lab_target_dir" cargo build --locked --profile "$profile" \
    --jobs "$cargo_jobs" \
    --features wayland --bin wallpaper --bin top-bar --bin dock --bin osd \
    --bin app-switcher --bin screenshot --bin mission-control
)

binary_source() {
  case "$1" in
    rmac-wallpaper) printf '%s\n' "$lab_target_dir/$profile/wallpaper" ;;
    rmac-top-bar) printf '%s\n' "$lab_target_dir/$profile/top-bar" ;;
    rmac-dock) printf '%s\n' "$lab_target_dir/$profile/dock" ;;
    rmac-osd) printf '%s\n' "$lab_target_dir/$profile/osd" ;;
    rmac-app-switcher) printf '%s\n' "$lab_target_dir/$profile/app-switcher" ;;
    rmac-screenshot) printf '%s\n' "$lab_target_dir/$profile/screenshot" ;;
    rmac-mission-control) printf '%s\n' "$lab_target_dir/$profile/mission-control" ;;
    *) printf '%s\n' "$target_dir/$profile/$1" ;;
  esac
}

require_space "$minimum_kib" "completed $profile build"
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
  # Strip only the staged copy (never the shared target directory's own
  # binary), so a candidate's installed package is close to release size
  # even though [profile.iterate] itself keeps debug symbols for ordinary
  # relinking and debugging.
  [[ "$profile" == release ]] || strip --strip-all "$staging/$name"
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

echo "Prepared ${#binary_names[@]} native $architecture package inputs ($profile profile)."
if [[ "$profile" == release ]]; then
  echo "Next: python3 scripts/linux/build-native-packages.py --binary-dir \"$output\" --output /absolute/empty/output --architecture $architecture"
else
  echo "Next (candidate build, not for Beta/stable release): python3 scripts/linux/build-native-packages.py --binary-dir \"$output\" --output /absolute/empty/output --architecture $architecture --build-metadata $profile"
fi
