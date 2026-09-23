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

- The Dock's layer-shell surface runs with `keyboard_interactive: false` (no
  compositor surface ever receives keyboard focus there), so there is no
  keyboard-only path into it yet -- macOS reaches the Dock with Control-F3,
  then arrow keys and Return. Making the Dock keyboard-reachable needs a
  layer-shell keyboard-interactivity mode switch, a new global shortcut, an
  arrow-key selection state machine, a visible focus ring, and an Escape
  route back to the previous focus: a cross-crate feature, not a small
  follow-up to the AT-SPI Click-action fix in the Dock and Spotlight
  (todo.md "Accessibility gates").
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
