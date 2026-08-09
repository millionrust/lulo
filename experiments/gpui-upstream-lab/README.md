# Current-upstream GPUI gate

This experiment answers two questions that the crates.io GPUI 0.2.2 platform
lab cannot answer:

1. Does GPUI's current AccessKit integration expose useful roles, names,
   states, values, focus order, and actions to Orca on Ubuntu/Wayland?
2. Does GPUI's layer-shell support produce a reliable top panel under niri,
   including around fullscreen windows and monitor changes?

It is deliberately excluded from the product Cargo workspace. The product
remains on the released GPUI 0.2.2 and Rust 1.94.1 while this experiment pins
Zed revision `76c93968da5b8b8809bdd72e4ad9e7d0e946bad0` and Rust 1.95.0.

## macOS compile and smoke test

Run from this directory so the experiment's toolchain file takes effect:

```sh
cargo check --bin a11y
cargo run --bin a11y
cargo run --bin layer-shell # expected to exit with an unsupported-platform message
```

## Ubuntu 26.04 Wayland gate

Install the native build dependencies listed by current upstream GPUI, then:

```sh
cargo check --features wayland --bins
cargo run --features wayland --bin a11y
cargo run --features wayland --bin layer-shell
cargo run --features wayland --bin top-bar
cargo run --features wayland --bin wallpaper
cargo run --features wayland --bin dock
```

To build and launch the visible shell candidates together instead of opening
them one by one, run this from the active niri session:

```sh
bash scripts/run-shell-preview.sh
```

That single foreground command starts the wallpaper, menu bar, and Dock, and
stops all three together when Ctrl+C is pressed. It enforces 25 GiB free before
the first build and the project's 15 GiB absolute floor afterward. This is the
framework-promotion preview; it is not the final signed rmac installer.

With Orca running, verify the application/heading/spin-button/switch roles,
their labels and state/value changes, Tab and Shift-Tab order, and Orca-issued
increment/decrement actions. Under niri, verify that the layer-shell surface is
40 logical pixels high, reserves that space, spans the active monitor's top
edge, does not steal keyboard focus, survives monitor changes, and behaves as
specified when another window is fullscreen.

The `top-bar` candidate creates one 32-logical-pixel surface per output. One
shared `rmac-shell-runtime` instance projects the focused application and the
available Focus, VPN, Wi-Fi, Bluetooth, sound, battery, and notification
snapshots into every surface; source-health-only publications do not request a
frame. The clock remains centered and the original rmac mark plus focused app
lead the bar.
Verify one bar and one exclusive zone per output, mixed/fractional scaling, no
keyboard focus theft, output hotplug, fullscreen behavior, named toolbar,
clock, and status semantics in Orca, and no continuous idle redraw.

The `wallpaper` candidate creates one passive background-layer surface per
output. It renders the original procedural rmac Aurora palette, extends behind
the top bar and Dock exclusive zones, requests no keyboard focus, exposes one
path-free image semantic per surface, and schedules no redraws after its first
frame. Verify that it covers every output without changing the work area,
stays behind ordinary and fullscreen windows, and never enters keyboard or
Orca focus order.

The `dock` candidate creates one bottom top-layer surface per output with an
84-logical-pixel stable reservation and a centered translucent shelf. Its six
first-login items match the shell-settings authority and launch the installed
rmac applications through fixed argument-separated executable names. It has
pointer activation but no keyboard-interactive layer surface, creates no fake
running state, and schedules no idle redraw. Live catalog/window indicators,
typed Dock dispatch, icon files, magnification, and accessible focus handoff
remain the next candidate slice rather than being represented as complete.

Record the compositor, display protocol, scale factors, GPU/driver, Orca
version, and pass/fail evidence in `docs/gpui-current-upstream-spike.md` before
changing any product dependency.

Create the fixed A4 result ledger before the run, change only its `pending`
tokens to `pass` or `fail` as each named observation is reviewed, then verify
it:

```sh
python3 scripts/a4-report.py create \
  > ../../target/linux-evidence/a4-upstream-report.txt
python3 scripts/a4-report.py verify \
  ../../target/linux-evidence/a4-upstream-report.txt
```

The verifier binds all 23 results to this experiment's exact upstream revision,
rejects missing/extra/reordered/duplicate fields, refuses non-ASCII and reports
over 16 KiB, distinguishes an incomplete report from a complete report with a
failure, and accepts only regular non-symlink files. It stores no evidence
descriptions or environment values: pair it with separately reviewed evidence.
Passing verification proves report completeness, not that the human
observations were truthful.

## Automated nested-compositor smoke test

On Linux with Sway, Mesa's software Vulkan driver, `wayland-info`, `jq`, D-Bus,
GSettings, AT-SPI, and Python pyatspi installed, run:

```sh
dbus-run-session -- bash scripts/nested-wayland-smoke.sh
```

The script builds all Wayland probes, starts a two-output headless nested Sway
session, verifies the layer-shell protocol and 40-pixel probe zone, then checks
one wallpaper, 32-pixel top bar, and 84-pixel Dock per 1x/2x output. It verifies
configured scale, work-area ownership, idle-render deltas, process health, and
non-focusable toolbar/clock semantics. It also checks the interactive probe's
roles, names, numeric value, click actions, and toggled state over AT-SPI. It is
a deterministic smoke gate, not a replacement for the Ubuntu/niri/GNOME/Orca
hardware protocol.

The live-status extension must be rerun through this Linux smoke gate before
its results replace the static-candidate evidence. Platform-neutral projection
tests can run on the macOS development host with `cargo test --locked --lib`.
Notification and scheduled-Focus authorities remain later roadmap work; their
projection here does not claim those providers are complete.

The report contract itself is dependency-free and testable on the development
host:

```sh
/usr/bin/python3 -m unittest scripts/test_a4_report.py
```
