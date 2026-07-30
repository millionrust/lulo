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
