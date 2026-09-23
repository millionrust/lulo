#!/usr/bin/env bash
# PLAN_NEW.md section 3.4 / ADR 0015: while rmac moves to gpui-kit's
# gpui-base, no file may start using gpui_component, and no file may add uses.
# The baseline only shrinks: regenerate it with --update after removing uses.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
baseline=scripts/gpui-component-baseline.txt

current() {
  grep -rc --include='*.rs' 'gpui_component' crates shell/bins shell/crates \
    | grep -v ':0$' | LC_ALL=C sort
}

if [[ "${1:-}" == "--update" ]]; then
  current > "$baseline"
  echo "recorded $(wc -l < "$baseline" | tr -d ' ') files in $baseline"
  exit 0
fi

status=0
while IFS=: read -r file count; do
  allowed="$(grep -F "$file:" "$baseline" | head -1 | cut -d: -f2 || true)"
  if [[ -z "$allowed" ]]; then
    echo "new gpui_component use in $file ($count); build UI through rmac-ui (ADR 0015)" >&2
    status=1
  elif (( count > allowed )); then
    echo "$file gained gpui_component uses ($allowed -> $count); use rmac-ui (ADR 0015)" >&2
    status=1
  fi
done < <(current)

if (( status == 0 )); then
  echo "gpui_component boundary holds ($(current | wc -l | tr -d ' ') files, none growing)"
fi
exit "$status"
