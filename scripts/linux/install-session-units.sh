#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "${script_dir}/../.." && pwd)
source_dir="${repo_root}/crates/rmac-session/units"
notification_install_dir="${repo_root}/crates/rmac-notifications-linux/install"
focus_install_dir="${repo_root}/crates/rmac-focus-linux/install"
lock_config_source="${repo_root}/crates/rmac-session/swaylock.conf"
lock_policy_source="${repo_root}/crates/rmac-session/lock-policy.json"
config_home=${XDG_CONFIG_HOME:-"${HOME}/.config"}
data_home=${XDG_DATA_HOME:-"${HOME}/.local/share"}
unit_dir=${RMAC_SYSTEMD_USER_DIR:-"${config_home}/systemd/user"}
libexec_dir="${HOME}/.local/libexec/rmac"
bin_dir=${RMAC_BIN_DIR:-"${HOME}/.local/bin"}
target_dir=${CARGO_TARGET_DIR:-target}
if [ ! -x /usr/bin/swaylock ]; then
    echo "swaylock is required at /usr/bin/swaylock for secure session locking." >&2
    exit 1
fi
if [ ! -x /usr/bin/systemd-notify ]; then
    echo "systemd-notify is required at /usr/bin/systemd-notify." >&2
    exit 1
fi
if [ ! -x /usr/bin/swayidle ]; then
    echo "swayidle is required at /usr/bin/swayidle for idle session locking." >&2
    exit 1
fi
if [ ! -x /usr/bin/busctl ]; then
    echo "busctl is required at /usr/bin/busctl for automatic suspend requests." >&2
    exit 1
fi
case ${target_dir} in
    /*) ;;
    *) target_dir="${repo_root}/${target_dir}" ;;
esac

(cd "${repo_root}" && cargo build --locked --release \
    -p rmac-session --bin rmac-session-supervisor \
    -p rmac-launcher-app --bin rmac-launcher \
    -p rmac-app-drawer --bin rmac-app-drawer \
    -p rmac-quick-settings-app --bin rmac-quick-settings \
    -p rmac-notification-center-app --bin rmac-notification-center-panel \
    -p rmac-system-settings --bin rmac-system-settings \
    -p rmac-notifications-linux --bin rmac-notification-center \
    -p rmac-focus-linux --bin rmac-focus-service \
    -p rmac-shortcuts --bin rmac-shortcut-broker --bin rmac-shortcut-dispatch --bin rmac-locker --bin rmac-lock-coordinator --bin rmac-idle-locker)

install -d -m 0755 "${unit_dir}"
install -d -m 0755 "${libexec_dir}"
install -d -m 0755 "${bin_dir}"
install -m 0755 "${target_dir}/release/rmac-session-supervisor" "${libexec_dir}/rmac-session-supervisor"
install -m 0755 "${target_dir}/release/rmac-launcher" "${libexec_dir}/rmac-launcher"
install -m 0755 "${target_dir}/release/rmac-app-drawer" "${libexec_dir}/rmac-app-drawer"
install -m 0755 "${target_dir}/release/rmac-quick-settings" "${libexec_dir}/rmac-quick-settings"
install -m 0755 "${target_dir}/release/rmac-notification-center-panel" "${libexec_dir}/rmac-notification-center-panel"
install -m 0755 "${target_dir}/release/rmac-system-settings" "${libexec_dir}/rmac-system-settings"
install -m 0755 "${target_dir}/release/rmac-notification-center" "${libexec_dir}/rmac-notification-center"
install -m 0755 "${target_dir}/release/rmac-focus-service" "${libexec_dir}/rmac-focus-service"
install -m 0755 "${target_dir}/release/rmac-shortcut-broker" "${libexec_dir}/rmac-shortcut-broker"
install -m 0755 "${target_dir}/release/rmac-shortcut-dispatch" "${libexec_dir}/rmac-shortcut-dispatch"
install -m 0755 "${target_dir}/release/rmac-locker" "${libexec_dir}/rmac-locker"
install -m 0755 "${target_dir}/release/rmac-lock-coordinator" "${libexec_dir}/rmac-lock-coordinator"
install -m 0755 "${target_dir}/release/rmac-idle-locker" "${libexec_dir}/rmac-idle-locker"
install -m 0755 "${script_dir}/start-rmac-session.sh" "${bin_dir}/rmac-session-start"
install -d -m 0755 "${config_home}/rmac"
if [ ! -e "${config_home}/rmac/swaylock.conf" ]; then
    install -m 0644 "${lock_config_source}" "${config_home}/rmac/swaylock.conf"
fi
if [ ! -e "${config_home}/rmac/lock-policy.json" ]; then
    install -m 0600 "${lock_policy_source}" "${config_home}/rmac/lock-policy.json"
fi
for unit in "${source_dir}"/*; do
    install -m 0644 "${unit}" "${unit_dir}/$(basename -- "${unit}")"
done

portal_dir="${data_home}/xdg-desktop-portal/portals"
portal_config_dir="${data_home}/xdg-desktop-portal"
dbus_service_dir="${data_home}/dbus-1/services"
install -d -m 0755 "${portal_dir}" "${portal_config_dir}" "${dbus_service_dir}"
install -m 0644 "${notification_install_dir}/rmac.portal" "${portal_dir}/rmac.portal"
install -m 0644 "${notification_install_dir}/rmac-portals.conf" "${portal_config_dir}/rmac-portals.conf"
activation_tmp=$(mktemp "${TMPDIR:-/tmp}/rmac-portal-service.XXXXXX")
trap 'rm -f "${activation_tmp}"' EXIT HUP INT TERM
sed "s|@RMAC_NOTIFICATION_EXEC@|${libexec_dir}/rmac-notification-center|g" \
    "${notification_install_dir}/org.freedesktop.impl.portal.desktop.rmac.service.in" \
    >"${activation_tmp}"
install -m 0644 "${activation_tmp}" \
    "${dbus_service_dir}/org.freedesktop.impl.portal.desktop.rmac.service"
sed "s|@RMAC_NOTIFICATION_EXEC@|${libexec_dir}/rmac-notification-center|g" \
    "${notification_install_dir}/org.rmac.NotificationCenter1.service.in" \
    >"${activation_tmp}"
install -m 0644 "${activation_tmp}" \
    "${dbus_service_dir}/org.rmac.NotificationCenter1.service"
rm -f "${activation_tmp}"
trap - EXIT HUP INT TERM

focus_activation_tmp=$(mktemp "${TMPDIR:-/tmp}/rmac-focus-service.XXXXXX")
trap 'rm -f "${focus_activation_tmp}"' EXIT HUP INT TERM
sed "s|@RMAC_FOCUS_EXEC@|${libexec_dir}/rmac-focus-service|g" \
    "${focus_install_dir}/org.rmac.Focus1.service.in" \
    >"${focus_activation_tmp}"
install -m 0644 "${focus_activation_tmp}" \
    "${dbus_service_dir}/org.rmac.Focus1.service"
rm -f "${focus_activation_tmp}"
trap - EXIT HUP INT TERM

systemctl --user daemon-reload
fallback_path="${config_home}/rmac/niri-shortcuts.kdl"
"${libexec_dir}/rmac-shortcut-dispatch" write-niri-fallback \
    "${fallback_path}" "${libexec_dir}/rmac-shortcut-dispatch"
echo "Installed rmac user units in ${unit_dir}."
echo "Installed the supervisor in ${libexec_dir}."
echo "Installed the supervised App Drawer, notification service, on-demand Center and Quick Settings panels, and rmac notification portal backend."
echo "Installed the Focus policy authority."
echo "Installed secure swaylock supervision, logind coordination, idle locking, and default lock policy."
echo "The three upstream shell surfaces remain an explicit framework-gated development candidate."
echo "Check them with: bash ${script_dir}/install-upstream-shell-candidate.sh --check"
echo "Start the session from niri with ${bin_dir}/rmac-session-start."
echo "If shortcuts-status.json reports fallback-required, add this to niri config:"
echo "include \"${fallback_path}\""
