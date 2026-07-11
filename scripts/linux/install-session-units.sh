#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "${script_dir}/../.." && pwd)
source_dir="${repo_root}/crates/rmac-session/units"
config_home=${XDG_CONFIG_HOME:-"${HOME}/.config"}
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
    -p rmac-shortcuts --bin rmac-shortcut-broker --bin rmac-shortcut-dispatch)

install -d -m 0755 "${unit_dir}"
install -d -m 0755 "${libexec_dir}"
install -d -m 0755 "${bin_dir}"
install -m 0755 "${target_dir}/release/rmac-session-supervisor" "${libexec_dir}/rmac-session-supervisor"
install -m 0755 "${target_dir}/release/rmac-shortcut-broker" "${libexec_dir}/rmac-shortcut-broker"
install -m 0755 "${target_dir}/release/rmac-shortcut-dispatch" "${libexec_dir}/rmac-shortcut-dispatch"
install -m 0755 "${script_dir}/start-rmac-session.sh" "${bin_dir}/rmac-session-start"
for unit in "${source_dir}"/*; do
    install -m 0644 "${unit}" "${unit_dir}/$(basename -- "${unit}")"
done

systemctl --user daemon-reload
fallback_path="${config_home}/rmac/niri-shortcuts.kdl"
"${libexec_dir}/rmac-shortcut-dispatch" write-niri-fallback \
    "${fallback_path}" "${libexec_dir}/rmac-shortcut-dispatch"
echo "Installed rmac user units in ${unit_dir}."
echo "Installed the supervisor in ${libexec_dir}."
echo "Start the session from niri with ${bin_dir}/rmac-session-start."
echo "If shortcuts-status.json reports fallback-required, add this to niri config:"
echo "include \"${fallback_path}\""
