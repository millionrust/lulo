#!/bin/sh
# Install rmac (Lulo OS) on a disposable Ubuntu 26.04 test account.
#
# Usage:
#   curl -fsSL https://millionrust.github.io/lulo/install.sh | sh
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

fail() {
    echo "install.sh: $*" >&2
    exit 1
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

main() {
    check_not_root
    check_placeholder_fingerprint_was_replaced
    check_platform
    download_and_verify_keyring
    install_keyring_and_repository
    install_session
    echo
    echo "rmac is installed alongside Ubuntu/GNOME. Nothing about your current"
    echo "session changed."
    echo
    echo "Log out and choose Lulo OS on the login screen."
}

main "$@"
