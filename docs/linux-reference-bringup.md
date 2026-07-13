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
  at-spi2-core build-essential clang curl dbus fonts-inter git jq libfontconfig1-dev \
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

With retained history present, open the installed Center surface directly:

```sh
~/.local/libexec/rmac/rmac-notification-center-panel
```

Verify newest-app-first grouping, exact localized identity/icon fallback,
automatic unread acknowledgement without deletion, per-app and all-history
clear, Turn Off, Notification Settings routing, default and nonzero-position
button actions, Escape and outside dismissal, and last-known-good content during
a controlled notification-service restart. Before restarting, invoke each live
action and verify the exact matching protocol signal/target. During the restart,
confirm retained records remain readable but expose no stale action buttons;
after a new notification arrives, confirm its ID does not collide with retained
history and only its own actions appear. Repeat at 100%, 125%, 150%, and 200%
scale and with Orca. Capture placement and focus evidence under niri; a correct
macOS-side window is not Linux proof. Record strict-focus behavior explicitly:
the current GPUI surface does not fabricate a Wayland activation token.

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

### 5.1 Accessibility, scale, privacy, and security evidence

Keep at least 15 GiB free throughout this pass. Before any Cargo command, record
`df -h /` and stop if less than 25 GiB is available for a build that may exceed
1 GiB. Reuse the repository's normal `target` directory and run one scoped Cargo
pipeline at a time.

Start the dedicated collector. Add the optional code checks only with at least
25 GiB free; they are package-scoped and reuse the normal target directory:

```sh
bash scripts/linux/run-accessibility-evidence.sh
bash scripts/linux/run-accessibility-evidence.sh --with-code-checks
bash scripts/linux/run-privacy-security-evidence.sh
bash scripts/linux/run-privacy-security-evidence.sh --with-code-checks
```

The commands write authoritative environment values and manual checklists under
the ignored `target/linux-evidence/<UTC timestamp>/accessibility/` and
`privacy-security/` directories. Complete every checklist row; its creation
alone is not evidence that a test passed.

In System Settings → Accessibility, exercise Standard, Large, and Extra Large
application text. At each value, inspect all seven apps at 100% output scale,
then repeat the Extra Large pass at every supported niri output scale from 100%
through 200%. Record clipping, overlap, truncation that hides state, incorrect
hit regions, focus-ring displacement, and text that does not update live. Do not
count editor, note-body, or terminal content fonts as failures; those are
separate user-controlled content typography.

Open one GTK application that uses the GNOME interface setting. In System
Settings → Accessibility → GTK Application Text, apply Standard, Large, and
Extra Large. Record the effective authority before and after each change:

```sh
gsettings get org.gnome.desktop.interface text-scaling-factor
```

The GTK application must visibly adopt the value without changing niri output
scale. This is external-toolkit evidence only; it does not prove rmac scaling.

For keyboard evidence, select each Key repeat preset, refresh System Settings,
and confirm that the exact delay and rate remain displayed. In `wev`, hold one
printable key long enough to distinguish Standard, Deliberate, and Minimal, and
record the observed initial delay and repeat cadence. A custom niri delay/rate
combination must leave every preset unselected while still showing its exact
values. Sticky Keys, Slow Keys, and Bounce Keys must remain described as
unavailable unless niri gains a real compositor authority.

For pointer evidence, select all three Mouse precision presets and confirm
pointer motion changes without output scaling or synthetic cursor movement.
Enable middle-button emulation, press left and right together in a test app,
and verify exactly one middle-click action; disable it and verify the chord no
longer produces that action. Repeat the configuration round-trip for a touchpad
that supports the libinput property. Test Trackpad drag lock and Ignore while
typing separately. Mouse Keys, dwell click, and double-click timing must remain
unavailable rather than showing switches that niri cannot enforce.

Finally, start Orca with niri's documented default `Super`–`Alt`–`S` shortcut.
Refresh the readiness card and record the full-niri-session, Xwayland, and Orca
rows independently. Then run the upstream accessibility probe below and record
the precise rmac AT-SPI failure; environment readiness is not application
accessibility proof.

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
keycodes, pre-map size rejection, Caps Lock decoding, and key-repeat
classification. The full package test also covers repeat rate/delay clamping,
cancellation, multi-seat Caps Lock aggregation, and the no-catch-up-burst rule
without connecting to a compositor:

```sh
cargo test -p rmac-lock-provider-linux xkb_keyboard:: --locked -- --test-threads=1
```

A full native package test also links the generated session-lock/shm wire but
does not issue a lock request. It also runs the portable runtime coordinator
and redacted visual-state/painting tests; Linux-only tests compile the pump,
reject invalid usernames before a Wayland connection is attempted, and—with
`fonts-inter` installed—shape, rasterize, cache, and redact a mixed-script PAM
prompt plus the verified account identity. On every configured output, confirm
the login name appears between the avatar and input field, PAM guidance appears
below the field, neither overlaps at integer scales 1–4, and neither value
appears in provider diagnostics. Portable tests also cover pointer hit regions
and gesture cancellation;
the live matrix must click submit, select both radio choices, drag out of a
pressed target, remove pointer capability, and hot-unplug the focused output.
It must also toggle Caps Lock on focused US and non-US layouts, move focus
between seats, replace the keymap, and remove the active keyboard capability:

During initial PAM work, after Return/click submission, and between supported
multi-message prompts, verify the fixed `Authenticating…` state appears without
animation or idle redraw. Clicking the submit target while that state or no
prompt is visible must do nothing; the next PAM prompt or outcome must replace
the status immediately.

With a real prompt visible, verify keyboard focus draws one crisp accent ring at
integer scales 1–4. Move focus away and back, repeat enter/leave, and exercise a
second seat where available; the ring must follow aggregate focus without
flashing or remaining after the final focused seat/capability disappears. It
must not appear for hidden, authenticating, or binary states.

```sh
cargo test -p rmac-lock-provider -p rmac-lock-provider-linux --locked
```

The evidence-only process boundary may be compiled, but not launched, before
the recovery harness exists. On Linux the compile-only gate is:

```sh
cargo clippy -p rmac-lock-provider-linux --locked \
  --features development-provider --lib --bin rmac-lock-provider --tests \
  -- -D warnings
```

On a macOS cross-check, add `--target x86_64-unknown-linux-gnu` and prefix the
command with `PAM_SYS_IMPL=linuxpam`. This proves the exact-session logind proxy,
systemd notification wrapper, and binary compile; it is not live lock evidence.

### Nested lock-provider recovery gate

Do not run this gate until the normal swaylock session units work, a local outer
terminal remains open, and you have separately proved that `Ctrl+Alt+F3` reaches
a login TTY. Follow `docs/secure-lock-recovery.md` and use a disposable test
user. Review
`crates/rmac-lock-provider-linux/pam/rmac-lock`, then install that exact policy
as `/etc/pam.d/rmac-lock` through the reference machine's authorized packaging
or administrator procedure; the repository scripts never write `/etc`.

The evidence installer requires at least 25 GiB free before its scoped release
build. It installs non-enabled evidence assets and does not change the normal
lock unit:

```sh
bash scripts/linux/install-lock-provider-evidence.sh
```

From the local graphical test session, run the interactive nested test:

```sh
bash scripts/linux/run-lock-provider-recovery-gate.sh --execute
```

The script asks for the exact `NESTED-LOCK-RECOVERY` acknowledgement. It opens a
nested Sway window, stops a ready custom provider and proves its watchdog starts
a new ready process, kills that replacement and proves ordinary crash restart,
then transfers the still-locked nested compositor to the swaylock evidence
fallback. Enter the test user's password in that nested window. Success requires
the fallback to exit normally after authentication. The trap stops evidence
units, terminates nested Sway, clears the advisory test hint, and removes its
private runtime files even on failure.

The gate writes only timestamps, pass/fail state, and numeric watchdog/crash
restart counts to
`target/linux-evidence/<timestamp>/lock-provider-recovery.txt`. It does not
capture the user name, session ID, display name, PID, journal, PAM messages, or
credential content. The nested-Sway log is kept only in the private runtime
directory and removed by cleanup; inspect it locally before changing that
policy during diagnosis.

This gate does not prove niri behavior, physical output hotplug, suspend, PAM
failure variants, or real red-screen key recovery. Before the later real-niri
test, verify the generated `Mod+Ctrl+Q` binding has
`allow-when-locked=true`. If the provider dies on the real locked session, use
that binding to start the installed swaylock unit. From a TTY, the equivalent
recovery is:

```sh
systemctl --user stop rmac-lock-provider-evidence.service
systemctl --user start rmac-lock.service
```

Do not clear `LockedHint` or terminate niri as a substitute for authentication.
Record unit `ActiveState`, `Result`, `MainPID`, and `NRestarts`, but review any
journal or nested-Sway log before sharing it.

Do not expose or invoke the crate-internal acquisition typestate ad hoc. Its
first live run belongs in the dedicated nested-compositor/recovery procedure
after the provider runtime, emergency TTY recovery, and kill/restart harness are
in place. The feature-gated binary does not waive this requirement.

Record the exact output. Outside the reviewed nested procedure above, do not
install `pam/rmac-lock` into `/etc/pam.d` or run real authentication until its
recovery-console and disposable-test-account prerequisites are satisfied. When
cross-checking Linux from macOS, prefix Cargo with `PAM_SYS_IMPL=linuxpam`;
native Linux builds select Linux-PAM automatically.

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
