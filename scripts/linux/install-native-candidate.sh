#!/usr/bin/env bash
# SPDX-License-Identifier: MIT

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
marker=/run/rmac-reference-pc
marker_value=rmac-reference-pc-install-v1
minimum_kib=$((15 * 1024 * 1024))
install_minimum_kib=$((25 * 1024 * 1024))
mode=
package_directory=
reinstall=false

usage() {
  cat >&2 <<'EOF'
usage: scripts/linux/install-native-candidate.sh --check|--execute \
  --directory /absolute/path/to/native-package-set [--reinstall]

  --check    Verify the Ubuntu/GNOME recovery boundary and candidate packages.
  --execute  Install that exact two-package set through APT and verify it.
  --reinstall  Reinstall an already-current package set (execute only).

Execution requires /run/rmac-reference-pc to contain exactly:
  rmac-reference-pc-install-v1
EOF
}

fail() {
  echo "native candidate installation refused: $*" >&2
  exit 1
}

available_kib() {
  df -Pk / | awk 'NR == 2 { print $4 }'
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

while [[ $# -gt 0 ]]; do
  case "$1" in
    --check|--execute)
      [[ -z "$mode" ]] || fail "choose exactly one of --check or --execute"
      mode=${1#--}
      ;;
    --directory)
      shift
      [[ $# -gt 0 && -z "$package_directory" ]] \
        || fail "--directory requires one value"
      package_directory=$1
      ;;
    --reinstall)
      [[ "$reinstall" == false ]] || fail "--reinstall may be specified once"
      reinstall=true
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

[[ -n "$mode" && -n "$package_directory" ]] || {
  usage
  exit 2
}
[[ "$mode" == execute || "$reinstall" == false ]] \
  || fail "--reinstall requires --execute"
[[ "$(uname -s)" == Linux ]] || fail "Ubuntu Linux is required"
[[ ${EUID} -ne 0 ]] || fail "run as the graphical test user, not root"
[[ -r /etc/os-release ]] || fail "/etc/os-release is unavailable"

# Ubuntu owns this shell-safe distribution metadata.
# shellcheck disable=SC1091
source /etc/os-release
[[ "${ID:-}" == ubuntu && "${VERSION_ID:-}" == 26.04 ]] \
  || fail "Ubuntu 26.04 is required"
[[ "${XDG_SESSION_TYPE:-}" == wayland ]] \
  || fail "run from the untouched GNOME Wayland session"
desktop_tokens=${XDG_CURRENT_DESKTOP,,}
[[ "$desktop_tokens" =~ (^|[:\;])gnome($|[:\;]) ]] \
  || fail "the current desktop must include the exact GNOME token"
[[ "$package_directory" == /* && -d "$package_directory" \
  && ! -L "$package_directory" ]] \
  || fail "--directory must be an absolute ordinary directory"

for command in apt-get dpkg dpkg-deb dpkg-query python3 sudo systemctl; do
  command -v "$command" >/dev/null 2>&1 || fail "$command is required"
done
[[ -f "$repo_root/rust-toolchain.toml" ]] || fail "run from an rmac checkout"
if systemctl --user --quiet is-active rmac-session.target 2>/dev/null \
  || systemctl --user --quiet is-active rmac-safe-mode.target 2>/dev/null; then
  fail "stop the rmac session before replacing its package files"
fi
require_space "$install_minimum_kib" "candidate installation"

architecture="$(dpkg --print-architecture)"
[[ "$architecture" == amd64 || "$architecture" == arm64 ]] \
  || fail "only amd64 and arm64 candidates are supported"
python3 "$repo_root/scripts/linux/verify-session-package.py" \
  --root / --recovery-only
python3 "$repo_root/scripts/linux/verify-native-packages.py" \
  --directory "$package_directory" --architecture "$architecture"
python3 "$repo_root/scripts/linux/archive-development-install.py" --check

shopt -s nullglob
apps_packages=("$package_directory"/rmac-apps_*_"$architecture".deb)
session_packages=("$package_directory"/rmac-session_*_"$architecture".deb)
shopt -u nullglob
[[ ${#apps_packages[@]} -eq 1 && ${#session_packages[@]} -eq 1 ]] \
  || fail "the verified directory did not contain one package of each type"
apps_package=${apps_packages[0]}
session_package=${session_packages[0]}
apps_version="$(dpkg-deb --field "$apps_package" Version)"
session_version="$(dpkg-deb --field "$session_package" Version)"
[[ -n "$apps_version" && "$apps_version" == "$session_version" ]] \
  || fail "candidate package versions do not match"

cat <<EOF
Verified native candidate install plan
  Ubuntu: 26.04
  Architecture: $architecture
  Version: $apps_version
  Packages: rmac-apps, rmac-session
  Package removals: forbidden
  Existing GNOME recovery session: verified
  Legacy source-install artifacts: archived if present
  User settings and documents: preserved
  Package repair: $reinstall
EOF

if [[ "$mode" == check ]]; then
  echo
  echo "To authorize this dedicated reference PC for one install attempt:"
  echo "  printf '$marker_value\\n' | sudo tee $marker >/dev/null"
  echo "Then repeat this command with --execute. The marker disappears at reboot."
  exit 0
fi

[[ -f "$marker" && ! -L "$marker" && "$(<"$marker")" == "$marker_value" ]] \
  || fail "$marker does not contain the exact one-boot authorization marker"
sudo -v
python3 "$repo_root/scripts/linux/archive-development-install.py" --execute
sudo rm -- "$marker"
apt_options=(install --yes --no-remove)
if [[ "$reinstall" == true ]]; then
  apt_options+=(--reinstall)
fi
sudo apt-get "${apt_options[@]}" "$apps_package" "$session_package"
require_space "$minimum_kib" "completed candidate installation"

tab=$'\t'
for package in rmac-apps rmac-session; do
  installed="$(dpkg-query --show --showformat='${Status}\t${Version}' "$package")"
  [[ "$installed" == "install ok installed${tab}${apps_version}" ]] \
    || fail "$package is not installed at the exact candidate version"
done
python3 "$repo_root/scripts/linux/verify-session-package.py" \
  --root / --installed-host
systemctl --user daemon-reload

echo
echo "Installed and verified rmac candidate $apps_version for $architecture."
echo "Sign out normally, choose rmac in GDM, and keep Ubuntu available for recovery."
echo "The one-attempt authorization marker has been consumed."
