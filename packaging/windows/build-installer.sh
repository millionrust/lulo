#!/usr/bin/env bash
# Build Lulo-Setup-<version>-x64.msi from already-built app exes (ADR 0023
# "Installer"). Run on windows-latest, with the `wix` CLI on PATH
# (`dotnet tool install --global wix`) and the apps already built with
# their Windows icons embedded (scripts/windows/make_icons.sh, then
# `cargo build --release`, before this script).
#
# Usage: build-installer.sh <exe-dir> <full-version> <output-msi-path>
set -euo pipefail

exe_dir=${1:?directory holding the built .exe files}
version=${2:?full semver, e.g. 0.9.0-beta.1}
out=${3:?output .msi path}

root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# The MSI Version attribute is strictly numeric (Win32 VERSIONINFO has the
# same limit; rmac-windows-resource-build's build.rs carries the full
# string separately in each exe's free-text FileVersion/ProductVersion).
numeric_version="${version%%[-+]*}"

python3 "$root/generate_apps_wxs.py" --exe-dir "$exe_dir" --output "$root/Apps.wxs"

wix build "$root/Product.wxs" "$root/Apps.wxs" \
  -arch x64 \
  -d "LuloVersion=$numeric_version" \
  -o "$out"

echo "built $out ($version, numeric $numeric_version)"
