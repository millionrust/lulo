# Security review — 0.9.0-beta.1 (source review)

This is the I6 security and privacy review
([security-release-review.md](security-release-review.md)) for the
0.9.0-beta.1 early-access release, run against the 80 checks in
`scripts/security-review.json`. The canonical summary sits beside this file
as [security-review-0.9.0-beta.1.json](security-review-0.9.0-beta.1.json).

**Gate status: Fail.** The source review is done: every one of the 10 domains
was read, 9 findings were fixed with regression tests, and nothing Critical
is open. Three things still block the gate:

- 18 findings are still open. None is Critical, one is High, and that High
  affects only development installs, not the `.deb`.
- 34 checks need native execution on real stations.
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

- **Reviewed tree:** `dev` at 5bea40ca, plus the fix commits listed below. The
  JSON's `revision` is the last fix commit.
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

Of the 80 checks, 46 are `pass` and 34 are `pending`.

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
regex rejected; it now accepts `~` (`install.sh:191`, with tests). **Check on
the first real release:** GitHub may rename `~` in uploaded asset names. If it
does, the names in `SHA256SUMS` will not match the download URLs.

## Open findings

| ID | Severity | Boundary | Evidence | Exploit scenario | Recommended fix |
|---|---|---|---|---|---|
| SR-10 | High (development installs only; the `.deb` is not affected) | lock-boundary | `scripts/linux/install-session-units.sh:46-59` never builds `rmac-lock-provider`, and `:92` deletes both it and `rmac-lock-fallback.service`. Nothing installs `/etc/pam.d/rmac-lock`. | On a machine set up this way, including the reference laptop's development install, the installed `rmac-lock.service` cannot lock. Before SR-02 it failed silently. Now it fails visibly, but logind still suspends after `InhibitDelayMaxSec`, so the session resumes unlocked. | Build `-p rmac-lock-provider-linux --features rmac-lock-provider-linux/provider --bin rmac-lock-provider`. Stop deleting the provider and the fallback unit. Refuse to finish (or print a clear instruction) until `/etc/pam.d/rmac-lock` is installed. Evidence of this state on the laptop invalidates any lock result taken there. |
| SR-11 | Medium | notifications | `crates/rmac-notifications/src/reducer.rs:68`: `active.insert` has no cap. `expire_one` keeps history-backed entries in `active`, and history eviction (500) never prunes `active`. `post_event` scans `active` linearly. | Any Flatpak app can call the Notification portal's `AddNotification` with ever-new ids, each up to about 16 KiB of body plus 8×16 KiB of targets. This exhausts memory and CPU in rmac-notifications and takes down banners and the Center. | Cap `active` overall (for example 1024) and per app (for example 100). Evict the oldest non-visible entry of that app, or reject. Prune `active` when history evicts an entry. Needs reducer tests. |
| SR-12 | Medium (latent: APT publishing is gated off) | updates | `.github/workflows/release.yml:518` and `rollout.yml:67` both run `wget --mirror https://millionrust.github.io/lulo/ \|\| true`. Pages has no directory listings, so the mirror is always empty. `publish-apt-snapshot.py` then sees no current repository. | The "Date/snapshot must increase" check (`publish-apt-snapshot.py:1186-1190`) never runs, and retained snapshots, by-hash indices and old pool objects are dropped on each deploy. Re-running an old tag republishes older packages with a fresh date, and rollback evidence disappears. | Keep the publisher's state (`.rmac-publisher/state.json` plus the pool) somewhere authoritative, such as a Pages branch or a release asset. Fail closed when it can't be fetched, unless an explicit first-publish variable is set. |
| SR-13 | Medium (needs verification) | dbus-polkit | `crates/rmac-keyboard/src/system.rs:113-114` adds the requesting user to group `keyd` (ADR 0017). The group grants access to keyd's IPC socket. | Every process of that user, not only the rmac follower, can use keyd's IPC. That covers at least `bind`, and depending on the keyd version also text input and `command()` actions. At minimum this is global keystroke injection that goes around Wayland's client isolation and reaches whatever surface or VT is active. If keyd 2.5.0 accepts `command()` over IPC from non-root users, it is a root escalation. The membership is also never removed when the feature is turned off. | Verify keyd 2.5.0's IPC restrictions on the station. If `command()` or `input` are open to the group, run the follower as a dedicated system user, or through a root helper that accepts only the enumerated profile names. Remove the group membership on opt-out. |
| SR-14 | Low | updates | `packaging/apt/rmac.pref:1-3` only sets rmac's three packages to 500. `docs/update-trust.md` claims it stops the repository replacing other Ubuntu packages. | Whoever holds a valid signing key could ship a higher-versioned `sudo` or `openssh-server` from the rmac origin. The same key can already ship a malicious `rmac-session`, so the added risk is small, but the documented control does not exist. | Add `Package: *` / `Pin: origin "millionrust.github.io"` / `Pin-Priority: -1`. Update `verify-update-trust.py` `EXPECTED_PREFERENCES` and the `install.sh` heredoc to match. |
| SR-15 | Low | packages | There is no `--remap-path-prefix`. Panic locations keep `$CARGO_HOME/…` and `../crates/*` absolute paths (`Cargo.toml:162-167` strips symbols only). | Local and reference-PC builds embed `/home/<user>/…`. `check-native-reproducibility.sh` builds twice on one host, so it cannot catch this. | Remap `$CARGO_HOME` and the repository root in `build-native-inputs.sh`, and scan packaged binaries for `/home/` and `/Users/`. |
| SR-16 | Low | packages | `.github/workflows/ci.yml` pins `actions/checkout@v4`, `actions/cache@v4` and `cargo-deny-action@v2.0.20` by tag. The same "v2.1.1" label maps to two different SHAs in `ci-quality.yml:104` and `release.yml:45`. | A moved tag would run code in CI (`contents: read`, no secrets) and could poison caches. | Pin by SHA, after checking each SHA against its tag online. |
| SR-17 | Low | packages | `install.sh:239-243`: without `gh`, `--from-release` checks only `SHA256SUMS`, which is fetched from the same release. | Authenticity then rests on HTTPS and GitHub account security, although the comment says otherwise. | Require `gh attestation verify` (optionally with `--signer-workflow`) unless `--allow-unattested` is passed, and reword the comment. |
| SR-18 | Low | packages | rustup is installed by `curl \| sh`, the `ubuntu:26.04` container is pinned by tag, and `cargo install` is pinned by version only (`release.yml:125,231,281`). The publisher checks InRelease against the keyring exported from the signing secret, not the packaged keyring (`release.yml:539-551`). The manifest mode check accepts any `[0-7]{4}` (`verify-*-package.py`). | Supply-chain drift. A wrong subkey causes an outage, which fails closed. A setuid mode in the manifest would pass verification. | Pin by digest or hash. Pass `--keyring` from `keyring/*.deb`. Allow only `0644` and `0755` in manifests. |
| SR-19 | Low | file-operations | `crates/preview/src/render.rs:193-200` runs the poppler tools with `.output()`, with no timeout and no output cap. | A crafted PDF can hang Preview or consume a lot of memory (denial of service only). | Reuse the thumbnail runner (`rmac-thumbnails/src/media.rs:176-260`). |
| SR-20 | Low | desktop-entry-execution | `crates/rmac-apps/src/platform.rs:229-233,246-249` accept a relative `Path=`, which is resolved against the launcher's own working directory. | Robustness and contract only. There is no shell, and argv boundaries hold. | Keep `Path=` only if it is absolute (`Path::is_absolute`). |
| SR-21 | Low | file-operations | `crates/finder/src/file_ops.rs:307-322` checks with `symlink_metadata`, then calls `File::open`, which follows symlinks. | In a shared writable source folder such as `/tmp`, a symlink swapped in between the two calls copies a victim-readable file into a destination the attacker can read. | Open with `O_NOFOLLOW` and take the type and mode from the opened file. |
| SR-22 | Low | dbus-polkit | `crates/rmac-network/src/secret_agent.rs:69-150` and `crates/rmac-bluetooth/src/pairing_agent.rs:437-530` never compare the caller with the owner of `org.freedesktop.NetworkManager` or `org.bluez`. | Only reachable under a permissive system-bus policy; stock policy lets only root send these. Another user could then claim a Wi-Fi secret that was just typed, or show fake pairing prompts. | Compare `header.sender()` with `GetNameOwner`, as the portal backends already do. |
| SR-23 | Low | dbus-polkit | `crates/rmac-session/src/main.rs:333-376`: the safe-mode notice's `ActionInvoked` match rule has no sender. | Any session peer can answer the safe-mode prompt for the user. | Accept only the unique name that answered `Notify`. |
| SR-24 | Low | dbus-polkit | `crates/player/src/mpris.rs:163-165` and `view.rs:181-190`: `OpenUri` has no length cap, no playlist cap and no path validation. | A Flatpak app with MPRIS talk access can learn whether any host file exists and read its tags, and can grow the playlist without limit. | Cap URI length and playlist size, and require an absolute, existing `file:` path with no host. |
| SR-25 | Low | dbus-polkit | Only `crates/rmac-app-menu/src/lib.rs:619` sets `method_timeout`. The notifications `ActivateAction` and Center `Invoke` calls go to app-controlled names with no timeout. | A peer that never replies keeps calls pending. | Set a 5-second `method_timeout` on each service connection builder. |
| SR-26 | Low | dbus-polkit | `crates/rmac-keyboard/src/system.rs:244-247` maps pkexec exit 126 (dismissed) and 127 (not authorised) both to "authentication was cancelled". | A denial is reported to the user as a cancellation (contract `denial-and-cancel-distinct`). | Report 127 as "not authorised". |
| SR-27 | Low (documentation) | lock-boundary | `docs/secure-lock-recovery.md:9-11,40-42` and `docs/secure-lock.md:13-27` still describe `rmac-lock.service` as swaylock. | The TTY recovery runbook does not match the installed unit (`tty-recovery-proven`). | Rewrite the runbook for `rmac-lock-provider` plus the fallback unit, then prove it on a station. |

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
| working-directory-validated | pending (SR-20) | |

### dbus-polkit
| Check | Verdict | Evidence |
|---|---|---|
| broadcasts-contain-no-secrets | pass | the Center, Clipboard and LockScreen `Changed` signals carry counts or policy only; Clipboard history skips password-manager offers (`crates/rmac-clipboard-linux/src/service.rs:254-259`) |
| bounded-call-time-and-output | pending (SR-25) | nmcli and helper output is bounded (`crates/rmac-network/src/vpn_import.rs:343-356`, `crates/rmac-privacy-linux/src/security.rs:33-80`) |
| denial-and-cancel-distinct | pending (SR-26) | |
| interactive-authorization-only-from-user-action | pending (native) | `interactive=true` is set on SetStaticHostname, SetTimezone, SetLocale and PackageKit install (`crates/rmac-system-info/src/host.rs:182`, `crates/rmac-time-linux/src/system.rs:79`, `crates/rmac-locale-linux/src/system.rs:281`, `rmac-updates-linux` `transaction.rs:217`); not every System Settings call site was traced back to a user action |
| mutation-requires-authoritative-readback | pending (native) | hostname verified after the write (`rmac-system-info` `verify_static_hostname`), VPN edits re-read (`vpn_editor.rs:300-317`), updates re-snapshot; Sharing and systemd unit changes need station proof |
| no-credential-collection | pass | admin credentials only through polkit agents; the keyboard helper takes enumerated flags only (`crates/rmac-keyboard/src/helper.rs:24-48`); Wi-Fi secrets are zeroized with redacted Debug (`crates/rmac-network/src/model.rs:191-260`) |
| system-bus-callers-treated-untrusted | pending (SR-22) | |
| unique-owner-revalidated | pending (SR-23) | the portal backends compare the caller with the owner of `org.freedesktop.portal.Desktop` (`crates/rmac-file-chooser/src/dbus.rs:245-266`) |

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
| symlink-and-root-boundaries-enforced | pending (SR-21) | SR-03 fixed; copy recreates links rather than following them; `.trashinfo` decoding rejects NUL, absolute paths and `..` (`trash_store.rs:903-968,1072-1085`) |
| trash-and-destructive-actions-confirmed | pass | `crates/finder/src/view/permanent_delete_controller.rs:51-88` |
| untrusted-content-never-executed | pending (native) | Files never executes anything itself; opening goes through the OpenURI portal or `xdg-open` (`crates/rmac-portal/src/open.rs:41-52`); handler behaviour for `.desktop` files and executables needs station proof |

### lock-boundary
| Check | Verdict | Evidence |
|---|---|---|
| compositor-exclusive-lock-proven | pending (native) | readiness only after `locked` (`crates/rmac-lock-provider-linux/src/wayland.rs:1661-1665`); `finished` handled (`:1667-1680`); `unlock_and_destroy` only from `Locked` (`:1151-1159`); niri holding the lock after the client dies is not provable from source |
| mfa-conversation-bounded | pass | 1..=32 messages of at most 512 B each; responses at most 512 B with no NUL (`src/pam/callback.rs:11-12,68,84,166-169,229-246`); one prompt at a time (`pam_broker.rs:27,338-349`) |
| no-password-or-keycode-logging | pass | only fixed-string `eprintln!`; every secret-bearing type has a redacted Debug (`secret.rs:88,113`, `keyboard.rs:29,69`, `pam_conversation.rs:39,78`, `pam_broker.rs:197`) |
| pam-is-sole-unlock-authority | pass | `UnlockAuthorization` is minted only on success (`crates/rmac-lock-provider/src/model.rs:82-85`, `provider.rs:127-135`); requires `pam_authenticate(PAM_DISALLOW_NULL_AUTHOK)`, then `acct_mgmt`, then `pam_end` (`src/pam/transaction.rs:129-173`); no D-Bus, env or timer unlock; `pam/rmac-lock` includes `common-auth` and `common-account` only |
| provider-crash-fails-closed | pending (SR-10, native) | SR-02 and SR-09 fixed; only an authenticated unlock exits 0 (`development_process.rs:67-87`); watchdog SIGKILL and restart |
| secret-lifetime-and-zeroization-reviewed | pass | fixed-capacity `SecretInput`, zeroized on edit and drop (`secret.rs:13-80`); a single calloc copy for libpam, wiped with `explicit_bzero` (`pam/callback.rs:67-79,113-135`); no `CString` of the secret; cores disabled (SR-09) |
| suspend-waits-for-lock-readiness | pending (native) | sleep delay inhibitor released only after a successful start (`crates/rmac-shortcuts/src/lock.rs:326-334,357-365`); SR-02 fixed |
| tty-recovery-proven | pending (SR-27, native) | |
| wrong-password-and-cancel-remain-locked | pass | failure and cancel return to `Locked` (`provider.rs:136-148`); the cancelled worker is drained (`runtime.rs:293-317`) |

### notifications
| Check | Verdict | Evidence |
|---|---|---|
| action-target-bound-to-notification | pass | owner checked on replace and withdraw (`crates/rmac-notifications/src/reducer.rs:36,85`); document-open requires a portal source and a matching app (`crates/rmac-notifications-linux/src/service.rs:1362-1400`) |
| diagnostics-redacted | pending (native) | not traced end to end in this pass |
| focus-suppression-authoritative | pending (native) | not traced in this pass |
| history-and-payload-bounded | pending (SR-11) | payload limits (`crates/rmac-notifications/src/lib.rs:20-29`); history 500 records, 100 per app, 8 MiB |
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
| signature-and-origin-claims-bounded | pending (SR-14, SR-17) | |
| unpackaged-executables-rejected | pass | `verify-native-packages.py:430-436`; no setuid in source |

### updates
| Check | Verdict | Evidence |
|---|---|---|
| apt-key-scope-isolated | pass | `Signed-By` keyring (`packaging/apt/rmac.sources.in:6`); `verify-update-trust.py:201-206` rejects `trusted=yes` and `trusted.gpg.d`; SR-01 fixed |
| atomic-inrelease-last-and-monotonic | pending (SR-12) | InRelease written last with `os.replace` and fsync (`publish-apt-snapshot.py:1231-1240`) |
| backend-failure-recovers-authoritatively | pass | re-read after install (`crates/system-settings/src/controller/software_updates.rs:194-195`) |
| cancellation-and-restart-readback | pass | `crates/rmac-updates-linux/src/transaction.rs:255-301` |
| immutable-pool-and-by-hash | pending (SR-12) | |
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
- **Nothing Critical is open, and nothing open affects the `.deb`-installed
  lock screen.** The one open High (SR-10) breaks locking on development
  installs, which the reference laptop uses. Fix it before any lock evidence
  is taken there.
- **Worth fixing before a public early-access build:**
  - SR-11: any Flatpak app can exhaust memory in the notification daemon.
  - SR-13: verify the keyd group's IPC reach on Ubuntu 26.04's keyd 2.5.0
    before shipping "Use Mac shortcuts in all apps" as an option.
- **SR-12 and SR-14** must be fixed before the signed APT repository is
  switched on. They do not block the GitHub-Release Beta.
