# ADR 0020 — Unsaved work survives every way a session can end

- **Status:** accepted 2026-09-24.
- **Scope:** `crates/rmac-ui/src/session.rs` and `session/`, `crates/rmac-app-menu/src/unsaved.rs`,
  `shell/bins/rmac-menubar/src/session_guard.rs` and `unsaved_guard.rs`, Text Editor's recovery
  (`crates/text-editor/src/recovery.rs`, `view/document_state.rs`), and Notes' quit path
  (`crates/notes/src/startup_controller.rs`, `crates/rmac-notes-runtime/src/worker.rs`). Item 7
  adds the power button: `crates/rmac-shortcuts/src/power_key.rs` and `lock.rs`, the
  `XF86PowerOff` bind in `packaging/rmac-session/shell.kdl`, and the menu bar's dialog
  (`menu_model::system_confirmation`).
- **Closes:** beta gap B9.

## The question

Log Out, Restart and Shut Down in the menu bar ask every window to close first, so an edited
document gets its Save / Don't Save / Cancel alert. A session can end in many other ways:

- `systemctl poweroff` or `reboot` typed in Terminal;
- the power button (logind `HandlePowerKey`);
- another tool calling logind's `PowerOff`, `Reboot` or `Suspend`;
- a lid close, or UPower's critical-battery action;
- the out-of-memory killer;
- niri exiting.

Before this change, none of these gave an app any chance to act. No app handled SIGTERM, and none
held a logind inhibitor. Text Editor writes its recovery draft 2 s after the last keystroke, and
Notes commits an edit 0.5 s after it. Whatever was typed inside those windows was lost.

What should Lulo OS do, and how close can Linux get to the Mac?

## What the Mac does

- A restart the user starts from the Apple menu asks every app to quit. Any app can cancel it.
  Lulo OS already does this through the menu bar.
- A forced end does not ask. That includes `sudo shutdown` in Terminal, a low-battery power-off
  and a long press of the power button. The Mac relies on autosave and Resume instead: a
  document-based app has already saved its edits, and the next launch reopens them "with unsaved
  changes".
- Sleep never asks apps anything.

So the Mac lets an app refuse only the restart the user asked it for. Every other path depends on
the work already being on disk.

## Decision

1. **Every rmac-ui app quits gracefully on SIGTERM, SIGHUP and SIGINT**
   (`rmac_ui::session`, installed by `init_application`).
   - The handler only writes the signal number to a pipe.
   - A thread blocked on that pipe passes it to the main thread, which calls `cx.quit()`. GPUI
     then runs every `on_app_quit` hook before the process exits.
   - Handlers are installed with `SA_RESETHAND`, so a second signal kills at once.
   - If the app has not exited 10 s after the signal, the thread re-raises the signal. A hung app
     therefore never holds up a shutdown.
   - A signal the process inherited as ignored, as under `nohup`, stays ignored.
2. **Apps register a synchronous "preserve" hook.** `rmac_ui::session::preserve_on_session_end`
   runs the hook on every quit, and whenever the session asks, as described in 4.
   - Text Editor's hook writes the window's recovery draft at once. It uses the same versioned,
     atomic record as the debounced autosave.
   - A new `RecoveryWriter` orders the two writers by document generation. An autosave that
     finishes late can then never overwrite or delete a newer draft written by the hook.
   - Notes' quit hook passes the newest typing to its repository worker, sends `Shutdown`
     (which commits a scheduled edit), and waits up to 3 s for the worker thread to stop.
     `NotesWorkerClient::wait_until_stopped` is new for this.
3. **Apps say when they hold unsaved work.** `rmac_ui::session::set_unsaved` tracks each view.
   - While any view in a process is unsaved, the process owns `org.rmac.UnsavedWork.p<pid>` on the
     session bus and serves `org.rmac.UnsavedWork1.Preserve` at `/org/rmac/UnsavedWork`.
   - The bus connection opens with the first unsaved document. An app that never has one never
     connects.
   - A process that dies loses its name with its connection, so a crash cannot leave a stale
     claim behind.
   - Text Editor reports its dirty state. A window that is closing (after Save, or after the user
     chooses Don't Save) is not unsaved.
4. **The menu bar holds a logind *delay* inhibitor, `shutdown:sleep`, while any unsaved-work name
   exists.**
   - It follows the names through `NameOwnerChanged` with `arg0namespace`, so it never polls.
   - When `PrepareForShutdown(true)` arrives, it calls `Preserve` on every owner. Before a
     shutdown, but not before a sleep, it also asks every window to close, as Log Out does. Then
     it releases the inhibitor.
   - All of this happens within logind's `InhibitDelayMaxUSec`, less a 0.5 s margin: 4.5 s with
     the default of 5 s.
   - `PrepareForSleep(true)` gets the same treatment without closing windows. After a wake, or a
     cancelled shutdown, the inhibitor is taken again if there is still unsaved work.
5. **No "block" inhibitor.** No Mac app can refuse a forced shutdown, a low-battery power-off or a
   lid-close sleep, and a Linux one should not either:
   - A block on `shutdown` makes the power button and UPower's critical action do nothing. The
     battery then runs flat and takes the unsaved work with it.
   - A block on `sleep` keeps a closed laptop running in a bag.

   The one path where the Mac lets an app refuse is the user's own restart, and there the menu
   bar's Log Out, Restart and Shut Down already ask each window. A delay inhibitor never refuses
   anything. It only holds logind for a few seconds.
6. **Recovery drafts are found again after a reboot.**
   - Text Editor names each draft after its owning process ID and skips drafts whose owner is
     alive. Process IDs restart after a boot, so an unrelated process could hold the old number
     and hide the draft forever.
   - On Linux, a live owner now also has to run the same program, compared through
     `/proc/<pid>/comm`.
7. **The power button sleeps; it never powers off by itself** (amended 2026-09-24, audit
   finding PWR-01). logind's Ubuntu default is `HandlePowerKey=poweroff`, so one short press
   used to end the session at once and take unsaved work with it. The Mac sleeps (and so locks)
   on a short press and shows "Are you sure you want to shut down your computer now?" with
   Restart / Sleep / Cancel / Shut Down on a long press.
   - Lulo OS does what GNOME does. While the session runs, the lock coordinator
     (`rmac-lock-coordinator`, `crates/rmac-shortcuts/src/lock.rs`) holds a logind
     `handle-power-key` **block** inhibitor and handles the key itself. It blocks only logind's
     reaction to the key. `systemctl poweroff`, the menu bar's Shut Down, UPower's critical
     action and a lid close still work, so item 5 stands.
   - niri is the only process that sees the key. `shell.kdl` turns off niri's own handling
     (`disable-power-key-handling`, which would suspend without locking and never show the
     dialog) and binds `XF86PowerOff` to `rmac-shortcut-dispatch power-key`. The coordinator
     listens on that dispatch socket. It binds the socket *before* it takes the inhibitor, and
     drops the inhibitor if the listener fails, so a press is never swallowed.
   - A single press locks the session, waits for the lock screen, and then calls logind's
     `Suspend`. The pre-sleep delay inhibitors above then preserve unsaved work as for any
     sleep. On the lock screen a press sleeps at once.
   - An x86 laptop's ACPI power button reports press and release together, so a long press
     cannot be detected. Holding it for about four seconds is the firmware's forced power-off,
     which no software sees. A **second press within 0.8 s** therefore stands in for the long
     press. The coordinator asks the menu bar, through the `shutdown-dialog` dispatch socket,
     for the dialog, which opens where the system menu opens. Its Restart and Shut Down run the
     same `quit_all_then` path as the menu items, so every window gets its Save alert first.
     The Mac's 60-second automatic shutdown is left out.
   - Presses in the 2 s after a wake are ignored. Many laptops deliver the press that woke them
     once they resume, and it would otherwise send the computer straight back to sleep.
   - Nothing changes logind's configuration. No `logind.conf` drop-in is shipped, so the Ubuntu
     / GNOME session keeps its own behaviour. While the coordinator is not running (before it
     starts, or during a 1 s restart), the button does what logind is configured to do.
   - logind honours a block inhibitor only while its session is the active one. After a switch
     to another VT or to the greeter, the button follows logind's own setting again.

## How each path is covered

| Path | What keeps the work |
|---|---|
| Menu bar Log Out / Restart / Shut Down | Each window's close guard (Save alert). Unchanged. |
| `systemctl poweroff` / `reboot`, logind `PowerOff` from any tool | Delay inhibitor, then `Preserve`, then every window is asked to close. Afterwards systemd's SIGTERM quits each app through its hooks. |
| The power button, one press | Never a shutdown: the lock coordinator's `handle-power-key` block inhibitor, then lock, then sleep (item 7). |
| The power button, a second press within 0.8 s | The Restart / Sleep / Cancel / Shut Down dialog. Restart and Shut Down ask every window to close, as the menu items do. |
| Lid close, `systemctl suspend`, idle suspend | Delay inhibitor, then `Preserve`. |
| UPower critical battery (power-off or hibernate) | Same as shutdown or sleep. |
| niri exits | Expected, but not yet verified: the calloop run returns an error when the Wayland connection drops, and GPUI then runs the quit hooks. The session stop that follows sends SIGTERM. |
| `kill <pid>`, Force Quit's Quit, `systemctl --user stop` | SIGTERM, then the quit hooks. |
| OOM killer, `kill -9`, power loss | Nothing can run. The debounced autosave (Text Editor 2 s, Notes 0.5 s) bounds the loss. |

## What is not covered, honestly

- **Terminal's running processes** cannot be preserved. A shell job ends with the session, as on
  the Mac. Terminal does not claim unsaved work, so it never delays a shutdown.
- **Notes does not claim unsaved work.** Its worker commits within 0.5 s of the last keystroke.
  The SIGTERM path waits for that commit, so only sleep followed by a dead battery can lose the
  last half second.
- **Hard kills** (SIGKILL, OOM, power loss) lose whatever the debounce had not yet written. That
  is at most 2 s of Text Editor typing.
- **The power button has no long press.** The hardware cannot report one, so a quick second
  press opens the Mac's long-press dialog instead (item 7). The dialog opens under the menu bar
  on the first display, not centred on the screen as on the Mac. While the lock coordinator is
  down, or the session is not the active one, the button follows logind's `HandlePowerKey`,
  which is `poweroff` on Ubuntu.
- **The close requests on a forced shutdown** are a courtesy. A Save alert that appears cannot be
  answered before logind continues. The draft is already on disk, and the next launch offers it
  back.

## Idle cost

- The signal thread is blocked in `read(2)`.
- The menu bar waits on three D-Bus signal streams.
- An app connects to the bus only once it has an unsaved document, and then only claims or
  releases a name when its dirty state flips.
- Nothing polls. The only timers run during a shutdown: the 250 ms window checks while windows
  close (as in Log Out) and the one-shot forced-exit deadline after a signal.

## How to verify on the reference laptop

Run each check once, with the journey lock held:

1. Type into a new Text Editor document and wait 1 s. Run `systemctl poweroff` in Terminal.
   - Expect a short delay, and `quitting on signal 15` in the Text Editor journal.
   - After boot, open Text Editor. It should offer the draft with every character typed.
2. Type into Text Editor and immediately run `systemctl suspend`, then wake the laptop. Run
   `busctl --user list | grep UnsavedWork`. The name should be present, and
   `systemd-inhibit --list` should show "Lulo OS … delay" again.
3. Type into Text Editor, save, and run `systemd-inhibit --list`. There should be no Lulo OS
   entry and no `org.rmac.UnsavedWork` name.
4. Type into a note and run `kill <Notes pid>` within 0.5 s. Reopen Notes. The note should show
   the last keystroke.
5. Type into Text Editor, run `kill -TERM <pid>`, and reopen it. It should offer to restore the
   draft.
6. Run `systemd-inhibit --list`. It should show `Lulo OS … rmac-lock-coord handle-power-key …
   block`, and no `niri … handle-power-key` entry. (polkit allows this inhibitor only to local
   sessions; a user service such as the coordinator qualifies, an SSH shell does not. The
   ignored test `power_key_inhibitor_is_a_block_on_handle_power_key`, run through
   `systemd-run --user`, showed the Lulo OS entry on the reference laptop on 2026-09-25.) Only the owner presses the power button: one
   press should lock and then sleep; two quick presses should show the dialog; neither should
   power off.
