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

- `rmac-ui` — visual tokens, window setup, dialogs, and context menus.
- `rmac-editor` — shared multiline editor construction and text helpers.

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
cargo build --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Use `--release` when measuring startup, memory, or animation performance:

```sh
cargo run --release -p rmac-activity-monitor
```

## Documentation

- [`PLAN_V2.md`](PLAN_V2.md) — current execution roadmap and acceptance gates.
- [`ARCHITECTURE.md`](ARCHITECTURE.md) — current and target architecture.
- [`CONTRIBUTING.md`](CONTRIBUTING.md) — change and verification rules.
- [`docs/phase-0-inventory.md`](docs/phase-0-inventory.md) — starting technical-debt inventory.
- [`PLAN.md`](PLAN.md) and [`PARITY.md`](PARITY.md) — prototype history.

## License

MIT. See [`LICENSE`](LICENSE). Third-party dependencies and assets retain their
respective licenses.

