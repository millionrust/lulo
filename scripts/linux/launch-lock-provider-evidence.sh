#!/bin/sh
set -eu

display=${WAYLAND_DISPLAY:-}
case ${display} in
    wayland-*) display_number=${display#wayland-} ;;
    *) display_number= ;;
esac
case ${display_number} in
    ''|*[!0-9]*)
        echo "nested evidence launcher received an invalid Wayland display" >&2
        exit 1
        ;;
esac
case ${XDG_SESSION_ID:-} in
    ''|*[!A-Za-z0-9_.-]*)
        echo "nested evidence launcher received an invalid session ID" >&2
        exit 1
        ;;
esac
if [ "${#XDG_SESSION_ID}" -gt 256 ]; then
    echo "nested evidence launcher received an oversized session ID" >&2
    exit 1
fi

expected_runtime_dir="/run/user/$(id -u)"
if [ "${XDG_RUNTIME_DIR:-}" != "${expected_runtime_dir}" ]; then
    echo "nested evidence launcher requires the test user's standard runtime directory" >&2
    exit 1
fi
runtime_dir="${expected_runtime_dir}/rmac-lock-evidence"
environment_file="${runtime_dir}/environment"
temporary="${environment_file}.tmp.$$"
umask 077
mkdir -p "${runtime_dir}"
trap 'rm -f "${temporary}"' EXIT HUP INT TERM
{
    printf 'WAYLAND_DISPLAY=%s\n' "${display}"
    printf 'XDG_SESSION_ID=%s\n' "${XDG_SESSION_ID}"
} >"${temporary}"
mv -f "${temporary}" "${environment_file}"
trap - EXIT HUP INT TERM

/usr/bin/systemctl --user start rmac-lock-provider-evidence.service
