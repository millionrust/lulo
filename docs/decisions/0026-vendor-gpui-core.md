# ADR 0026 — Vendor GPUI core

- Status: accepted
- Date: 2026-10-09
- Follows: ADR 0013 (patched `gpui_linux` and `gpui_wgpu`), ADR 0025
  (patched `gpui_windows`)
- Branch: `op/speed-12`

## Context

System Settings kept only about 71 % of its frames within 16.7 ms during a
live resize (SPEED-12; target 95 %). Speed round 11
(docs/perf/speed-round-11-2026-10-09.md) traced the cost to GPUI core, not to
the Linux backend this repository already carries:

- `Window::bounds_changed` (`crates/gpui/src/window.rs`) calls `refresh()` on
  every resize configure. `refresh()` sets `window.refreshing`, which stays
  true for the whole next draw.
- `ViewElement::prepaint` (`crates/gpui/src/view.rs`) reuses a cached view's
  last prepaint and paint only when `!window.refreshing`. So during a resize
  every cached view re-rendered, laid out and painted again on every frame,
  even views whose own bounds never changed. In Settings that is the
  fixed-width sidebar, which cost about as much as the detail pane (p50
  5.4 ms, p95 25 ms) on every resize frame.

`gpui` is the git-pinned `zed-industries/zed` dependency, so rmac could not
change this without carrying the crate.

## Decision

Vendor `crates/gpui` from zed-industries/zed at
`76c93968da5b8b8809bdd72e4ad9e7d0e946bad0`, the revision both lockfiles
already pinned, into `shell/compat/gpui` (Apache-2.0; `LICENSE-APACHE` kept
beside it).

- **One unmodified import commit.** It holds `src/`, `build.rs`, `resources/`
  (the Windows manifest that `build.rs` embeds) and `README.md` (which
  `src/gpui.rs` includes as its crate docs). Upstream's examples, tests and
  docs are left out.
- **Manifest.** Only `Cargo.toml` is rewritten, as in ADR 0013 and ADR 0025.
  Workspace inheritance is replaced with the versions and features from
  Zed's root manifest at that revision, and Zed's workspace lints are
  dropped. The dev-dependencies are left out, because they need Zed's whole
  workspace. Two further changes:
  - The optional `criterion` and `hdrhistogram` dependencies are dropped, so
    no new crate enters either dependency graph. The `bench` and
    `input-latency-histogram` features remain, with no dependencies, and
    must not be enabled.
  - The `proptest` git dependency names its version (1.10.0), so
    cargo-deny's wildcard check passes now that `gpui` is a path crate.
- **Wiring.** Both workspaces name the path crate as their `gpui` workspace
  dependency, as they already do for `gpui_linux`. A git workspace
  dependency that the patch section then replaces leaves cargo-deny's
  `workspace-dependencies.unused` check unable to match it. Both
  `[patch."https://github.com/zed-industries/zed.git"]` sections also point
  `gpui` at it, so Zed's own crates use the same copy on Linux, macOS and
  Windows: `gpui_platform`, `gpui_macos`, `gpui_linux`, `gpui_wgpu`,
  `gpui_windows`, `gpui_web` and `vendor/gpui-component`.
- **Not a workspace member.** The shell workspace lists `compat/gpui` under
  `exclude`. Otherwise the path crate would become an implicit member. Then
  cargo-deny's `unmaintained = "workspace"` advisory check would flag its
  `ttf-parser` dependency (RUSTSEC-2026-0192, no maintained release), and
  Zed's unexpected `rust_analyzer` cfg would warn. As with the git copy,
  neither is ours to change. The manifest allows unexpected cfgs, as Zed's
  workspace lints do.
- **Lockfiles and packaging.** Each lockfile loses `gpui`'s git `source`
  line and nothing else. The Flatpak
  Cargo sources drop the git copy of `gpui`, as they did for the other path
  crates. `cargo deny check bans licenses sources` passes in the application
  workspace, and `advisories bans sources` in the shell workspace.
- **rmac's changes.** They are separate commits on top, each marked `rmac`
  in the code, so a GPUI bump can re-import and re-apply them
  (`git log -- shell/compat/gpui`).

### Resize reuses cached views

`Window::bounds_changed` no longer refreshes the window for a new size or
position. It marks the window dirty, so the next frame lays the window out
again. A cached view is then reused when all of these hold:

- its own bounds and text style are unchanged, and it is not dirty;
- its content mask is unchanged where the view draws, that is, intersected
  with its bounds. Upstream compared the whole mask, but with no clipping
  parent the mask is the whole window, which every resize changes, so even
  with the refresh gone no view below an unclipped root would have been
  reused;
- and every part of the window's geometry it read while rendering or painting still has the value it saw.

GPUI now tracks those geometry reads. `Window::viewport_size`, `bounds`,
`window_bounds`, `inner_window_bounds`, `is_maximized`, `is_fullscreen` and
`window_decorations` each set one bit in a per-window read mask. A cached
view collects the bits set during its own render, layout, prepaint and
paint, and stores them in its cache key together with those parts' values
then. It is reused only while every part it read still has the same value.
A view that read the viewport size therefore renders again on every resize,
as before, but a view that only asked whether the window is full screen
does not. A reused view sets its bits again, so the views around it record
the dependency as well. This
covers GPUI's own `anchored` element, which fits popovers to the viewport,
and rmac-ui's `window_content_size`.

A new scale factor or a move to another display still refreshes the whole
window, because every cached glyph and image is stale then. Appearance
changes are unchanged: GPUI never refreshed for them itself, and apps that
observe the appearance refresh or notify as before.

The change is in platform-independent code, so Linux, macOS and Windows
behave the same. `RMAC_GPUI_RESIZE_REFRESH=1` brings back upstream's full
refresh, so one build can be measured both ways.

Settings' sidebar view took its master/detail split from `window.bounds()`,
which would have tied it to every resize. It now reads the split the root
view computed earlier in the same frame. A change of split resizes the
sidebar's column, which renders it again anyway.

Known gap: code that reads the window's geometry through anything other than
those methods (for example, a value cached in an entity during an earlier
frame) is not tracked. Such a view keeps its last frame until its bounds
change or it is notified.

## Results

See docs/perf/speed-round-12-2026-10-09.md for the raw reports.

Same binaries with the change on and off (`RMAC_GPUI_RESIZE_REFRESH=1`),
three interleaved runs each, nested niri on the reference laptop:

| Settings live resize | Before | After |
|---|---|---|
| Edge drag, frames within 16.7 ms (target 95 %) | 77.7 / 80.4 / 74.0 % | 97.3 / 93.6 / 96.4 % |
| Edge drag, GPUI render p50 (ms) | 6.8 / 6.9 / 8.4 | 2.9 / 2.7 / 3.4 |
| niri Mod-drag, frames within 16.7 ms | 45.2 / 65.2 / 69.0 % | 83.3 / 91.1 / 83.3 % |

The refresh was not the only reason views missed their cache. Two other
causes turned up with `RMAC_GPUI_CACHE_TRACE=1`, which prints why each
cached view missed its cache:

- the full-window content mask, which a resize changes for every view;
- the vendored gpui-component text input, which dirtied its view from every
  paint. That fix sits in `vendor/gpui-component`, not here.

With all three fixed, the Settings sidebar renders on 13 of 104 frames of an
edge drag, against every frame before.

## Consequences

- A GPUI bump now means re-importing `gpui` as well as `gpui_linux`,
  `gpui_wgpu` and `gpui_windows`, and re-applying the rmac commits on each.
- Every rmac binary now builds GPUI from the tree. A change under
  `shell/compat/gpui` rebuilds every app and surface, so changes there
  should be few and small.
- The resize change is worth offering to Zed once it has run on the owner's
  laptop for a while; the owner decides when anything is published.
- No GPL code is involved: GPUI is Apache-2.0.
