#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
output_dir="${1:-$repo_root/target/linux-evidence/$timestamp}"
if [[ "$output_dir" != /* ]]; then
  output_dir="$repo_root/$output_dir"
fi

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "this evidence collector must run on Linux" >&2
  exit 2
fi
available_kib=$(df -Pk "$repo_root" | awk 'NR == 2 { print $4 }')
minimum_kib=$((15 * 1024 * 1024))
if (( available_kib < minimum_kib )); then
  echo "evidence collection stopped: ${available_kib} KiB free; ${minimum_kib} KiB required" >&2
  exit 3
fi
mkdir -p "$output_dir"

command_version() {
  local command_name=$1
  shift
  if command -v "$command_name" >/dev/null 2>&1; then
    "$command_name" "$@" 2>&1 || true
  else
    echo "$command_name: not installed"
  fi
}

{
  echo "captured_at=$timestamp"
  echo "git_commit=$(git -C "$repo_root" rev-parse HEAD)"
  if [[ -n "$(git -C "$repo_root" status --porcelain --untracked-files=no)" ]]; then
    echo "git_dirty=true"
  else
    echo "git_dirty=false"
  fi
  echo
  grep -E '^(NAME|VERSION|VERSION_ID|VERSION_CODENAME|ID)=' /etc/os-release || true
  echo
  uname -srmo
  echo "session_type=${XDG_SESSION_TYPE:-unset}"
  echo "current_desktop=${XDG_CURRENT_DESKTOP:-unset}"
  echo "session_desktop=${XDG_SESSION_DESKTOP:-unset}"
  echo "wayland_display=${WAYLAND_DISPLAY:-unset}"
  echo "x11_display=${DISPLAY:-unset}"
  echo "repository_available_kib=$(df -Pk "$repo_root" | awk 'NR == 2 { print $4 }')"
  echo
  command_version rustc --version --verbose
  command_version cargo --version
  command_version niri --version
  command_version gnome-shell --version
  command_version orca --version
} >"$output_dir/environment.txt"

{
  if command -v lspci >/dev/null 2>&1; then
    lspci -nnk | sed -n '/VGA compatible controller/,+3p; /3D controller/,+3p; /Display controller/,+3p'
  else
    echo "lspci: not installed"
  fi
  echo
  command_version vulkaninfo --summary | sed -E '/(deviceUUID|driverUUID)/d'
} >"$output_dir/graphics.txt"

if command -v wayland-info >/dev/null 2>&1 && [[ -n "${WAYLAND_DISPLAY:-}" ]]; then
  timeout 20s wayland-info >"$output_dir/wayland-info.txt" 2>&1 || true
else
  echo "wayland-info unavailable or this is not a Wayland session" >"$output_dir/wayland-info.txt"
fi

if command -v niri >/dev/null 2>&1 && [[ "${XDG_CURRENT_DESKTOP:-}" == *niri* ]]; then
  niri msg --json outputs 2>"$output_dir/niri-outputs.err" \
    | jq 'walk(if type == "object" then del(.serial) else . end)' \
    >"$output_dir/niri-outputs.json" || true
fi

{
  command_version orca --version
  if command -v gsettings >/dev/null 2>&1; then
    echo -n "toolkit-accessibility="
    gsettings get org.gnome.desktop.interface toolkit-accessibility 2>&1 || true
    echo -n "screen-reader-enabled="
    gsettings get org.gnome.desktop.a11y.applications screen-reader-enabled 2>&1 || true
    echo -n "gtk-text-scaling-factor="
    gsettings get org.gnome.desktop.interface text-scaling-factor 2>&1 || true
  else
    echo "gsettings: not installed"
  fi
} >"$output_dir/accessibility.txt"

{
  command_version pw-play --version
  command_version pw-cli --version
  command_version wpctl --version
  command_version wireplumber --version
} >"$output_dir/audio.txt"

{
  if command -v systemctl >/dev/null 2>&1; then
    systemctl --user --no-pager --full status xdg-desktop-portal.service 2>&1 || true
    systemctl --user --no-pager --full status xdg-desktop-portal-gnome.service 2>&1 || true
  else
    echo "systemctl: not installed"
  fi
} >"$output_dir/portals.txt"

{
  if command -v systemctl >/dev/null 2>&1; then
    for unit in rmac-session.target rmac-safe-mode.target \
      rmac-session-supervisor.service rmac-top-bar.service rmac-dock.service \
      rmac-launcher.service rmac-app-drawer.service rmac-quick-settings.service rmac-notification-center.service rmac-notification-center-panel.service rmac-focus.service rmac-wallpaper.service \
      rmac-shortcut-broker.service; do
      systemctl --user show "$unit" --no-pager \
        --property=Id,LoadState,ActiveState,SubState,Result,NRestarts,MainPID,ExecMainStatus \
        2>&1 || true
    done
  else
    echo "systemctl: not installed"
  fi
  supervisor="$HOME/.local/libexec/rmac/rmac-session-supervisor"
  if [ -x "$supervisor" ]; then
    "$supervisor" status 2>&1 || true
  else
    echo "rmac-session-supervisor: not installed"
  fi
  shortcut_status=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/rmac/shortcuts-status.json
  if [ -f "$shortcut_status" ]; then
    cat "$shortcut_status"
  else
    echo "shortcut status: unavailable"
  fi
} >"$output_dir/rmac-session.txt"

if command -v dpkg-query >/dev/null 2>&1; then
  dpkg-query -W -f='${Package}\t${Version}\n' \
    at-spi2-core clang dbus git libsndfile1 libvulkan-dev libwayland-dev pipewire-bin \
    mesa-vulkan-drivers orca python3-pyatspi rustc sway \
    vulkan-tools wayland-utils wireplumber xdg-desktop-portal \
    xdg-desktop-portal-gnome >"$output_dir/packages.txt" 2>&1 || true
fi

echo "Linux reference evidence written to $output_dir"
