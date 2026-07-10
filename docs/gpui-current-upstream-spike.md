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
job compiles both upstream probes with the Wayland feature and Rust 1.95.0.

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

**Pending Linux runtime evidence.** Do not migrate the product workspace yet.
The API gap seen in released GPUI 0.2.2 is plausibly resolved upstream, and the
integration is small enough to test, but the decisive Orca, Wayland, scaling,
focus, fullscreen, and soak gates remain open.
