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
| Language & Region | Real locale1 assignments and installed-locale inventory; validated LANG mutation preserving every LC_* override; deterministic native date/time, number, and currency examples; validated multi-layout XKB source/variant editing when niri follows localed; exact locale and keyboard rollback; property-change/restart stream with reconnect; authoritative refresh; explicit sign-out requirement | `rmac-locale`, `rmac-locale-linux`, `rmac-input`, POSIX locale objects, `org.freedesktop.locale1`, `locale -a`, `localectl` layout inventory | Included niri config traversal and Linux polkit/restart/scale/accessibility evidence |
| Login Items | Effective bounded XDG autostart enumeration with config-directory precedence, desktop-session applicability, atomic `Hidden` overrides, malformed-entry visibility, portal-selected validated add/replace, Trash-backed user-entry removal, and portal-backed reveal; bounded systemd user unit-file inventory with persistent enable/disable, protected rmac infrastructure, explicit runtime/masked/static states, and resolvable-file reveal; filtered filesystem plus user-manager signal stream with restart/reconnect; authoritative refresh and partial-authority failures | `rmac-login-items`, `rmac-login-items-linux`, `rmac-portal`, XDG specifications, freedesktop Trash, `org.freedesktop.systemd1` user manager | Linux interaction/scale/accessibility evidence |
| Sharing | Capability-detected OpenSSH and Samba services with separate runtime/boot state, explicit enable/disable confirmation, systemd system-manager/polkit mutation, bounded completion wait and exact rollback; effective bounded Samba share names from `testparm -s`; separate read-only UFW SSH/Samba allowance truth; live systemd, UFW, and Samba configuration refresh; no AirDrop branding | `rmac-sharing`, `rmac-sharing-linux`, `org.freedesktop.systemd1`, `ssh.service`, `smbd.service`, Samba `testparm`, UFW status/configuration | Linux polkit/network/scale/accessibility evidence and reviewed share editing if added |
| Accessibility | Live rmac increased-contrast, reduced-motion, and bounded application text-size preferences with effective-state display; text size updates the shared GPUI rem base and migrated semantic/shared UI text without changing display or content-font scaling; separate confirmed GNOME/GTK text scaling; authoritative niri-backed keyboard, mouse, and trackpad controls; off-thread full-niri-session, Xwayland, and installed-Orca readiness with the documented default shortcut; explicit rmac AT-SPI limits | `rmac-theme`, `rmac-ui`, `rmac-gtk-settings`, GPUI per-window rem size, GNOME interface GSettings, Settings portal appearance values, `rmac-input`, niri configuration and accessibility bridge | Linux text/output clipping, keyboard, pointer, and AT-SPI/Orca evidence |
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
| Privacy & Security | Confirmed XDG PermissionStore camera/microphone decisions with raw tokens and version-gated reset; PackageKit security-update summary; Ubuntu Pro Client package-origin, contract/service, and unattended-upgrades authorities with partial-failure visibility; explicit native/active-access and non-APT limits | `rmac-privacy`, `rmac-privacy-linux`, `org.freedesktop.impl.portal.PermissionStore`, PackageKit, Ubuntu Pro Client offline API | Standard-release support-period authority, non-APT application source inventory, supported automatic-update mutation, live PermissionStore changes, and Linux revoke/access evidence |

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

Language & Region reads locale assignments and default keyboard metadata from
systemd-localed, and obtains a bounded installed-locale inventory from the
argument-separated standard `locale -a` command. The domain service validates
locale syntax and installation before any mutation. Changing the language
replaces only `LANG` in a full assignment preview, preserves every existing
`LC_*` format override, requests interactive polkit authorization, and accepts
only the fresh post-mutation service snapshot. The previous exact assignment
set remains available for one-step rollback after success; failed, denied, and
unavailable mutations preserve the last known-good snapshot. A bounded event
stream refreshes from authority on localed property changes and daemon
reappearance, reconnecting after bus failure without treating the daemon's
normal idle exit as an error. The pane states
that the current session must sign out and back in, because running processes
do not adopt the new environment. Deterministic examples use independent POSIX
locale objects for `LC_TIME`, `LC_NUMERIC`, and `LC_MONETARY`; this avoids
changing process-global locale state and makes each retained override visible.
If native preview construction fails, the authoritative assignment remains
visible with an explicit preview error. Input sources accept one to four
installed comma-separated XKB layouts plus aligned variants and validated XKB
switching options. The adapter preserves the existing XKB model, calls localed's
`SetX11Keyboard` with console conversion disabled and interactive authorization,
then accepts only the refreshed service snapshot. Exact keyboard state remains
available for one-step rollback. `rmac-input` proves whether niri has no explicit
XKB block before enabling the editor: current niri follows localed in that case.
An explicit XKB block or an include graph keeps the system default read-only so
rmac does not fight an unproven config authority. Included-config traversal and
Linux interaction evidence remain pending. Authority
references: [Ubuntu 26.04 `org.freedesktop.locale1(5)`](https://manpages.ubuntu.com/manpages/resolute/man5/org.freedesktop.locale1.5.html),
the [official niri integration contract](https://github.com/YaLTeR/niri/wiki/Integrating-niri),
Linux [`nl_langinfo_l(3)`](https://man7.org/linux/man-pages/man3/nl_langinfo.3.html),
[`strftime_l(3)`](https://man7.org/linux/man-pages/man3/strftime.3.html), and
[`strfmon_l(3)`](https://man7.org/linux/man-pages/man3/strfmon.3.html).

Login Items currently owns the XDG application-autostart half of F14. It scans
`$XDG_CONFIG_HOME/autostart` before each `$XDG_CONFIG_DIRS/autostart`, so a
higher-priority filename always hides lower copies exactly as specified. Valid
entries expose their effective `Hidden` state and whether `OnlyShowIn` or
`NotShowIn` excludes the current desktop, including unavailable `TryExec`
requirements. Malformed or unreadable higher-priority entries remain visible as issues and still suppress lower
copies; rmac never silently runs or rewrites them. Disabling a user entry
atomically preserves its contents while changing `Hidden`. Disabling a system
entry creates a full user copy marked as an rmac-managed hidden override;
re-enabling removes only that marked override and reveals the original system
entry. Every mutation re-enumerates authority, and failures retain the last
known-good snapshot. Reveal resolves the effective entry again immediately
before passing its path to the desktop portal. Adding starts with the portal
file chooser restricted to local `.desktop` files, validates the chosen entry,
and shows an inline Add or Replace confirmation. A target that appears after
preview is never overwritten without a new replacement confirmation. The
installed copy is normalized to enabled state. Remove is offered only for
user-owned, non-managed entries and requires a second confirmation before the
file moves to the desktop Trash. If removing a user override reveals a lower
system entry, rmac immediately creates a managed disabled override rather than
silently starting that system item at the next login. Applications themselves
and systemd unit files are never deleted from this pane.
Authority references: the freedesktop.org
[Desktop Application Autostart Specification](https://specifications.freedesktop.org/autostart/0.5/),
[Desktop Entry Specification](https://specifications.freedesktop.org/desktop-entry/latest-single/),
the [XDG Base Directory Specification](https://specifications.freedesktop.org/basedir/),
and the [freedesktop Trash Specification](https://specifications.freedesktop.org/trash/latest/).

The same Login Items snapshot now includes systemd user services from the
session-bus `org.freedesktop.systemd1.Manager.ListUnitFiles()` authority. It
shows enabled/linked services plus user-installed disabled, masked, runtime,
and static units without dumping every inactive distribution unit. Only
persistent enabled, linked, or disabled states expose a toggle. Runtime-only,
masked, static, generated, and unknown states remain read-only with their exact
reason, and `rmac-*` services are protected because disabling shell
infrastructure from inside the shell is not a recoverable Login Items action.
Mutations call `EnableUnitFiles()` or `DisableUnitFiles()` on the user manager,
reload it, and require a refreshed unit-file state to confirm success. They do
not start or stop the currently running service; the pane states that the
change applies at the next sign-in. A missing user manager degrades only the
background-service section, leaving XDG application autostart usable.
When systemd exposes an absolute unit path, or the unit can be resolved through
its authoritative `UnitPath` search order, the pane offers the same portal-backed
reveal action. Stale IDs and files that disappear before activation fail visibly
instead of opening a guessed location.
One capacity-one stream combines filtered events from the effective XDG
autostart and user-unit directories with systemd's `UnitFilesChanged` signal.
Manager reappearance triggers a refresh, session-bus loss reports a separate
stream error, and the watcher reconnects without allowing event bursts to grow
memory. Every event causes a complete off-thread authority resample; event
payloads never mutate UI state directly.
Authority reference: [Ubuntu 26.04 `org.freedesktop.systemd1(5)`](https://manpages.ubuntu.com/manpages/resolute/en/man5/org.freedesktop.systemd1.5.html).

Sharing now exposes Remote Login only when the installed system unit inventory
contains Ubuntu's canonical `ssh.service` (or its `sshd.service` alias). Runtime
state and persistent boot enablement are read separately, and a toggle requires
an explicit warning confirmation before systemd's system manager requests
polkit authorization. Enabling creates persistent unit links, reloads the
manager, starts SSH, and rolls enablement back if start submission fails.
Disabling stops SSH first, then disables it; a failed disable attempts to
restart the service. Any submission or bounded convergence failure attempts to
restore the exact previous runtime and boot combination. The adapter waits for
both authorities to confirm the requested state before replacing the UI
snapshot. It never edits SSH authentication configuration.

Firewall reachability is a separate read-only fact. A fixed, argument-separated
`ufw status` query reports an explicit OpenSSH/TCP 22 allow rule, inactive UFW,
active-but-unverified policy, or unavailable/authorization-required inspection.
The pane does not change firewall rules and states that daemon activity cannot
prove reachability through host, network, or router firewalls. File Sharing now
reads Ubuntu's canonical `smbd.service` runtime and boot state independently.
It runs Samba's fixed, argument-separated `testparm -s` validator off the UI
thread and displays only bounded effective share names, excluding global and
printer-only sections; share paths and credentials are never exposed. Invalid,
oversized, unavailable, or non-UTF-8 configuration output is reported without
inventing state. A separate UFW result recognizes only the complete named Samba
application profile, rather than claiming that one manually opened SMB port is
sufficient. The File Sharing switch has the same explicit warning, polkit-backed
systemd submission, bounded convergence, and exact previous runtime/boot rollback
contract as Remote Login. Its warning makes clear that every accepted effective
share may become reachable, while the operation changes only `smbd.service`: it
never edits share definitions, paths, file permissions, credentials, or
firewall policy. AirDrop is not presented because Linux has no compatible local authority.
Authority references: Ubuntu's
[OpenSSH server guidance](https://documentation.ubuntu.com/server/how-to/security/openssh-server/),
[firewall guidance](https://documentation.ubuntu.com/server/how-to/security/firewalls/),
[Samba file-server guidance](https://documentation.ubuntu.com/server/how-to/samba/file-server/),
[Samba's `testparm(1)` reference](https://www.samba.org/samba/samba/docs/man/manpages/testparm.1.html),
and [`org.freedesktop.systemd1(5)`](https://manpages.ubuntu.com/manpages/resolute/en/man5/org.freedesktop.systemd1.5.html).

A capacity-one Sharing stream follows systemd unit properties, unit-file
changes, and manager reappearance on the system bus, while a filtered native
watch covers UFW policy/profile files and Samba configuration snippets. Bus loss and watcher failures surface as
a separate live-update error, and the systemd stream reconnects after failure.
Every event schedules a complete off-thread systemd/UFW/Samba resample; signal and
filesystem payloads never directly mutate presentation state.

Accessibility now has a dedicated pane instead of the generic unavailable
renderer. Increased contrast and reduced motion reuse the same versioned,
atomic, live-watched rmac theme authority as Appearance, and the pane shows the
resolved effective values rather than only the selected preference. Motor
shortcuts navigate directly to the existing niri-backed Keyboard, Mouse, and
Trackpad controls. A bounded Standard/Large/Extra Large preference now persists
in the same backward-compatible versioned theme document. The shared UI runtime
applies its factor to each GPUI window's rem base on open and on live changes;
semantic typography tokens and shared rmac components use the same factor.
This is deliberately labeled rmac application text size: display scale,
terminal/editor content fonts, GTK, browsers, and other toolkit applications
remain separate. All System Settings-owned labels, values, section headings,
buttons, segmented controls, sidebar text, and pane chrome now use the live
factor; fixed-height segmented/accent controls retain sufficient logical height
at the bounded 130% maximum. Every app-owned label and chrome surface in
Activity Monitor, Finder, App Drawer, Notes, Terminal, and Text Editor follows
the live factor, while independently controlled editor and terminal content
fonts remain separate. Linux clipping evidence is still required.
GTK Application Text is a separate Standard/Large/Extra Large control backed by
`org.gnome.desktop.interface text-scaling-factor`. It distinguishes missing
GSettings/schema support from a policy-locked key, validates requested factors,
and reports success only after an authoritative readback matches. Custom GTK
factors remain visible without falsely selecting a preset. This changes GTK
application text without claiming to control browsers, document fonts, niri
output scale, or rmac's GPUI typography.
The Motor section also exposes three atomic keyboard-response presets backed by
niri's real repeat delay and rate. It always shows the exact effective values,
leaves custom combinations visibly unselected, validates the complete candidate
configuration before replacement, and relies on niri live reload. Sticky Keys,
Slow Keys, and Bounce Keys remain explicitly unavailable because niri currently
provides no compositor authority for them. Authority reference:
[niri Input](https://github.com/niri-wm/niri/wiki/Configuration%3A-Input).
The same Motor section provides atomic mouse precision presets and real
libinput middle-button emulation, with exact effective acceleration and speed.
Mouse Keys, dwell click, and session-wide double-click timing remain explicitly
unavailable because niri exposes no authority for them. Mouse and Trackpad also
show middle-emulation controls beside their existing niri-backed speed,
acceleration, handedness, scrolling, typing suppression, and drag-lock controls.
The pane also discovers screen-reader readiness off the UI thread: a full niri
desktop session, non-empty Xwayland `DISPLAY`, and an Orca
executable found within a bounded PATH search are reported independently. It
shows niri's documented default `Super`–`Alt`–`S` shortcut but does not claim
that a user-customized binding exists. An explicit off-thread refresh resamples
all three prerequisites after session or package changes. This readiness
describes niri and Orca, not rmac: application roles, names, states, actions,
focus, and announcements
remain gated on Linux AT-SPI/Orca runtime evidence. Authority reference:
[niri Accessibility](https://github.com/niri-wm/niri/wiki/Accessibility).

Privacy & Security no longer uses the generic unavailable renderer. Its first
permission slice reads only the PermissionStore `devices` table's `camera` and
`microphone` resources, preserving the store's uninterpreted permission strings
instead of translating them into invented policy. Missing resources are empty
states, store/interface failures remain visible, and interface version 1 stays
read-only. Version 2 decisions can be reset per application only after explicit
confirmation through `DeletePermission`; completion is followed by a complete
authoritative resample. Reset is described as removing a stored decision so the
next portal request may ask again, never as terminating active capture or
revoking native-application access. The pane also reuses the cached PackageKit
security-update count and opens the full Software Update destination, while
keeping Ubuntu security coverage and repository trust explicitly separate.
Authority reference: [XDG PermissionStore](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.impl.portal.PermissionStore.html).

The Ubuntu security subsection now reads three independent, offline Ubuntu Pro
Client APIs rather than treating the PackageKit update count as coverage. The
package summary reports installed APT origins; attachment validity and enabled
services report Ubuntu Pro state; and the unattended-upgrades status reports
the timer, periodic job, frequency, allowed origins, disabled reason, and last
run. Each endpoint can fail without hiding successful authorities. “Enabled”
requires the unattended-upgrades service, APT timer, periodic job, and a
non-zero upgrade interval. The pane does not infer repository trust, standard
release support dates, or Flatpak/Snap/AppImage/manual-install status from these
values. Authority references: [Ubuntu Pro Client API](https://documentation.ubuntu.com/pro-client/en/docs/references/api/)
and [unattended-upgrade](https://manpages.ubuntu.com/manpages/noble/man8/unattended-upgrade.8.html).

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
