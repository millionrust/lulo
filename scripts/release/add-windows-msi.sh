#!/usr/bin/env bash
# Add the Windows MSI to an already-published Lulo Release (the second step
# of a Ubuntu-first release; see docs/release-process.md).
#
#   add-windows-msi.sh prepare <tag> <msi> <out-dir>
#       Check the Release exists and is not sealed by the APT repository,
#       download its SHA256SUMS, and write <out-dir>/SHA256SUMS with the
#       MSI's line added (replacing an older line for the same file name).
#   add-windows-msi.sh publish <tag> <msi> <out-dir> <notes-dir>
#       Upload the MSI and the new SHA256SUMS, and set the Release's title
#       and description from <notes-dir>/title.txt and body.md.
#
# Needs `gh` with GH_TOKEN and GITHUB_REPOSITORY set (GitHub Actions does).
set -euo pipefail

fail() { echo "add-windows-msi.sh: $*" >&2; exit 1; }

command=${1:?prepare|publish}
tag=${2:?release tag}
msi=${3:?MSI path}
out=${4:?output directory}
repo=${GITHUB_REPOSITORY:?GITHUB_REPOSITORY}
name="$(basename "$msi")"
[[ "$name" =~ ^Lulo-Setup-[0-9A-Za-z.+-]+-x64\.msi$ ]] || fail "unexpected MSI name: $name"
[ -f "$msi" ] || fail "missing $msi"

case "$command" in
  prepare)
    gh release view "$tag" --repo "$repo" >/dev/null 2>&1 || fail "release $tag does not exist; publish it first"
    sealed="$(gh release view "$tag" --repo "$repo" --json assets --jq '[.assets[].name | select(startswith("apt-snapshot-"))] | length')"
    [ "$sealed" = 0 ] || fail "release $tag is already published in the APT repository; its assets are sealed"
    mkdir -p "$out"
    rm -f "$out/SHA256SUMS"
    gh release download "$tag" --repo "$repo" --pattern SHA256SUMS --dir "$out" \
      || fail "could not download SHA256SUMS from release $tag"
    sum="$(sha256sum -- "$msi" | cut -d' ' -f1)"
    awk -v name="$name" '$NF != name' "$out/SHA256SUMS" > "$out/SHA256SUMS.new"
    printf '%s  %s\n' "$sum" "$name" >> "$out/SHA256SUMS.new"
    mv "$out/SHA256SUMS.new" "$out/SHA256SUMS"
    cp -- "$msi" "$out/$name"
    echo "prepared $name ($sum) for $tag"
    ;;
  publish)
    notes=${5:?notes directory}
    [ -f "$out/SHA256SUMS" ] && [ -f "$out/$name" ] || fail "run prepare first"
    [ -s "$notes/title.txt" ] && [ -s "$notes/body.md" ] || fail "missing release notes in $notes"
    gh release upload "$tag" --repo "$repo" "$out/$name" "$out/SHA256SUMS" --clobber
    gh release edit "$tag" --repo "$repo" --title "$(cat "$notes/title.txt")" --notes-file "$notes/body.md"
    echo "added $name to $tag"
    ;;
  *) fail "usage: add-windows-msi.sh prepare|publish ..." ;;
esac
