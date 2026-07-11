# Component gallery

> Status: deterministic gallery and contract implemented; native Linux visual,
> input, and Orca evidence remains part of the A1–A4 hardware gate.

`rmac-component-gallery` is the executable visual contract for the shared rmac
design system. It renders every state declared by `rmac_ui::gallery` at logical
100%, 150%, and 200% preview scales and follows the live rmac appearance
snapshot. The gallery is the reference used while B7 replaces app-local
controls with shared components.

Run it from the repository root:

```sh
cargo run --locked -p rmac-component-gallery
```

Use `1`, `2`, and `3` to select 100%, 150%, and 200%. `Ctrl+-` / `Ctrl+=` and
`Command+-` / `Command+=` step between scales. The chosen preview is local to
the process and never changes the system or persisted rmac appearance.

## Reference inventory

The authoritative inventory is `rmac_ui::gallery::COMPONENT_SPECS`. It covers:

- Button, Toggle, Slider, TextField, and SearchField;
- List, Table, Tree, and Tabs;
- Dialog, Alert, ContextMenu, and Tooltip;
- Progress, EmptyState, and Toast.

There are 80 state specimens. Contract tests require each B7 component exactly
once, reject duplicate states, pin the three required preview scales, and
require keyboard/focus plus disabled or unavailable coverage for interactive
controls. Adding or removing a state requires updating the contract and the
renderer together.

Current B7 implementation status: Button, Toggle, Slider, TextField, and
SearchField are exported by `rmac-ui`. Dialog buttons and Activity Monitor
actions consume the shared Button; all System Settings switches and sliders use
the shared Toggle and Slider boundary; Activity Monitor and System Settings use
the shared SearchField. The remaining inventory stays a contract until its
focused B7 migration lands.

## Visual-reference protocol

The executable previews are deterministic logical references. They answer
whether tokens, state differentiation, spacing, and text wrapping remain
coherent as dimensions increase. They are not evidence that a compositor,
renderer, or assistive technology behaves correctly at native output scale.

On the Linux reference PC, capture the complete gallery in light, dark, and
increased-contrast modes at native 100%, 150%, and 200% output scale. For each
combination:

1. launch a fresh gallery process and confirm the live appearance is correct;
2. select the matching logical preview with `1`, `2`, or `3`;
3. traverse the scale selector and scroll surface using only the keyboard;
4. inspect focus visibility, clipping, wrapping, hit targets, and state contrast;
5. run the available accessibility-tree/Orca probe after the A4 framework gate;
6. store reviewed raw captures under the ignored Linux evidence directory,
   never in the repository by default.

Use filenames of the form:

```text
component-gallery-<scheme>-<contrast>-native-<scale>-preview-<scale>.png
```

Record pass/fail and the exact compositor, GPU, output scale, font set, theme
snapshot, and commit in the Linux evidence manifest. A simulated preview must
never be reported as native scaling evidence.

## Semantic boundary

Pinned GPUI 0.2.2 does not expose the programmatic accessibility tree required
for semantic role/name/state/action assertions. The contract therefore tests
the semantic inventory that rmac owns, while real semantic assertions remain
blocked on the A4 upstream framework report. This is an explicit gate, not a
waiver. B7 controls must consume the declared keyboard journeys and expose the
corresponding semantics once the selected framework path supports them.
