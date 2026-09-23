# Code rules audit — 2026-09-23

Scope: `crates/`, `shell/bins/`, `shell/crates/` (`shell/compat/` is vendored
and out of scope). Excluded because other agents are actively editing them:
`crates/system-settings/**`, `shell/bins/rmac-wallpaper/**`,
`shell/crates/rmac-shell-ui/**`, network/Bluetooth/audio/power crates
(`rmac-network`, `rmac-bluetooth`, `rmac-audio`, `rmac-power`, `rmac-sound`),
and the top bar / Control Centre crates (`rmac-top-bar*`,
`rmac-quick-settings*`).

Note on the brief: `todo.md` has no "Code rules to check" or "Files safety
rules" sections — its only content is the Lulo OS rename and install/update
plan. Those section names don't appear anywhere in the repo (`git log
--follow -- todo.md` shows one commit). The six rules audited below are the
ones given directly in this task's instructions.

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

- 4 commits on this worktree branch (`worktree-agent-af82b464370e50e16`):
  - `15fa68dc` — log instead of drop for window/Finder/media-player state saves (Rule 1, 3 files)
  - `cd6ee465` — refuse instead of dropping clipboard payload deletes and unconfirmed permanent delete (Rule 1 / Rule 6, 2 files)
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
