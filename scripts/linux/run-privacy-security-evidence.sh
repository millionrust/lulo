#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
evidence_dir="$repo_root/target/linux-evidence/$timestamp/privacy-security"
run_code_checks=false
minimum_kib=$((15 * 1024 * 1024))
build_minimum_kib=$((25 * 1024 * 1024))

usage() {
  echo "usage: $0 [--with-code-checks]" >&2
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --with-code-checks) run_code_checks=true ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      usage
      exit 2
      ;;
  esac
  shift
done

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "this privacy and security evidence pass must run on Linux" >&2
  exit 2
fi

available_kib() {
  df -Pk "$repo_root" | awk 'NR == 2 { print $4 }'
}

require_space() {
  local required_kib=$1
  local phase=$2
  local available
  available=$(available_kib)
  if (( available < required_kib )); then
    echo "$phase stopped: ${available} KiB free; ${required_kib} KiB required" >&2
    exit 3
  fi
}

capture() {
  local output=$1
  shift
  if command -v "$1" >/dev/null 2>&1; then
    timeout 20s "$@" >"$output" 2>&1 || true
  else
    echo "$1: not installed" >"$output"
  fi
}

require_space "$minimum_kib" "privacy and security evidence"
mkdir -p "$evidence_dir"
bash "$repo_root/scripts/linux/collect-reference-evidence.sh" "$evidence_dir"

series="$(sed -n 's/^VERSION_CODENAME=//p' /etc/os-release | tr -d '\"' | head -n 1)"
{
  echo "captured_at=$timestamp"
  echo "git_commit=$(git -C "$repo_root" rev-parse HEAD)"
  echo "available_kib=$(available_kib)"
  echo "ubuntu_series=${series:-unavailable}"
  if command -v ubuntu-distro-info >/dev/null 2>&1 && [[ -n "$series" ]]; then
    echo -n "standard_support_days_remaining="
    timeout 20s ubuntu-distro-info --series "$series" --days=eol 2>&1 || true
  else
    echo "standard_support_days_remaining=unavailable"
  fi
} >"$evidence_dir/f17-authorities.txt"

capture "$evidence_dir/pro-package-sources.json" \
  pro api u.pro.packages.summary.v1
capture "$evidence_dir/pro-attachment.json" \
  pro api u.pro.status.is_attached.v1
capture "$evidence_dir/pro-enabled-services.json" \
  pro api u.pro.status.enabled_services.v1
capture "$evidence_dir/unattended-upgrades.json" \
  pro api u.unattended_upgrades.status.v1

if command -v gdbus >/dev/null 2>&1; then
  {
    timeout 20s gdbus call --session \
      --dest org.freedesktop.impl.portal.PermissionStore \
      --object-path /org/freedesktop/impl/portal/PermissionStore \
      --method org.freedesktop.DBus.Properties.Get \
      org.freedesktop.impl.portal.PermissionStore version 2>&1 || true
    timeout 20s gdbus call --session \
      --dest org.freedesktop.impl.portal.PermissionStore \
      --object-path /org/freedesktop/impl/portal/PermissionStore \
      --method org.freedesktop.impl.portal.PermissionStore.Lookup \
      devices camera 2>&1 || true
    timeout 20s gdbus call --session \
      --dest org.freedesktop.impl.portal.PermissionStore \
      --object-path /org/freedesktop/impl/portal/PermissionStore \
      --method org.freedesktop.impl.portal.PermissionStore.Lookup \
      devices microphone 2>&1 || true
  } >"$evidence_dir/permission-store.txt"
else
  echo "gdbus: not installed" >"$evidence_dir/permission-store.txt"
fi

capture "$evidence_dir/flatpak-applications.txt" \
  flatpak list --app --columns=application,origin
capture "$evidence_dir/snap-applications.txt" snap list

cat >"$evidence_dir/f17-manual-checklist.md" <<'CHECKLIST'
# F17 Linux Privacy & Security evidence

Record pass/fail and attach a screenshot or reproduction reference for every
item. The generated files and unchecked items are not passing evidence.

## Portal permission decisions

- [ ] Camera entries match the raw `devices/camera` PermissionStore lookup
- [ ] Microphone entries match the raw `devices/microphone` lookup
- [ ] Unknown permission tokens are shown verbatim and do not select invented policy
- [ ] PermissionStore version 1 is read-only; version 2 enables per-app reset
- [ ] Reset requires explicit confirmation and deletes only the selected app/resource pair
- [ ] Reset completion resamples both resources and the next portal request may ask again
- [ ] An external portal decision appears without pressing Refresh
- [ ] PermissionStore restart preserves last-known-good state, reports disruption, and reconnects
- [ ] Active capture and native application access are never presented as revoked
- [ ] Missing session bus, portal service, and resource each have truthful distinct states

## Ubuntu lifecycle and updates

- [ ] The displayed Ubuntu series and days to standard EOL match `ubuntu-distro-info`
- [ ] Package-origin counts match the Ubuntu Pro package-summary API
- [ ] Contract validity and enabled services match their separate Ubuntu Pro APIs
- [ ] Automatic updates show enabled only when service, timer, periodic job, and interval agree
- [ ] Allowed origins, disabled reason, frequency, and last run match the unattended-upgrades API
- [ ] PackageKit security count remains separate from lifecycle and coverage status
- [ ] A missing or old helper leaves other successful authorities visible

## Application provenance

- [ ] Flatpak count matches live exported desktop entries
- [ ] Snap count matches live Snap desktop entries
- [ ] Integrated AppImages are detected from integration ID or `.AppImage` executable
- [ ] System and user desktop entries are not claimed to be APT-owned
- [ ] Command-line-only packages and repository trust remain explicitly outside the inventory

## Interaction and accessibility

- [ ] Refresh buttons expose loading and cannot launch duplicate work
- [ ] Long labels/tokens remain readable at every supported scale through 200%
- [ ] Keyboard focus reaches Refresh, Open, Reset, Cancel, and confirmation in logical order
- [ ] Orca announces section names, values, busy states, warnings, and destructive confirmation
CHECKLIST

if [[ "$run_code_checks" == true ]]; then
  require_space "$build_minimum_kib" "scoped privacy and security code checks"
  cd "$repo_root"
  cargo test --offline --locked -p rmac-privacy -p rmac-privacy-linux -p rmac-apps \
    2>&1 | tee "$evidence_dir/domain-tests.log"
  require_space "$minimum_kib" "post-test storage floor"
  cargo check --offline --locked -p rmac-system-settings --tests \
    2>&1 | tee "$evidence_dir/system-settings-check.log"
  require_space "$minimum_kib" "post-check storage floor"
fi

echo "F17 evidence written to $evidence_dir"
echo "Complete $evidence_dir/f17-manual-checklist.md on the Linux reference PC."
