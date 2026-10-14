#!/usr/bin/env bash
# Split a GitHub Release notes file into its title and description.
#
#   split-release-notes.sh <notes.md> <output-dir>
#
# The notes file's first line is "# <title>"; the rest (leading blank lines
# dropped) is the description. Writes <output-dir>/title.txt and
# <output-dir>/body.md. Used by release.yml and windows-release.yml.
set -euo pipefail

notes=${1:?notes file}
out=${2:?output directory}
[ -f "$notes" ] || { echo "split-release-notes.sh: missing $notes" >&2; exit 1; }
title="$(head -n 1 "$notes")"
[[ "$title" == "# "* ]] || { echo "split-release-notes.sh: $notes must start with '# <title>'" >&2; exit 1; }
mkdir -p "$out"
printf '%s\n' "${title#\# }" > "$out/title.txt"
tail -n +2 "$notes" | sed '/./,$!d' > "$out/body.md"
[ -s "$out/body.md" ] || { echo "split-release-notes.sh: $notes has no description" >&2; exit 1; }
cat "$out/title.txt"
