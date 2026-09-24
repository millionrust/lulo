#!/bin/sh
set -eu

usage()
{
    echo "usage: rmac-session-start [--system-package] [--safe-mode | --clear-safe-mode]" >&2
    exit 2
}

# --safe-mode: the package login wrapper already consumed the safe-mode marker
#   for this login and chose safe mode.
# --clear-safe-mode: leave safe mode now, without logging out. Clears the
#   marker, points niri back at the rmac configuration and starts the normal
#   rmac session. If niri cannot switch its configuration live, it says so and
#   the next login starts normally.
system_package=false
safe_mode_requested=false
clear_safe_mode=false
for argument do
    case ${argument} in
        --system-package) system_package=true ;;
        --safe-mode) safe_mode_requested=true ;;
        --clear-safe-mode) clear_safe_mode=true ;;
        *) usage ;;
    esac
done
if [ "${safe_mode_requested}" = true ] && [ "${clear_safe_mode}" = true ]; then
    usage
fi
set --

# The development installer creates these files directly. A native package
# keeps immutable defaults under /usr and provisions only missing user-owned
# copies on first session start. Never replace a user's existing policy.
case ${system_package} in
    false)
        ;;
    true)
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
esac

supervisor=${HOME}/.local/libexec/rmac/rmac-session-supervisor
if [ "${system_package}" = true ] || [ ! -x "${supervisor}" ]; then
    supervisor=/usr/libexec/rmac/rmac-session-supervisor
fi
state_home=${XDG_STATE_HOME:-"${HOME}/.local/state"}
safe_mode_marker=${state_home}/rmac/session/safe-mode.json
normal_session=true
if [ "${clear_safe_mode}" = true ]; then
    if [ ! -x "${supervisor}" ]; then
        echo "rmac-session-supervisor is not installed" >&2
        exit 1
    fi
    "${supervisor}" leave-safe-mode
    # The wrapper gave the safe login niri's own identity; this is rmac again.
    XDG_SESSION_DESKTOP=rmac
    export XDG_SESSION_DESKTOP
elif [ "${safe_mode_requested}" = true ]; then
    normal_session=false
elif [ "${system_package}" = false ] &&
    { [ -e "${safe_mode_marker}" ] || [ -L "${safe_mode_marker}" ]; }; then
    # A development session has no login wrapper, so the marker is consumed
    # here: safe mode lasts this start only.
    login_mode=
    if [ -x "${supervisor}" ]; then
        login_mode=$("${supervisor}" begin-login) || login_mode=
    fi
    case ${login_mode} in
        normal) ;;
        safe) normal_session=false ;;
        *)
            normal_session=false
            /usr/bin/mv -f "${safe_mode_marker}" \
                "${state_home}/rmac/session/safe-mode.last.json" ||
                echo "rmac could not clear safe mode for the next start" >&2
            ;;
    esac
fi

# Import only graphical-session routing values. Never copy the whole login
# environment because it can contain credentials and application secrets.
graphical_invocation=false
case ${XDG_SESSION_TYPE:-} in
    wayland)
        [ -n "${WAYLAND_DISPLAY:-}" ] && graphical_invocation=true
        ;;
    x11)
        [ -n "${DISPLAY:-}" ] && graphical_invocation=true
        ;;
esac
if [ "${normal_session}" = true ] && [ "${graphical_invocation}" = true ]; then
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
    # Qt apps take their font, palette and dark mode from the rmac GTK theme
    # through Qt's GTK 3 platform theme (qt6-gtk-platformtheme); without the
    # plugin Qt warns once and keeps its own defaults (FEEL_SPEC.md §D.7).
    export QT_QPA_PLATFORMTHEME="gtk3"
    # Qt's built-in Wayland title bar looks like no desktop at all. Use the
    # GNOME-style decoration plugin when it is installed.
    for qt_decoration in \
        /usr/lib/*/qt6/plugins/wayland-decoration-client/libqadwaitadecorations.so; do
        if [ -f "${qt_decoration}" ]; then
            export QT_WAYLAND_DECORATION="adwaita"
            break
        fi
    done
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
    [ "${QT_QPA_PLATFORMTHEME+x}" = x ] && set -- "$@" QT_QPA_PLATFORMTHEME
    [ "${QT_WAYLAND_DECORATION+x}" = x ] && set -- "$@" QT_WAYLAND_DECORATION
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
    if [ "${clear_safe_mode}" = true ]; then
        # As the login wrapper does for a normal login: rmac owns the only
        # menu bar, so stop the bar the plain niri session may have started.
        /usr/bin/systemctl --user stop waybar.service >/dev/null 2>&1 || :
    fi
    /usr/bin/systemctl --user start rmac-session.target
    if [ -x /usr/libexec/rmac/rmac-sound ]; then
        /usr/libexec/rmac/rmac-sound login >/dev/null 2>&1 &
    fi
    # xdg-desktop-portal reads desktop-specific backend selection at startup.
    # Restart only an already-running frontend after the rmac backend is ready.
    /usr/bin/systemctl --user try-restart xdg-desktop-portal.service
fi
