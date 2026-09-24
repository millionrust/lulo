# Changelog

All notable user-facing changes to Lulo OS are recorded here, newest first.
Versions follow [Cargo/semver](https://semver.org) pre-release syntax
(`X.Y.Z-beta.N`); see [docs/beta-checklist.md](docs/beta-checklist.md) for
what each release gate actually verified before a tag went out.

## [0.9.0-beta.1] - 2026-09-24

First public Beta. See
[docs/release-notes/0.9.0-beta1.md](docs/release-notes/0.9.0-beta1.md) for
the full user-facing write-up, and
[docs/known-limitations.md](docs/known-limitations.md) for what still doesn't
work. Grouped by area:

### Desktop & Dock

- A menu bar, Dock, wallpaper, and window chrome drawn to match macOS 26,
  running as their own niri layer-shell surfaces.
- Mission Control, App Exposé, and Show Desktop, with Spaces and hot corners.
- Keyboard access to the Dock (Control-F3: arrows and Tab move, Return/Space
  open, Up opens a tile's menu, Escape returns focus) and recently-used apps
  kept in the Dock after they quit.
- Screenshots, clipboard history, and desktop icons/Stacks/widgets in the
  macOS 26 style.
- A lock screen and login window, and a one-time Setup Assistant on first
  login.

### Menu Bar & Control Centre

- Control Center and Notification Center as real popovers, with Wi-Fi,
  Bluetooth, sound, and battery status wired to the underlying Linux
  services.
- Mac-style keyboard shortcuts (⌘C, ⌘V, and the rest) working across every
  app on a PC keyboard.

### Spotlight

- App and file search, calculator-style answers (sums, units, currency, city
  times, definitions), Actions, and a Clipboard view, in the macOS 26 layout.

### Apps

- **Files**: browse, preview, copy, move, rename, and trash, with Undo, a
  Quick Look panel, and clipboard interop with other Wayland file managers
  (copy/cut now write formats other apps can read, and Paste reads theirs
  back).
- **Terminal**: a real PTY with resize, scrollback, selection, copy/paste,
  and tabs.
- **Notes**: folders, tags, search, and a note list.
- **Text Editor**: a TextEdit-style plain document window.
- **System Monitor**: Activity Monitor's toolbar, process table, and summary
  panel, with keyboard-accessible banners.
- **System Settings**: Wi-Fi, Bluetooth, Network, Battery, Sound, Displays,
  Appearance, Accessibility, Sharing, Focus, Lock Screen, Privacy & Security,
  and more, every pane backed by a real system service (NetworkManager,
  BlueZ, UPower, PipeWire, systemd, or one of Lulo OS's own settings stores;
  see [docs/settings-backend-audit.md](docs/settings-backend-audit.md)).
- New apps: **Calculator**, **Clock** (alarms and timers that still ring
  when the app is closed), **Weather**, **Media Player** (media keys and Now
  Playing), **Preview** (images and PDFs), and **Archive Utility**.

### Accessibility

- A shared focus ring, keyboard operation, and AT-SPI roles/names/states
  added across `rmac-ui`'s toggles, checkboxes, radios, dialogs, menus,
  toasts, and window traffic lights (see
  [docs/accessibility-audit.md](docs/accessibility-audit.md)).
- Live AT-SPI acceptance scripts for product journeys 1-4 on the reference
  laptop, which found and, where fixable in this codebase, closed real
  keyboard/screen-reader gaps in the Dock and Spotlight. Terminal's and
  Notes' own content areas still have no accessible text surface; see
  Known limitations.

### Keyboard

- Mac-style shortcuts across every app, and Control-F3 Dock keyboard
  navigation as described above.

### Safe mode & reliability

- A session safe mode that starts niri without Lulo OS's portal selection or
  optional shell surfaces after a crash loop, lasts exactly one login, and
  explains itself; a crashing shell component no longer takes down the rest
  of the desktop.
- Linux audio and sharing status now read PipeWire's and UFW/Samba's own
  structured state instead of parsing command-line text output.
- Destructive file operations (permanent delete, clipboard eviction) can no
  longer silently drop an error.

### Install & updates

- `install.sh`/`uninstall.sh` and a tag-triggered release pipeline that
  builds `.deb` packages, an SBOM, and build provenance, and attaches them
  to a GitHub Release.
- A daily update-check timer that asks PackageKit for updates and shows a
  notification (installing them is still done by hand in System Settings;
  see Known limitations).
- This Beta ships as a GitHub Release with `.deb` packages and a manual
  install guide. The signed APT repository comes after Beta.

### Brand

- Renamed the project from rmac to **Lulo OS**: the login session, About
  windows, system menus, Setup Assistant, and documentation all say Lulo OS
  now. The Lulo OS mark replaces the placeholder wordmark in the top bar,
  About, the README, and the icon set. (Internal code, package, and D-Bus
  names still say `rmac`; that rename is tracked separately in `todo.md`.)
