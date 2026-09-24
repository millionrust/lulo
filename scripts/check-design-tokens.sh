#!/usr/bin/env bash
# Fail when product UI source hard-codes a color, calls hsla() directly, or
# passes a literal radius to rounded(), instead of using rmac-design tokens.
#
# Token-conversion modules are exempt: `crates/rmac-design` is the token source
# itself, and `shell/crates/rmac-shell-ui/src/tokens.rs` converts the tokens to
# GPUI values. Test files are always exempt.
#
# Files that still hard-code values while the app sweep is open are recorded,
# with their counts, in scripts/design-token-baseline.txt. The baseline only
# shrinks: a new file or a growing count fails; after tokenizing values,
# regenerate it with --update.
#
# Usage: scripts/check-design-tokens.sh [--report | --update]
#   --report  print every hard-coded value and exit 0
#   --update  rewrite the baseline from the current tree
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
baseline=scripts/design-token-baseline.txt
mode="fail"
if [[ $# -gt 1 ]]; then
  echo "usage: scripts/check-design-tokens.sh [--report | --update]" >&2
  exit 2
fi
case "${1:-}" in
  "") ;;
  --report) mode="report" ;;
  --update) mode="update" ;;
  *)
    echo "usage: scripts/check-design-tokens.sh [--report | --update]" >&2
    exit 2
    ;;
esac

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
counts="$(mktemp)"
cleanup() { rm -f "$raw" "$filtered" "$filtered.tmp" "$counts"; }
trap cleanup EXIT
: >"$raw"

for path in "${paths[@]}"; do
  [[ -d "$path" ]] || continue
  grep -rEnH "$pattern" "$path" --include='*.rs' >>"$raw" 2>/dev/null || true
done

grep -vE 'tests?\.rs|/tests/' "$raw" >"$filtered" || true
for exempted in "${exempt[@]}"; do
  grep -vF "$exempted:" "$filtered" >"$filtered.tmp" || true
  mv "$filtered.tmp" "$filtered"
done

# file:count, one line per file, sorted like the baseline.
cut -d: -f1 "$filtered" | LC_ALL=C sort | uniq -c \
  | awk '{ print $2 ":" $1 }' >"$counts"

total="$(wc -l <"$filtered" | tr -d ' ')"
if [[ "$mode" == "update" ]]; then
  cp "$counts" "$baseline"
  echo "recorded $total hard-coded value(s) in $(wc -l <"$baseline" | tr -d ' ') files in $baseline"
  exit 0
fi
if [[ "$mode" == "report" ]]; then
  echo "check-design-tokens: $total hard-coded design value(s)" >&2
  cat "$filtered" >&2
  exit 0
fi

status=0
while IFS=: read -r file count; do
  allowed="$(grep -F "$file:" "$baseline" | head -1 | cut -d: -f2 || true)"
  if [[ -z "$allowed" ]]; then
    echo "check-design-tokens: new hard-coded design value(s) in $file; use rmac-design tokens:" >&2
    grep -F "$file:" "$filtered" >&2
    status=1
  elif (( count > allowed )); then
    echo "check-design-tokens: $file gained hard-coded design values ($allowed -> $count); use rmac-design tokens:" >&2
    grep -F "$file:" "$filtered" >&2
    status=1
  fi
done <"$counts"

if (( status == 0 )); then
  echo "check-design-tokens: holds ($total baselined value(s) in $(wc -l <"$counts" | tr -d ' ') files, none growing)"
fi
exit "$status"
