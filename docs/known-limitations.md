# Known limitations

This page is for **0.9.0 Beta 3**, released as "Lulo 0.9.0 Beta 3
(Ubuntu) + Windows Preview", with the first Lulo OS USB image. Lulo OS on
Ubuntu is the beta; Lulo for Windows is an early preview (see
[Windows preview](#windows-preview)); the USB image has its own section
([Lulo OS USB image](#lulo-os-usb-image)).

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
  "Accessibility limits" below. The automated Orca audit
  (`scripts/a11y/orca_audit.py`) now covers 13 app and shell journeys,
  including Calendar and Mail (parity row `ACC-32`); eleven findings remain
  open (see `docs/accessibility-audit.md`). A formal, owner-run Orca audit at
  200% text scaling (journey 9) has still not happened — only the owner can
  enable Orca.
- The signed APT repository is not published yet, so Lulo OS does not
  update itself in this beta. Each release's own `install.sh` installs that
  release's packages from its GitHub Release ([Install on Ubuntu](install-ubuntu.md));
  to update, run the next release's install line, and `uninstall.sh`
  removes Lulo OS. Without `gh`, that installer checks the packages against
  the Release's `SHA256SUMS` only (fetched over the same HTTPS connection
  as the script itself); with `gh` installed and logged in it also verifies
  each package's build-provenance attestation. Clean native install,
  upgrade, rollback, and uninstall evidence exists from the disposable-install
  GitHub Actions station (`.github/workflows/security-station.yml`, see
  [security-review-0.9.0-beta.1.md](security-review-0.9.0-beta.1.md),
  "package-lifecycle").
- One owner-reported Dock bug reproduces live but not in the nested test
  compositor, so it remains open: the Dock's Bin icon can vanish and the
  Dock itself lag or disappear for a moment after a delete while its
  service keeps running (DOCK-27). Logging out and in again recovers it.
- Critical visual references and all Orca observations still need native
  candidate evidence; no hardware station is yet certified for Alpha, Beta,
  or 1.0. The 8-hour memory soak has run against release binaries on the
  reference laptop and passed, with one tracked note: Files' private
  footprint grew past its 16 MiB/8h growth budget before leveling off,
  traced to two unbounded in-memory caches and fixed (bounded LRU caches,
  parity row `MEM-03`), with the fix not yet re-run through the same 8-hour
  soak to confirm the new ceiling in practice (see
  [perf/reference-laptop-2026-10-04-memory-soak.md](perf/reference-laptop-2026-10-04-memory-soak.md)).
  This Beta has only been exercised on Intel graphics — **no NVIDIA hardware
  has been tested**; treat NVIDIA/proprietary-driver systems as unverified.
  The owner decided on 2026-10-04 to ship Beta 1 without an NVIDIA test,
  because no NVIDIA machine is available; NVIDIA results are a later-Beta item.
- **This beta is tested on one hardware class only.** The H8 Beta tier names
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

## Speed

- **Some apps and panels open slower than they should.** On the reference
  laptop (Intel, 6.7 GB RAM), most apps take about 0.6–1.8 s to show their
  first window, Files up to about 5 s on a cold start, and Spotlight, Control
  Centre and Notification Centre about 0.35 s to open. The targets are 0.3 s
  and 0.1 s. Oversized icon drawing is already fixed. Further speed fixes come
  in the first Beta updates (docs/perf/speed-sweep-2026-10-05.md).
- **System Settings' live resize** now keeps about 94–97 % of frames on time
  when dragging the window's edge (target 95 %). niri's own Mod-drag resize
  reaches about 83–91 %, and each drag still starts with one slow frame of
  40–70 ms. **⌘Tab's quick tap** (press and release) still takes about
  100 ms in the nested test session (target 50 ms). Holding ⌘ and releasing
  on an app now switches in about 28 ms (was 56). The remaining quick-tap
  time is spent waiting for niri. See
  docs/perf/speed-round-12-2026-10-13.md and parity rows SPEED-11/SPEED-12.

## Feature limits

- **Stage Manager is off by default.** It is built (ADR 0030) and switches on
  from Control Centre or Desktop & Dock, but it is still being polished.
- **No screen recording.** Screenshots work (including the capture
  toolbar), but recording the screen, as the Mac's toolbar and QuickTime do,
  is not in this release (SHOT-01, PLAYER-01).
- **No emoji and symbols picker yet.** Control-Command-Space and Edit ▸ Emoji
  & Symbols do nothing so far; the picker is planned for the next release
  (KB-02, OTHER-15).

- Mail's Google and Microsoft sign-in (OAuth in the browser) is built and
  tested against recorded protocol fixtures, but has not yet been tried
  against real Google or Microsoft accounts. IMAP/SMTP accounts with an app
  password are the safer choice for this beta.
- Open item: the top-bar logo slot still draws the "R" glyph inherited from
  the rmac name. It needs a real Lulo mark before release; that is a design
  task (new artwork), not a text rename, and is intentionally not addressed
  by the rmac-to-Lulo-OS text rename.
- Lulo OS does not clone Apple services, proprietary assets, iCloud, AirDrop,
  AppleCare, Time Machine, or Apple account behavior. The macOS panes and
  menu items that exist only for those services (Apple Account, iCloud,
  AirDrop, Handoff, Time Machine and similar)
  are left out on purpose rather than shown as controls that do nothing.
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

Two Low findings from the
[security review](security-review-0.9.0-beta.1.md) are open. Each has a
mitigation, and both still count against the security gate. SR-18 is
accepted for Beta; SR-39 is accepted for Beta too.

- **SR-18, release build inputs.** Release containers, the rustup installer,
  and the `cargo-cyclonedx` source archive now have content pins in source.
  Rust toolchain artifacts are still selected by version, and the changed
  workflow has not had a native release run. Actions are pinned by commit,
  Rust dependencies are locked and checked by cargo-deny, and every package
  carries a provenance attestation that `install.sh --from-release` verifies.
- **SR-39, terminal apps can lose an argument.** When an app's desktop entry
  asks to run in a terminal, Lulo OS hands its exact arguments to the
  default terminal, Ptyxis. Ptyxis drops an argument written as `$(…)`. It
  never runs it, but the app starts with that argument missing. No app that
  Lulo OS ships is affected.
- **Automatic updates on older installs (SR-29, fixed).** Builds before the
  fix, such as `8ba31b82`, schedule automatic updates without first checking
  whether they remove a package. Current packages simulate first and never
  schedule a removal; this was tested against the real PackageKit. On an older
  install, turn off Automatic Updates and use Update Now until you update.
- **Builds from before the SR-15 and SR-38 fixes.** Older packages embed
  the builder's checkout path in four shell binaries (SR-15). They also keep
  showing banners while the screen is locked, so with the screen reader on,
  a notification's text may be read aloud (SR-38). Current packages fix both.

`install.sh --from-release` needs `gh` (and `gh auth login`) to verify who
built the packages. Without it the install stops; `--allow-unattested`
installs on the release's checksums alone. A Release's own stamped
`install.sh`, run with no arguments, verifies with `gh` when it is installed
and otherwise accepts the checksums, as described under "Release blockers".

## Windows preview

Lulo for Windows is an early preview: it runs on top of Windows 11 (the
Lulo apps as Windows programs, and an optional layer with Lulo's menu bar,
Dock and Spotlight over the Windows desktop). It has been tried on one
Windows 11 laptop and in GitHub's Windows runners; Windows 10 is untested.

- **The installer is unsigned.** No code-signing certificate is bought for
  this free project, so SmartScreen warns on first run: choose **More info**,
  then **Run anyway**. The MSI's SHA-256 is in the Release's `SHA256SUMS`
  and it carries a GitHub build-provenance attestation.
- **Media Player** now ships its LGPL libmpv and FFmpeg beside the app
  (built in CI from pinned tags, with their licences). It has not yet been
  checked on the Windows test PC, and there is no media-key or Now Playing
  integration (WIN-OS-88).
- **Bluetooth** connects only audio devices whose endpoint Windows already
  has, and there is no "discoverable" switch, because Windows apps cannot
  make the PC discoverable (WIN-SET-06). Control Centre's Bluetooth module
  says Unavailable (WIN-OS-63).
- **Not there yet:** printing (Export as PDF works; WIN-OS-10), spelling
  underlines (WIN-OS-09), Software Update, and System Settings' Network, VPN,
  Internet Accounts, Notifications, Focus, Accessibility, Privacy &
  Security, Printers & Scanners and Lulo Intelligence panes (left out, not
  faked; WIN-SET-05).
- **Updates:** install the next release's MSI over this one. Each build's
  MSI version is `0.9.<commit count>`, so a later release always upgrades an
  earlier one.
- **Mail's** Google and Microsoft sign-in has not been tried against real
  accounts, as on Ubuntu.
- Some Windows-reserved shortcuts (several Win+Ctrl chords) cannot be used
  for the Mac's ⌃⌘ shortcuts (WIN-OS-02). Spotlight is Alt+Space, because
  Win+Space switches the input language.

## Lulo OS USB image

`lulo-os-<version>-amd64.iso` (ADR 0031) is attached for the first time in
Beta 3. `lulo-os-iso.yml` builds it after the tagged Release, boots it in
QEMU (UEFI with Secure Boot, and BIOS), installs it with Calamares and boots
the installed disk before it attaches the ISO, so it reaches the Release an
hour or two after the other files.

- It needs a GPU with a Mesa hardware driver: niri refuses software
  rendering, so with `nomodeset`, a plain framebuffer or a VM without 3D the
  session starts but shows nothing. There is no "safe graphics" boot entry.
- NVIDIA's proprietary driver is not on the image (the open driver works);
  the installed system can add it the Ubuntu way.
- Tested only in virtual machines in CI so far, not on a range of real PCs.
- Branding and live-session files are copied into the image, not packaged,
  so later releases change them only for new installs.

## Compatibility limits

Ubuntu 26.04 and niri are the selected reference environment. Other
distributions, compositors, desktop portals, GPU drivers, architectures,
filesystems, input methods, and devices are unverified unless named in
[Hardware support](hardware-support.md).

Use only synthetic test data and keep Ubuntu/GNOME installed as the recovery
session. Check [Release notes](release-notes.md) and
[Troubleshooting](troubleshooting.md) before each test cycle.
