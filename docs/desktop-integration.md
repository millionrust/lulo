# Desktop integration

rmac applications use reverse-domain desktop identities. On Linux the same
identity must be used for the desktop filename (without `.desktop`), icon name,
AppStream component ID, and every GPUI Wayland toplevel `app_id`.

| Application | Desktop identity | Binary |
| --- | --- | --- |
| Files | `org.rmac.Files` | `rmac-files` |
| Terminal | `org.rmac.Terminal` | `rmac-terminal` |
| Notes | `org.rmac.Notes` | `rmac-notes` |
| Text Editor | `org.rmac.TextEditor` | `rmac-text-editor` |
| System Monitor | `org.rmac.SystemMonitor` | `rmac-system-monitor` |
| Applications | `org.rmac.AppDrawer` | `rmac-app-drawer` |
| System Settings | `org.rmac.SystemSettings` | `rmac-system-settings` |

Each identity now has original MIT-licensed scalable artwork under
`packaging/rmac-apps/icons`. The seven icons share a 128-unit canvas, 28-unit
outer corner, quiet solid background, and high-contrast geometric foreground.
They contain no Apple artwork, names, logos, fonts, embedded raster images,
external references, scripts, filters, or text. Their motifs are rmac-owned:
a file card, command prompt, note card, caret document, system activity trace,
application grid, and colored settings sliders.

The icon fixture gate requires the exact seven-file inventory, distinct
content, regular non-symlink files, a 16 KiB cap, the MIT notice, a fixed
scalable view box, and a small safe SVG element/attribute subset. Rendered
thumbnails still require visual review because structural validation cannot
prove optical quality.

`rmac-ui` owns these constants and creates both standard and unified-toolbar
window options with the exact identity. Files, Terminal, Text Editor,
System Monitor, Apps, and System Settings publish it today. Notes is
reserved in the same domain but still needs its running window switched before
the application metadata package can pass the complete seven-app gate.

The retired `org.rmac.Finder` identity is not installed. Shell-settings v4
migrates an existing v1, v2, or v3 Dock pin to `org.rmac.Files`, preserves its
position, and collapses only an old/new alias pair. The installed executable
names are likewise `rmac-files` and `rmac-system-monitor`; the monitor reads
its retired preference directory only as a compatibility fallback and writes
new choices to its current directory.

## File activation

Text Editor accepts zero to 32 startup document paths. Zero paths or the exact
`--new-document` action opens one untitled window. `%F`-style paths open
independent windows and retain non-Unicode platform paths. Unknown options,
empty paths, and excessive input fail before GPUI starts. A literal relative
path beginning with `-` must follow `--`.

Its desktop entry therefore advertises only the bounded plain text, Markdown,
and RTF MIME types already handled by its document codec. No other rmac app
declares a MIME association because its `Exec` launch path does not yet consume
a file or URL field code.

## Activation claims

Desktop entries must keep `DBusActivatable` absent (its specified default is
false) until an application owns the matching session-bus name and implements
the freedesktop application activation contract. A Wayland `app_id`, systemd
service, or existing internal D-Bus authority is not application D-Bus
activation.

Similarly, `StartupNotify=true` must not be added until the application proves
the startup-notification completion protocol. Stable `app_id` matching is
necessary for launcher grouping, Dock state, activation tokens, and icon
resolution, but it is not evidence for that separate protocol.

## Metadata package

`stage-application-package.py` copies the exact seven desktop entries, icons,
AppStream components, reviewed localization sources, and MIT license inventory
into an empty absolute DESTDIR. It builds in a sibling directory and publishes
atomically. It never installs into `/`, stages user data, or invents application
binaries.

The manifest records every owned `/usr/share` path, mode, and SHA-256 digest.
The independent verifier checks that exact inventory plus desktop/AppStream
identity links, executable names, actions, MIME scope, visibility, Hindi
translations, license coverage, safe XML, and the deliberate absence of
`DBusActivatable` and `StartupNotify`. Its installed-host mode additionally
requires all seven `/usr/bin` executables and executes
`desktop-file-validate` and `appstreamcli validate --no-net`.
The Linux reference runner also stages the immutable metadata into a private
temporary tree and runs those host validators before any rmac package is
installed. Validator absence or rejection fails the summarized reference gate;
this proves freedesktop metadata acceptance without claiming runtime identity.

English is the source language. The complete Hindi catalog is merged directly
into desktop names, generic names, comments, keywords, action labels, and
AppStream names/summaries. The POT template, PO catalog, language inventory,
and source inventory are staged as auditable package documentation. Runtime
application UI localization remains a separate product-wide gate.

Fixture validation does not replace the remaining H1 acceptance evidence:
Notes must publish its reserved identity, the standard validators must pass on
Ubuntu, and niri grouping, activation-token delivery, open-with behavior,
localization, scaling, and accessibility must be observed on Linux.
