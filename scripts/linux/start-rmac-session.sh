#!/bin/sh
set -eu

# The development installer creates these files directly. A native package
# keeps immutable defaults under /usr and provisions only missing user-owned
# copies on first session start. Never replace a user's existing policy.
case ${1-} in
    "")
        ;;
    --system-package)
        shift
        defaults_dir=/usr/share/rmac/session
        config_home=${XDG_CONFIG_HOME:-"${HOME}/.config"}
        if [ ! -f "${defaults_dir}/swaylock.conf" ] ||
            [ -L "${defaults_dir}/swaylock.conf" ] ||
            [ ! -f "${defaults_dir}/lock-policy.json" ] ||
            [ -L "${defaults_dir}/lock-policy.json" ]; then
            echo "rmac session defaults are unavailable" >&2
            exit 1
        fi
        /usr/bin/install -d -m 0700 "${config_home}/rmac"
        if [ ! -e "${config_home}/rmac/swaylock.conf" ]; then
            /usr/bin/install -m 0600 \
                "${defaults_dir}/swaylock.conf" \
                "${config_home}/rmac/swaylock.conf"
        fi
        if [ ! -e "${config_home}/rmac/lock-policy.json" ]; then
            /usr/bin/install -m 0600 \
                "${defaults_dir}/lock-policy.json" \
                "${config_home}/rmac/lock-policy.json"
        fi
        ;;
    *)
        echo "usage: rmac-session-start [--system-package]" >&2
        exit 2
        ;;
esac
if [ "$#" -ne 0 ]; then
    echo "usage: rmac-session-start [--system-package]" >&2
    exit 2
fi

# Import only graphical-session routing values. Never copy the whole login
# environment because it can contain credentials and application secrets.
state_home=${XDG_STATE_HOME:-"${HOME}/.local/state"}
normal_session=true
graphical_invocation=false
case ${XDG_SESSION_TYPE:-} in
    wayland)
        [ -n "${WAYLAND_DISPLAY:-}" ] && graphical_invocation=true
        ;;
    x11)
        [ -n "${DISPLAY:-}" ] && graphical_invocation=true
        ;;
esac
if [ -f "${state_home}/rmac/session/safe-mode.json" ]; then
    normal_session=false
elif [ "${graphical_invocation}" = true ]; then
    case ":${XDG_CURRENT_DESKTOP:-}:" in
        *:rmac:*) ;;
        ::) XDG_CURRENT_DESKTOP=rmac ;;
        *) XDG_CURRENT_DESKTOP="rmac:${XDG_CURRENT_DESKTOP}" ;;
    esac
    export XDG_CURRENT_DESKTOP
    # Original rmac pointer theme (FEEL_SPEC.md §D.2); the compositor reads the
    # niri cursor rule, this is for GTK/Qt/Electron clients.
    export XCURSOR_THEME="rmac"
    export XCURSOR_SIZE="24"
fi

set --
if [ "${graphical_invocation}" = true ]; then
    [ "${WAYLAND_DISPLAY+x}" = x ] && set -- "$@" WAYLAND_DISPLAY
    [ "${DISPLAY+x}" = x ] && set -- "$@" DISPLAY
    [ "${XAUTHORITY+x}" = x ] && set -- "$@" XAUTHORITY
    [ "${XDG_CURRENT_DESKTOP+x}" = x ] && set -- "$@" XDG_CURRENT_DESKTOP
    [ "${XDG_SESSION_ID+x}" = x ] && set -- "$@" XDG_SESSION_ID
    [ "${XDG_SESSION_DESKTOP+x}" = x ] && set -- "$@" XDG_SESSION_DESKTOP
    [ "${XDG_SESSION_TYPE+x}" = x ] && set -- "$@" XDG_SESSION_TYPE
    [ "${XDG_RUNTIME_DIR+x}" = x ] && set -- "$@" XDG_RUNTIME_DIR
    [ "${DBUS_SESSION_BUS_ADDRESS+x}" = x ] && set -- "$@" DBUS_SESSION_BUS_ADDRESS
    [ "${NIRI_SOCKET+x}" = x ] && set -- "$@" NIRI_SOCKET
    [ "${RMAC_COLOR_SCHEME+x}" = x ] && set -- "$@" RMAC_COLOR_SCHEME
    [ "${XCURSOR_THEME+x}" = x ] && set -- "$@" XCURSOR_THEME
    [ "${XCURSOR_SIZE+x}" = x ] && set -- "$@" XCURSOR_SIZE
fi

if [ "$#" -gt 0 ]; then
    /usr/bin/systemctl --user import-environment "$@"
    if [ -x /usr/bin/dbus-update-activation-environment ]; then
        /usr/bin/dbus-update-activation-environment --systemd "$@"
    fi
fi

if [ "${normal_session}" = false ]; then
    /usr/bin/systemctl --user start rmac-safe-mode.target
else
    /usr/bin/systemctl --user start rmac-session.target
    # xdg-desktop-portal reads desktop-specific backend selection at startup.
    # Restart only an already-running frontend after the rmac backend is ready.
    /usr/bin/systemctl --user try-restart xdg-desktop-portal.service
fi
