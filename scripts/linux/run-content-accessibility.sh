#!/usr/bin/env bash
# Launch Terminal, Notes, Files, Clock, Mail and Calendar once each, one at a
# time, with temporary XDG_* directories, and run their
# assert_*_accessibility.py checks.
# Needs a live niri session with the AT-SPI bus
# enabled (org.a11y.Status.IsEnabled). It never turns on the screen reader,
# never injects input, and never touches the user's own app data.
#
# usage: run-content-accessibility.sh BIN_DIR [terminal|notes|files|clock|mail|calendar]...
#   BIN_DIR holds the corresponding rmac-* binaries.
set -euo pipefail

bin_dir=${1:?usage: run-content-accessibility.sh BIN_DIR [terminal|notes|files|clock|mail|calendar]...}
shift
apps=("$@")
[[ ${#apps[@]} -gt 0 ]] || apps=(terminal notes files clock mail calendar)
scripts_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
export DBUS_SESSION_BUS_ADDRESS=${DBUS_SESSION_BUS_ADDRESS:-unix:path=$XDG_RUNTIME_DIR/bus}
export WAYLAND_DISPLAY=${WAYLAND_DISPLAY:-wayland-1}

enabled=$(gdbus call --session --dest org.a11y.Bus --object-path /org/a11y/bus \
  --method org.freedesktop.DBus.Properties.Get org.a11y.Status IsEnabled)
[[ "$enabled" == *true* ]] || { echo "AT-SPI IsEnabled is off; not changing it" >&2; exit 1; }

work=$(mktemp -d "${TMPDIR:-/tmp}/rmac-content-a11y.XXXXXX")
echo "evidence: $work"

running() {
  local comm=$1 dir
  for dir in /proc/[0-9]*; do
    [[ "$(cat "$dir/comm" 2>/dev/null)" == "$comm" ]] && return 0
  done
  return 1
}

run_app() {
  local app=$1 binary comm assert
  local -a arguments=()
  case "$app" in
    terminal) binary=rmac-terminal; assert=assert_terminal_accessibility.py ;;
    notes) binary=rmac-notes; assert=assert_notes_accessibility.py ;;
    files) binary=rmac-files; assert=assert_files_accessibility.py ;;
    clock) binary=rmac-clock; assert=assert_clock_accessibility.py ;;
    mail) binary=rmac-mail; assert=assert_mail_accessibility.py ;;
    calendar) binary=rmac-calendar; assert=assert_calendar_accessibility.py ;;
    *) echo "unknown app $app" >&2; return 2 ;;
  esac
  comm=${binary:0:15}
  # A running copy would receive this launch as a new window instead.
  if running "$comm"; then
    echo "$app: skipped, $binary is already running" >&2
    return 3
  fi

  local root="$work/$app"
  mkdir -p "$root/data" "$root/state" "$root/config" "$root/cache"
  local -a env_vars=(
    XDG_DATA_HOME="$root/data" XDG_STATE_HOME="$root/state"
    XDG_CONFIG_HOME="$root/config" XDG_CACHE_HOME="$root/cache"
  )
  case "$app" in
    terminal)
      cat >"$root/shell.sh" <<'EOF'
#!/bin/sh
printf 'rmac-a11y-marker\nsecond line h\303\251llo\n'
PS1='a11y$ ' exec /bin/sh -i
EOF
      chmod +x "$root/shell.sh"
      env_vars+=(SHELL="$root/shell.sh")
      ;;
    files)
      mkdir -p "$root/folder/content-a11y/beta folder"
      printf 'hello\n' >"$root/folder/content-a11y/beta folder/nested.txt"
      printf 'alpha\n' >"$root/folder/content-a11y/alpha.txt"
      printf '%%PDF-1.4\n' >"$root/folder/content-a11y/gamma.pdf"
      arguments=(--path "$root/folder/content-a11y")
      export RMAC_FILES_FOLDER="$root/folder/content-a11y"
      ;;
    mail)
      # The only legitimate use of Mail's fixture data: a private nested
      # session asserting the sidebar/list/viewer tree, never a real
      # account (docs/design/calendar-mail.md §3).
      env_vars+=(RMAC_MAIL_FIXTURE=1)
      ;;
    calendar)
      # Same reasoning as Mail above: no EDS source here, so without the
      # fixture Calendar would show an empty week with nothing to assert.
      env_vars+=(RMAC_CALENDAR_FIXTURE=1)
      ;;
  esac

  env "${env_vars[@]}" "$bin_dir/$binary" "${arguments[@]}" \
    >"$root/app.log" 2>&1 9>&- &
  local pid=$!
  local status=0
  RMAC_A11Y_DUMP="$root/tree.jsonl" /usr/bin/python3 "$scripts_dir/$assert" \
    >"$root/assert.log" 2>&1 9>&- || status=$?
  kill "$pid" 2>/dev/null || true
  for _ in $(seq 50); do
    kill -0 "$pid" 2>/dev/null || break
    sleep 0.1
  done
  kill -9 "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  echo "$app: exit $status"
  cat "$root/assert.log"
  return "$status"
}

exec 9>/tmp/lulo-journey.lock
flock 9
overall=0
for app in "${apps[@]}"; do
  run_app "$app" || overall=1
done
exit "$overall"
