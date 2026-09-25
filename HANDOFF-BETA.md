# Hand-off: getting Lulo OS to Beta 1

Written 2026-09-25 for whoever picks this up next. `dev` on GitHub (millionrust/lulo) is the integration branch; everything below is relative to it.

## 1. Where things stand

- **dev builds and passes.** The last full laptop run was 509 + 1,012 tests with 0 failures and clippy `-D warnings` clean on every touched package. CI is green except the new non-blocking `behavior-parity` job, which is expected to report differences.
- **The reference laptop runs the latest build.** It was built from `e6b23a42` and installed via `~/install-lulo.sh` on 2026-09-25 19:43. That build includes our own niri `26.04+lulo1-1`, 21 services active, no failed units, AT-SPI registration of the shell, both power-key inhibitors, the cursor theme in the right place, and the touchpad-after-resume hook.
- **Mac parity is tracked in ONE file,** `docs/parity.md` (398 rows: 84 Fixed, 76 Partial, 238 open). Rules are in its header. There are no P0 left; 73 P1 rows are open or partial.
- **The behaviour suite is automated** (`docs/behavior-suite.md`). Scenarios are recorded on a real Mac (`scripts/behavior/record_mac.py`) and replayed against Lulo in a nested headless compositor (`scripts/behavior/run_lulo.py`). The first run matched 6 of 23 scenarios.

## 2. Beta 1 blockers, in order

1. **Release signing key and publishing: the owner must do this.** It takes about 30 minutes. Follow `docs/release-process.md` ("Owner setup"):
   - Run `scripts/release/create-archive-key.sh`, then store the private key backup offline.
   - Add the GitHub environment and secrets it prints (signing subkey, passphrase).
   - Enable GitHub Pages for the APT archive.
   - Without this, nobody else can install Lulo or receive updates.
2. **Merge the in-flight branch `wip/behavior-fixes`** (pushed alongside this file). It holds code for FILES-44/45/47/48/49, ACC-09, TE-19 and TE-20. Its state when written: it compiles, but the scenarios have **not** been run against it. Still to do on it:
   - FILES-43: Return doesn't commit a rename. Confirm with `run_lulo.py --explore` whether the cause is app code or the virtual keyboard.
   - FILES-46: Undo of Move to Trash should reselect the restored file. There's a sketch in the branch's commit message.
   - Run every scenario in `tests/behavior/` until it passes, then package tests and clippy, then update the parity rows.
3. **Re-run the Beta checklist** (`docs/beta-checklist.md`). Its results are stale: journeys 2–4 were "Fail" for accessibility reasons fixed on 2026-09-25 (ACC-01/02/03). Journeys 5, 6, 7 and 9 were never run. Update the table with real results.
4. **Clean-machine install test** (`docs/beta-clean-vm-checklist.md`) on a spare PC or VM with a fresh Ubuntu 26.04.
5. **Tag `v0.9.0-beta.1`** once 1–4 are done. The release workflow builds, signs and publishes. Add `packaging/release-notes/0.9.0~beta.1.txt` first so Software Update shows release notes.

Everything else, meaning the P1/P2 rows in `docs/parity.md`, can ship in Beta 2 and later through the update system.

## 3. Highest-value P1 work after Beta 1

- **Files:** FILES-15/16 (Get Info as a real window), FILES-08 (List view disclosure triangles), FILES-13 (⌘J View Options), FILES-37 (search scope), FILES-20 (tags).
- **Settings:** SET-78/79/83/84/86/91/99 (large feature panes); SET-05/06/08 (password, users, lock delay: security-sensitive).
- **Auth:** SWU-07, a polkit authentication agent. The design is in `docs/software-update.md`. It is needed for admin actions other than updates.
- **Menus:** BAR-03 (status-item menus for Bluetooth, Sound, Focus, VPN).
- **Preview:** PREV-02/03/08/09 (print images, export, markup).
- **Other:** OTHER-02 (Open/Save as a sheet on the parent window), OTHER-12 (Print dialog).
- **Accessibility:** ACC-04/08, which cover typing through AT-SPI (an upstream accesskit limit) and button names.

## 4. How we work (read before touching anything)

- **Branches:**
  - Do work on a feature branch from `dev`, merge into `dev`, then push `dev`.
  - The owner rule is **no Co-Authored-By or AI attribution lines in commits**.
- **Build and test on the reference laptop, not the Mac.** The laptop is `jacob@192.168.18.52`, i5-5300U, 6.7 GB RAM.
  - Push your branch to the laptop repo (`ssh://jacob@192.168.18.52/home/jacob/rmac`) as `refs/heads/claude/<branch>`, then `git -C ~/rmac worktree add ~/rmac-<name>-wt <sha>`.
  - Build under the shared lock and target dir: `exec 8>/tmp/lulo-cargo.lock; flock 8; export CARGO_TARGET_DIR=$HOME/rmac-wt/target`.
  - Use package-scoped builds only, with `--profile iterate`. Touch changed `.rs` files first, because the shared target dir can serve stale crates.
  - Build shell crates with `cd shell && cargo … --features wayland`. The wayland feature is what the real build uses, so check with it.
  - CI runs `cargo clippy --keep-going --workspace --all-targets --all-features -- -D warnings`. Run clippy on your packages with `--message-format short` to see every finding at once.
  - After any dependency change, refresh both lockfiles offline: `cargo metadata --offline` in the root and in `shell/`.
  - AGENTS.md applies: no Docker, no separate `CARGO_TARGET_DIR`, no concurrent cargo pipelines, and keep 25 GiB of disk free.
- **Packages:**
  - `~/rmac-coord/pkg-build.sh` fast-forwards `~/rmac-wt` to the `claude/incoming` ref, builds the .debs, and copies our niri debs in.
  - To stage an install set:
    1. Copy the debs plus `native-packages.json` into a new folder.
    2. Run `sha256sum -- *.deb > SHA256SUMS`, then `chmod 644 *`.
    3. Verify it with `scripts/linux/verify-native-packages.py`, **run from a checkout at the same commit the packages were built from**.
    4. Point `~/install-lulo.sh` at the set, and `~/rmac-install-wt` at that commit.
  - The owner runs `./install-lulo.sh` from the **Ubuntu (GNOME)** session; the installer refuses to run inside Lulo.
- **The owner's live laptop session: never disturb it.**
  - No input injection into `wayland-1` or `/run/user/1000`. Use the nested headless compositor in `run_lulo.py` instead.
  - One app instance at a time. Leaked Settings instances once exhausted the system D-Bus connection limit.
  - Use temporary `XDG_*` dirs for GUI tests, and never set `screen-reader-enabled`, because Orca starts talking.
  - Never put `pkill`/`pgrep` patterns on an ssh command line: they match your own session. Use a script file.
- **Mac reference:**
  - Mac captures are never committed. A CI check blocks them, and parity rows describe the Mac only in words.
  - When auditing the owner's Mac, never confirm destructive dialogs: open, record, Cancel.

## 5. Known hazards

- **xwayland-satellite:** the owner's machine has the PPA's `0.8.3ppa1`, newer than our pinned `0.8.2+lulo1-1`. The installer now keeps a newer third-party package instead of downgrading. Consider bumping the pin to 0.8.3 when rebuilding niri (`scripts/linux/build-niri-packages.sh`, `packaging/third-party/upstreams.json`).
- **Resume on the Synaptics RMI4 touchpad:** the `system-sleep/rmac-input-resume` hook's reload path is unit-tested but not yet proven on a real failing resume.
- **AT-SPI application name:** apps announce as `rmac-calculator` rather than "Calculator" (A11Y-01, needs a small accesskit_unix fork).
- **Calculator's Scientific window size:** fixed in `shell/compat/gpui_linux` (runtime resize double-subtracted the CSD inset). This affects every app's runtime resize, so watch for regressions.
