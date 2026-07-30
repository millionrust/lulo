# rmac

rmac is a native Rust desktop suite bringing macOS-like ergonomics to a
Linux/Wayland desktop: a coherent top bar, Dock, launcher, notifications,
Quick Settings, System Settings, and seven focused first-party applications.
Linux services remain authoritative for hardware, authorization, packages,
login, and files.

## Status

- The application, shell-domain, Settings-service, packaging, and release-gate
  implementations are extensive, but rmac is not yet a supported release.
- Ubuntu 26.04 with niri is the reference target; stock Ubuntu/GNOME remains
  the mandatory recovery session.
- Native hardware, final shell presentation, accessibility, visual,
  performance, soak, security, signing, install, update, and rollback evidence
  remains open. See [Known limitations](docs/known-limitations.md).

## Applications

| Application | Package | Run command |
|---|---|---|
| System Monitor | `rmac-activity-monitor` | `cargo run -p rmac-activity-monitor` |
| App Drawer | `rmac-app-drawer` | `cargo run -p rmac-app-drawer` |
| Launcher / Spotlight | `rmac-launcher-app` | `cargo run -p rmac-launcher-app -- --show` |
| Files | `rmac-finder` | `cargo run -p rmac-finder` |
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
- `rmac-focus` — real Focus modes, allow-lists, schedules, temporary activation, and delivery enforcement.
- `rmac-focus-store` — private versioned Focus preferences/manual state with last-good recovery.
- `rmac-focus-runtime` — persisted Focus orchestration, local clock sampling, enforcement, and shell projection.
- `rmac-focus-linux` — single-writer Focus D-Bus authority, typed clients, and timedate/logind wake hints.
- `rmac-focus-settings` — validated whole-configuration edits used by the real Focus Settings pane.
- `rmac-storage` — atomic filesystem writes, durable cleanup, and typed failures.
- `rmac-apps` — macOS bundle and Linux desktop-entry discovery and launching.
- `rmac-launcher` — private, cancellable cross-provider launcher ranking and actions.
- `rmac-launcher-providers` — local app, Settings, file/recents, and calculator results.
- `rmac-launcher-runtime` — concurrent provider dispatch and truthful overlay lifecycle.
- `rmac-launcher-system` — private-safe launcher activation and desktop portal handoff.
- `rmac-dock` — pinned/running app grouping, output scope, and activation policy.
- `rmac-dock-runtime` — coherent live catalog, settings, niri, and hotplug state.
- `rmac-dock-system` — safe launch, niri window actions, and durable Dock pins.
- `rmac-bluetooth` — BlueZ/macOS Bluetooth state, live discovery, secure pairing,
  trust, and device control.
- `rmac-portal` — Linux desktop-portal open/reveal operations with platform fallbacks.
- `rmac-places` — safe XDG Downloads resolution and typed Trash state.
- `rmac-places-system` — filesystem/portal places and confirmed multi-trash operations.
- `rmac-search` — cancellable, exclusion-aware Spotlight/filesystem/recent-document search.
- `rmac-mounts` — macOS and Linux mounted-volume discovery and unmounting.
- `rmac-network` — live/reconnecting NetworkManager/macOS Wi-Fi state, scanning, radio control, exact activation/forgetting, Known Networks, and one-shot WPA Personal/SAE secret delivery.
- `rmac-notifications` — bounded notification validation, replacement, policy, actions, and history.
- `rmac-notifications-linux` — shared notification service, Focus-aware admission, history runtime, and wire decoder.
- `rmac-notifications-runtime` — live banner placement, motion, focus, and service-command orchestration.
- `rmac-notifications-store` — private crash-safe Center history, grouping, unread state, and app policy.
- `rmac-shell-status` — redraw-aware focused-context and shell-indicator projection.
- `rmac-shell-status-linux` — coalesced D-Bus and PipeWire status refresh events.
- `rmac-shell-runtime` — live top-bar/Quick Settings state, health, and last-known-good values.
- `rmac-shortcuts` — portal/niri global shortcuts plus the fail-closed supervised session-lock boundary.
- `rmac-quick-settings` — truthful transactions and keyboard-safe popover lifecycle.
- `rmac-quick-settings-system` — typed quick-settings mutations and authority rereads.
- `rmac-thumbnails` — XDG-aware, invalidation-safe image thumbnail generation.
- `rmac-wallpaper` — per-output source planning and exact wallpaper fit geometry.
- `rmac-wallpaper-image` — bounded procedural/file rasterization and shared LRU cache.
- `rmac-wallpaper-portal` — authenticated, cancellable Wallpaper backend requests and durable confirmed imports.
- `rmac-wallpaper-runtime` — live niri/settings orchestration with off-render resolution.
- `rmac-wallpaper-system` — bounded, magic-checked local wallpaper file authority.

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

- [`docs/user-guide.md`](docs/user-guide.md) — everyday desktop and application guide.
- [`docs/install.md`](docs/install.md) — honest contributor/test installation boundary.
- [`docs/hardware-support.md`](docs/hardware-support.md) — intended hardware matrix and current claims.
- [`docs/settings-guide.md`](docs/settings-guide.md) — user-facing System Settings guide.
- [`docs/shortcuts.md`](docs/shortcuts.md) — global and application keyboard conventions.
- [`docs/privacy.md`](docs/privacy.md) — local data, permissions, credentials, and diagnostics.
- [`docs/troubleshooting.md`](docs/troubleshooting.md) — recovery, safe mode, logs, and issue reports.
- [`docs/update-and-remove.md`](docs/update-and-remove.md) — updates, rollback, uninstall, and data retention.
- [`docs/release-notes.md`](docs/release-notes.md) — current unreleased product status.
- [`docs/alpha-contributor-build.md`](docs/alpha-contributor-build.md) — contributor Alpha publish and issue-intake boundary.
- [`docs/beta-cohort.md`](docs/beta-cohort.md) — invited daily-driver Beta safety and cohort boundary.
- [`docs/one-dot-zero-candidate.md`](docs/one-dot-zero-candidate.md) — two-build full-matrix 1.0 promotion boundary.
- [`docs/release-contracts.md`](docs/release-contracts.md) — fast build-free integrity gate for release definitions.
- [`docs/linux-foundation-report.md`](docs/linux-foundation-report.md) — privacy-safe A1–A4 handoff into the framework decision.
- [`docs/keyring-packaging.md`](docs/keyring-packaging.md) — reproducible package-managed APT trust anchor and source offer.
- [`SECURITY.md`](SECURITY.md) — private vulnerability reporting policy.
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
- [`docs/wallpaper.md`](docs/wallpaper.md) — wallpaper authority, sources, geometry, decoding, and runtime contract.
- [`docs/decisions/0003-wallpaper-portal-authority.md`](docs/decisions/0003-wallpaper-portal-authority.md) — session versus XDG portal wallpaper ownership.
- [`docs/decisions/0004-secure-lock-boundary.md`](docs/decisions/0004-secure-lock-boundary.md) — compositor, PAM, logind, and crash-recovery ownership for locking.
- [`PLAN.md`](PLAN.md) and [`PARITY.md`](PARITY.md) — prototype history.

## License

MIT. See [`LICENSE`](LICENSE). Third-party dependencies and assets retain their
respective licenses.
