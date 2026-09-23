# ADR 0012 — rmac's Open/Save panel is the portal FileChooser backend

- **Status:** accepted 2026-09-23.
- **Scope:** `crates/rmac-file-chooser` (library + `rmac-file-chooser` binary),
  `crates/finder/src/{listing,places}.rs`, `rmac-file-chooser.service`, the
  `rmac-file-chooser.portal` descriptor, `rmac-portals.conf`, and the session package.
- **Reference:** `design-lab/file-chooser.html` (measured from TextEdit ⌘O/⌘S on the
  owner's Mac, macOS 26.2, dark mode).

## The question

Every app on the rmac PC (Firefox, GTK, Qt, Electron, Flatpak, and rmac's own apps
through `rmac-portal`) asks `org.freedesktop.portal.FileChooser` for Open and Save
dialogs. `rmac-portals.conf` said `default=gnome;gtk;*`, so every one of them got
GNOME's file chooser — the most visible "this is Linux" moment on the desktop. How does
rmac show its own Mac-shaped panel for all of them, safely?

## Decision

1. **One backend, every app.** rmac implements
   `org.freedesktop.impl.portal.FileChooser` (`OpenFile`, `SaveFile`, `SaveFiles`, plus
   `org.freedesktop.impl.portal.Request.Close`) in a new process, `rmac-file-chooser`.
   It decodes every option the spec defines: `accept_label`, `modal`, `multiple`,
   `directory`, `filters`, `current_filter`, `choices`, `current_name`,
   `current_folder`, `current_file`, and `files`, and answers `uris`, `choices`, and
   `current_filter`. MIME filters are resolved against shared-mime-info
   (`globs2` + `subclasses`), so `text/plain` also admits its subtypes and
   `image/*` admits the whole family.

2. **Its own bus name and portal descriptor.** The notification service already owns
   `org.freedesktop.impl.portal.desktop.rmac`, and a D-Bus name has one owner. Putting
   FileChooser in `rmac.portal` would force the notification daemon to host a GPUI
   window process (or crash both together). The panel therefore owns
   `org.freedesktop.impl.portal.desktop.rmac.filechooser` and ships
   `rmac-file-chooser.portal`, selected in `rmac-portals.conf` as
   `org.freedesktop.impl.portal.FileChooser=rmac-file-chooser;gnome;gtk` — the same
   pattern GNOME uses for `gnome-keyring.portal`, and the same split ADR 0003 made for
   the wallpaper backend. GNOME remains the fallback if the binary is missing.

3. **The panel.** A GPUI 0.2 window per request, drawn at the measured macOS 26
   geometry (`rmac_file_chooser::metrics`): inset sidebar pane (Recents, Shared,
   Favourites, Locations — the same places Files shows, from
   `rmac_finder::places`), the toolbar row (back/forward, view, sort, Where pop-up,
   search), icon and list views listed with `rmac_finder::listing`, the compact Save
   sheet (Save As, File Format when the app offers more than one filter, one row per
   app choice, Where + disclosure) and its expanded form, and the Go to Folder sheet.
   Keyboard: ⇧⌘G, ⇧⌘D, ⇧⌘H, ⌘↑, ⌘↓, ⌘[ ⌘], ⌘1/⌘2, ⌘F, ⇧⌘N, ⇧⌘., arrows,
   type-to-select, Return, Esc.

## Parent window handling

The portal passes `parent_window` as `wayland:<xdg-foreign handle>` or `x11:<XID>`.
The proper Wayland behaviour is to import the handle with `zxdg_importer_v2` and call
`set_parent_of`, making the panel transient for (and modal over) the app window.
GPUI 0.2 exposes neither a foreign-toplevel import nor an X11 transient hint, so:

- the handle is parsed and bounded (`rmac_file_chooser::parent`) and kept on the request;
- the panel opens as `WindowKind::Dialog`, which GPUI maps to an `xdg_dialog_v1`
  **modal** toplevel; niri floats it centred on the active output above the app;
- the Save panel uses app id `org.rmac.FileChooser.Save` so the session's niri rule
  clips it at the measured sheet radius (24); Open keeps the 16 pt window radius.

What differs from the Mac: the panel is not attached to its parent as a sheet and the
parent is not blocked. When GPUI gains an xdg-foreign import hook this becomes a
`set_parent_of` call in `open_panel`; nothing else changes.

## Security: only what the user chose

- Only the current owner of `org.freedesktop.portal.Desktop` may call the backend
  (sender checked against `GetNameOwner`), exactly like the notification and wallpaper
  backends. The frontend, not rmac, turns returned URIs into document-portal grants.
- Every wire value is bounded and validated (`request.rs`): text ≤ 1 KiB without NULs,
  ≤ 64 filters, ≤ 16 choices, absolute `ay` paths ≤ 4 KiB, SaveFiles names reduced to
  one valid component. Invalid requests are rejected with `InvalidArgs`.
- Results are produced only by the panel from the user's own actions — a click,
  Return, a typed Save As name, or a typed Go to Folder path — and are re-checked at
  return time (`outcome.rs`): Open paths must be plain absolute paths that still exist
  with the requested kind; a Save target is exactly one valid name inside an existing
  folder (an existing file asks “already exists. Do you want to replace it?” first);
  SaveFiles names are numbered (“name 2.ext”) instead of overwriting. Nothing the app
  suggested (`current_file`, `current_folder`) is ever returned unless the user accepts
  it in the panel.
- Close(), a compositor close, or a crash answers Cancel (response 1); no partial result
  is ever sent. At most eight panels may be live at once.

## Packaging

- **Activation, not residency.** `rmac-file-chooser.service` (`Type=dbus`, bus name as
  above, `NoNewPrivileges=yes`, deliberately **no** `PrivateTmp` so /tmp is the real
  one) is started by D-Bus activation from
  `org.freedesktop.impl.portal.desktop.rmac.filechooser.service` on the first request.
  It is not wanted by `rmac-session.target` and not in the supervisor's
  `COMPONENT_UNITS`, so a panel crash never counts toward safe mode. Once running it
  stays resident, keeping later panels instant.
- **Package contract.** `rmac-session` ships the binary in `/usr/libexec/rmac`, the unit,
  `portals/rmac-file-chooser.portal`, and the activation file. `SESSION_BINARIES`
  grows from 21 to **22** and `ALL_BINARIES` from 27 to **28**
  (`scripts/test_native_packages.py`); `verify-session-package.py`,
  `stage-session-package.py`, `install-session-units.sh`, `build-native-inputs.sh`,
  and `archive-development-install.py` list the new files.
- **After install**, `systemctl --user restart xdg-desktop-portal` picks up the new
  selection; `rmac-file-chooser --preview open|folder|save|save-files` opens a panel
  without a bus for visual checks.

## Not done (and why)

- **Tags** row and sidebar section: Linux has no tag store rmac can write.
- **Column and Gallery views**: the panel ships Icons and List; Columns needs the Files
  column browser extracted from the Files binary first.
- **Thumbnails and the image-dimensions line** under icons: not wired yet.
- **New Folder name sheet**: New Folder creates “untitled folder” (numbered) and opens it.
- **Pop-up menus are drawn inside the panel window**, so a long Where menu in the
  compact Save sheet scrolls instead of extending past the window.
- **Light mode colours** are the shared rmac tokens, not measured panel values.
