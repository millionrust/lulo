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

## User-data export evidence

Every release candidate must complete this journey in a disposable account
before upgrade, rollback, or removal evidence is accepted. Use synthetic text,
images, filenames, and tags; never use a daily account or attach the exported
data to a public report.

1. In Notes, create one folder, two live notes, a pin, tags, and one managed
   image. Move a third note to Recently Deleted. Finish every pending save and
   confirm previews/search use the accepted library.
2. Choose **Export Notes**, select **All Notes**, review the live/deleted note
   and attachment counts plus checked byte total, and export the versioned rmac
   Notes bundle through the system file chooser. Record success only after the
   completion sheet reports verified final readback. Cancellation must also be
   exercised once and must leave no destination artifact.
3. In Text Editor, save representative plain-text, Markdown, and supported RTF
   documents into a user-chosen export folder through the portal. Close and
   reopen them from Files, including one non-ASCII document, so unsaved recovery
   state is not mistaken for an export.
4. Use Files to copy that export folder and the Notes bundle to the reviewed
   destination. Unmount removable media through Files when applicable, then
   reconnect it and reopen the ordinary documents.
5. In a distinct clean disposable profile, import the Notes bundle with the
   reviewed **Keep Both** policy. Verify the exact folder, live/deleted note,
   pin, tag, attachment, image-preview, and search results against the synthetic
   source account. The import must not overwrite pre-existing destination data.
6. Remove and purge the packages through the H5 lifecycle, reinstall the exact
   candidate, and separately prove the original account's XDG configuration,
   data, cache, state, recovery records, and safe-mode state were preserved.

The reviewed report records only the candidate revision, architecture, package
versions, portal cancellation/success, source and imported counts, checked byte
totals, document format/encoding categories, removable-media applicability,
package lifecycle result, and pass/fail. It must not contain document text,
filenames, tags, attachment bytes, paths, file hashes, account names, media
identifiers, or raw portal output. Keep the private bundle only long enough to
perform the clean-profile import, then remove both disposable accounts through
the VM reset procedure.

This is the portable user-data claim. Terminal profiles, System Monitor column
choices, shell/Focus/theme settings, notification history, recovery journals,
and recent-document state remain preserved XDG data but do not currently claim
a cross-profile import format. Network/VPN credentials remain with
NetworkManager, portal permissions remain with PermissionStore, and PAM/polkit
state is never copied into an rmac export.

## Configuration migration evidence

Package installation success does not prove that a new application can safely
read state written by an older one. For every candidate that changes a durable
format, run this matrix with real signed baseline and candidate packages in a
disposable account:

| Authority | Supported legacy input | Required candidate result |
| --- | --- | --- |
| Desktop/session settings | Shell settings versions 1, 2, and 3 | Read every preserved user choice, migrate the retired Files identity without reordering or duplicate pins, write canonical version 4 only after successful load, and reread the same complete state after service restart |
| Notes library | Binary library schema version 1 | Open as the same stable folders/notes/attachments with the version-2 edited-sort default, commit the next ordinary edit transactionally, and reopen the exact accepted version-2 library |
| Notes prototype library | The last supported pre-store folder/note layout | Offer the bounded non-destructive review, preserve every source in private recovery storage, import only after explicit acceptance, verify the migration receipt, and never offer the committed source again |
| Terminal profile | Legacy numeric built-in profile index | Select the same profile, rewrite its stable profile name on the next accepted save, and preserve that choice after restart |
| System Monitor columns | Retired `rmac-activity-monitor` preference path | Preserve the exact valid column order, write only the current `rmac-system-monitor` path on the next accepted choice, and ignore neither the required Name column nor malformed input |

Create each legacy input by running the exact baseline package that owned it;
hand-authored fixtures and unit tests do not satisfy the native migration row.
Capture the baseline version and a privacy-safe semantic inventory before the
upgrade, stop every owning process, install the complete matching candidate
package set, then exercise the real application/service entrypoint. Restart the
owner before recording candidate readback so an in-memory value cannot pass.

For each row also preserve one malformed input and one unsupported future
version. The candidate must retain the original bytes, avoid partial canonical
output, and expose its documented recovery/read-only/export path. After the
candidate writes its current format, attempt the reviewed rollback package: an
older reader must refuse or follow its documented recovery boundary rather than
silently reinterpret newer state. Reinstall the candidate and prove the last
accepted state remains recoverable.

Theme, Focus, notification history, recent documents, and other stores that
currently expose only their first accepted version receive current-version,
corruption, future-version, last-known-good, and preservation coverage; they do
not claim a legacy migration until a second format exists. niri, NetworkManager,
PermissionStore, PAM, polkit, and other Linux-owned formats remain outside rmac
migration authority.

The reviewed migration report contains only candidate/baseline package versions,
architecture, authority ID, source/current version numbers, semantic counts or
enumerated non-private choices, migration/restart/rollback/reinstall outcomes,
and pass/fail. It contains no settings bytes, document content, titles, tags,
paths, account/session/device identities, credentials, hashes of private data,
or raw logs. Pair it with locally reviewed native interaction evidence; a
canonical report or passing unit fixture alone cannot satisfy H5.

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
GNOME recovery session survive every step. That synthetic preservation proof
must be paired with the real application-owned export/import journey above; it
does not replace it.

Development installs under the user's XDG directories are not system packages.
Follow [Session supervisor](session-supervisor.md) and inspect the exact
installed paths before removing them; do not use a recursive home-directory
cleanup command.
