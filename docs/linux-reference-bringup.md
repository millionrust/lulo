# Linux reference PC bring-up

This machine is the truth source for turning rmac from macOS-developed
prototypes into a coherent Linux desktop. Virtual machines and nested
compositors remain useful CI tools, but they cannot approve GPU, input, scaling,
accessibility, portal, multi-monitor, or session behavior.

## 1. Reference installation

Install Ubuntu Desktop 26.04 LTS with current firmware updates. Keep the stock
GNOME Wayland session: it is the compatibility baseline and recovery desktop.
Ubuntu 26.04 no longer offers a GNOME Xorg session, so an ordinary GNOME login
is already the required Wayland path.

Prefer an AMD or Intel GPU for the first complete gate. NVIDIA is still part of
the release matrix, but it should be the second independent result rather than
the only development machine. Record the GPU and driver; do not replace a
working distribution driver merely to chase a newer version during baseline
capture.

Before changing compositor or shell configuration, confirm that networking,
audio, Bluetooth, suspend/resume, display scaling, keyboard layout, and the
Ubuntu recovery login all work. Use a separate test user if the machine also
contains important personal data.

## 2. Development dependencies

Enable Ubuntu's `universe` repository, update the machine, and install the
product build/runtime and evidence tools:

```sh
sudo add-apt-repository universe
sudo apt update
sudo apt full-upgrade
sudo apt install --yes \
  at-spi2-core build-essential clang curl dbus git jq libfontconfig1-dev \
  libfreetype-dev libglib2.0-bin libpam0g-dev libssl-dev libvulkan-dev libwayland-dev \
  libx11-xcb-dev libxcb1-dev libxcb-render0-dev libxcb-shape0-dev \
  libxcb-xfixes0-dev libxkbcommon-dev libxkbcommon-x11-dev mesa-vulkan-drivers \
  orca pciutils pkg-config python3-pyatspi sway swayidle swaylock vulkan-tools wayland-utils \
  xdg-desktop-portal xdg-desktop-portal-gnome
```

Install Rust through rustup, then let the repository's
`rust-toolchain.toml` select Rust 1.94.1. Pin the dependency-policy tool to the
version used by CI:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
cargo install --locked cargo-deny --version 0.19.8
```

Clone rmac normally. Do not run the applications or benchmark as root.

## 3. Capture the untouched GNOME baseline

Log into “Ubuntu” in GDM, open a terminal, and verify:

```sh
echo "$XDG_SESSION_TYPE"
echo "$XDG_CURRENT_DESKTOP"
vulkaninfo --summary
wayland-info | less
```

The session type must be `wayland`, and `vulkaninfo` must identify the real GPU
driver rather than llvmpipe/lavapipe. From the repository, run:

```sh
bash scripts/linux/run-reference-gates.sh --with-upstream-smoke
```

This stores non-serial hardware, session, Wayland, Vulkan, accessibility,
portal, package, Clippy, test, policy, and nested-smoke evidence under ignored
`target/linux-evidence/<UTC timestamp>/`. Review the files before sharing them;
the collector deliberately omits hostname, machine ID, and hardware serials.

## 4. Manual GNOME platform-lab pass

Run `cargo run -p rmac-platform-lab` and record pass/fail for every item:

- text entry, selection, copy, paste, and keyboard shortcuts;
- an IME composition sequence in the keyboard layouts you actually use;
- menu and dialog keyboard operation, including Escape and focus return;
- smooth mouse and touchpad scrolling without idle redraw activity;
- external file drop and clipboard data from another application;
- the portal file chooser, cancel path, permission denial, and a successful
  file selection;
- 100%, 125%, 150%, and 200% scale where supported;
- moving the window between differently scaled displays;
- suspend/resume and display disconnect/reconnect while the app is open.

Enable Orca and repeat the keyboard journey. The stable GPUI 0.2.2 lab is
expected to expose the documented accessibility gap; capture the failure
precisely rather than marking the whole application “broken.”

## 5. Add niri without removing GNOME

Use niri 26.04 as the shell-development compositor. Prefer a trusted
distribution package when one is available. Otherwise follow niri's official
Getting Started and manual-install instructions for the exact `v26.04` tag;
do not copy a random desktop “rice” or install an unreviewed shell bundle. Keep
GNOME selectable in GDM throughout Phase 1.

Start the real session through `niri-session` from GDM. The upstream guidance
uses this path so systemd user services, D-Bus, and portals receive the correct
environment. Confirm these before testing rmac:

```sh
echo "$XDG_CURRENT_DESKTOP"
niri --version
niri msg --json outputs | jq
systemctl --user --no-pager status xdg-desktop-portal.service
```

Install the development supervisor and user units, then add the installed
start command to niri's session startup configuration:

```sh
bash scripts/linux/install-session-units.sh
~/.local/bin/rmac-session-start
systemctl --user --no-pager status rmac-session.target
~/.local/libexec/rmac/rmac-session-supervisor status
cat "$XDG_RUNTIME_DIR/rmac/shortcuts-status.json"
```

After `rmac-session-start`, `XDG_CURRENT_DESKTOP` in the user manager must begin
with `rmac:`, `XDG_SESSION_ID` must identify the current logind session, and the
lock coordinator must be active. The restarted portal frontend must select the
rmac notification backend. Verify both owned interfaces and their exact
versions:

```sh
systemctl --user show-environment | grep '^XDG_CURRENT_DESKTOP=rmac:'
systemctl --user show-environment | grep "^XDG_SESSION_ID=${XDG_SESSION_ID}$"
systemctl --user --no-pager status rmac-lock-coordinator.service
systemctl --user --no-pager status rmac-idle-lock.service
busctl --user introspect org.rmac.LockScreen1 \
  /org/rmac/LockScreen1 org.rmac.LockScreen1
busctl --user call org.rmac.LockScreen1 \
  /org/rmac/LockScreen1 org.rmac.LockScreen1 Settings
busctl --system call org.freedesktop.login1 \
  /org/freedesktop/login1 org.freedesktop.login1.Manager CanSuspend
busctl --user introspect org.freedesktop.Notifications \
  /org/freedesktop/Notifications org.freedesktop.Notifications
busctl --user introspect org.freedesktop.impl.portal.desktop.rmac \
  /org/freedesktop/portal/desktop org.freedesktop.impl.portal.Notification
busctl --user get-property org.freedesktop.impl.portal.desktop.rmac \
  /org/freedesktop/portal/desktop \
  org.freedesktop.impl.portal.Notification version
```

The final property command must return `u 2`. Send one legacy notification and
one notification through a sandboxed test app, replace each in place, invoke an
action with an activation token, close each, and record the matching signal and
absence of duplicate banners. This is reference-PC evidence; do not substitute
the macOS compile-time introspection test.

The D-phase component units are condition-gated until their binaries are
installed, so they remain skipped rather than entering false crash loops. The
supervisor must be active and its JSON health output must identify every unit.
To inspect one component locally, use
`journalctl --user -u rmac-dock.service -b`; review logs for private paths or
content before adding them to an evidence bundle.

If shortcut status reports `fallback-required`, add the exact include line
printed by the installer to the niri configuration. Do not enable that include
while status reports the portal backend, or each shortcut would have two
owners. Reload niri and validate the generated file before interaction testing:

```sh
niri validate
```

Run the evidence collector again, then launch the current-upstream probes:

```sh
bash scripts/linux/collect-reference-evidence.sh
cd experiments/gpui-upstream-lab
cargo run --features wayland --bin a11y
cargo run --features wayland --bin layer-shell
cargo run --features wayland --bin top-bar
```

With Orca active, verify the heading, counter, switch, focus order, actions, and
state. For the layer surface, verify the 40-logical-pixel exclusive zone,
keyboard non-interference, overview behavior, maximize/fullscreen interaction,
output hotplug, and mixed scaling. Then complete the four-hour interaction soak
from ADR 0001.

Run the Linux-only PAM callback and transaction fault tests natively. They use
injected function tables and do not authenticate the current account or install
the development PAM service:

```sh
cargo test -p rmac-lock-provider-linux pam:: --locked -- --test-threads=1
```

Run the Linux-only XKB decoder tests against the distribution libxkbcommon and
keyboard data. They cover the Wayland keycode offset, ordinary text, a literal
space in credentials, submit/cancel/backspace actions, missing keymaps, invalid
keycodes, and pre-map size rejection:

```sh
cargo test -p rmac-lock-provider-linux xkb_keyboard:: --locked -- --test-threads=1
```

Record the exact output. Do not install `pam/rmac-lock` into `/etc/pam.d` or run
real authentication until the separate recovery-console procedure and test
account are ready. When cross-checking Linux from macOS, prefix Cargo with
`PAM_SYS_IMPL=linuxpam`; native Linux builds select Linux-PAM automatically.

For the top-bar candidate, require exactly one 32-logical-pixel bar on every
output at 100%, 125%, 150%, and 200%. Verify crisp rendering across a mixed-DPI
pair, a centered clock that changes at the minute without continuous idle
rendering, no keyboard-focus theft, and correct behavior through output
disconnect/reconnect, scale changes, overview, maximize, and fullscreen. With
Orca, each surface must be discoverable as “rmac top bar,” its clock must have
a useful date/time name, and neither static node may become a Tab stop. Record
failures rather than treating the passing nested-Sway gate as niri approval.

## 6. Performance capture

After correctness passes, close unrelated applications and run:

```sh
bash scripts/linux/run-reference-gates.sh --with-performance
```

Compare the resulting JSON with `docs/performance-baseline.md`. Do not compare
debug builds, software Vulkan, or measurements taken while package updates are
running. The first Linux run establishes the hardware baseline; optimization
work then targets measured failures, beginning with Terminal and App Drawer
idle CPU and System Settings first-frame startup.

## 7. Evidence needed from the machine

The first complete report should contain:

- CPU, memory, GPU, kernel, driver, display resolutions/refresh rates/scales;
- GNOME, niri, Orca, portal, Rust, and exact rmac revisions;
- pass/fail notes for every platform-lab and upstream-probe check;
- GNOME and niri performance JSON;
- screenshots or short recordings for scaling, focus, fullscreen, and
  layer-shell failures;
- crash logs and exact reproduction steps, without private filenames or user
  data.

Do not migrate the seven product apps to upstream GPUI until this evidence
satisfies ADR 0002. Once it does, Text Editor is the representative migration;
the rest follow only after that bounded diff passes on the Linux PC.

## Primary setup references

- [Ubuntu Desktop 26.04 installation](https://documentation.ubuntu.com/desktop/en/latest/tutorial/install-ubuntu-desktop/)
- [Ubuntu 26.04 release summary](https://documentation.ubuntu.com/release-notes/26.04/summary-for-lts-users/)
- [niri Getting Started](https://github.com/niri-wm/niri/wiki/Getting-Started)
- [rustup installer](https://rustup.rs/)
