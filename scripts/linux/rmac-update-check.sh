#!/bin/sh
# rmac-update-check: a daily, read-only nudge that rmac software updates are
# available. It never installs, downloads, or removes anything -- it only
# asks PackageKit whether updates exist and, if so, shows a desktop
# notification. Reviewing and installing updates stays in System Settings
# (docs/software-update.md), which remains the only place rmac drives a
# PackageKit transaction that changes package state.
#
# Shipped by the rmac-session package as
# /usr/libexec/rmac/rmac-update-check and run once a day by the
# rmac-update-check.timer / rmac-update-check.service user units.
set -eu

main() {
    # No PackageKit CLI, no check. This is a soft nudge, not the update
    # authority, so a missing tool is not an error.
    if ! command -v pkcon >/dev/null 2>&1; then
        exit 0
    fi

    # `pkcon get-updates` prints one line per available update, each
    # embedding a PackageKit package ID (name;version;architecture;origin --
    # see docs/software-update.md). Refresh the cache first so a stale
    # metadata snapshot doesn't hide or invent updates; both steps are
    # read-only PackageKit transactions. The exact category labels pkcon
    # prints are locale- and version-dependent, so count lines that embed a
    # package ID instead of matching a label.
    pkcon refresh force >/dev/null 2>&1 || true

    update_count=$(pkcon get-updates 2>/dev/null | grep -c '.*;.*;.*;.*' || true)
    case "$update_count" in
        '' | 0)
            exit 0
            ;;
    esac

    # No notifier, no notification: still not an error, since the timer
    # should keep trying daily rather than fail permanently.
    if ! command -v notify-send >/dev/null 2>&1; then
        exit 0
    fi

    if [ "$update_count" = 1 ]; then
        body="1 update is available."
    else
        body="$update_count updates are available."
    fi

    notify-send \
        --app-name="Software Update" \
        --icon=software-update-available \
        "Updates available" \
        "$body — open System Settings to review and install them." \
        || true

    exit 0
}

main "$@"
