#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "${script_dir}/../.." && pwd)
source_dir="${repo_root}/crates/rmac-session/units"
notification_install_dir="${repo_root}/crates/rmac-notifications-linux/install"
config_home=${XDG_CONFIG_HOME:-"${HOME}/.config"}
data_home=${XDG_DATA_HOME:-"${HOME}/.local/share"}
unit_dir=${RMAC_SYSTEMD_USER_DIR:-"${config_home}/systemd/user"}
libexec_dir="${HOME}/.local/libexec/rmac"
bin_dir=${RMAC_BIN_DIR:-"${HOME}/.local/bin"}
target_dir=${CARGO_TARGET_DIR:-target}
case ${target_dir} in
    /*) ;;
    *) target_dir="${repo_root}/${target_dir}" ;;
esac

(cd "${repo_root}" && cargo build --locked --release \
    -p rmac-session --bin rmac-session-supervisor \
    -p rmac-notifications-linux --bin rmac-notification-center \
    -p rmac-shortcuts --bin rmac-shortcut-broker --bin rmac-shortcut-dispatch)

install -d -m 0755 "${unit_dir}"
install -d -m 0755 "${libexec_dir}"
install -d -m 0755 "${bin_dir}"
install -m 0755 "${target_dir}/release/rmac-session-supervisor" "${libexec_dir}/rmac-session-supervisor"
install -m 0755 "${target_dir}/release/rmac-notification-center" "${libexec_dir}/rmac-notification-center"
install -m 0755 "${target_dir}/release/rmac-shortcut-broker" "${libexec_dir}/rmac-shortcut-broker"
install -m 0755 "${target_dir}/release/rmac-shortcut-dispatch" "${libexec_dir}/rmac-shortcut-dispatch"
install -m 0755 "${script_dir}/start-rmac-session.sh" "${bin_dir}/rmac-session-start"
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
rm -f "${activation_tmp}"
trap - EXIT HUP INT TERM

systemctl --user daemon-reload
fallback_path="${config_home}/rmac/niri-shortcuts.kdl"
"${libexec_dir}/rmac-shortcut-dispatch" write-niri-fallback \
    "${fallback_path}" "${libexec_dir}/rmac-shortcut-dispatch"
echo "Installed rmac user units in ${unit_dir}."
echo "Installed the supervisor in ${libexec_dir}."
echo "Installed the notification service and rmac notification portal backend."
echo "Start the session from niri with ${bin_dir}/rmac-session-start."
echo "If shortcuts-status.json reports fallback-required, add this to niri config:"
echo "include \"${fallback_path}\""
