# Code rules audit — 2026-09-23

Scope: `crates/`, `shell/bins/`, `shell/crates/` (`shell/compat/` is vendored
and out of scope). Excluded because other agents are actively editing them:
`crates/system-settings/**`, `shell/bins/rmac-wallpaper/**`,
`shell/crates/rmac-shell-ui/**`, network/Bluetooth/audio/power crates
(`rmac-network`, `rmac-bluetooth`, `rmac-audio`, `rmac-power`, `rmac-sound`),
and the top bar / Control Centre crates (`rmac-top-bar*`,
`rmac-quick-settings*`).

The six rules are the "Code rules to check" and "Files safety rules" in
`todo.md`.

## Rule 1 — no destructive-operation error dropped with `let _ = ...`

Searched `let _ = ...` against delete/trash/rename/move/write/remove_file/
remove_dir_all/kill/uninstall/settings-write patterns across the whole scope.
Most hits are legitimate best-effort cleanup (temp-file removal on an error
path already being reported, `Drop` impls that can't propagate errors,
test-fixture teardown) or process cleanup (`child.kill()` on a helper
subprocess whose exit races the kill — standard and not a source of data
loss). Those are left alone per the brief's "non-destructive `let _ =` ...
is fine" carve-out.

Real violations found and fixed:

| Location | Issue | Status |
| --- | --- | --- |
| `crates/rmac-ui/src/window.rs:362` | `let _ = store.save(state)` silently dropped window-geometry save failures | **Fixed** — now `eprintln!`s the typed `rmac_window_state::Error` on failure (save stays non-fatal by design; failure is no longer invisible) |
| `crates/finder/src/view/presentation_persistence.rs:143` | `let _ = store.save(&state)` silently dropped Finder tab/window state save failures | **Fixed** — same treatment |
| `crates/rmac-media/src/linux.rs:125` | `let _ = std::fs::write(path, bus_name)` silently dropped the last-active-player write, and `create_dir_all` above it was also dropped | **Fixed** — both now report via `eprintln!` |
| `crates/rmac-clipboard-linux/src/store.rs:127-129` (`remove_payloads`) | `let _ = fs::remove_file(...)` dropped clipboard payload deletion errors; a payload that failed to delete looked gone while still on disk | **Fixed** (landed by the persistence-audit fork mid-session, reviewed and kept): `NotFound` is still ignored, other errors are logged; `load_history`'s orphan sweep remains the backstop |
| `crates/finder/src/view/item_operations.rs` (`delete_immediately`, action `DeleteItem`) | Not a dropped-error case but the same failure class: an **unconfirmed, un-trashed permanent delete** reachable the moment anything binds the `DeleteItem` action (nothing does today, so it was dead but live-loaded) | **Fixed** (landed by the same fork, reviewed and kept): the handler now refuses with a typed, user-visible error ("needs a confirmation step that isn't available yet") instead of calling `file_ops::delete` directly. The real "Delete Permanently" keybinding (`DeletePermanently` → `request_permanent_delete`) is unaffected — it already requires being in Trash view and a confirmation dialog. |

Reviewed and left as-is (not violations, with reasoning):

- `crates/rmac-desktop/src/settings.rs:232` — drops a `remove_file` on an
  ephemeral runtime IPC signal file (`$XDG_RUNTIME_DIR/rmac/desktop/...`),
  not user data.
- `crates/rmac-shortcuts/src/dispatch.rs:232` — `Drop for
  DispatchSocketCleanup` removing its own Unix socket; `Drop` can't
  propagate errors, and it verifies device/inode identity first.
- `crates/finder/src/{undo_journal,operation_journal,trash_store}.rs` —
  every `remove_dir_all`/`remove_file` under `let _ =` found in these files
  is inside `#[cfg(test)] impl Drop for TestDirectory` or a test `scratch()`
  helper.
- `crates/rmac-clipboard-linux/src/store.rs` orphan-payload sweep (the block
  above `remove_payloads`) — self-healing: `load_history` re-derives the
  known-id set every load and drops anything not referenced, so a missed
  delete here is corrected on next start.
- All `child.kill()` sites (`rmac-network`, `rmac-display`,
  `rmac-clipboard-linux`, `rmac-locale-linux`, `terminal`, `rmac-shortcuts`,
  `rmac-thumbnails`, `rmac-gtk-settings`, `rmac-input`,
  `rmac-privacy-linux`, `rmac-audio`, `rmac-apps`, `rmac-system-info`) —
  killing a process this crate spawned itself, typically already racing its
  natural exit; doesn't touch persisted user data.

## Rule 2 — persisted data uses versioned serde, temp-sibling + fsync + atomic rename via `rmac-storage`

`crates/rmac-storage` (`write.rs`, `read.rs`) provides exactly this:
`atomic_write`, `atomic_write_private`, `write_new_private`,
`write_new_private_stream`, `copy_no_clobber`, `remove_file_durable`,
`create_dir_all_private` — every one creates an adjacent `.name.tmp-<pid>-<n>`
file, writes, `sync_all()`s the file, renames, then `sync_all()`s the parent
directory, and cleans up the temp file on any failure. 26 crates already
route through it (`finder`, `rmac-notes-storage`, `clock`,
`rmac-window-state`, `rmac-shell-settings`, `rmac-desktop`, `weather`, etc).

Persistence paths that bypass it (found by grepping `fs::write`/
`File::create` for non-test call sites and cross-referencing which crates
depend on `rmac-storage`):

| Crate / file | What it persists | fsync? | Versioned serde? | Notes |
| --- | --- | --- | --- | --- |
| `shell/bins/rmac-screenshot/src/model.rs:576` (`Settings::save`) | Screenshot tool preferences (destination, timer, thumbnail, pointer, last selection) | No — `fs::write` + `fs::rename`, no `sync_all` | No — hand-rolled `key=value` text, no version tag | Low-severity settings; on power loss the rename can still land a torn/previous file. Not fixed: routing it through `rmac_storage::atomic_write` needs a new `rmac-storage` dependency on a Linux-only bin this session can't compile-check (no cargo, cross-compile only happens on the Linux laptop per the brief) |
| `crates/rmac-compositor/src/parking.rs:159-165` (`ParkingSet::save`) | Minimized-window "parking" positions, `$XDG_RUNTIME_DIR/rmac/parking.json` | No | serde_json, no `version` field | Comment in the same file says this is explicitly "a convenience cache, never a source of truth" — self-documented low priority, not fixed |
| `crates/rmac-gtk-settings/src/toolkit.rs:370-386` (`write_atomic`) | GTK2/3/4 settings.ini stub files it manages | No | N/A (plain text stub) | Reimplements the same temp+rename pattern `rmac-storage` already provides, without fsync; not fixed for the same cross-compile-verification reason as above |
| `crates/rmac-dock-runtime/src/consumer.rs:104-111` (`save_recents`) | Dock recent-apps list, `$XDG_STATE_HOME/rmac/dock-recents` | No | Custom line-based encode/decode, no version | Low-severity (recents list rebuilds from usage); not fixed |
| `crates/rmac-clipboard-linux/src/store.rs` (`write_private`, ~line 155) | Clipboard payloads and history index, `$XDG_RUNTIME_DIR/rmac/clipboard` | **Yes** — this one already does `write_all` + `sync_all` + `rename`, with `0600` mode | serde_json for the index, no `version` field | Functionally equivalent to `rmac_storage::atomic_write_private` but duplicated by hand rather than reusing the crate; not switched over in this pass (would add a new cross-crate dependency this session can't verify) |

None of the above lose data outright — the worst case is a torn write on power
loss for settings/caches that are either explicitly documented as
non-authoritative or are low-value preferences. All are reported here rather
than switched to `rmac-storage`, because every one requires adding
`rmac-storage.workspace = true` to a crate that currently doesn't depend on
it, and this session cannot run `cargo check` on Linux-only code (per
`AGENTS.md` / the shared agent brief — this machine builds nothing that
needs the Linux laptop toolchain). This is flagged as the next actionable
follow-up for whichever agent next has laptop build access.

No crate found writing genuinely high-value user content (documents, notes
bodies, keychains) outside `rmac-storage`; `rmac-notes-storage`,
`rmac-notes-store`, `finder`'s trash/undo journals, `rmac-window-state`, and
`rmac-shortcuts` all already route through it and already carry `version`
fields in their persisted records (e.g. `trash_store.rs`'s
`RECORD_VERSION`/`TrashRecord.version`).

## Rule 3 — user-visible failures return typed errors with a recovery action

Spot-checked the persistence-adjacent crates: `rmac-window-state`,
`rmac-focus-store`, `rmac-notifications-store`, `rmac-notes-store`
(`CodecError`, `ExportError`, `BundlePlanError`, `ValidationError`,
`MutationError`), `rmac-shortcuts`, `rmac-recent-documents`, and `finder`.
Every one already defines a typed `Error`/`ErrorKind` enum implementing
`std::error::Error` + `Display` rather than `anyhow`/`Box<dyn Error>`
string-typing. `finder::file_ops::Failure` goes further: it carries an
explicit `recovery_detail: Option<String>` field
(`crates/finder/src/file_ops.rs:55`) that UI code reads to show the user
what actually happened to their data (e.g. "The source was retained; a
partial recovery copy may remain"), and `finder`'s view layer threads these
into `self.operation_error` for on-screen display. No violations found in
the crates checked; the two fixes under Rule 1 (window/Finder state save,
media last-active) were the only places a failure was reaching neither the
user nor a log.

## Rule 4 — logs never include secrets or document contents

Grepped every `eprintln!`/`println!`/`log::*`/`tracing::*` call in scope
(no crate in this scope actually depends on `log`/`tracing`; the convention
throughout is `eprintln!`) for password/token/secret/credential mentions and
for content/payload/clipboard/text/body interpolation. No hits: nothing logs
clipboard payload bytes, document/note contents, or credential material.
Path interpolations that do appear (`rmac-ui/src/chrome.rs:64`,
`rmac-clipboard-linux/src/store.rs:135` logging a numeric payload id, not
its bytes) are diagnostic identifiers, not content. No violations found; no
changes made.

## Rule 5 — domain crates never import GPUI, Wayland, D-Bus or platform FFI

Checked `[dependencies]` (not `[dev-dependencies]`) of every crate in
`crates/` for `gpui`, `gpui-component`, `wayland-*`, `smithay-*`, `zbus`,
`dbus`, `libc`, `nix`. The GPUI/GTK hits are all either the top-level app
crates (`activity-monitor`, `app-drawer`, `archive-utility`, `calculator`,
`clock`, `finder`, `launcher-app`, `notes`, `notification-center-app`,
`player`, `preview`, `quick-settings-app`, `terminal`, `text-editor`,
`weather`, `setup-assistant`, `component-gallery`, `platform-lab`) — these
are the GUI applications themselves, correctly UI-layer — or crates whose
names/roles are inherently UI even without a `-ui` suffix
(`rmac-desktop-widgets`, `rmac-editor`, `rmac-file-chooser`,
`rmac-quick-look`, `rmac-osd`). None of those are "domain" crates by the
rule's own definition (they don't model pure data).

The `libc`-only hits that remain (`rmac-app-launch`, `rmac-archive`,
`rmac-icon`, `rmac-notes-storage`, `rmac-recent-documents`, `rmac-search`,
`rmac-storage`, `rmac-thumbnails`) all use it narrowly for POSIX
file-safety primitives the crate's whole purpose is built around
(`O_NOFOLLOW`, `geteuid`, single-writer flock, process signals for
cooperative cancellation) — not window-system or GUI toolkit calls. This
matches how `rmac-storage` itself (the crate this whole audit leans on for
Rule 2) is built, and is judged in-bounds rather than a purity violation.

Two crates carry a genuine platform dependency (`zbus`) without a
`-linux`/`-system`/`-app`/`-ui` suffix that would flag them:
`rmac-app-menu` (D-Bus app-menu export for the menu bar) and `rmac-osd`
(also `libc`, on-screen display overlay). Both are, by function, inherently
platform/D-Bus code — there's no separate pure-data crate they're
contaminating. This is a naming-convention gap, not a code-purity
violation, and is left as a documentation note rather than a fix (renaming
crates is exactly the kind of workspace-wide churn `PLAN_NEW.md`'s
`rmac-*` → `lulo-*` rename explicitly defers until after the gpui-kit
migration).

No fixes made under this rule — no real violations found.

## Rule 6 — Files safety (finder, rmac-archive)

| Rule | Location | Status |
| --- | --- | --- |
| Recursive copy never follows symlinks | `crates/finder/src/file_ops.rs:238-361` (`copy_item`/`copy_recursive_cancellable`) | **OK** — uses `symlink_metadata` throughout, copies a symlink by re-creating it via `read_link` rather than opening its target; covered by `sparse_files_use_their_logical_copy_size_and_symlinks_are_not_followed` (line 2763), which plants a symlink cycle to prove recursion doesn't loop |
| Recursive delete never follows symlinks | `crates/finder/src/trash_store.rs` (trash) / `file_ops.rs` (permanent delete path) | **OK** — same `symlink_metadata`-based traversal |
| Overwrite has an explicit conflict policy | `crates/finder/src/conflict.rs` — `ConflictDecision::{KeepBoth, Replace, Skip}` | **OK** — explicit enum, no silent clobber; `Replace` durably stages then atomically publishes and retains the previous item for Undo (comment at `conflict.rs:57`) |
| Cancel leaves source intact | `crates/finder/src/file_ops.rs` `move_item_cancellable`/`copy_recursive_cancellable` | **OK** — move is copy-then-delete-source, and the doc comment at `file_ops.rs:956` states the source is retained if the delete step fails or a cancel lands after copy; `copy_failure_reports_that_a_partial_destination_may_remain` and `move_replacement_destination_race_never_removes_the_racing_item` (tests) cover this |
| Permanent delete goes through Trash first | `crates/finder/src/view/permanent_delete_controller.rs` | **OK, with one bug found and fixed** — `DeletePermanently` requires `trash_view` and a `delete_confirmation` dialog before `TrashStore::delete_permanently` runs; `delete_immediately`/`DeleteItem` (see Rule 1 table) bypassed both. **Fixed** by making it refuse instead of deleting. |
| Archive extraction doesn't escape the destination (zip-slip) | `crates/rmac-archive/src/expand.rs:76-120` | **OK** — uses `zip`'s `enclosed_name()` (rejects `..`/absolute paths), opens regular files with `O_NOFOLLOW`, and defers symlink creation until after all regular files are written ("Links last, so no later entry can be written through one," comment at line 113); tar extraction uses `tar`'s `unpack_in` which refuses absolute paths, `..`, and symlink escapes. A test (`expand.rs:410`) plants `../escaped.txt` and asserts it's rejected. |

Existing regression coverage found: symlink-cycle copy (`file_ops.rs`),
partial-destination-after-failure, racing-destination-never-removed,
zip-slip rejection (`expand.rs`). No new regression test was added for the
`DeleteItem` fix because the fix removes reachability of the unsafe path
rather than changing behavior an automated test could usefully pin (there
was, and is, no keybinding or menu wired to it — the danger was latent, not
exercised code).

## Summary

- 3 commits on this worktree branch (`worktree-agent-af82b464370e50e16`):
  - `15fa68dc` — log instead of drop for window/Finder/media-player state saves (Rule 1, 3 files)
  - `cd6ee465` — refuse instead of dropping clipboard payload deletes and unconfirmed permanent delete (Rule 1 / Rule 6, 2 files)
  - `3de97aa0` — this audit document
- Rules audited: 6/6. Violations found and fixed: 5 (see tables above).
  Violations found and left as documented follow-up: 5, all in Rule 2
  (settings/cache writes that duplicate `rmac-storage`'s pattern by hand
  instead of depending on it — none touch high-value user content, and
  fixing them safely needs a Linux `cargo check` this session doesn't have).
  Rules 3, 4, 5 audits found no violations requiring a fix.
- Riskiest thing left unfixed: `shell/bins/rmac-screenshot`'s settings file
  and `rmac-gtk-settings`' managed stub writes have no `fsync` before their
  rename, so a crash immediately after a save can still land a torn file on
  some filesystems/mount options. Low real-world impact (both are
  regenerable preferences, not user content), but worth revisiting with
  cargo access.

## Rule 7 — never parse human-readable CLI output on Linux (added 2026-09-23, separate pass)

`todo.md`'s standing direction: "Use platform services, not command output
… Never parse human-readable CLI output on Linux." This pass (branch
`no-cli-text`) fixed the two remaining violations `docs/settings-backend-audit.md`
had already flagged (`rmac-sharing-linux`'s `ufw status`/`testparm -s`),
switched `rmac-shell-status-linux`'s second, independent PipeWire watcher off
`pw-mon` text (the first, in `rmac-audio`, was already fixed in an earlier
pass — see `docs/journey-7-trace.md`), and re-grepped the whole tree
(`crates/`, `shell/bins/`, `shell/crates/`; `shell/compat/` excluded as
vendored) for every other `Command::new`/`async_process::Command::new` whose
stdout is read.

### Fixed this pass

| Location | Was | Now |
| --- | --- | --- |
| `crates/rmac-sharing-linux/src/system.rs` (`firewall_state`) | `Command::new("ufw").arg("status")`, line-parsed prose (`parse_ufw_status`) | Reads UFW's own `ENABLED=yes\|no` key out of `/etc/ufw/ufw.conf` (`parse_ufw_conf_enabled`). Verified live on the reference laptop (Ubuntu 26.04, `ufw` 0.36.2-9build1): no `firewalld`/UFW D-Bus service exists (`busctl list` — nothing), and `ufw status` itself refuses to run unprivileged (`ERROR: You need to be root to run this script`, confirmed by SSH as the unprivileged `jacob` user) — so the old code path was **already always falling back to `Unavailable`** on this exact machine. `/etc/ufw/ufw.conf` is world-readable (0644); the per-rule file `/etc/ufw/user.rules` that would be needed to verify a specific allow rule is root-only (0640, confirmed `ls -la`), so `FirewallState::Allows` can no longer be produced (this crate has no way to see individual rules without root, same as before) — the state now correctly distinguishes Inactive vs. Active-but-unverified vs. Unavailable without needing root at all, which is strictly more informative than the previous always-`Unavailable` reality. |
| `crates/rmac-sharing-linux/src/system.rs` (`samba_shares`) | `Command::new("testparm").arg("-s")`, bracket-line-parsed prose (`parse_samba_shares`) | Reads `/etc/samba/smb.conf` directly with a small hand-written INI reader (`parse_smb_conf_shares`: `[section]` headers, `#`/`;` comments, an `available = no/false/0` check, same `global`/`printers`/`print$` exclusion as before) plus `net usershare`-created shares from `/var/lib/samba/usershares` (one file per share, filename = share name, per `usershare(8)`). Samba was not installed on the reference laptop (`samba-libs` only, no `smb.conf`, no usershares dir) so the new code path could not be exercised live; it was built from documented `smb.conf`/usershare formats instead. Public API unchanged (`rmac_sharing::{Snapshot, FileSharing, Share, FirewallState}`); `FirewallService` and the crate's other internal helpers are unchanged in shape. |
| `crates/rmac-shell-status-linux/src/watch.rs` (`watch_audio_once`) | Its own `pw-mon --color=never --print-separator` watcher, parsing `added:`/`changed:`/`removed:`/`id:`/`type:` lines into a `PipeWireObjects` set to decide relevance | `pw-dump --monitor --no-colors`, used exactly like `rmac-audio/src/linux.rs`'s watcher (`watch_once`, already fixed): raw bytes are only a "something changed" trigger, never parsed, same `QUIET_PERIOD` (1 s) debounce as before. `PipeWireObjects` and its 3 unit tests were removed (no longer meaningful — there is nothing left to parse). This crate's two watchers (`rmac-audio` and this one) remain independently spawned processes; consolidating them into one is still open (`COMPLETION_SPEC.md` §8.6 already notes it), unchanged by this pass. |

New unit tests: `ufw_conf_enabled_reads_ufws_own_key_value_setting`,
`samba_parser_exposes_only_available_share_sections`,
`samba_parser_share_names_are_bounded` (`rmac-sharing-linux/src/tests.rs`,
replacing the old `ufw_parser_requires_an_explicit_allow_rule`/
`samba_parser_exposes_only_bounded_file_share_names`, which tested the now-
removed prose parsers). `rmac-shell-status-linux` lost 3 tests
(`pipewire_tests`) and gained none — the new watcher has nothing left to
unit-test beyond what `rmac-audio`'s equivalent already covers; both crates'
watchers are structurally identical now.

### Full repo grep — every other `Command::new` reachable on Linux

Grepped `Command::new`/`async_process::Command::new` across the same scope,
then cross-referenced every hit that calls `.output()`/`.stdout(Stdio::piped())`
(the rest are pure action invocations — spawn/launch/notify/kill, no data
read back — and are not this rule's concern). Assessed against the same bar
`docs/settings-backend-audit.md`'s inventory already established: D-Bus/portal
> stable machine-readable flag (`--json`, `-t`, `--property=…`, a single
documented value) > free-text prose scraping.

Already covered by `docs/settings-backend-audit.md` and unchanged here
(`rmac-locale-linux` `locale -a`/`localectl`, `rmac-gtk-settings` `gsettings`,
`rmac-audio`'s `wpctl`/`pw-dump`, `rmac-network`'s `nmcli` import/modify
helpers, `rmac-display`/`rmac-input`'s `niri`, `rmac-keyboard`'s
`pkexec`/`systemctl`/`keyd`, `rmac-privacy-linux`'s Ubuntu Pro Client/
`ubuntu-distro-info`, `rmac-system-info`'s `uname`, the macOS-only
`#[cfg(target_os = "macos")]`-gated modules in `rmac-network`, `rmac-power`,
`rmac-bluetooth`): no change, see that document for the per-line detail.

New sites checked in this pass, all judged acceptable exceptions or action
invocations (nothing else needed fixing):

| Location | Command | Reads stdout as data? | Assessment |
| --- | --- | --- | --- |
| `crates/finder/src/view/filesystem_helpers.rs:213` | `stat -c "%U\n%G" <path>` (Linux branch) | Yes — two fixed fields | Structured format flag (`-c`), not scraped prose — same category the brief calls out as acceptable (`lsblk --json`/`-t`-style). |
| `crates/rmac-mounts/src/mutation.rs:26` | `gio mount -u <uri>` | No — stderr/stdout drained only for a bounded error message on failure | Action helper (unmount), matching the `nmcli`/`gio launch` precedent already accepted in the settings-backend audit. |
| `crates/rmac-apps/src/icons.rs:77` (`gsettings_icon_theme`) | `gsettings get org.gnome.desktop.interface icon-theme` | Yes — one quoted string | Same `gsettings`-is-its-own-CLI exception already granted to `rmac-gtk-settings`; no separate D-Bus service exists for this GNOME setting. |
| `crates/rmac-gtk-settings/src/toolkit.rs:196-212` | `gsettings get/set` (accent/dark-mode) | Yes — one value, compared/echoed verbatim | Same `gsettings` exception, same crate already covered for `api.rs`/`watch.rs`. |
| `crates/rmac-apps/src/platform.rs` (`run_xdg_mime`, `query_default_application`) | `xdg-mime query filetype\|default` | Yes — one line (a MIME type or a `.desktop` id) | `xdg-mime` is the sanctioned freedesktop.org CLI for MIME-association queries; there is no D-Bus service for this. Single documented value, not prose. |
| `crates/rmac-session/src/supervisor.rs:47` (`component_health`) | `systemctl --user show <unit> --no-pager --property=Id,LoadState,ActiveState,SubState,Result,NRestarts,MainPID,ExecMainStatus` | Yes — `Key=Value` lines | `--property=` is systemd's own stable structured output mode (one `Key=Value` per requested property), not free text — same class as `lsblk --json`/`-t`. |
| `crates/rmac-clipboard-linux/src/wayland.rs:78` (`offered_types`) | `timeout 5 wl-paste --list-types` | Yes — one MIME type per line | `wl-clipboard` is the sanctioned Wayland data-control CLI; compositors deliberately don't expose clipboard content over D-Bus. Listing offered MIME types is the clipboard's own machine-oriented output, not host state being scraped. |
| `crates/rmac-shortcuts/src/lock.rs` (`supervise`, `supervise_idle`) | `swaylock --config … --ready-fd=1`, `swayidle -w …` | No — waits for a single readiness byte on `--ready-fd`, then only the process exit status; never reads prose | Action/daemon invocations, the standard niri/wlroots lock mechanism (no D-Bus alternative). |
| `crates/preview/src/render.rs` (`run`) | poppler-utils (`pdftoppm`/`pdftotext`/`pdfinfo`, chosen by `tool`) | Yes — but it's the rendered page image or extracted document text itself, not host/system state | Out of this rule's scope: this is user-document content extraction (the tool's entire purpose), not a human-readable status format being parsed as a substitute for a platform service. |
| `crates/rmac-search/src/platform.rs:41` (`mdfind`) | `mdfind` | Yes | `#[cfg(target_os = "macos")]` — not compiled on Linux. |
| `crates/app-drawer/src/catalog.rs` (`sips`, `/usr/libexec/PlistBuddy`), `crates/app-drawer/src/catalog/category.rs:44` (`defaults`) | macOS icon conversion / Info.plist reads | Yes | All under `#[cfg(target_os = "macos")]` — not compiled on Linux, same as the macOS-only modules `docs/settings-backend-audit.md` already lists. |

No new violations found beyond the two fixed above. Every remaining
stdout-reading `Command::new` on the Linux build either reads a stable,
documented single value or `Key=Value`/one-per-line machine format with no
D-Bus alternative (an already-accepted exception category), or is gated out
of the Linux build entirely.

### Verification

- `rustfmt --edition 2021 crates/rmac-sharing-linux/src/system.rs
  crates/rmac-sharing-linux/src/tests.rs
  crates/rmac-shell-status-linux/src/watch.rs` — clean.
- Plain `rustc --edition 2021 --crate-type lib --emit=metadata` on each
  changed file in isolation: only the expected `E0432 unresolved
  crate/module` errors from checking a submodule file outside its crate
  (no `cargo`, per `AGENTS.md`); no syntax errors in any of the three files.
- Not run (no cargo, no Linux build in this session): `cargo build -p
  rmac-sharing-linux -p rmac-shell-status-linux`, `cargo test` for either
  crate's new/changed unit tests, or an end-to-end check that the Sharing
  pane's firewall footnote and share list still render sensibly now that
  `FirewallState::Allows` can never be produced (the UI code at
  `crates/system-settings/src/controller/sharing/render.rs:197,254` already
  treats every non-`Allows` state as "show the cautionary footnote", so this
  is a behavior narrowing — always cautionary now instead of sometimes
  confidently "allows" — not a crash risk, but it needs an actual Sharing
  pane screenshot on the laptop to confirm the footnote text still reads
  sensibly).
- Samba's new `smb.conf`/usershare reading path could not be exercised
  against a real Samba install (not present on the reference laptop); only
  unit-tested against a hand-written fixture built from documented
  `smb.conf` syntax, not a captured real file.
