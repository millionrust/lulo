# System Settings implementation audit

The product target is macOS-like organization and interaction backed by real
Linux services. A visible mutation is not complete until it changes the host
service, reports authorization/failure, and refreshes from authoritative state.
Apple-only branding and cloud services are replaced with honest rmac/Linux
equivalents rather than simulated.

| Pane | Current state | Linux authority | Required completion |
|---|---|---|---|
| Wi-Fi | Real state, radio mutation, access-point scan | NetworkManager D-Bus | Known/open connection activation, secret agent, live signals |
| Bluetooth | Real adapter, discovery, and known-device connections | BlueZ D-Bus | Confirmation agent for new-device pairing and live signals |
| Network | Real interfaces, route, IP, gateway, and DNS state | NetworkManager D-Bus | Safe connection editing and live signals |
| VPN | Real profile listing and activation/deactivation | NetworkManager VPN plugins | Import supported profiles and live signals |
| Battery | Real battery/AC state, health, and power profiles | UPower and power-profiles-daemon | Live signals and supported charge thresholds |
| General/About | Typed privacy-safe OS, kernel, architecture, hardware, graphics, and session facts; validated hostname mutation with busy/error state; authoritative refresh; redacted clipboard report; Apple-only rows removed | `rmac-system-info`, systemd-hostnamed D-Bus/polkit, os-release, procfs/sysfs, display service | Linux runtime evidence for successful/cancelled/denied polkit flows, external hostname refresh, and clipboard contents |
| Software Update | Live bounded PackageKit update status with security/blocked classification, cached startup query, explicit freshness request, timeout, service/backend errors, and last-known-good refresh behavior | PackageKit system D-Bus over the Ubuntu APT backend | Trusted download/install transaction, progress/cancel, restart requirements, polkit outcomes, live signals, and Linux interaction evidence |
| Storage | Direct `statvfs` usage for the system volume and user-visible removable/network mounts; per-volume capacity failures; authoritative refresh; low-space state and conservative cleanup guidance | `rmac-mounts`, proc mount table, `statvfs` | Live mount events, measured categories where supportable, reviewed reversible cleanup actions, and Linux scale/accessibility evidence |
| Date & Time | Real timedated timezone inventory/current zone, clock and RTC state, NTP capability/enabled/synchronized state, validated timezone and automatic-time mutations with interactive polkit, property-change/restart stream with reconnect, refresh, and last-known-good failures | `rmac-time`, `rmac-time-linux`, `org.freedesktop.timedate1` | Manual clock editing with confirmation plus Linux polkit/restart/scale/accessibility evidence |
| Language & Region | Placeholder | locale1 D-Bus and input services | Locale, formats, keyboard/input sources |
| Login Items | Placeholder | systemd user/XDG autostart | Enable/disable user startup entries |
| Sharing | Placeholder | Explicit service adapters | Capability-detected SSH/file sharing controls |
| Accessibility | Placeholder | Settings portal and accessibility stack | Contrast, motion, text scale, Orca-facing controls |
| Appearance | Real scheme, accent, contrast, and motion preferences with host-following automatic modes, atomic persistence, recovery, refresh, and live adoption across all seven apps | Settings portal plus `rmac-theme` | Linux visual, scaling, contrast, motion, and Orca evidence |
| Assistant & Intelligence | Hidden from production navigation | Optional local/provider integrations | Leave absent until a privacy design exists |
| Desktop & Dock | Placeholder | rmac shell and niri IPC | Dock, desktop, workspaces, window behavior |
| Displays | Real layout plus transient mode, scale, and rotation controls | niri output IPC/Wayland | Persistent validated layouts, visual positioning, and live signals |
| Spotlight | Placeholder | rmac-search | Sources, exclusions, indexing state, global shortcut |
| Wallpaper | Placeholder | rmac shell | Per-output wallpaper selection and persistence |
| Notifications | Live service-backed app/policy list with desktop-entry names/icons plus allow/block, top-bar badge, and history controls with separate stream/mutation errors | `org.rmac.NotificationCenter1` plus live XDG app catalog | Banner/sound UI after presentation exists, per-app lock-preview controls after a PAM-capable provider exists, scoped GPUI build, and Linux interaction/accessibility evidence |
| Sound | Real devices, defaults, volume, and mute; local-only alert/startup/UI-sound controls removed | PipeWire/WirePlumber | Live signals, routes, authoritative session sound policy, and per-channel balance |
| Keyboard, Mouse & Trackpad | Real persistent input configuration with validation and atomic rollback | niri/libinput | Included-config editing, per-device overrides, and live signals |
| Focus | Live service-backed state/configuration, desktop-entry names/icons, manual mode/duration activation, urgent and per-app allow-list policy, plus create/edit/enable/delete schedule controls | `org.rmac.Focus1`, `org.rmac.NotificationCenter1`, and live XDG app catalog | Scoped GPUI build and Linux/niri interaction/accessibility evidence |
| Screen Time | Hidden from production navigation | No service selected | Usage model only after a local-first privacy design |
| Lock Screen | Real live lock and capability-gated automatic-suspend choices; truthful Hidden preview state; secure manual/logind/pre-sleep and idle paths | `org.rmac.LockScreen1`, provider security state machine, bounded action-free notification projection, logind `CanSuspend`/`Suspend(false)`, niri `ext-session-lock-v1`, PAM-enabled swaylock, and delay inhibitor | Reviewed Wayland/PAM adapter and rmac presentation, preview controls, scoped build, and Linux security/accessibility evidence |
| Privacy & Security | Placeholder | Portals, polkit, package security sources | Permission visibility and supported security controls |

## Delivery order

1. Connectivity: Wi-Fi, Bluetooth, Network, VPN.
2. Daily hardware: Sound, Battery, Displays, keyboard/mouse/trackpad.
3. Shell ownership: Appearance, Desktop & Dock, Wallpaper, Notifications,
   Focus, Lock Screen, Spotlight.
4. Host administration: About, updates, storage, date/time, locale, users,
   login items, sharing, privacy/security.
5. Optional services only after local-first privacy and maintenance plans exist.

Every pane keeps slow I/O off the first-frame/UI thread, consumes typed service
snapshots, and must not persist a local toggle as a substitute for system state.

General now exposes only About, the explicitly read-only Software Update status,
and measured Storage. Device-continuity and media-receiver controls are hidden
until reviewed Linux service authorities exist. System Settings no longer reads
or writes its obsolete private `settings.json`; NetworkManager, BlueZ, PipeWire,
and the other typed services seed their own authoritative state. A stale file
from an older installation is ignored and can be removed by packaging or
uninstall cleanup. AppleCare, AutoFill, Startup Disk, and Time Machine rows are
absent rather than mapped to generic clickable placeholders.

About delegates platform identity to `rmac-system-info`; the GPUI view performs
no host reads or D-Bus work. On Linux, static-hostname changes call
`org.freedesktop.hostname1.SetStaticHostname` with interactive authorization,
then replace the UI snapshot only with the service response. Invalid,
unavailable, denied, and failed mutations preserve the last known-good facts.
The copied report deliberately omits hostname, username, serial numbers,
addresses, machine IDs, and paths. This follows the hostname service contract
documented by the official systemd
[`org.freedesktop.hostname1` manual](https://www.freedesktop.org/software/systemd/man/latest/org.freedesktop.hostname1.html).
Linux reference-PC interaction evidence remains pending.

Software Update now queries PackageKit directly instead of parsing `apt` output.
The adapter creates a transaction, subscribes before starting it, sends bounded
non-interactive cache-age hints, collects typed package/error/completion signals,
and cancels after 45 seconds. Package identifiers and backend text are bounded
before entering UI state. A failed refresh preserves the last successful list;
an absent daemon is an explicit unavailable state. The current slice is status
only: downloads, installation, restart handling, and authorization are still
unavailable, and the pane directs people to Ubuntu Software Updater. This
matches Ubuntu 26.04's documented updater path and PackageKit's official D-Bus
transaction contract; applying changes will not be claimed until it has its own
confirmation, polkit, progress, cancellation, and recovery slice.

Authority references: [Ubuntu 26.04 Software Updater guidance](https://documentation.ubuntu.com/desktop/en/26.04/tutorial/install-ubuntu-desktop/)
and the [PackageKit transaction API](https://packagekit.freedesktop.org/gtk-doc/Transaction.html).

Storage no longer parses `df` or invents a `Macintosh HD` label on Linux.
`rmac-mounts` returns the system volume plus user-visible removable and network
mounts, then measures each independently with `statvfs`. One inaccessible or
disconnected volume reports its own failure without hiding healthy volumes.
The pane flags low space, refreshes off the UI thread, and offers conservative
guidance. It does not fabricate storage categories or expose cleanup buttons
until category measurement and reversible deletion plans exist.

Date & Time reads typed properties and the bounded `ListTimezones()` inventory
from systemd-timedated. Time-zone and automatic-time changes validate locally,
request interactive polkit authorization, and replace UI state only with a
fresh service snapshot. Unsupported NTP, unavailable service, invalid zone,
denied authorization, and mutation failure preserve the last known-good state.
The hardware clock is deliberately read-only and identifies UTC as the normal
Linux configuration. A bounded event stream refreshes from authority on
timedated property changes and daemon reappearance, reconnecting after bus
failure without treating the daemon's normal idle exit as an error. Manual
clock editing remains pending and is stated in the pane. Authority reference:
[Ubuntu 26.04 `org.freedesktop.timedate1(5)`](https://manpages.ubuntu.com/manpages/resolute/man5/org.freedesktop.timedate1.5.html).

Assistant & Intelligence and Screen Time are absent from sidebar and search
navigation because neither has an accepted local-first privacy/service design.
Required roadmap destinations that do not yet have a dedicated renderer show
one explicit unavailable explanation; the old generic rows and clickable
placeholder subpages have been removed.

Sound now renders only controls backed by the system audio service. The former
alert-sound picker, alert volume, startup sound, UI-effect toggles, and unused
balance state were local persistence pretending to be session policy; they are
hidden until an authoritative rmac sound-policy service exists. The obsolete
private settings file containing those keys is ignored rather than migrated.
