# Known limitations

rmac is under active development and is not yet a supported daily-driver
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
- Control-F2 (move focus to the menu bar) is not implemented. The top bar's
  surface is on-demand (niri focuses it only on click) and its key handling
  runs only while a menu is open, so it needs the Dock's invisible focus
  surface, a command endpoint, a "title highlighted, no menu open" state
  and its accessible focus -- the same mechanism, but not a small addition.
- AT-SPI's `EditableText` interface (needed to type into Spotlight's search
  field without a keyboard injector) is not implemented by the pinned
  `accesskit_unix`/`accesskit_atspi_common` versions at all -- confirmed
  against the vendored dependency source, independent of which rmac binary
  is deployed. The field reports a real `TextInput` role with its value
  (readable over AT-SPI's `Text` interface) and already handles AccessKit's
  `SetValue`/`ReplaceSelectedText` actions, ready for when a dependency
  upgrade adds the AT-SPI bridge for them.

## Feature limits

- rmac does not clone Apple services, proprietary assets, iCloud, AirDrop,
  AppleCare, Time Machine, or Apple account behavior.
- System Settings exposes only authorities Linux/niri can support without
  inventing state. Some accessibility, display mirroring, per-device input,
  credential, sharing, and hardware-specific controls remain unavailable.
- Portal permission reset is not universal revocation and cannot prove native
  application access or active capture ended.
- Application provenance does not prove that an app is safe, signed,
  sandboxed, updated, or owned by APT.
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
