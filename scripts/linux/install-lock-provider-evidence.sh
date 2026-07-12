#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "${script_dir}/../.." && pwd)
asset_dir="${repo_root}/crates/rmac-lock-provider-linux/evidence"
config_home=${XDG_CONFIG_HOME:-"${HOME}/.config"}
unit_dir=${RMAC_SYSTEMD_USER_DIR:-"${config_home}/systemd/user"}
libexec_dir="${HOME}/.local/libexec/rmac-evidence"
target_dir=${CARGO_TARGET_DIR:-target}
minimum_build_kib=$((25 * 1024 * 1024))

if [ "$(uname -s)" != Linux ]; then
    echo "lock-provider evidence installation requires Linux" >&2
    exit 1
fi
if [ "$(id -u)" -eq 0 ]; then
    echo "lock-provider evidence must run as the graphical test user, not root" >&2
    exit 1
fi
for executable in /usr/bin/systemctl /usr/bin/systemd-notify /usr/bin/sway /usr/bin/swaylock; do
    if [ ! -x "${executable}" ]; then
        echo "required evidence executable is unavailable: ${executable}" >&2
        exit 1
    fi
done
if [ ! -f /etc/pam.d/rmac-lock ]; then
    echo "review and install crates/rmac-lock-provider-linux/pam/rmac-lock before evidence setup" >&2
    exit 1
fi
if [ ! -x "${HOME}/.local/libexec/rmac/rmac-locker" ] ||
   [ ! -f "${config_home}/rmac/swaylock.conf" ]; then
    echo "install the normal swaylock-backed rmac session units before evidence setup" >&2
    exit 1
fi

available_kib=$(df -Pk "${repo_root}" | awk 'NR == 2 { print $4 }')
case ${available_kib} in
    ''|*[!0-9]*)
        echo "could not determine available build storage" >&2
        exit 1
        ;;
esac
if [ "${available_kib}" -lt "${minimum_build_kib}" ]; then
    echo "at least 25 GiB free is required before the evidence release build" >&2
    exit 1
fi

case ${target_dir} in
    /*) ;;
    *) target_dir="${repo_root}/${target_dir}" ;;
esac

(cd "${repo_root}" && cargo build --locked --release \
    -p rmac-lock-provider-linux --features development-provider \
    --bin rmac-lock-provider)

install -d -m 0755 "${unit_dir}" "${libexec_dir}"
install -m 0755 "${target_dir}/release/rmac-lock-provider" \
    "${libexec_dir}/rmac-lock-provider"
install -m 0755 "${script_dir}/launch-lock-provider-evidence.sh" \
    "${libexec_dir}/rmac-lock-provider-evidence-launch"
install -m 0644 "${asset_dir}/units/rmac-lock-provider-evidence.service" \
    "${unit_dir}/rmac-lock-provider-evidence.service"
install -m 0644 "${asset_dir}/units/rmac-lock-fallback-evidence.service" \
    "${unit_dir}/rmac-lock-fallback-evidence.service"
/usr/bin/systemctl --user daemon-reload

echo "Installed opt-in nested lock evidence assets; no provider was started or enabled."
echo "Run scripts/linux/run-lock-provider-recovery-gate.sh only from the test user's local graphical session."
