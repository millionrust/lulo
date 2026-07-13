#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
timestamp="$(date -u +%Y%m%dT%H%M%SZ)"
evidence_dir="$repo_root/target/linux-evidence/$timestamp/accessibility"
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
  echo "this accessibility evidence pass must run on Linux" >&2
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

require_space "$minimum_kib" "accessibility evidence"
mkdir -p "$evidence_dir"
bash "$repo_root/scripts/linux/collect-reference-evidence.sh" "$evidence_dir"

{
  echo "captured_at=$timestamp"
  echo "git_commit=$(git -C "$repo_root" rev-parse HEAD)"
  echo "available_kib=$(available_kib)"
  echo "session_type=${XDG_SESSION_TYPE:-unset}"
  echo "current_desktop=${XDG_CURRENT_DESKTOP:-unset}"
  echo "session_desktop=${XDG_SESSION_DESKTOP:-unset}"
  echo "wayland_display=${WAYLAND_DISPLAY:-unset}"
  echo "x11_display=${DISPLAY:-unset}"
  echo "orca_path=$(command -v orca || echo unavailable)"
  if command -v gsettings >/dev/null 2>&1; then
    echo -n "gtk_text_factor="
    gsettings get org.gnome.desktop.interface text-scaling-factor 2>&1 || true
    echo -n "gtk_text_writable="
    gsettings writable org.gnome.desktop.interface text-scaling-factor 2>&1 || true
  else
    echo "gtk_text_factor=unavailable"
    echo "gtk_text_writable=unavailable"
  fi
  if command -v niri >/dev/null 2>&1; then
    echo -n "niri_version="
    niri --version 2>&1 || true
    echo -n "niri_config="
    niri validate >/dev/null 2>&1 && echo valid || echo invalid-or-unavailable
  else
    echo "niri_version=unavailable"
    echo "niri_config=unavailable"
  fi
} >"$evidence_dir/f16-authorities.txt"

cat >"$evidence_dir/f16-manual-checklist.md" <<'CHECKLIST'
# F16 Linux accessibility evidence

Record pass/fail, the exact scale, and a screenshot or reproduction reference
for every item. An unchecked item is not evidence of completion.

## rmac text and output scale

- [ ] Standard text at 100% output scale across all seven apps
- [ ] Large text at 100% output scale across all seven apps
- [ ] Extra Large text at 100% output scale across all seven apps
- [ ] Extra Large text at every supported output scale through 200%
- [ ] Live changes do not clip, overlap, displace focus, or break hit regions
- [ ] Editor, note-body, and terminal content fonts remain independent

## GTK text authority

- [ ] Standard is confirmed as 100% by GSettings and a GTK application
- [ ] Large is confirmed as 120% by GSettings and a GTK application
- [ ] Extra Large is confirmed as 130% by GSettings and a GTK application
- [ ] GTK changes do not alter niri output scale or rmac text scale
- [ ] A custom factor is shown exactly with no preset falsely selected

## Keyboard

- [ ] Standard repeat persists and matches observed delay/rate
- [ ] Deliberate repeat persists and matches observed delay/rate
- [ ] Minimal repeat persists and matches observed delay/rate
- [ ] A custom niri delay/rate leaves all presets unselected
- [ ] Sticky, Slow, and Bounce Keys remain explicitly unavailable

## Pointer

- [ ] Standard, Steady, and Precise mouse presets persist and feel distinct
- [ ] Mouse middle-button emulation produces exactly one middle click
- [ ] Disabling mouse middle-button emulation removes the chord action
- [ ] Touchpad middle emulation round-trips on supported hardware
- [ ] Trackpad drag lock and Ignore while typing work independently
- [ ] Mouse Keys, dwell click, and double-click timing remain unavailable

## Orca

- [ ] Full niri session is detected
- [ ] Xwayland DISPLAY is detected
- [ ] Orca executable is detected
- [ ] Super–Alt–S starts Orca in the default niri configuration
- [ ] Upstream accessibility probe roles, names, states, actions, and focus work
- [ ] rmac AT-SPI gaps are recorded precisely and are not marked as passing
CHECKLIST

if [[ "$run_code_checks" == true ]]; then
  require_space "$build_minimum_kib" "scoped accessibility code checks"
  cd "$repo_root"
  cargo test --offline --locked -p rmac-gtk-settings -p rmac-input \
    2>&1 | tee "$evidence_dir/domain-tests.log"
  require_space "$minimum_kib" "post-test storage floor"
  cargo check --offline --locked -p rmac-system-settings --tests \
    2>&1 | tee "$evidence_dir/system-settings-check.log"
  require_space "$minimum_kib" "post-check storage floor"
fi

echo "F16 evidence written to $evidence_dir"
echo "Complete $evidence_dir/f16-manual-checklist.md on the Linux reference PC."
