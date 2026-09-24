#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
#
# Build the Debian 3.0 (quilt) source package `rmac` for one exact commit, or
# rebuild the rmac-apps and rmac-session binary packages from such a source
# package, offline. This is the source offer docs/update-trust.md ("Source
# and license obligations") requires beside every binary publication.
#
#   source   The checkout must be clean and at --revision. Makes
#              rmac_<upstream>.orig.tar.xz         `git archive` of the commit
#              rmac_<upstream>.orig-vendor.tar.xz  one `cargo vendor --locked`
#                                                  over Cargo.lock and
#                                                  shell/Cargo.lock (the only
#                                                  networked step)
#            adds packaging/rmac-source/debian plus a generated changelog,
#            vendored-crate licence notices, and debian/rmac-revision, and runs
#            `dpkg-buildpackage -S` (unsigned). OUTPUT receives the .dsc, both
#            orig tarballs, the .debian.tar.xz, the _source.buildinfo and
#            _source.changes, rmac-source.json, and SHA256SUMS.
#   rebuild  Verifies a directory made by `source`, unpacks the .dsc, and runs
#            `dpkg-buildpackage -b` without -d, so dpkg-checkbuilddeps proves
#            Build-Depends complete. Cargo runs offline from vendor/. The two
#            .debs are checked with verify-native-packages.py; OUTPUT receives
#            them, the .buildinfo and .changes, rebuild.json, and SHA256SUMS.
#
# The rebuilt .debs are not expected to be byte-identical to a Release's:
# build paths differ. update-trust.md binds the commit, the Cargo.lock files,
# the packaging, and this procedure instead.
#
# Never needs root. Refuses to start with less than --minimum-free-gib (25 by
# default, AGENTS.md) free on WORK's filesystem. Note that the rebuild's
# build-native-inputs.sh separately requires 25 GiB free for the compile.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
helper="$repo_root/scripts/linux/rmac_source_package.py"
minimum_free_gib=25

usage() {
  cat >&2 <<'EOF'
usage: build-rmac-source-package.sh source --revision SHA --output /abs/new/dir
                                    [--tag TAG] [--work-dir /abs/dir]
                                    [--minimum-free-gib N]
       build-rmac-source-package.sh rebuild --source-dir /abs/dir
                                    --output /abs/new/dir [--work-dir /abs/dir]
                                    [--jobs N] [--minimum-free-gib N]

  --revision    the full commit SHA; the checkout's HEAD must be this commit
                and its tracked files unmodified
  --tag         the release tag, recorded in debian/changelog and
                rmac-source.json; it must point at --revision
  --source-dir  a directory written by the `source` mode
  --output      where the results go; must not exist yet
  --work-dir    scratch space, emptied after each run
                (default: ~/rmac-source-build)
  --jobs        Cargo jobs for the rebuild (default: CARGO_BUILD_JOBS or 1)
  --minimum-free-gib
                refuse to start with less free space than this on WORK's
                filesystem (default 25)
EOF
}

fail() {
  echo "build-rmac-source-package: $*" >&2
  exit 1
}

[[ $# -gt 0 ]] || { usage; exit 2; }
mode=$1
shift
case "$mode" in
  source|rebuild) ;;
  -h|--help) usage; exit 0 ;;
  *) usage; exit 2 ;;
esac

revision=""
tag=""
source_dir=""
output=""
work_dir="${HOME:-}/rmac-source-build"
jobs=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --revision|--tag|--source-dir|--output|--work-dir|--jobs|--minimum-free-gib)
      option=$1
      shift
      [[ $# -gt 0 ]] || { usage; exit 2; }
      case "$option" in
        --revision) [[ "$mode" == source ]] || { usage; exit 2; }; revision=$1 ;;
        --tag) [[ "$mode" == source ]] || { usage; exit 2; }; tag=$1 ;;
        --source-dir) [[ "$mode" == rebuild ]] || { usage; exit 2; }; source_dir=$1 ;;
        --output) output=$1 ;;
        --work-dir) work_dir=$1 ;;
        --jobs)
          [[ "$mode" == rebuild && "$1" =~ ^[1-9][0-9]*$ ]] || { usage; exit 2; }
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

[[ -n "$output" ]] || { usage; exit 2; }
if [[ "$mode" == source ]]; then
  [[ "$revision" =~ ^[0-9a-f]{40}$ ]] || fail "--revision must be a full lowercase commit SHA"
  if [[ -n "$tag" && ! "$tag" =~ ^[A-Za-z0-9][A-Za-z0-9._+-]*$ ]]; then
    fail "--tag is not a plain tag name"
  fi
else
  [[ -n "$source_dir" ]] || { usage; exit 2; }
  [[ "$source_dir" == /* && -d "$source_dir" && ! -L "$source_dir" ]] \
    || fail "--source-dir must be an absolute, ordinary directory"
fi

[[ "$(uname -s)" == Linux ]] || fail "Linux is required"
[[ ${EUID} -ne 0 ]] || fail "run as your normal user, not root"
[[ "$work_dir" == /* && "$work_dir" != / ]] || fail "--work-dir must be an absolute, non-root path"
[[ "$output" == /* && "$output" != / ]] || fail "--output must be an absolute, non-root path"
[[ ! -e "$output" && ! -L "$output" ]] || fail "$output already exists"

tools=(python3 sha256sum tar xz make dpkg-buildpackage dpkg-source dpkg-parsechangelog dpkg-architecture cargo)
if [[ "$mode" == source ]]; then
  tools+=(git)
else
  tools+=(dpkg-deb dpkg-checkbuilddeps dpkg-genchanges dpkg-shlibdeps)
fi
for tool in "${tools[@]}"; do
  command -v "$tool" >/dev/null 2>&1 || fail "'$tool' is required but was not found"
done

mkdir -p "$work_dir"
[[ -d "$work_dir" && ! -L "$work_dir" ]] || fail "$work_dir must be an ordinary directory"
available_kib="$(df -Pk "$work_dir" | awk 'NR == 2 { print $4 }')"
if [[ ! "$available_kib" =~ ^[0-9]+$ ]] || (( available_kib < minimum_free_gib * 1024 * 1024 )); then
  fail "$work_dir has less than $minimum_free_gib GiB free"
fi
mkdir -p "$(dirname "$output")"

run_dir="$(mktemp -d "$work_dir/run.XXXXXX")"
staging="$(mktemp -d "$(dirname "$output")/.$(basename "$output").XXXXXX")"
trap 'rm -rf "$run_dir" "$staging"' EXIT HUP INT TERM

export LC_ALL=C
export PYTHONDONTWRITEBYTECODE=1

# The repository pins its toolchain in rust-toolchain.toml; name it
# explicitly (as build-niri-packages.sh does) so rustup never tries to add the
# file's clippy and rustfmt components.
if command -v rustup >/dev/null 2>&1 && [[ -z "${RUSTUP_TOOLCHAIN:-}" ]]; then
  RUSTUP_TOOLCHAIN="$(sed -n 's/^channel *= *"\([^"]*\)".*/\1/p' "$repo_root/rust-toolchain.toml")"
  [[ -n "$RUSTUP_TOOLCHAIN" ]] || fail "rust-toolchain.toml has no channel"
  export RUSTUP_TOOLCHAIN
fi

build_source() {
  local head
  head="$(git -C "$repo_root" rev-parse --verify 'HEAD^{commit}')" \
    || fail "$repo_root is not a git checkout"
  [[ "$head" == "$revision" ]] || fail "the checkout is at $head, not $revision"
  git -C "$repo_root" update-index -q --refresh || true
  git -C "$repo_root" diff-index --quiet HEAD -- \
    || fail "the checkout has modified tracked files; the source must be exactly $revision"
  if [[ -n "$tag" ]]; then
    local tagged
    tagged="$(git -C "$repo_root" rev-parse -q --verify "refs/tags/$tag^{commit}")" \
      || fail "tag $tag is not in the checkout"
    [[ "$tagged" == "$revision" ]] || fail "tag $tag is $tagged, not $revision"
  fi
  local epoch
  epoch="$(git -C "$repo_root" log -1 --format=%ct "$revision")"
  [[ "$epoch" =~ ^[0-9]+$ ]] || fail "could not read the commit time of $revision"

  local version
  version="$(python3 "$helper" version --repo-root "$repo_root")" \
    || fail "the workspace version is not canonical"
  local names
  names="$(python3 "$helper" shell-vars --version "$version")" \
    || fail "$version is not a source package version"
  eval "$names"
  echo "build-rmac-source-package: rmac $SRC_VERSION from $revision${tag:+ ($tag)}"
  echo "build-rmac-source-package: using $(cargo --version)"

  local parent="$run_dir/source"
  mkdir -p "$parent"
  # git archive stamps every member with the commit time, so the tarball
  # depends only on the commit; single-threaded xz keeps it deterministic.
  git -C "$repo_root" archive --format=tar --prefix="$SRC_TOP_DIRECTORY/" "$revision" \
    | xz -T1 -6 -c >"$parent/$SRC_ORIG" \
    || fail "git archive of $revision failed"
  tar -xJf "$parent/$SRC_ORIG" -C "$parent"
  local tree="$parent/$SRC_TOP_DIRECTORY"
  [[ -f "$tree/Cargo.lock" && -f "$tree/shell/Cargo.lock" ]] \
    || fail "the export is missing Cargo.lock or shell/Cargo.lock"
  [[ ! -e "$tree/debian" && ! -e "$tree/vendor" ]] \
    || fail "the export already has a top-level debian/ or vendor/"
  [[ "$(python3 "$helper" version --repo-root "$tree")" == "$SRC_VERSION" ]] \
    || fail "the exported tree's version differs from the checkout's"

  # The only networked step: both workspaces' exact Cargo.lock graphs into one
  # vendor/ directory. cargo prints the source replacement on stdout.
  (cd "$tree" && cargo vendor --locked --sync shell/Cargo.toml vendor >"$parent/cargo-vendor-config.toml") \
    || fail "cargo vendor failed"
  python3 "$helper" check-vendor-config --input "$parent/cargo-vendor-config.toml" \
    || fail "cargo vendor printed an unexpected source replacement"
  mv "$parent/cargo-vendor-config.toml" "$tree/vendor/.lulo-cargo-config.toml"

  (cd "$tree" && tar --format=gnu --sort=name --mtime="@$epoch" \
      --owner=0 --group=0 --numeric-owner --mode=go-w -cf - vendor) \
    | xz -T1 -6 -c >"$parent/$SRC_ORIG_VENDOR" \
    || fail "packing vendor/ failed"
  local vendor_sha256
  vendor_sha256="$(sha256sum "$parent/$SRC_ORIG_VENDOR" | awk '{ print $1 }')"
  echo "build-rmac-source-package: $SRC_ORIG_VENDOR SHA-256 $vendor_sha256"

  cp -R "$tree/packaging/rmac-source/debian" "$tree/debian"
  chmod 0755 "$tree/debian/rules"
  local tag_arguments=()
  [[ -z "$tag" ]] || tag_arguments=(--tag "$tag")
  python3 "$helper" changelog --version "$SRC_VERSION" --commit "$revision" \
    --epoch "$epoch" ${tag_arguments[@]+"${tag_arguments[@]}"} \
    --output "$tree/debian/changelog" || fail "could not write debian/changelog"
  printf '%s\n' "$revision" >"$tree/debian/rmac-revision"
  python3 "$helper" notices --vendor "$tree/vendor" --version "$SRC_VERSION" \
    --commit "$revision" --output "$tree/debian/dependency-licenses.txt" \
    || fail "a vendored crate declares no licence"
  [[ "$(dpkg-parsechangelog -l "$tree/debian/changelog" -SVersion)" == "$SRC_VERSION" ]] \
    || fail "debian/changelog does not parse to $SRC_VERSION"

  # Source only; -d because making the source package needs none of the
  # Build-Depends (the rebuild mode checks them).
  (cd "$tree" && SOURCE_DATE_EPOCH="$epoch" dpkg-buildpackage -S -sa -us -uc -d) \
    || fail "dpkg-buildpackage -S failed"

  local name
  for name in "$SRC_DSC" "$SRC_ORIG" "$SRC_ORIG_VENDOR" "$SRC_DEBIAN" "$SRC_BUILDINFO" "$SRC_CHANGES"; do
    [[ -f "$parent/$name" ]] || fail "dpkg-buildpackage did not produce $name"
    install -m 0644 "$parent/$name" "$staging/$name"
  done
  python3 "$helper" manifest --directory "$staging" --version "$SRC_VERSION" \
    --commit "$revision" --epoch "$epoch" ${tag_arguments[@]+"${tag_arguments[@]}"} \
    --source-tree "$tree" || fail "could not write rmac-source.json"
  chmod 0644 "$staging/rmac-source.json"
}

rebuild_binaries() {
  local listed present
  [[ -f "$source_dir/SHA256SUMS" ]] || fail "$source_dir has no SHA256SUMS"
  (cd "$source_dir" && sha256sum --check --strict --quiet SHA256SUMS) \
    || fail "$source_dir does not match its SHA256SUMS"
  listed="$(awk '{ print $2 }' "$source_dir/SHA256SUMS" | sort)"
  present="$(cd "$source_dir" && find . -mindepth 1 -maxdepth 1 ! -name SHA256SUMS -printf '%P\n' | sort)"
  [[ "$listed" == "$present" ]] || fail "SHA256SUMS does not list exactly the files in $source_dir"
  local identity
  identity="$(python3 "$helper" check-source-dir --directory "$source_dir")" \
    || fail "$source_dir does not match its rmac-source.json"
  eval "$identity"

  local architecture
  architecture="$(dpkg-architecture -qDEB_HOST_ARCH)"
  case "$architecture" in
    amd64|arm64) ;;
    *) fail "only amd64 and arm64 are supported, not $architecture" ;;
  esac
  echo "build-rmac-source-package: rebuilding rmac $SRC_VERSION ($SRC_COMMIT) for $architecture"
  echo "build-rmac-source-package: using $(cargo --version)"

  local parent="$run_dir/rebuild"
  local tree="$parent/$SRC_TOP_DIRECTORY"
  mkdir -p "$parent"
  dpkg-source -x "$source_dir/$SRC_DSC" "$tree" || fail "dpkg-source -x failed"
  [[ "$(cat "$tree/debian/rmac-revision")" == "$SRC_COMMIT" ]] \
    || fail "debian/rmac-revision does not name $SRC_COMMIT"

  (
    cd "$tree"
    export CARGO_NET_OFFLINE=true
    [[ -z "$jobs" ]] || export CARGO_BUILD_JOBS="$jobs"
    dpkg-buildpackage -b -us -uc
  ) || fail "dpkg-buildpackage -b failed"

  local stem="rmac_${SRC_VERSION}_${architecture}"
  local verify="$parent/verify"
  local name
  mkdir -p "$verify"
  for name in "rmac-apps_${SRC_VERSION}_${architecture}.deb" "rmac-session_${SRC_VERSION}_${architecture}.deb"; do
    [[ -f "$parent/$name" ]] || fail "dpkg-buildpackage did not produce $name"
    install -m 0644 "$parent/$name" "$verify/$name"
    install -m 0644 "$parent/$name" "$staging/$name"
  done
  for name in native-packages.json SHA256SUMS; do
    install -m 0644 "$tree/debian/native-packages/$name" "$verify/$name"
  done
  python3 "$tree/scripts/linux/verify-native-packages.py" --directory "$verify" \
    --architecture "$architecture" --version "$SRC_VERSION" \
    || fail "the rebuilt packages do not verify"
  for name in "$stem.buildinfo" "$stem.changes"; do
    [[ -f "$parent/$name" ]] || fail "dpkg-buildpackage did not produce $name"
    install -m 0644 "$parent/$name" "$staging/$name"
  done
  python3 "$helper" rebuild-manifest --directory "$staging" --source-dir "$source_dir" \
    --architecture "$architecture" || fail "could not write rebuild.json"
  chmod 0644 "$staging/rebuild.json"
}

if [[ "$mode" == source ]]; then
  build_source
else
  rebuild_binaries
fi

(cd "$staging" && sha256sum -- * >SHA256SUMS)
chmod 0644 "$staging/SHA256SUMS"
chmod 0755 "$staging"
mv "$staging" "$output"
rm -rf "$run_dir"
trap - EXIT HUP INT TERM
echo "build-rmac-source-package: wrote $output"
ls -l "$output"
