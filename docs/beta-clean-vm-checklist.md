# Beta clean-VM install/upgrade/uninstall checklist

This is a manual runbook for a human (owner or an agent with real VM access)
to exercise the full Beta package lifecycle on a disposable Ubuntu 26.04
virtual machine: **install → first login → upgrade → uninstall**, and
confirm that removing Lulo OS leaves the stock Ubuntu/GNOME session intact.

It exists because `scripts/linux/run-package-lifecycle.py` needs a real,
disposable Ubuntu 26.04 machine with root and a marked-disposable VM
(`/run/rmac-disposable-vm`) to run at all -- by design, no agent on this
project runs it (see its module docstring and `packaging/native/lifecycle.json`
"disposable_marker"). `run-package-lifecycle.py --print-checklist` prints the
path to this file so the automated and manual paths stay discoverable from
one place.

Do this on a **new, throwaway** VM. Never on the reference laptop, and never
on a machine with data you care about.

## Before you start

- A fresh Ubuntu 26.04 (`resolute`) amd64 or arm64 VM (virt-manager or
  `multipass launch 26.04`), not yet logged into past initial setup.
- At least 25 GiB free in the VM (`df -h /`); stop if it drops below 15 GiB
  at any point (matches `packaging/native/lifecycle.json`
  `minimum_free_gib`).
- **niri is not currently installable from the Ubuntu archive.** As of this
  writing, `resolute` has no package literally named `niri` (only
  `niri-companion` and `librust-niri-ipc-dev`, unrelated to the compositor
  itself), and `xwayland-satellite` is not packaged for any Ubuntu release
  either (checked against packages.ubuntu.com). `rmac-session`'s `Depends`
  on `niri` means `apt-get install` of `rmac-session` will fail dependency
  resolution on a stock VM until one of these is true:
  - niri lands in the Ubuntu archive or a trusted PPA you add to the VM
    before installing rmac, or
  - rmac's own `rmac-wm` fork (ADR 0008, Track B) is packaged and declared
    as the dependency instead.
  Check `docs/install.md` and `docs/decisions/0008-window-layer-strategy.md`
  for the current state before starting a run; if this is still unresolved,
  install niri manually in the VM first (however you are sourcing it for
  development) so apt sees the dependency as already satisfied, and note
  that in your report -- it is a real packaging gap, not a VM-setup mistake.
- Two package sets to test an upgrade: a "baseline" (older) and a
  "candidate" (newer) `rmac-apps`/`rmac-session` `.deb` pair, either from two
  tagged GitHub Releases or from two local `build-native-inputs.sh` +
  `check-native-reproducibility.sh` runs. `scripts/linux/verify-native-packages.py
  --directory <dir> --architecture <amd64|arm64>` should pass against both
  before you start.
- A screenshot tool that works from inside the VM's own session (the
  VM's own screenshot utility, or `grim` once logged into Lulo OS).
  **Screenshots stay local to your run's evidence directory; do not commit
  them** (`scripts/check-no-mac-captures.sh` blocks tracked macOS captures,
  but any screenshot of a real desktop session — Mac or Linux — should stay
  out of git for the same privacy reason).

## Steps and what to capture

For each step, note pass/fail and keep any terminal output that shows the
check succeeded (dpkg status lines, `df -h`, etc.) alongside the screenshot.

1. **Baseline install** — `sh install.sh --from-release <baseline-tag>` (or
   `--from-dir <baseline-dir>`) from a normal user account with `sudo`.
   - Screenshot: the final terminal output ("rmac is installed alongside
     Ubuntu/GNOME...").
   - Pass: exit 0; `dpkg-query -W -f='${Status}\t${Version}' rmac-apps
     rmac-session` shows `install ok installed` at the baseline version for
     both; Ubuntu/GNOME's own session entry is still present and unmodified
     (`ls /usr/share/wayland-sessions/`, still has GNOME's `.desktop`).

2. **First login** — log out, and at GDM choose **Lulo OS**, not Ubuntu.
   - Screenshot: the GDM session picker showing both Lulo OS and Ubuntu, and
     the Lulo OS desktop after login (top bar, Dock visible).
   - Pass: session reaches an interactive desktop without falling back to
     safe mode (`systemctl --user is-active rmac-session.target` is
     `active`, `rmac-safe-mode.target` is not); Wi-Fi/Bluetooth/sound
     indicators and the Dock respond; logout returns cleanly to GDM.

3. **Upgrade** — from Ubuntu/GNOME (not while inside Lulo OS — see the
   uninstall.sh guard note below, the same risk applies to upgrading live),
   run `sh install.sh --from-release <candidate-tag>` (or `--from-dir
   <candidate-dir>`) again.
   - Screenshot: terminal output of the upgrade.
   - Pass: `dpkg-query` shows both packages at the candidate version;
     logging into Lulo OS afterward still reaches an interactive desktop;
     no duplicate GDM entries appeared.

4. **Uninstall** — from Ubuntu/GNOME, run
   `sh uninstall.sh`.
   - Screenshot: terminal output ("rmac has been removed. Ubuntu/GNOME is
     unaffected.").
   - Pass: exit 0; `dpkg-query -W rmac-apps rmac-session rmac-archive-keyring`
     reports none of the three as installed (missing or `config-files` is
     fine, `install ok installed` is not); `/etc/apt/sources.list.d/rmac.sources`
     and `/etc/apt/preferences.d/rmac.pref` are gone.

5. **Confirm the stock session is intact** — log out fully, and at GDM
   confirm **Lulo OS is no longer offered** and **Ubuntu/GNOME still is**;
   log into Ubuntu/GNOME.
   - Screenshot: the GDM session picker showing only Ubuntu now, and the
     GNOME desktop after login.
   - Pass: GNOME starts normally; no rmac top bar, Dock, or wallpaper
     remains; the account's Documents/home-directory files created during
     steps 2–3 are still present (uninstall must never touch user data —
     see `packaging/rmac-session/debian/postrm` and
     `scripts/linux/run-package-lifecycle.py`'s protected-user-data
     sentinels for what "never touch" means in the automated version of this
     check).

6. **Reinstall sanity check (optional but recommended)** — repeat step 1
   with the candidate release/dir, to confirm a clean reinstall after a full
   purge works (this is exactly `run-package-lifecycle.py`'s
   `reinstall-candidate`/`final-purge` steps, done by hand).

## Reporting the run

Report, per step: pass/fail, the exact commands run, package versions
involved, and anything that needed a workaround (especially the niri/
xwayland-satellite gap above, if it was hit). Keep screenshots and raw
terminal logs in your own evidence directory (never committed); summarize
findings in prose for the coordinator.
