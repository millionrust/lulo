# ADR 0027 — Hardware profile: detect each PC once, adapt the shell to it

- Status: accepted
- Date: 2026-10-14
- Follows: ADR 0023 (Lulo on Windows)
- Branch: `op/hw-profile`

## Context

The owner on 2026-10-13: "whenever users install lulo in windows it will detect
everything give everything according to each laptops". Lulo runs on whatever PC
someone installs it on: a desktop with no battery or radios, a convertible with a
pen, a 4 GB laptop with an old iGPU. Until now each surface asked its own service
(UPower, NetworkManager, BlueZ, the WLAN API) and showed a control whether or not
the hardware existed: Control Centre drew a greyed Wi-Fi pill on a desktop on
Ethernet, and low-spec PCs paid for the full blur. System Settings had its own
private sysfs scan (`crates/system-settings/src/hardware.rs`, LINUX-HW-01) that
nothing else could use, and nothing at all on Windows.

## Decision

### One crate, one profile

`crates/rmac-hardware` holds `HardwareProfile`: battery count, panel backlight,
touchscreen (and its touch points), pen, touchpad (precision or not), pointing
stick, external mouse, fingerprint (sensor, and whether the sign-in service can
use it), IR face camera, webcam, microphone, speakers, Bluetooth, Wi-Fi,
Ethernet, cellular, ambient light sensor, orientation accelerometer,
convertible/tablet mode, keyboard backlight, portable, every display (internal or
external, size, refresh rate, scale or ppi, HiDPI), every GPU (vendor, ids,
integrated, old), memory, CPU cores and threads, and the **spec class**.

**Low-spec** is 4 GB of memory or less (≤ 4.5 GiB as the OS reports it), four CPU
threads or fewer, or an old integrated GPU as the main adapter (Intel Haswell and
older by PCI id, AMD on the `radeon` driver). The reasons are kept in
`spec_reasons` and printed.

Surfaces do not read devices. They read `HardwareProfile::adaptations()`, an
`Adaptations` value with one flag per decision (battery item, Wi-Fi, Bluetooth,
display and keyboard brightness, nearby sharing, touch targets, fingerprint,
reduce transparency, reduce motion), so the rules live in one tested place.
`Adaptations::default()` shows everything and turns nothing down: the state
before detection finishes, and the fixed shell scenes (`RMAC_SHELL_SCENE=1`).

### Detection: cheap, off the UI thread, no polling

- **Lulo OS** (`linux.rs`): `/sys/class/{power_supply,backlight,leds,bluetooth,
  net,input,video4linux,sound,drm,wwan}`, `/sys/bus/{usb,iio}`, the input
  devices' capability bitmaps and properties classified as udev's `input_id`
  does (touchpad, touchscreen, pen, stick; virtual devices skipped), the panels'
  EDID for refresh rate and size, CPU topology, `/proc/meminfo`, and whether
  fprintd is installed. No device nodes, D-Bus or subprocesses.
  `LULO_FAKE_SYS_ROOT` points it at the behaviour suite's fake tree, as for
  System Settings and `rmac-osd`.
- **Windows** (`windows_probe.rs` reads, `windows.rs` decides): one SetupAPI
  enumeration of present devices, with properties read only for the classes
  used (Battery, Biometric, Bluetooth, Camera, Image, HIDClass, Mouse, Sensor,
  AudioEndpoint; HID usages for touchpad, touchscreen, pen, light sensor,
  accelerometer and LampArray); the brightness device interface;
  `GetIfTable2` for Wi-Fi, Ethernet and mobile broadband adapters;
  `QueryDisplayConfig` and `GetDpiForMonitor` for displays; DXGI's adapters;
  `GetSystemMetrics(SM_DIGITIZER, SM_MAXIMUMTOUCHES, SM_CONVERTIBLESLATEMODE)`,
  `GetAutoRotationState`, `PowerDeterminePlatformRoleEx`, `GetSystemPowerStatus`,
  `GlobalMemoryStatusEx` and `GetLogicalProcessorInformationEx`.
- **Budget: 20 ms.** `rmac-hardware --check` fails above it; CI runs it on
  `windows-latest` (`ci.yml`, Windows tests) and on the Linux runner the nested
  sessions use (`runtime.yml`).
- **Once per process, then on hot-plug.** `rmac_hardware::start()` detects on a
  thread of its own and caches the result (`current()`); `subscribe()` gives the
  profile and every change. The thread then sleeps in the kernel: a
  `NETLINK_KOBJECT_UEVENT` socket on Linux (add/remove of the subsystems above, a
  `change` only for `drm`), `CM_Register_Notification` for all device instances
  on Windows. A burst is let settle for 400 ms, then detection runs again; an
  unchanged profile wakes nobody.

### Where it is stored

`$XDG_CONFIG_HOME/rmac/hardware.json` on Lulo OS (beside `shell.json`),
`%APPDATA%\Lulo\hardware.json` on Windows. It is written on the first run
(Windows: the first `lulo-shell` start after install, which calls `start()`
itself; Lulo OS: the first session start, through the menu bar's and Control
Centre's subscriptions) and again when the hardware changes, and each write logs
the one-line summary to stderr and to `hardware.log` beside it (Windows' shell has
no console).

### What adapts

| Surface | Rule | Where |
|---|---|---|
| Menu bar | The battery item only when the PC has a battery (a UPS or a peripheral's cell reported by UPower does not count). | `rmac-shell-runtime` coordinator |
| Control Centre | The Wi-Fi and Bluetooth pills only with their radios; the half-width modules close up beside Now Playing, and the panel opens at that height. Display brightness keeps following the backlight it reads. | `rmac-quick-settings::layout`, `quick-settings-app` |
| Low-spec, Windows | Solid materials: the wallpaper's average colour instead of the blurred copy under the Dock, menus and Control Centre. | `rmac-shell-layer::windows::backdrop` |
| Low-spec, Lulo OS | niri's blur at one pass instead of three, through `niri/hardware-generated.kdl`, which the session now includes after the shell's configuration. | `store::compositor_include`, `rmac-wayland-session` |
| Low-spec, both | Control Centre's detail views open without the grow-out motion. | `quick-settings-app` |

### The API for other crates

System Settings and the backend crates (`rmac-audio`, `-display`, `-network`,
`-bluetooth`, `-power`, `-input`) belong to `op/win-settings`; this branch does
not touch them. They can adopt the profile with:

- `rmac_hardware::subscribe() -> async_channel::Receiver<HardwareProfile>`: the
  profile now and on each hot-plug (System Settings' own sysfs scan and udev
  watcher can go; `Capabilities::allows_pane` maps onto `batteries`, `touchpad`,
  `mouse || pointing_stick`, `bluetooth`, `wifi`, `touchscreen`).
- `rmac_hardware::current()` / `adaptations()`: the cached profile, instantly.
- `rmac_hardware::store::load()`: the stored profile without detecting.
- `Adaptations::fingerprint`: offer Touch ID-style settings and the lock screen's
  fingerprint prompt only when a sensor and its sign-in service exist.
- `Adaptations::keyboard_brightness`, `nearby_sharing`: the Keyboard Brightness
  and AirDrop-like Control Centre modules, once those modules exist.
- `Adaptations::touch_targets`: larger hit targets on a touchscreen.

## Not done

- **Touch targets.** The flag is there; no surface enlarges its targets yet.
  Touch scrolling and taps already work where GPUI supports them (TOUCH-01).
- **Fingerprint** at the lock screen and in settings: the lock screen follows the
  PAM stack (`rmac-lock-provider-linux`), and the settings pane is
  `op/win-settings`' to adopt.
- **Keyboard Brightness and AirDrop modules** do not exist in Control Centre yet
  (CC-07); only their flags do.
- **Low-spec motion beyond Control Centre** (Dock magnification and minimise,
  Mission Control) still follows Reduce Motion only.
- **Touch points on Linux** need the device node (`EVIOCGABS`), which the session
  user cannot open; `max_touch_points` is `None` there.
- **Windows keyboard backlight** has no standard interface; only Dynamic
  Lighting (LampArray) keyboards are found.

## Consequences

- The reference laptop (EliteBook 840 G2, 2 cores / 4 threads) is low-spec by
  this rule: from its next login niri blurs with one pass. Emptying
  `~/.config/rmac/niri/hardware-generated.kdl` restores three passes until the
  hardware changes.
- The fixed shell scenes and their pixel comparison are unaffected: they ignore
  the profile on both platforms.
- On the Windows CI runner (no radios), Control Centre shows neither pill outside
  the scenes.
