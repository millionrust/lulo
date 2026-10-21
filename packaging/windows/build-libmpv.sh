#!/usr/bin/env bash
# Build Media Player's libmpv for Windows under the LGPL (WIN-OS-88).
#
# Run in an MSYS2 UCRT64 shell (windows-preview.yml's `libmpv` job, cached by
# this file's hash). FFmpeg and mpv are built from their pinned release tags
# without any GPL part: FFmpeg without --enable-gpl or --enable-nonfree, mpv
# with -Dgpl=false. The libraries they load come from MSYS2's UCRT64
# packages; each one's licence is checked and copied, and SOURCES.txt
# records every source, version and option, as the LGPL asks of whoever
# ships the binaries.
#
# Usage: build-libmpv.sh <output-dir>
# Output: <output-dir>/*.dll and <output-dir>/licenses/libmpv/*.
set -euo pipefail

FFMPEG_TAG=n7.1.3
MPV_TAG=v0.40.0

out=$(realpath -m "${1:?output directory}")
work=$(mktemp -d)
prefix="$work/prefix"
jobs=$(nproc)
mkdir -p "$out/licenses/libmpv"

ffmpeg_options=(
  --prefix="$prefix" --enable-shared --disable-static
  --disable-programs --disable-doc --disable-debug
  --enable-libdav1d --enable-d3d11va --enable-dxva2
)
git clone --quiet --depth 1 --branch "$FFMPEG_TAG" https://github.com/FFmpeg/FFmpeg.git "$work/ffmpeg"
(
  cd "$work/ffmpeg"
  ./configure "${ffmpeg_options[@]}"
  # Refuse anything but an LGPL build.
  grep -q '^#define FFMPEG_LICENSE "LGPL version 2.1 or later"' config.h
  make -j"$jobs"
  make install
)

mpv_options=(
  --prefix="$prefix" --buildtype=release
  -Dlibmpv=true -Dcplayer=false -Dgpl=false -Dtests=false
  -Dlua=disabled -Djavascript=disabled -Dlibarchive=disabled
  -Drubberband=disabled -Dvapoursynth=disabled -Dzimg=disabled
)
git clone --quiet --depth 1 --branch "$MPV_TAG" https://github.com/mpv-player/mpv.git "$work/mpv"
(
  cd "$work/mpv"
  PKG_CONFIG_PATH="$prefix/lib/pkgconfig" meson setup build "${mpv_options[@]}"
  meson compile -C build
  meson install -C build
)

# libmpv, FFmpeg's libraries, and every non-system DLL they load.
cp "$prefix/bin/libmpv-2.dll" "$out/"
cp "$prefix"/bin/av*.dll "$prefix"/bin/sw*.dll "$out/"
packages=()
while read -r dll; do
  case "$dll" in
    /ucrt64/bin/*)
      cp "$dll" "$out/"
      package=$(pacman -Qqo "$dll")
      packages+=("$package")
      ;;
  esac
done < <(ntldd -R "$out/libmpv-2.dll" | sed -n 's/.*=> \(.*\.dll\).*/\1/p' | sed 's|\\|/|g' | xargs -r cygpath -u | sort -u)
mapfile -t packages < <(printf '%s\n' "${packages[@]}" | sort -u)

# Every library's licence must allow shipping it beside an MIT app: no
# GPL-only package (a GPL-or-other dual licence, such as FreeType's, and
# GCC's runtime with its library exception, are fine).
for package in "${packages[@]}"; do
  licence=$(pacman -Qi "$package" | sed -n 's/^Licenses *: //p')
  if grep -q 'GPL' <<<"$licence" && ! grep -qE 'LGPL|OR|FTL|custom|exception' <<<"$licence"; then
    echo "build-libmpv.sh: $package is licensed '$licence'; refusing to bundle it" >&2
    exit 1
  fi
  name=${package#mingw-w64-ucrt-x86_64-}
  if [ -d "/ucrt64/share/licenses/$name" ]; then
    # Every file, nested ones too, flattened as name-path-file.
    while read -r file; do
      relative=${file#"/ucrt64/share/licenses/$name/"}
      cp "$file" "$out/licenses/libmpv/$name-${relative//\//-}"
    done < <(find "/ucrt64/share/licenses/$name" -type f)
  fi
done
cp "$work/ffmpeg/COPYING.LGPLv2.1" "$out/licenses/libmpv/ffmpeg-COPYING.LGPLv2.1"
cp "$work/mpv/LICENSE.LGPL" "$out/licenses/libmpv/mpv-LICENSE.LGPL"

{
  echo "Media Player's libmpv for Windows, built by packaging/windows/build-libmpv.sh"
  echo "under the GNU Lesser General Public License, version 2.1 or later."
  echo
  echo "FFmpeg $FFMPEG_TAG: https://github.com/FFmpeg/FFmpeg/tree/$FFMPEG_TAG"
  echo "  ./configure ${ffmpeg_options[*]/--prefix=*/}"
  echo "mpv $MPV_TAG: https://github.com/mpv-player/mpv/tree/$MPV_TAG"
  echo "  meson setup ${mpv_options[*]/--prefix=*/}"
  echo
  echo "Libraries from MSYS2 (sources: https://github.com/msys2/MINGW-packages):"
  for package in "${packages[@]}"; do
    pacman -Q "$package"
  done
  echo
  echo "To use a changed version, rebuild these libraries from those sources"
  echo "and replace the DLLs beside rmac-player.exe."
} > "$out/licenses/libmpv/SOURCES.txt"

ls -l "$out"
