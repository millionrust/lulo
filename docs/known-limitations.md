# Known limitations

Lulo OS is under active development and is not yet a supported daily-driver
release. The application and service domains are broad, but many final claims
require the selected Linux UI framework, real niri layer surfaces, packaged
Ubuntu execution, accessibility evidence, and the H8 hardware matrix.

## Release blockers

- The current stable GPUI line does not prove the required Linux accessibility
  semantics or layer-shell path. Final shell rendering and Orca claims remain
  gated by the upstream framework decision.
- The signed APT repository, clean native install, upgrade, rollback, and
  uninstall evidence are not complete. Repository tools are not public
  installers.
- Critical visual references, all Orca observations, performance traces,
  chaos/soak runs, and the security review still need native candidate evidence.
- No hardware station is yet certified for Alpha, Beta, or 1.0.

## Accessibility limits

- The Dock is keyboard-reachable with Control-F3 (niri runs
  `rmac-dock focus`): arrows and Tab move, Return/Space open, Up opens the
  tile's menu, Escape refocuses the previous window. The Dock's own layer
  surface still has no keyboard interactivity -- GPUI cannot change a mapped
  layer surface's interactivity, and re-creating the Dock would reflow every
  window -- so an invisible 1x1 overlay surface with an exclusive keyboard
  holds the keys while the Dock is focused and carries the AccessKit focus.
  Not yet verified on the laptop with Orca. Known gaps: while focused, the
  Dock captures one click anywhere (as its menus do) to end keyboard mode;
  a Dock surface re-created mid-navigation (an app launching changes the
  shelf length) leaves the focus surface until the next key; the focused
  minimized-window tile's look was not captured on the Mac; the name bubble
  still uses the older rmac pill rather than the measured bubble in
  design-lab/dock.html (scene 3); Fn may be needed for F3 on keyboards
  whose top row defaults to media keys.
- Control-F2 (move focus to the menu bar) is implemented: niri's bind
  spawns `rmac-shortcut-dispatch menu-bar-focus` (the same command-endpoint
  mechanism the power key uses) to a `menu-bar-focus` dispatch socket the
  menu bar watches; the bar takes the keyboard through its own invisible
  1x1 overlay surface (`MenuKeyboard`), the same technique ⌃F3 uses for the
  Dock, since the bar's own layer surface only takes keyboard on a click.
  Left/Right moves the highlighted title, Down/Return opens it, Esc backs
  out one level at a time (an open menu closes to the highlight; the
  highlight then leaves), and typing a letter jumps to a title starting
  with it. `MenuKeyboard`'s single accessible node carries AccessKit focus
  for the highlighted title and, once a menu opens, the highlighted row.
  Verified by compiling, `cargo clippy -D warnings`, `niri validate`, and
  new unit tests covering the dispatch socket and the shell.kdl bind;
  **not yet verified live or nested with Orca/AT-SPI** -- doing that needs
  either a visible nested-niri window on the reference laptop's real
  screen (niri has no headless backend to test its own keybindings and
  layer-shell keyboard-interactivity in isolation) or toggling Full
  Keyboard Access on the owner's own Mac to record a behaviour-suite
  scenario, and this pass avoided both rather than disturb the owner's
  live session or its physical Mac's system settings without asking.
  Known scope limit: only the bar's own menu titles (the Lulo/system menu,
  the bold app menu, and the app's declared menus) are reachable this way;
  the status items on the right (Wi-Fi, Battery, the clock) are not part
  of this ⌃F2 pass, matching the brief's own "titles" wording.
- AT-SPI's `EditableText` interface (needed to type into Spotlight's search
  field without a keyboard injector) is not implemented by the pinned
  `accesskit_unix`/`accesskit_atspi_common` versions at all -- confirmed
  against the vendored dependency source, independent of which rmac binary
  is deployed. The field reports a real `TextInput` role with its value
  (readable over AT-SPI's `Text` interface) and already handles AccessKit's
  `SetValue`/`ReplaceSelectedText` actions, ready for when a dependency
  upgrade adds the AT-SPI bridge for them.

## Feature limits

- Open item: the top-bar logo slot still draws the "R" glyph inherited from
  the rmac name. It needs a real Lulo mark before release; that is a design
  task (new artwork), not a text rename, and is intentionally not addressed
  by the rmac-to-Lulo-OS text rename.
- Lulo OS does not clone Apple services, proprietary assets, iCloud, AirDrop,
  AppleCare, Time Machine, or Apple account behavior.
- System Settings exposes only authorities Linux/niri can support without
  inventing state. Some accessibility, display mirroring, per-device input,
  credential, sharing, and hardware-specific controls remain unavailable.
- Portal permission reset is not universal revocation and cannot prove native
  application access or active capture ended.
- Application provenance does not prove that an app is safe, signed,
  sandboxed, updated, or owned by APT.
- Files does not yet browse network shares (SMB/SFTP), mount MTP phones or
  cameras, or drag files into other apps. Ubuntu's own Files (Nautilus) stays
  installed for those jobs, kept out of Spotlight, the App Drawer, and the
  Dock's suggestions so it does not sit next to ours and cause confusion, but
  it is not removed: it is still offered in any file's "Open With" menu, and
  it still opens with its own `nautilus` command. Other GNOME apps we ship a
  first-party equivalent for -- the text editor, calculator, system monitor,
  terminal, image and document viewer, clock, and weather app -- are hidden
  from the same browsing surfaces the same way and reachable the same way.
- Search is local, bounded, exclusion-aware, and not a promise to index every
  file format or location.

## Compatibility limits

Ubuntu 26.04 and niri are the selected reference environment. Other
distributions, compositors, desktop portals, GPU drivers, architectures,
filesystems, input methods, and devices are unverified unless named in
[Hardware support](hardware-support.md).

Use only synthetic test data and keep Ubuntu/GNOME installed as the recovery
session. Check [Release notes](release-notes.md) and
[Troubleshooting](troubleshooting.md) before each test cycle.
