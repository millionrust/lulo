# Updates, rollback, and removal

rmac does not yet publish a supported APT repository. Do not add an unofficial
key or source, bypass APT authentication, or use repository staging tools as a
live-root installer.

## Updates

The intended release flow uses signed Ubuntu packages and PackageKit. System
Settings checks, downloads, and installs through typed PackageKit operations;
the system polkit agent owns authorization. Signature, trust, licence, media,
network, lock, space, cancellation, and backend failures remain visible and
must recover through a fresh authoritative read.

The exact repository key isolation, metadata expiry, rotation, staged rollout,
and rollback rules are in [Update trust](update-trust.md).

## Before changing versions

1. Read the [release notes](release-notes.md) and known limitations.
2. Export important user documents using their owning application or Files.
3. Confirm the stock Ubuntu/GNOME recovery session works.
4. Keep the previous signed package version available through the reviewed
   repository/snapshot process.
5. Ensure adequate free space and do not begin below the build/update floor.

## Rollback

Rollback is a package-manager operation to a reviewed signed version; it is not
copying old binaries over a live session. Sign out, use the recovery session,
install the complete matching package set, and verify the installed-host
manifest. A rollback must not reinterpret or silently downgrade persisted
data. If the older version cannot read a newer format safely, export/recovery
guidance must be supplied before release.

## Remove packages

Use the package manager once native packages are released. Uninstall removes
only immutable package-owned files under rmac-specific names. It must preserve
documents and user-owned XDG configuration, data, cache, state, recovery, and
safe-mode files.

Removing user data is a separate explicit purge/export decision. Never infer
consent to delete it from package uninstall. Confirm that the rmac GDM entry is
gone, Ubuntu/GNOME still starts, rmac units and portal descriptors are removed,
and unrelated niri/portal configuration is unchanged.

The disposable-VM lifecycle procedure in
[Ubuntu session packaging](ubuntu-session-packaging.md#upgrade-rollback-and-uninstall-ownership)
automates baseline install, candidate upgrade, interrupted rollback recovery,
remove, purge, and reinstall while proving synthetic XDG/document data and the
GNOME recovery session survive every step.

Development installs under the user's XDG directories are not system packages.
Follow [Session supervisor](session-supervisor.md) and inspect the exact
installed paths before removing them; do not use a recursive home-directory
cleanup command.
