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
- **niri and xwayland-satellite come from the same Release.** Neither is in
  the Ubuntu 26.04 archive, so every Release (and every `--from-dir` set
  assembled from `build-niri-packages.sh` output) carries Lulo OS's own
  `niri_26.04-0luloN` and `xwayland-satellite_0.8.2-0luloN` packages, and
  `install.sh` installs them with rmac. Do **not** add the danklinux PPA to
  a clean VM: the point of this run is to prove the Release alone is
  enough. (On a machine that already has the PPA's newer `26.04ppa3`,
  `install.sh` keeps it and says so; that is expected, not a failure.)
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
     both; `dpkg-query -W -f='${Status}\t${Version}\n' niri
     xwayland-satellite` shows both installed at a `-0luloN` version;
     `niri --version` prints `26.04 (8ed0da4)`; Ubuntu/GNOME's own session
     entry is still present and unmodified (`ls
     /usr/share/wayland-sessions/`, still has GNOME's `.desktop`; niri's own
     `niri.desktop` is expected beside it).

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
     and `/etc/apt/preferences.d/rmac.pref` are gone. `niri` and
     `xwayland-satellite` stay installed by design; `sudo apt-get purge niri
     xwayland-satellite` must then also succeed and leave GNOME working.

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
involved, and anything that needed a workaround (for example if apt could not
resolve a `niri` or `xwayland-satellite` runtime dependency). Keep screenshots and raw
terminal logs in your own evidence directory (never committed); summarize
findings in prose for the coordinator.
