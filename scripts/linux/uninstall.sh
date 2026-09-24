#!/bin/sh
# Remove rmac (Lulo OS) and its repository configuration from this machine.
# Ubuntu/GNOME is never touched: this only purges the rmac packages and the
# apt configuration install.sh may have written.
#
# This reverses both install.sh paths with the same steps: the signed
# APT-repository install (rmac-archive-keyring plus its sources.list.d /
# preferences.d files) and the Beta `--from-release` / `--from-dir` path
# (which never wrote any repository configuration, so that cleanup is a
# harmless no-op there). No arguments are needed either way.
#
# Usage:
#   curl -fsSL https://millionrust.github.io/lulo/uninstall.sh | sh
set -eu

RMAC_SOURCES_PATH="/etc/apt/sources.list.d/rmac.sources"
RMAC_PREFERENCES_PATH="/etc/apt/preferences.d/rmac.pref"
RMAC_PACKAGES="rmac-session rmac-apps rmac-archive-keyring"

fail() {
    echo "uninstall.sh: $*" >&2
    exit 1
}

check_not_root() {
    if [ "$(id -u)" -eq 0 ]; then
        fail "run this as your normal user, not root or sudo; it calls sudo itself for the steps that need it"
    fi
    command -v sudo >/dev/null 2>&1 || fail "sudo is required but was not found"
}

# Refuse to purge rmac-session out from under a live Lulo OS session: doing
# so pulls its binaries and systemd user units out while they may still be
# running, with no fallback to GNOME in progress. Mirrors the equivalent
# install-time guard in install-native-candidate.sh.
check_not_in_rmac_session() {
    if command -v systemctl >/dev/null 2>&1 \
        && systemctl --user --quiet is-active rmac-session.target 2>/dev/null; then
        fail "log out of Lulo OS and choose Ubuntu/GNOME first, then rerun this script"
    fi
}

# `apt-get purge name1 name2` aborts before removing ANY of them if even one
# name is entirely unknown to apt (not merely "not installed") -- which
# rmac-archive-keyring always is on a machine that only ever used the
# `--from-release`/`--from-dir` install path, since that path never adds the
# rmac APT repository. Only pass apt-get the package names dpkg actually
# knows about, so removal of rmac-session/rmac-apps still succeeds.
purge_packages() {
    to_purge=""
    for package in $RMAC_PACKAGES; do
        if dpkg-query --show --showformat='${db:Status-Status}' "$package" \
            >/dev/null 2>&1; then
            to_purge="$to_purge $package"
        fi
    done
    if [ -z "$to_purge" ]; then
        return 0
    fi
    # shellcheck disable=SC2086 # word-splitting a list of bare package names is intended
    sudo apt-get purge --yes $to_purge \
        || fail "removing the rmac packages failed"
    sudo apt-get autoremove --yes || true
}

remove_repository_configuration() {
    sudo rm -f "$RMAC_SOURCES_PATH" "$RMAC_PREFERENCES_PATH"
    sudo apt-get update || true
}

main() {
    check_not_root
    check_not_in_rmac_session
    purge_packages
    remove_repository_configuration
    echo
    echo "rmac has been removed. Ubuntu/GNOME is unaffected."
}

main "$@"
