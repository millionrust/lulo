#!/usr/bin/env bash
# SPDX-License-Identifier: MIT

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
minimum_kib=$((15 * 1024 * 1024))

[[ "$(uname -s)" == Linux ]] || {
  echo "desktop metadata validation requires Linux" >&2
  exit 2
}
for command_name in appstreamcli desktop-file-validate python3; do
  command -v "$command_name" >/dev/null 2>&1 || {
    echo "desktop metadata validation requires $command_name" >&2
    exit 2
  }
done
available_kib="$(df -Pk "$repo_root" | awk 'NR == 2 { print $4 }')"
if [[ ! "$available_kib" =~ ^[0-9]+$ ]] || (( available_kib < minimum_kib )); then
  echo "desktop metadata validation requires at least 15 GiB free" >&2
  exit 3
fi

[[ ! -L "$repo_root/target" ]] || {
  echo "desktop metadata validation refuses a linked target directory" >&2
  exit 3
}
mkdir -p "$repo_root/target"
workspace="$(mktemp -d "$repo_root/target/.rmac-desktop-metadata.XXXXXX")"
trap 'rm -rf "${workspace:-}"' EXIT HUP INT TERM
python3 "$repo_root/scripts/linux/stage-application-package.py" \
  --destdir "$workspace/root"
python3 "$repo_root/scripts/linux/verify-application-package.py" \
  --root "$workspace/root" \
  --standard-validators
echo "Ubuntu desktop and AppStream metadata validators passed."
