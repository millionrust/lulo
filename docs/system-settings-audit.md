# System Settings implementation audit

The product target is macOS-like organization and interaction backed by real
Linux services. A visible mutation is not complete until it changes the host
service, reports authorization/failure, and refreshes from authoritative state.
Apple-only branding and cloud services are replaced with honest rmac/Linux
equivalents rather than simulated.

## Application module boundaries

- `main.rs` is a 15-line binary composition boundary that declares the focused
  authorities and starts the application.
- `controller.rs` owns the remaining GPUI controller, pane orchestration,
  service subscriptions, and rendering. Its explicit 19,428-line size keeps the
  required controller/update/render splits visible rather than hiding them in
  the entrypoint.
- `shell_settings.rs` owns Dock, Wallpaper, and Spotlight mutations over the
  complete versioned shell-settings document; bounded watching, atomic
  persistence and authoritative readback; wallpaper inheritance, validation,
  and fixed-size preview rendering; and canonical Spotlight exclusion
  validation. The GPUI controller consumes typed snapshots and mutations
  without duplicating their storage rules.
- `service_updates.rs` owns the common generation/loading/mutation admission
  rule shared by every live service snapshot, the exact condition that requires
  an independent recovery read, and capability-aware audio choice admission.
  Pane-specific callbacks retain only their typed snapshot and follow-up work.
- `appearance.rs` owns the typed preference changes and option catalog, host
  appearance plus rmac preference loading, conflict refusal, scoped mutation,
  authoritative save/readback, and custom accent conversion. The controller
  retains only subscription lifecycle, mutation generation, and GPUI rendering.
- `displays.rs` owns typed temporary layout changes, the bounded confirmation
  state, checked relative-placement math, and compositor refresh admission.
  The controller retains transaction timing, persistence/rollback orchestration,
  and the visual arrangement editor.
- `notifications.rs` owns typed, field-scoped application policy changes and
  their preservation contract. The controller retains service mutation,
  authoritative reload, application identity, and pane rendering.
- `focus.rs` owns the combined configuration/runtime load, current-activation
  routing, and canonical schedule-day catalog. The controller retains service
  lifecycle, optimistic configuration replacement, rollback, and editors.
- `input.rs` owns typed keyboard, mouse, and trackpad changes; scoped mutation
  of the complete input document; preset catalogs and speed normalization; and
  niri configuration-event admission. The controller retains mutation
  generation, authoritative save/readback, and GPUI rendering.
- `sound.rs` owns typed volume, mute, default-device, profile, route, and
  balance dispatch plus capability-aware choice admission. The controller
  retains slider coalescing, mutation generation, live recovery, and rendering.
- `power.rs` owns power-profile application, charge-threshold recovery,
  duration/threshold/degradation descriptions, and bounded history sampling.
  The controller retains capability admission, mutation generations, live
  refresh, and the GPUI history chart.
- `system_environment.rs` owns off-thread initial account/system/storage
  collection, bounded executable discovery, and niri/Xwayland/Orca readiness
  policy. The controller retains refresh generations, error presentation, and
  the About and Accessibility panes.

| Pane | Current state | Linux authority | Required completion |
|---|---|---|---|
| Wi-Fi | Real state, radio mutation, access-point scan, typed security, exact saved/open/Enhanced Open activation, masked WPA Personal/SAE password sheet, certificate-verified PEAP/MSCHAPv2 enterprise sheet with optional anonymous identity, one-shot profile-UUID-bound Secret Agent, exact failed-join cleanup plus recovery readback, cancellation, complete Known Networks inventory, confirmed exact-profile forgetting with active disconnect, coalesced live signals, owner-loss/restart recovery, bounded completion, and authoritative readback | NetworkManager D-Bus | Linux signal/restart/permission/cancellation/wrong-secret cleanup/partial-delete and certificate-verified enterprise evidence; reviewed private-CA/certificate/smart-card/other-EAP designs if added |
| Bluetooth | Real adapter and discovery state through shared keyboard-operable radio, discoverability, and scan controls with truthful loading/scanning state; exact-device connect/disconnect; one-transaction KeyboardDisplay agent for numeric comparison, PIN/passkey entry, just-works and service authorization; explicit reject/cancel and 60-second prompt timeout; post-pair trust plus authoritative `Paired`/`Trusted` verification; confirmed adapter-owned removal; coalesced live signals and BlueZ owner-loss/restart recovery | BlueZ ObjectManager, AgentManager1, Agent1, Device1, and Adapter1 D-Bus APIs | Ubuntu pairing matrix, timeout/rejection/cancel, remove/partial-failure, daemon-restart, keyboard, scale, and accessibility evidence |
| Network | Real interfaces, route, IP, gateway, and DNS state with a shared keyboard-operable refresh action; active-profile IPv4/IPv6/DNS/PAC editor with strict typed validation, exact opaque identity, stable full-map reads, version-checked in-memory staging and disk persistence, applied-state verification, ownership-aware rollback, coalesced live signals, and owner-loss/restart recovery | NetworkManager Settings, Settings.Connection, Device, ActiveConnection, and IP configuration D-Bus APIs | Ubuntu mutation/polkit/failure/concurrency/rollback matrix plus keyboard, scale, and accessibility evidence |
| VPN | Exact opaque profile identity; shared keyboard-operable refresh disabled across every editor/import/delete/secret mutation state; libnm-discovered reviewed import-capable plugins plus native WireGuard; portal-selected bounded temporary import with unchanged-file proof, autoconnect rejection, exact preview Save/Delete, and no secret-map rewrite; stable-version existing-profile deletion preview, exact active disconnect, post-disconnect revalidation and authoritative absence; non-lossy common name/account/persistence/timeout editing with stable-map preflight, partial official NetworkManager mutation, full typed/unrelated-field readback, and no secret reads or lossy rollback; exact confirmed plugin-only `ClearSecrets` with active-tunnel truth and an explicit native-WireGuard private-key exclusion; plugin-aware live state; bounded activation/deactivation; ownership-safe Stop/timeout cleanup; authoritative recovery and NetworkManager owner-loss/restart refresh | NetworkManager Settings, Settings.Connection, SecretAgent, ActiveConnection, VPN.Connection, libnm VPN editor plugins, native WireGuard, bounded argument-separated `nmcli`, and the desktop portal | Ubuntu secret-agent/plugin authentication, edit/import/deletion/interaction/accessibility evidence |
| Battery | Real battery/AC state, health, time/rate, physical capacity/cycles/model, and advertised power profiles through shared pointer/keyboard controls with exact selected/disabled/busy state; coalesced UPower plus modern/legacy profile signals; owner-loss/restart recovery; generation-safe authoritative resampling with a retained post-mutation refresh; single-battery capability-gated optimized charging with private owner/device identity and verified readback; bounded authoritative 24-hour charge history with explicit unsupported/empty/error states | UPower and power-profiles-daemon | Linux threshold/history hardware, polkit, transition, hotplug, restart, suspend, interaction, scale, and accessibility evidence |
| General/About | Typed bounded privacy-safe OS, kernel, architecture, hardware, DRM graphics, and session facts; validated hostname mutation with exact readback; coalesced property/service watching with generation-safe authoritative refresh; injection-safe redacted clipboard report; Apple-only rows removed | `rmac-system-info`, systemd-hostnamed D-Bus/polkit, os-release, procfs, DMI/DRM sysfs, XDG session environment | Linux runtime evidence for successful/cancelled/denied polkit flows, external hostname/service refresh, hardware facts, clipboard contents, and interaction/accessibility |
| Software Update | Complete bounded PackageKit status; advertised install capability; trusted-only exact simulation and confirmation; destructive-plan warning; exact pre-install revalidation; interactive authorization; live download/install phase, percentage, package, remaining-time and capability-gated cancellation; restart requirements; privacy-safe typed failures; coalesced repository/service signals; generation-safe authoritative recovery with stale-state refusal | `rmac-updates`, `rmac-updates-linux`, modern PackageKit system D-Bus, polkit, and the Ubuntu APT backend | Ubuntu APT transaction, auth/cancel/failure/recovery/restart matrix plus keyboard, scale, performance, and accessibility evidence in `docs/software-update.md` |
| Storage | Bounded current-namespace system/removable/network inventory; privacy-safe display names and opaque identities; independent saturating `statvfs` capacity; low-space state; kernel-poll live refresh with generation-safe coalescing; exact revalidated portal-backed Review in Files; no guessed categories or destructive cleanup | `rmac-mounts`, `/proc/self/mountinfo`, pollable `/proc/self/mounts`, `statvfs`, and the desktop OpenURI portal | Ubuntu filesystem/mount/hotplug/portal matrix, a reviewed reversible cleanup design, and keyboard/scale/performance/accessibility evidence in `docs/storage.md` |
| Date & Time | Bounded timedated timezone inventory/current zone, live system clock and read-only RTC mode, NTP capability/enabled/synchronized state, exact timezone and automatic-time transactions, canonical offset-bearing manual clock input with safety confirmation and elapsed-aware readback, interactive polkit, filtered property/restart signals plus kernel timerfd clock-jump detection, generation-safe refresh, low-wakeup display, and last-known-good failures | `rmac-time`, `rmac-time-linux`, `org.freedesktop.timedate1`, and `CLOCK_REALTIME` timerfd | Ubuntu NTP/timezone/manual-clock/polkit/jump/restart/suspend matrix plus keyboard, scale, performance, and accessibility evidence in `docs/date-time.md` |
| Language & Region | Canonical bounded locale1 state and installed inventory; independent Language and Region mutations preserving unrelated effective categories; deterministic native previews; exact readback; conflict-checked locale rollback; validated multi-layout XKB editing only when niri follows localed; exact keyboard readback/rollback; filtered generation-safe property/restart refresh; privacy-safe failures; explicit sign-out requirement | `rmac-locale`, `rmac-locale-linux`, `rmac-input`, POSIX locale objects, `org.freedesktop.locale1`, `locale -a`, and `localectl` layout inventory | Ubuntu language/region/XKB/polkit/concurrency/restart/sign-out matrix plus keyboard, scale, performance, and accessibility evidence in `docs/language-region.md` |
| Login Items | Effective bounded XDG precedence with non-symlink reads, session applicability, atomic exact-readback `Hidden` overrides, and malformed-entry suppression; portal add/replace with exact source/destination-byte and sign-in-command review; exact revalidated Trash removal with lower-system-entry protection; bounded systemd inventory with user-owned-only persistent toggles and read-only system/protected/runtime/masked/static states; sender-filtered generation-safe live refresh, privacy-safe failures, and partial-authority recovery | `rmac-login-items`, `rmac-login-items-linux`, `rmac-portal`, XDG specifications, freedesktop Trash, `org.freedesktop.systemd1` user manager | Ubuntu XDG/systemd mutation, conflict, restart, privacy, interaction, scale, performance, and accessibility evidence in `docs/login-items.md` |
| Sharing | Capability-detected OpenSSH and Samba services with separate runtime/boot state, explicit enable/disable confirmation, systemd system-manager/polkit mutation, bounded completion wait and exact rollback; effective bounded Samba share names from `testparm -s`; separate read-only UFW SSH/Samba allowance truth; live systemd, UFW, and Samba configuration refresh; no AirDrop branding | `rmac-sharing`, `rmac-sharing-linux`, `org.freedesktop.systemd1`, `ssh.service`, `smbd.service`, Samba `testparm`, UFW status/configuration | Linux polkit/network/scale/accessibility evidence and reviewed share editing if added |
| Accessibility | Generation-safe live rmac contrast, motion, and bounded application text preferences with scoped conflict-refusing save and exact readback; shared GPUI rem adoption without changing output/content fonts; separate bounded, live-watched, conflict-refusing GNOME/GTK text authority; niri-backed keyboard/pointer controls; off-thread full-session, enabled-output, exported-`DISPLAY`, satellite, and Orca readiness; explicit EGL, zoom, curtain, and rmac AT-SPI limits | `rmac-theme`, `rmac-ui`, `rmac-gtk-settings`, Settings portal appearance values, GNOME interface GSettings, `rmac-input`, direct niri state/configuration and accessibility bridge | Ubuntu/niri text, input, watcher/recovery, output, Xwayland/EGL, and application AT-SPI/Orca matrix in `docs/accessibility.md` |
| Appearance | Real scheme, accent, contrast, and motion preferences with host-following automatic modes, atomic persistence/recovery, portal and preference-file watching, generation-safe refresh, conflict-refusing scoped mutation, exact readback, live adoption across all seven apps, shared pointer/keyboard controls, and bounded truthful pane semantics | Settings portal plus `rmac-theme` | Linux visual, external-change/concurrency, scaling, contrast, motion, keyboard, and Orca evidence |
| Assistant & Intelligence | Hidden from production navigation | Optional local/provider integrations | Leave absent until a privacy design exists |
| Desktop & Dock | Live C4 shell-settings editor for placement, all/primary/named-output scope, autohide, magnification/scale, reserved space, and supported repeated-click behavior; every mutation reloads the latest complete document, changes only Dock policy, saves atomically, rereads authority, retains last-known-good UI state, and offers one-step Dock rollback; bounded file watching resamples external edits and reconnects; direct niri events expose connection and enabled-output capability; primary scope consumes the persistent display Main authority without connector-order guessing; unsupported application-hide behavior is explicit | `rmac-shell-settings`, `rmac-compositor`, `rmac-display`, direct `rmac-compositor-niri` event stream, and the existing `rmac-dock`/`rmac-dock-runtime` consumer | Actual layer-surface Dock presentation plus live niri/reference-PC hotplug, scale, keyboard, accessibility, and performance evidence |
| Displays | Typed live output identity and layout; proportional arrangement preview plus shared pointer/keyboard choices for Main, edge placement, mode, scale, and rotation; exact advertised mutations; complete pre-change snapshot with focusable 15-second Keep/Revert safety actions; concurrent-change-safe whole-layout restore; coalesced live resampling; one persistent Main output; bounded/symlink-refusing owned niri include; complete-layout validation, candidate `niri validate`, atomic save, last-good copy, verified readback, and truthful native-mirror limitation | `rmac-display`, bounded argument-separated niri output IPC, direct `rmac-compositor-niri` event hints, and an isolated first-position niri include | Ubuntu/niri multi-monitor persistence, hotplug/dock/lid/suspend/restart/failure/concurrency interaction matrix plus keyboard, scale, and accessibility evidence recorded in `docs/displays.md` |
| Spotlight | Live C4 editor for application, Settings, file, and calculator providers; explicit private-file permission and removable-mount scope; portal-selected canonical directory exclusions; truthful on-demand/no-background-index state; reviewed clearing of rmac's merged recent-document view through an owner-only monotonic boundary that never deletes files or rewrites another application's history and fails closed if both private copies are unavailable; readable session shortcut-backend status with portal and niri trigger forms; capability-gated, acknowledged GlobalShortcuts v2 configuration request through the broker's existing session, with no second binding authority; authoritative save/readback, external-change resampling, last-known-good state, and one-step search-policy rollback; consumed live by the centered launcher surface | `rmac-shell-settings`, `rmac-recent-documents`, `rmac-launcher-providers`, `rmac-search`, `rmac-portal`, `rmac-shortcuts`, and `rmac-launcher-app` | Linux portal consent/configuration, recent-history clearing/repopulation, placement/focus, keyboard, scale, performance, and accessibility evidence |
| Wallpaper | Live default and stable-output-specific C4 choices; connected and persisted-offline output targeting; inherited/override state; portal-mediated local image choice with MIME/extension guidance followed by bounded magic-byte/size/dimension/decode validation; original Aurora reset; exact Fill/Fit/Stretch/Center/Tile preview geometry; exact selected-file watching with generation-safe refresh; authoritative save/readback, external-change resampling, and one-step whole-wallpaper rollback; filenames only, never private paths; exclusive staged third-party portal import with frozen decode preview, content-addressed durable copy, whole-desktop Fill commit, explicit response mapping, cleanup, and crash recovery; exact authenticated `SetWallpaperURI` backend wire method on a dedicated bus name; bounded mandatory path-free Open/Close preview lifecycle with terminal-event backpressure; one-shot consent; dynamically exported cancellable Request handle; stale/replayed decision rejection; one-at-a-time FIFO presenter with exact Fill preview, consequence disclosure, semantic labels, keyboard/default focus, resolving state, and external dismissal | `rmac-shell-settings`, `rmac-portal`, `rmac-wallpaper`, `rmac-wallpaper-system`, `rmac-wallpaper-image`, `rmac-wallpaper-portal`, `org.freedesktop.impl.portal.Wallpaper`, `org.freedesktop.impl.portal.Request`, and direct `rmac-compositor-niri` output events | Wayland background layer-surface presentation; supervised wallpaper executable and mandatory GPUI rendering of the consent presenter; backend descriptor/install selection; Linux portal/hotplug/scale/keyboard/accessibility evidence |
| Notifications | Live service-backed app/policy list with desktop-entry names/icons plus shared keyboard-operable refresh/navigation/toggles for master delivery, banner, sound, top-bar badge, history, and urgent-through-Focus controls with exact loading/busy/dependent-disabled state; field-preserving mutations and authoritative readback with separate stream/mutation errors; paired with an on-demand translucent Notification Center panel using the same authoritative grouped history, clear/read mutations, per-app disable policy, and live-only default/button actions whose private IDs and targets remain service-owned | `org.rmac.NotificationCenter1`, `org.rmac.Focus1`, `rmac-notification-center-panel`, plus live XDG app catalog | Lock-preview controls only after secure preview presentation is integrated, plus Linux policy enforcement, interaction, accessibility, and action-focus evidence |
| Sound | Real machine-readable device identity, friendly labels, verified defaults, volume, mute, advertised ports, hardware profiles, and capability-gated front-stereo balance through shared pointer/keyboard controls with exact selected/active/disabled/busy state; exact node/device/active-profile/route/channel associations and availability-gated mutations; private authority names are redacted and never persisted; missing or ambiguous capability data has a separate failure state; local-only alert/startup/UI-sound controls removed; coalesced PipeWire graph monitoring with outage/recovery state; generation-safe full resampling and retained post-mutation refresh | PipeWire/WirePlumber through bounded argument-separated `wpctl`, `pw-dump`, `pw-cli`, and `pw-mon` | Linux hotplug/restart/suspend/interaction/scale/accessibility evidence |
| Keyboard, Mouse & Trackpad | Recursive positional niri graph resolution; effective keyboard merge and complete pointing-section replacement; explicit off/false and empty-localed-XKB semantics; exact final owned KDL v1 include; complete graph concurrency checks; bounded candidate validation and direct successful-reload witness; atomic save, last-good, rollback, and full readback; live privacy-safe kernel/udev inventory and coalesced hotplug/config refresh; truthful individual-device limitation | `rmac-input`, niri/libinput KDL v1 configuration and direct IPC, `/sys/class/input`, `/run/udev/data`, `/dev/input`, and `rmac-compositor-niri` successful `ConfigLoaded` hints | Ubuntu/niri graph, hardware/hotplug/restart/suspend, transaction-failure, interaction, scale, and accessibility matrix in `docs/input.md`; individual-device overrides require upstream niri support |
| Focus | Live service-backed atomic state/configuration with shared keyboard-operable refresh, current-mode action, mode/app navigation, activation, urgent-policy, and create/edit/enable/delete schedule controls; desktop-entry names/icons; manual mode/duration activation; urgent and per-app allow-list policy; exact source-aware current state offers Turn Off only for manual Focus and routes scheduled Focus to its authoritative editor with truthful loading/busy state | `org.rmac.Focus1`, `org.rmac.NotificationCenter1`, and live XDG app catalog | Linux/niri schedule, interaction, scaling, and accessibility evidence |
| Screen Time | Hidden from production navigation | No service selected | Usage model only after a local-first privacy design |
| Lock Screen | Real live lock and capability-gated automatic-suspend choices through shared pointer/keyboard controls; truthful selected, disabled, busy, and Hidden-preview state; a readiness-gated Lock Now test using the shipping provider; secure manual/logind/pre-sleep and idle paths; unsupported avatar/message controls omitted without rewriting preserved swaylock appearance | `org.rmac.LockScreen1`, provider security state machine, bounded action-free notification projection, logind `CanSuspend`/`Suspend(false)`, niri `ext-session-lock-v1`, PAM-enabled swaylock, and delay inhibitor | Reviewed custom Wayland/PAM presentation before richer preview/login controls, plus Linux security, interaction, scaling, and accessibility evidence |
| Privacy & Security | Bounded XDG PermissionStore camera/microphone decisions with uninterpreted tokens; version-gated confirmed reset with exact `GetPermission` preflight and post-delete absence proof; reconnecting generation-safe live recovery; PackageKit security-update summary; bounded privacy-safe Ubuntu lifecycle and Pro package-origin/contract/service/unattended-upgrades authorities; live Flatpak/Snap/AppImage desktop provenance; explicit access, trust, and coverage limits | `rmac-privacy`, `rmac-privacy-linux`, `rmac-apps`, `org.freedesktop.impl.portal.PermissionStore`, PackageKit, `ubuntu-distro-info`, Ubuntu Pro Client offline API | Ubuntu/niri reset/concurrency/restart, helper failure, provenance, interaction, scaling, performance, and accessibility matrix in `docs/privacy-security.md`; automatic-update mutation requires a reviewed polkit/rollback design |

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
Every actionable System Settings controller path now renders through the shared
focusable Button, ListRow, Toggle, Slider, TextField, SearchField, or dialog
controls. This includes top-bar Back, generic and application subpage rows,
Focus schedule edit/day/time actions, GTK text-scale presets, niri input presets
and switches, and all pane refresh actions. A source scan has no remaining
private `cursor_pointer` control in the controller; Linux Tab/activation/focus
visibility and Orca behavior still require reference-PC evidence.

Wi-Fi rows retain exact private SSID bytes plus security class instead of using
their lossy display labels as command identifiers. Selecting a known network
activates its most recent compatible saved profile; selecting an open network
asks NetworkManager to complete and persist a profile from the live device and
access point. A bounded ActiveConnection state watch and fresh snapshot are
required before Settings reports success. New WPA Personal and SAE networks use
a masked sheet and an exact-match one-shot NetworkManager Secret Agent; the
password is redacted, zeroized, and delegated to NetworkManager for system-owned
persistence. Saved profiles remain manageable while out of range or with the
radio off; forgetting confirms, disconnects an active exact profile, deletes
all accessible compatible profiles, and recovers authoritative state after a
partial failure. Enterprise and legacy networks remain honestly unavailable.
NetworkManager object signals are coalesced into full snapshot reads; service
owner loss preserves the last known-good state with a separate stream error,
and reappearance guarantees a recovery read. The complete transaction and
remaining F1 gates are recorded in
[`wifi.md`](wifi.md).

Bluetooth device rows retain BlueZ object paths only as opaque service
identities. Before connect, cancel, pair, or remove, `rmac-bluetooth` re-reads
ObjectManager and requires that the exact path still exposes `Device1` under a
live `Adapter1`. New-device pairing registers a transaction-scoped
`KeyboardDisplay` agent on the same system-bus connection used for `Pair`, and
the agent refuses callbacks for any device other than the selected one. The
sheet handles numeric comparison, legacy PIN, passkey entry, just-works
authorization, service authorization, explicit rejection, cancellation, and a
60-second user-response timeout. Success is shown only after BlueZ completes
pairing, accepts `Trusted=true`, and a fresh snapshot reports both `Paired` and
`Trusted`. Forgetting is confirmed and delegates disconnect plus pairing-data
removal to the owning adapter's `RemoveDevice`; failures trigger a recovery
snapshot. BlueZ signals are coalesced into full reads, while owner loss keeps
last-known-good state and owner recovery forces a refresh. The exact contract
and remaining F2 evidence are recorded in [`bluetooth.md`](bluetooth.md).

Network Connection Details edits only the profile currently active on an exact
NetworkManager device. The private mutation identity combines device,
ActiveConnection, Settings.Connection, and UUID; every relationship is
revalidated before a full non-secret settings map is cloned. IPv4/IPv6 methods,
CIDR addresses, gateway, DNS, automatic-DNS policy, and NetworkManager's PAC
proxy model are strictly typed and bounded. Unsupported methods, inline PAC
scripts, deprecated arrays, inaccessible settings, and external unsaved state
remain read-only. The transaction uses global `Settings.VersionId` around a
stable read, `Update2` for version-checked in-memory staging,
`GetAppliedConnection`/`Reapply` to stage and verify IP/DNS without dirtying the
saved profile, fresh applied readback, and version-checked
`Update2` to persist only after proof.
Rollback restores and verifies the original complete map only if that map still
exactly matches rmac's staged candidate; a newer external edit is never
overwritten. The shared coalesced NetworkManager stream refreshes both Network
and Wi-Fi with independent mutation generations. The complete contract and F3
reference-PC matrix are recorded in [`network.md`](network.md).

VPN profiles retain the exact private NetworkManager Settings object path and
UUID rather than using their display name as a mutation target. Connecting and
disconnecting wait for the exact ActiveConnection authority with bounded
timeouts and fresh readback; plugin VPN state distinguishes authentication,
activation, failure, and disconnection. A pending activation exposes Stop and
Escape cancellation. Cleanup first proves the exact profile path, UUID, and
VPN type and only deactivates an activation created by rmac, so a pre-existing
or replacement connection is never torn down by an ownership guess. Every
failure takes an independent recovery snapshot. The shared coalesced
NetworkManager stream now refreshes VPN alongside Wi-Fi and Network with an
independent generation and explicit daemon outage/recovery state.

Import capability is read from libnm's installed plugin inventory and actual
`IMPORT` capability rather than guessed package names; reviewed secure types and
native WireGuard appear only with the required NetworkManager frontend. A
generic local portal choice is bounded to 4 MiB, proven unchanged across a
30-second no-shell import, and retained only as one exact `Unsaved` D-Bus
profile with explicit autoconnect disabled. The preview never calls
`GetSecrets` or rewrites the non-secret map. Confirmation revalidates the exact
path, UUID, temporary state, and full visible settings before `Save`; Cancel
does the same before `Delete`. Command output is bounded, zeroized, and never
used as profile authority. Existing profile deletion separately captures a
stable-version exact full-map preview, refuses temporary profiles, confirms
active disconnection and irreversible secret removal, then revalidates the
unchanged map after exact ActiveConnection teardown before calling `Delete`.
Both the Settings inventory and fresh VPN snapshot must prove absence; partial
failure recovers current authority. The common non-secret editor and confirmed
plugin-only saved-authentication clearing action follow the same stable-map
identity boundary; the latter calls `ClearSecrets` without reading credentials
and never touches native WireGuard private keys. Plugin-specific configuration
and interactive credential acquisition remain delegated to the installed
plugin/Secret Agent. The exact boundary and remaining F4 matrix are recorded in
[`vpn.md`](vpn.md).

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
then replace the UI snapshot only when a complete readback reports the exact
requested static hostname. Hostname property and service-owner changes
coalesce into complete generation-checked reads, preserving last-known-good
facts through service loss and preventing a stale stream read from crossing a
mutation. OS release, procfs, DMI, DRM, and XDG session facts are byte-bounded;
helper processes have bounded concurrent output drains and a five-second
timeout. Graphics reporting admits exact DRM card entries and exposes only the
driver plus public PCI IDs, never bus addresses or serials. Invalid,
unavailable, denied, failed, and mismatched mutations preserve the previous
snapshot. The copied report revalidates fields against control-character
injection and deliberately omits hostname, username, serial numbers, addresses,
machine IDs, and paths. The complete contract and F9 reference-PC matrix are in
[`about.md`](about.md). This follows the hostname service contract documented
by the official systemd
[`org.freedesktop.hostname1` manual](https://www.freedesktop.org/software/systemd/man/latest/org.freedesktop.hostname1.html).
Linux reference-PC interaction evidence remains pending.

Software Update delegates package truth and mutation to modern PackageKit
instead of parsing or constructing `apt` commands. A bounded snapshot exposes
security, critical, blocked, normal, truncated, and backend-capability state.
Install All first refreshes and simulates with PackageKit's trusted-only flag,
then confirms the typed dependency/removal/downgrade and restart plan. After
confirmation, rmac refreshes and reruns the simulation; any exact-plan change
returns to review. The real trusted-only transaction permits the system polkit
agent to authorize it and presents live phase, percentage, current package,
remaining time, restart requirements, and cancellation only while PackageKit
advertises `AllowCancel`.

Repository and service signals coalesce into complete generation-safe reads.
Every success, failure, cancellation, or timeout performs an independent fresh
recovery read, and an unconfirmed current state clears the old list so it cannot
enable a stale retry. Signing, trust, licence, media, authorization, low-space,
database-change, and backend failures are typed without surfacing private
backend strings. The exact contract and pending F10 reference-PC matrix are in
[`software-update.md`](software-update.md), following PackageKit's current
[`Transaction` interface](https://github.com/PackageKit/PackageKit/blob/main/src/org.freedesktop.PackageKit.Transaction.xml).

Storage no longer parses `df` or invents a `Macintosh HD` label on Linux.
`rmac-mounts` performs a size-bounded current-namespace mountinfo read, admits
only the system disk and user-visible removable/network roots, rejects an
unbounded volume set, and separates opaque mount identity from a bounded
control-free label. GVfs host and account fields are not displayed. Each
volume is measured independently with saturating `statvfs`; one inaccessible
or disconnected volume reports its own failure without hiding healthy ones.

Linux mount changes arrive through the kernel's pollable mount table and drive
coalesced complete generation-safe reads. Review in Files revalidates the exact
opaque identity/path/class immediately before using the desktop portal, so a
stale label or replaced mount is never opened as authority. The pane flags low
space and offers conservative guidance, but it does not fabricate storage
categories or expose destructive cleanup until measurements and a reversible
transaction exist. The full F11 contract and remaining reference-PC evidence
are recorded in [`storage.md`](storage.md).

Date & Time reads typed properties and the bounded `ListTimezones()` inventory
from systemd-timedated. Time-zone and automatic-time changes validate locally,
request interactive polkit authorization, and replace UI state only with a
fresh service snapshot. Unsupported NTP, unavailable service, invalid zone,
denied authorization, and mutation failure preserve the last known-good state.
Manual clock editing requires automatic time off, canonical offset-bearing
input, an impact confirmation, interactive authorization, and elapsed-aware
exact readback. The hardware-clock mode is deliberately read-only and identifies
UTC as the normal Linux configuration. A bounded event stream refreshes from
authority on filtered timedated property changes and daemon reappearance;
kernel realtime timer cancellation also detects discontinuous clock jumps that
do not emit a property signal. Generation guards preserve mutation readback and
last-known-good state across reconnects. The complete contract and remaining
Linux matrix are in [`date-time.md`](date-time.md). Authority reference:
[Ubuntu 26.04 `org.freedesktop.timedate1(5)`](https://manpages.ubuntu.com/manpages/resolute/man5/org.freedesktop.timedate1.5.html).

Language & Region reads locale assignments and default keyboard metadata from
systemd-localed, and obtains a bounded installed-locale inventory from the
argument-separated standard `locale -a` command. The domain service validates
locale syntax and installation before any mutation. Language changes replace
only `LANG` while preserving every effective `LC_*` override; Region changes
only numeric, time, monetary, paper, name, address, telephone, and measurement
formats. Both account for localed merge, redundant-assignment simplification,
and derived `LANGUAGE`; request interactive polkit authorization; and require
canonical complete readback. One-step rollback requires the current state to
match the exact applied state, refusing to overwrite concurrent edits. Failed,
denied, unavailable, conflicting, and mismatched mutations preserve the last
known-good snapshot. Filtered property and owner signals drive generation-safe
coalesced refresh with retained recovery after a busy transaction. The pane
states that the current session must sign out and back in, because running
processes do not adopt the new environment. Deterministic examples use
independent POSIX locale objects for `LC_TIME`, `LC_NUMERIC`, and
`LC_MONETARY`; this avoids
changing process-global locale state and makes each retained override visible.
If native preview construction fails, the authoritative assignment remains
visible with an explicit preview error. Input sources accept one to four
installed comma-separated XKB layouts plus aligned variants and validated XKB
switching options. The adapter preserves the existing XKB model, calls localed's
`SetX11Keyboard` with console conversion disabled and interactive authorization,
then requires exact model/layout/variant/options readback. Keyboard rollback
also refuses a changed applied state. `rmac-input` proves whether niri has no
explicit XKB block before enabling the editor: current niri follows localed in
that case. A direct or traversed included XKB block keeps the system default
read-only so rmac does not fight the active config authority. The complete
contract and remaining evidence are in
[`language-region.md`](language-region.md). Authority
references: [Ubuntu 26.04 `org.freedesktop.locale1(5)`](https://manpages.ubuntu.com/manpages/resolute/man5/org.freedesktop.locale1.5.html),
the [official niri integration contract](https://github.com/YaLTeR/niri/wiki/Integrating-niri),
Linux [`nl_langinfo_l(3)`](https://man7.org/linux/man-pages/man3/nl_langinfo.3.html),
[`strftime_l(3)`](https://man7.org/linux/man-pages/man3/strftime.3.html), and
[`strfmon_l(3)`](https://man7.org/linux/man-pages/man3/strfmon.3.html).

Login Items owns the XDG application-autostart half of F14. It applies user and
system config-directory precedence before parsing, so malformed or unreadable
higher-priority files remain bounded issues and still suppress lower copies.
Regular-file-only bounded reads expose `Hidden`, session inclusion, and private-
safe `TryExec` availability. Toggle transactions revalidate exact snapshots and
source bytes, write atomic user-owned overrides, and demand exact file plus
inventory readback.

Portal-selected Add/Replace previews retain exact source and optional target
bytes and show the exact sign-in command before confirmation. A changed source,
appeared/removed target, or changed replacement is refused. Remove captures the
fresh user-owned file and repeats exact preparation before using desktop Trash.
If removal reveals an enabled system entry, a revalidated managed hidden
override prevents a new next-login launch without overwriting a concurrent user
entry. Applications and systemd unit files are never deleted from this pane.

The same bounded snapshot uses the session-bus systemd user manager's
`ListUnitFiles()` authority. User-owned persistent enabled, linked, and disabled
states may be toggled after a second exact inventory check and complete
readback. System-provided, runtime-only, masked, static/generated, unknown, and
`rmac-*` infrastructure states remain read-only with an explanation. Changes
call `EnableUnitFiles()` or `DisableUnitFiles()` and `Reload()` but do not start
or stop an already-running service. Missing systemd authority degrades only the
background-service section.

A capacity-one filesystem stream, sender-filtered `UnitFilesChanged`, and
filtered manager-owner changes drive complete off-thread snapshots and
reconnect after loss. Settings generations prevent an older stream read from
crossing a preview, reveal, refresh, or mutation, while retaining one pending
refresh through busy work. The full authority contract, source references, and
remaining Linux matrix are in [`login-items.md`](login-items.md).

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

Appearance exposes scheme, accent, contrast, and motion through the shared
keyboard-operable control path used by pointer activation. Its bounded semantic
projection preserves exact visual order and selected, disabled, busy, loading,
unavailable, error, host-authority, and effective-appearance state. Typed actions
dispatch through the same authoritative mutation methods as the visible pane;
they never infer a setting from display text. A custom valid accent remains
truthfully labeled Custom with no preset falsely selected, malformed colors and
oversized dynamic text fail closed, and diagnostic formatting redacts recovery
detail. The pinned GPUI stack still cannot export this contract to AT-SPI, so
Linux keyboard and Orca evidence remains required.

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
fonts remain separate. Portal and theme-file events now coalesce into complete
generation-safe reads. Every in-pane change performs a fresh whole-preference
identity check, changes only its selected field, saves atomically, and requires
exact readback, so a cached Settings snapshot cannot overwrite another editor.
Linux clipping evidence is still required.
GTK Application Text is a separate Standard/Large/Extra Large control backed by
`org.gnome.desktop.interface text-scaling-factor`. It distinguishes missing
GSettings/schema support from a policy-locked key, validates requested factors,
performs a second exact value-and-policy preflight, refuses concurrent changes,
and reports success only after an authoritative readback matches. A bounded,
reconnecting `gsettings monitor` stream drives generation-safe complete reads,
preserves last-known-good state through failure, and never surfaces raw command
diagnostics. Custom GTK factors remain visible without falsely selecting a
preset. This changes GTK application text without claiming to control browsers,
document fonts, niri output scale, or rmac's GPUI typography.
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
desktop session, a directly reported enabled output, exported Xwayland
`DISPLAY`, bounded-PATH `xwayland-satellite`, and Orca executable are reported
independently. `DISPLAY` is not presented as proof that Xwayland works, and the
satellite check does not prove its version or a configured custom path. The pane
shows niri's documented default `Super`–`Alt`–`S` shortcut but does not claim
that a user-customized binding exists. An explicit off-thread refresh resamples
the environment and package prerequisites after changes. Working EGL, speech,
focus transfer, built-in zoom, and screen curtain remain explicit limitations.
This readiness describes niri and Orca, not rmac: application roles, names,
states, actions, focus, and announcements remain gated on Linux AT-SPI/Orca
runtime evidence. Authority reference:
[niri Accessibility](https://niri-wm.github.io/niri/Accessibility.html).

Privacy & Security no longer uses the generic unavailable renderer. Its first
permission slice reads only the PermissionStore `devices` table's `camera` and
`microphone` resources, preserving the store's uninterpreted permission strings
instead of translating them into invented policy. Missing resources are empty
states, store/interface failures remain visible, and interface version 1 stays
read-only. Version 2 decisions can be reset per application only after explicit
confirmation. The retained resource, application ID, and uninterpreted tokens
must match an immediate `GetPermission` preflight before `DeletePermission`;
completion requires complete authoritative resampling and selected-pair
absence. Missing or changed preflight state is refused. The interface has no
compare-and-delete operation, so the pane documents that narrow concurrency
limit rather than attempting a lossy rollback. Reset is described as removing a
stored decision so the next portal request may ask again, never as terminating
active capture or revoking native-application access. The pane also reuses the
cached PackageKit security-update count and opens the full Software Update
destination, while
keeping Ubuntu security coverage and repository trust explicitly separate.
PermissionStore `Changed` signals now trigger a complete resample after the
subscribed watcher publishes an initial refresh. Bounded, control-free IDs,
tokens, and complete inventory limits prevent authority payloads from becoming
unbounded UI. Settings generations retain one pending refresh across a manual
read or reset, so an older stream result cannot replace newer transaction
readback. Raw bus diagnostics and peers never reach presentation. Filtered
well-known-name loss/reappearance reports a temporary live-update failure then
resamples without discarding the last known good decisions. Signal storms are
coalesced through a bounded channel, while explicit mutation completion remains
the authority during a reset.
Authority reference: [XDG PermissionStore](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.impl.portal.PermissionStore.html).

The Ubuntu security subsection now reads three independent, offline Ubuntu Pro
Client APIs rather than treating the PackageKit update count as coverage. The
package summary reports installed APT origins; attachment validity and enabled
services report Ubuntu Pro state; and the unattended-upgrades status reports
the timer, periodic job, frequency, allowed origins, disabled reason, and last
run. Each endpoint can fail without hiding successful authorities. “Enabled”
requires the unattended-upgrades service, APT timer, periodic job, and a
non-zero upgrade interval. The pane does not infer repository trust, standard
release support, or Flatpak/Snap/AppImage/manual-install status from those
values. Standard release support is queried separately from the installed
`distro-info-data` authority through `ubuntu-distro-info --days=eol`, after
validating Ubuntu and its series from `/etc/os-release`. Both helpers have a
15-second timeout and 1 MiB output limit, drain both bounded pipes, validate
every rendered string/list, and replace raw stderr/API failure titles with fixed
privacy-safe capability failures. Authority references:
[ubuntu-distro-info](https://manpages.ubuntu.com/manpages/questing/man1/ubuntu-distro-info.1.html),
[Ubuntu Pro Client API](https://documentation.ubuntu.com/pro-client/en/docs/references/api/),
and [unattended-upgrade](https://manpages.ubuntu.com/manpages/noble/man8/unattended-upgrade.8.html).

Application provenance is a separate live inventory over the same XDG desktop
catalog used throughout rmac. Exact Flatpak export and Snap desktop paths are
counted separately, while AppImage entries require an integration ID or an
`.AppImage` launch executable. Remaining entries are labeled system, user, or
other desktop entries; rmac does not equate those locations with APT ownership.
The count therefore covers desktop-visible applications, not command-line-only
packages, repository signatures, or whether an individual application is safe.

Assistant & Intelligence and Screen Time are absent from sidebar and search
navigation because neither has an accepted local-first privacy/service design.
Every production category is regression-tested to have an explicit renderer.
Required roadmap destinations that do not yet have a reviewed editor show a
specific capability/limitation state; the old generic rows and clickable
placeholder subpages have been removed.

Sound now renders only controls backed by the system audio service. The former
alert-sound picker, alert volume, startup sound, UI-effect toggles, and unused
balance preference were local persistence pretending to be session policy; the
obsolete private settings file containing those keys is ignored rather than
migrated. Balance has returned as live PipeWire state only for an exact writable
front-stereo channel map, with no rmac-owned persistence.
