#!/usr/bin/env bash
# SPDX-License-Identifier: MIT

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
minimum_kib=$((15 * 1024 * 1024))
build_minimum_kib=$((25 * 1024 * 1024))

usage() {
  echo "usage: $0 --binary-dir /absolute/inputs --output /absolute/new/directory --architecture amd64|arm64 --source-date-epoch SECONDS" >&2
}

binary_dir=""
output=""
architecture=""
epoch=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --binary-dir|--output|--architecture|--source-date-epoch)
      option=$1
      shift
      [[ $# -gt 0 ]] || { usage; exit 2; }
      case "$option" in
        --binary-dir) binary_dir=$1 ;;
        --output) output=$1 ;;
        --architecture) architecture=$1 ;;
        --source-date-epoch) epoch=$1 ;;
      esac
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

fail() {
  echo "native reproducibility check refused: $*" >&2
  exit 1
}

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
[[ ${EUID} -ne 0 ]] || fail "run as the package builder, not root"
command -v cmp >/dev/null 2>&1 || fail "cmp is required"
command -v git >/dev/null 2>&1 || fail "git is required"
command -v python3 >/dev/null 2>&1 || fail "python3 is required"
command -v sha256sum >/dev/null 2>&1 || fail "sha256sum is required"
[[ "$architecture" == amd64 || "$architecture" == arm64 ]] \
  || fail "architecture must be amd64 or arm64"
[[ "$epoch" =~ ^(0|[1-9][0-9]*)$ ]] \
  || fail "source date epoch must be canonical decimal seconds"
[[ "$binary_dir" == /* && -d "$binary_dir" && ! -L "$binary_dir" ]] \
  || fail "binary directory must be an absolute ordinary directory"
[[ "$output" == /* && "$output" != / ]] \
  || fail "output must be an absolute non-root path"
[[ ! -e "$output" && ! -L "$output" ]] || fail "output must not already exist"
output_parent="$(dirname "$output")"
[[ -d "$output_parent" && ! -L "$output_parent" ]] \
  || fail "output parent must be an existing ordinary directory"
git -C "$repo_root" diff --quiet --ignore-submodules -- \
  || fail "tracked worktree changes make the result non-reproducible"
git -C "$repo_root" diff --cached --quiet --ignore-submodules -- \
  || fail "staged changes make the result non-reproducible"
require_space "$build_minimum_kib" "two-build package check"

staging="$(mktemp -d "$output_parent/.rmac-native-reproducibility.XXXXXX")"
trap 'rm -rf "${staging:-}"' EXIT HUP INT TERM
for run in run-a run-b; do
  SOURCE_DATE_EPOCH="$epoch" \
    python3 "$repo_root/scripts/linux/build-native-packages.py" \
      --binary-dir "$binary_dir" \
      --output "$staging/$run" \
      --architecture "$architecture"
  python3 "$repo_root/scripts/linux/verify-native-packages.py" \
    --directory "$staging/$run" \
    --architecture "$architecture"
  require_space "$minimum_kib" "$run"
  if [[ "$run" == run-a ]]; then
    first_run_kib="$(du -sk "$staging/run-a" | awk '{ print $1 }')"
    available="$(available_kib)"
    if [[ ! "$first_run_kib" =~ ^[0-9]+$ ]] \
      || (( available - first_run_kib < minimum_kib )); then
      fail "the second package set would cross the 15 GiB storage floor"
    fi
  fi
done

mapfile -t first_files < <(
  find "$staging/run-a" -mindepth 1 -maxdepth 1 -type f -printf '%f\n' \
    | LC_ALL=C sort
)
mapfile -t second_files < <(
  find "$staging/run-b" -mindepth 1 -maxdepth 1 -type f -printf '%f\n' \
    | LC_ALL=C sort
)
[[ ${#first_files[@]} -eq 4 && "${first_files[*]}" == "${second_files[*]}" ]] \
  || fail "the two package result inventories differ"
for name in "${first_files[@]}"; do
  cmp --silent "$staging/run-a/$name" "$staging/run-b/$name" \
    || fail "the repeated package result differs: $name"
done

summary="$staging/reproducibility.tsv"
{
  printf 'schema\t1\n'
  printf 'git_commit\t%s\n' "$(git -C "$repo_root" rev-parse HEAD)"
  printf 'architecture\t%s\n' "$architecture"
  printf 'source_date_epoch\t%s\n' "$epoch"
  for name in "${first_files[@]}"; do
    read -r digest _ < <(sha256sum "$staging/run-a/$name")
    printf 'artifact\t%s\t%s\n' "$name" "$digest"
  done
  printf 'overall\tpass\n'
} >"$summary"

mv "$staging" "$output"
trap - EXIT HUP INT TERM
require_space "$minimum_kib" "completed reproducibility check"
echo "Verified two byte-identical $architecture package builds in $output."
