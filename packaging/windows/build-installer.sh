#!/usr/bin/env bash
# Build Lulo-Setup-<version>-x64.msi from already-built app exes (ADR 0023
# "Installer"). Run on windows-latest, with the `wix` CLI on PATH
# (`dotnet tool install --global wix`) and the apps already built with
# their Windows icons embedded (scripts/windows/make_icons.sh, then
# `cargo build --release`, before this script).
#
# Usage: build-installer.sh <exe-dir> <full-version> <output-msi-path> [build-number]
#
# The MSI's ProductVersion is <major>.<minor>.<build-number> (see
# msi_version.py for the mapping and why): the build number is the
# argument, else $LULO_BUILD_NUMBER, else the commit count of the checkout
# (`git rev-list --count HEAD`, which needs full history: check out with
# fetch-depth 0).
set -euo pipefail

exe_dir=${1:?directory holding the built .exe files}
version=${2:?full semver, e.g. 0.9.0-beta.1}
out=${3:?output .msi path}
build=${4:-${LULO_BUILD_NUMBER:-}}

root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ -z "$build" ]]; then
  if [[ "$(git -C "$root" rev-parse --is-shallow-repository)" == "true" ]]; then
    echo "build-installer.sh: a shallow checkout has no commit count; pass a build number or check out with fetch-depth 0" >&2
    exit 1
  fi
  build="$(git -C "$root" rev-list --count HEAD)"
fi
# Strictly numeric (Win32 VERSIONINFO has the same limit;
# rmac-windows-resource-build's build.rs carries the full string separately
# in each exe's free-text FileVersion/ProductVersion).
python_bin=python3
command -v "$python_bin" >/dev/null 2>&1 || python_bin=python
msi_version="$("$python_bin" "$root/msi_version.py" "$version" "$build")"

"$python_bin" "$root/generate_apps_wxs.py" --exe-dir "$exe_dir" --output "$root/Apps.wxs"

wix build "$root/Product.wxs" "$root/Apps.wxs" \
  -arch x64 \
  -d "LuloVersion=$msi_version" \
  -o "$out"

echo "built $out ($version, MSI version $msi_version, build $build)"
