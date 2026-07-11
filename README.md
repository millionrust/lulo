# rmac

rmac is a native Rust desktop suite that explores macOS-like ergonomics for a
Linux/Wayland desktop. The repository currently contains seven functional GPUI
applications developed on macOS. Linux product work follows the gated roadmap
in [`PLAN_V2.md`](PLAN_V2.md).

## Status

- The macOS prototypes build and run.
- CI is configured to check both Ubuntu and macOS builds.
- Ubuntu 26.04 runtime behavior is not yet validated; that is Phase 1.
- The dock, top bar, launcher, and full desktop session are planned after the
  Linux foundations and three vertical-slice applications pass their gates.

## Applications

| Application | Package | Run command |
|---|---|---|
| Activity Monitor | `rmac-activity-monitor` | `cargo run -p rmac-activity-monitor` |
| App Drawer | `rmac-app-drawer` | `cargo run -p rmac-app-drawer` |
| Finder | `rmac-finder` | `cargo run -p rmac-finder` |
| Notes | `rmac-notes` | `cargo run -p rmac-notes` |
| System Settings | `rmac-system-settings` | `cargo run -p rmac-system-settings` |
| Terminal | `rmac-terminal` | `cargo run -p rmac-terminal` |
| Text Editor | `rmac-text-editor` | `cargo run -p rmac-text-editor` |

Shared crates:

- `rmac-ui` — semantic light/dark/accessibility tokens, window setup, dialogs, and menus.
- `rmac-appearance` — platform-neutral appearance snapshots, events, reducer, and test fake.
- `rmac-appearance-portal` — read-only Linux Settings portal adapter with live reconnect.
- `rmac-theme` — writable rmac theme preferences, resolution, live file events, and recovery.
- `rmac-editor` — shared multiline editor construction and text helpers.
- `rmac-storage` — atomic filesystem writes, durable cleanup, and typed failures.
- `rmac-apps` — macOS bundle and Linux desktop-entry discovery and launching.
- `rmac-launcher` — private, cancellable cross-provider launcher ranking and actions.
- `rmac-launcher-providers` — local app, Settings, file/recents, and calculator results.
- `rmac-dock` — pinned/running app grouping, output scope, and activation policy.
- `rmac-dock-runtime` — coherent live catalog, settings, niri, and hotplug state.
- `rmac-dock-system` — safe launch, niri window actions, and durable Dock pins.
- `rmac-bluetooth` — BlueZ/macOS Bluetooth state, discovery, and device control.
- `rmac-portal` — Linux desktop-portal open/reveal operations with platform fallbacks.
- `rmac-places` — safe XDG Downloads resolution and typed Trash state.
- `rmac-places-system` — filesystem/portal places and confirmed multi-trash operations.
- `rmac-search` — Spotlight and Linux filesystem/XDG recent-document providers.
- `rmac-mounts` — macOS and Linux mounted-volume discovery and unmounting.
- `rmac-network` — NetworkManager/macOS Wi-Fi state, scanning, and radio control.
- `rmac-shell-status` — redraw-aware focused-context and shell-indicator projection.
- `rmac-shell-status-linux` — coalesced D-Bus and PipeWire status refresh events.
- `rmac-shell-runtime` — live top-bar/Quick Settings state, health, and last-known-good values.
- `rmac-quick-settings` — truthful transactions and keyboard-safe popover lifecycle.
- `rmac-quick-settings-system` — typed quick-settings mutations and authority rereads.
- `rmac-thumbnails` — XDG-aware, invalidation-safe image thumbnail generation.

Phase 1 diagnostic:

```sh
cargo run -p rmac-platform-lab
```

The lab exercises GPUI input, clipboard, file chooser, external file drop,
scrolling, and scaling behavior without coupling the experiment to a product
application. See [`docs/gpui-0.2.2-stable-spike.md`](docs/gpui-0.2.2-stable-spike.md).

The separate current-upstream experiment probes APIs unavailable in the stable
release without changing product dependencies:

```sh
cd experiments/gpui-upstream-lab
cargo run --bin a11y
# Linux/Wayland only:
cargo run --features wayland --bin layer-shell
cargo run --features wayland --bin top-bar
```

See [`docs/gpui-current-upstream-spike.md`](docs/gpui-current-upstream-spike.md)
for the evidence collected so far and the remaining Ubuntu runtime protocol.

## Prerequisites

The repository pins Rust in `rust-toolchain.toml`; `rustup` installs the correct
toolchain automatically.

### macOS

Install Xcode and its command-line tools. GPUI requires the separate Metal
toolchain on current Xcode versions:

```sh
xcodebuild -downloadComponent MetalToolchain
```

### Ubuntu build dependencies

The continuous-integration package list is the source of truth. On Ubuntu:

```sh
sudo apt-get install --yes \
  clang libfontconfig1-dev libfreetype-dev libssl-dev libvulkan-dev \
  libwayland-dev libx11-xcb-dev libxcb1-dev libxcb-render0-dev \
  libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev \
  libxkbcommon-x11-dev
```

## Build and verify

```sh
cargo build --locked --workspace
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
cargo deny --locked --log-level error check
```

Use `--release` when measuring startup, memory, or animation performance:

```sh
cargo run --release -p rmac-activity-monitor
```

Run the repeatable seven-application startup, idle CPU, and RSS baseline with:

```sh
python3 scripts/measure-baseline.py
```

The generated JSON stays under ignored `target/baselines/`. See
[`docs/performance-baseline.md`](docs/performance-baseline.md) for the committed
macOS baseline, method, known failures, and remaining Linux/frame-time evidence.

## Documentation

- [`PLAN_V2.md`](PLAN_V2.md) — current execution roadmap and acceptance gates.
- [`ARCHITECTURE.md`](ARCHITECTURE.md) — current and target architecture.
- [`CONTRIBUTING.md`](CONTRIBUTING.md) — change and verification rules.
- [`docs/phase-0-inventory.md`](docs/phase-0-inventory.md) — starting technical-debt inventory.
- [`docs/performance-baseline.md`](docs/performance-baseline.md) — reproducible startup, idle CPU, and RSS evidence.
- [`docs/dependency-policy.md`](docs/dependency-policy.md) — advisory, license, dependency, and source rules.
- [`docs/linux-reference-bringup.md`](docs/linux-reference-bringup.md) — reference-PC setup, automated evidence, and Phase 1 manual gates.
- [`docs/system-settings-audit.md`](docs/system-settings-audit.md) — real-service status and completion contract for every Settings pane.
- [`docs/decisions/0001-gpui-linux-gate.md`](docs/decisions/0001-gpui-linux-gate.md) — framework migration gate.
- [`docs/decisions/0002-gpui-version-policy.md`](docs/decisions/0002-gpui-version-policy.md) — exact pins, upgrade cadence, promotion, and rollback policy.
- [`docs/gpui-current-upstream-spike.md`](docs/gpui-current-upstream-spike.md) — pinned upstream comparison and pending Linux gates.
- [`PLAN.md`](PLAN.md) and [`PARITY.md`](PARITY.md) — prototype history.

## License

MIT. See [`LICENSE`](LICENSE). Third-party dependencies and assets retain their
respective licenses.
