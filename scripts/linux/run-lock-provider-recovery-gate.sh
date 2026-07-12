#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/../.." && pwd)"
config="${repo_root}/crates/rmac-lock-provider-linux/evidence/nested-sway.conf"
config_home=${XDG_CONFIG_HOME:-"${HOME}/.config"}
unit_dir=${RMAC_SYSTEMD_USER_DIR:-"${config_home}/systemd/user"}
libexec_dir="${HOME}/.local/libexec/rmac-evidence"
runtime_dir="/run/user/$(id -u)/rmac-lock-evidence"
evidence_dir=
report=
custom_unit=rmac-lock-provider-evidence.service
fallback_unit=rmac-lock-fallback-evidence.service
sway_pid=

usage() {
  echo "usage: $0 --execute" >&2
}

if [[ ${1:-} != --execute || $# -ne 1 ]]; then
  usage
  exit 2
fi
if [[ $(uname -s) != Linux || $(id -u) -eq 0 ]]; then
  echo "run this gate as a non-root Linux graphical test user" >&2
  exit 1
fi
if [[ -n ${SSH_CONNECTION:-} || -n ${SSH_TTY:-} ]]; then
  echo "remote execution is forbidden; local TTY recovery must remain available" >&2
  exit 1
fi
if [[ -z ${WAYLAND_DISPLAY:-} || -z ${XDG_SESSION_ID:-} || -z ${XDG_RUNTIME_DIR:-} ]]; then
  echo "WAYLAND_DISPLAY, XDG_SESSION_ID, and XDG_RUNTIME_DIR are required" >&2
  exit 1
fi
if [[ ${XDG_RUNTIME_DIR} != "/run/user/$(id -u)" ]]; then
  echo "the supported Ubuntu user runtime directory is required" >&2
  exit 1
fi
for path in \
  /usr/bin/busctl \
  /usr/bin/sway \
  /usr/bin/systemctl \
  "${libexec_dir}/rmac-lock-provider" \
  "${libexec_dir}/rmac-lock-provider-evidence-launch"; do
  if [[ ! -x ${path} ]]; then
    echo "required recovery-gate asset is unavailable: ${path}" >&2
    exit 1
  fi
done
for path in "${unit_dir}/${custom_unit}" "${unit_dir}/${fallback_unit}"; do
  if [[ ! -f ${path} ]]; then
    echo "required recovery-gate unit is unavailable: ${path}" >&2
    exit 1
  fi
done

echo "This test opens a nested compositor, kills its lock provider, and requires"
echo "you to authenticate through swaylock before the test can finish."
read -r -p "Type NESTED-LOCK-RECOVERY to continue: " confirmation </dev/tty
if [[ ${confirmation} != NESTED-LOCK-RECOVERY ]]; then
  echo "recovery gate cancelled" >&2
  exit 1
fi

unit_value() {
  /usr/bin/systemctl --user show "$1" --property="$2" --value 2>/dev/null
}

wait_for_active() {
  local unit=$1
  local attempts=0
  while (( attempts < 150 )); do
    if [[ $(unit_value "${unit}" ActiveState) == active ]]; then
      return 0
    fi
    if [[ -n ${sway_pid} ]] && ! kill -0 "${sway_pid}" 2>/dev/null; then
      return 1
    fi
    sleep 0.1
    ((attempts += 1))
  done
  return 1
}

cleanup() {
  local gate_status=$?
  set +e
  /usr/bin/systemctl --user stop "${custom_unit}" "${fallback_unit}" >/dev/null 2>&1
  if [[ -n ${sway_pid} ]] && kill -0 "${sway_pid}" 2>/dev/null; then
    kill "${sway_pid}" >/dev/null 2>&1
    wait "${sway_pid}" >/dev/null 2>&1
  fi
  /usr/bin/busctl call org.freedesktop.login1 \
    /org/freedesktop/login1/session/auto \
    org.freedesktop.login1.Session SetLockedHint b false >/dev/null 2>&1
  rm -rf "${runtime_dir}"
  if [[ -n ${report} ]]; then
    if [[ ${gate_status} -eq 0 ]]; then
      printf 'result=pass\n' >>"${report}"
    else
      printf 'result=fail\n' >>"${report}"
    fi
    printf 'completed_utc=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >>"${report}"
    echo "Redacted recovery evidence: ${report}"
  fi
}
trap cleanup EXIT
trap 'exit 130' HUP INT TERM

evidence_dir="${repo_root}/target/linux-evidence/$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "${evidence_dir}"
report="${evidence_dir}/lock-provider-recovery.txt"
printf 'gate=lock-provider-nested-recovery\n' >"${report}"
printf 'started_utc=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >>"${report}"

/usr/bin/systemctl --user stop "${custom_unit}" "${fallback_unit}" >/dev/null 2>&1 || true
rm -rf "${runtime_dir}"
umask 077
mkdir -p "${runtime_dir}"

PATH="${libexec_dir}:${PATH}" \
WLR_BACKENDS=wayland \
WLR_RENDERER=pixman \
WLR_WL_OUTPUTS=1 \
/usr/bin/sway --config "${config}" >"${runtime_dir}/nested-sway.log" 2>&1 &
sway_pid=$!

if ! wait_for_active "${custom_unit}"; then
  echo "custom provider did not securely lock the nested compositor" >&2
  exit 1
fi
first_pid=$(unit_value "${custom_unit}" MainPID)
first_restarts=$(unit_value "${custom_unit}" NRestarts)
case ${first_pid} in
  ''|0|*[!0-9]*)
    echo "custom provider did not expose a valid main PID" >&2
    exit 1
    ;;
esac
case ${first_restarts} in
  ''|*[!0-9]*)
    echo "custom provider did not expose a valid restart count" >&2
    exit 1
    ;;
esac

/usr/bin/systemctl --user kill --kill-whom=main --signal=KILL "${custom_unit}"
attempts=0
restarted=false
while (( attempts < 150 )); do
  next_pid=$(unit_value "${custom_unit}" MainPID)
  restarts=$(unit_value "${custom_unit}" NRestarts)
  if [[ ${next_pid} =~ ^[1-9][0-9]*$ && ${next_pid} != "${first_pid}" &&
        ${restarts} =~ ^[0-9]+$ && ${restarts} -gt ${first_restarts} &&
        $(unit_value "${custom_unit}" ActiveState) == active ]]; then
    restarted=true
    break
  fi
  sleep 0.1
  ((attempts += 1))
done
if [[ ${restarted} != true ]]; then
  echo "custom provider did not recover after SIGKILL" >&2
  exit 1
fi
echo "Automatic custom-provider restart passed."
printf 'custom_restart=pass\n' >>"${report}"
printf 'custom_restart_count=%s\n' "${restarts}" >>"${report}"

/usr/bin/systemctl --user stop "${custom_unit}"
/usr/bin/systemctl --user start --no-block "${fallback_unit}"
if ! wait_for_active "${fallback_unit}"; then
  echo "swaylock fallback did not acquire the still-locked nested session" >&2
  exit 1
fi
echo "Authenticate in the nested swaylock window to complete fallback recovery."

attempts=0
while (( attempts < 1800 )); do
  state=$(unit_value "${fallback_unit}" ActiveState)
  result=$(unit_value "${fallback_unit}" Result)
  if [[ ${state} == inactive && ${result} == success ]]; then
    printf 'swaylock_fallback=pass\n' >>"${report}"
    echo "Nested lock crash/restart and swaylock fallback recovery passed."
    exit 0
  fi
  if [[ -n ${sway_pid} ]] && ! kill -0 "${sway_pid}" 2>/dev/null; then
    echo "nested compositor exited before authenticated recovery" >&2
    exit 1
  fi
  sleep 0.1
  ((attempts += 1))
done

echo "timed out waiting for authenticated fallback recovery" >&2
exit 1
