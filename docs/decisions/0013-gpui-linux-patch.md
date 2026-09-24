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

### Idle frame loop (amended 2026-09-24)

The first idle fix stopped requesting a frame callback once a frame drew
nothing, but then re-checked the window on a timer that backed off from 16 ms
to 250 ms, so every visible surface still woke about 4 times a second. GPUI at
this revision has no way to tell the platform that a window became dirty
(upstream `gpui-pre` added a platform frame waker for that), so the frame loop
now parks instead:

- While frames draw, the window asks for the next vblank as before; after two
  frames that draw nothing and with no callback pending, it is *parked*.
- A parked window is re-checked only at the end of an event-loop iteration
  (`WaylandClient::run`). GPUI marks a window dirty only on the main thread,
  inside a task, a timer or a platform callback, and each of those wakes the
  event loop first, so no change is missed while an idle process never wakes.
- A check that finds nothing dirty draws and commits nothing, so the
  compositor is not woken either. Input still runs a frame at once
  (`wake_frame`), which also keeps the kinetic-scroll momentum ticks drawing.
- Known gap: a `Window::on_next_frame` callback queued by a frame that drew
  nothing waits for the next event-loop wake-up rather than the next vblank.

### AT-SPI registration (amended 2026-09-24)

Zed's manifest at the pinned revision asks for `accesskit_unix` 0.21, which
registers a window with the AT-SPI registry only while
`org.a11y.Status.ScreenReaderEnabled` is true. The Lulo session reports
`IsEnabled` without a screen reader running, so no rmac surface appeared
under the registry root and AT-SPI clients (Orca, Accerciser, pyatspi) saw
none of Lulo. `accesskit_unix` 0.22 watches `IsEnabled`, which GTK and Qt
honour, and keeps the same public API over `accesskit` 0.24, so the vendored
manifest now requires 0.22.1. `scripts/test_accesskit_activation.py` fails if
either lockfile falls back to an older release.

### AT-SPI application name and toolkit metadata (amendment 2026-09-24)

Once `accesskit_unix` 0.22.1 started registering every window with AT-SPI
(previous amendment), each app's node under the registry root spoke as
`rmac-calculator`, `rmac-text-editor`, and so on — the executable's file
name — instead of a user-visible name like Calculator, the way VoiceOver
would say it on the Mac.

Tracing the exact API (`~/.cargo/registry/src/*/accesskit_unix-0.22.1` and
`accesskit_atspi_common-0.19.1`, cross-checked against the latest published
`accesskit_unix` 0.23.0 via docs.rs) shows this AT-SPI `Application.Name`
comes from `accesskit_atspi_common::AppContext.name`, which
`accesskit_unix::context::get_or_init_app_context` sets exactly once, from a
process-global `OnceLock`, by calling a private `app_name()` helper that
returns `std::env::current_exe()`'s file name. `AppContext` has no public
setter for `name`, `accesskit_unix::Adapter::new` takes only the three
AccessKit handlers (no app-name argument in any published version, including
0.23.0), and `AppContext::new` itself is only ever called from inside
`accesskit_unix`. No amount of `accesskit::Tree`/`TreeUpdate` content changes
what `AppContext.name` reports, because it is derived once, independently of
the tree, before the first frame. **There is no hook in application code —
gpui_linux included — that can change this property without forking
`accesskit_unix`.** That fork is out of scope here: it is a much larger
change than one accessibility ticket warrants, and this machine's rule against
uncontrolled Cargo rebuilds makes an unvalidated fork of an async D-Bus crate
too risky to land blind.

What gpui_linux *can* do, and now does (`shell/compat/gpui_linux/src/linux/a11y.rs`,
called from both backends' `a11y_tree_update`): fill in the separate, genuinely
reachable `accesskit::Tree.toolkit_name`/`toolkit_version` fields, which
upstream GPUI's `crates/gpui/src/window/a11y.rs` leaves `None`. Those back
AT-SPI's `Application.ToolkitName`/`Application.Version` properties, distinct
from `Application.Name`; without this, `accesskit_consumer` reports the
generic fallback `"AccessKit"` with no version for every rmac window. rmac
windows now report `gpui_linux` and this crate's own version there.

Each app's *window*-level accessible name (`Role::Window`, what a screen
reader speaks once focus reaches the window rather than the top-level
application object) was already correct: `rmac_ui::window_options_for_app`
sets the GPUI window title from `rmac_apps::identity::window_title(app_id)`
("Calculator", "Text Editor", …), and shell surfaces set their own accessible
names directly ("top bar", "Dock"), which is why
`shell/scripts/assert_*_accessibility.py` continues to pass unchanged. The
remaining gap is specifically the AT-SPI application object's `Name`.

Follow-up, if this is worth the cost later: fork `accesskit_unix` (and its
`accesskit_atspi_common` dependency) the way `gpui_linux` is vendored here,
adding a real constructor argument or setter for the application name, then
thread each app's `rmac_apps::identity::window_title` through it.

## Consequences

- A GPUI bump now also means re-importing `gpui_linux` and re-applying the
  rmac commits (see `git log -- shell/compat/gpui_linux`).
- No GPL code is involved: GPUI is Apache-2.0.
