# ADR 0021 — Carry one niri patch so third-party windows can minimise

- **Status:** accepted 2026-09-25.
- **Amends:** ADR 0007 (niri stays the compositor, and no longer ships unmodified).
- **Scope:** `packaging/third-party/niri/debian/patches/`, `crates/rmac-compositor-niri/src/minimize.rs`,
  `crates/rmac-dock-runtime/src/consumer.rs`, the ⌘M bind in `packaging/rmac-session/shell.kdl`,
  `shell/bins/rmac-mission-control` (`minimize`), `crates/rmac-ui/src/chrome.rs`.

## Context

niri has no minimized state (ADR 0007). rmac fakes one: the yellow traffic light of an rmac app
records where the window was in `$XDG_RUNTIME_DIR/rmac/parking.json`, takes a thumbnail with
`grim`, and moves the window to the hidden `rmac-parking` workspace. The Dock shows a tile for every
window on that workspace and restores it from the record.

Every other app was left out. Firefox, GTK and Qt draw their own minimise button, which sends
`xdg_toplevel.set_minimized`. niri 26.04 uses smithay's default `minimize_request`, which does
nothing, so the button did nothing. ⌘M did nothing outside rmac apps either. The owner's report:
"when I minimise it doesn't come and sit in the Dock; even Firefox doesn't minimise, nothing does."

Nothing outside niri can see `set_minimized`: it is a request from the client to the compositor.
niri has to report it.

## Decision

1. **niri reports the request and does nothing else.** One Debian quilt patch,
   `0001-ipc-report-minimize-requests.patch` (about 30 lines), overrides `minimize_request` in
   niri's xdg-shell handler. For a mapped toplevel it sends a new event-stream event,
   `WindowMinimizeRequested { id }`, and leaves the window where it is. The event is transient,
   like `ScreenshotCaptured`: it is not part of the replicated state, so a client that connects
   later never sees an old request. `niri msg event-stream` prints it.

2. **rmac does the minimising, through the one shared path.**
   `rmac_compositor_niri::minimize_window_in` records the origin, takes the thumbnail while the
   window is still on screen, then parks the window. Every entry point uses it:
   - the yellow traffic light and ⌘M in rmac apps (`rmac-ui` `chrome.rs`);
   - ⌘M for every app: `Mod+M` spawns `rmac-mission-control minimize`, and the resident service
     minimises the focused window (keyd never translates M, per ADR 0017);
   - a third-party minimise button: the Dock runtime already follows niri's event stream, and it
     runs the same path when `WindowMinimizeRequested` arrives.

   We considered moving the window inside niri, but grim copies the screen, so the thumbnail has to
   be taken before the move. niri would also have had to know the name `rmac-parking` and the
   shell's record format. With the policy in rmac, the tile, thumbnail and restore work the same for
   every app, and the patch stays small enough to rebase onto each niri release.

3. **Restore needs no new code.** A third-party window now has a parking record like any rmac
   window, so the Dock's tile click, ⌘Tab and Show All restore it. A parked window without a
   record, left by a lost race between two writers of the set, goes back to the focused Space.

4. **The parking set follows window lifetime.** On `WindowClosed` the Dock forgets the window and
   deletes its thumbnails. When it reconnects to niri, it prunes records and sweeps thumbnails of
   windows that no longer exist. Both are event-driven, with no polling. Each capture writes
   `thumbnails/<window>-<millis>.png`, because GPUI caches images by path: a second minimise of the
   same window would otherwise show the first picture.

5. **A blank capture is refused.** If the window's Space is not showing, or the overview covers it,
   no thumbnail is taken. A capture that comes back a flat colour (a powered-off or locked screen,
   or a window that blocks capture) is thrown away, and the Dock shows the app icon instead of a
   black tile. Thumbnails are scaled to at most 320 px.

## Consequences

- Lulo's niri is `26.04+lulo1-2`, one Debian revision above the unmodified build. The orig and
  vendor tarballs are unchanged: the vendor tarball's mtime now comes from the first changelog entry
  of the upstream version (`third_party_packages.py vendor-epoch`). `rmac-session`'s floor becomes
  `niri (>= 26.04+lulo1-2)` automatically, because it is read from the pins.
- On an unpatched niri the event never arrives: third-party minimise buttons stay inert, while ⌘M
  and rmac's own buttons still work.
- A client that deserialises niri events into a closed enum (the `niri-ipc` crate from an older
  niri) fails on the new variant. rmac matches on event names and keeps unknown events. Waybar and
  similar tools do the same.
- X11 apps under xwayland-satellite 0.8.x still cannot minimise: satellite does not forward
  `WM_CHANGE_STATE`/`IconicState` to `set_minimized`. That is a separate satellite change
  (docs/parity.md WIN-09).
- The foreign-toplevel (`zwlr_foreign_toplevel_handle_v1.set_minimized`) request is still ignored,
  since no Lulo component sends it.
- Rebasing onto a new niri: re-apply the patch, run `cargo test -p niri-ipc`
  (`window_minimize_requested_wire_format`), and check the event in a nested niri.
