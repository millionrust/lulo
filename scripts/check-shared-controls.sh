#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

product_roots=(
  crates/finder/src
  crates/terminal/src
  crates/notes/src
  crates/text-editor/src
  crates/activity-monitor/src
  crates/system-settings/src
  crates/app-drawer/src
)

pattern='gpui_component::(button|input|switch|slider|table)|^[[:space:]]*(button|input|switch|slider|table)::'

if matches="$(rg -n "$pattern" "${product_roots[@]}" --glob '*.rs' || true)" && [[ -n "$matches" ]]; then
  echo "product apps must consume Button/Input/Switch/Slider/Table through rmac-ui:" >&2
  echo "$matches" >&2
  exit 1
fi

echo "shared-control boundary passed for all seven product apps"
