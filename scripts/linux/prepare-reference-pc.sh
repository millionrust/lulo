#!/usr/bin/env bash
# SPDX-License-Identifier: MIT

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
minimum_kib=$((15 * 1024 * 1024))
build_minimum_kib=$((25 * 1024 * 1024))
execute=false

packages=(
  at-spi2-core
  build-essential
  clang
  curl
  dbus
  dpkg-dev
  fonts-inter
  git
  jq
  libfontconfig1-dev
  libfreetype-dev
  libglib2.0-bin
  libpam0g-dev
  libsndfile1
  libssl-dev
  libvulkan-dev
  libwayland-dev
  libx11-xcb-dev
  libxcb-render0-dev
  libxcb-shape0-dev
  libxcb-xfixes0-dev
  libxcb1-dev
  libxkbcommon-dev
  libxkbcommon-x11-dev
  mesa-vulkan-drivers
  orca
  pciutils
  pipewire-bin
  pkg-config
  python3-pyatspi
  software-properties-common
  sway
  swayidle
  swaylock
  vulkan-tools
  wayland-utils
  wireplumber
  xdg-desktop-portal
  xdg-desktop-portal-gnome
)

usage() {
  cat >&2 <<'EOF'
usage: scripts/linux/prepare-reference-pc.sh --check|--execute

  --check    Validate the host and print the exact preparation plan.
  --execute  Update Ubuntu, install prerequisites, Rust, and cargo-deny.

The script intentionally does not install niri or remove the stock GNOME
session. Follow docs/linux-reference-bringup.md after the GNOME baseline passes.
EOF
}

case "${1:-}" in
  --check) ;;
  --execute) execute=true ;;
  -h|--help)
    usage
    exit 0
    ;;
  *)
    usage
    exit 2
    ;;
esac
if [[ $# -ne 1 ]]; then
  usage
  exit 2
fi

fail() {
  echo "reference-PC preparation refused: $*" >&2
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

[[ "$(uname -s)" == Linux ]] || fail "Ubuntu Linux is required"
[[ ${EUID} -ne 0 ]] || fail "run as the graphical test user, not root"
[[ -r /etc/os-release ]] || fail "/etc/os-release is unavailable"

# This file is owned by the installed operating system and contains shell-safe
# distribution metadata on Ubuntu.
# shellcheck disable=SC1091
source /etc/os-release
[[ "${ID:-}" == ubuntu ]] || fail "Ubuntu is required"
[[ "${VERSION_ID:-}" == 26.04 ]] || fail "Ubuntu 26.04 is required"
command -v sudo >/dev/null 2>&1 || fail "sudo is required"
[[ -f "$repo_root/rust-toolchain.toml" ]] || fail "run from an rmac checkout"
require_space "$build_minimum_kib" "preparation"

rust_channel="$(awk -F '"' '/^channel = / { print $2; exit }' "$repo_root/rust-toolchain.toml")"
[[ "$rust_channel" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] \
  || fail "rust-toolchain.toml does not contain an exact stable toolchain"

cat <<EOF
Reference-PC preparation plan
  Ubuntu: 26.04
  Rust: $rust_channel with clippy and rustfmt
  cargo-deny: 0.19.8
  APT packages: ${#packages[@]}
  GNOME recovery session: preserved
  niri: intentionally deferred until the GNOME baseline passes
EOF

if [[ "$execute" != true ]]; then
  echo "Host checks passed. Re-run with --execute to apply this plan."
  exit 0
fi

sudo -v
sudo apt-get update
sudo apt-get install --yes software-properties-common
sudo add-apt-repository --yes universe
sudo apt-get update
sudo apt-get full-upgrade --yes
require_space "$build_minimum_kib" "post-upgrade dependency installation"
sudo apt-get install --yes "${packages[@]}"
sudo apt-get clean
require_space "$minimum_kib" "installed prerequisites"

if ! command -v rustup >/dev/null 2>&1; then
  command -v curl >/dev/null 2>&1 || fail "curl was not installed"
  rustup_installer="$(mktemp "${TMPDIR:-/tmp}/rmac-rustup.XXXXXX")"
  trap 'rm -f "${rustup_installer:-}"' EXIT HUP INT TERM
  curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location \
    https://sh.rustup.rs --output "$rustup_installer"
  sh "$rustup_installer" -y --profile minimal
  rm -f "$rustup_installer"
  trap - EXIT HUP INT TERM
fi

export PATH="$HOME/.cargo/bin:$PATH"
command -v rustup >/dev/null 2>&1 || fail "rustup is unavailable after installation"
require_space "$build_minimum_kib" "Rust tool installation"
rustup toolchain install "$rust_channel" \
  --profile minimal --component clippy --component rustfmt

if ! cargo deny --version 2>/dev/null | grep -qx 'cargo-deny 0.19.8'; then
  require_space "$build_minimum_kib" "cargo-deny installation"
  cargo install --locked cargo-deny --version 0.19.8 --force
fi
require_space "$minimum_kib" "completed preparation"

echo
echo "Reference-PC prerequisites are ready."
if [[ -e /var/run/reboot-required ]]; then
  echo "Ubuntu requires a reboot before the untouched GNOME baseline."
else
  echo "Log into the untouched GNOME Wayland session before baseline capture."
fi
echo "Then run: bash scripts/linux/run-reference-gates.sh --preflight-only"
