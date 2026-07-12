#!/bin/sh
set -eu

# Import only graphical-session routing values. Never copy the whole login
# environment because it can contain credentials and application secrets.
state_home=${XDG_STATE_HOME:-"${HOME}/.local/state"}
normal_session=true
if [ -f "${state_home}/rmac/session/safe-mode.json" ]; then
    normal_session=false
else
    case ":${XDG_CURRENT_DESKTOP:-}:" in
        *:rmac:*) ;;
        ::) XDG_CURRENT_DESKTOP=rmac ;;
        *) XDG_CURRENT_DESKTOP="rmac:${XDG_CURRENT_DESKTOP}" ;;
    esac
    export XDG_CURRENT_DESKTOP
fi

set --
[ "${WAYLAND_DISPLAY+x}" = x ] && set -- "$@" WAYLAND_DISPLAY
[ "${DISPLAY+x}" = x ] && set -- "$@" DISPLAY
[ "${XAUTHORITY+x}" = x ] && set -- "$@" XAUTHORITY
[ "${XDG_CURRENT_DESKTOP+x}" = x ] && set -- "$@" XDG_CURRENT_DESKTOP
[ "${XDG_SESSION_DESKTOP+x}" = x ] && set -- "$@" XDG_SESSION_DESKTOP
[ "${XDG_SESSION_TYPE+x}" = x ] && set -- "$@" XDG_SESSION_TYPE
[ "${XDG_RUNTIME_DIR+x}" = x ] && set -- "$@" XDG_RUNTIME_DIR
[ "${DBUS_SESSION_BUS_ADDRESS+x}" = x ] && set -- "$@" DBUS_SESSION_BUS_ADDRESS
[ "${NIRI_SOCKET+x}" = x ] && set -- "$@" NIRI_SOCKET

if [ "$#" -gt 0 ]; then
    systemctl --user import-environment "$@"
    if command -v dbus-update-activation-environment >/dev/null 2>&1; then
        dbus-update-activation-environment --systemd "$@"
    fi
fi

if [ "${normal_session}" = false ]; then
    systemctl --user start rmac-safe-mode.target
else
    systemctl --user start rmac-session.target
    # xdg-desktop-portal reads desktop-specific backend selection at startup.
    # Restart only an already-running frontend after the rmac backend is ready.
    systemctl --user try-restart xdg-desktop-portal.service
fi
