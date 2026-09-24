# Accessibility authority and Linux acceptance

System Settings presents one macOS-like Accessibility pane, but every control
maps to a real Linux or rmac authority. The pane does not simulate missing
session features inside individual applications and does not treat detectable
screen-reader prerequisites as proof that an application is accessible.

The current upstream contracts are niri's
[Accessibility guide](https://niri-wm.github.io/niri/Accessibility.html),
[niri integration guidance](https://github.com/niri-wm/niri/wiki/Integrating-niri),
[niri input configuration](https://niri-wm.github.io/niri/Configuration%3A-Input.html),
and GIO's
[`Settings::changed`](https://docs.gtk.org/gio/signal.Settings.changed.html)
notification contract.

## rmac visual preferences

Increased contrast, reduced motion, and Standard/Large/Extra Large rmac text
size use the versioned `rmac-theme` preference document. The desktop Settings
portal remains a separate read-only source for automatic host scheme, accent,
contrast, and motion values. Reconnecting file and portal watchers coalesce
external changes into complete off-thread reads. Settings retains the last
known-good snapshot through watcher failure and uses generation guards so an
older refresh cannot cross a manual refresh or mutation.

A mutation captures the complete expected preference identity, reloads both
authorities, refuses a changed document, changes only the selected field, saves
atomically, and requires exact preference readback. A file event raised by the
save schedules another authoritative refresh. No cached whole-document write
may silently overwrite another editor.

The effective rmac text factor updates the shared GPUI rem base and semantic
interface typography. It deliberately does not change display/output scale,
terminal or editor content fonts, web content, or applications from another
toolkit.

## GTK application text

GTK text size is independent and uses
`org.gnome.desktop.interface text-scaling-factor` through argument-separated,
bounded `gsettings` processes. Reads distinguish a missing schema from a
policy-locked key. Writes validate the requested range, reread the exact value
and writability immediately before mutation, refuse a concurrent change, and
require exact authoritative readback. A matching request is a no-op.

A bounded `gsettings monitor` process supplies coalescible refresh hints and is
restarted after failure. Settings preserves the last known-good value, exposes
a separate live-update failure, and prevents stream results from crossing a
newer mutation generation. Command output, time, and error text are bounded;
raw stderr and private process diagnostics are never presented.

## Keyboard and pointer controls

Key-repeat presets change niri's real delay and rate together and always show
the effective pair. Pointer precision and middle-button emulation map to niri's
libinput configuration. The full Keyboard, Mouse, and Trackpad panes expose the
remaining supported niri controls and reuse their validated, atomic,
reload-confirmed transaction path.

Current niri has no session authority for Sticky Keys, Slow Keys, Bounce Keys,
Mouse Keys, dwell click, session-wide double-click timing, or per-device input
overrides. These features remain explicitly unavailable rather than becoming
local switches that work only in rmac applications.

## Orca readiness boundary

Niri provides basic Orca support only when it runs as a full desktop session,
not as a nested window or a plain compositor launched on a TTY. Orca currently
also requires Xwayland, a connected and enabled screen, and working EGL. The
pane checks the niri desktop/session environment, a non-empty `NIRI_SOCKET`, an
enabled output from direct niri state, an exported `DISPLAY`, an Orca executable
in a bounded `PATH` search, and reports whether `xwayland-satellite` is present
in that search. A non-empty `DISPLAY` does not prove that Xwayland works, and an
executable lookup does not prove the satellite version or a custom configured
path.

Environment discovery also cannot prove working EGL, speech output, focus
transfer, or rmac application roles, names, states, actions, and
announcements. Niri does not currently provide built-in desktop zoom or
screen curtain. Those controls remain absent.

## AT-SPI registration

Every rmac window registers with the AT-SPI registry as soon as the session
reports `org.a11y.Status.IsEnabled`, whether or not a screen reader is
running, the way GTK and Qt applications do. The top bar, Dock, Launcher and
every rmac app then show up under the registry root for Orca, Accerciser and
pyatspi. This needs `accesskit_unix` 0.22 or later in the vendored
`gpui_linux`; older releases waited for `ScreenReaderEnabled` and left Lulo
invisible to AT-SPI clients (ADR 0013).

## Screen Reader on/off

Accessibility > Screen Reader shows a real on/off switch, not just readiness:
the `rmac-screen-reader` crate reads and writes GNOME's standard
`org.gnome.desktop.a11y.applications screen-reader-enabled` key (the same
bounded, validated `gsettings` pattern GTK text scaling already uses) and
starts or stops Orca by reading `/proc/*/comm` and `/proc/*` ownership
directly, rather than shelling out to `pgrep`/`pkill`. `set_enabled` is the
only place in rmac that starts or stops Orca, so the switch and the ⌘F5
(Super+F5 on a PC keyboard) shortcut - both of which call it - are the only
things in the Lulo session that can make Orca start talking.

Listing rmac's own top-level `binds` block in shell.kdl replaces niri's
compiled-in defaults wholesale (niri does not merge unlisted defaults back
in), so niri's own default `Super+Alt+S -> spawn-sh "pkill orca || exec
orca"` bind - a raw, un-synced process toggle - is already inert in the Lulo
session. shell.kdl claims that combo anyway and routes it through the same
safe toggle as ⌘F5, both so it matches muscle memory from stock niri and so
the session can never grow an un-synced Orca toggle by accident. Session
start also calls `rmac_screen_reader::disable_gnome_shortcut`, which sets
GNOME's own `org.gnome.desktop.wm.keybindings toggle-screen-reader` key to an
empty binding list, in case a legacy GNOME session component still requests
that accelerator through the GlobalShortcuts portal.

## Linux acceptance matrix

F16 remains unchecked until the Ubuntu/niri reference PC proves:

- live external and in-pane rmac contrast, motion, and text changes across all
  seven apps without stale overwrite, clipping, displaced hit regions, or
  incorrect effective state at 100–200% output scale;
- GTK Standard/Large/Extra Large, custom value, policy lock, missing schema,
  monitor loss/recovery, concurrent edit refusal, and exact readback in a real
  GTK application without changing niri output scale;
- every keyboard-response and mouse-precision preset, custom values,
  middle-button emulation, touchpad support and unsupported-device states,
  config validation failure, niri reload, and external-change recovery;
- full-session versus nested/plain launch, enabled and disabled outputs,
  automatic or custom Xwayland, missing/unsupported satellite, Orca absent and
  present, broken EGL, speech output, customized shortcut, and niri restart;
- keyboard-only navigation, focus order and restoration, visible focus,
  contrast, reduced motion, 100–200% scaling, bounded idle work, and
  privacy-safe errors; and
- Orca/AT-SPI roles, names, descriptions, states, values, actions,
  announcements, dialogs, errors, and all critical journeys in every rmac app
  and shell surface.

Use `scripts/linux/run-accessibility-evidence.sh` and the manual procedure in
`docs/linux-reference-bringup.md`. Evidence output stays under the ignored
`target/linux-evidence` tree. Commit only reviewed, privacy-safe results; never
commit usernames, hostnames, private paths, environment dumps, bus peers, raw
tool diagnostics, recordings with personal content, or speech transcripts
containing private data.
