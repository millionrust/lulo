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

  status   Show the installed and current-commit candidate versions.
  check    Verify the current-commit candidate without changing the PC.
  install  Safely install or upgrade to the current-commit candidate.
  repair   Re-verify and reinstall the current-commit candidate.

The candidate is selected from the current commit automatically. No package
directory, authorization-marker command, or package filename is required.
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

if [[ "$action" == status ]]; then
  free_kib="$(df -Pk / | awk 'NR == 2 { print $4 }')"
  [[ "$free_kib" =~ ^[0-9]+$ ]] || fail "free storage could not be measured"
  printf 'rmac maintenance status\n'
  printf '  Revision: %s\n' "$revision"
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
  if [[ -d "$candidate" && ! -L "$candidate" ]]; then
    printf '  Current candidate: %s\n' "$candidate"
  else
    printf '  Current candidate: not-built\n'
  fi
  exit 0
fi

[[ -d "$candidate" && ! -L "$candidate" ]] \
  || fail "build the current candidate first: $candidate"

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
