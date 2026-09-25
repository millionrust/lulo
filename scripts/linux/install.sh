#!/bin/sh
# Install rmac (Lulo OS) on a disposable Ubuntu 26.04 test account.
#
# Usage:
#   # Default: add the signed rmac APT repository (its keyring is checked
#   # against the fingerprint pinned below before APT trusts it) and install
#   # rmac-session, rmac-apps, niri, and xwayland-satellite from it. APT and
#   # PackageKit then see every later Lulo OS release as an ordinary update,
#   # and rmac-update-check.timer prepares it to install at the next restart.
#   curl -fsSL https://millionrust.github.io/lulo/install.sh | sh
#
#   # Offline or pinned installs: rmac-apps, rmac-session, and Lulo OS's niri
#   # and xwayland-satellite builds straight from a tagged GitHub Release,
#   # verified by SHA256SUMS and its build-provenance attestation, which
#   # needs `gh` (run `gh auth login` first). This path adds no repository,
#   # so it receives no automatic updates.
#   sh install.sh --from-release vX.Y.Z
#
#   # Or from a directory you already downloaded/verified yourself (for
#   # example scp'd from a build host, or produced by build-native-inputs.sh
#   # + check-native-reproducibility.sh):
#   sh install.sh --from-dir /absolute/path/to/native-package-set
#
# This script only ever adds packages; it never removes Ubuntu/GNOME and
# never touches the GNOME session (no session-default change, no GDM
# restart). See docs/install.md for the equivalent manual steps and
# docs/update-trust.md for what this bootstrap is, and is not, trusted to do.
#
# The whole install lives in main(), called once on the last line, so a
# truncated download (a network failure mid-`curl | sh`) can only run a
# partial function body -- never a partial, half-executed script.
set -eu

RMAC_REPOSITORY_URI="https://millionrust.github.io/lulo/"
RMAC_SUITE="resolute"
RMAC_COMPONENT="main"
RMAC_KEYRING_PATH="/usr/share/keyrings/rmac-archive-keyring.gpg"
RMAC_SOURCES_PATH="/etc/apt/sources.list.d/rmac.sources"
RMAC_PREFERENCES_PATH="/etc/apt/preferences.d/rmac.pref"
# The bootstrap keyring package is republished at this fixed name beside
# every promoted APT snapshot (docs/release-process.md); the versioned
# rmac-archive-keyring_*.deb it installs still lives in the signed pool.
RMAC_KEYRING_URL="${RMAC_REPOSITORY_URI}rmac-archive-keyring-latest.deb"

# The archive's offline PRIMARY key fingerprint. It must equal
# packaging/apt/archive-key.json; scripts/release/create-archive-key.sh
# writes both (docs/release-process.md "Switching on signed updates").
# Until the owner has created the key this is a placeholder that is not a
# valid OpenPGP fingerprint on purpose, so the repository install refuses to
# run and points at --from-release instead.
RMAC_ARCHIVE_KEYRING_FINGERPRINT="TODO_REPLACE_WITH_THE_REAL_ARCHIVE_FINGERPRINT"

# The GitHub repository that `--from-release` downloads .debs from, and that
# `gh attestation verify` checks build provenance against.
RMAC_GITHUB_REPOSITORY="millionrust/lulo"

# Lulo OS's own builds of rmac-session's compositor dependencies, published
# in the same GitHub Release (docs/release-process.md "Third-party packages:
# niri and xwayland-satellite"). Neither is in the Ubuntu 26.04 archive.
RMAC_THIRD_PARTY_PACKAGES="xwayland-satellite niri"

fail() {
    echo "install.sh: $*" >&2
    exit 1
}

usage() {
    cat >&2 <<'EOF'
usage: install.sh [--from-release TAG [--allow-unattested] | --from-dir DIRECTORY]

  (no argument)        Add the signed rmac APT repository (keyring checked
                        against the pinned archive fingerprint) and install
                        rmac-session from it. Later releases arrive as
                        ordinary APT/PackageKit updates.
  --from-release TAG   Offline/pinned install without the repository (no
                        automatic updates): download rmac-apps, rmac-session, niri, and
                        xwayland-satellite for this machine's architecture
                        from the named GitHub Release tag (e.g. v0.5.0),
                        verify them against the release's SHA256SUMS and
                        its build-provenance attestation (`gh attestation
                        verify`; install gh and run `gh auth login`
                        first), then install them with apt. A newer
                        niri or xwayland-satellite that is already
                        installed (for example from a PPA) is kept.
  --allow-unattested   With --from-release on a machine without gh:
                        install on SHA256SUMS alone. Who built the
                        packages then rests on HTTPS and the GitHub
                        account, not on a signature.
  --from-dir DIRECTORY Install from .deb files and a SHA256SUMS you already
                        have locally, skipping the download.
EOF
}

require_command() {
    command -v "$1" >/dev/null 2>&1 || fail "'$1' is required but was not found"
}

check_not_root() {
    if [ "$(id -u)" -eq 0 ]; then
        fail "run this as your normal user, not root or sudo; it calls sudo itself for the steps that need it"
    fi
    require_command sudo
}

check_placeholder_fingerprint_was_replaced() {
    case "$RMAC_ARCHIVE_KEYRING_FINGERPRINT" in
        TODO_*)
            fail "the signed rmac APT repository is not published yet; install a tagged release with: sh install.sh --from-release vX.Y.Z (see docs/install.md)"
            ;;
    esac
}

check_platform() {
    [ -r /etc/os-release ] || fail "cannot read /etc/os-release to identify this machine"
    # shellcheck disable=SC1091
    . /etc/os-release
    [ "${ID:-}" = "ubuntu" ] && [ "${VERSION_ID:-}" = "26.04" ] \
        || fail "rmac targets Ubuntu 26.04; this machine reports ${ID:-unknown} ${VERSION_ID:-unknown}"

    require_command dpkg
    architecture="$(dpkg --print-architecture)"
    case "$architecture" in
        amd64 | arm64) ;;
        *) fail "rmac supports amd64 and arm64, not $architecture" ;;
    esac
}

download_and_verify_keyring() {
    require_command curl
    require_command gpg
    require_command dpkg-deb

    work_dir="$(mktemp -d)"
    trap 'rm -rf "$work_dir"' EXIT

    keyring_deb="$work_dir/rmac-archive-keyring.deb"
    curl -fsSL --proto '=https' --tlsv1.2 -o "$keyring_deb" "$RMAC_KEYRING_URL" \
        || fail "could not download the rmac archive keyring package"

    extracted="$work_dir/extracted"
    dpkg-deb -x "$keyring_deb" "$extracted" \
        || fail "the downloaded keyring package could not be unpacked"
    keyring_file="$extracted$RMAC_KEYRING_PATH"
    [ -s "$keyring_file" ] || fail "the downloaded keyring package did not contain a keyring"

    fingerprint_listing="$(gpg --batch --no-default-keyring --with-colons --show-keys "$keyring_file" 2>/dev/null)" \
        || fail "the downloaded keyring could not be parsed"
    verify_keyring_listing "$fingerprint_listing"

    verified_keyring_file="$keyring_file"
}

# The pinned fingerprint only authenticates anything when it is the ONLY
# primary key APT will trust through Signed-By: a keyring that also carried
# a second (attacker) primary key would let that key sign InRelease. So
# require exactly one "pub" record, and require the fingerprint record right
# after it (the primary key's own; later "fpr" records belong to subkeys)
# to be the pinned one. A gpg --with-colons "fpr" record is exactly
# "fpr:::::::::<fingerprint>:" (nine empty fields, then the fingerprint).
verify_keyring_listing() {
    primary_keys=$(printf '%s\n' "$1" | grep -c '^pub:' || true)
    [ "$primary_keys" -eq 1 ] \
        || fail "the downloaded keyring must contain exactly one primary key (found $primary_keys)"
    primary_fingerprint=$(printf '%s\n' "$1" \
        | awk -F: '$1 == "pub" { want = 1; next } want && $1 == "fpr" { print $10; exit }')
    [ "$primary_fingerprint" = "$RMAC_ARCHIVE_KEYRING_FINGERPRINT" ] \
        || fail "the downloaded keyring does not contain the pinned rmac archive fingerprint"
}

install_keyring_and_repository() {
    # Install only the fingerprint-verified public keyring file, never the
    # downloaded .deb itself: that package is signed by nothing, so
    # `dpkg -i` would run its maintainer scripts and unpack any other files
    # it carries as root on the strength of an HTTPS download alone. The
    # rmac-archive-keyring package then comes from the signed repository
    # (install_session) and takes this file over.
    sudo install -o root -g root -m 0644 "$verified_keyring_file" "$RMAC_KEYRING_PATH" \
        || fail "could not install the rmac archive keyring"
    [ -s "$RMAC_KEYRING_PATH" ] || fail "the rmac archive keyring did not install to $RMAC_KEYRING_PATH"

    sudo tee "$RMAC_SOURCES_PATH" >/dev/null <<EOF
Types: deb deb-src
URIs: $RMAC_REPOSITORY_URI
Suites: $RMAC_SUITE
Components: $RMAC_COMPONENT
Architectures: amd64 arm64
Signed-By: $RMAC_KEYRING_PATH
Check-Valid-Until: yes
EOF

    sudo tee "$RMAC_PREFERENCES_PATH" >/dev/null <<'EOF'
Package: niri rmac-apps rmac-archive-keyring rmac-session xwayland-satellite
Pin: release o=rmac,n=resolute,c=main
Pin-Priority: 500

Package: *
Pin: release o=rmac
Pin-Priority: -1
EOF
}

install_session() {
    sudo apt-get update \
        || fail "apt update failed; check the rmac repository configuration"
    if ! sudo apt-get install --yes rmac-archive-keyring rmac-session; then
        if [ "$architecture" = arm64 ]; then
            fail "installing rmac-session failed; the repository may not carry arm64 builds yet (only the keyring is published for architectures without a build)"
        fi
        fail "installing rmac-session failed"
    fi
}

# Find the filenames in a SHA256SUMS listing for one package and this
# machine's architecture (e.g. "rmac-apps" -> "rmac-apps_1.2.3-4_amd64.deb").
# Any Debian version is accepted, including a pre-release whose "~" the
# Release workflow rewrote to "." (GitHub renames "~" in asset names).
release_asset_matches() {
    sums_file="$1"
    package="$2"
    pattern="^${package}_[0-9A-Za-z.+~-]+_${architecture}\\.deb\$"
    awk '{print $NF}' "$sums_file" | grep -E "$pattern" || true
}

# Exactly one match, or fail loudly rather than guess.
release_asset_name() {
    matches="$(release_asset_matches "$1" "$2")"
    count="$(printf '%s\n' "$matches" | grep -c . || true)"
    [ "$count" -eq 1 ] \
        || fail "SHA256SUMS did not list exactly one $2 package for $architecture (found $count)"
    printf '%s' "$matches"
}

# Zero or one match; prints nothing when the listing has none.
optional_release_asset_name() {
    matches="$(release_asset_matches "$1" "$2")"
    count="$(printf '%s\n' "$matches" | grep -c . || true)"
    [ "$count" -le 1 ] \
        || fail "SHA256SUMS listed more than one $2 package for $architecture (found $count)"
    printf '%s' "$matches"
}

# Choose the package files to verify and install from one SHA256SUMS:
# rmac-apps and rmac-session (required) plus Lulo OS's own niri and
# xwayland-satellite builds when the listing has them. Every release made by
# release.yml does; a hand-assembled --from-dir set may not, and then apt has
# to find them in another configured source. Sets $selected_names, a
# space-separated list (the name pattern above admits no whitespace).
select_release_assets() {
    sums_file="$1"
    selected_names="$(release_asset_name "$sums_file" rmac-apps) $(release_asset_name "$sums_file" rmac-session)"
    for package in $RMAC_THIRD_PARTY_PACKAGES; do
        name="$(optional_release_asset_name "$sums_file" "$package")"
        if [ -n "$name" ]; then
            selected_names="$selected_names $name"
        else
            echo "install.sh: this package set has no $package for $architecture; apt must find it in another configured source" >&2
        fi
    done
}

# Download rmac-apps, rmac-session, niri, and xwayland-satellite for this
# architecture from a tagged GitHub Release and verify them against that
# release's SHA256SUMS. SHA256SUMS comes from the same release, so it proves
# only that the download is intact, not who built it. The build-provenance
# attestation is the authenticity check: it is mandatory and must have been
# signed by this repository's release workflow (.github/workflows/release.yml
# "attach-release"). Without `gh` the install refuses (SR-17), because these
# packages' maintainer scripts run as root; only an explicit
# --allow-unattested ($allow_unattested=true) accepts SHA256SUMS alone, and
# then install.sh says what was not checked.
#
# Sets $downloaded_package_dir rather than returning the path on stdout: a
# caller capturing this function's output with "$(...)" would run it in a
# subshell, and the `trap ... EXIT` below would then fire -- deleting the
# work directory -- the moment that subshell exits, before the caller ever
# gets to use the path it just printed.
download_release_packages() {
    tag="$1"
    [ -n "$tag" ] || fail "--from-release requires a release tag, e.g. v0.5.0"
    require_command curl
    # Refuse before downloading anything when the attestation cannot be
    # checked and the person has not accepted that explicitly.
    if ! command -v gh >/dev/null 2>&1 && [ "${allow_unattested:-false}" != true ]; then
        fail "cannot check who built release $tag: 'gh' is not installed. Install it (sudo apt-get install gh), run 'gh auth login', and try again; or pass --allow-unattested to trust SHA256SUMS and HTTPS alone"
    fi

    work_dir="$(mktemp -d)"
    trap 'rm -rf "$work_dir"' EXIT

    base_url="https://github.com/${RMAC_GITHUB_REPOSITORY}/releases/download/${tag}"

    curl -fsSL --proto '=https' --tlsv1.2 -o "$work_dir/SHA256SUMS" \
        "$base_url/SHA256SUMS" \
        || fail "could not download SHA256SUMS for release $tag"

    select_release_assets "$work_dir/SHA256SUMS"

    for name in $selected_names; do
        curl -fsSL --proto '=https' --tlsv1.2 -o "$work_dir/$name" \
            "$base_url/$name" \
            || fail "could not download $name from release $tag"
    done

    verify_local_package_directory "$work_dir"

    if command -v gh >/dev/null 2>&1; then
        for name in $selected_names; do
            gh attestation verify "$work_dir/$name" --repo "$RMAC_GITHUB_REPOSITORY" \
                --signer-workflow "$RMAC_GITHUB_REPOSITORY/.github/workflows/release.yml" \
                || fail "build provenance attestation did not verify for $name (is 'gh auth login' done?)"
        done
    elif [ "${allow_unattested:-false}" = true ]; then
        echo "install.sh: --allow-unattested: 'gh' is not installed, so the build-provenance attestation was not checked." >&2
        echo "install.sh: SHA256SUMS only proves the download is intact; who built it rests on HTTPS and the GitHub account." >&2
    fi

    downloaded_package_dir="$work_dir"
}

# Verify the SHA256SUMS entries for exactly the packages
# select_release_assets chose from DIRECTORY. Never trusts a directory's
# SHA256SUMS blindly: only the lines that name those packages for this
# architecture are checked, so an incomplete local copy (missing the SBOM,
# the source packages, or the other architecture) is not treated as a
# verification failure.
verify_local_package_directory() {
    directory="$1"
    [ -n "$directory" ] || fail "a package directory is required"
    [ -d "$directory" ] && [ ! -L "$directory" ] \
        || fail "$directory must be an existing, non-symlinked directory"
    [ -f "$directory/SHA256SUMS" ] \
        || fail "$directory does not contain a SHA256SUMS file"
    require_command sha256sum

    select_release_assets "$directory/SHA256SUMS"

    # No temporary file (and so no EXIT trap) here on purpose: this function
    # can run inside download_release_packages, which already owns the EXIT
    # trap for its own work directory, and a second `trap ... EXIT` in the
    # same shell would silently replace (not stack with) the first one.
    for name in $selected_names; do
        line_count="$(awk -v name="$name" '$NF == name' "$directory/SHA256SUMS" | grep -c . || true)"
        [ "$line_count" -eq 1 ] \
            || fail "SHA256SUMS did not list exactly one line for $name"
    done
    (
        cd "$directory" || exit 1
        for name in $selected_names; do
            awk -v name="$name" '$NF == name' SHA256SUMS
        done | sha256sum -c -
    ) || fail "downloaded/local package checksums did not match SHA256SUMS"
}

# True when PACKAGE is already installed at a version newer than DEB's.
# Lulo OS's own build ("26.04+lulo1-1") sorts above the danklinux PPA's
# "26.04ppaN" for any N, so this normally lets an ordinary `apt-get install`
# upgrade the PPA build to ours. It still guards the rarer case -- an
# official Debian/Ubuntu package, or a later PPA release, already newer than
# this release's pinned build -- where apt would refuse the downgrade; that
# install is kept and the way to switch is printed instead.
installed_third_party_is_newer() {
    package="$1"
    deb="$2"
    status="$(dpkg-query -W -f='${db:Status-Abbrev}' "$package" 2>/dev/null || true)"
    case "$status" in
        ii*) ;;
        *) return 1 ;;
    esac
    installed="$(dpkg-query -W -f='${Version}' "$package")" \
        || fail "could not read the installed $package version"
    candidate="$(dpkg-deb -f "$deb" Version)" \
        || fail "could not read the version of $deb"
    if dpkg --compare-versions "$installed" gt "$candidate"; then
        echo "install.sh: keeping the installed $package $installed, which is newer than this release's $candidate and already satisfies rmac-session." >&2
        echo "install.sh: to use the Lulo OS build instead, run: sudo apt-get install --allow-downgrades ./$(basename "$deb") (from the release's or directory's copy)" >&2
        return 0
    fi
    return 1
}

# Install the already-checksum-verified packages with apt, so remaining
# Depends (keyd, wl-clipboard, xwayland, ...) still resolve from the
# machine's normal Ubuntu archive. This never touches the APT repository
# configuration: nothing here writes a sources.list.d/preferences.d entry,
# so uninstall.sh's repository cleanup is simply a no-op for this path.
install_local_packages() {
    directory="$1"
    require_command dpkg-deb
    require_command dpkg-query
    verify_local_package_directory "$directory"
    set --
    for name in $selected_names; do
        package="${name%%_*}"
        case " $RMAC_THIRD_PARTY_PACKAGES " in
            *" $package "*)
                if installed_third_party_is_newer "$package" "$directory/$name"; then
                    continue
                fi
                ;;
        esac
        set -- "$@" "$directory/$name"
    done
    sudo apt-get install --yes -- "$@" \
        || fail "installing the rmac packages from $directory failed"
}

main() {
    mode="repo"
    release_tag=""
    allow_unattested=false
    local_dir=""
    while [ $# -gt 0 ]; do
        case "$1" in
            --from-release)
                shift
                [ $# -gt 0 ] || fail "--from-release requires a release tag"
                mode="release"
                release_tag="$1"
                ;;
            --allow-unattested)
                allow_unattested=true
                ;;
            --from-dir)
                shift
                [ $# -gt 0 ] || fail "--from-dir requires a directory path"
                mode="dir"
                local_dir="$1"
                ;;
            -h | --help)
                usage
                return 0
                ;;
            *)
                usage
                fail "unrecognized argument: $1"
                ;;
        esac
        shift
    done

    if [ "$allow_unattested" = true ] && [ "$mode" != release ]; then
        fail "--allow-unattested applies only to --from-release"
    fi

    check_not_root

    case "$mode" in
        repo)
            # Unchanged from the original bootstrap: the placeholder
            # fingerprint check runs before check_platform on purpose, so a
            # not-yet-released rmac refuses for the right reason even on a
            # machine that also is not Ubuntu 26.04.
            check_placeholder_fingerprint_was_replaced
            check_platform
            download_and_verify_keyring
            install_keyring_and_repository
            install_session
            ;;
        release)
            check_platform
            download_release_packages "$release_tag"
            install_local_packages "$downloaded_package_dir"
            ;;
        dir)
            check_platform
            install_local_packages "$local_dir"
            ;;
    esac

    echo
    echo "rmac is installed alongside Ubuntu/GNOME. Nothing about your current"
    echo "session changed."
    echo
    echo "Log out and choose Lulo OS on the login screen."
}

main "$@"
