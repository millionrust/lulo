# ADR 0013 — Carry a patched copy of GPUI's Linux backend

- Status: accepted
- Date: 2026-09-23
- Amends: ADR 0006 (shell GPUI pin now lives in `shell/Cargo.lock`)

## Context

Two macOS behaviours are impossible to reach from rmac code because they live
in GPUI's Wayland backend (`gpui_linux`):

1. **Idle wake-ups.** Every `wl_callback::Done` immediately requests another
   frame callback, so each GPUI surface wakes itself and the compositor about
   60 times a second even when nothing changes. On the reference laptop this
   kept niri near 8 % CPU with the desktop idle.
2. **Kinetic scrolling.** `wl_pointer::AxisStop` is ignored and every scroll
   is reported as `TouchPhase::Moved`, so a touchpad scroll stops dead when
   the fingers lift. macOS continues with momentum.
3. **Panicking when no compositor is reachable.** `WaylandClient::new()`
   unwraps `Connection::connect_to_env()`. During logout or a niri session
   switch, a shell surface's `Restart=on-success` unit can respawn the
   process in the brief window after niri exits but before the session's
   units are stopped; the new instance's connect then fails with
   `ConnectError::NoCompositor` and panics with a backtrace, which is what
   showed up as a crash for `rmac-quick-settings`, `rmac-launcher`,
   `rmac-app-drawer`, and `rmac-notification-center-panel`. The panic itself
   is a symptom worth silencing even though the real fix is closing the
   restart race in the session's units (`rmac-session.target` now binds
   directly to `niri.service`, see `crates/rmac-session/units`).

## Decision

Vendor `crates/gpui_linux` from zed-industries/zed at
`76c93968da5b8b8809bdd72e4ad9e7d0e946bad0` into `shell/compat/gpui_linux`
(Apache-2.0; `LICENSE-APACHE` kept beside it) and use it through
`[patch."https://github.com/zed-industries/zed.git"]` in both the application
workspace and the shell workspace.

- The import is one commit with the upstream sources unmodified; only
  `Cargo.toml` is rewritten to replace workspace inheritance with the versions
  from Zed's root manifest at that revision.
- rmac's changes are separate, small commits on top, each explaining the
  behaviour it adds, so a GPUI bump can re-import and re-apply them.
- Both workspaces name GPUI by repository URL only and pin the revision in
  their lockfiles, so the patched crate's own `gpui` dependency resolves to the
  same package in each. `install-upstream-shell-candidate.sh` reads the pin
  from `shell/Cargo.lock`.
- The changes are offered upstream to Zed once proven on the reference PC; the
  owner decides when anything is published.

## Consequences

- A GPUI bump now also means re-importing `gpui_linux` and re-applying the
  rmac commits (see `git log -- shell/compat/gpui_linux`).
- No GPL code is involved: GPUI is Apache-2.0.
