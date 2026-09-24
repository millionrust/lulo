#!/bin/sh
# Remove rmac (Lulo OS) and its repository configuration from this machine.
# Ubuntu/GNOME is never touched: this only purges the three rmac packages
# and the apt configuration install.sh wrote.
#
# Usage:
#   curl -fsSL https://millionrust.github.io/lulo/uninstall.sh | sh
set -eu

RMAC_SOURCES_PATH="/etc/apt/sources.list.d/rmac.sources"
RMAC_PREFERENCES_PATH="/etc/apt/preferences.d/rmac.pref"

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

purge_packages() {
    sudo apt-get purge --yes rmac-session rmac-apps rmac-archive-keyring \
        || fail "removing the rmac packages failed"
    sudo apt-get autoremove --yes || true
}

remove_repository_configuration() {
    sudo rm -f "$RMAC_SOURCES_PATH" "$RMAC_PREFERENCES_PATH"
    sudo apt-get update || true
}

main() {
    check_not_root
    purge_packages
    remove_repository_configuration
    echo
    echo "rmac has been removed. Ubuntu/GNOME is unaffected."
}

main "$@"
