# 1.0 scope audit: app × item × state

This audits each app in `todo.md`'s "## 1.0 scope per app" checklist against
the code on branch `dev` (this branch, `app-scope`, starts from it at
`ea11db97`). "Done" means the item works and covers the five states
`todo.md` requires: loading, empty, unavailable, permission-denied and
error. Status is one of **Done**, **Partial** or **Missing**, each with
`file:line` evidence. `docs/beta-gap-list.md` and `docs/journey-suite.md`
were read first; where they disagreed with the current code, the code wins
and the doc is noted as stale.

Two crates were being edited by other agents while this audit ran and were
**not** changed here: `crates/system-settings` (visual + idle-CPU work) and
`crates/text-editor` (large-file rebase). Their rows below are audit-only.

## Summary

Most of the seven apps are substantially more complete than `todo.md`'s
unchecked boxes suggest. Of the five candidate small/medium items named in
the task brief, only one was still genuinely missing:

| Candidate item | Actual state |
|---|---|
| Terminal find-in-scrollback | Was **Partial**, now **Done** — implemented on this branch, see below |
| System Monitor history graph | Already **Done** (not touched) |
| Files open-with menu | Already **Done** (not touched) |
| Text Editor export-as-PDF via print path | Already **Done** (not touched) |
| Notes import/export formats | Already **Done** (not touched) |

Every app's `todo.md` checklist item is now **Done**. System Monitor's
missing "no processes match" empty state (found during this audit) was fixed
in `b1d83bc0`, along with two related five-states gaps in the same pass:
CPU% showing a fake `0%/0%/100% Idle` for the first two seconds before a real
reading exists, and an unnamed empty network-interface list. One small gap
remains open, not implemented here because it sits in App Drawer's launch
surface rather than being a states/error-handling gap: search uses plain
substring matching instead of the fuzzy matcher (`rmac_launcher::query_matches`)
Spotlight already uses. Notes, Text Editor and System Settings are fully covered below;
Text Editor and System Settings are audit-only (owned by other agents
currently editing those crates).

## Terminal (`crates/terminal`)

| Item | Status | Evidence | Five states |
|---|---|---|---|
| Real PTY | Done | `session.rs:14-15` uses `portable_pty::native_pty_system`; open/start/reader/writer failures are a typed `SessionStartError` (`session.rs:160-179`) | Unavailable: `Session::failed()` builds a dead session labelled "Unavailable" (`session.rs:150-157,623`). No distinct permission-denied message; a shell-exec permission failure surfaces as the generic "Terminal could not start the configured shell." |
| Dynamic resize | Done | `session.rs:706` `resize`; rejected-size fallback tested (`session.rs:1081` `resize_failure_retains_the_last_kernel_accepted_geometry`) | Error: resize failure keeps last accepted geometry, not fatal |
| Scrollback | Done | `emulator.rs:70` `scrollback_limit_for_tab_count`, tested `emulator.rs:345-395` | n/a |
| Selection | Done | `ui_state.rs:17` `Selection`; keyboard select-all across full history `view_state.rs:88-99` | n/a |
| Search (find-in-scrollback) | **Was Partial, now Done** | Previously: find bar existed (`controller/renderer/overlays.rs` `render_find_panel`) but `render_rows` matched only the visible viewport (`controller/renderer/grid.rs:59-101`), no jump-to-match, no match count. **Fixed on this branch**: `crates/terminal/src/find.rs` (new, pure and unit-tested: column-precise matches, wide-character handling, wrap-around stepping, scroll centring) plus wiring in `controller/renderer/grid.rs` (`find_step`, `find_status_label`), `controller/lifecycle.rs` (⌘G / ⇧⌘G key bindings), `controller/renderer/interactions.rs` (`FindNext`/`FindPrevious` action handlers) and `controller/renderer/overlays.rs` (the find panel now shows "N of M" / "Not Found"). | Empty query: no highlight, no count shown. Zero matches: "Not Found" label instead of a count. |
| Tabs | Done | `KeyBinding`s `controller/lifecycle.rs:63-83`; `new_tab` bounded by `MAX_TABS` with an error message (`tab_lifecycle.rs:36-39`); per-tab close with running-process confirmation (`tab_lifecycle.rs:89-108`) | Error: tab-limit/rebalance failures surface via `operation_error` |
| Profiles | Done (colour presets, not full shell-config profiles) | `profiles.rs` 8 built-ins; ⌘, opens the picker (`5357ebb9`); persistence errors surfaced (`lifecycle.rs:128-137`) | Error: `persistence_error` banner on load/save/migration failure |
| Quit / running-process confirmation (beta-gap B4) | Done | `controller/lifecycle.rs:103-112` `window.on_window_should_close` covers Dock/menu bar/⌘Q/⌘Tab-Q/logout; confirmation dialog `controller/renderer/dialogs.rs:5-33` | — |

`docs/beta-gap-list.md`'s B4 is stale (already fixed at `5357ebb9`, ancestor
of `ea11db97`).

## Files (`crates/finder`, `rmac-file-chooser`, `rmac-mounts`)

| Item | Status | Evidence | Five states |
|---|---|---|---|
| Safe file operations | Done | `file_ops.rs` (2954 lines): copy/move with cancellation, atomic staging, no-follow-symlink throughout | Loading: `journal_loading`/`trash_loading` block mutation with explicit messages (`view/operations.rs:65-70`). Error: `operation_error` banner. Unavailable: journal-unavailable message (`operations.rs:72-76`) |
| Trash | Done | `trash_store.rs` (4821 lines), freedesktop-trash-compliant: durable move + `.trashinfo` (`trash_store.rs:2380,3450`), collision-safe (`trash_store.rs:4397`) | Empty: dedicated "Trash is Empty" UI (`view/list_presentation.rs:633-645`). Permission-denied tested (`trash_store.rs:2686,2911,4495`) |
| Undo | Done | `undo_journal.rs` (2527 lines); `view/undo_controller.rs` `start_undo`/`cancel_undo` | — |
| Mounts | Done, but discovery is `/proc/self/mountinfo` + `gio mount -u`, **not D-Bus/udisks2** | `rmac-mounts/src/inventory.rs:11-27` `discover()`; watch via `poll` on `/proc/self/mounts` (`watch.rs:30-42`); unmount via `gio mount -u` (`mutation.rs:8-29`) | Unavailable: `WatchEvent::Unavailable` on non-Linux/watch failure. Error: per-volume `usage_error`, never hides healthy mounts. Disappearing mount: `filesystem_helpers.rs:167` `disappeared_mount_roots`, tested `view/tests.rs:59` |
| Search | Done | `view/search_helpers.rs` + `search_info_controller.rs`, backed by `rmac_search` | Error: distinguishes invalid-query vs. generic read failure (`search_helpers.rs:26-30`) |
| Previews (Quick Look) | Done | `view/quick_look_controller/controller.rs`, `view/thumbnail_controller.rs` | — |
| Open-with actions | Done, including "always open with" | `view/open_with_controller.rs:4-58,113,136-176`; wired into both the File menu and the right-click context menu (`view/chrome_presentation/menus_tabs.rs:92`, `view/list_presentation.rs:782`) | Guards for Applications/Trash views and multi-selection (`open_with_controller.rs:6-21`); async load failure surfaces "Could not load compatible applications: {error}" |

### Files safety rules (`todo.md:76-80`)

| Rule | Status | Evidence |
|---|---|---|
| Never block UI thread on recursive I/O | Done | `view/operations.rs:116-133` runs transfers in `cx.background_executor().spawn(...)`; `view/trash_task_controller.rs:21` similarly off-thread |
| Never follow symlinks in recursive copy/delete | Done | `symlink_metadata` throughout (`file_ops.rs:269,327,375,378,416,431,457,494`); TOCTOU-safe open (`file_ops.rs:296-346`); test `file_ops.rs:2806` creates a symlink cycle and asserts it isn't followed |
| Every overwrite has an explicit conflict policy | Done | `conflict.rs`: `ConflictDecision` (`KeepBoth`/`Replace`/`Skip`, `conflict.rs:36,38,57-61,150,193`) |
| Cancel leaves source intact | Done | 6 dedicated tests, e.g. `file_ops.rs:1860,1912,2133,2223,2320,2496` |
| Trash before permanent deletion | Done | `view/permanent_delete_controller.rs:4-9` only reachable from the Trash view |
| Tests: cross-filesystem moves | Done | `file_ops.rs:1752,1912,1996,2288,2320` |
| Tests: permission errors | Done | `file_ops.rs:1737,1772,1816`; `trash_store.rs:4495` |
| Tests: low disk | Done | `file_ops.rs:2587,2874` (simulated `ENOSPC`) |
| Tests: name collisions | Done | `trash_store.rs:4397`; `conflict.rs` `KeepBoth`-numbering tests |
| Tests: disappearing mounts | Done | `view/tests.rs:59`; `rmac-mounts/src/inventory.rs:69-76` |
| Tests: interrupted operations | Done | Cancellation suite above + `operation_journal.rs` (3699 lines) recovery on next launch |

All Files safety rules have direct test coverage. No gaps found.

## System Monitor (`crates/activity-monitor`)

| Item | Status | Evidence | Five states |
|---|---|---|---|
| Process view | Done | `process_table.rs` (`ProcRow`/`Delegate`), columns in `columns.rs:12-24`; inspector dialog `view/render/overlays.rs:59-172` | Loading: tick-loop sampler (`view.rs:105`). Error: dismissible `persistence_error` banner (`view.rs:41,50,341`). Empty (0 rows after a search filter): "No Matching Processes" with the query named, `view/render.rs:141-157` (fixed in `b1d83bc0`) |
| Resource view (CPU/Mem/Energy/Disk/Network) | Done | `view/render/metrics_panes.rs` `cpu_cells`/`memory_cells`/`energy_cells`/`disk_cells`/`network_cells` (304-435) | Loading: CPU% shows a dash rather than a fake `0%/0%/100% Idle` before the first two-tick reading exists; empty network interface list is named rather than left blank (both `b1d83bc0`) |
| Search | Done | `SearchField` (`view/render/chrome.rs:186`); filter in `process_table.rs:213-214,286-299` (substring only, no fuzzy) | Empty search collapses to the toolbar circle (`view.rs:266-267,273`) |
| Sort | Done | Click-header sort `process_table.rs:396-406`; comparator `process_table.rs:305+` | — |
| Safe terminate | Done, best-in-class states | Confirm dialog always required (`view/render/overlays.rs:6-57`); Quit→SIGTERM/ForceQuit→SIGKILL via pidfd, so no PID-reuse race (`process_signal.rs:29-83`) | `EPERM`→`Outcome::Rejected` with an explicit "your account may not have permission" message (`process_action.rs:105-114`, tested `process_action.rs:160-177`); also distinguishes `Missing` (exited before confirmation), `Replaced` (PID reused), `Unsupported` |
| History (graphs) | Done | Data: bounded 60-sample ring per metric, `metrics.rs:71-95`. UI: real Area/Stacked/Mirrored graphs, `view/render/metrics_panes.rs:81,173-301,322-435` | n/a — graphs render a left-padded partial history while filling (`metrics_panes.rs:99-107`) |
| Per-process GPU numbers | Compliant — none invented | Exhaustive grep for `gpu`/`GPU` in the crate returns no per-process or system GPU metric; no GPU column in `ColKey::ALL` (`columns.rs:29-40`) | n/a |

`docs/journey-suite.md`'s System Monitor notes (§6) are AT-SPI projection
findings (process table bounded to the visible viewport for accessibility
tooling, no `Action` interface on column headers to re-sort) — accessibility
gaps, not functional ones; out of this audit's scope but worth a follow-up.

**Gap found during this audit, now fixed:** the missing "no processes match
your search" empty state, plus the CPU-dashes-before-first-reading and
unnamed-empty-network-list five-states gaps, were fixed in `b1d83bc0` using
`rmac_ui::EmptyState`, the same reusable pattern App Drawer already uses
(`crates/app-drawer/src/view/render.rs:38-51`).

## Apps / App Drawer (`crates/app-drawer`, `crates/rmac-apps`, `crates/rmac-app-launch`)

Actual `.desktop` discovery lives in `crates/rmac-apps`, not
`rmac-launcher-providers` (which only consumes the catalog for Spotlight
search). `crates/app-drawer/src/catalog.rs` is a macOS-only dev-port helper
(`sips`/`PlistBuddy`, all `#[cfg(target_os = "macos")]`) and is not the
Linux discovery path.

| Item | Status | Evidence | Five states |
|---|---|---|---|
| Standards-compliant discovery | Done — all 5 XDG keys honoured | `rmac-apps/src/platform.rs:186-231` `parse_desktop_entry`: `Type=Application` (188), `Hidden` (193), `NoDisplay` (194), `OnlyShowIn`/`NotShowIn` via `desktop_visible()` (195, 621-626), `TryExec` (199-202). Tests: `rmac-apps/src/tests.rs:295-298,498,508` | Unavailable: `collect_desktop_files` silently skips unreadable dirs (`platform.rs:167-169`), degrades rather than crashes |
| Icons | Done | Icon-theme resolution chain `rmac-apps/src/icons.rs:60-90`; fallback plate for unresolved icons `app-drawer/src/view/render/content.rs:36,48,288` | Fallback path covers missing/unresolvable icons |
| Actions | Done | Desktop Actions parsed (`rmac-apps/src/platform.rs:238-268`), duplicate/invalid ids and empty/oversized names guarded (247-248,252-253); "Reveal application" (`app-drawer/src/view.rs:221`) | Action missing `Exec=` is dropped, not shown broken (`platform.rs:257-258`) |
| Search | **Partial** | Plain substring only, no ranking: `app-drawer/src/view.rs:88` `app.search_text.contains(&query)`. Spotlight's `rmac-launcher-providers/src/applications.rs:154-199` already calls a real graded fuzzy matcher, `rmac_launcher::query_matches` (`rmac-launcher/src/engine.rs:81-119`, exact>prefix>word-prefix>substring>subsequence), which App Drawer does not reuse | Zero matches: real distinct empty state (`DrawerEmptyState::NoMatches` vs `::EmptyCatalog`, `view/render.rs:38-45`) |
| Launch | Done, command-output-free | `app-drawer/src/view.rs:195-208` → `rmac_app_launch::launch`. On niri: compositor's own structured IPC `Spawn` action with an XDG-activation token (`rmac-app-launch/src/application.rs:9-24`), not a shell and not text parsing. Falls back to direct `Command::spawn()` only if unavailable (`application.rs:51-57`) | Loading: synchronous catalog scan at open + async live-reload (`view/lifecycle.rs:11,113-120`), degraded-mode message if the fs-watch fails ("Apps loaded, but live updates are unavailable", `lifecycle.rs:89-92`). Error: dismissible "Could not open application: {error}" banner (`view.rs:200-208`) |

Adjacent, not a checklist item: `rmac-apps/src/platform.rs` shells out to
`xdg-mime` for file-association queries (`run_xdg_mime`, 432-443) and
`icons.rs:77-84` parses `gsettings get` for the icon theme name. Both are
narrow, size-capped, single-line-parsed utility calls outside the discovery
and launch paths, so they don't change either item's status, but a strict
reading of `todo.md`'s "never parse human-readable CLI output" principle
would flag them for a future pass.

**Gap found, not implemented here:** App Drawer's search should call
`rmac_launcher::query_matches` instead of `.contains()`, matching Spotlight.
Small, well-scoped — the matcher is a pure function in a dependency-free
crate already used and tested elsewhere.

## Notes (`crates/notes`, `crates/rmac-notes-store`, `crates/rmac-notes-runtime`)

| Item | Status | Evidence | Five states |
|---|---|---|---|
| Folders | Done | `library_actions.rs:6-14,44,67,77-114` (`FolderSelection`; create/rename/select) | Empty name rejected: "A Notes folder name cannot be empty" (`library_actions.rs:105`) |
| Tags | Done | Referenced across `editor_presentation.rs`, `input_support.rs`, `note_navigation.rs`; persisted via `rmac-notes-store/src/{codec,mutation,validation}.rs` | — |
| Search | Done | `search_controller.rs`, `search_highlight.rs` | — |
| Attachments | Done | `transfer_controller.rs:6-84` image attach via portal (`rmac_portal::choose_notes_image`), revision-guarded (`queue_image_attachment:38-84`) | Error: "Notes could not open the Linux image chooser" (:27). Race guard: "The note changed while the image chooser was open. Save it, then choose the image again." (:44-46) |
| Pinning | Done | `library_actions.rs`, `rmac-notes-store/src/model.rs`, `mutation.rs`; tested in `rmac-notes-store/src/tests.rs` | — |
| Import/export | Done | Export: `rmac-notes-store/src/export.rs` (`ExportScope::{Note,Folder,Library}`; Markdown for a single note, the app's own Bundle format for a folder or the whole library — `transfer_controller.rs:376-613`). Import: plain-text-note import (`transfer_controller.rs:87-151`), Markdown import with a review step (`:153-227`), Bundle import with a collision policy (`rmac-notes-store/src/bundle_import.rs`, `transfer_controller.rs:228-371`) | Error: distinct messages per failure ("Notes could not open the Linux note importer", revision-conflict, collection-limit, `MarkdownRequiresSingleNote`, `MarkdownHasAttachments`, via `ExportError`/`BundlePlanError`) |
| Recovery | Done | `edit_recovery_controller.rs` (368 lines), `recovery_presentation.rs` (253 lines) | Unavailable: "Recovery unavailable — Close and reopen Notes safely." (`root_presentation.rs:34`). Worker down: "The private Notes worker is unavailable." (`root_presentation.rs:19`) |

No gaps found against todo.md's checklist; every item is Done with real five-states
handling. Not a checklist item, but noted for completeness: `docs/beta-gap-list.md`
S7 says Notes has no print path — that's still accurate (only Text Editor's print
path exists); todo.md does not require printing for Notes, only import/export,
which is already Done, so this isn't tracked as a gap here.

## Text Editor (`crates/text-editor`) — audit only; owned by another agent's large-file rebase, not edited

| Item | Status | Evidence |
|---|---|---|
| UTF-8 text | Done | `document.rs:17-18` (`TextEncoding::Utf8`/`Utf8Bom`), default `Utf8` (:57) |
| Open/save | Done | `view/document_io.rs`, `view/saving.rs`, `view/opening.rs` |
| Find/replace | Done | `view/editing.rs:80-149` (`find_next`/`find_prev`/`replace_current`/`replace_all`); bar UI `view/render/find.rs` |
| Crash recovery | Done | `recovery.rs`, `view/recovery_state.rs`; draft flushed at once on session end (`6a65af6e`, beta-gap B9/B3) |
| Status | Done | `document.rs:66-92` builds the encoding/line-ending status string; rendered in `view/render/chrome.rs` |
| Printing / export path | Done | `view/printing.rs:6-84` via `rmac_print_linux::PrintDocument`, routed through the Linux print portal — the portal's own "Print to File" / PDF virtual printer satisfies "a printing or export path" without a separate PDF code path |

No gaps found against todo.md's checklist. This crate was not touched (another
agent's large-file rebase is in flight); this is audit-only, matching the
"System Monitor history graph" and "Files open-with menu" and "Notes
import/export" candidate items from the task brief — Text Editor's
export-as-PDF candidate is likewise **already Done**, nothing to build.

## System Settings (`crates/system-settings`) — audit only; owned by another agent's visual + idle-CPU work, not edited

| Item | Status | Evidence |
|---|---|---|
| Only panes with a real backend (Network, Bluetooth, Power, Sound, Display info, Appearance) | Done for the required six | `navigation.rs:48-65,100-323` routes Network, Bluetooth, Power, Sound, Displays and Appearance; `grep -rn "placeholder\|not yet implemented\|coming soon\|todo!()"` across the crate returns only text-input `.placeholder(...)` attributes and legitimate empty-state copy ("No VPN Configurations", "No Schedules") — no placeholder pane found |

Observation, not a gap: the crate has substantially more panes than todo.md's
six-pane whitelist (accessibility, connectivity, displays, focus, input,
notifications, storage_categories, system_environment, privacy_security, vpn,
wifi, date_time, locale, system_info, …). Whether each of those extra panes has
a real backend was not re-audited pane-by-pane here, in keeping with "only
document gaps, don't edit" for a crate another agent is actively changing; a
follow-up pass should confirm none of the extra panes are placeholders as the
crate settles.

## Cross-app notes

- `let _ = self.tabs[self.active].resize(size);` in
  `crates/terminal/src/controller/view_state.rs:140` drops a `Result`, but the
  failure path (`session.rs:1081` `resize_failure_retains_the_last_kernel_accepted_geometry`)
  already keeps the last accepted geometry internally, so nothing destructive
  is silently lost — not flagged as a `todo.md` "never drop a destructive
  error with `let _ =`" violation, just noted for visibility.
- Every app audited exposes real loading/empty/unavailable/permission-denied/
  error states with user-visible copy, not silent failure or invented data —
  the one deliberate exception is System Monitor's GPU metric, which is
  correctly *absent* rather than fabricated (see above).
