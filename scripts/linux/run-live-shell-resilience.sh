#!/usr/bin/env bash
# SPDX-License-Identifier: MIT

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
minimum_kib=$((15 * 1024 * 1024))
mode=
components=(rmac-wallpaper.service rmac-top-bar.service rmac-dock.service rmac-osd.service)
supervisor=rmac-session-supervisor.service
all_units=("${components[@]}" "$supervisor")
config="${XDG_CONFIG_HOME:-$HOME/.config}/rmac/shell.json"
config_dir="$(dirname "$config")"
last_good="$config.last-good"
safe_mode="${XDG_STATE_HOME:-$HOME/.local/state}/rmac/session/safe-mode.json"

usage() {
  echo "usage: $0 --check|--execute" >&2
}

fail() {
  echo "live shell resilience failed: $*" >&2
  exit 1
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --check|--execute)
      [[ -z "$mode" ]] || fail "choose exactly one mode"
      mode=${1#--}
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

[[ -n "$mode" ]] || {
  usage
  exit 2
}
[[ "$(uname -s)" == Linux ]] || fail "Linux is required"
[[ ${EUID} -ne 0 ]] || fail "run as the graphical user, not root"
[[ "${XDG_SESSION_TYPE:-}" == wayland ]] || fail "an active Wayland session is required"
for command in awk df dirname git grep install jq kill niri sleep systemctl; do
  command -v "$command" >/dev/null 2>&1 || fail "$command is required"
done
available_kib="$(df -Pk / | awk 'NR == 2 { print $4 }')"
[[ "$available_kib" =~ ^[0-9]+$ ]] || fail "free storage could not be measured"
(( available_kib >= minimum_kib )) || fail "at least 15 GiB free is required"
[[ -S "${NIRI_SOCKET:-}" ]] || fail "NIRI_SOCKET is not an active socket"
[[ ! -e "$config" && ! -L "$config" ]] \
  || fail "the live settings file must be absent on this dedicated test account"
[[ ! -e "$last_good" && ! -L "$last_good" ]] \
  || fail "the last-known-good settings file must be absent on this dedicated test account"
[[ ! -L "$config_dir" && ( ! -e "$config_dir" || -d "$config_dir" ) ]] \
  || fail "the settings directory is unsafe"
[[ ! -e "$safe_mode" && ! -L "$safe_mode" ]] \
  || fail "safe mode is already present"
for unit in "${all_units[@]}"; do
  systemctl --user --quiet is-active "$unit" || fail "$unit is not active"
done

revision="$(git -C "$repo_root" rev-parse HEAD)"
[[ "$revision" =~ ^[0-9a-f]{40}$ ]] || fail "the repository revision is invalid"
echo "Live shell resilience plan"
echo "  revision: ${revision:0:12}"
echo "  crash recovery: wallpaper, menu bar, Dock, system OSD, supervisor"
echo "  malformed config: isolated absent-file fixture"
echo "  storage floor: 15 GiB"

if [[ "$mode" == check ]]; then
  echo "Preflight passed. Re-run with --execute to perform the controlled fault injection."
  exit 0
fi

cleanup() {
  rm -f -- "$config"
  systemctl --user start "${all_units[@]}" >/dev/null 2>&1 || true
  systemctl --user reset-failed "${all_units[@]}" >/dev/null 2>&1 || true
}
trap cleanup EXIT HUP INT TERM
systemctl --user reset-failed "${all_units[@]}"

recover_unit() {
  local unit=$1 before after restarts
  before="$(systemctl --user show "$unit" -p MainPID --value)"
  [[ "$before" =~ ^[1-9][0-9]*$ ]] || fail "$unit has no main process"
  kill -KILL "$before"
  after=0
  for _ in 1 2 3 4 5 6; do
    sleep 1
    after="$(systemctl --user show "$unit" -p MainPID --value)"
    if systemctl --user --quiet is-active "$unit" \
      && [[ "$after" =~ ^[1-9][0-9]*$ && "$after" != "$before" ]]; then
      break
    fi
  done
  [[ "$after" =~ ^[1-9][0-9]*$ && "$after" != "$before" ]] \
    || fail "$unit did not recover with a new process"
  restarts="$(systemctl --user show "$unit" -p NRestarts --value)"
  [[ "$restarts" == 1 ]] || fail "$unit reported an unexpected restart count"
  echo "crash_recovery=pass unit=$unit"
}

for unit in "${components[@]}"; do
  recover_unit "$unit"
done
recover_unit "$supervisor"

wallpaper_pid="$(systemctl --user show rmac-wallpaper.service -p MainPID --value)"
top_bar_pid="$(systemctl --user show rmac-top-bar.service -p MainPID --value)"
dock_pid="$(systemctl --user show rmac-dock.service -p MainPID --value)"
osd_pid="$(systemctl --user show rmac-osd.service -p MainPID --value)"
if [[ ! -d "$config_dir" ]]; then
  install -d -m 0700 "$config_dir"
fi
printf '{invalid-json\n' >"$config"
sleep 3
for unit in "${components[@]}"; do
  systemctl --user --quiet is-active "$unit" || fail "$unit stopped on malformed settings"
done
[[ "$(systemctl --user show rmac-wallpaper.service -p MainPID --value)" == "$wallpaper_pid" ]] \
  || fail "wallpaper restarted on malformed settings"
[[ "$(systemctl --user show rmac-top-bar.service -p MainPID --value)" == "$top_bar_pid" ]] \
  || fail "top bar restarted on malformed settings"
[[ "$(systemctl --user show rmac-dock.service -p MainPID --value)" == "$dock_pid" ]] \
  || fail "Dock restarted on malformed settings"
[[ "$(systemctl --user show rmac-osd.service -p MainPID --value)" == "$osd_pid" ]] \
  || fail "system OSD restarted on malformed settings"
rm -f -- "$config"
sleep 3
for unit in "${components[@]}"; do
  systemctl --user --quiet is-active "$unit" || fail "$unit did not survive settings recovery"
done
echo "malformed_settings=pass"

layers="$(niri msg --json layers | jq -r '.. | objects | select(has("namespace")) | .namespace')"
for namespace in rmac-wallpaper- rmac-top-bar- rmac-dock- rmac-osd-; do
  grep -q "^$namespace" <<<"$layers" || fail "the $namespace layer was not restored"
done
[[ ! -e "$safe_mode" ]] || fail "fault injection entered safe mode"
systemctl --user reset-failed "${all_units[@]}"
trap - EXIT HUP INT TERM

echo "layer_recovery=pass"
echo "safe_mode=absent"
echo "live_shell_resilience=pass"
