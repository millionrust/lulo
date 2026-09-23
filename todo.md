# To do

## Rename rmac to Lulo OS

The public name is now Lulo OS, and the README uses it. The code still says
`rmac` in about 12,000 places across 1,224 files. Rename it in stages so
nothing breaks and existing test machines keep their settings.

### Claim the name

- [ ] Search the USPTO and EUIPO trademark registers for "Lulo" in software
      before announcing it.
- [ ] Create the `lulo-os` GitHub organization (free as of 2026-09-23; the
      `luloos` account is already taken).
- [ ] Register a domain (`lulo-os.org`, `lulo-os.dev`, `luloos.dev` and
      `luloos.org` had no DNS as of 2026-09-23). Point it at GitHub Pages; it
      becomes the APT repository address below.
- [ ] Transfer `snehacodex/rmac` to `lulo-os/lulo`. GitHub redirects the old
      URL.
- [ ] Design a logo that isn't a single fruit with a leaf (see Apple's 2020
      opposition to Prepear's pear logo).

### Rename what people see

- [ ] Login screen session name: `Name=rmac` in
      `packaging/rmac-session/rmac.desktop`.
- [ ] About windows, the system menu and any remaining "rmac" UI strings.
- [ ] User guide, install and troubleshooting docs.
- [ ] Remove "rmac" from the About text once no user-visible string is left.

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
  → machines: apt / PackageKit read https://<owner>.github.io/<repo>/
```

### Decisions needed

- [ ] Make the repository public before launch, or use a separate public repo
      (such as `rmac-apt`) for Pages. Pages on a private repo needs a paid plan,
      and assets on private releases can't be downloaded anonymously.
- [ ] Choose the address: `https://<owner>.github.io/<repo>/`, or a custom
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
    - [ ] Finish with "Log out and choose rmac on the login screen".
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
