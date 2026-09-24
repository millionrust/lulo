#!/usr/bin/env bash
# SPDX-License-Identifier: MIT

set -euo pipefail

lab_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
repo_root="$(cd "$lab_dir/../.." && pwd)"
target_dir="$lab_dir/target"
manifest="${XDG_DATA_HOME:-${HOME}/.local/share}/rmac/development/upstream-shell-candidate.txt"
minimum_kib=$((15 * 1024 * 1024))
expected_gpui_revision=76c93968da5b8b8809bdd72e4ad9e7d0e946bad0

fail() {
  echo "real niri accessibility smoke failed: $*" >&2
  exit 1
}

[[ "$(uname -s)" == Linux ]] || fail "Linux is required"
[[ ${EUID} -ne 0 ]] || fail "run as the desktop user, not root"
command -v systemctl >/dev/null 2>&1 || fail "systemctl is required"
command -v gsettings >/dev/null 2>&1 || fail "gsettings is required"
command -v niri >/dev/null 2>&1 || fail "niri is required"
[[ -x /usr/bin/python3 ]] || fail "/usr/bin/python3 is required"

if [[ -z "${XDG_RUNTIME_DIR:-}" ]]; then
  runtime_dir="/run/user/$(id -u)"
  [[ -d "$runtime_dir" && -O "$runtime_dir" ]] \
    || fail "the current user's runtime directory is unavailable"
  export XDG_RUNTIME_DIR="$runtime_dir"
fi
if [[ -z "${DBUS_SESSION_BUS_ADDRESS:-}" && -S "$XDG_RUNTIME_DIR/bus" ]]; then
  export DBUS_SESSION_BUS_ADDRESS="unix:path=$XDG_RUNTIME_DIR/bus"
fi

manager_environment="$(systemctl --user show-environment)"
import_manager_value() {
  local name=$1
  local value
  value="$(sed -n "s/^${name}=//p" <<<"$manager_environment" | head -n 1)"
  [[ -n "$value" ]] || fail "$name is missing from the graphical user manager"
  export "$name=$value"
}
for name in WAYLAND_DISPLAY NIRI_SOCKET XDG_SESSION_TYPE XDG_CURRENT_DESKTOP; do
  import_manager_value "$name"
done
[[ "$XDG_SESSION_TYPE" == wayland ]] || fail "the active graphical session is not Wayland"
case ":$XDG_CURRENT_DESKTOP:" in
  *:niri:*|*:rmac:niri:*) ;;
  *) fail "the active graphical session is not niri" ;;
esac

available_kib="$(df -Pk "$repo_root" | awk 'NR == 2 { print $4 }')"
[[ "$available_kib" =~ ^[0-9]+$ && "$available_kib" -ge "$minimum_kib" ]] \
  || fail "the reference PC is below the 15 GiB storage floor"
git -C "$repo_root" diff --quiet --ignore-submodules -- \
  || fail "tracked worktree changes prevent revision-bound evidence"
git -C "$repo_root" diff --cached --quiet --ignore-submodules -- \
  || fail "staged worktree changes prevent revision-bound evidence"

repo_revision="$(git -C "$repo_root" rev-parse HEAD)"
[[ -f "$manifest" && ! -L "$manifest" ]] \
  || fail "the supervised upstream shell candidate manifest is unavailable"
installed_revision="$(sed -n 's/^rmac_revision=//p' "$manifest")"
installed_gpui_revision="$(sed -n 's/^gpui_revision=//p' "$manifest")"
[[ "$installed_revision" == "$repo_revision" ]] \
  || fail "the installed shell candidate does not match the checked-out rmac revision"
[[ "$installed_gpui_revision" == "$expected_gpui_revision" ]] \
  || fail "the installed shell candidate does not match the pinned GPUI revision"

for executable in "$target_dir/debug/a11y" \
  "$HOME/.local/libexec/rmac/rmac-top-bar" \
  "$HOME/.local/libexec/rmac/rmac-dock"; do
  [[ -f "$executable" && ! -L "$executable" && -x "$executable" ]] \
    || fail "required executable is unavailable: $(basename "$executable")"
done
for unit in rmac-session.target rmac-top-bar.service rmac-dock.service; do
  systemctl --user is-active --quiet "$unit" || fail "$unit is not active"
done

outputs_json="$(niri msg --json outputs)"
output_count="$(/usr/bin/python3 -c '
import json, sys
outputs = json.load(sys.stdin)
if not isinstance(outputs, dict):
    raise SystemExit("niri output inventory is not an object")
enabled = [value for value in outputs.values() if isinstance(value, dict) and value.get("logical")]
print(len(enabled))
' <<<"$outputs_json")"
[[ "$output_count" =~ ^[0-9]+$ && "$output_count" -ge 1 && "$output_count" -le 16 ]] \
  || fail "niri did not report one to sixteen enabled logical outputs"

focused_window_id() {
  niri msg --json focused-window | /usr/bin/python3 -c '
import json, sys
window = json.load(sys.stdin)
print(window.get("id", "none") if isinstance(window, dict) else "none")
'
}

# Only toolkit-accessibility is needed: since accesskit_unix 0.22
# (ADR 0013, commit 8de9528a) every rmac window registers with AT-SPI as
# soon as the bus reports IsEnabled, which this key drives. The separate
# screen-reader-enabled key also flips GNOME's Orca autostart condition,
# which starts Orca talking on the owner's real session -- leave it alone.
old_toolkit="$(gsettings get org.gnome.desktop.interface toolkit-accessibility)"
temporary="$(mktemp -d "$XDG_RUNTIME_DIR/rmac-real-niri-smoke.XXXXXX")"
a11y_pid=""
cleanup() {
  local result=$?
  trap - EXIT HUP INT TERM
  if [[ -n "$a11y_pid" ]] && kill -0 "$a11y_pid" 2>/dev/null; then
    kill "$a11y_pid" 2>/dev/null || true
    wait "$a11y_pid" 2>/dev/null || true
  fi
  gsettings set org.gnome.desktop.interface toolkit-accessibility "$old_toolkit" >/dev/null
  systemctl --user restart rmac-top-bar.service rmac-dock.service >/dev/null 2>&1 || true
  rm -rf -- "$temporary"
  exit "$result"
}
trap cleanup EXIT HUP INT TERM

gsettings set org.gnome.desktop.interface toolkit-accessibility true
/usr/bin/python3 -c 'import pyatspi; pyatspi.Registry.getDesktop(0)'

focus_before="$(focused_window_id)"
systemctl --user restart rmac-top-bar.service rmac-dock.service
sleep 2
focus_after="$(focused_window_id)"
[[ "$focus_after" == "$focus_before" ]] \
  || fail "restarting passive shell layers changed focused-window ownership"

RMAC_EXPECTED_TOP_BARS="$output_count" \
  /usr/bin/python3 "$lab_dir/scripts/assert_top_bar_accessibility.py"
RMAC_EXPECTED_DOCKS="$output_count" \
  /usr/bin/python3 "$lab_dir/scripts/assert_dock_accessibility.py"

RMAC_SMOKE_READY_FILE="$temporary/a11y.ready" \
  "$target_dir/debug/a11y" >"$temporary/a11y.log" 2>&1 &
a11y_pid=$!
for _ in {1..200}; do
  [[ -f "$temporary/a11y.ready" ]] && break
  kill -0 "$a11y_pid" 2>/dev/null \
    || fail "the accessibility probe exited before its first frame"
  sleep 0.1
done
[[ -f "$temporary/a11y.ready" ]] || fail "the accessibility probe did not render"
grep -qx a11y "$temporary/a11y.ready" \
  || fail "the accessibility probe published an invalid readiness marker"
/usr/bin/python3 "$lab_dir/scripts/assert_accessibility.py"
kill -0 "$a11y_pid" 2>/dev/null || fail "the accessibility probe exited after assertions"

for unit in rmac-top-bar.service rmac-dock.service; do
  systemctl --user is-active --quiet "$unit" || fail "$unit stopped during assertions"
done

echo "real_niri_accessibility_smoke=pass"
echo "rmac_revision=${repo_revision:0:12}"
echo "gpui_revision=${installed_gpui_revision:0:12}"
echo "enabled_outputs=$output_count"
echo "shell_focus_noninterference=pass"
echo "at_spi_shell_semantics=pass"
echo "at_spi_actions_and_state=pass"
