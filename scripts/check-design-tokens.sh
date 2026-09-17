#!/usr/bin/env bash
# Fail when product UI source hard-codes a color, calls hsla() directly, or
# passes a literal radius to rounded(), instead of using rmac-design tokens.
#
# Token-conversion modules are exempt: `crates/rmac-design` is the token source
# itself, and `shell/crates/rmac-shell-ui/src/tokens.rs` converts the tokens to
# GPUI values. Test files are always exempt.
#
# Usage: scripts/check-design-tokens.sh [--report]
#   --report  print violations but exit 0 (used while the app sweep is open)
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mode="fail"
if [[ "${1:-}" == "--report" ]]; then
  mode="report"
fi
if [[ $# -gt 1 || ( $# -eq 1 && "$1" != "--report" ) ]]; then
  echo "usage: scripts/check-design-tokens.sh [--report]" >&2
  exit 2
fi

paths=(
  crates/finder/src
  crates/system-settings/src
  crates/notes/src
  crates/terminal/src
  crates/text-editor/src
  crates/activity-monitor/src
  crates/app-drawer/src
  crates/launcher-app/src
  crates/quick-settings-app/src
  crates/notification-center-app/src
  shell/bins
  shell/crates
)
exempt=(
  shell/crates/rmac-shell-ui/src/tokens.rs
)
pattern='rgb\(0x|rgba\(0x|hsla\(0x|\.rounded\(px\([0-9]'

raw="$(mktemp)"
filtered="$(mktemp)"
cleanup() { rm -f "$raw" "$filtered" "$filtered.tmp"; }
trap cleanup EXIT
: >"$raw"

for path in "${paths[@]}"; do
  dir="$root/$path"
  [[ -d "$dir" ]] || continue
  grep -rEnH "$pattern" "$dir" --include='*.rs' >>"$raw" 2>/dev/null || true
done

grep -vE 'tests?\.rs|/tests/' "$raw" >"$filtered" || true
for exempted in "${exempt[@]}"; do
  grep -vF "$root/$exempted:" "$filtered" >"$filtered.tmp" || true
  mv "$filtered.tmp" "$filtered"
done

count="$(wc -l <"$filtered" | tr -d ' ')"
if [[ "$count" != "0" ]]; then
  echo "check-design-tokens: $count hard-coded design value(s)" >&2
  cat "$filtered" >&2
  if [[ "$mode" == "report" ]]; then
    exit 0
  fi
  exit 1
fi

echo "check-design-tokens: clean"
