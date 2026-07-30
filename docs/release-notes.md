# Release notes

## Unreleased

rmac is preparing its first contributor/Alpha candidate. There is no supported
public package or upgrade channel yet.

### Product direction

- A coherent macOS-like Linux desktop with top bar, Dock, launcher,
  Notification Center, Quick Settings, lock, wallpaper, and shared appearance
- Seven native first-party applications: Files, Notes, Text Editor, Terminal,
  System Monitor, Applications, and System Settings
- Linux authorities kept explicit: niri, portals, systemd, D-Bus/polkit,
  NetworkManager, BlueZ, PipeWire/WirePlumber, UPower, logind, PackageKit, PAM,
  and the filesystem remain in control

### Release engineering now defined

- Native Debian package, Flatpak, session integration, APT trust, update,
  hardware, automated journey, deterministic visual, accessibility,
  performance, chaos/soak, and security-review contracts
- Exact Alpha/Beta/1.0 H8 station tiers
- Safe-mode and stock Ubuntu/GNOME recovery boundaries
- User-facing install, everyday-use, Settings, shortcut, privacy,
  troubleshooting, update/rollback/removal, and hardware guidance

### Known blockers

The first Alpha remains blocked on the Linux framework decision and native
layer/accessibility evidence, real package installation, critical visual
review, Orca, performance, hardware, chaos/soak, and security gates. See
[Known limitations](known-limitations.md).

### Compatibility and data

The reference target is Ubuntu 26.04 with niri. No migration or rollback claim
is made for an unreleased build. Test only with synthetic data and retain the
stock Ubuntu/GNOME recovery session.

Future entries must state user-visible changes, fixed defects, security/privacy
impact, persisted-data or package compatibility, known limitations, required
restart/sign-out, and exact rollback steps.
