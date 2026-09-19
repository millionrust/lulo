#!/usr/bin/env bash
# Capture each installed rmac application and overlay from the active reference
# session. The PNGs are intentionally written to /tmp/caps so the Mac-side
# audit command in docs/live-surface-audit-2026-09-19.md can copy one bounded
# evidence set without touching user files.

set -euo pipefail

out=${RMAC_CAPTURE_DIR:-/tmp/caps}
settle=${RMAC_CAPTURE_SETTLE:-2}
runtime=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
export XDG_RUNTIME_DIR=${runtime}
export WAYLAND_DISPLAY=${WAYLAND_DISPLAY:-wayland-1}

files_bin=${RMAC_FILES_BIN:-/usr/bin/rmac-files}
settings_bin=${RMAC_SETTINGS_BIN:-/usr/bin/rmac-system-settings}
terminal_bin=${RMAC_TERMINAL_BIN:-/usr/bin/rmac-terminal}
notes_bin=${RMAC_NOTES_BIN:-/usr/bin/rmac-notes}
monitor_bin=${RMAC_MONITOR_BIN:-/usr/bin/rmac-system-monitor}
text_editor_bin=${RMAC_TEXT_EDITOR_BIN:-/usr/bin/rmac-text-editor}
shortcut_dispatch_bin=${RMAC_SHORTCUT_DISPATCH_BIN:-/usr/libexec/rmac/rmac-shortcut-dispatch}

if [ -z "${NIRI_SOCKET:-}" ]; then
    NIRI_SOCKET=$(find "${runtime}" -maxdepth 1 -type s -name 'niri.*.sock' -print -quit)
    export NIRI_SOCKET
fi

for command in grim niri python3; do
    command -v "${command}" >/dev/null 2>&1 || {
        echo "capture-live-surfaces: missing ${command}" >&2
        exit 1
    }
done
if [ -z "${NIRI_SOCKET:-}" ]; then
    echo "capture-live-surfaces: no active niri IPC socket" >&2
    exit 1
fi

rm -rf -- "${out}"
install -d -m 0700 "${out}"

window_ids() {
    app_id=$1
    niri msg -j windows | python3 -c '
import json, sys
app_id = sys.argv[1]
for window in json.load(sys.stdin):
    if window.get("app_id") == app_id:
        print(window["id"])
' "${app_id}"
}

wait_for_window() {
    app_id=$1
    attempt=0
    while [ "${attempt}" -lt 50 ]; do
        id=$(window_ids "${app_id}" | head -n 1)
        if [ -n "${id}" ]; then
            printf '%s\n' "${id}"
            return 0
        fi
        attempt=$((attempt + 1))
        sleep 0.1
    done
    return 1
}

shot() {
    name=$1
    sleep "${settle}"
    grim "${out}/${name}.png"
    niri msg -j windows >"${out}/${name}-windows.json"
    niri msg -j layers >"${out}/${name}-layers.json"
    echo "captured ${name}.png"
}

capture_app() {
    name=$1
    app_id=$2
    shift 2
    existing=$(window_ids "${app_id}" | head -n 1)
    launched=false
    if [ -z "${existing}" ]; then
        "$@" >"${out}/${name}.log" 2>&1 &
        launched=true
    fi
    id=$(wait_for_window "${app_id}") || {
        echo "capture-live-surfaces: ${app_id} did not open" >&2
        return 1
    }
    niri msg action focus-window --id "${id}" >/dev/null
    shot "${name}"
    if [ "${launched}" = true ]; then
        niri msg action close-window --id "${id}" >/dev/null 2>&1 || true
        sleep 0.2
    fi
}

capture_overlay() {
    name=$1
    endpoint=$2
    "${shortcut_dispatch_bin}" "${endpoint}" \
        >"${out}/${name}.log" 2>&1 || true
    shot "${name}"
    "${shortcut_dispatch_bin}" "${endpoint}" >/dev/null 2>&1 || true
}

{
    printf 'captured_at=%s\n' "$(date --iso-8601=seconds)"
    printf 'git_head=%s\n' "$(git -C "${HOME}/rmac" rev-parse HEAD 2>/dev/null || echo unavailable)"
    printf 'niri_socket=%s\n' "${NIRI_SOCKET}"
    printf 'niri_config=%s\n' "$(systemctl --user show-environment | sed -n 's/^NIRI_CONFIG=//p')"
    journalctl --user -u niri.service -b --no-pager -o cat 2>/dev/null \
        | sed -n 's/.*loaded config from "\([^"]*\)".*/loaded_config=\1/p' \
        | tail -n 1
} >"${out}/session.txt"

capture_app app-files org.rmac.Files "${files_bin}"
capture_app app-settings org.rmac.SystemSettings "${settings_bin}"
capture_app app-terminal org.rmac.Terminal "${terminal_bin}"
capture_app app-notes org.rmac.Notes "${notes_bin}"
capture_app app-monitor org.rmac.SystemMonitor "${monitor_bin}"
capture_app app-texteditor org.rmac.TextEditor "${text_editor_bin}" --new-document

capture_overlay overlay-launcher launcher
capture_overlay overlay-quick-settings quick-settings
capture_overlay overlay-notification-center notification-center
capture_overlay overlay-app-drawer app-drawer

echo "live surface evidence: ${out}"
