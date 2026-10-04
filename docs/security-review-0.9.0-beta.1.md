# Security review — 0.9.0-beta.1 (source review and disposable station)

This is the I6 security and privacy review
([security-release-review.md](security-release-review.md)) for the
0.9.0-beta.1 early-access release, run against the 80 checks in
`scripts/security-review.json`. The canonical summary sits beside this file
as [security-review-0.9.0-beta.1.json](security-review-0.9.0-beta.1.json).

**Gate status: Fail.** The source review is done, the disposable-install
station ran on 2026-10-04 (see "Station evidence"), and nothing Critical or
High is open. What still blocks the gate:

- 3 findings are open (see "Open findings"). None is above Low.
- 11 checks still need a station run, most of them on the
  reference laptop (see "Reference-laptop checks").
- The reference laptop (`amd64-intel-laptop`) and `amd64-amd-desktop`
  stations have not run. `amd64-nvidia-desktop` is waived for Beta 1 by the
  owner's 2026-10-04 decision, recorded in the verifier as
  `owner-2026-10-04-beta1-without-nvidia`.

The verifier therefore fails, as it should:

```sh
python3 scripts/verify-security-review.py \
  --evidence <copy of docs/security-review-0.9.0-beta.1.json outside the checkout> \
  --tier beta --revision <the JSON's revision, checked out clean>
```

## Scope and method

- **Reviewed tree:** `dev` at 5bea40ca, plus the fix commits listed below,
  plus a fresh pass over the privileged code added on 2026-09-24/25 (see
  "Fresh pass, 2026-09-25"). That review's last fix commit
  was `af238a44`. The post-review fixes were checked with the repository's Python
  suites and `rustfmt`, with `rustc` unit-test builds of the changed pure
  modules on macOS, and with Linux-configuration type checks of
  `rmac-keyboard`, `rmac-network` and `rmac-bluetooth` against local rlibs. No
  cargo build ran; the coordinator's Linux build is still owed.
- **2026-10-04 pass:** the disposable-install station (below), a full source
  pass over logs and diagnostics (508 sink lines in both workspaces, every
  `Debug` on a secret-bearing type), and complete source traces for the
  remaining D-Bus/polkit, notification, Files and terminal checks. Its fixes
  (SR-30 to SR-37) are on branch `op/security-stations`; GitHub CI builds and
  clippy-checks them on Linux and macOS. The JSON's `revision` is that pass's
  last fix commit (`ef447b04`).
- **Method:** source review with `file:line` evidence, plus native execution
  on the disposable-install station. The review includes the crates,
  packaging, maintainer scripts, install and uninstall scripts, and the
  GitHub workflows.
- **Priorities:** in order, the lock screen and PAM; privileged helpers
  (pkexec/polkit, keyd, localed/timedated/hostnamed, systemd units for
  Sharing); portals and every `org.rmac.*` D-Bus name and runtime socket;
  clipboard history and network fetches; install, update and release; and
  Files operations.
- **Not reviewed:** CalDAV, which is not merged.
- **JSON `results`:** `pass` means the review is complete for that check and
  no finding against it is open; where the check needs native behaviour, the
  station evidence below supplies it. `pending` means a station run is still
  owed or a finding is open.
- **Evidence format:** format 2 pins a SHA-256 for every unique source file
  listed by the review contract, so any source change makes the summary
  stale. Candidate verification also requires the JSON's revision to match a
  clean checkout, so verify a copy kept outside the checkout. The `stations`
  list is the tier's H8 stations plus `disposable-install`; a waived station
  carries the owner decision's id.

Of the 80 checks, 69 are `pass` and 11 are `pending` (the JSON is canonical).

## Station evidence

| Station | Status | Evidence |
|---|---|---|
| `amd64-intel-laptop` (reference laptop) | pending | needs the owner; read-only steps in "Reference-laptop checks" |
| `amd64-amd-desktop` | pending | still required by the H8 Beta tier; no run yet, and no AMD desktop station has been named |
| `amd64-nvidia-desktop` | waived | owner decision 2026-10-04: Beta 1 ships without NVIDIA testing because no NVIDIA machine is available ([known-limitations.md](known-limitations.md)). Verifier waiver id `owner-2026-10-04-beta1-without-nvidia`, beta tier only. It never passes a check or another station |
| `disposable-install` | pending (ran; 9 of 12 station checks pass, 3 still need work) | `.github/workflows/security-station.yml`, run [37175024540](https://github.com/millionrust/lulo/actions/runs/37175024540) |

### Disposable install (GitHub Actions `ubuntu-26.04`)

A GitHub-hosted runner is a fresh Ubuntu 26.04 VM for each job. It has
passwordless sudo and is thrown away afterwards, so the destructive checks run
there and never on the reference laptop. The workflow takes the
`lulo-candidate-<sha>` artifact that `candidate.yml` built for the latest
green `dev` commit (`2f3ea7a4`, version `0.9.0~beta.1-38`) and verifies its `SHA256SUMS`. It then
marks the runner disposable (`/run/rmac-disposable-vm`) and runs
`scripts/linux/run-security-station.py` as root. The script refuses to run
anywhere else.

Each check writes its own JSON file. The run also writes a summary,
`station-evidence.json`, which is uploaded as the `security-station-evidence`
artifact (kept 90 days). The summary is copied to
[security-station-evidence-0.9.0-beta.1.json](security-station-evidence-0.9.0-beta.1.json).
All fixtures are synthetic: the users, the password, the package names, the
APT repository and its key exist only on the runner. The journal check proves
that none of the planted values reached the log.

Run [37175024540](https://github.com/millionrust/lulo/actions/runs/37175024540)
(image `ubuntu26` 20260927.149.1, station scripts `a19e5ae8`) tested
candidate `0.9.0~beta.1-38` from `dev` `2f3ea7a4`
(candidate run 37152104352). It is the third run of the workflow; the first
two exposed bugs in the station script, which are fixed.

| Station check | Result | What it proved |
|---|---|---|
| candidate-provenance | pass, with SR-15 still open | `SHA256SUMS` and `verify-native-packages.py` pass, and no personal home path is in any rmac file. Four shell binaries (`rmac-app-switcher`, `rmac-dock`, `rmac-mission-control`, `rmac-osd`) still embed the CI builder's checkout path through `env!("CARGO_MANIFEST_DIR")`. SR-15 stays open for this. |
| install-effects | pass | With dependencies preinstalled, installing `rmac-apps` and `rmac-session` adds exactly the 288 paths dpkg lists. It changes and removes nothing else, apart from trigger-rebuilt caches. Purge restores the tree exactly. The maintainer scripts never create `/etc/keyd/rmac.conf`, never touch an administrator's own file there (on configure or purge), regenerate a stale rmac-owned file on configure, and remove it on purge. |
| package-permissions | pass | All 288 package-owned paths are `root:root` with no setuid, setgid or sticky bit. Nothing is group- or world-writable, files are only `0644`/`0755`, there are no file capabilities and nothing lands in `/usr/local` (30 shared base directories skipped). |
| package-lifecycle | not proven | `run-package-lifecycle.py` now gets past its preflight; its os-release bug is fixed (`f032fb9a`). In run 37176094048 it installed the baseline but stopped at its own metadata gate: Ubuntu 26.04's `appstreamcli validate --no-net` rejects rmac's installed metainfo (functional issue F-2). Rollback stays pending until that passes. |
| systemd-hardening | pass | `systemd-analyze verify` is clean for the relay socket and service. `systemd-analyze security` rates the relay 2.7 (OK): `DynamicUser`, `NoNewPrivileges`, an empty capability set, `PrivateNetwork` and `AF_UNIX` only. |
| keyboard-relay | pass | SR-13 relay side: the socket is `0666 root`, and keyd and the relay ran. `native` was applied (`ok`). A `command()` binding, shell metacharacters, no newline, two lines, a NUL byte and 70 KB were all refused. No command ran, the session user is not in `keyd`, and the journal never echoes a request. |
| polkit-policy | pass | `org.rmac.mac-keyboard.apply` is `auth_admin`/`auth_admin`/`auth_admin_keep`, and rmac ships no `.rules` file. Unprivileged `pkexec` without an agent exits 127 ("not authorised", distinct from 126 cancel) and the helper does not run. `pkcheck` answers `auth_admin`. |
| lock-units | pass | `rmac-lock.service` has `AssertPathExists=/etc/pam.d/rmac-lock`, `OnFailure=rmac-lock-fallback.service` (swaylock, installed) and `LimitCORE=0`. `/etc/pam.d/rmac-lock` includes only `common-auth` and `common-account`. This proves only the runbook's prerequisites; the TTY recovery itself needs the laptop. |
| untrusted-open | pass | Eight fixtures in a synthetic user's `~/Downloads`: an executable and a non-executable `.desktop`, `.sh`, an extensionless script, `.py`, an ELF binary, an executable `.txt` and a `$(…)` file name. Each went through Files' open path headless: the OpenURI portal on a private session bus, then `xdg-open` (`crates/rmac-portal/src/open.rs`), with `XDG_CURRENT_DESKTOP=rmac:niri` and rmac's MIME defaults. No marker file appeared, so nothing was executed. Scripts and text resolve to Text Editor. |
| terminal-wrapper | not proven | Ptyxis, the default `x-terminal-emulator`, never ran the probe. In run 37175024540 there was no user manager for its systemd scope. In run 37176094048 there was, but Ptyxis then timed out waiting for the Settings portal on the headless runner. No marker appeared, and no argv was recorded either. |
| sr29-packagekit | safety proven, script verdicts outdated | Run 37176094048, installed `rmac-update-check.service` against the real PackageKit 1.3.4 and a synthetic signed repository. (1) **Safe:** 2.0 was simulated, downloaded and scheduled (`reboot`, prepared ID exactly 2.0). (2) **Stale:** after the repository moved to 2.1, PackageKit itself discarded the prepared 2.0 at refresh, and the checker scheduled only a freshly simulated 2.1. (3) **Refused trigger:** with the offline action denied, `trigger failed` exits 1 and nothing is scheduled. (4) **Destructive:** a 3.0 that conflicts with an installed package was never scheduled, whether or not 2.0 had been scheduled before. (5) **Untrusted:** a repository re-signed by an unknown key fails at refresh (`cannot-fetch-sources`) with nothing scheduled. The station's own verdicts expected the checker to cancel stale plans itself and so reported failure; the corrected verdicts need one rerun. The cancellation-failure warning cannot be reached natively, because PackageKit drops a stale plan before the checker could cancel it. It stays covered by the fake PackageKit tests. |
| journal-redaction | pass | This covers 93 rmac journal entries from package scripts, the keyboard helper and relay, polkit and the update checker. The planted password, private path, package name, repository path, relay request and key ID never appear. There are no control characters and no D-Bus unique names, and the longest line is 134 characters. |


## Findings fixed in this review

| ID | Severity | Boundary | Finding | Fix |
|---|---|---|---|---|
| SR-01 | High (latent) | updates | `install.sh` passed any keyring that merely *contained* the pinned fingerprint (`grep -Fc … -ge 1`), so an extra primary key in that keyring would also be trusted through `Signed-By`. It then ran `sudo dpkg -i` on the downloaded, unsigned bootstrap `.deb`, so that package's maintainer scripts ran as root on the strength of an HTTPS download alone. Not reachable today, because the placeholder fingerprint makes the repository path refuse to run. | Now requires exactly one `pub` record whose own `fpr` is the pin (`scripts/linux/install.sh:138`). Installs only the verified keyring file (`:155`); `rmac-archive-keyring` then comes from the signed repository. `docs/install.md` updated. Tests: `scripts/test_install_uninstall.py` `InstallKeyringVerificationTests`. |
| SR-02 | High | lock-boundary | `rmac-lock.service` used `ConditionPathExists=/etc/pam.d/rmac-lock`. When a Condition fails, systemd skips the unit and `systemctl --user start` still exits 0. `crates/rmac-shortcuts/src/lock.rs:140-151` treats that as locked, and the pre-sleep path (`lock.rs:326-330`) then drops the logind sleep inhibitor. **Exploit:** with the PAM file missing (a development install, or an admin deleting the conffile), idle lock, `Lock()` and the lock before suspend all report success while nothing is locked; the lid opens to a live session. | `AssertPathExists=` fails the start job and runs `OnFailure=` (`crates/rmac-session/units/rmac-lock.service:13`). Test in `crates/rmac-session/src/tests.rs`. |
| SR-03 | High | file-operations | Zip expansion created link entries last but with `create_dir_all(parent)`, so a link could be created *through* an earlier link. **Exploit:** a downloaded zip containing `x -> ../../.config/autostart` and then `x/evil.desktop -> …/payload.desktop` plants an autostart entry when opened in Files, because Files auto-expands on open. | Every folder above a link is created one component at a time and must be a real folder under staging; otherwise the expansion fails as damaged (`crates/rmac-archive/src/expand.rs:120,129`). Tests: `a_link_is_never_created_through_an_earlier_link`, `links_inside_real_folders_still_expand`. The tar path was already safe (`unpack_in`). |
| SR-04 | Medium | packages | The `keyring` job's job-level `if:` read `secrets.*`. GitHub rejects that for the whole workflow file, so no tag push would produce the checksums, SBOM or provenance. | Gated on `vars.RMAC_ARCHIVE_SIGNING_FINGERPRINT` (`.github/workflows/release.yml:394`); the first step refuses when the secret is missing. Test: `test_no_job_condition_reads_the_secrets_context`. |
| SR-05 | Medium | packages | `attach-release` did not depend on `dependency-policy`, so `.deb`s with a denied advisory or licence were still attached and attested. | `needs: [dependency-policy, …]` plus a success condition (`release.yml:308`). Test added. |
| SR-06 | Low | packages | `rollout.yml` pasted `steps.live.outputs.phase`, which is parsed from the fetched `Packages` file, straight into a `run:` script, so whoever controls the site's content could inject shell. | Values are passed through `env:` (`.github/workflows/rollout.yml:94-97`). Test: `test_fetched_repository_values_never_expand_inside_run_scripts`. |
| SR-07 | Low | packages | Release and rollout checkouts, including the jobs holding `contents: write`, `id-token` and `pages`, kept the job token in `.git/config` for every later step. | `persist-credentials: false` on every checkout in `release.yml` and `rollout.yml`. |
| SR-08 | Low | lock-boundary | Typing past the 512-byte password field returned `EditError::Full`, which was fatal. Holding a key crashed the provider, which then restarted. | A full field ignores input (`crates/rmac-lock-provider-linux/src/runtime.rs:287`). Test: `typing_into_a_full_password_field_is_ignored_not_fatal`. |
| SR-09 | Low | lock-boundary | The release profile is `panic = "abort"`, so a panic in the locker dumps core through systemd-coredump. That core can hold the typed password (libpam's copy and `SecretInput`). | `LimitCORE=0` (`rmac-lock.service:27`). |

Also fixed, not a security finding: `install.sh --from-release` could not
find Beta packages. The Debian version is `0.9.0~beta.1`, which the asset
regex rejected; it now accepts `~` (`install.sh:191`, with tests). GitHub
stores `~` in an uploaded asset's name as `.`, so `release.yml` renames `~` to
`.` before it writes `SHA256SUMS`, attests and uploads, and `install.sh`
accepts the stored names (dev `b31c3e2c` and earlier; tests `f7d482f3`:
`test_uploaded_asset_names_match_sha256sums_after_github_renames_tildes`,
`test_finds_a_beta_package_under_the_name_github_stores`).

## Findings fixed after the review

| ID | Severity | Boundary | Fix | Commit |
|---|---|---|---|---|
| SR-10 | High (development installs) | lock-boundary | `install-session-units.sh` builds and installs `rmac-lock-provider` (`--features rmac-lock-provider-linux/provider`), keeps `rmac-lock-fallback.service`, and refuses to build or install anything while `/etc/pam.d/rmac-lock` is missing, printing the `sudo install` command. It re-checks that the provider, locker and both units are installed. Tests: `DevelopmentInstallLockTests` in `scripts/test_session_package.py`. | `57d61973` |
| SR-11 | Medium | notifications | Live notifications are capped at 100 and 4 MiB of payload per sender and 1024 and 32 MiB overall. A post past a cap closes the sender's (or, globally, anyone's) oldest non-urgent, non-persistent notification, hidden banners first, and reports it as an expiry (legacy `NotificationClosed` reason 1). When only urgent or persistent notifications could make room, the post fails with `LimitsExceeded`. A live entry whose banner is gone is released when history drops it. Tests: seven reducer tests in `crates/rmac-notifications/src/tests.rs`. | `5b8a0824` |
| SR-13 | Medium | dbus-polkit | Verified against keyd 2.5.0's source (Debian 2.5.0-5 does not patch it). The daemon runs as root, the socket is mode 0660 for group `keyd`, there is no peer check, `bind` accepts `command()` bindings that run `/bin/sh -c` as root, and `input`/`macro` inject keystrokes. So the group is root. No session is added to `keyd` any more, and the helper removes a leftover membership. The follower names one of three profiles on `/run/rmac-mac-keyboard.socket`. A socket-activated `DynamicUser` relay that holds only the `keyd` group (no capabilities, AF_UNIX only) accepts exactly `native`, `pc-app` or `terminal` and applies rmac's generated bindings, which a test proves never contain `command(`. See [ADR 0017](decisions/0017-mac-keyboard.md), "Revision". Tests: `parse_relay_request`, `no_profile_binding_can_run_a_command`, `MacKeyboardRelayTests`. | `27a1e644` |
| SR-12 | Medium | updates | The publisher's state no longer lives on Pages. Every release attaches `apt-inputs-<tag>.tar`, and every publication attaches `apt-snapshot-<id>.tar` (the signed metadata snapshot plus a sidecar naming each pool object's Release) to its Release. `scripts/linux/apt-publication.py collect` verifies the newest three snapshots with `gpgv` against the packaged keyring (pinned by `packaging/apt/archive-key.json`), checks every metadata file against the signed manifest, and re-fetches the pool from `SHA256SUMS`- and attestation-checked release inputs. `publish-apt-snapshot.py` therefore promotes onto the real previous repository, so the monotonic, immutable-pool and retention checks run. No history fails closed unless `RMAC_APT_FIRST_PUBLICATION` is set, and that flag is refused once history exists. The stager carries published versions forward byte for byte and refuses version regressions. A Release the repository serves is sealed against re-upload, and the `wget` mirror is gone. Tests: `scripts/test_apt_publication.py` (rebuilt repository equals the published one byte for byte; tampered bundle, missing attestation, SHA256SUMS mismatch, vanished pool object and backwards Date are all refused; an older version is refused; unchanged niri is carried forward), plus a real-APT check of a subkey-signed repository (`RealKeySigningTests`, run on the reference laptop). See `docs/update-trust.md` "Stateless publication from GitHub Releases". | `7121c7ff` |
| SR-14 | Low | updates | `rmac.pref` adds `Package: *` / `Pin: release o=rmac` / `Pin-Priority: -1`. The verifier, the `install.sh` heredoc and `docs/install.md` all match. Test: `test_the_rmac_origin_cannot_replace_other_packages`. The priority-500 list now also names `niri` and `xwayland-satellite`, which the repository publishes (`7121c7ff`). | `ff3653cf`, `c27dd2e1` |
| SR-16 | Low | packages | Every `ci.yml` action is pinned by a commit SHA, each checked against its tag through the GitHub API. `ci-quality.yml` pinned cargo-deny-action "v2.1.1" to the annotated tag object's SHA, not the commit; it now uses the commit (`3c634983`). The pin tests cover every workflow, and one label must map to one SHA. | `81f8f770` |
| SR-19 | Low | file-operations | The poppler tools run through `preview::bounded::run`: 60-second deadline, 256 MiB stdout cap, 16 KiB stderr, and the tool is killed and reaped on either. Tests in `crates/preview/src/bounded.rs`. | `abcd747c` |
| SR-20 | Low | desktop-entry-execution | `Path=` is used only when absolute (`platform::working_directory`). Test: `only_an_absolute_desktop_entry_path_becomes_the_working_directory`. | `cc492231` |
| SR-21 | Low | file-operations | The copy source is opened with `O_NOFOLLOW` and `O_NONBLOCK`, must be a regular file, and the mode comes from the opened file. Test: `copy_sources_are_opened_without_following_a_swapped_in_link`. | `1f5c7937` |
| SR-22 | Low | dbus-polkit | The Wi-Fi secret agent and the BlueZ pairing agent resolve the service's unique name at registration and reject every call from any other sender. An agent that never learned the name rejects all calls. Tests: `only_networkmanager_s_unique_name_is_a_valid_caller`, `only_bluez_s_unique_name_is_a_valid_caller`. | `ab1f31a3`, `90edb436` |
| SR-23 | Low | dbus-polkit | The safe-mode notice accepts `ActionInvoked` and `NotificationClosed` only from the unique name that answered `Notify`. | `70fa34ef` |
| SR-24 | Low | dbus-polkit | `OpenUri` accepts only a `file:///` URI of at most 8 KiB whose decoded path is absolute, has no `..` or NUL, has a playable extension and is an existing regular file. The playlist holds at most 10,000 items. Tests in `crates/player/src/playlist.rs`. | `855b68df` |
| SR-25 | Low | dbus-polkit | The notification, clipboard, Focus, FileChooser and Wallpaper portal service connections and both system-bus agents set a 5-second `method_timeout`. Guard: `test_service_connections_bound_outgoing_calls`. | `4ee32bd5` |
| SR-26 | Low | dbus-polkit | pkexec 127 is reported as "not authorised" and 126 as "cancelled" (`rmac_keyboard::command_failure`). Test: `pkexec_denial_is_not_reported_as_cancellation`. | `1ef3b352` |
| SR-27 | Low (documentation) | lock-boundary | `secure-lock-recovery.md` and `secure-lock.md` describe `rmac-lock-provider`, its watchdog and start limit, and the `OnFailure=` swaylock fallback, including how to restart the fallback from a TTY. Station proof is still owed (`tty-recovery-proven` stays pending). | `21c1234e` |
| SR-17 | Low (must-fix for Beta: update trust, and `--from-release` is the only install path until the archive key exists) | packages | `install.sh --from-release` refuses before downloading anything when `gh` is missing, because the packages' maintainer scripts run as root. `--allow-unattested` accepts `SHA256SUMS` alone and says what was not checked; it never skips a failing attestation and is refused outside `--from-release`. With `gh` the attestation stays mandatory and bound to `release.yml` (`072ccc09`). `docs/install.md` updated. Tests: `InstallReleaseAttestationTests` (refuses without gh and downloads nothing; the flag installs and warns; the flag cannot skip a failed attestation), `test_allow_unattested_applies_only_to_a_release_download`. | `65414a77` |
| SR-28 | Low (new, fresh pass) | lock-boundary | The lock provider read its picture caches (`~/.cache/rmac/lock-*.rgb`, LOCK-01) by checking the path with `symlink_metadata` and then `fs::read`ing it. A FIFO swapped in between would block the locker while it builds its surfaces; a file swapped in at another size was read in full. It now opens with `O_NOFOLLOW`, `O_NONBLOCK` and `O_NOCTTY`, checks the opened descriptor is a regular file of exactly the expected size, and reads at most one byte more (`crates/rmac-lock-provider-linux/src/picture.rs` `read_exact_plain_file`). Only the same user can write there, so this hardens the lock boundary rather than closing a cross-user hole. Tests: five `picture::tests` (exact size, other sizes, symlink, FIFO without a writer, directory). | `b49028c8`, `af238a44` |

Found and fixed in the 2026-10-04 pass (branch `op/security-stations`):

| ID | Severity | Boundary | Fix | Commit |
|---|---|---|---|---|
| SR-30 | Medium | dbus-polkit | Remote Login read and changed only `ssh.service`. Ubuntu socket-activates sshd, so `ssh.socket` can hold port 22 open while the service idles. The pane then showed Remote Login **off**, and turning it off returned "already off". Now either unit counts as on, and turning Remote Login off stops and disables `ssh.socket` before `ssh.service`, restoring both on failure. Test: `a_listening_ssh_socket_means_remote_login_is_on`. A station toggle is still owed (see `mutation-requires-authoritative-readback`). | `0420a7e6`, `c3ee7b60` |
| SR-31 | Low | notifications | While Focus state was unavailable, the fail-closed fallback still let urgent notifications show banners and play sounds. The fallback now also clears `allow_urgent_through_focus`. | `a0033cd5` |
| SR-32 | Low | notifications | The Focus policy call ran on the shared connection, which has no method timeout, so a hung Focus service stalled every `Notify`. The call is now bounded at 2 s and fails closed. | `9c0aedca` |
| SR-33 | Low | logs-diagnostics | Private paths reached the journal in four places: Preview's Move to Bin (`trash::Error` prints the path), poppler's first stderr line, the Dock's Stack item opener, and Files' launch-argument errors. All four now log kinds only. | `bb293d67` |
| SR-34 | Low | logs-diagnostics | Peer-supplied D-Bus error text from application menus was logged raw, so a same-user peer could forge journal lines or inject terminal escapes. It is now control-character-normalized and capped at 240 characters. Test: `peer_supplied_bus_error_text_cannot_forge_journal_lines`. | `bb293d67` |
| SR-35 | Low | logs-diagnostics | Preview's New from Clipboard copies (`$XDG_CACHE_HOME/rmac-preview/clipboard`) were never deleted, so copied screenshots piled up on disk. Closing the window now removes the copy, and copies left by exited processes are swept. Test: `only_a_new_from_clipboard_copy_is_removed_on_close`. | `bb293d67` |
| SR-36 | Low | logs-diagnostics | A real home Wi-Fi name appeared in a test fixture, `docs/parity.md` and two design-lab mocks. It is replaced with "Example Wi-Fi". | `31bf78db` |
| SR-37 | Low | packages | `release.yml` ran cargo-deny on the root workspace only, but `rmac-session` also ships the shell workspace's binaries. The release dependency gate now checks both. | `88529418` |

SR-29 (Low, updates) is closed. Its source fix ships in the tested
candidate `2f3ea7a4`, and the disposable station proved it natively against
the real PackageKit (station check `sr29-packagekit`, run 37176094048):
- the automatic set is simulated before anything is scheduled;
- a destructive plan is never scheduled;
- a stale plan is never applied (PackageKit discards it at refresh, and only
  a re-simulated plan is scheduled);
- a refused trigger and an untrusted repository both fail closed.

The `8ba31b82` build now on the reference laptop still predates the fix
until the laptop is updated.

The station also exposed a test-tool bug, which is now fixed:
`run-package-lifecycle.py` refused every real Ubuntu host, because Ubuntu
ships `/etc/os-release` as a symlink (`f032fb9a`, with a test).

Partly fixed; the rest stays open below:

- **SR-18** (`e16f8d5c`): the application and session package verifiers accept
  only the modes `0644` and `0755`. Tests:
  `test_manifest_cannot_claim_a_privileged_mode` (both packages). (`7121c7ff`):
  the publisher now verifies InRelease against the keyring inside the
  Release's own `rmac-archive-keyring` package, whose primary fingerprints must
  equal `packaging/apt/archive-key.json`, rather than a keyring exported from
  the signing secret. `sign-apt-release.sh` refuses a secret that carries the
  offline primary key.

## Open findings

| ID | Severity | Boundary | Evidence | Exploit scenario | Recommended fix |
|---|---|---|---|---|---|
| SR-15 | Low | packages | The 2026-10-04 station scanned the CI-built candidate `2f3ea7a4`. `verify-native-packages.py` passes and no personal home path appears. But `rmac-app-switcher`, `rmac-dock`, `rmac-mission-control` and `rmac-osd` embed the builder's checkout path (`/home/runner/work/lulo/lulo/shell/bins/…`) through `env!("CARGO_MANIFEST_DIR")` development-asset fallbacks, which `--remap-path-prefix` does not rewrite. The verifier's regex misses them because the path follows other text with no separator. | A local or reference-PC build embeds `/home/<user>/…` in those four binaries, and the release binaries probe a builder path for SVG assets at run time. | Put the source-tree fallbacks (app-switcher `packaged_icon`, Dock `packaged_icon`/`dock_asset_path`, mission-control `packaged_icon`, OSD `glyph_path`) behind `#[cfg(debug_assertions)]`, and drop the regex lookbehind in `verify-native-packages.py`. |
| SR-18 | Low (source fix pending native run) | packages | Release containers pin the reviewed Ubuntu 26.04 index digest. The rustup installer and `cargo-cyclonedx` source archive have checked SHA-256 pins in `release.yml`; focused workflow tests pass. Rust 1.95.0 toolchain artifacts remain version-selected, and no native release workflow has run with these changes. | Supply-chain drift or a broken release job. | Run the release workflow on native builders and review the resulting artifacts and provenance before closing this finding. |
| SR-38 | Low | notifications | Found 2026-10-04. The banner daemon (`crates/notification-center-app/src/daemon/host.rs:585-615`) ignores lock state. Banners are still created while the session is locked, and their title and body sit in an `assertive` live region; post sounds still play. | With Orca on and the screen locked, an incoming notification's content may be spoken to anyone nearby. The exclusive lock surface keeps it off the screen. | Subscribe to the lock state (`org.rmac.LockScreen1` or logind `LockedHint`) and suppress banners, live announcements and sounds while locked, keeping Center history. |

A read-only PackageKitGlib probe on the reference PC on 2026-09-28 found
`offline_get_action() == 3` (`UNSET`), so no offline update was scheduled at
that moment. It also found `offline_cancel_with_flags` and `OfflineFlags.NONE`
in the installed API. It did not invoke cancellation or test the new checker
on the native PackageKit service.

### Beta decision

Every open finding was classified against the Beta must-fix bar: exploitable
by another local user, network input, privilege escalation, secret or
password exposure, unsafe file handling on user data, or update trust.
SR-17 met it (update trust) and is fixed above. The rest are accepted for
Beta with this risk and mitigation; each has a Known issues note in
[known-limitations.md](known-limitations.md).

| ID | Severity | Decision | Risk | Mitigation until fixed |
|---|---|---|---|---|
| SR-15 | Low | Accept for Beta | A binary built on a person's machine may name that machine's home directory in panic locations; nothing else leaks. | Current native-build source remaps checkout and Cargo-home paths, and package verification scans inventoried ELF binaries. A fresh build still needs to pass that verification. Do not distribute locally built artifacts without checking them. |
| SR-18 | Low | Accept for Beta | A release build can still fail or drift in untested toolchain artifact selection; the new content-pinned workflow has not had a native run. | Release containers, rustup installer, and `cargo-cyclonedx` archive are content-pinned in source. Actions are pinned by commit SHA (SR-16), builds use `Cargo.lock` with `--locked`, `cargo-deny` gates advisories and licences (SR-05), outputs carry a workflow-bound provenance attestation that `install.sh` now requires (SR-17), and the APT publisher re-verifies every input (SR-12). |
| SR-38 | Low | Not yet decided: the owner decides (proposed: fix before Beta, because it exposes notification content while locked) | With a screen reader on, a locked session may speak an incoming notification's title and body. | Keep the screen reader off while away from a locked machine. Notification previews are already hidden by default, but that setting does not reach the banner daemon yet. |

**Informational, no severity:**

- Files rename accepts `/` (a move, with `RENAME_NOREPLACE`).
- Trash restore canonicalises the parent without re-checking the drive root; this is not exploitable (EXDEV and a uid check stop it).
- There is a tiny race in clipboard history between reading the offered types and reading the data.
- Archive expansion now budgets output against free space measured before extraction, subtracts a 1 GiB reserve, and limits one expansion to 16 GiB (8 GiB for tar.xz because its decoded tar and extracted files coexist). Concurrent disk writes can reduce that reserve; ordinary disk-full errors still clean up the staged output. This source change has passed scoped Mac tests but still needs native Linux validation before release.
- `LaunchSpec` derives `Debug` including its arguments; nothing logs it.
- The portal `ActionInvoked` signal is broadcast rather than addressed to the portal.
- `Request.Close` accepts any caller, as the reference backends do.
- The Wallpaper portal backend has no `.portal` routing file (a functional gap).
- The lock provider leaves wrong-password throttling to PAM (pam_unix delay and faillock).
- The panic-containment tests in the lock provider do not match the release `panic = "abort"`.
- Release notes (`crates/rmac-updates/src/notes.rs`) are read from any APT list whose InRelease says `Origin: rmac` and `Label: rmac`; another signed archive could claim those strings and supply notes for the exact offered `rmac-session` version. The text is bounded, has no control characters and is shown as plain text only.
- ⌘F5 toggles the screen reader while locked (as GNOME's lock screen and the Mac's login window allow). Orca then runs with the session's rights; the lock surface is exclusive, so only the lock screen is shown.
- The wallpaper process decodes the account picture (`~/.face` or AccountsService's `IconFile`) with no pixel cap; it is the user's own file and is never decoded by the lock provider.

## Fresh pass, 2026-09-25

Source review of the privileged or root-run code added on 2026-09-24/25:

| Area | Result |
|---|---|
| `packaging/rmac-session/system-sleep/rmac-input-resume` (root, on every resume) | No finding. Absolute tool paths, no environment or user input, reads only `/proc` and `/sys`, acts only in the `post` phase, each `modprobe` bounded by `timeout 5`. A device name a user could influence (for example through `uinput`) can at most make it skip the reload. |
| `packaging/rmac-session/debian/postinst`, `postrm` | Unchanged since the review; they touch only `/etc/keyd/rmac.conf` and only a file carrying rmac's header. |
| Power-key inhibitor (`rmac-session-supervisor hold-power-key`, `rmac-wayland-session`) and the coordinator's `power_key` | No finding. A logind `handle-power-key` block inhibitor tied to the wrapper by `PR_SET_PDEATHSIG` (parent re-checked after `prctl`) and stopped with the session; poweroff, restart and critical-battery actions are not blocked. A press locks before it sleeps; an unreadable `LockedHint` counts as unlocked, which is the safe side. |
| `rmac-process` (`bind_to_parent`) | No finding. The `pre_exec` closure makes only async-signal-safe calls, runs after std has set up stdio, and marks every descriptor from 3 up close-on-exec (`close_range`, `fcntl` fallback), which closes a descriptor leak into helpers. |
| Update flow (`rmac-updates`, `rmac-updates-linux`, `rmac-update-check`) | SR-29 (above). Download-only and offline-trigger calls need no polkit prompt under PackageKit's own policy; every transaction keeps `ONLY_TRUSTED`; Settings re-simulates and compares the plan before downloading; the settings file is read with `O_NOFOLLOW`, bounded and defaulting on; the status file is written private and atomic; error lines carry classes, not messages. Release notes: informational note above. |
| `rmac-dbus` shared connection | No finding. Only client calls use the shared connections; every exported service (portals, agents, notifications, clipboard, Focus) still builds its own connection with a 5-second `method_timeout` (SR-25), and the sender checks (SR-22, SR-23) read each message's header, so sharing does not widen who can call them. |
| Lock screen pictures (LOCK-01) | SR-28, fixed. |
| `install-native-candidate.sh`, `install.sh` niri version changes | No finding: install scripts that compare versions with `dpkg --compare-versions` and keep a newer package instead of downgrading. |

## Per-check verdicts

Legend:
- **pass**: the source review is complete and no finding is open against the check.
- **pending (native)**: the check needs station or adversarial execution.
- **pending (SR-nn)**: a finding is open against the check.

### desktop-entry-execution
| Check | Verdict | Evidence |
|---|---|---|
| exact-field-code-expansion | pass | `crates/rmac-apps/src/platform.rs:666-700`: file and URL codes are dropped, `%i`, `%c`, `%k` and `%%` are expanded, and any unknown `%` rejects the entry |
| executable-boundary-preserved | pass | argv is spawned directly (`crates/rmac-apps/src/catalog.rs:249-263`); Open With goes through `gio launch` with argv (`:139-144`) |
| hidden-tryexec-precedence | pass | the id is claimed before parsing (`platform.rs:148-151,193-203`); TryExec is checked by path or PATH plus the executable bit (`:642-664`) |
| launch-diagnostics-redacted | pass | `crates/rmac-app-launch/src/application.rs` returns kinds only |
| no-shell-interpolation | pass | only two `sh -c` uses in the tree, both constant scripts with positional arguments (`crates/rmac-clipboard-linux/src/wayland.rs:25,47`, `shell/compat/gpui_linux/src/linux/platform.rs:305-321`) |
| terminal-wrapper-argument-boundary | pending (native) | rmac keeps argv after `-e` intact (`catalog.rs:283-334`) and the session never sets `TERMINAL`; x-terminal-emulator is `/usr/bin/ptyxis` on the reference laptop. Station `terminal-wrapper` has not yet run Ptyxis to completion (see Station evidence). |
| working-directory-validated | pass | only an absolute `Path=` becomes the working directory (`crates/rmac-apps/src/platform.rs` `working_directory`); SR-20 fixed |

### dbus-polkit
| Check | Verdict | Evidence |
|---|---|---|
| broadcasts-contain-no-secrets | pass | the Center, Clipboard and LockScreen `Changed` signals carry counts or policy only; Clipboard history skips password-manager offers (`crates/rmac-clipboard-linux/src/service.rs:254-259`) |
| bounded-call-time-and-output | pass | nmcli and helper output is bounded (`crates/rmac-network/src/vpn_import.rs:343-356`, `crates/rmac-privacy-linux/src/security.rs:33-80`); service and agent connections set a 5-second `method_timeout` (SR-25 fixed) |
| denial-and-cancel-distinct | pass | pkexec 126 is "cancelled", 127 is "not authorised" (`rmac_keyboard::command_failure`); SR-26 fixed |
| interactive-authorization-only-from-user-action | pass | 2026-10-04 trace: rmac never sets the message flag. Every `interactive=true` argument (hostnamed, timedated, localed, `SetX11Keyboard`) and the only `pkexec` come from a Save, Apply, switch or confirmation. The PackageKit interactive install has no caller, and the automatic path uses `interactive=false`. The one background call, NetworkManager `RequestScan`, is `allow_active`/`allow_inactive: yes` on the reference laptop (policy file read 2026-10-04), so it cannot prompt. Station: `polkit-policy` |
| mutation-requires-authoritative-readback | pending (native) | hostname, time, locale, Bluetooth, Wi-Fi, VPN and updates re-read the authority. Sharing waits for systemd and re-snapshots, and SR-30 (ssh.socket) is fixed. A Remote Login toggle on a real session is still owed (Reference-laptop checks; functional issue F-1) |
| no-credential-collection | pass | admin credentials only through polkit agents; the keyboard helper takes enumerated flags only (`crates/rmac-keyboard/src/helper.rs:24-48`); Wi-Fi secrets are zeroized with redacted Debug (`crates/rmac-network/src/model.rs:191-260`) |
| system-bus-callers-treated-untrusted | pass | the NetworkManager secret agent and BlueZ pairing agent accept calls only from the service's unique name, resolved at registration; SR-22 fixed |
| unique-owner-revalidated | pass | the portal backends compare the caller with the owner of `org.freedesktop.portal.Desktop` (`crates/rmac-file-chooser/src/dbus.rs:245-266`); the safe-mode notice accepts answers only from the server that answered `Notify` (SR-23 fixed) |

### portals
| Check | Verdict | Evidence |
|---|---|---|
| backend-routing-verified | pass | `rmac.portal`, `rmac-file-chooser.portal` and `rmac-portals.conf`; every call checks the portal frontend's owner (file chooser `dbus.rs:245-266`, notifications `service.rs:1423`) |
| document-access-remains-scoped | pass | only `file://` URIs are returned; the frontend creates the document grants (ADR 0012) |
| permissionstore-does-not-overclaim | pass | only the `devices` table is read, and a reset is read back (`crates/rmac-privacy-linux/src/portal.rs:45-90`) |
| portal-mediates-user-consent | pass | FileChooser results come only from the panel; Wallpaper always shows a preview step (`crates/rmac-wallpaper-portal/src/broker.rs:95,360-378`) |
| request-cancel-and-close-bounded | pass | `Request.Close` is exported before the panel opens, at most 8 live panels (file chooser `dbus.rs:94-119`, `service.rs:16,92-101`); Wallpaper holds at most 8 requests and 256 MiB |
| response-handle-correlated | pass | reused handles are rejected (`dbus.rs:108-112`); the client uses ashpd's request/response |
| selected-uri-revalidated | pass | re-checked at return time: absolute path, no `..`, still exists, correct kind (`crates/rmac-file-chooser/src/outcome.rs:65-116`); Wallpaper staging uses `O_NOFOLLOW` |

### file-operations
| Check | Verdict | Evidence |
|---|---|---|
| atomic-save-and-authoritative-readback | pass | `crates/text-editor/src/storage.rs:115-145`; `crates/rmac-storage/src/read.rs:120-165` (create-new temporary file, fsync, rename, fsync of the folder) |
| cancellation-preserves-recoverable-destination | pass | staged, journaled copy and move (`crates/finder/src/file_ops.rs:1869-2313`); archive staging is removed on cancel |
| conflict-refuses-stale-overwrite | pass | `RENAME_NOREPLACE` (`file_ops.rs:499-511`, `crates/rmac-archive/src/staging.rs:101`) |
| mount-disappearance-recovers | pending (native) | source supports it: the mount watcher drops lost roots and moves tabs Home with a notice (`mount_controller.rs:54-142`), and a cross-volume move removes the source only after a synced copy (`file_ops.rs:1066-1110`). No test simulates ENODEV/EIO; the station steps are in Reference-laptop checks |
| private-path-diagnostics-redacted | pass | Trash errors leave out storage paths (test at `file_ops.rs:1784`) |
| symlink-and-root-boundaries-enforced | pass | SR-03 and SR-21 fixed; the copy source is opened with `O_NOFOLLOW`; copy recreates links rather than following them; `.trashinfo` decoding rejects NUL, absolute paths and `..` (`trash_store.rs:903-968,1072-1085`) |
| trash-and-destructive-actions-confirmed | pass | `crates/finder/src/view/permanent_delete_controller.rs:51-88` |
| untrusted-content-never-executed | pass | Files opens through the OpenURI portal or `xdg-open` (`crates/rmac-portal/src/open.rs`). Station `untrusted-open`: eight executable and `.desktop` fixtures opened through that path executed nothing |

### lock-boundary
| Check | Verdict | Evidence |
|---|---|---|
| compositor-exclusive-lock-proven | pending (native) | readiness only after `locked` (`crates/rmac-lock-provider-linux/src/wayland.rs:1661-1665`); `finished` handled (`:1667-1680`); `unlock_and_destroy` only from `Locked` (`:1151-1159`); niri holding the lock after the client dies is not provable from source |
| mfa-conversation-bounded | pass | 1..=32 messages of at most 512 B each; responses at most 512 B with no NUL (`src/pam/callback.rs:11-12,68,84,166-169,229-246`); one prompt at a time (`pam_broker.rs:27,338-349`) |
| no-password-or-keycode-logging | pass | only fixed-string `eprintln!`; every secret-bearing type has a redacted Debug (`secret.rs:88,113`, `keyboard.rs:29,69`, `pam_conversation.rs:39,78`, `pam_broker.rs:197`) |
| pam-is-sole-unlock-authority | pass | `UnlockAuthorization` is minted only on success (`crates/rmac-lock-provider/src/model.rs:82-85`, `provider.rs:127-135`); requires `pam_authenticate(PAM_DISALLOW_NULL_AUTHOK)`, then `acct_mgmt`, then `pam_end` (`src/pam/transaction.rs:129-173`); no D-Bus, env or timer unlock; `pam/rmac-lock` includes `common-auth` and `common-account` only |
| provider-crash-fails-closed | pending (native) | SR-02, SR-09 and SR-10 fixed; only an authenticated unlock exits 0 (`development_process.rs:67-87`); watchdog SIGKILL and restart |
| secret-lifetime-and-zeroization-reviewed | pass | fixed-capacity `SecretInput`, zeroized on edit and drop (`secret.rs:13-80`); a single calloc copy for libpam, wiped with `explicit_bzero` (`pam/callback.rs:67-79,113-135`); no `CString` of the secret; cores disabled (SR-09) |
| suspend-waits-for-lock-readiness | pending (native) | sleep delay inhibitor released only after a successful start (`crates/rmac-shortcuts/src/lock.rs:326-334,357-365`); SR-02 fixed |
| tty-recovery-proven | pending (native) | the runbook matches the installed units (SR-27 fixed); needs station proof |
| wrong-password-and-cancel-remain-locked | pass | failure and cancel return to `Locked` (`provider.rs:136-148`); the cancelled worker is drained (`runtime.rs:293-317`) |

### notifications
| Check | Verdict | Evidence |
|---|---|---|
| action-target-bound-to-notification | pass | owner checked on replace and withdraw (`crates/rmac-notifications/src/reducer.rs:36,85`); document-open requires a portal source and a matching app (`crates/rmac-notifications-linux/src/service.rs:1362-1400`) |
| diagnostics-redacted | pass | 2026-10-04 trace: the notification crates log nothing, and `notification-center-app` prints only `Copy` error kinds and fixed strings. No summary, body, app name, icon path, action ID or sender is printed |
| focus-suppression-authoritative | pass | the server asks Focus at delivery time (`service.rs` `policy` → `org.rmac.Focus1.DeliveryPolicy`) and fails closed. SR-31 (urgent posts no longer bypass the fallback) and SR-32 (2 s bound) are fixed |
| history-and-payload-bounded | pass | payload limits (`crates/rmac-notifications/src/lib.rs:20-29`); history 500 records, 100 per app, 8 MiB; live notifications 100 and 4 MiB per sender, 1024 and 32 MiB overall (SR-11 fixed) |
| lock-screen-content-redacted | pending (SR-38) | the lock provider has no notification path and `LockPreview` defaults to `Hide`, but the banner daemon ignores lock state (SR-38). A station check is in Reference-laptop checks |
| markup-treated-as-untrusted | pass | legacy body is plain text; portal markup is reduced to inert text, capped at 64 KiB (`decode.rs:29-31,196-270`); image paths are ignored and portal media accepted only as sealed memfds (`media.rs:749-790`) |
| sender-attribution-not-invented | pass | identity is the unique bus name; the claimed `app_name` is ignored (`origin.rs:1-10`) |

### search-indexing
| Check | Verdict | Evidence |
|---|---|---|
| allowed-roots-only | pass | `crates/rmac-launcher-providers/src/files.rs:163-173` |
| cancellation-rejects-stale-results | pass | `crates/rmac-launcher/src/session.rs:135-143`; Notes `SearchGeneration` |
| excluded-roots-never-indexed | pass | `crates/rmac-search/src/platform.rs:136-140,226,275`; `files.rs:166-169` |
| private-path-diagnostics-redacted | pass | error detail holds paths but has no log sink; `Row` Debug is redacted |
| query-and-content-bounded | pass | `crates/rmac-search/src/lib.rs:23-28`; `crates/rmac-notes-runtime/src/search.rs:9` |
| result-count-bounded | pass | limit 500 in the provider and in Notes (`search.rs:73-74`) |
| stored-index-does-not-expand-authority | pass | results are re-checked with `path_allowed`; opening goes through the portal |

### packages
| Check | Verdict | Evidence |
|---|---|---|
| architecture-and-file-inventory-exact | pass | `scripts/linux/verify-native-packages.py:390-436` (see SR-18 for the mode check) |
| artifact-contains-no-build-host-data | pending (SR-15) | station `candidate-provenance`: no personal home path in any rmac file, but four shell binaries embed the builder's checkout path through `CARGO_MANIFEST_DIR` (SR-15) |
| dependency-and-advisory-policy-passes | pass | cargo-deny `check` (advisories, licences, bans, sources) passed for the root workspace (`Dependency policy`) and the shell workspace (`Current GPUI Linux runtime gate`) on the candidate commit `2f3ea7a4` (CI run 37152104376). SR-37: `release.yml` now gates both workspaces |
| license-inventory-complete | pending (native) | Rust dependencies are covered by cargo-deny. The provenance of the non-Rust assets (wallpapers, sounds, cursors, icons, brand, greeter art) needs the owner's record (Reference-laptop checks) |
| native-and-sandbox-boundaries-explicit | pass (source policy) | `packaging/flatpak/decisions.json` covers all thirteen packaged apps and `verify-flatpak-package.py` binds each decision to its desktop executable; only Text Editor has a reviewed Flatpak manifest with `--socket=wayland --device=dri`. Maintainer scripts touch only `/etc/keyd/rmac.conf` (`packaging/rmac-session/debian/postinst`, `postrm`). Native station proof remains separate. |
| rollback-and-uninstall-tested | pending (native) | station `install-effects`: install and purge are exact, and the maintainer scripts touch only `/etc/keyd/rmac.conf`. The baseline → candidate upgrade, interrupted rollback, remove, purge and reinstall cycle (`run-package-lifecycle.py`) has not completed on the station yet. |
| signature-and-origin-claims-bounded | pass | SR-14 and SR-17 fixed: the release attestation is mandatory and workflow-bound; without `gh` only an explicit `--allow-unattested` installs |
| unpackaged-executables-rejected | pass | `verify-native-packages.py:430-436`; no setuid in source |

### updates
| Check | Verdict | Evidence |
|---|---|---|
| apt-key-scope-isolated | pass | `Signed-By` keyring (`packaging/apt/rmac.sources.in:6`); `verify-update-trust.py:201-206` rejects `trusted=yes` and `trusted.gpg.d`; SR-01 fixed |
| atomic-inrelease-last-and-monotonic | pass | InRelease written last with `os.replace` and fsync (`publish-apt-snapshot.py` `promote`); the Date/snapshot check now runs against the repository rebuilt from the retained signed snapshots (SR-12 fixed) |
| backend-failure-recovers-authoritatively | pass | re-read after install (`crates/system-settings/src/controller/software_updates.rs:194-195`) |
| cancellation-and-restart-readback | pass | `crates/rmac-updates-linux/src/transaction.rs:255-301` |
| immutable-pool-and-by-hash | pass | the rebuilt repository carries the retained pool and by-hash objects byte for byte; the stager carries published versions forward and refuses a published path with new bytes; published Releases are sealed (SR-12 fixed) |
| keyring-public-only-and-package-scoped | pass | `keyring_package_contract.py:237-238,631-633`; SR-01 fixed |
| packagekit-invoked-without-shell | pass | zbus only (`crates/rmac-updates-linux/src/transaction.rs:327-389`) |
| polkit-interaction-is-user-initiated | pass | `interactive=true` only on install, reached only from `confirm_update_plan` (`transaction.rs:217`) |
| signature-failure-fails-closed | pass | `crates/rmac-updates/src/normalize.rs:16-31,64-67`; `transaction.rs:480` |
| trusted-only-install-enforced | pass | `FLAG_ONLY_TRUSTED` (`transaction.rs:144,221`) |
| update-error-states-privacy-safe | pass | `crates/rmac-updates/src/normalize.rs:41`; `transaction.rs:497-519` |

### logs-diagnostics
The 2026-10-04 source pass is complete:
- **Sinks:** it reviewed all 508 logging sink lines in both workspaces: about
  403 `eprintln!` lines and about 10 `println!` lines in CLIs, plus 95
  `log::` calls in vendored GPUI.
- **Logger:** no global logger is installed in either workspace, so `log::`
  and `tracing::` calls are no-ops. Only stderr reaches the journal.
- **Debug:** it checked 1,845 derived and 350 hand-written `Debug` impls.
- **Scripts:** it covered every shipped script.

The station's `journal-redaction` check is the native half.

| Check | Verdict | Evidence |
|---|---|---|
| bus-peers-and-session-identities-redacted | pass | no sink prints a sender, unique name, uid, session or seat; station: no unique names in rmac journal lines |
| control-characters-normalized | pass | SR-34 fixed (application-menu peer text); Files no longer echoes raw launch arguments (SR-33); poppler text no longer leaves Preview; station: no control characters |
| debug-implementations-redact-private-fields | pass | secret-bearing types (Wi-Fi and Enterprise credentials, mail `Secret`, OAuth attempt, lock `SecretInput`) have redacted or no `Debug`; derived `Debug` on content types is never formatted into a sink |
| evidence-uses-synthetic-data | pass | no tokens or keys in fixtures or evidence. SR-36 removed a real Wi-Fi name. Informational: docs and older perf/evidence JSON still name the reference laptop's RFC 1918 address, its login, and `/home/<owner>` paths; these are the project owner's own public author identity |
| failure-text-and-output-bounded | pass | poppler output capped (`bounded.rs`); `rmac-update-check` logs class tokens cut to 200 characters; application-menu peer text capped at 240 (SR-34); station: longest rmac line 134 characters |
| log-growth-and-retention-bounded | pass | rmac writes no log files; histories are capped (clipboard 100/64 MiB, notifications 500/8 MiB, recents, launcher learning, Files undo journal); SR-35 fixed the unbounded Preview clipboard cache |
| no-private-paths-or-content | pass | SR-33 fixed the four path leaks; no sink logs clipboard, notification, search, document or URL content; station: planted private path and package name absent |
| no-secrets-credentials-or-tokens | pass | lock and PAM code logs fixed strings; Wi-Fi/VPN secrets are redacted and zeroized; station: planted password absent from the journal |

## Reference-laptop checks

These need the reference laptop's real hardware, a live Lulo session, or a
real lock screen. Agents may not lock the laptop, suspend it, switch VTs or
touch the owner's session, so the owner runs them, or a later run on an
equivalent disposable machine that has a display. Every step is read-only
apart from the action it names. Keep raw logs and screenshots out of Git.

| Check | Steps | Pass when |
|---|---|---|
| lock-boundary / compositor-exclusive-lock-proven | Lock with the shortcut. From a second machine, run `ssh <station> journalctl --user -u rmac-lock.service -b -n 50`. From a local TTY (Ctrl+Alt+F3), run `pkill -KILL -x rmac-lock-provider`, then return to the session VT. | The journal shows provider readiness only after `locked`. After the kill, niri keeps the session hidden, and `rmac-lock-fallback.service` (swaylock) appears through `OnFailure=`. No desktop frame shows at any point. |
| lock-boundary / provider-crash-fails-closed | As above. Also run `scripts/linux/run-lock-provider-recovery-gate.sh --execute` from a local graphical login (it refuses SSH). | The gate script passes; a crash or hang never unlocks; only a successful PAM authentication exits 0. |
| lock-boundary / suspend-waits-for-lock-readiness | With the session unlocked, choose Sleep from the menu bar. Resume, then run `journalctl --user -u rmac-lock-coordinator -u rmac-lock -b -o short-monotonic`. | The lock-ready line comes before logind's `PrepareForSleep`, and the first frame after resume is the lock screen. |
| lock-boundary / tty-recovery-proven | Follow [secure-lock-recovery.md](secure-lock-recovery.md) from Ctrl+Alt+F3 with the provider deliberately stopped. | Every runbook command works as written. The disposable station already proved the units, PAM service and swaylock it names are installed as documented (`lock-units`). |
| notifications / lock-screen-content-redacted | Lock, with Orca off and then on. From a TTY in the same session, post `notify-send -u critical 'Synthetic title' 'Synthetic body'`. | Nothing but the lock surface is visible, and nothing is spoken. This stays pending until SR-38 is fixed. |
| file-operations / mount-disappearance-recovers | Make a 64 MiB image with `truncate -s 64M /tmp/sr.img; mkfs.vfat /tmp/sr.img`, then run `udisksctl loop-setup -f /tmp/sr.img` and `udisksctl mount -b /dev/loopN`. In Files, start a large copy into the volume and a cross-volume move out of it, then run `udisksctl unmount -f -b /dev/loopN` mid-transfer. Browse inside the mount while it disappears. | The source hashes are unchanged, the error says the source was kept, no hidden staging is left after a remount, and a tab inside the mount returns to Home with the disconnect notice. |
| dbus-polkit / mutation-requires-authoritative-readback | With `openssh-server` installed, run `systemctl is-enabled ssh.socket ssh.service; ss -ltn 'sport = :22'`. Toggle System Settings > General > Sharing > Remote Login on and off, and run the same commands after each toggle. Also toggle Date & Time > Set automatically. | The pane always matches systemd and the listening socket (SR-30). Today the Sharing calls carry no interactive-authorization flag, so they are likely to fail as "denied" (functional issue F-1 below). |
| packages / license-inventory-complete | The owner confirms the provenance of the non-Rust assets that the packages ship (`packaging/rmac-session/wallpapers`, `assets/sounds`, `assets/cursors`, `assets/icons`, `assets/brand`, the greeter artwork) and records it in a `packaging/rmac-session/LICENSES.md`, as `packaging/rmac-apps/LICENSES.md` already does for the apps. | Every shipped non-ELF file falls under a recorded licence. |

**Functional issues (not security findings).**
F-2: Ubuntu 26.04's `appstreamcli validate --no-net` rejects at least one
installed rmac metainfo file, so `verify-application-package.py
--installed-host`, and with it the package lifecycle, cannot pass on a clean
install. Run the validator on `packaging/rmac-apps/metainfo/*.xml` on
Ubuntu 26.04 and fix what it reports.

F-1: the Sharing systemd1 calls (`crates/rmac-sharing-linux/src/system.rs`
`system_set_service`, `restore_service`) send no
`ALLOW_INTERACTIVE_AUTHORIZATION` flag. With systemd's default
`auth_admin_keep`, polkit cannot prompt, so turning Remote Login or File
Sharing on or off is likely to always fail as "denied or cancelled". Every
call comes from a toggle, so the fix is to send these calls with
`MethodFlags::AllowInteractiveAuth`.

## What blocks Beta

- **Security gate: Fail.** The gate itself is never waived. It passes only
  when `open_findings` is empty, all 80 checks are `pass`, and every Beta
  station has run. The one exception is a station with a recorded owner
  waiver: NVIDIA, for Beta 1.
- **Nothing Critical or High is open.** SR-10 is fixed, but a development
  install made before it may still lack the provider or its PAM service; rerun
  `install-session-units.sh` before taking lock evidence there.
- **SR-13 changed a privileged boundary.** The disposable station proved the
  relay side (`keyboard-relay`):
  - A valid profile is applied (`ok`).
  - Six malformed requests are refused: a `command()` binding, shell
    metacharacters, no newline, two lines, a NUL byte and 70 KB.
  - No command ran, the session user is not in `keyd`, and the relay never
    logs the request.

  Per-app switching under niri still needs the reference laptop.
- **SR-12 is fixed**, but the stateless publication has only run against
  fixtures and a local APT client; the first real tag must prove it on
  GitHub Actions and Pages.
