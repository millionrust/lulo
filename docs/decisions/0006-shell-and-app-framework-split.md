# ADR 0006: split the shell onto a pinned upstream GPUI revision and keep apps on 0.2.2

- Status: accepted
- Date: 2026-09-17
- Owners: rmac maintainers
- Amends: ADR 0001, ADR 0002
- Review cadence: at most monthly, after the Phase 8 shell smoke passes

## Context

macOS-inspired shell surfaces (menu bar, Dock, OSD, Spotlight, Control Center,
Notification Center, banners, Apps, app switcher, screenshot overlay, Force
Quit, logout/shutdown dialog) must behave like compositor overlays: they must
never appear in window MRU or Alt-Tab lists, must open on the focused output,
must stack above full-screen windows, and must dismiss on click-away. On niri
only `wlr-layer-shell` guarantees those properties.

The product line pins GPUI `=0.2.2` and `gpui-component =0.5.1` (ADR 0002).
The 0.2.2 window backend has no layer-shell API and no programmatic
accessibility tree, so shell surfaces built on it are ordinary `xdg` windows:
they enter niri's window lists and switchers, cannot reliably target the focused
output or stack above full-screen windows, and cannot expose AT-SPI semantics.

`shell` already pins Zed revision
`76c93968da5b8b8809bdd72e4ad9e7d0e946bad0` (with `gpui_platform` and the
`wayland` feature). It has compiled probes for layer-shell, accessibility, and
the four visible shell candidates (`wallpaper`, `top-bar`, `dock`, `osd`). The
ADR 0001 Linux gate on this line is still incomplete, but the layer-shell probe
demonstrates the behavior the product requires, while the 0.2.2 line cannot.

Migrating the whole product — including its `gpui-component`-based applications
— onto the unreleased revision would combine framework churn with the Linux
application port and make failures hard to isolate. Keeping shell surfaces as
`xdg` windows, however, cannot meet the overlay contract at all.

## Decision

1. All **shell surfaces** run on the pinned upstream GPUI revision
   `76c93968da5b8b8809bdd72e4ad9e7d0e946bad0` as **`wlr-layer-shell`** surfaces.
   They live in one promoted workspace, `shell/`, alongside the shared
   `rmac-shell-ui` and `rmac-shell-layer` crates. Shell surfaces are: wallpaper,
   menu bar, Dock, OSD, Spotlight, Control Center, Notification Center,
   banners, Apps, app switcher, screenshot overlay, Force Quit, and the
   logout/shutdown dialog.
2. **Applications** (Files, System Settings, Terminal, Notes, Text Editor,
   System Monitor) stay on GPUI `=0.2.2` + `gpui-component =0.5.1` until the
   Phase 10.3 migration. Domain, runtime, and system crates remain GPUI-free so
   each host stays thin and a future migration touches only UI code.
3. Shell hosts consume framework-neutral rmac runtime/model crates; the
   `SHIPPING_SHELL_SOURCES` mapping in `scripts/linux/native_package_contract.py`
   remains part of the release ABI.
4. The shell graph keeps its own `Cargo.lock`, `rust-toolchain.toml`, and
   `deny.toml`. It is excluded from the product workspace and built only on the
   Ubuntu reference PC.
5. ADR 0002's product-line pins continue to govern the application workspace
   unchanged.

## Upgrade policy

- The shell revision is a full immutable Git revision. It may be bumped **at
  most monthly**, and only in a dedicated change after the Phase 8 shell smoke
  passes on the reference PC: layer-shell placement and focus, AT-SPI output,
  fullscreen and output hotplug behavior, idle CPU, and a bounded interaction
  soak.
- A bump records the upstream comparison, the resolved dependency/lockfile
  diff, the macOS smoke result, and the Ubuntu compile and runtime result.
- Out-of-cycle bumps are allowed only for a relevant security advisory or an
  upstream fix for a confirmed product blocker, with the same evidence scaled
  to the affected path.
- `shell/README.md`, ADR 0002, and the installer
  revision checks are updated in the same change.

## Rollback

- The immediately preceding shell revision is
  `07fe8e9bb1484b2771d8a9d80f7fc370cee9c4ac` with its committed
  `shell/Cargo.lock` (currently `shell/Cargo.lock`).
- Rollback reverts the shell workspace, its lockfile, and the installer's
  expected revision as one unit; it must not require user-data conversion
  because shell persistence formats do not change with a framework bump.
- If a shell revision cannot be corrected immediately after a regression in
  crash safety, layer-shell focus/fullscreen, input/IME, or AT-SPI output, the
  previous revision is restored before further feature work.

## Consequences

- Shell overlays gain the compositor behavior macOS users expect, and the
  product no longer ships a convincing experiment separate from the product.
- Two GPUI dependency lines must be maintained and validated deliberately until
  Phase 10.3. This duplication is bounded to UI hosts; domain crates stay
  framework-free.
- Application accessibility on 0.2.2 remains a tracked gap, recorded in
  `docs/known-limitations.md` and used as the Phase 10.3 migration driver. It is
  never claimed as working.
- Shell surfaces require the Ubuntu reference PC for runtime proof; macOS-only
  compilation is not acceptance.

## Evidence

- `shell/` (pinned revision, probes, smoke scripts)
- `docs/decisions/0001-gpui-linux-gate.md`
- `docs/decisions/0002-gpui-version-policy.md`
- `docs/gpui-current-upstream-spike.md`
- `docs/linux-reference-bringup.md`
- `scripts/linux/install-upstream-shell-candidate.sh`
- `scripts/linux/native_package_contract.py` (`SHIPPING_SHELL_SOURCES`)
