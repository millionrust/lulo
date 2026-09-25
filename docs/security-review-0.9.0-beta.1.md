# Security review — 0.9.0-beta.1 (source review)

This is the I6 security and privacy review
([security-release-review.md](security-release-review.md)) for the
0.9.0-beta.1 early-access release, run against the 80 checks in
`scripts/security-review.json`. The canonical summary sits beside this file
as [security-review-0.9.0-beta.1.json](security-review-0.9.0-beta.1.json).

**Gate status: Fail.** The source review is done: every one of the 10 domains
was read, 9 findings were fixed during the review and 17 more after it (the
tables below give each fix's commit), with regression tests wherever code changed, and nothing
Critical or High is open. Three things still block the gate:

- 3 findings are still open, all Low, each accepted for Beta with a
  documented risk and mitigation ("Beta decision" below) and a note in
  [known-limitations.md](known-limitations.md). Accepted is not closed: the
  verifier still counts them.
- 24 checks need native execution on real stations.
- None of the three Beta H8 stations has been run. `amd64-nvidia-desktop`
  does not exist yet.

The verifier therefore fails, as it should:

```sh
python3 scripts/verify-security-review.py \
  --evidence docs/security-review-0.9.0-beta.1.json \
  --tier beta --revision <reviewed revision in the JSON>
# verify-security-review: security evidence open_findings differs
```

## Scope and method

- **Reviewed tree:** `dev` at 5bea40ca, plus the fix commits listed below,
  plus a fresh pass over the privileged code added on 2026-09-24/25 (see
  "Fresh pass, 2026-09-25"). The JSON's `revision` is the last fix commit
  (`af238a44`). The post-review fixes were checked with the repository's Python
  suites and `rustfmt`, with `rustc` unit-test builds of the changed pure
  modules on macOS, and with Linux-configuration type checks of
  `rmac-keyboard`, `rmac-network` and `rmac-bluetooth` against local rlibs. No
  cargo build ran; the coordinator's Linux build is still owed.
- **Method:** source-only review with `file:line` evidence. There was no
  cargo build, no station run and no adversarial execution on Ubuntu 26.04.
  The review includes the crates, packaging, maintainer scripts, install and
  uninstall scripts, and the GitHub workflows.
- **Priorities:** in order, the lock screen and PAM; privileged helpers
  (pkexec/polkit, keyd, localed/timedated/hostnamed, systemd units for
  Sharing); portals and every `org.rmac.*` D-Bus name and runtime socket;
  clipboard history and network fetches; install, update and release; and
  Files operations.
- **Not reviewed:** CalDAV, which is not merged.
- **JSON `results`:** `pass` means the source review is complete for that
  check and no finding against it is open. `pending` means the check needs
  native or station evidence, or has an open finding. `pass` never claims
  native proof. `stations` all remain `pending`.

Of the 80 checks, 56 are `pass` and 24 are `pending` (the JSON is canonical).

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
| SR-15 | Low | packages | There is no `--remap-path-prefix`. Panic locations keep `$CARGO_HOME/…` and `../crates/*` absolute paths (`Cargo.toml:162-167` strips symbols only). | Local and reference-PC builds embed `/home/<user>/…`. `check-native-reproducibility.sh` builds twice on one host, so it cannot catch this. | Remap `$CARGO_HOME` and the repository root in `build-native-inputs.sh`, and scan packaged binaries for `/home/` and `/Users/`. Not done here: it changes every release binary and needs a build to verify. |
| SR-18 | Low (partly fixed) | packages | rustup is installed by `curl \| sh`, the `ubuntu:26.04` container is pinned by tag, and `cargo install` is pinned by version only (`release.yml`). | Supply-chain drift. | Pin by digest or hash. (The manifest mode check and the publisher's keyring source are fixed.) |
| SR-29 | Low (new, fresh pass) | updates | `scripts/linux/rmac-update-check` schedules the automatic set (Lulo OS and security updates) as a PackageKit offline update with a download-only `UpdatePackages` and `offline_trigger`, without a `SIMULATE` pass. System Settings' Update Now simulates and stops a plan with removals for confirmation (`rmac-updates-linux` `packagekit_prepare_offline`); the daily run does not. | An update whose dependencies need a removal is applied unattended at the next restart. It is still a trusted, signed package from a configured archive, so this is data safety, not an authenticity gap. | Simulate the automatic set with `ONLY_TRUSTED \| SIMULATE` first and leave it for review in System Settings when the plan removes or obsoletes anything. |

### Beta decision

Every open finding was classified against the Beta must-fix bar: exploitable
by another local user, network input, privilege escalation, secret or
password exposure, unsafe file handling on user data, or update trust.
SR-17 met it (update trust) and is fixed above. The rest are accepted for
Beta with this risk and mitigation; each has a Known issues note in
[known-limitations.md](known-limitations.md).

| ID | Severity | Decision | Risk | Mitigation until fixed |
|---|---|---|---|---|
| SR-15 | Low | Accept for Beta | A binary built on a person's machine names that machine's home directory in panic locations; nothing else leaks. | Release `.deb`s are built only by `release.yml` on GitHub runners, whose paths name no person; locally built packages are not distributed. |
| SR-18 | Low | Accept for Beta | A compromised rustup script, `ubuntu:26.04` tag or crates.io release of a build tool could reach a release build. | Actions are pinned by commit SHA (SR-16), builds use `Cargo.lock` with `--locked`, `cargo-deny` gates advisories and licences (SR-05), outputs carry a workflow-bound provenance attestation that `install.sh` now requires (SR-17), and the APT publisher re-verifies every input (SR-12). |
| SR-29 | Low | Accept for Beta | An automatic update that needs a package removal happens at restart without a confirmation. | Only `ONLY_TRUSTED` packages from signed archives are scheduled; the automatic set is limited to Lulo OS's five packages and packages PackageKit marks as security updates; turning off the Automatic Updates switches (or the timer) stops it; Update Now in System Settings simulates and asks. |

**Informational, no severity:**

- Files rename accepts `/` (a move, with `RENAME_NOREPLACE`).
- Trash restore canonicalises the parent without re-checking the drive root; this is not exploitable (EXDEV and a uid check stop it).
- There is a tiny race in clipboard history between reading the offered types and reading the data.
- There is no cap on the expanded size of an archive (same as the Mac).
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
| terminal-wrapper-argument-boundary | pending (native) | rmac keeps argv after `-e` intact (`catalog.rs:283-318`); whether the terminal re-parses it depends on `x-terminal-emulator` |
| working-directory-validated | pass | only an absolute `Path=` becomes the working directory (`crates/rmac-apps/src/platform.rs` `working_directory`); SR-20 fixed |

### dbus-polkit
| Check | Verdict | Evidence |
|---|---|---|
| broadcasts-contain-no-secrets | pass | the Center, Clipboard and LockScreen `Changed` signals carry counts or policy only; Clipboard history skips password-manager offers (`crates/rmac-clipboard-linux/src/service.rs:254-259`) |
| bounded-call-time-and-output | pass | nmcli and helper output is bounded (`crates/rmac-network/src/vpn_import.rs:343-356`, `crates/rmac-privacy-linux/src/security.rs:33-80`); service and agent connections set a 5-second `method_timeout` (SR-25 fixed) |
| denial-and-cancel-distinct | pass | pkexec 126 is "cancelled", 127 is "not authorised" (`rmac_keyboard::command_failure`); SR-26 fixed |
| interactive-authorization-only-from-user-action | pending (native) | `interactive=true` is set on SetStaticHostname, SetTimezone, SetLocale and PackageKit install (`crates/rmac-system-info/src/host.rs:182`, `crates/rmac-time-linux/src/system.rs:79`, `crates/rmac-locale-linux/src/system.rs:281`, `rmac-updates-linux` `transaction.rs:217`); not every System Settings call site was traced back to a user action |
| mutation-requires-authoritative-readback | pending (native) | hostname verified after the write (`rmac-system-info` `verify_static_hostname`), VPN edits re-read (`vpn_editor.rs:300-317`), updates re-snapshot; Sharing and systemd unit changes need station proof |
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
| mount-disappearance-recovers | pending (native) | |
| private-path-diagnostics-redacted | pass | Trash errors leave out storage paths (test at `file_ops.rs:1784`) |
| symlink-and-root-boundaries-enforced | pass | SR-03 and SR-21 fixed; the copy source is opened with `O_NOFOLLOW`; copy recreates links rather than following them; `.trashinfo` decoding rejects NUL, absolute paths and `..` (`trash_store.rs:903-968,1072-1085`) |
| trash-and-destructive-actions-confirmed | pass | `crates/finder/src/view/permanent_delete_controller.rs:51-88` |
| untrusted-content-never-executed | pending (native) | Files never executes anything itself; opening goes through the OpenURI portal or `xdg-open` (`crates/rmac-portal/src/open.rs:41-52`); handler behaviour for `.desktop` files and executables needs station proof |

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
| diagnostics-redacted | pending (native) | not traced end to end in this pass |
| focus-suppression-authoritative | pending (native) | not traced in this pass |
| history-and-payload-bounded | pass | payload limits (`crates/rmac-notifications/src/lib.rs:20-29`); history 500 records, 100 per app, 8 MiB; live notifications 100 and 4 MiB per sender, 1024 and 32 MiB overall (SR-11 fixed) |
| lock-screen-content-redacted | pending (native) | the policy defaults to `Hide`, but no lock-screen client renders notifications yet |
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
| artifact-contains-no-build-host-data | pending (SR-15) | |
| dependency-and-advisory-policy-passes | pending (native) | `deny.toml` is sound; SR-05 fixed; the release does not deny-check the `shell/` workspace (`ci.yml:278-284` does); cargo-deny was not run here |
| license-inventory-complete | pending (native) | Rust dependencies are covered; non-Rust assets are not established |
| native-and-sandbox-boundaries-explicit | pass | Flatpak `finish-args` are `--socket=wayland --device=dri` only; maintainer scripts touch only `/etc/keyd/rmac.conf` (`packaging/rmac-session/debian/postinst`, `postrm`) |
| rollback-and-uninstall-tested | pending (native) | |
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
This domain was only spot-checked, so all 8 checks stay **pending (native)**.
The spot checks found nothing: the lock provider has redacted Debug
throughout and fixed-string logging, the Wi-Fi Enterprise credentials have a
redacted Debug and are zeroized on drop, and the keyboard helper's errors
carry only the helper's own stderr tail. A full pass over every `eprintln!`
and `Debug` derive, plus journal inspection on a station, is still required:

- bus-peers-and-session-identities-redacted
- control-characters-normalized
- debug-implementations-redact-private-fields
- evidence-uses-synthetic-data
- failure-text-and-output-bounded
- log-growth-and-retention-bounded
- no-private-paths-or-content
- no-secrets-credentials-or-tokens

## What blocks Beta

- **Security gate: Fail.** The gate is never waived. It passes only when
  `open_findings` is empty, all 80 checks are `pass`, and the three Beta
  stations have run.
- **Nothing Critical or High is open.** SR-10 is fixed, but a development
  install made before it may still lack the provider or its PAM service; rerun
  `install-session-units.sh` before taking lock evidence there.
- **SR-13 changed a privileged boundary.** The keyd relay needs a station
  check: Mac shortcuts switch per app without the user in `keyd`, and a
  request other than the three profile names is refused.
- **SR-12 is fixed**, but the stateless publication has only run against
  fixtures and a local APT client; the first real tag must prove it on
  GitHub Actions and Pages.
