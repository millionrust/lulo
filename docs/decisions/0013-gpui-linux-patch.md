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

### Cross-process drag source (amended 2026-09-24)

macOS lets a Launchpad drag land on the Dock to pin the application. Building
the rmac equivalent (drag an app from Apps, `crates/app-drawer`, onto the
Dock) meant first checking whether `gpui_linux`'s Wayland client can act as a
drag *source* — offering data to another client — not just a drop *target*.

It cannot. `shell/compat/gpui_linux/src/linux/wayland/client.rs` creates a
`wl_data_source` in exactly two places (`write_to_clipboard`,
`write_to_primary`), and both call `wl_data_device.set_selection` immediately
— never `start_drag`. The `Dispatch<wl_data_source::WlDataSource, ()>` impl
handles only `Event::Send` (serving clipboard bytes) and `Event::Cancelled`;
none of the drag-specific events (`DndDropPerformed`, `DndFinished`,
`Action`) are handled anywhere. `PlatformInput::FileDrop` (`client.rs`,
`Event::Enter`/`Motion`/`Leave`/`Drop` on `wl_data_device`) is the *only*
drag-and-drop code path, and it is inbound only, feeding `gpui::ExternalPaths`
into GPUI's `on_drop`. GPUI's own `Div::on_drag`/`on_drag_move` API
(`crates/gpui/src/elements/div.rs`) confirms the same boundary at the
framework level: the dragged payload lives in `AnyDrag` as an `Arc<dyn Any>`
matched by Rust `TypeId`, with no serialization or platform hand-off — it
cannot leave the process it started in, by construction. `crates/finder`'s
`DraggedPaths` drag (list/gallery views) is therefore in-process reordering
and self-drop only, never a real cross-application Wayland drag, matching
what `crates/app-drawer/SPEC.md` already documents about "Show in Folder" as
the honest bridge for the same underlying gap.

Implementing `wl_data_source`/`start_drag` (the option this ADR would
otherwise prefer, since Files and every other rmac surface would gain a real
drag-out for free) is a real Wayland-protocol addition to the vendored
backend — new source/offer state, `Action` negotiation, and interaction with
niri's own drag handling — that needs an interactive test on the reference
laptop to land safely, not something to attempt without hands-on Wayland
verification. It stays open as future work.

For now the Dock keeps an application dragged out of Apps through a command
endpoint instead: a second bounded local `UnixDatagram` socket,
`dock-drag.sock` (`shell/bins/rmac-dock/src/drag_endpoint.rs`, alongside the
existing `dock.sock` `⌃F3` transport, `shell/bins/rmac-dock/src/ipc.rs`).
Apps reports its own window-relative pointer position while a tile drag is
held, and the Dock resolves `Drop` against its catalog exactly like a
`.desktop` file drop (`PinCommand::Pin` then `PinCommand::MoveTo`). This
rests on an **unverified** assumption: that niri keeps delivering pointer
motion to Apps' surface (an implicit button-held grab) even once the pointer
visually crosses into the Dock's on-screen rectangle, the same convention
most Wayland compositors extend to support ordinary same-window click-drag
interactions. Nothing in this repository confirms niri does this across
*different* surfaces; it needs an interactive check on the reference laptop
before the live gap-preview and drop placement can be trusted. If it does not
hold, the fallback is the wl_data_source work above.

### Downloads stack special icon lives in rmac-dock, not gpui_linux

Unrelated to the Wayland backend itself, but recorded here since it touches
the same "Downloads left the default Dock" history (15180aa5): folder/file
stacks (§ folder/file stacks left of the Trash) are a new, separate
`rmac_shell_settings::DockStackEntry`/`rmac_dock::StackPlace` model, not a
reuse of `SpecialItemKind`. `SpecialItemKind` is `Copy` and matched
exhaustively across menu, presentation, accessibility, and dispatch code with
no payload; giving it a `Path(PathBuf)` variant for arbitrary stack folders
would have broken that `Copy` bound and every exhaustive match for a feature
that only needed the *icon* reused. The Downloads stack instead reuses
`BuiltinIcon::Downloads` (`crates/rmac-dock/src/presentation.rs`) by kind, and
a new `BuiltinIcon::Folder` (original artwork,
`crates/rmac-dock/assets/icons/folder.svg`) covers every other stack.

## Consequences

- A GPUI bump now also means re-importing `gpui_linux` and re-applying the
  rmac commits (see `git log -- shell/compat/gpui_linux`).
- No GPL code is involved: GPUI is Apache-2.0.
