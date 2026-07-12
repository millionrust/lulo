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
| General/About | Platform identity snapshot | os-release, sysinfo, DMI | Kernel detail and live hostname mutation |
| Software Update | Placeholder | Ubuntu update services | Check, progress, restart requirements; privileged actions via polkit |
| Storage | macOS-shaped `df` snapshot | Filesystem/mount service | Per-volume usage and safe cleanup guidance |
| Date & Time | Placeholder | timedate1 D-Bus | Time zone, automatic time, clock settings |
| Language & Region | Placeholder | locale1 D-Bus and input services | Locale, formats, keyboard/input sources |
| Login Items | Placeholder | systemd user/XDG autostart | Enable/disable user startup entries |
| Sharing | Placeholder | Explicit service adapters | Capability-detected SSH/file sharing controls |
| Accessibility | Placeholder | Settings portal and accessibility stack | Contrast, motion, text scale, Orca-facing controls |
| Appearance | Real scheme, accent, contrast, and motion preferences with host-following automatic modes, atomic persistence, recovery, refresh, and live adoption across all seven apps | Settings portal plus `rmac-theme` | Linux visual, scaling, contrast, motion, and Orca evidence |
| Assistant & Intelligence | Renamed placeholder | Optional local/provider integrations | Leave absent until a privacy design exists |
| Desktop & Dock | Placeholder | rmac shell and niri IPC | Dock, desktop, workspaces, window behavior |
| Displays | Real layout plus transient mode, scale, and rotation controls | niri output IPC/Wayland | Persistent validated layouts, visual positioning, and live signals |
| Spotlight | Placeholder | rmac-search | Sources, exclusions, indexing state, global shortcut |
| Wallpaper | Placeholder | rmac shell | Per-output wallpaper selection and persistence |
| Notifications | Live service-backed app/policy list with desktop-entry names/icons plus allow/block, top-bar badge, and history controls with separate stream/mutation errors | `org.rmac.NotificationCenter1` plus live XDG app catalog | Banner/sound UI after presentation exists, lock previews after secure lock, scoped GPUI build, and Linux interaction/accessibility evidence |
| Sound | Real devices, defaults, volume, and mute | PipeWire/WirePlumber | Live signals, routes, and per-channel balance |
| Keyboard, Mouse & Trackpad | Real persistent input configuration with validation and atomic rollback | niri/libinput | Included-config editing, per-device overrides, and live signals |
| Focus | Live service-backed state/configuration, desktop-entry names/icons, manual mode/duration activation, urgent and per-app allow-list policy, plus create/edit/enable/delete schedule controls | `org.rmac.Focus1`, `org.rmac.NotificationCenter1`, and live XDG app catalog | Scoped GPUI build and Linux/niri interaction/accessibility evidence |
| Screen Time | Placeholder | No service selected | Usage model only after a local-first privacy design |
| Lock Screen | Placeholder | session shell and logind | Idle timeout, lock, suspend, login presentation |
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
