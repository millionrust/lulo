# Current-upstream GPUI comparison spike

- Date: 2026-07-10
- Upstream: Zed/GPUI revision
  `76c93968da5b8b8809bdd72e4ad9e7d0e946bad0`
- Experiment: `experiments/gpui-upstream-lab`
- Product workspace: unchanged on GPUI 0.2.2 and Rust 1.94.1
- Experiment toolchain: Rust 1.95.0, matching the pinned upstream revision

## Why this is separate

The released GPUI 0.2.2 platform lab can exercise ordinary windows and desktop
input integrations, but it has no exposed accessibility-tree or layer-shell
API. Current upstream has both. The experiment is excluded from the root Cargo
workspace so evaluating those APIs cannot silently migrate the seven product
applications or their transitive dependency graph.

Both the Git revision and the experiment's resolved `Cargo.lock` are committed.
`target/` remains ignored because it is generated build output.
The regular product checks still use the stable workspace; a separate Linux CI
job compiles and smoke-tests both upstream probes with the Wayland feature and
Rust 1.95.0.

## Implemented probes

### Accessibility

`a11y` creates a semantic application and heading, a focusable spin button with
numeric state and AccessKit increment/decrement actions, and a focusable switch
with an exposed toggled state. Tab and Shift-Tab explicitly move focus.

This is intentionally small: if these primitives do not reach Orca correctly,
building the product component library on top of them would be premature.

### Layer shell

`layer-shell` creates a top-layer Wayland surface anchored to the top, left, and
right edges. It requests a 40-logical-pixel exclusive zone and no keyboard
focus. On non-Linux platforms or without the `wayland` feature, it exits with a
clear unsupported-platform message.

## Evidence collected on macOS

Environment: Apple arm64, macOS development host.

| Check | Result | Evidence |
|---|---|---|
| Pinned upstream dependency resolution | Pass | Lockfile resolves Zed revision `76c93968` |
| `cargo check --bin a11y` | Pass | Clean first build with Rust 1.95.0 |
| `cargo check --bins` | Pass | Both host-valid binary paths compile |
| `cargo clippy --bins -- -D warnings` | Pass | No warnings in the experiment |
| Accessibility window launch | Pass | Window launched and remained responsive |
| Non-Linux layer-shell guard | Pass | Exited with status 2 and guidance |
| Accessibility reaches Orca | Not testable here | Requires Linux AT-SPI/Orca runtime |
| Layer-shell protocol behavior | Not testable here | Requires Linux Wayland compositor |

The macOS result proves that current upstream's application split and semantic
API can be integrated in a bounded crate. It does not prove Linux accessibility
or shell behavior.

## Linux compile-only evidence

On 2026-07-10, the committed experiment was mounted read-only into a clean
ARM64 Debian Bookworm container using Rust 1.95.0. The container installed the
same GPUI native development libraries as the Linux CI job and ran:

```sh
cargo fmt --all -- --check
cargo clippy --locked --bins --features wayland -- -D warnings
```

Both commands passed. This type-checks the Linux-only layer-shell module and
the AccessKit/AT-SPI, Wayland, and Vulkan dependency paths.

## Automated Linux runtime evidence

On 2026-07-10, `scripts/nested-wayland-smoke.sh` also passed in an ARM64 Debian
Bookworm container with Sway 1.7. It used wlroots' headless backend and Pixman
renderer for the compositor, Mesa lavapipe for GPUI's Vulkan renderer, and an
isolated D-Bus/AT-SPI session.

| Check | Result | Evidence |
|---|---|---|
| Wayland layer-shell availability | Pass | `wayland-info` advertised `zwlr_layer_shell_v1` version 4 |
| GPUI rendering | Pass | Both probes wrote their opt-in marker after the first completed frame |
| Layer-shell placement | Pass | Sway mapped the 640x40 top-layer surface across the top edge |
| Exclusive zone | Pass | Sway IPC reported the normal workspace at logical `y = 40` |
| Semantic tree | Pass | AT-SPI exposed the heading, spin button, and toggle button with their expected names |
| Assistive actions and state | Pass | AT-SPI click incremented the numeric value from 0 to 1 and changed the switch to `pressed` |
| Process health | Pass | Both probes remained alive after all assertions |

The same script now runs in the dedicated Ubuntu CI job after strict Wayland
Clippy. This is a deterministic protocol and accessibility smoke gate. It does
not exercise Orca speech output, physical GPU drivers, GNOME or niri behavior,
mixed-scale or multi-display layouts, fullscreen interactions, keyboard focus,
or the required soak duration; those remain part of the manual protocol below.

## Ubuntu 26.04 runtime protocol

Run on both GNOME Wayland and niri where specified. Capture the GPUI lab commit,
kernel, compositor version, session type, GPU and driver, monitor layouts and
scale factors, and Orca version with every result.

1. From `experiments/gpui-upstream-lab`, run
   `cargo check --features wayland --bins`.
2. Launch `cargo run --features wayland --bin a11y` with Orca active.
3. Confirm Orca announces the application, heading, spin button, counter value,
   switch label, and switch state without duplicated or missing nodes.
4. Confirm Tab and Shift-Tab traverse each interactive element once and leave no
   unreachable focus target.
5. Invoke increment and decrement through Orca, not the pointer, and confirm the
   announced numeric value changes and never goes below zero.
6. Toggle the switch with keyboard/Orca action and confirm the announced state
   changes. Pointer-only activation is a failure.
7. Test at 100%, 125%, 150%, and 200%, including moving the window between two
   displays with different scales. Record clipping, focus, and stale-tree bugs.
8. Under niri, launch `cargo run --features wayland --bin layer-shell` and verify
   that it spans the active output's top edge and reserves exactly 40 logical
   pixels without taking keyboard focus.
9. Change outputs and scales, then enter and leave fullscreen on another window.
   Compare behavior with the niri layer-shell rules referenced by `PLAN_V2.md`.
10. Repeat interaction for 30 minutes, then run the complete four-hour soak
    required by ADR 0001 before approving a migration.

## Decision status

**Automated Linux smoke passed; manual acceptance is pending.** Do not migrate
the product workspace yet. The released GPUI 0.2.2 API gap is plausibly resolved
upstream and the basic Linux protocol paths now work, but the decisive Orca,
GNOME/niri, hardware, scaling, focus, fullscreen, and soak gates remain open.
