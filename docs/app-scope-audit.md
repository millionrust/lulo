# 1.0 app scope audit, 2026-09-24

This audit checks each app against the "1.0 scope per app" list in
[`todo.md`](../todo.md). There, "Done" includes the loading, empty,
unavailable, permission-denied and error states, so every item below records
those states as well as the feature.

The audit read the code on `dev` at `ea11db97`, plus the fixes made on branch
`app-scope` (listed in section 9). Nothing was run on the laptop, so every
**Done** means "present and wired in the code" and still needs the laptop
checks in section 10.

- **Status key:** **Done**, **Partial**, **Missing**, **Fixed** (fixed on
  `app-scope`).
- **State key:** L = loading, E = empty, U = unavailable, P =
  permission-denied, X = error. "n/a" means the state cannot happen for that
  item.
- **Out of scope for fixes:** System Settings and Text Editor were being
  edited by other agents during this audit. Their gaps are recorded here but
  were not fixed.

## Summary

| App | Verdict | Open gaps |
|---|---|---|
| Text Editor | Every item Done except export | No Export as PDF… (print to PDF works through the portal) |
| Notes | Done, now including printing | No import from other apps (`.enex`) and no PDF export |
| Terminal | Done | Find doesn't join soft-wrapped lines; a failed shell start gives one message for every cause |
| Files | Done | Open With shows no loading state while associations load |
| System Monitor | Done | Per-process CPU reads 0.0 until the second sample; the Energy column is an estimate the UI doesn't label |
| Apps | Done, now with ranked search | First catalog scan blocks the UI thread; unreadable `.desktop` files are skipped silently |
| System Settings | Partial: only the six named panes were verified | 19 more panes to verify for real backends |

## 1. Text Editor (`crates/text-editor`, document only)

| Item | Status | Evidence | States |
|---|---|---|---|
| UTF-8 text | Done | `src/document.rs:6-18` `TextEncoding` (UTF-8, UTF-8 BOM, UTF-16 LE/BE); decode `:146-159`, encode `:189-205`; round-trip tests `:333-423` | X: `CodecError::InvalidUtf8` (`:116-126`) is shown as "Text Editor could not read the selected document" (`src/view/document_io.rs:32-51`) |
| Open/save | Done | `src/view/document_io.rs:14-51`, `src/view/saving.rs`, bounded reads in `src/storage.rs` | X: `SaveFailure` (`document_io.rs:15-29`); external changes are `ExternalChange::{Modified, Missing, Unreadable}` (`src/view.rs:88-92`) with a reload flow (`src/view/conflicts.rs`); U: `DocumentWatchEvent::Unavailable` (`view.rs:94-97`) |
| Find/replace | Done | `src/view/render/find.rs` | E: blank status for an empty query and "Not found" for no matches (`find.rs:12-17`); Replace is disabled while printing (`:88`, `:97`) |
| Crash recovery | Done | `src/recovery.rs` (bounded discovery `:12-15`), `ActiveAlert::Recover` (`view.rs:73-74`); drafts are written at session end (`6a65af6e`) | L: `recovery_loading` (`view.rs:143`); U: `Discovery.unavailable` (`recovery.rs:60`, `:106`); X: malformed records are counted, never fatal (`recovery.rs:56`) |
| Status | Done | Format and encoding status in the document menu (`src/view/render/chrome.rs:52`, `:68`), one-line notices (`view.rs:148`). TextEdit has no status bar, so none is drawn. | — |
| Printing or export | **Partial** | ⌘P goes through the XDG Print portal (`src/view/printing.rs`, `crates/rmac-print-linux/src/linux.rs`). There is no Export as PDF… action. | L: `print_busy`; U: a non-Linux build explains that printing is unavailable (`printing.rs:78-87`); X: portal and window-handle failures raise an alert (`printing.rs:31-43`, `:66-71`); cancelling is silent; RTF previews are refused with an explanation (`:16-24`) |

**Gap (not fixed; the crate belongs to another agent).** The PDF renderer
already exists and printing already uses it: `rmac_print::render_pdf`
(`crates/rmac-print/src/lib.rs:12`, called from `rmac-print-linux/src/linux.rs`).
Export as PDF… needs only a Save panel and a write of that output.

## 2. Notes (`crates/notes`, `rmac-notes-runtime`, `rmac-notes-storage`, `rmac-notes-store`)

| Item | Status | Evidence | States |
|---|---|---|---|
| Folders | Done | `crates/notes/src/library_actions.rs:59` (create), `:73` (rename), `:133`/`:145` (delete); `FolderRecord` in `rmac-notes-store/src/model.rs:50` | E: "No notes in this folder" (`note_navigation.rs:516`); X: rejected actions set the status message |
| Tags | Done | The Tags field (`editor_presentation.rs:233-255`) and inline `#tags`; `tags` in `model.rs:77` | No tag browser (not required for 1.0) |
| Search | Done | `SearchState` in `rmac-notes-runtime/src/search.rs:447-454`; rendered in `note_navigation.rs:502-511`; background worker in `search_worker.rs` | L: "Searching…"; E: "No matching notes"; U/X: "Search is unavailable", with the failure reason where one is known |
| Attachments | Done | `rmac-notes-storage/src/attachment.rs`; UI in `editor_presentation.rs:6-140`; add and remove in `transfer_controller.rs:5-83` | L: "Loading preview…"; U: "Preview unavailable" with Try Again; E: no attachment row; X: an image chooser that fails to open is reported (`transfer_controller.rs:28`) |
| Pinning | Done | `TogglePin` (`root_presentation.rs:88`), `SetPinned` (`library_actions.rs:349-352`), `set_note_pinned` (`rmac-notes-store/src/mutation.rs:517-536`), the Pinned group (`note_navigation.rs:216`) | X: rejections go to the status message |
| Import/export | Done | Markdown export of one note (`rmac-notes-store/src/export.rs:246`) and whole-library bundles (`rmac-notes-storage/src/export.rs:17-24`); Markdown import with review and bundle import with collision review (`dialog_presentation.rs:320-675`, `transfer_controller.rs:88-293`) | L: progress dialogs; E: a cancelled chooser does nothing; X: chooser failures (`transfer_controller.rs:114`, `:270`); a library change during review is detected (`:168-171`, `:263-266`). Not supported: Apple Notes `.enex` import and PDF export |
| Recovery | Done | Each debounced edit is saved as a durable draft before commit (`rmac-notes-runtime/src/worker.rs:2274`, `:2320`); drafts are reviewed on the next start (`recovery_presentation.rs:76`); the last edit is flushed on quit (`startup_controller.rs:223-240`) | Covers a crash, not only a clean quit; conflicting and orphaned drafts get their own choices |
| Close (⌘W) and quit | Done | `startup_controller.rs:30`, `root_presentation.rs:127`, `main.rs:234` | Open choosers, imports and print dialogs block closing, with a message saying why |
| Printing | **Fixed** | ⌘P and File › Print…: `src/print_controller.rs:29`, menu at `crates/rmac-app-menu/src/lib.rs:157` | E: "Select a note to print."; L: the window stays open while the dialog is up (`runtime_controller.rs:84`); X: portal errors appear in the status line; an edit made while the dialog is open prints nothing |

## 3. Terminal (`crates/terminal`)

| Item | Status | Evidence | States |
|---|---|---|---|
| Real PTY | Done | `portable_pty` `openpty` (`src/session.rs:536`); spawn `:566` | X/U: `SessionStartError` (`session.rs:159-181`) is written into the grid by `Session::failed` (`:623`), and the tab reads "Unavailable". P: a shell that can't run shows the generic start error |
| Dynamic resize | Done | `resize_to` (`controller/view_state.rs`), PTY resize with rejection handling (`session.rs:240-270`) | X: "The shell rejected the new window size…" |
| Scrollback | Done | Budget shared across tabs (`src/emulator.rs:70`), rebalanced per tab (`controller/tab_lifecycle.rs:6-30`) | X: a poisoned grid lock sets `operation_error` |
| Selection | Done | `Selection` (`src/ui_state.rs`), pointer selection (`controller/pointer.rs`), Select All / Command / Command Output | — |
| Search | **Fixed** | Find (⌘F) used to highlight only the rows on screen, by byte offset. ⌘G, ⇧⌘G, Return and Shift-Return now step through every match in the scrollback, scroll to it and select it, and show "3 of 12": `src/find.rs`, `controller/renderer/grid.rs:38` (`find_step`), `controller/renderer/overlays.rs:24-60`, bindings `controller/lifecycle.rs:22-55` | E: "Not found"; X: a poisoned grid lock sets `operation_error`. Limitation: a match that soft-wraps onto the next row isn't found |
| Tabs | Done | `controller/tab_lifecycle.rs:32-168`; `MAX_TABS` is reported, not silent | Closing a tab or window with a running job asks "Do you want to terminate running processes…?" (`controller/renderer/dialogs.rs:17`, `:26`); Dock, menu-bar, ⌘Q and log-out closes go through the same question (`controller/lifecycle.rs:134`) |
| Profiles | Done | `src/profiles.rs`; ⌘, opens the picker (`controller/lifecycle.rs:122`) | X: a profile that fails to load or save shows `persistence_error` |

## 4. Files (`crates/finder`, `rmac-quick-look`, `rmac-mounts`)

| Item | Status | Evidence | States |
|---|---|---|---|
| Safe file operations | Done | Recursive copy off the UI thread (`src/view/operations.rs:115-134`); symlinks are recreated, never followed (`src/file_ops.rs:316`, `:329`); `O_NOFOLLOW` opens; copying a folder into itself is refused | P: `permission_denied_move_never_falls_back_to_copy_and_delete` (`file_ops.rs:1737`); low disk: `low_space_preflight_refuses_the_batch_before_mutation` (`:2705`); cross-filesystem: `:1752`; X: a partial destination is reported |
| Conflict policy | Done | `ConflictDecision` Keep Both / Replace / Skip (`src/conflict.rs:35`) | No silent overwrite path was found |
| Cancel | Done | Cancellation tests (`file_ops.rs:1860`, `:1912`, `:2133`, `:2223`, `:2320`) | A cancel leaves the source intact, including mid cross-device and journaled moves |
| Trash | Done | Trash first; Delete Immediately is a separate command; Empty Trash… with confirmation (`src/view/permanent_delete_controller.rs:67`, ⇧⌘⌫) | Collisions never overwrite (`src/trash_store.rs:4397`) |
| Undo | Done | `src/undo_journal.rs`, `src/view/undo_controller.rs` | X: refuses to undo a move when space is short (`file_ops.rs:223-229`) |
| Mounts | Done | `rmac-mounts` discover, revalidate, unmount and watch; `src/view/mount_controller.rs` | U: a failed mount watch shows a banner (`mount_controller.rs:9`); a mount disappearing is tested (`src/view/tests.rs:59`); X: eject errors are shown |
| Search | Done, now with **Fixed** empty state | Ranked recursive search (`src/view/search_info_controller.rs:23`) with a result summary; List and Icon views now say "No Matching Items" (`src/view/list_presentation.rs:663`), as Gallery already did | L: "Searching…"; E: **Fixed**; X: "Search could not safely read this folder"; a cancelled search is silent; skipped folders are counted in the summary |
| Previews | Done | Quick Look `Load::{Loading, Ready, Failed}` (`crates/rmac-quick-look/src/panel.rs:73`) | P: "You don't have permission to see this item." (`content.rs:410`); U: a missing converter falls back to a summary; X: separate messages for not found and changed while loading |
| Open with | Done | Context menu Open With… (`src/view/chrome_presentation/menus_tabs.rs:91-92`), File menu, picker with Always Open With (`src/view/open_with_controller.rs`) | X: refused for the Trash, the Applications view and folders; a failed association load is reported. L: no indicator while associations load (gap) |

Files safety rules from `todo.md`: all are met in the code (off-thread
recursion, no symlink following, explicit conflict policy, cancel keeps the
source, Trash first), and there are tests for cross-filesystem moves,
permission errors, low disk, collisions, disappearing mounts and interrupted
journaled moves.

## 5. System Monitor (`crates/activity-monitor`)

| Item | Status | Evidence | States |
|---|---|---|---|
| Process view | Done | `src/process_table.rs:87-284` (real `sysinfo`, sampled every 2 s from `src/sampling.rs`), 11 columns (`src/columns.rs`) | P: an unknown user shows as `uid N`; L: per-process CPU reads 0.0 until the second sample (gap) |
| Resource views | Done | CPU, Memory, Energy, Disk and Network tabs (`src/metrics.rs:6-49`, `src/view/render/metrics_panes.rs`); rows the platform doesn't provide are left out rather than invented (`src/host_stats.rs:1-29`) | L: **Fixed**. The CPU figures show "—" until two readings exist (`metrics_panes.rs:309`); they used to read 100% Idle. E: **Fixed**. "No Network Interfaces" (`metrics_panes/network.rs:106`) |
| Search and sort | Done | Search by name, PID or path (`src/view.rs:58-67`, `process_table.rs:287-300`); click-to-sort (`process_table.rs:396-407`) | E: **Fixed**. "No Matching Processes" (`src/view/render.rs:153`) |
| Safe terminate | Done | PID identity is checked again before confirming (`src/process_action.rs:39-67`); signals go through a pidfd (`src/process_signal.rs:33-80`); Quit / Force Quit dialog (`src/view/render/overlays.rs:6-57`) | E: "exited before confirmation"; U: "This system cannot send the requested signal."; P: "your account may not have permission"; X: other rejections. All are tested (`process_action.rs:118-178`) |
| History | Done | 60-sample (2 min) ring per metric (`src/metrics.rs:71-97`), drawn as the bottom graphs (`metrics_panes.rs:173-302`) | — |
| No invented GPU numbers | Done | There is no GPU column or tab at all (`src/columns.rs:12-24`, `src/metrics.rs:6-21`) | Related: the Energy column is an estimate from CPU and disk I/O (`process_table.rs:30-35`, `:272`). The code says so, but the UI does not (gap) |

## 6. Apps: App Drawer and Spotlight (`crates/app-drawer`, `rmac-apps`, `rmac-launcher*`)

| Item | Status | Evidence | States |
|---|---|---|---|
| Standards-compliant discovery | Done | `Type`, `Hidden`, `NoDisplay`, `OnlyShowIn`/`NotShowIn` and `TryExec` (`crates/rmac-apps/src/platform.rs:188-202`, `:624-625`); XDG data-dir order (`:82-93`); localized keys (`:203-210`); superseded GNOME apps hidden from browsing (`src/superseded.rs:52-107`, `crates/app-drawer/src/catalog.rs:111`) | L: **Partial**. The first `catalog::scan()` runs on the UI thread with no indicator (`crates/app-drawer/src/view/lifecycle.rs:11`); rescans run in the background. P: an unreadable `.desktop` file is skipped silently |
| Icons | Done | Icon-theme lookup with inheritance, sizes and `pixmaps` (`crates/rmac-apps/src/icons.rs:1-91`) | U: an app without an icon falls back to the generic icon plate (`8dd62774`; not checked line by line) |
| Actions | Done | `[Desktop Action]` groups (`platform.rs:237-275`) shown as alternate actions (`crates/rmac-launcher-runtime/src/coordinator.rs:308-315`) | Actions are launched by `Exec` only; D-Bus activation isn't supported (a documented limitation at `platform.rs:256-257`) |
| Search and launch | Done, search ranking **Fixed** | `Phase::{Loading, Results, Empty, Unavailable, Activating, ActivationFailed}` (`crates/rmac-launcher-runtime/src/model.rs:47-58`); `DrawerEmptyState` (`crates/app-drawer/src/accessibility.rs:146-165`). Search used to keep catalog order and match only a plain substring (`app.search_text.contains(&query)`); `crates/app-drawer/src/search.rs` (new, unit-tested) now grades each match exact > prefix > word-prefix > substring > subsequence — the same order Spotlight's `rmac_launcher::engine::match_quality` uses, kept as a small local copy rather than a dependency on that crate, which also pulls in `rmac-compositor` and shell invocation that catalog search has no other reason to need. `view.rs`'s `search_matching_indices` sorts by score, stable so ties keep catalog order (`76832c8c`) | P: "permission to start the application was denied" (`crates/rmac-app-launch/src/model.rs:33-34`); X: a dismissable error in the drawer (`crates/app-drawer/src/view/render.rs:170-204`); U: "Search providers are unavailable" |

## 7. System Settings (`crates/system-settings`, document only)

`todo.md` asks for real backends in Network, Bluetooth, Power, Sound, Display
info and Appearance, and for no placeholder panes.

| Pane | Status | Backend | States |
|---|---|---|---|
| Network | Done | NetworkManager through `rmac-network` (`src/connectivity.rs`) | L/X: `controller/network/refresh.rs:15-72` ("Could not update Network: …") |
| Bluetooth | Done | BlueZ through `rmac-bluetooth` | U: "No Bluetooth adapter is available…" (`controller/bluetooth/render.rs:66-68`), and actions are disabled; X: `state.rs:17-34` |
| Power (Battery) | Done | UPower and power-profiles-daemon through `rmac-power` | L/X: `controller/power.rs:14-24` |
| Sound | Done | PipeWire through `rmac-audio` | U: `Availability::{Available, Unknown, Unavailable}` (`src/sound.rs:38-72`); X: `controller/sound.rs:158-162` |
| Display info | Done | niri through `rmac-display` | L: "Loading displays from the compositor…" (`controller/displays/render.rs:25-26`) |
| Appearance | Done | `rmac-appearance` and its portal | L/X: `controller/appearance.rs:16-26` |
| No placeholder panes | **Partial** | 25 panes (`src/navigation.rs:46`). No `todo!()` or "coming soon" text was found, and each pane has a backend crate in `Cargo.toml`, but only the six above were traced end to end. The laptop audit also found Settings idling at 25% CPU (`docs/system-audit-2026-09-24.md` #1). | Empty states follow one pattern (`group_placeholder`, `controller/view_helpers/form.rs:319`) |

## 8. Gaps left open

Small or medium, with a clear place in the code:

1. **Text Editor: Export as PDF…** Write `rmac_print::render_pdf` output to a
   file chosen in a Save panel (owner: the Text Editor agent).
2. **System Monitor: per-process CPU on the first paint.** Show "—" in the
   CPU column until `sysinfo` has two samples, or take a second sample about
   200 ms after launch (`src/view.rs`, `process_table.rs:253`, `:269`).
3. **System Monitor: the Energy column.** Say in its header tooltip or help
   that it is estimated from CPU and disk I/O.
4. **Apps: the first catalog scan.** Move it to the background executor and
   show the drawer's loading state (`crates/app-drawer/src/view/lifecycle.rs:11`).
5. **Files: Open With loading.** Show progress in the picker while
   `file_association` runs (`src/view/open_with_controller.rs:26-59`).
6. **Terminal: shell start errors.** Tell "permission denied" apart from
   "not found" in `SessionStartError::StartShell` (`src/session.rs:159-181`).

Need design or backend work first:

- Notes import from Apple Notes `.enex` and other apps, and Notes PDF export
  (the latter could reuse `render_pdf`).
- Terminal Find across soft-wrapped lines.
- Desktop actions over D-Bus activation.
- Confirming that each of System Settings' other 19 panes has a real backend.

## 9. Changes on `app-scope`

| Commit | Change | Test |
|---|---|---|
| `a23265a8` | Terminal Find steps through the whole scrollback with ⌘G, ⇧⌘G, Return and Shift-Return, with a "3 of 12" count; highlights use cell columns, not bytes | `crates/terminal/src/find.rs` (7 tests, run standalone with `rustc --test`) |
| `b1d83bc0` | System Monitor: No Matching Processes, "—" before the first CPU reading, No Network Interfaces | none (view code) |
| `e6b131dd` | Notes prints with ⌘P; the menu bar lists Notes' Print… and Terminal's Find Next and Find Previous; the portal's errors no longer name Text Editor | `print_controller.rs` (3 tests, run standalone), `rmac-app-menu` `exported_hints_preserve_standard_macos_shortcuts` |
| `30c5de94` | Files: No Matching Items in List and Icon views | none (view code) |
| `dd043ed6` | Terminal's Find bar says "Not found", matching Text Editor | `find.rs` |
| `76832c8c` | Apps ranks search results by match quality (exact > prefix > word-prefix > substring > subsequence) instead of plain substring, keeping catalog order for ties | `crates/app-drawer/src/search.rs` (5 tests, run standalone with `rustc --test`) |

## 10. Laptop checks

1. **Terminal.** Run `seq 1 5000`, press ⌘F and type `4999`. Return should
   jump to the match and show "1 of 1". Search for `9`: ⌘G and ⇧⌘G should
   move down and up and wrap. Search for `zzz`: the bar should say "Not
   found". With two tabs, each keeps its own search.
2. **Notes.** Select a note and press ⌘P: the portal dialog should open.
   Cancel, then print to a PDF file. With the dialog open, ⌘W should say to
   finish the print dialog first. Choose File › Print… from the menu bar.
3. **System Monitor.** At launch, the CPU figures should read "—" for about
   2 s. Search for `zzzz` to see No Matching Processes.
4. **Files.** Type a name that matches nothing in List and in Icon view.
   Press Return for a ranked search with no results.
5. **Apps.** Open the drawer and type a partial app name (e.g. "term" for
   Terminal): an exact or prefix match should sort to the top over a
   substring hit elsewhere in an app's keywords.
