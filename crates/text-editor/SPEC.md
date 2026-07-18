# Text Editor product and authority specification

Text Editor is rmac's fast local plain-text editor. It should feel as calm and
direct as TextEdit on macOS while remaining honest about Linux authorities,
file formats, conflicts, and recovery. It never invents cloud sync, silently
repairs undecodable bytes, overwrites an externally changed document, or writes
plain text over a rich-text file.

## Prioritized journeys

1. Create an untitled document, type with a keyboard or IME, save through the
   desktop chooser, close, and reopen it without changing encoding or line
   endings unexpectedly.
2. Open UTF-8, UTF-8 BOM, UTF-16 LE, or UTF-16 BE text, inspect the detected
   format, edit, save atomically, and verify exact authoritative readback.
3. Open a document, change it in another application, then save in Text Editor.
   Text Editor must refuse the stale overwrite and offer Reload, Save a Copy,
   or an explicit reviewed overwrite path; it must retain the user's buffer.
4. Crash or terminate with unsaved text, reopen, restore the correct draft, and
   keep the draft recoverable until a document save or explicit discard has
   succeeded.
5. Find and replace by keyboard, navigate every result, cancel with Escape, and
   preserve selection/focus through empty, no-match, and replacement states.
6. Open RTF as a bounded read-only preview. Continue as a new plain-text
   document only after an explicit action; never overwrite the source RTF.
7. Create independent document windows with Command-N or open multiple selected
   documents at once. A clean empty untitled window may adopt the first
   selection; every other document gets its own window. Each window owns its
   path, revision, dirty baseline, recovery identity, dialogs, watcher, and
   close guard, and closing one window never bypasses another window's guard.
8. Print or export only through a documented Linux authority. Until that
   authority is implemented, no control may imply printing or PDF export works.

## Platform authorities

- On Linux, GPUI's path prompts use the XDG Desktop Portal FileChooser and bind
  the request to the application window. Cancellation is a normal outcome.
  Portal-returned local paths are still revalidated by the document storage
  boundary before every read or write.
- `rmac-storage` owns adjacent-temporary atomic replacement, file sync, parent
  directory sync, permission preservation, and temporary-file cleanup.
- The document domain owns bounded decoding/encoding, line-ending policy, the
  exact raw revision opened or last written, conflict preflight, and readback.
- The XDG state directory owns private crash-recovery records. Recovery data is
  not a substitute for the user document and is never treated as saved content.
- The filesystem remains authoritative. File monitors are refresh hints only;
  an exact fresh preflight is required before overwriting an opened document.

There is no portable atomic compare-and-replace operation against arbitrary
cooperating and non-cooperating editors. Text Editor therefore performs an
exact byte preflight immediately before its atomic replacement and exact
readback afterward, reports the narrow after-preflight race, and keeps watching
for subsequent changes. It must never describe this as a transactional lock.

## Document contract

- Maximum supported document size is explicit and bounded before decoding.
- Supported writable encodings are UTF-8, UTF-8 with BOM, UTF-16 little-endian
  with BOM, and UTF-16 big-endian with BOM. Invalid Unicode and unsupported
  byte-oriented encodings fail without lossy replacement characters.
- Internal text uses LF. Uniform LF, CRLF, and CR sources retain that convention
  on save. A mixed source records both its mixed origin and a deterministic
  normalization target; the status UI must disclose conversion before a dirty
  save.
- A clean save may preserve the exact original bytes. A dirty save encodes from
  the typed format, writes atomically, reads back the complete file, and adopts
  a new baseline only when bytes match exactly.
- RTF parsing is a bounded, inert preview path. Embedded commands, objects,
  links, and attachments are not executed.
- File paths, raw bytes, document text, recovery text, and filesystem details
  are private. User-facing errors describe the operation and safe resolution
  without logging content or exposing unrelated paths.

## Recovery contract

- Recovery writes are debounced, atomic, private (`0600` on Unix), versioned,
  bounded, and associated with the window/document identity rather than one
  global untyped slot.
- A newer edit invalidates an older scheduled write. New/Open/Close cannot clear
  the only recoverable copy until the requested save or explicit discard has
  completed.
- Startup tolerates a missing record, quarantines malformed/oversized records,
  migrates the legacy single draft without data loss, and presents recovery
  metadata before restoration.
- Successful document save removes only the matching recovery record. Cleanup
  failure stays visible and does not make a failed document save look clean.

## External changes and failures

The UI has distinct states for loading, saving, chooser cancellation, missing
file, permission denial, read-only destination, unsupported encoding, excessive
size, malformed UTF-16, external replacement/removal, conflict, disk full,
atomic-write failure, readback mismatch, recovery failure, and portal failure.
The editable buffer and last-known-good document metadata remain visible after
failure. Retry never bypasses conflict validation.

External-change choices are precise:

- **Reload** first protects a dirty local buffer (Save a Copy or discard), then
  replaces the buffer from a fresh bounded decode.
- **Save a Copy** uses the portal and never mutates the conflicting source.
- **Overwrite Anyway** is destructive, names the changed document, requires a
  second exact preflight against the revision shown in that confirmation, and
  performs normal atomic write/readback. It is never the default button.

## Keyboard and accessibility

- `Super/Command-N`, `-O`, `-S`, `-Shift-S`, `-F`, `-Shift-F`, and `-W` cover
  New Window, Open, Save, Save As, Find, Replace, and Close Window.
- `Enter`/`Shift-Enter` or `Super/Command-G`/`-Shift-G` traverse matches;
  Escape closes the find bar or cancels the topmost dialog without data loss.
- Tab order follows toolbar, find/replace controls, document, status controls,
  and modal buttons. Destructive actions are never initial focus or default.
- The editor, filename, edited state, format, line/column, counts, matches,
  loading/saving, recovery warning, conflict warning, and dialog purpose need
  roles, names, state, and announcements in the supported accessibility stack.
- At 100%, 125%, 150%, and 200%, text remains readable, controls do not clip,
  focus is visible, contrast tokens remain semantic, and reduced motion removes
  nonessential transitions.

## Visual states

The window uses the shared rmac toolbar, semantic tokens, spacing, controls,
dialogs, focus ring, and live appearance preferences. Required references cover
untitled clean/edited, opened format variants, find/replace, RTF preview,
recovery, portal/storage error, external conflict, busy save, light, dark,
increased contrast, reduced motion, and every supported scale.

## Acceptance evidence

G1 remains unchecked until focused unit tests and the Ubuntu/niri reference PC
prove all journeys above, including portal cancellation/restart, IME and
clipboard, atomic and injected storage failures, exact conflict/readback,
external delete/replace, crash recovery, malformed and maximum-size fixtures,
multi-window isolation, keyboard-only operation, Orca/AT-SPI semantics, visual
references, launch/idle/edit/search/save performance, and no private content in
errors or committed evidence.
