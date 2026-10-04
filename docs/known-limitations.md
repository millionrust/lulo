# Known limitations

Lulo OS is under active development and is not yet a supported daily-driver
release. The application and service domains are broad, but many final claims
require the selected Linux UI framework, real niri layer surfaces, packaged
Ubuntu execution, accessibility evidence, and the H8 hardware matrix.

## Release blockers

- The Linux layer-shell path and GPUI's accessibility semantics are no longer
  an open framework question: `crates/gpui_linux` is vendored and patched
  in-tree (ADR 0013), and Terminal, Notes, and Files' content surfaces now
  publish real AT-SPI roles/text/caret (ACC-01/02/03). The one confirmed
  remaining gap is upstream, not a framework decision: the pinned
  `accesskit_unix`/`accesskit_atspi_common` versions implement no
  `EditableText` interface at all, so Spotlight, Terminal, Notes, and Files'
  search/rename fields cannot be **typed into** by assistive technology or a
  keyboard-injector script (real keyboard/pointer typing is unaffected). See
  "Accessibility limits" below. A formal, owner-run Orca audit at 200% text
  scaling (journey 9) has still not happened — only the owner can enable Orca.
- The signed APT repository, clean native install, upgrade, rollback, and
  uninstall evidence are not complete; this Beta ships `.deb` files you
  install and later remove by hand (see "How to go back" in the release
  notes). Signed, automatic in-place updates over APT are not part of this
  release.
- Two owner-reported desktop/Dock bugs reproduce live but not in the nested
  test compositor, so they remain open and unverified-fixed: desktop icons
  can stay hidden until a click after login (parity row DESK-12), and the
  Dock's Bin icon can vanish and the Dock itself lag/disappear for a moment
  after a delete while its service keeps running (DOCK-27). Both are narrowed
  to "live session only" — if you hit either, a relaunch of the affected
  surface (or `niri msg action` as documented in Troubleshooting) recovers it.
- Critical visual references, all Orca observations, performance traces,
  chaos/soak runs, and the security review still need native candidate
  evidence. No hardware station is yet certified for Alpha, Beta, or 1.0, and
  this Beta has only been exercised on Intel graphics — **no NVIDIA hardware
  has been tested**; treat NVIDIA/proprietary-driver systems as unverified.
  The owner decided on 2026-10-04 to ship Beta 1 without an NVIDIA test,
  because no NVIDIA machine is available; NVIDIA results are a later-Beta item.
- **Beta 1 is tested on one hardware class only.** The H8 Beta tier names
  three stations. The owner has neither an AMD desktop nor an NVIDIA desktop,
  so both are waived for Beta 1 (decision of 2026-10-04). The security-review
  and Beta-candidate verifiers carry the waiver as
  `owner-2026-10-04-beta1-without-amd-nvidia-desktops` and print it. The
  waiver removes only those two station runs and cohort quotas: every check
  must still pass, and the Intel reference laptop and the disposable-install
  station must still run. Treat AMD desktop and NVIDIA systems as untested.

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
  A 2026-09-26 nested-niri virtual-keyboard run passed 8/8 menu-bar
  focus/navigation checks, including AT-SPI focus and visible screenshots.
  Letter typeahead, Return activation, complete keyboard-only journeys,
  and the owner-only Orca pass remain unverified.
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

- Clicking the Dock while a menu (the app/system menu, a status menu,
  Control Center, or the Notification Center panel) is open doesn't close
  that menu yet. Clicking the wallpaper, another window, pressing Escape, or
  switching to a different menu title all close/switch it correctly — only
  a Dock click doesn't. A fix is in progress (parity row `MENU-15`).
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

## Known issues: security findings accepted for Beta

One Low finding from the
[security review](security-review-0.9.0-beta.1.md) is open and accepted for
Beta with a mitigation. It still counts against the security gate.

- **SR-18, release build inputs.** Release containers, the rustup installer,
  and the `cargo-cyclonedx` source archive now have content pins in source.
  Rust toolchain artifacts are still selected by version, and the changed
  workflow has not had a native release run. Actions are pinned by commit,
  Rust dependencies are locked and checked by cargo-deny, and every package
  carries a provenance attestation that `install.sh --from-release` verifies.
- **Automatic updates on older installs (SR-29, fixed).** Builds before the
  fix, such as `8ba31b82`, schedule automatic updates without first checking
  whether they remove a package. Current packages simulate first and never
  schedule a removal; this was tested against the real PackageKit. On an older
  install, turn off Automatic Updates and use Update Now until you update.
- **Builds from before the SR-15 and SR-38 fixes.** Older packages embed
  the builder's checkout path in four shell binaries (SR-15). They also keep
  showing banners while the screen is locked, so with the screen reader on,
  a notification's text may be read aloud (SR-38). Current packages fix both.

`install.sh --from-release` now needs `gh` (and `gh auth login`) to verify
who built the packages. Without it the install stops; `--allow-unattested`
installs on the release's checksums alone.

## Compatibility limits

Ubuntu 26.04 and niri are the selected reference environment. Other
distributions, compositors, desktop portals, GPU drivers, architectures,
filesystems, input methods, and devices are unverified unless named in
[Hardware support](hardware-support.md).

Use only synthetic test data and keep Ubuntu/GNOME installed as the recovery
session. Check [Release notes](release-notes.md) and
[Troubleshooting](troubleshooting.md) before each test cycle.
