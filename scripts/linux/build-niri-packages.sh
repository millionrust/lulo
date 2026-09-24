#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
#
# Build Lulo OS's niri and xwayland-satellite Debian packages from the exact
# upstream sources pinned in packaging/third-party/upstreams.json.
#
# For each package this:
#   1. downloads the upstream tag tarball (cached under WORK/downloads) and
#      refuses it unless its SHA-256 and its embedded commit ID match the pin;
#   2. runs `cargo vendor --locked` once, packs vendor/ into a deterministic
#      <name>_<version>.orig-vendor.tar.xz, and checks it against the pinned
#      vendor SHA-256 once one is recorded;
#   3. adds packaging/third-party/<name>/debian plus the vendored crates'
#      licence notices, and runs dpkg-buildpackage (unsigned). The build
#      itself is offline (`cargo build --frozen`) and produces the .deb and a
#      complete 3.0 (quilt) source package (.dsc, both orig tarballs,
#      .debian.tar.xz, .buildinfo, .changes) -- the GPL/MPL source offer;
#   4. writes a CycloneDX SBOM and a SHA256SUMS for the output directory.
#
# It never needs root. One target directory (WORK/target) is reused across
# runs and packages. It refuses to start with less than 25 GiB free on
# WORK's filesystem (AGENTS.md) unless --minimum-free-gib says otherwise.
#
# Build dependencies (--build-deps):
#   system        require the Build-Depends in debian/control to be installed
#                 (release.yml installs them in its ubuntu:26.04 container).
#   user-sysroot  for a machine without sudo: `apt-get download` the missing
#                 -dev packages (no root needed), unpack them under
#                 WORK/sysroot, and point pkg-config, the C compiler, and
#                 bindgen at it. Runtime libraries must already be installed
#                 (they are wherever the danklinux PPA's niri is).
#   auto          (default) system if dpkg-checkbuilddeps passes, otherwise
#                 user-sysroot.
#
# Usage on the reference laptop (see docs/release-process.md):
#   bash scripts/linux/build-niri-packages.sh --jobs 2
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
contract="$repo_root/scripts/linux/third_party_packages.py"
minimum_free_gib=25

work_dir="${HOME:-}/rmac-niri-build"
output=""
build_deps="auto"
jobs=""
selected=(xwayland-satellite niri)

usage() {
  cat >&2 <<'EOF'
usage: build-niri-packages.sh [--work-dir /absolute/dir] [--output /absolute/new/dir]
                              [--package niri|xwayland-satellite|all]
                              [--build-deps auto|system|user-sysroot] [--jobs N]
                              [--minimum-free-gib N]

  --work-dir    downloads, sources, the shared cargo target directory, and the
                optional user sysroot (default: ~/rmac-niri-build)
  --output      where the .debs, source packages, SBOMs, and SHA256SUMS go;
                must not exist yet (default: WORK/packages-<UTC timestamp>)
  --minimum-free-gib
                refuse to start with less free space than this on WORK's
                filesystem (default 25, the AGENTS.md floor for local builds;
                release.yml lowers it for the throwaway CI container)
EOF
}

fail() {
  echo "build-niri-packages: $*" >&2
  exit 1
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --work-dir|--output|--package|--build-deps|--jobs|--minimum-free-gib)
      option=$1
      shift
      [[ $# -gt 0 ]] || { usage; exit 2; }
      case "$option" in
        --work-dir) work_dir=$1 ;;
        --output) output=$1 ;;
        --package)
          case "$1" in
            all) selected=(xwayland-satellite niri) ;;
            niri|xwayland-satellite) selected=("$1") ;;
            *) usage; exit 2 ;;
          esac
          ;;
        --build-deps)
          case "$1" in
            auto|system|user-sysroot) build_deps=$1 ;;
            *) usage; exit 2 ;;
          esac
          ;;
        --jobs)
          [[ "$1" =~ ^[1-9][0-9]*$ ]] || { usage; exit 2; }
          jobs=$1
          ;;
        --minimum-free-gib)
          [[ "$1" =~ ^[1-9][0-9]*$ ]] || { usage; exit 2; }
          minimum_free_gib=$1
          ;;
      esac
      ;;
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

[[ "$(uname -s)" == Linux ]] || fail "Linux is required"
[[ ${EUID} -ne 0 ]] || fail "run as your normal user, not root"
[[ "$work_dir" == /* && "$work_dir" != / ]] || fail "--work-dir must be an absolute, non-root path"
for tool in curl sha256sum tar gzip xz python3 git make dpkg-buildpackage \
  dpkg-source dpkg-shlibdeps dpkg-gencontrol dpkg-deb dpkg-parsechangelog \
  dpkg-architecture cargo; do
  command -v "$tool" >/dev/null 2>&1 || fail "'$tool' is required but was not found"
done

mkdir -p "$work_dir"
[[ -d "$work_dir" && ! -L "$work_dir" ]] || fail "$work_dir must be an ordinary directory"
available_kib="$(df -Pk "$work_dir" | awk 'NR == 2 { print $4 }')"
if [[ ! "$available_kib" =~ ^[0-9]+$ ]] || (( available_kib < minimum_free_gib * 1024 * 1024 )); then
  fail "at least $minimum_free_gib GiB must be free on the filesystem holding $work_dir"
fi

if [[ -z "$output" ]]; then
  output="$work_dir/packages-$(date -u +%Y%m%dT%H%M%SZ)"
fi
[[ "$output" == /* && "$output" != / ]] || fail "--output must be an absolute, non-root path"
[[ ! -e "$output" && ! -L "$output" ]] || fail "$output already exists"

# The repository's own pinned toolchain (rust-toolchain.toml) builds both
# packages too, so CI and the laptop compile with the same rustc.
if command -v rustup >/dev/null 2>&1 && [[ -z "${RUSTUP_TOOLCHAIN:-}" ]]; then
  RUSTUP_TOOLCHAIN="$(awk -F'"' '/^channel *=/ { print $2; exit }' "$repo_root/rust-toolchain.toml")"
  [[ -n "$RUSTUP_TOOLCHAIN" ]] || fail "rust-toolchain.toml has no channel"
  export RUSTUP_TOOLCHAIN
fi
echo "build-niri-packages: using $(cargo --version)"

mkdir -p "$work_dir/downloads" "$work_dir/sources" "$work_dir/target"
multiarch="$(dpkg-architecture -qDEB_HOST_MULTIARCH)"
build_architecture="$(dpkg-architecture -qDEB_HOST_ARCH)"
case "$build_architecture" in
  amd64|arm64) ;;
  *) fail "only amd64 and arm64 are supported, not $build_architecture" ;;
esac

# Build-Depends that are tools rather than -dev libraries; the user sysroot
# never tries to provide these.
sysroot_skip='^(clang|dpkg-dev|libclang-dev|pkgconf|python3|xz-utils)$'

build_dependency_names() {
  # Package names from one debian/control's Build-Depends, without versions.
  awk '
    /^Build-Depends:/ { collecting = 1; sub(/^Build-Depends:/, "") }
    collecting && /^[A-Za-z-]+:/ && !/^Build-Depends:/ { collecting = 0 }
    collecting { print }
  ' "$1" | tr ',' '\n' | sed -e 's/(.*)//' -e 's/[[:space:]]//g' | grep -v '^$'
}

prepare_user_sysroot() {
  local sysroot="$work_dir/sysroot"
  local debs="$work_dir/sysroot-debs"
  local -a wanted=()
  local name
  for control in "$repo_root"/packaging/third-party/*/debian/control; do
    while IFS= read -r name; do
      [[ "$name" =~ $sysroot_skip ]] || wanted+=("$name")
    done < <(build_dependency_names "$control")
  done
  # Rebuilt from scratch each run so a stale header never outlives an update.
  rm -rf "$sysroot" "$debs"
  mkdir -p "$sysroot" "$debs"
  # --print-uris needs no root: it prints one "'URI' FILE SIZE HASH" line for
  # each package apt would have to fetch, i.e. exactly what is missing.
  local listing
  listing="$(apt-get install --print-uris -qq --no-install-recommends "${wanted[@]}")" \
    || fail "apt-get could not resolve the build dependencies: ${wanted[*]}"
  local -a specs=()
  local file version
  while read -r _ file _; do
    [[ "$file" == *.deb ]] || continue
    name="${file%%_*}"
    version="${file#*_}"
    version="${version%_*}"
    version="${version//%3a/:}"
    specs+=("$name=$version")
  done <<<"$listing"
  if [[ ${#specs[@]} -gt 0 ]]; then
    echo "build-niri-packages: unpacking ${#specs[@]} missing build packages into $sysroot"
    (cd "$debs" && apt-get download "${specs[@]}") \
      || fail "apt-get download of the build dependencies failed"
    local deb
    for deb in "$debs"/*.deb; do
      dpkg-deb -x "$deb" "$sysroot"
    done
  fi
  # Point the unpacked pkg-config files at the sysroot.
  local pc
  while IFS= read -r -d '' pc; do
    sed -i -E "s#^([A-Za-z_]+)=/usr(/|\$)#\\1=$sysroot/usr\\2#" "$pc"
  done < <(find "$sysroot" -name '*.pc' -type f -print0)
  # -dev packages carry libfoo.so -> libfoo.so.N links whose targets are the
  # already-installed runtime libraries; repoint them at the real system.
  local link target system_target
  while IFS= read -r -d '' link; do
    [[ -e "$link" ]] && continue
    target="$(readlink "$link")"
    case "$target" in
      /*) system_target="$target" ;;
      *) system_target="${link#"$sysroot"}"; system_target="$(dirname "$system_target")/$target" ;;
    esac
    [[ -e "$system_target" ]] \
      || fail "runtime library $system_target is not installed (needed by ${link#"$sysroot"})"
    ln -sfn "$system_target" "$link"
  done < <(find "$sysroot" -type l -name '*.so' -print0)

  export PKG_CONFIG_PATH="$sysroot/usr/lib/$multiarch/pkgconfig:$sysroot/usr/share/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
  export LIBRARY_PATH="$sysroot/usr/lib/$multiarch${LIBRARY_PATH:+:$LIBRARY_PATH}"
  export CPATH="$sysroot/usr/include${CPATH:+:$CPATH}"
  export BINDGEN_EXTRA_CLANG_ARGS="-I$sysroot/usr/include${BINDGEN_EXTRA_CLANG_ARGS:+ $BINDGEN_EXTRA_CLANG_ARGS}"
  if [[ -z "${LIBCLANG_PATH:-}" ]]; then
    local candidate
    for candidate in /usr/lib/llvm-*/lib; do
      if compgen -G "$candidate/libclang*.so*" >/dev/null; then
        LIBCLANG_PATH="$candidate"
      fi
    done
    [[ -n "${LIBCLANG_PATH:-}" ]] || fail "no libclang found under /usr/lib/llvm-*/lib (install clang)"
    export LIBCLANG_PATH
  fi
}

dpkg_buildpackage_args=(-us -uc -sa)
if [[ "$build_deps" == auto ]]; then
  build_deps=system
  for name in "${selected[@]}"; do
    if ! (cd "$repo_root/packaging/third-party/$name" && dpkg-checkbuilddeps 2>/dev/null); then
      build_deps=user-sysroot
    fi
  done
  echo "build-niri-packages: build dependencies: $build_deps"
fi
if [[ "$build_deps" == user-sysroot ]]; then
  prepare_user_sysroot
  # Build-Depends are satisfied by the sysroot, which dpkg cannot see.
  dpkg_buildpackage_args+=(-d)
fi

staging="$(mktemp -d "$work_dir/output.XXXXXX")"
trap 'rm -rf "$staging"' EXIT

for name in "${selected[@]}"; do
  eval "$(python3 "$contract" shell-vars --name "$name")"
  echo "build-niri-packages: $PIN_NAME $PIN_DEBIAN_VERSION from $PIN_TAG ($PIN_COMMIT)"

  tarball="$work_dir/downloads/$PIN_ORIG_TARBALL"
  if [[ ! -f "$tarball" ]]; then
    curl -fsSL --proto '=https' --tlsv1.2 -o "$tarball.part" "$PIN_TARBALL_URL" \
      || fail "could not download $PIN_TARBALL_URL"
    mv "$tarball.part" "$tarball"
  fi
  actual="$(sha256sum "$tarball" | awk '{ print $1 }')"
  if [[ "$actual" != "$PIN_TARBALL_SHA256" ]]; then
    rm -f "$tarball"
    fail "$PIN_ORIG_TARBALL has SHA-256 $actual, not the pinned $PIN_TARBALL_SHA256"
  fi
  commit="$(gzip -dc "$tarball" | git get-tar-commit-id)" \
    || fail "$PIN_ORIG_TARBALL does not record its commit"
  [[ "$commit" == "$PIN_COMMIT" ]] \
    || fail "$PIN_ORIG_TARBALL was made from $commit, not the pinned $PIN_COMMIT"

  parent="$work_dir/sources/$PIN_NAME"
  rm -rf "$parent"
  mkdir -p "$parent"
  cp "$tarball" "$parent/$PIN_ORIG_TARBALL"
  tar -xzf "$parent/$PIN_ORIG_TARBALL" -C "$parent"
  source_dir="$parent/$PIN_TOP_DIRECTORY"
  [[ -f "$source_dir/Cargo.lock" ]] || fail "$PIN_NAME has no Cargo.lock"

  # The only networked step: fetch the exact Cargo.lock graph.
  (cd "$source_dir" && cargo vendor --locked vendor >"$parent/cargo-vendor-config.toml") \
    || fail "cargo vendor failed for $PIN_NAME"
  grep -q '^directory = "vendor"$' "$parent/cargo-vendor-config.toml" \
    || fail "cargo vendor did not print the expected source replacement"
  mv "$parent/cargo-vendor-config.toml" "$source_dir/vendor/.lulo-cargo-config.toml"

  epoch="$(dpkg-parsechangelog -l "$repo_root/packaging/third-party/$PIN_NAME/debian/changelog" -STimestamp)"
  (cd "$source_dir" && tar --format=gnu --sort=name --mtime="@$epoch" \
      --owner=0 --group=0 --numeric-owner --mode=go-w -cf - vendor) \
    | xz -T1 -6 -c >"$parent/$PIN_VENDOR_TARBALL"
  vendor_sha256="$(sha256sum "$parent/$PIN_VENDOR_TARBALL" | awk '{ print $1 }')"
  echo "build-niri-packages: $PIN_VENDOR_TARBALL SHA-256 $vendor_sha256"
  python3 "$contract" check-vendor --name "$PIN_NAME" --sha256 "$vendor_sha256"

  cp -R "$repo_root/packaging/third-party/$PIN_NAME/debian" "$source_dir/debian"
  python3 "$contract" notices --name "$PIN_NAME" --vendor "$source_dir/vendor" \
    --output "$source_dir/debian/dependency-licenses.txt"

  (
    cd "$source_dir"
    export LULO_CARGO_TARGET_DIR="$work_dir/target"
    export LULO_CARGO_JOBS="$jobs"
    dpkg-buildpackage "${dpkg_buildpackage_args[@]}"
  ) || fail "dpkg-buildpackage failed for $PIN_NAME"

  artifacts=()
  for file in \
    "$parent/${PIN_NAME}_${PIN_DEBIAN_VERSION}_${build_architecture}.deb" \
    "$parent/${PIN_NAME}_${PIN_DEBIAN_VERSION}.dsc" \
    "$parent/$PIN_ORIG_TARBALL" \
    "$parent/$PIN_VENDOR_TARBALL" \
    "$parent/${PIN_NAME}_${PIN_DEBIAN_VERSION}.debian.tar.xz" \
    "$parent/${PIN_NAME}_${PIN_DEBIAN_VERSION}_${build_architecture}.buildinfo" \
    "$parent/${PIN_NAME}_${PIN_DEBIAN_VERSION}_${build_architecture}.changes"; do
    [[ -f "$file" ]] || fail "dpkg-buildpackage did not produce $(basename "$file")"
    cp "$file" "$staging/"
    artifacts+=(--artifact "$staging/$(basename "$file")")
  done
  python3 "$contract" sbom --name "$PIN_NAME" --cargo-lock "$source_dir/Cargo.lock" \
    --vendor-sha256 "$vendor_sha256" "${artifacts[@]}" \
    --output "$staging/${PIN_NAME}_${PIN_DEBIAN_VERSION}_${build_architecture}.cdx.json"
done

(cd "$staging" && sha256sum -- * >SHA256SUMS)
mv "$staging" "$output"
trap - EXIT
echo "build-niri-packages: wrote $output"
ls -l "$output"
