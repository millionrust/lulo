# Desktop integration

rmac applications use reverse-domain desktop identities. On Linux the same
identity must be used for the desktop filename (without `.desktop`), icon name,
AppStream component ID, and every GPUI Wayland toplevel `app_id`.

| Application | Desktop identity | Binary |
| --- | --- | --- |
| Finder | `org.rmac.Finder` | `rmac-finder` |
| Terminal | `org.rmac.Terminal` | `rmac-terminal` |
| Notes | `org.rmac.Notes` | `rmac-notes` |
| Text Editor | `org.rmac.TextEditor` | `rmac-text-editor` |
| Activity Monitor | `org.rmac.ActivityMonitor` | `rmac-activity-monitor` |
| Applications | `org.rmac.AppDrawer` | `rmac-app-drawer` |
| System Settings | `org.rmac.SystemSettings` | `rmac-system-settings` |

`rmac-ui` owns these constants and creates both standard and unified-toolbar
window options with the exact identity. Finder, Terminal, Text Editor,
Activity Monitor, App Drawer, and System Settings publish it today. Notes is
reserved in the same domain but still needs its running window switched before
the application metadata package can pass the complete seven-app gate.

## File activation

Text Editor accepts zero to 32 startup document paths. Zero paths or the exact
`--new-document` action opens one untitled window. `%F`-style paths open
independent windows and retain non-Unicode platform paths. Unknown options,
empty paths, and excessive input fail before GPUI starts. A literal relative
path beginning with `-` must follow `--`.

The future desktop entry may therefore truthfully advertise the bounded plain
text, Markdown, and RTF MIME types already handled by its document codec. No
other rmac app may declare a MIME association until its `Exec` launch path
actually consumes the corresponding field code.

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

The desktop, icon, AppStream, localization, and license payload remains an H1
gate. It must be validated with `desktop-file-validate`, `appstreamcli`, the
package metadata verifier, and real niri launch/group/open evidence before H1
is checked.
