#!/usr/bin/env bash
# SPDX-License-Identifier: MIT

set -euo pipefail

repo_root="${RMAC_REPO_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
[[ "$repo_root" == /* ]] || {
  echo "upstream shell candidate refused: RMAC_REPO_ROOT must be absolute" >&2
  exit 1
}
lab_dir="$repo_root/shell"
target_dir="$lab_dir/target"
libexec_dir="${HOME}/.local/libexec/rmac"
data_home="${XDG_DATA_HOME:-${HOME}/.local/share}"
manifest_dir="$data_home/rmac/development"
manifest_path="$manifest_dir/upstream-shell-candidate.txt"
minimum_kib=$((15 * 1024 * 1024))
build_minimum_kib=$((25 * 1024 * 1024))
expected_upstream_revision=76c93968da5b8b8809bdd72e4ad9e7d0e946bad0
components=(wallpaper top-bar dock osd)
app_packages=(rmac-finder rmac-terminal rmac-text-editor rmac-activity-monitor rmac-system-settings)
app_binaries=(rmac-files rmac-terminal rmac-text-editor rmac-system-monitor rmac-system-settings)
app_ids=(org.rmac.Files org.rmac.Terminal org.rmac.TextEditor org.rmac.SystemMonitor org.rmac.SystemSettings)

usage() {
  echo "usage: $0 --check|--execute [--no-build] [--release-shell]" >&2
}

fail() {
  echo "upstream shell candidate refused: $*" >&2
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

mode=""
build=true
shell_profile=debug
while [[ $# -gt 0 ]]; do
  case "$1" in
    --check|--execute)
      [[ -z "$mode" ]] || { usage; exit 2; }
      mode=${1#--}
      ;;
    --no-build)
      build=false
      ;;
    --release-shell)
      shell_profile=release
      ;;
    *)
      usage
      exit 2
      ;;
  esac
  shift
done
[[ -n "$mode" ]] || { usage; exit 2; }
[[ "$mode" == execute || "$build" == true ]] \
  || fail "--no-build is meaningful only with --execute"

[[ "$(uname -s)" == Linux ]] || fail "Linux is required"
[[ ${EUID} -ne 0 ]] || fail "run as the desktop user, not root"
[[ -f /etc/os-release ]] || fail "/etc/os-release is unavailable"
# shellcheck disable=SC1091
source /etc/os-release
[[ "${ID:-}" == ubuntu && "${VERSION_ID:-}" == 26.04 ]] \
  || fail "Ubuntu 26.04 is required"
if ! command -v cargo >/dev/null 2>&1 && [[ -x "$HOME/.cargo/bin/cargo" ]]; then
  export PATH="$HOME/.cargo/bin:$PATH"
fi
command -v cargo >/dev/null 2>&1 || fail "cargo is required"
command -v git >/dev/null 2>&1 || fail "git is required"
command -v systemctl >/dev/null 2>&1 || fail "systemctl is required"
[[ -f "$lab_dir/Cargo.toml" && -f "$lab_dir/Cargo.lock" ]] \
  || fail "the pinned upstream GPUI lab is incomplete"
[[ -d "$repo_root/.git" ]] || fail "run from the rmac source checkout"

mapfile -t pinned_revisions < <(
  sed -nE 's/.*rev = "([0-9a-f]{40})".*/\1/p' "$lab_dir/Cargo.toml" | sort -u
)
[[ ${#pinned_revisions[@]} -eq 1 ]] \
  || fail "the upstream GPUI revision is not one exact immutable pin"
[[ "${pinned_revisions[0]}" == "$expected_upstream_revision" ]] \
  || fail "the upstream GPUI revision changed without updating this handoff"

git -C "$repo_root" diff --quiet --ignore-submodules -- \
  || fail "tracked worktree changes must be committed before installation"
git -C "$repo_root" diff --cached --quiet --ignore-submodules -- \
  || fail "staged worktree changes must be committed before installation"

if [[ "$build" == true ]]; then
  require_space "$build_minimum_kib" "building the upstream shell candidate"
else
  require_space "$minimum_kib" "installing the upstream shell candidate"
fi

repo_revision="$(git -C "$repo_root" rev-parse HEAD)"
[[ "$repo_revision" =~ ^[0-9a-f]{40}$ ]] || fail "the rmac revision is invalid"

echo "Upstream shell candidate plan"
echo "  rmac revision: ${repo_revision:0:12}"
echo "  GPUI revision: ${pinned_revisions[0]:0:12}"
echo "  components: wallpaper, menu bar, Dock, system OSD, and five first-party apps"
echo "  destination: $libexec_dir"
echo "  build: $build"
echo "  shell profile: $shell_profile"
echo "  app profile: debug"
echo "  supervised units: preserved"
echo "  GNOME recovery session: untouched"
echo "  public packages: unchanged"

if [[ "$mode" == check ]]; then
  echo "Host checks passed. Re-run with --execute to build and install this development candidate."
  exit 0
fi

if [[ "$build" == true ]]; then
  (
    cd "$lab_dir"
    shell_build=(build --locked --jobs "${CARGO_BUILD_JOBS:-2}")
    if [[ "$shell_profile" == release ]]; then
      shell_build+=(--release)
    fi
    CARGO_TARGET_DIR="$target_dir" cargo "${shell_build[@]}" \
      --features wayland --bin wallpaper --bin top-bar --bin dock --bin osd
  )
  build_args=(build --locked --jobs "${CARGO_BUILD_JOBS:-2}")
  for package in "${app_packages[@]}"; do
    build_args+=(--package "$package")
  done
  (
    cd "$repo_root"
    cargo "${build_args[@]}"
  )
fi
require_space "$minimum_kib" "installing the built upstream shell candidate"

for component in "${components[@]}"; do
  source_path="$target_dir/$shell_profile/$component"
  [[ -f "$source_path" && ! -L "$source_path" && -x "$source_path" ]] \
    || fail "$source_path is not a built executable; rerun without --no-build"
done
for binary in "${app_binaries[@]}"; do
  source_path="$repo_root/target/debug/$binary"
  [[ -f "$source_path" && ! -L "$source_path" && -x "$source_path" ]] \
    || fail "$source_path is not a built executable; rerun without --no-build"
done

if pgrep -f "^${target_dir}/${shell_profile}/(wallpaper|top-bar|dock|osd)( --service)?$" >/dev/null 2>&1; then
  fail "the manual shell preview is still running; stop it before installing supervised copies"
fi

applications_dir="$data_home/applications"
icons_dir="$data_home/icons/hicolor/scalable/apps"
install -d -m 0755 "$libexec_dir" "$manifest_dir" "$applications_dir" "$icons_dir"
declare -a staged=()
declare -a destinations=()
cleanup() {
  for path in "${staged[@]:-}"; do
    rm -f -- "$path"
  done
}
trap cleanup EXIT HUP INT TERM

for component in "${components[@]}"; do
  destination="$libexec_dir/rmac-$component"
  temporary="$(mktemp "$libexec_dir/.rmac-${component}.XXXXXX")"
  staged+=("$temporary")
  destinations+=("$destination")
  install -m 0755 "$target_dir/$shell_profile/$component" "$temporary"
done
for index in "${!app_binaries[@]}"; do
  binary="${app_binaries[$index]}"
  app_id="${app_ids[$index]}"

  destination="$libexec_dir/$binary"
  temporary="$(mktemp "$libexec_dir/.${binary}.XXXXXX")"
  staged+=("$temporary")
  destinations+=("$destination")
  install -m 0755 "$repo_root/target/debug/$binary" "$temporary"

  desktop_source="$repo_root/packaging/rmac-apps/applications/${app_id}.desktop"
  desktop_destination="$applications_dir/${app_id}.desktop"
  desktop_temporary="$(mktemp "$applications_dir/.${app_id}.XXXXXX")"
  staged+=("$desktop_temporary")
  destinations+=("$desktop_destination")
  sed \
    -e "s|^TryExec=/usr/bin/${binary}|TryExec=${libexec_dir}/${binary}|" \
    -e "s|^Exec=/usr/bin/${binary}|Exec=${libexec_dir}/${binary}|" \
    "$desktop_source" >"$desktop_temporary"
  chmod 0644 "$desktop_temporary"

  icon_source="$repo_root/packaging/rmac-apps/icons/${app_id}.svg"
  icon_destination="$icons_dir/${app_id}.svg"
  icon_temporary="$(mktemp "$icons_dir/.${app_id}.XXXXXX")"
  staged+=("$icon_temporary")
  destinations+=("$icon_destination")
  install -m 0644 "$icon_source" "$icon_temporary"
done
for index in "${!staged[@]}"; do
  mv -f -- "${staged[$index]}" "${destinations[$index]}"
done
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$applications_dir"
fi

manifest_temporary="$(mktemp "$manifest_dir/.upstream-shell-candidate.XXXXXX")"
staged+=("$manifest_temporary")
{
  echo "format=1"
  echo "rmac_revision=$repo_revision"
  echo "gpui_revision=${pinned_revisions[0]}"
  if [[ "$shell_profile" == debug ]]; then
    echo "cargo_profile=debug"
  else
    echo "cargo_profile=mixed"
  fi
  echo "shell_cargo_profile=$shell_profile"
  echo "app_cargo_profile=debug"
  echo "components=rmac-wallpaper,rmac-top-bar,rmac-dock,rmac-osd,rmac-files,rmac-terminal,rmac-text-editor,rmac-system-monitor,rmac-system-settings"
} >"$manifest_temporary"
chmod 0644 "$manifest_temporary"
mv -f -- "$manifest_temporary" "$manifest_path"

trap - EXIT HUP INT TERM
if [[ -z "${XDG_RUNTIME_DIR:-}" ]]; then
  runtime_dir="/run/user/$(id -u)"
  [[ -d "$runtime_dir" && -O "$runtime_dir" ]] \
    || fail "the current user's systemd runtime directory is unavailable"
  export XDG_RUNTIME_DIR="$runtime_dir"
fi
if [[ -z "${DBUS_SESSION_BUS_ADDRESS:-}" && -S "$XDG_RUNTIME_DIR/bus" ]]; then
  export DBUS_SESSION_BUS_ADDRESS="unix:path=$XDG_RUNTIME_DIR/bus"
fi
systemctl --user daemon-reload
systemctl --user reset-failed \
  rmac-wallpaper.service rmac-top-bar.service rmac-dock.service rmac-osd.service >/dev/null 2>&1 || true

if systemctl --user is-active --quiet rmac-session.target; then
  systemctl --user restart \
    rmac-wallpaper.service rmac-top-bar.service rmac-dock.service rmac-osd.service
  echo "Restarted the four shell surfaces in the active rmac session."
else
  state_home="${XDG_STATE_HOME:-${HOME}/.local/state}"
  if [[ -f "$state_home/rmac/session/safe-mode.json" ]]; then
    echo "A previous component failure has kept rmac in safe mode."
    echo "Review: ~/.local/libexec/rmac/rmac-session-supervisor diagnostics"
    echo "Recover: ~/.local/libexec/rmac/rmac-session-supervisor clear-safe-mode"
  else
    echo "The rmac session is not active; start it from niri with ~/.local/bin/rmac-session-start."
  fi
fi

echo "Installed the pinned upstream shell, system OSD, and five first-party app candidates."
echo "This handoff does not promote GPUI or make these binaries public release artifacts."
