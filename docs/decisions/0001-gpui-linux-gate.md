# ADR 0001: keep GPUI 0.2.2 pinned until the upstream Linux gate passes

- Status: accepted for the stable spike
- Date: 2026-07-10
- Owners: rmac maintainers

## Context

The product applications currently use GPUI 0.2.2 and gpui-component 0.5.1.
The published GPUI version includes Linux Wayland and X11 window backends, but
source/API inspection found no exposed programmatic-accessibility tree API and
no layer-shell API. Both are mandatory for the rmac product plan.

Current upstream GPUI has changed its platform split and is developing
AccessKit-based accessibility. Migrating the entire workspace before runtime
evidence would combine framework churn with the Linux application port and make
failures difficult to isolate.

## Decision

1. Keep product crates pinned to GPUI 0.2.2 during the stable behavior spike.
2. Use `rmac-platform-lab` to record the stable input, clipboard, file dialog,
   file-drop, scaling, GPU, and idle-behavior baseline.
3. Build a separate current-upstream lab before changing workspace dependencies.
4. Do not migrate product applications until the upstream lab passes the gates
   below and the migration diff is reviewed independently.

## Upstream spike gates

- launches under Ubuntu 26.04 on niri and GNOME Wayland;
- Unicode text and the selected Linux IMEs work;
- clipboard, file chooser, and external file drop work;
- fractional scaling and multi-display movement render correctly;
- accessible roles, names, states, focus, text, and actions reach Orca;
- a layer-shell top surface and overlay surface follow expected fullscreen and
  keyboard-focus behavior under niri;
- idle CPU has no unconditional redraw loop;
- no crash in a four-hour interaction soak;
- the smallest representative product migration is bounded and documented.

## Migration decision

After the upstream spike, choose exactly one:

- **Adopt a released successor:** preferred if it passes and supports the
  required APIs.
- **Pin an upstream revision:** allowed only with a documented update cadence
  and a small rmac adapter surface.
- **Keep 0.2.2 for apps and use another shell toolkit temporarily:** allowed if
  accessibility can be delivered for apps through a bounded backport but
  layer-shell remains unavailable.
- **Reconsider GPUI:** required if accessible input/render behavior cannot pass
  without maintaining a large framework fork.

No option may silently waive accessibility or layer-shell requirements.

## Consequences

- Product feature work avoids an unmeasured framework migration.
- The platform lab is intentionally duplicated for the upstream spike.
- CI compilation alone is insufficient; the decision requires real Wayland,
  GPU, IME, and Orca evidence.
- GPUI upgrades remain isolated changes with their own rollback point.

## Evidence

- `docs/gpui-0.2.2-stable-spike.md`
- `crates/platform-lab`
- `docs/gpui-current-upstream-spike.md`
- `shell`
- GPUI 0.2.2 source in the Cargo registry
- current upstream GPUI README and accessibility implementation linked from
  `PLAN_V2.md`
