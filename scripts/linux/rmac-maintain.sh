#!/usr/bin/env bash
# SPDX-License-Identifier: MIT

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
installer="$repo_root/scripts/linux/install-native-candidate.sh"
marker=/run/rmac-reference-pc
marker_value=rmac-reference-pc-install-v1
action=${1:-status}

usage() {
  cat <<'EOF'
usage: scripts/linux/rmac-maintain.sh status|check|install|repair

  status   Show the installed and current package-candidate versions.
  check    Verify the current package candidate without changing the PC.
  install  Safely install or upgrade to the current package candidate.
  repair   Re-verify and reinstall the current package candidate.

The exact current-commit candidate is preferred. If only installer code changed,
the sole verified-looking candidate for the current package version is reused.
No directory, authorization-marker command, or package filename is required.
Run install, repair, and check from the stock Ubuntu GNOME Wayland session.
EOF
}

fail() {
  echo "rmac maintenance refused: $*" >&2
  exit 1
}

installed_version() {
  local package=$1
  local value
  value="$(dpkg-query --show --showformat='${Status}\t${Version}' "$package" 2>/dev/null || true)"
  case "$value" in
    $'install ok installed\t'*) printf '%s\n' "${value#*$'\t'}" ;;
    *) printf '%s\n' "not-installed" ;;
  esac
}

current_package_version() {
  python3 - "$repo_root" <<'PY'
from pathlib import Path
import sys

root = Path(sys.argv[1])
sys.path.insert(0, str(root / "scripts" / "linux"))
from native_package_contract import ContractError, native_version

try:
    print(native_version(root))
except ContractError as error:
    raise SystemExit(str(error)) from error
PY
}

resolve_candidate() {
  local exact=$1
  local version=$2
  local path
  local matches=()
  if [[ -d "$exact" && ! -L "$exact" ]]; then
    printf '%s\n' "$exact"
    return
  fi
  shopt -s nullglob
  for path in "$repo_root"/target/native-"$architecture"-*; do
    if [[ -d "$path" && ! -L "$path" \
      && -f "$path/rmac-apps_${version}_${architecture}.deb" \
      && -f "$path/rmac-session_${version}_${architecture}.deb" \
      && -f "$path/native-packages.json" \
      && -f "$path/SHA256SUMS" ]]; then
      matches+=("$path")
    fi
  done
  shopt -u nullglob
  if [[ ${#matches[@]} -eq 1 ]]; then
    printf '%s\n' "${matches[0]}"
  elif [[ ${#matches[@]} -gt 1 ]]; then
    fail "multiple candidates match package version $version"
  fi
}

[[ $# -le 1 ]] || {
  usage >&2
  exit 2
}
case "$action" in
  status|check|install|repair) ;;
  -h|--help)
    usage
    exit 0
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac

[[ "$(uname -s)" == Linux ]] || fail "Ubuntu Linux is required"
[[ ${EUID} -ne 0 ]] || fail "run as the graphical user, not root"
for command in dpkg dpkg-query git python3 sudo; do
  command -v "$command" >/dev/null 2>&1 || fail "$command is required"
done
[[ -x "$installer" ]] || fail "the native candidate installer is unavailable"

architecture="$(dpkg --print-architecture)"
[[ "$architecture" == amd64 || "$architecture" == arm64 ]] \
  || fail "only amd64 and arm64 candidates are supported"
revision="$(git -C "$repo_root" rev-parse --short HEAD 2>/dev/null)" \
  || fail "the rmac checkout revision is unavailable"
[[ "$revision" =~ ^[0-9a-f]{7,40}$ ]] || fail "the rmac checkout revision is invalid"
candidate="$repo_root/target/native-$architecture-$revision"
package_version="$(current_package_version)" \
  || fail "the current package version is unavailable"
[[ "$package_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+-[0-9]+$ ]] \
  || fail "the current package version is invalid"
candidate="$(resolve_candidate "$candidate" "$package_version")"

if [[ "$action" == status ]]; then
  free_kib="$(df -Pk / | awk 'NR == 2 { print $4 }')"
  [[ "$free_kib" =~ ^[0-9]+$ ]] || fail "free storage could not be measured"
  printf 'rmac maintenance status\n'
  printf '  Revision: %s\n' "$revision"
  printf '  Package candidate version: %s\n' "$package_version"
  printf '  Architecture: %s\n' "$architecture"
  printf '  Free storage: %s GiB\n' "$((free_kib / 1024 / 1024))"
  printf '  rmac-apps: %s\n' "$(installed_version rmac-apps)"
  printf '  rmac-session: %s\n' "$(installed_version rmac-session)"
  if python3 "$repo_root/scripts/linux/verify-session-package.py" \
    --root / --recovery-only >/dev/null 2>&1; then
    printf '  Ubuntu recovery session: verified\n'
  else
    printf '  Ubuntu recovery session: FAILED\n'
  fi
  if [[ -n "$candidate" ]]; then
    printf '  Candidate: %s\n' "$candidate"
  else
    printf '  Candidate: not-built\n'
  fi
  exit 0
fi

[[ -n "$candidate" && -d "$candidate" && ! -L "$candidate" ]] \
  || fail "build package version $package_version for revision $revision first"

"$installer" --check --directory "$candidate"
if [[ "$action" == check ]]; then
  exit 0
fi

sudo -v
cleanup_marker() {
  sudo rm -f -- "$marker" 2>/dev/null || true
}
trap cleanup_marker EXIT INT TERM
printf '%s\n' "$marker_value" | sudo tee "$marker" >/dev/null
execute_options=(--execute --directory "$candidate")
if [[ "$action" == repair ]]; then
  execute_options+=(--reinstall)
fi
"$installer" "${execute_options[@]}"
trap - EXIT INT TERM

echo
if [[ "$action" == repair ]]; then
  echo "rmac repair completed for revision $revision."
else
  echo "rmac install completed for revision $revision."
fi
