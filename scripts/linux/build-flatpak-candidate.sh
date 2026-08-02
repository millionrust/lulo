#!/usr/bin/env bash
# SPDX-License-Identifier: MIT

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
manifest="$repo_root/packaging/flatpak/org.rmac.TextEditor.json"
minimum_kib=$((15 * 1024 * 1024))
build_minimum_kib=$((25 * 1024 * 1024))

usage() {
  cat >&2 <<'EOF'
usage: scripts/linux/build-flatpak-candidate.sh --prepare-online|--build-offline

  --prepare-online  Install required user runtimes and cache every source.
  --build-offline   Refuse downloads, build/export, and write bounded evidence.
EOF
}

case "${1:-}" in
  --prepare-online) mode=prepare ;;
  --build-offline) mode=offline ;;
  -h|--help)
    usage
    exit 0
    ;;
  *)
    usage
    exit 2
    ;;
esac
[[ $# -eq 1 ]] || { usage; exit 2; }

fail() {
  echo "Flatpak candidate build refused: $*" >&2
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
[[ ${EUID} -ne 0 ]] || fail "run as the graphical build user, not root"
for command_name in flatpak flatpak-builder git python3 sha256sum; do
  command -v "$command_name" >/dev/null 2>&1 \
    || fail "$command_name is required"
done
case "$(uname -m)" in
  x86_64) architecture=x86_64 ;;
  aarch64) architecture=aarch64 ;;
  *) fail "only native x86_64 and aarch64 builders are supported" ;;
esac

work_root="$repo_root/target/flatpak-candidate/$architecture"
state_dir="$work_root/state"
build_dir="$work_root/build"
result_dir="$work_root/result"
preparation="$work_root/online-preparation.tsv"

[[ ! -L "$repo_root/target" ]] || fail "the target directory must not be linked"
mkdir -p "$work_root"
[[ -d "$work_root" && ! -L "$work_root" ]] \
  || fail "candidate workspace must be an ordinary directory"
for candidate in "$state_dir" "$build_dir" "$result_dir" "$preparation"; do
  [[ ! -L "$candidate" ]] || fail "candidate workspace entries must not be linked"
done
require_space "$build_minimum_kib" "Flatpak candidate work"
python3 "$repo_root/scripts/linux/verify-flatpak-package.py"
git -C "$repo_root" diff --quiet --ignore-submodules -- \
  || fail "tracked worktree changes make the candidate non-reproducible"
git -C "$repo_root" diff --cached --quiet --ignore-submodules -- \
  || fail "staged changes make the candidate non-reproducible"
[[ -z "$(git -C "$repo_root" ls-files --others --exclude-standard)" ]] \
  || fail "untracked source files make the candidate non-reproducible"
git_commit="$(git -C "$repo_root" rev-parse HEAD)"
manifest_hash="$(sha256sum "$manifest" | awk '{ print $1 }')"
sources_hash="$(sha256sum "$repo_root/packaging/flatpak/cargo-sources.json" | awk '{ print $1 }')"
builder_version="$(flatpak-builder --version | head -n 1 | tr -d '\r')"
[[ "$git_commit" =~ ^[0-9a-f]{40}$ \
  && "$manifest_hash" =~ ^[0-9a-f]{64}$ \
  && "$sources_hash" =~ ^[0-9a-f]{64}$ \
  && "$builder_version" =~ ^[[:print:]]{1,128}$ ]] \
  || fail "candidate provenance values are invalid"

provenance() {
  printf 'schema\t1\n'
  printf 'git_commit\t%s\n' "$git_commit"
  printf 'architecture\t%s\n' "$architecture"
  printf 'flatpak_builder\t%s\n' "$builder_version"
  printf 'manifest_sha256\t%s\n' "$manifest_hash"
  printf 'cargo_sources_sha256\t%s\n' "$sources_hash"
}

prepare_online() {
  flatpak-builder \
    --arch="$architecture" \
    --download-only \
    --force-clean \
    --install-deps-from=flathub \
    --state-dir="$state_dir" \
    --user \
    "$build_dir" \
    "$manifest"
  preparation_tmp="$(mktemp "$work_root/.online-preparation.XXXXXX")"
  trap 'rm -f "${preparation_tmp:-}"' EXIT HUP INT TERM
  {
    provenance
    printf 'sources_cached\ttrue\n'
    printf 'overall\tpass\n'
  } >"$preparation_tmp"
  mv "$preparation_tmp" "$preparation"
  trap - EXIT HUP INT TERM
  require_space "$minimum_kib" "online Flatpak preparation"
  echo "Flatpak sources and user runtimes are prepared."
  echo "Disconnect networking if desired, then run this script with --build-offline."
}

build_offline() {
  [[ -d "$state_dir" && ! -L "$state_dir" ]] \
    || fail "run --prepare-online before the offline build"
  [[ -f "$preparation" && ! -L "$preparation" ]] \
    || fail "online preparation evidence is unavailable"
  preparation_size="$(stat -c '%s' "$preparation")"
  [[ "$preparation_size" =~ ^[0-9]+$ \
    && "$preparation_size" -gt 0 \
    && "$preparation_size" -le 4096 ]] \
    || fail "online preparation evidence is invalid"
  expected_preparation="$(
    provenance
    printf 'sources_cached\ttrue\n'
    printf 'overall\tpass\n'
  )"
  [[ "$(<"$preparation")" == "$expected_preparation" ]] \
    || fail "online preparation does not match this candidate"
  [[ ! -e "$result_dir" && ! -L "$result_dir" ]] \
    || fail "offline candidate output already exists"
  result_tmp="$(mktemp -d "$work_root/.offline-result.XXXXXX")"
  trap 'rm -rf "${result_tmp:-}"' EXIT HUP INT TERM
  staged_repo="$result_tmp/repo"
  staged_bundle="$result_tmp/org.rmac.TextEditor.$architecture.flatpak"
  staged_summary="$result_tmp/offline-build.tsv"
  flatpak-builder \
    --arch="$architecture" \
    --disable-download \
    --force-clean \
    --repo="$staged_repo" \
    --sandbox \
    --state-dir="$state_dir" \
    "$build_dir" \
    "$manifest"
  flatpak build-bundle \
    "$staged_repo" \
    "$staged_bundle" \
    org.rmac.TextEditor

  bundle_hash="$(sha256sum "$staged_bundle" | awk '{ print $1 }')"
  bundle_size="$(stat -c '%s' "$staged_bundle")"
  [[ "$bundle_hash" =~ ^[0-9a-f]{64}$ \
    && "$bundle_size" =~ ^[0-9]+$ \
    && "$bundle_size" -gt 0 ]] \
    || fail "offline evidence values are invalid"
  {
    provenance
    printf 'source_downloads\tdisabled\n'
    printf 'bundle_sha256\t%s\n' "$bundle_hash"
    printf 'bundle_size\t%s\n' "$bundle_size"
    printf 'overall\tpass\n'
  } >"$staged_summary"
  mv "$result_tmp" "$result_dir"
  trap - EXIT HUP INT TERM
  require_space "$minimum_kib" "offline Flatpak build"
  echo "Offline Text Editor Flatpak candidate built and summarized under $result_dir."
}

case "$mode" in
  prepare) prepare_online ;;
  offline) build_offline ;;
esac
