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
| Appearance | Local persisted choices | Settings portal plus rmac theme service | Apply color scheme/accent to every rmac surface |
| Assistant & Intelligence | Renamed placeholder | Optional local/provider integrations | Leave absent until a privacy design exists |
| Desktop & Dock | Placeholder | rmac shell and niri IPC | Dock, desktop, workspaces, window behavior |
| Displays | macOS read-only snapshot | niri output IPC/Wayland | Live layout, scale, mode, orientation, apply/revert |
| Spotlight | Placeholder | rmac-search | Sources, exclusions, indexing state, global shortcut |
| Wallpaper | Placeholder | rmac shell | Per-output wallpaper selection and persistence |
| Notifications | Placeholder | Notification portal/daemon | Per-app policy, previews, badges, do-not-disturb |
| Sound | Real devices, defaults, volume, and mute | PipeWire/WirePlumber | Live signals, routes, and per-channel balance |
| Focus | Placeholder | rmac notification service | Modes, schedules, shell indicator |
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
