# Notes product and authority specification

Notes is rmac's calm, local-first place for short writing, lists, reference
material, and image attachments. It should feel immediate and thoughtfully
organized like Notes on macOS without inventing iCloud, collaboration, account
state, OCR, or synchronization. The application owns a durable local library;
ordinary files enter or leave it only through explicit import and export.

## Prioritized journeys

1. Create a note, type with a keyboard or IME, observe a truthful saved state,
   restart the application or session, and recover the exact title, body,
   tags, folder, pin, checklist, and attachment relationships.
2. Create, rename, and organize folders; move notes between them; tag and pin
   notes; change sorting; and retain the same selected note across each update.
3. Search title, body, tags, and attachment names with immediate cancellation,
   deterministic ranking, highlighted matches, and distinct empty/no-match/
   indexing/unavailable states.
4. Add a portal-selected image, validate and copy it into the managed library,
   preview it safely, export it with the note, and remove unreferenced data only
   after the note transaction is durable.
5. Import supported Markdown/plain-text notes and a documented rmac bundle;
   review collisions and unsupported content; export one note, a folder, or the
   whole library without mutating the authoritative store.
6. Interrupt autosave, exhaust disk space, corrupt an index, externally change
   a managed record, or terminate the process mid-transaction. Reopen without
   losing the last durable revision or the newest recoverable draft.
7. Move a note to Notes Trash, restore it with its metadata and attachments,
   then permanently erase it only through a separate reviewed action.
8. Complete create, edit, search, organize, attach, export, delete, restore, and
   recovery journeys with keyboard only and with Orca on the supported Linux
   stack.

## Platform authorities

- The versioned Notes library lives beneath the user's XDG data directory. It
  is app-owned state, not a loose collection of user-edited Markdown files.
  The root path must be absolute, private where supported, and resolved without
  falling back to the process working directory.
- Opening the writable library acquires one private, canonical, kernel-backed
  nonblocking writer lease. A second store instance or process opens no writer;
  the persistent lock file itself has no authority, remains after shutdown,
  and cannot be used to infer a stale owner. Descriptor close or process death
  releases the lease. Recovery, migration, and every mutation require the same
  live lease.
- XDG FileChooser is the only Linux authority for importing attachments or
  notes and selecting export destinations. Cancellation is a normal result.
  Returned paths and file descriptors are revalidated at the storage boundary.
- `rmac-storage` owns adjacent temporary files, atomic replacement, file and
  parent-directory sync, permission preservation, and cleanup. A Notes domain
  transaction coordinates record, manifest, attachment, recovery, and index
  updates; individual successful writes must not be presented as a committed
  multi-file mutation.
- The library manifest and per-note records are authoritative. The search index,
  thumbnails, snippets, relative date labels, folder counts, and sort sections
  are derived, bounded, disposable caches that can be rebuilt.
- The filesystem remains authoritative for durable bytes. Native watchers are
  hints only; complete bounded rereads and exact retained revisions decide
  whether a managed record changed or disappeared.
- No network or account authority exists in 1.0. The UI must never suggest
  cloud sync, sharing, collaboration, cross-device availability, or remote
  conflict resolution.

## Versioned library contract

- Every note has an opaque stable identifier independent of its title, folder,
  path, or ordering. Records contain a schema version, monotonic revision,
  title, body, normalized tags, folder identifier, pin/order metadata,
  attachment references, creation/edit timestamps, and deletion state.
- Folder and attachment identities are likewise stable. Names are display data;
  renaming a folder never rewrites absolute paths in an unrelated preferences
  file. Unknown future fields are either preserved or rejected according to an
  explicit migration rule, never silently discarded.
- Opening a library runs bounded, idempotent migrations from the last supported
  schema and retains a last-known-good manifest until the new schema is synced.
  Unsupported newer schemas open read-only with export/recovery guidance.
- A mutation computes and validates its complete candidate before writing. The
  commit protocol leaves either the previous durable revision or the complete
  next revision recoverable after interruption; startup deterministically
  finishes or rolls back an incomplete journal entry.
- Autosave is edit-triggered and debounced, never a permanent polling loop. A
  generation token prevents an older queued save from replacing a newer edit.
  The UI distinguishes editing, saving, saved, recovery-only, conflict, and
  failed states.
- Note text, metadata, paths, attachment names, and search terms are private.
  Errors and diagnostics identify the operation and resolution without logging
  content, absolute paths, raw storage bytes, portal details, or search queries.

## Recovery and conflicts

- Each open dirty note owns a bounded, versioned recovery record containing the
  base durable revision and newest text/metadata candidate. Recovery writes are
  atomic, private (`0600` on Unix), debounced, aggregate-bounded, and removed
  only after the matching library transaction is durably committed or the user
  explicitly discards it.
- Startup discovers all records off the first frame, quarantines malformed or
  oversized data, and presents note identity, edit time, and safe Restore/
  Discard choices without exposing content in logs. One corrupt record cannot
  hide other recoverable notes.
- Before replacing a durable note, the transaction rereads its complete bounded
  record and compares the exact retained revision. A changed, missing, or
  unreadable record keeps the local draft visible and offers reviewed choices:
  reload, preserve as a new note, or explicitly overwrite after a second exact
  preflight. Retry never bypasses this check.
- Low disk, sync failure, readback mismatch, journal corruption, and cleanup
  failure remain distinct. A failed cleanup never makes an uncommitted note
  look saved, and attachment/source files are never deleted to manufacture
  free space.

## Attachments, import, export, and deletion

- Imported files are bounded before allocation, recognized by content rather
  than extension alone, decoded with reviewed libraries, and copied to a fresh
  managed identity. Initial 1.0 support is limited to explicitly documented
  safe image formats; unsupported or malformed content remains unexecuted.
- Attachment references cannot escape the library root through absolute paths,
  `..`, symlinks, aliases, Markdown URLs, or crafted bundle metadata. Preview
  rendering is inert and bounded. Removing a reference and collecting an orphan
  are separate durable steps.
- Plain-text/Markdown import is lossless for supported text encodings and
  reports unsupported constructs. Bundle import validates schema, counts,
  sizes, identifiers, paths, and hashes before changing the library; collisions
  are resolved by stable identity and explicit policy rather than overwrite.
- Export uses a staging directory or file, exact readback, then portal-selected
  atomic placement where supported. A bundle contains a documented manifest,
  UTF-8 note content, and referenced attachments only; it contains no absolute
  paths, recovery data, search index, diagnostics, or undeclared executable
  content.
- Delete moves the note to an app-owned trash transactionally. Restore recovers
  its last folder where possible and otherwise explains the fallback. Empty
  Trash is destructive, never the default action, names the scope, and does not
  claim secure erasure on general filesystems.

## Search and organization

- Search work runs off the UI thread, is generation-cancelled, bounded by result
  and work limits, and ranks exact title, title prefix, tag, body, then
  attachment-name matches deterministically. Stable identifiers—not list
  indices or paths—carry selection through refresh and sorting.
- The index stores only the minimum local data required for search, is private,
  versioned, and rebuilt from authoritative records after mismatch or damage.
  Indexing status is truthful; stale results cannot replace a newer query.
- Tags are normalized with documented Unicode/case/whitespace rules while
  preserving user-visible spelling. Duplicate folder names, invalid control
  characters, excessive names/tags, and reserved internal names fail before
  any write. Counts are derived from the same accepted library snapshot shown
  in the list.

## Keyboard and accessibility

- `Super/Command-N` creates a note; `Shift-Super/Command-N` creates a folder;
  `Super/Command-F` focuses search; `Super/Command-S` flushes the current note;
  `Super/Command-Shift-E` exports; and Delete/Backspace actions require the same
  reviewed policy as pointer activation. Escape closes the topmost menu, sheet,
  rename, search, or recovery surface without discarding edits.
- Tab order follows sidebar, note list/search, editor title/body/tags, format
  controls, attachment list, and status. Arrow-key list navigation preserves
  focus and saves or blocks safely before switching notes.
- Folders, notes, pin state, tags, checklists, attachments, search matches,
  selected state, edited/saving/saved state, recovery, conflicts, errors, and
  dialogs require supported roles, names, states, actions, and announcements.
  Color or position is never the only carrier of selection, pin, failure, or
  deletion state.
- IME composition, grapheme movement, selection, clipboard, undo/redo, focus
  restoration, increased contrast, reduced motion, and 200% text scaling are
  release gates rather than follow-up polish.

## Visual states

The three-column window uses shared rmac chrome, semantic tokens, controls,
focus rings, alerts, menus, and live appearance settings. It should feel quiet
and content-first: restrained materials, clear selection, compact metadata,
generous editor space, and no ornamental imitation of Apple assets.

Reviewed references cover empty library, folder selection, pinned/unpinned
sections, long localized content, tags, checklist editing/preview, attachment
loading/failure, active search/no matches/indexing, rename, import/export,
autosave, recovery, conflict, Notes Trash, storage failure, light/dark,
increased contrast, reduced motion, and 100/125/150/200% scales.

## Current implementation audit

The current application provides a useful prototype: Markdown notes under
`~/Documents/rmac-notes`, folders, tags in a trailing comment, pin/sort files,
search, preview, image copy, atomic individual writes, and visible storage
errors. These behaviors are inputs to migration, not proof of this contract.

The GPUI-free `rmac-notes-store` foundation now defines path-independent stable
folder/note/attachment identities, bounded versioned records, canonical binary
encoding, and strict cross-record validation for revisions, references,
deletion, names, tags, timestamps, attachment ownership, sizes, and hashes. It
now also provides complete validated transactions for stable-ID note/folder
creation, editing, moving, pinning, sorting, folder deletion, and Notes Trash
restore transitions. Exact per-record revisions reject stale commands, IDs and
revisions are monotonic and bounded, invalid/no-op candidates cannot reach the
storage adapter, and deleting a folder moves live notes without erasing a
trashed note's restore context. The existing prototype remains the live
authority until application integration is complete.

The separate `rmac-notes-storage` adapter now provides the metadata transaction
protocol: private primary/last-known-good/journal files, exact loaded-byte
preflight, candidate and journal revalidation, atomic replacement, exact
readback, committed-with-maintenance reporting, corrupt-primary restoration,
and deterministic interrupted-save rollback/finish. A malformed or ambiguous
journal remains preserved and blocks writes. This adapter is not yet wired to
the Notes process; attachment mutation transactions, recovery drafts, and live
application integration remain required.

The version-2 library schema now carries authoritative sort order and reads
version 1 with the documented Date Edited default. A bounded deterministic
legacy planner maps sorted prototype paths to stable IDs, preserves folder,
pin, sort, tag, timestamp, note, and recognized image relationships, and emits
exact source hashes plus warnings for stale pins, duplicate tags, unsupported
references, and unclaimed files. It refuses traversal, duplicate paths,
invalid UTF-8/timestamps/metadata, excessive input, and normalized folder
collisions. The transaction adapter now compares a fresh bounded reread with
that complete reviewed plan, create-new stages private raw copies of every
legacy note and attachment plus managed attachment identities, verifies every
byte, and publishes metadata last. A versioned private receipt retains the
original relative-path/length/hash mapping; exact staged files are reusable on
retry, conflicting files fail closed, metadata failure leaves recovery data
intact, and an existing nonempty library is never overwritten. The prototype
source is not mutated. Filesystem discovery, symlink-safe source traversal,
full image-decoder validation, and Notes process integration remain. A bounded
legacy discovery adapter now requires an absolute non-symlink source, sorts
every entry, opens regular files no-follow, caps entries and aggregate bytes,
preserves empty folders and nested unclaimed files, normalizes only inside-root
legacy absolute pins, and rejects links, deeper trees, invalid metadata/sort/
timestamps, unreadable entries, and excessive files without partially
importing them. Its deterministic output feeds the planner directly, so the
application can scan once for review and scan again for the commit's exact-plan
comparison. Full image decoding and Linux runtime wiring remain. The real store
now canonicalizes an absolute app-owned root, makes it private, rejects
symlink/hard-link lock substitution, and holds one nonblocking advisory writer
lease across recovery, migration, and saves. Same-process duplicate stores and
independent kernel descriptors are rejected; the persistent private rendezvous
file is safely reusable after lease drop or process termination. Linux
contention/crash evidence remains an integration gate. An accepted-library
repository now exposes only readback-verified snapshots to its caller. Failed
transactions retain the complete candidate while the previous durable snapshot
stays authoritative; retry first resolves the journal, adopts a candidate that
committed before an error was reported, retries a rolled-back candidate, or
surfaces an unrelated durable change without overwrite.

Startup path authority now accepts only normalized absolute `HOME` and
`XDG_DATA_HOME` values, rejects overlapping managed/legacy roots, uses
`$XDG_DATA_HOME/rmac/notes` or the specified `$HOME/.local/share/rmac/notes`
fallback, and never falls back to the working directory. First run holds the
writer lease while it returns either a ready accepted library or a migration
review containing hashes/metadata rather than retained source bytes. Accept
performs the complete fresh scan and plan comparison before the metadata-last
commit; a changed source cannot publish. Starting empty is session-only unless
the user then commits a new library, restart never reoffers an already migrated
library, and blocking journal maintenance takes priority over a migration the
store cannot accept.

The new GPUI-free `rmac-notes-runtime` begins the event-driven application
boundary. Its bounded 500 ms edit scheduler has no idle timer or synthetic
save: a newer generation for the same stable note replaces the older complete
edit, changing notes returns the previous edit for ordered flush, and only an
exact pending generation can be cancelled. Stale generations and deadline/
configuration overflow fail without replacing pending content, and debug
output redacts titles, bodies, and tags. A dedicated repository worker now owns
startup inspection, the explicit migration-review decision, the writer lease,
accepted snapshots, transactions, and delayed edit commits off the UI thread.
Its strict review/ready/pending phases prevent mutation before migration is
resolved and prevent a second mutation from overtaking a failed durable
candidate. It executes stable-ID note/folder creation, rename, folder deletion,
move, pin, sort, trash, restore, explicit flush, retry, and explicit discard;
every accepted event carries the exact readback-verified snapshot. Same-note
edits coalesce, switching notes flushes the previous edit without dropping the
new note when the previous edit is rejected, and a failed store commit retains
the complete candidate while an unrelated durable change becomes an explicit
conflict. Fixed 64-command and 16-event queues apply visible backpressure
instead of unbounded growth, and an idle worker blocks without a timer. Event
debug output reports revisions/counts but not note content. A split endpoint
gives the UI a cloneable nonblocking command client while one background task
owns blocking event delivery; dropping that event endpoint shuts down and joins
the repository thread even if a UI client remains. The runtime session
projection consumes those events without optimistic publish, preserves note
selection by stable ID across reordered readback snapshots, normalizes deleted
folder selection, provides deterministic pinned/folder/Trash ordering, reveals
an accepted created note, and retains pending/conflict/rejection phases without
debugging note content. The live Notes view wiring and draft lifecycle remain;
the prototype's 1.5-second loop is still the running behavior until that
integration lands and is validated.

The storage layer now has the first complete draft-recovery foundation. Each
versioned record is keyed by stable note ID and retains its base note revision,
strict edit generation, update time, and complete bounded title/body/tag
candidate. Saves create an owner-only directory, atomically replace a `0600`
record, refuse final symlinks and multiply linked files on reread, and report
success only after exact byte and decoded-record readback. Startup discovery is
deterministic and bounded by scanned entries, retained records, individual
bytes, and aggregate bytes; malformed, oversized, misnamed, linked, or
identity-mismatched records are isolated without allowing one record to hide
the others, while excess valid records are preserved for explicit attention.
Errors and debug output contain operation/kind/count/identity information but
not draft text, tags, or library paths. The repository worker now writes the
complete draft at the same debounced boundary before attempting the library
transaction. Verified acceptance removes only its matching record; a store
failure or unrelated durable change retains both the candidate and draft;
explicit pending discard must remove the draft first; save and cleanup failures
remain visible in typed events. Startup prunes a record already identical to
the durable note, and classifies remaining records as directly applicable,
conflicting, or orphaned using stable identity and exact revisions. The UI
receives only bounded summaries until it explicitly requests Restore, while
Discard removes and verifies the exact stable-ID record. The session projection
keeps review/restored state until accepted cleanup or explicit discard. Live
recovery dialogs and editor restoration remain, so draft recovery is not yet a
complete application claim.

Known live-prototype gaps include path-based identity and pins, synchronous
scans/reads on the UI thread, a permanent 1.5-second save loop, no versioned
manifest/journal or aggregate bounds, no exact conflict preflight/readback, no
recovery records, silent scan/decode failures, permanent file/folder deletion,
attachment copies outside a note transaction, no orphan policy, no safe import/
export/bundle format, no derived cancellable index, and no Linux accessibility/runtime
evidence. Migration must preserve every readable existing note and attachment;
it must not delete the prototype library after a partial import.

## Acceptance evidence

G2 remains unchecked until focused domain/storage tests and the Ubuntu/niri
reference PC prove every journey above. Fixtures cover supported migrations,
newer/corrupt schemas, transaction interruption at every write/sync/rename,
exact conflicts, recovery ordering and bounds, malformed/oversized imports,
attachment traversal/symlink/hash failures, collisions, low disk, read-only
storage, index corruption/rebuild, delete/restore/empty-trash, portal
cancellation/restart, and private-error redaction. Runtime evidence covers
keyboard-only operation, IME/clipboard/undo, Orca/AT-SPI semantics, visual
states at every scale/theme, launch/search/save latency, idle CPU/wakeups, and
an eight-hour edit/search/attachment soak without memory or data growth.
