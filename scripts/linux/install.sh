#!/bin/sh
# Install rmac (Lulo OS) on a disposable Ubuntu 26.04 test account.
#
# Usage:
#   curl -fsSL https://millionrust.github.io/lulo/install.sh | sh
#
#   # Beta path, before the signed APT repository exists: install the two
#   # .debs straight from a tagged GitHub Release, verified by SHA256SUMS
#   # (and, when `gh` is installed, its build-provenance attestation).
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

# TODO(owner): docs/update-trust.md's "Decisions needed" still has to settle
# who holds the signing keys before the real archive key can be generated
# (docs/install.md "Server side (GitHub)"). This placeholder is not a valid
# OpenPGP fingerprint on purpose, so install.sh refuses to run until it is
# replaced with the real 40- or 64-character hex fingerprint published in
# the README, docs/install.md, and the GitHub Release notes.
RMAC_ARCHIVE_KEYRING_FINGERPRINT="TODO_REPLACE_WITH_THE_REAL_ARCHIVE_FINGERPRINT"

# The GitHub repository that `--from-release` downloads .debs from, and that
# `gh attestation verify` checks build provenance against.
RMAC_GITHUB_REPOSITORY="millionrust/lulo"

fail() {
    echo "install.sh: $*" >&2
    exit 1
}

usage() {
    cat >&2 <<'EOF'
usage: install.sh [--from-release TAG | --from-dir DIRECTORY]

  (no argument)        Install from the signed rmac APT repository. Not
                        available yet; see docs/install.md.
  --from-release TAG   Download rmac-apps and rmac-session for this
                        machine's architecture from the named GitHub
                        Release tag (e.g. v0.5.0), verify them against the
                        release's SHA256SUMS (and its build-provenance
                        attestation when `gh` is installed), then install
                        them with apt.
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
            fail "the archive keyring fingerprint has not been published yet; rmac is not released for install yet (see docs/install.md)"
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
    # A gpg --with-colons "fpr" record is exactly
    # "fpr:::::::::<fingerprint>:" (nine empty fields, then the fingerprint).
    match=$(printf '%s\n' "$fingerprint_listing" \
        | grep -Fc "fpr:::::::::${RMAC_ARCHIVE_KEYRING_FINGERPRINT}:" || true)
    [ "$match" -ge 1 ] || fail "the downloaded keyring does not contain the pinned rmac archive fingerprint"

    downloaded_keyring_deb="$keyring_deb"
}

install_keyring_and_repository() {
    sudo dpkg -i "$downloaded_keyring_deb" \
        || fail "could not install the rmac archive keyring package"
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
Package: rmac-apps rmac-archive-keyring rmac-session
Pin: release o=rmac,n=resolute,c=main
Pin-Priority: 500
EOF
}

install_session() {
    sudo apt-get update \
        || fail "apt update failed; check the rmac repository configuration"
    sudo apt-get install --yes rmac-session \
        || fail "installing rmac-session failed"
}

# Find the single filename in a SHA256SUMS listing for one package and this
# machine's architecture (e.g. "rmac-apps" -> "rmac-apps_1.2.3-4_amd64.deb").
# Fails loudly if there is not exactly one match, rather than guess.
release_asset_name() {
    sums_file="$1"
    package="$2"
    pattern="^${package}_[0-9]+\\.[0-9]+\\.[0-9]+-[0-9]+_${architecture}\\.deb\$"
    matches="$(awk '{print $NF}' "$sums_file" | grep -E "$pattern" || true)"
    count="$(printf '%s\n' "$matches" | grep -c . || true)"
    [ "$count" -eq 1 ] \
        || fail "SHA256SUMS did not list exactly one $package package for $architecture (found $count)"
    printf '%s' "$matches"
}

# Download rmac-apps and rmac-session for this architecture from a tagged
# GitHub Release, verify them against that release's SHA256SUMS, and -- when
# `gh` is installed -- verify actions/attest-build-provenance attestations
# too (see .github/workflows/release.yml "attach-release"). HTTPS transport
# is never treated as authentication on its own; the checksum (and, when
# available, the attestation) is what is actually trusted, matching the
# APT-repository path's stance in docs/update-trust.md.
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

    work_dir="$(mktemp -d)"
    trap 'rm -rf "$work_dir"' EXIT

    base_url="https://github.com/${RMAC_GITHUB_REPOSITORY}/releases/download/${tag}"

    curl -fsSL --proto '=https' --tlsv1.2 -o "$work_dir/SHA256SUMS" \
        "$base_url/SHA256SUMS" \
        || fail "could not download SHA256SUMS for release $tag"

    apps_name="$(release_asset_name "$work_dir/SHA256SUMS" rmac-apps)"
    session_name="$(release_asset_name "$work_dir/SHA256SUMS" rmac-session)"

    for name in "$apps_name" "$session_name"; do
        curl -fsSL --proto '=https' --tlsv1.2 -o "$work_dir/$name" \
            "$base_url/$name" \
            || fail "could not download $name from release $tag"
    done

    verify_local_package_directory "$work_dir"

    if command -v gh >/dev/null 2>&1; then
        for name in "$apps_name" "$session_name"; do
            gh attestation verify "$work_dir/$name" --repo "$RMAC_GITHUB_REPOSITORY" \
                || fail "build provenance attestation did not verify for $name"
        done
    else
        echo "install.sh: 'gh' is not installed; skipping the optional build-provenance attestation check (SHA256SUMS was still verified)" >&2
    fi

    downloaded_package_dir="$work_dir"
}

# Verify the SHA256SUMS entries for exactly the rmac-apps and rmac-session
# packages present in DIRECTORY, and record their paths in $apps_deb /
# $session_deb. Never trusts a directory's SHA256SUMS blindly: only the two
# lines that name our own packages for this architecture are checked, so an
# incomplete local copy (missing the SBOM or the other architecture) is not
# treated as a verification failure.
verify_local_package_directory() {
    directory="$1"
    [ -n "$directory" ] || fail "a package directory is required"
    [ -d "$directory" ] && [ ! -L "$directory" ] \
        || fail "$directory must be an existing, non-symlinked directory"
    [ -f "$directory/SHA256SUMS" ] \
        || fail "$directory does not contain a SHA256SUMS file"
    require_command sha256sum

    apps_name="$(release_asset_name "$directory/SHA256SUMS" rmac-apps)"
    session_name="$(release_asset_name "$directory/SHA256SUMS" rmac-session)"

    # No temporary file (and so no EXIT trap) here on purpose: this function
    # can run inside download_release_packages, which already owns the EXIT
    # trap for its own work directory, and a second `trap ... EXIT` in the
    # same shell would silently replace (not stack with) the first one.
    selected_count="$(grep -cE "  (${apps_name}|${session_name})\$" "$directory/SHA256SUMS" || true)"
    [ "$selected_count" -eq 2 ] \
        || fail "SHA256SUMS did not list exactly one line for each of $apps_name and $session_name"
    (cd "$directory" && grep -E "  (${apps_name}|${session_name})\$" SHA256SUMS | sha256sum -c -) \
        || fail "downloaded/local package checksums did not match SHA256SUMS"

    apps_deb="$directory/$apps_name"
    session_deb="$directory/$session_name"
}

# Install an already-checksum-verified rmac-apps/rmac-session pair with apt,
# so missing Depends (keyd, niri, wl-clipboard, ...) still resolve from the
# machine's normal Ubuntu archive. This never touches the APT repository
# configuration: nothing here writes a sources.list.d/preferences.d entry,
# so uninstall.sh's repository cleanup is simply a no-op for this path.
install_local_packages() {
    directory="$1"
    verify_local_package_directory "$directory"
    sudo apt-get install --yes -- "$apps_deb" "$session_deb" \
        || fail "installing rmac-apps and rmac-session from $directory failed"
}

main() {
    mode="repo"
    release_tag=""
    local_dir=""
    while [ $# -gt 0 ]; do
        case "$1" in
            --from-release)
                shift
                [ $# -gt 0 ] || fail "--from-release requires a release tag"
                mode="release"
                release_tag="$1"
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
