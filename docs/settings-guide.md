# System Settings guide

System Settings uses a macOS-like sidebar and grouped rows, but every value
comes from a real Lulo OS or Linux authority. A switch is absent or disabled when
the session cannot perform that action safely. Loading, stale, unavailable,
permission-denied, restart-required, and failed states remain distinct.

## Personalize the desktop

- **Appearance** controls light/dark/automatic mode, accent, increased
  contrast, reduced motion, and Lulo OS text size.
- **Desktop & Dock** controls Dock placement, output scope, autohide,
  magnification, reserved space, and supported click behavior.
- **Wallpaper** selects the original built-in wallpaper or a portal-selected
  local image, with per-output choices and exact fit previews.
- **Spotlight** selects search providers, allowed private-file scope,
  exclusions, recent-history clearing, and shortcut status.
- **Notifications & Focus** manages application delivery policies, schedules,
  allow-lists, history behavior, and temporary Focus activation.

## Connect devices and networks

- **Wi-Fi and Network** use NetworkManager for radio, known networks, secure
  connection creation, IP/DNS/proxy editing, VPN plugins, and authoritative
  readback.
- **Bluetooth** uses BlueZ for discovery, secure pairing, trust, connection,
  and removal.
- **Sound** uses PipeWire/WirePlumber for devices, routes, profiles, volume,
  mute, and supported balance.
- **Displays** uses niri output authority for layout, modes, scale, rotation,
  Main display, timed confirmation, and rollback.
- **Keyboard, Mouse & Trackpad** edits supported niri/libinput configuration.
  Unsupported accessibility or per-device controls are stated explicitly.

## System and account behavior

Battery and power, Storage, Date & Time, Language & Region, Login Items,
Sharing, Users, About, Software Update, Accessibility, and Privacy & Security
each retain their own service boundary. Privileged actions use the system
polkit agent; Lulo OS does not ask for or store an administrator password.

## Saving changes

Settings does not assume a request succeeded. A mutation revalidates the
authority, performs one bounded change, and reads the effective state back.
Where safe, the previous value is retained for one-step rollback. A concurrent
external change is refused instead of being overwritten.

Some language, input, update, login, or session changes require sign-out or
restart. The row states that boundary before applying it.

## When a pane is unavailable

Check the pane's capability or service message first. Refresh after the owning
service returns. If it remains unavailable, use
[Troubleshooting](troubleshooting.md) and collect only privacy-safe status
information. Technical authority details for every destination live in the
[Settings audit](system-settings-audit.md).
