# ADR 0015 — Build on gpui-kit's gpui-base, with rmac-ui as the design system

- Status: accepted (2026-09-23), migration in progress on `migrate/gpui-kit`
- Amends: ADR 0002 (GPUI version policy), ADR 0006 (shell/app framework split),
  ADR 0013 (patched `gpui_linux`)
- Plan: `PLAN_NEW.md` workstream A

## Context

rmac's apps use `gpui-component =0.5.2` (longbridge, rev `0775df3`) on Zed git
`76c93968`. That library is now `longbridge/gpui-kit`; fixes land only on its
0.6 line, which builds on `gpui-pre =0.3.6`, a crates.io snapshot of Zed
`bcf6582` (1,125 commits ahead). rmac already draws nearly all of its interface
from measured macOS 26 components in `rmac-ui`; `gpui-component`'s styled
widgets remain in 22 crates (about 67 references). gpui-kit splits out
`gpui-base`: unstyled behaviour — focus management and traps, accessibility
roles, text input/editing, virtual lists, motion with reduced-motion support,
and theme tokens — which is exactly the layer rmac should not maintain itself.

## Decision

1. rmac depends on `gpui-kit =0.6.6` with **default features off**, so only
   `gpui-base` (re-exported as `gpui_kit::base`) and GPUI (`gpui_kit::*`,
   `gpui_kit::platform`) are used. `gpui-component`'s styled widgets are not.
2. `rmac-design` holds the tokens and `rmac-ui` is the design system: every
   control, menu, table and icon an app uses comes from `rmac-ui`, assembled on
   `gpui-base` behaviour. Apps never name `gpui` or `gpui-kit` crates directly
   beyond what `rmac-ui` re-exports.
3. GPUI comes from exact crates.io pins. A bump is a dedicated pull request, at
   most monthly, that passes the reference-laptop gate (Orca output, layer-shell
   placement/focus, idle CPU, touchpad momentum, four-hour soak).
4. ADR 0013's vendored backend moves to `gpui-pre-linux 0.3.6` through
   `[patch.crates-io]`. Kinetic scrolling is re-applied (upstream still ignores
   `wl_pointer.axis_stop`); the idle-frame patch is dropped only if the laptop
   shows upstream's parked frame loop keeps idle surfaces quiet.
5. Until the migration lands, no file may start using `gpui_component` or add
   uses: `scripts/check-gpui-component-imports.sh` runs in CI against
   `scripts/gpui-component-baseline.txt`, which may only shrink.

## Migration order

A1 spike (`rmac-ui` core, component gallery, one Text Editor window) → A2
vendored `gpui-pre-linux` with kinetic scrolling → A3 probes → A4 design system
on `gpui-base` → A5 one app per pull request (Calculator … Terminal) → A6 shell
→ A7 remove Zed git and `gpui-component`. If the laptop gate (plan B2) fails,
rmac stays on `76c93968` with ADR 0013's patches.

## Consequences

- rmac follows gpui-kit's release cadence for GPUI, trading control of the exact
  Zed revision for maintained behaviour primitives.
- `gpui-pre` is a snapshot republished by one maintainer, not an official Zed
  release; exact pins and the laptop gate contain that risk.
- `gpui-base` has no icons, context menus or sortable data table; `rmac-ui`
  supplies them (`rmac-icon`, the measured menus, a virtual table).
