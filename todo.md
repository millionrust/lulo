# To do

The current plan is [PLAN_NEW.md](PLAN_NEW.md). The standing direction below
comes from [PLAN_V2.md](PLAN_V2.md) and applies to every task in this file.

## Direction

### Principles

- **Linux is the product; macOS is a development port.** A feature isn't done
  because it works on macOS. No Linux code path may call macOS tools.
- **Build on niri; don't fork a compositor before 1.0.** Use layer-shell for
  the menu bar, Dock, Spotlight and overlays.
- **Use platform services, not command output.** XDG portals, desktop entries,
  NetworkManager, BlueZ, UPower, PipeWire, polkit. Never parse human-readable
  CLI output on Linux.
- **Accessibility and performance are release gates, not polish.** Shared
  components expose semantics, keyboard and focus behaviour before apps adopt
  them.
- **Idle means idle.** No unconditional redraw timers; animation renders only
  while it runs, and reduced motion is honoured.
- **Cut scope, never gates.** Data safety, accessibility and security gates are
  never waived.
- **An honest limitation beats simulated system behaviour.** Prefer the
  smallest complete journey that is safe, accessible and measurable.

### Not before 1.0

- Writing our own compositor
- Replacing every Ubuntu settings backend
- Universal global menus for GTK, Qt, Electron, XWayland and games
- A package manager or app store
- Cloud sync, accounts or telemetry
- Rich-text editing, unless GPUI gains an accessible rich-text primitive
- Distributions other than Ubuntu 26.04 LTS

## Product journeys

These decide priority. Each needs an automated or scripted acceptance test
and a manual accessibility check on the reference laptop.

- [ ] 1. Log in, launch an app from the Dock or Spotlight, switch apps, and
      close it.
- [ ] 2. Find a file, preview it, copy, move, rename and trash it, and undo a
      destructive operation.
- [ ] 3. Open Terminal, run a command, scroll, select, copy and paste, and
      manage tabs.
- [ ] 4. Create, search and edit a note, and recover it after a crash.
- [ ] 5. Open, edit and save a text file through the portal without losing
      content.
- [ ] 6. Inspect resource use and safely stop a process, with confirmation.
- [ ] 7. Join Wi-Fi, connect Bluetooth, change audio output and check battery.
- [ ] 8. Complete journeys 1–7 with the keyboard only.
- [ ] 9. Complete the core of journeys 1–7 with Orca at 200% scaling.

## 1.0 scope per app

Check each item against the app and tick what's already done. "Done" includes
loading, empty, unavailable, permission-denied and error states.

- [ ] **Text Editor:** UTF-8 text, open/save, find/replace, crash recovery,
      status, a printing or export path. Later: rich text.
- [ ] **Notes:** folders, tags, search, attachments, pinning, import/export,
      recovery. Later: collaboration.
- [ ] **Terminal:** real PTY, dynamic resize, scrollback, selection, search,
      tabs, profiles. Later: multiplexing.
- [ ] **Files:** safe file operations, Trash, undo, mounts, search, previews,
      open-with actions. Later: universal remote filesystems.
- [ ] **System Monitor:** process and resource views, search and sort, safe
      terminate, history. Never invent per-process GPU numbers.
- [ ] **Apps:** standards-compliant discovery, icons, actions, search and
      launch. Later: installing apps.
- [ ] **System Settings:** only panes with a real backend (Network,
      Bluetooth, Power, Sound, Display info, Appearance). No placeholder panes.

Files safety rules: never block the UI thread on recursive I/O; never follow
symlinks during recursive copy or delete; every overwrite has an explicit
conflict policy; cancel leaves the source intact; Trash comes before permanent
deletion. Tests cover cross-filesystem moves, permission errors, low disk,
name collisions, disappearing mounts and interrupted operations.

## Quality

### CI

Every pull request runs `cargo fmt --check`, Clippy with `-D warnings`, tests
and `cargo deny`. The main branch is never left red.

- [ ] Move the Linux jobs from `ubuntu-24.04` to Ubuntu 26.04. GitHub's
      hosted image went GA 2026-09-17, so self-hosting is no longer needed.
      `release-contracts`/`dependency-policy` already moved (no `apt-get`
      step); `linux-2604` trials the GPUI job's checks on 26.04
      non-blocking until its `apt` package list is verified, then it should
      replace `linux` and `upstream-gpui-linux` outright (docs/ci.md).
- [ ] Add a Linux aarch64 job: cross-build first (done, non-blocking:
      `linux-aarch64` cross-builds the GPUI-free crates via `cross`), native
      smoke test before Beta (not started; docs/ci.md notes
      `ubuntu-26.04-arm` hosted runners as a option).
- [x] Add a minimum-supported-Rust-version job.
- [x] Add a scheduled dependency and security audit.
- [ ] Add a release build and package smoke test.
- [x] Add bounded fuzz runs for desktop entries, config files, terminal input
      and niri IPC JSON (terminal input covers the pinned `vte`/
      `alacritty_terminal` parser, not rmac's own wrapper — see docs/ci.md).
- [ ] Add a Windows job once the Windows port starts (PLAN_NEW workstream C).

### Tests

- [ ] Every platform adapter has contract tests against fake and live
      D-Bus, portal or niri fixtures.
- [ ] Every service interface (`AppCatalog`, `FileOperations`,
      `SystemMonitor`, network, Bluetooth, power, audio, portals, compositor,
      settings store) has an in-memory fake; app tests use fakes, not the live
      system bus.
- [ ] Visual reference screenshots of rmac at 100%, 150% and 200% for critical
      screens, with reviewed diffs.
- [ ] Every bug fix adds a regression test, or a golden screenshot if the fix
      is visual.

### Accessibility gates

- [ ] Audit every shared `rmac-ui` component. Each interactive element has:
    - [ ] a stable accessible identity, role, name, state, value and actions;
    - [ ] a correct tab order and a visible focus ring;
    - [ ] full keyboard operation, with no pointer-only controls;
    - [ ] announcements for asynchronous status and errors;
    - [ ] no clipping at 200% text and UI scaling;
    - [ ] usable high-contrast colours;
    - [ ] reduced motion read from the Settings portal.
- [ ] Verify each release journey with Orca on Ubuntu.
- [ ] Automate semantic-tree assertions where GPUI allows it.

### Performance budgets

Measure on the reference laptop at 100%, 125%, 150% and 200% scaling, with
multiple monitors, suspend/resume and hotplug. The budgets may be adjusted once,
with a decision record.

| Metric | Budget |
|---|---|
| Warm launch to interactive | p95 ≤ 500 ms for simple apps; ≤ 900 ms for Files and Terminal |
| Idle CPU | ≤ 0.3% per app; ≤ 1% for all shell surfaces combined |
| Idle wake-ups | No redraw while nothing changes |
| Input to visible response | p95 ≤ 50 ms |
| 60 Hz animation | ≥ 99% of frames within 16.67 ms |
| 120 Hz animation | ≥ 95% of frames within 8.33 ms |
| Memory | Per-app budget, no leaks over an 8-hour soak |

- [ ] Record every metric on the reference laptop and commit the results.
- [ ] Repeat on an NVIDIA system before Beta.

### Code rules to check

- [ ] No destructive-operation error is dropped with `let _ = ...`.
- [ ] Persisted data uses versioned serde formats, written to a temporary
      sibling, flushed and atomically renamed, keeping the last known-good copy.
- [ ] User-visible failures return typed errors with a recovery action.
- [ ] Logs redact secrets and document contents.
- [ ] Domain crates never import GPUI, Wayland, D-Bus or platform FFI.

## How we work

**Every issue states:** the user outcome; what's in and out of scope; failure
and recovery behaviour; accessibility behaviour; performance impact; the test
plan; screenshots or a recording if visual; the platforms; and the docs to
update.

**A feature is done when:**
- it meets its acceptance criteria, including failure states;
- unit, component and integration tests pass;
- keyboard and accessible semantics are verified;
- it adds no polling or main-thread blocking;
- errors are visible and actionable;
- it works on the reference laptop, and the macOS build impact is known;
- formatting, Clippy, tests, audit and packaging checks pass;
- the docs and known limitations are updated;
- a reviewer can reproduce it from a clean checkout.

**Milestones:** one active phase at a time; small vertical slices; a decision
record for every GPUI, compositor, data-format, privilege or packaging choice;
a demo of a real user journey every two weeks.

- [ ] Review the risk register in PLAN_V2 §9 and PLAN_NEW §7 at every phase
      gate.

## After Beta (research, never blocks 1.0)

Each item starts with a two-week spike and a go/no-go decision.

- [ ] Global menus for third-party apps through GIO exported menus and D-Bus
      menus, where detected; never claim universal coverage.
- [ ] Richer Mission Control (per-window pictures; needs our own compositor).
- [ ] Dragging files out of apps, if GPUI and Wayland allow it.
- [ ] Our own compositor on Smithay (ADR 0007).
- [ ] An Ubuntu flavour or remix with a graphical installer.
- [ ] Other distributions and compositors.

## Rename rmac to Lulo OS

The public name is now Lulo OS, and the README uses it. The repository lives
at [millionrust/lulo](https://github.com/millionrust/lulo). The code still says
`rmac` in about 12,000 places across 1,224 files. Rename it in stages so
nothing breaks and existing test machines keep their settings.

### Claim the name

- [x] Move the repository to `millionrust/lulo` and make it public. GitHub
      redirects `snehacodex/rmac`.
- [ ] Point local clones at the new address:
      `git remote set-url origin https://github.com/millionrust/lulo.git`.
- [ ] Register a domain (`lulo-os.org`, `lulo-os.dev`, `luloos.dev` and
      `luloos.org` had no DNS as of 2026-09-23). Point it at GitHub Pages; it
      becomes the APT repository address below.

### Rename what people see

- [x] Login screen session name: `Name=rmac` in
      `packaging/rmac-session/rmac.desktop`.
- [x] About windows, the system menu and any remaining "rmac" UI strings.
- [x] User guide, install and troubleshooting docs.

### Rename the technical names (one pull request each)

- [ ] Debian packages: `lulo-apps`, `lulo-session`, `lulo-archive-keyring`.
      Add transitional `rmac-*` packages that depend on them, so APT upgrades
      existing machines.
- [ ] D-Bus names: the 31 `org.rmac.*` names become `org.lulo.*`. Keep the old
      names as aliases for one release.
- [ ] systemd units and `rmac-session.target`, binaries under
      `/usr/libexec/rmac`, and `XDG_CURRENT_DESKTOP=rmac:niri`.
- [ ] User data: on first start, move `$XDG_CONFIG_HOME/rmac`,
      `$XDG_STATE_HOME/rmac` and `$XDG_RUNTIME_DIR/rmac` to `lulo`, and leave
      a note in the log.
- [ ] Crates: `rmac-*` becomes `lulo-*`, after the gpui-kit migration in
      `PLAN_NEW.md` lands, to avoid two huge overlapping changes.
- [ ] Documentation checks, packaging contracts and release scripts that
      name `rmac`.
- [ ] Remove the transitional packages and D-Bus aliases one release later.

## Install and auto-update

The full design is in [Update trust](docs/update-trust.md) and
[Software Update](docs/software-update.md). **GitHub is the update source:**
GitHub Releases hold the packages, and GitHub Pages serves the signed APT
repository built from them. Machines update through APT and PackageKit, never a
custom downloader. curl is used only to bootstrap the first install.

```
git tag vX.Y.Z → GitHub Actions builds .debs (amd64, arm64)
  → attaches them to the GitHub Release
  → builds and signs the APT repository → deploys it to GitHub Pages
  → machines: apt / PackageKit read https://millionrust.github.io/lulo/
```

### Decisions needed

- [x] Make the repository public (done: `millionrust/lulo`).
- [ ] Choose the address: `https://millionrust.github.io/lulo/`, or a custom
      domain pointed at Pages so hosting can move later without changing
      clients. It fills `@RMAC_REPOSITORY_URI@` in
      `packaging/apt/rmac.sources.in`.
- [ ] Decide who holds the signing keys: the primary key offline, and a
      signing subkey in a GitHub Actions environment secret that needs a
      reviewer's approval.

### Server side (GitHub)

- [ ] Generate the archive key and publish its fingerprint in the README,
      `docs/install.md` and each GitHub Release.
- [ ] Add a `release.yml` workflow triggered by a `v*` tag:
    - [ ] Build `rmac-apps`, `rmac-session`, `rmac-archive-keyring` and the
          repository configuration package on amd64 and arm64 (use
          self-hosted runners if GitHub has no Ubuntu 26.04 image yet).
    - [ ] Attach the `.deb` files, checksums, the SBOM and build provenance to
          the GitHub Release.
    - [ ] Run `scripts/linux/publish-apt-snapshot.py`, sign `InRelease` with
          the CI subkey, and deploy the tree with `actions/deploy-pages`.
          Deploy an artifact rather than committing to a `gh-pages` branch, so
          package files don't pile up in git history.
    - [ ] Serve `install.sh` and `uninstall.sh` from the same Pages site.
- [ ] Add a scheduled `rollout.yml` workflow that republishes Pages with the
      next `Phased-Update-Percentage` step (10 → 25 → 50 → 100%, at least 24
      hours apart). A manual run with 0% halts a rollout.
- [ ] Keep at least 3 snapshots on Pages, and document the emergency path of
      tagging a higher version.
- [ ] Watch the GitHub Pages limits (1 GB site, about 100 GB a month of
      traffic). If they're outgrown, move to a CDN behind the same custom
      domain.

### First install

- [ ] Add an `rmac-archive-source` package that installs
      `/etc/apt/sources.list.d/rmac.sources` and
      `/etc/apt/preferences.d/rmac.pref`, or have `install.sh` write them.
- [ ] Write `install.sh`:
    - [ ] Check for Ubuntu 26.04 on amd64 or arm64; use `sudo`, never run as root.
    - [ ] Download `rmac-archive-keyring.deb` and verify the fingerprint
          written into the script.
    - [ ] Install the keyring and the repository configuration.
    - [ ] Run `apt update && apt install rmac-session`.
    - [ ] Never touch the GNOME session.
    - [ ] Put the whole body in `main` and call it on the last line, so a
          partial download can't run.
    - [ ] Finish with "Log out and choose Lulo OS on the login screen".
- [ ] Write `uninstall.sh`: `apt purge rmac-session rmac-apps
      rmac-archive-keyring` and remove the repository configuration.
- [ ] Update `docs/install.md` with the one-line install and the same steps
      written out by hand.

### Updates on the machine

- [ ] Add `rmac-update-check.timer`: once a day, ask PackageKit for updates
      and show an "Updates available" notification.
- [ ] Install on restart with PackageKit offline updates, so running session
      programs are never replaced mid-session.
- [ ] Add an optional "Install updates automatically" setting that adds
      `origin=rmac` to `unattended-upgrades`, with security updates on by
      default.
- [ ] Verify install, update, rollback and uninstall in a VM, then on the
      reference laptop (plan phase B5 in [PLAN_NEW.md](PLAN_NEW.md)).

### Windows (later)

- [ ] MSIX package attached to each GitHub Release, with an `.appinstaller`
      file on GitHub Pages for background updates.
- [ ] `winget` listing whose installer URLs point at the GitHub Release assets.
- [ ] Optional bootstrap: `irm https://<domain>/install.ps1 | iex`.
