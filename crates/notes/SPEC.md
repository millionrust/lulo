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
- A single permanent delete retains the exact trashed-note revision reviewed by
  the user. Empty Trash retains the exact accepted library revision and removes
  only the notes present in that review; it never sweeps in a concurrently
  trashed note. Both operations run in an otherwise empty transaction and
  return the stable note IDs, every attachment record owned by those notes
  (including an already-unreferenced tombstone), and the checked byte total.
  Managed bytes are collected only after that exact metadata revision is
  durable. The runtime may report completion only when the accepted commit
  separately proves that managed-attachment cleanup is no longer pending; it
  never claims secure erasure on a general filesystem.

## Search and organization

- Search work runs off the UI thread, is generation-cancelled, bounded by result
  and work limits, and ranks exact title, title prefix, tag, title-contains,
  body, then attachment-name matches deterministically. Stable identifiers—not
  list indices or paths—carry selection through refresh and sorting.
- The index stores only the minimum local data required for search, is private,
  versioned, and rebuilt from authoritative records after mismatch or damage.
  Indexing status is truthful; stale query generations and results from an
  older accepted library revision cannot replace current results.
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

The domain layer now additionally produces bounded permanent-delete plans.
Deleting one note requires its exact trashed revision; Empty Trash requires the
exact reviewed library revision; and both refuse to share a transaction with
any other mutation. The resulting next-revision candidate removes the complete
note and every attachment record it owns while preserving monotonic next-ID
sequences. Its private-safe plan contains only stable note/attachment IDs and a
checked byte total for post-commit collection. This is deliberately not a
runtime action yet; the repository commit path described below must be used so
metadata acceptance and physical cleanup remain distinguishable.

The separate `rmac-notes-storage` adapter now provides the metadata transaction
protocol: private primary/last-known-good/journal files, exact loaded-byte
preflight, candidate and journal revalidation, atomic replacement, exact
readback, committed-with-maintenance reporting, corrupt-primary restoration,
and deterministic interrupted-save rollback/finish. A malformed or ambiguous
journal remains preserved and blocks writes. This adapter is not yet wired to
the Notes process; attachment mutation transactions and live application
integration remain required.

Permanent-delete storage now uses a separate bounded version-1 private intent
written and exact-readback verified before the ordinary metadata journal. The
storage boundary re-derives the scope from the accepted base and rejects any
candidate that is not canonically the exact next revision with only the
reviewed notes and owned attachment records removed. The intent retains base
and candidate revisions and canonical hashes plus stable note IDs and exact
attachment ID/length/SHA-256 identities; debug and errors expose none of the
names, paths, content, or hashes. Attachment verification streams in 64 KiB
chunks through owner/single-link/no-follow storage and never allocates the
maximum 256 MiB file in full. Bytes are durably unlinked only after primary,
last-known-good, and journal maintenance prove the candidate authoritative.
Startup removes a rolled-back intent without touching attachments, resumes an
accepted or proven-descendant cleanup idempotently, preserves changed or linked
files, and blocks later writes on malformed, ambiguous, or incomplete cleanup.
Repository retry retains the purge plan, and `AcceptedCommit` reports purge
cleanup independently from general maintenance.

The repository worker now exposes separate one-note permanent-delete and Empty
Trash requests only through `commit_purge`. It retains the exact note or
library revision in the action, reports stable counts and checked bytes as
`PermanentDeleteAccepted`/`EmptyTrashAccepted` rather than claiming completion,
and carries `purge_cleanup_pending` on the accepted commit. Startup notices or
an accepted purge with unfinished cleanup project to a distinct Maintenance
session phase, so the view cannot present ordinary Ready state or permit later
writes silently. Pending metadata commits keep the purge plan through Retry,
and stale/live-note/empty/mixed requests remain typed rejections. The GPUI
confirmation sheets, destructive-action wiring, and Linux failure evidence are
still absent, so the running prototype does not yet expose these operations.

The domain layer now also admits an image attachment only through an exclusive
revision-checked import transaction. It validates the live note, display name,
nonempty bounded byte identity, monotonic modification time, per-note/global
limits, and fresh stable attachment ID before returning a complete candidate.
Its private-safe import plan binds the exact base/candidate library and note
revisions, attachment identity, recognized kind, byte length, and SHA-256; the
plan can replay the ordinary mutation and prove that no unrelated metadata
change entered the candidate. Raw bytes and source paths never enter the domain
model, and names and hashes are redacted from debug output. The storage boundary
now completely reads at most 64 MiB, recognizes content rather than extension,
and fully decodes PNG, JPEG, or WebP with 16,384-axis, 40-million-pixel, and
160-MiB decoder-allocation limits. It retains a path-free prepared value,
canonicalizes the display extension, and binds its exact length/SHA-256 to a
private versioned import intent before create-new staging under the stable ID.
Metadata publishes last. Startup removes only an exact staged orphan when the
base remains authoritative, keeps and verifies exact bytes when the candidate
was accepted, and preserves changed, linked, missing-after-acceptance,
malformed, or ambiguous state as blocking maintenance. The accepted-library
repository now retains both the exact plan and path-free prepared bytes when a
commit fails; Retry first resolves the
journal/import intent, adopts an already accepted candidate or safely restages
a rolled-back candidate, and never exposes the candidate's private content in
debug output. The repository worker accepts a stable-note/revision/timestamp
image action whose source path is redacted, performs prepare/decode and commit
off GPUI, reports unsupported/malformed/oversized input as typed storage
rejections, and publishes only an accepted attachment ID, dimensions, byte
length, and snapshot. Import maintenance is projected separately from purge
cleanup. Managed preview now rereads only the stable-ID path through the
owner/single-link/no-follow boundary, requires the authoritative length,
SHA-256, and content kind, reuses the reviewed full-decode limits, and emits a
non-upscaled RGBA thumbnail under 4,096-axis/16-million-pixel output limits.
A separate four-command/two-event preview worker keeps decode off both GPUI and
the repository writer; generation, accepted-library revision, and attachment
identity gate Started/Ready/Unavailable projection, a newer request cancels the
old one, and diagnostics expose no names, paths, hashes, or pixels. Portal
dispatch, the live Notes view, and preview rendering/accessibility still remain.

Removing an attachment reference is now a separate ordinary metadata
transaction: it requires the exact live-note and attachment revisions, verifies
ownership and monotonic modification time, removes the stable ID from the note,
increments both records, and retains the attachment as a deleted orphan
tombstone with its byte identity intact. The worker result is explicitly
`AttachmentReferenceRemoved`; tests prove the managed file remains unchanged,
and search/preview omit the tombstone. No path claims physical deletion. A
second exclusive `OrphanCollectionPlan` requires the exact tombstone revision
and proves the candidate is only the next library revision with that one record
removed. Storage writes and verifies a distinct private version-1 intent before
metadata publication, waits for primary/last-known-good/journal authority, then
streams an owner/single-link/no-follow length-and-SHA verification before a
durable unlink. Rollback never touches bytes; accepted and proven-descendant
recovery is idempotent; changed, linked, malformed, ambiguous, or simultaneous
cleanup state remains preserved and blocks writes. Repository Retry retains the
plan, the worker reports `OrphanCollectionAccepted` rather than completed, and
`orphan_collection_pending` projects separately into Maintenance. The GPUI
review/confirmation surface remains absent. Permanent note purge continues to
include all owned live and tombstoned attachments.

The first ordinary-file note import path is now strict and path-free. Storage
reads at most twice the 4 MiB decoded-body limit plus BOM allowance; accepts
UTF-8, UTF-8 BOM, and BOM-marked UTF-16 LE/BE without lossy replacement;
preserves Unicode and original line-ending sequences; rejects UTF-32, malformed
UTF-8/UTF-16, NUL, and decoded overflow; and derives a bounded safe title from
the source filename without retaining that path. Prepared debug output redacts
title and body. A redacted worker action binds the requested creation time and
stable folder, creates the complete candidate off GPUI, reports a typed source
failure, and reveals only a durably accepted stable note with encoding and
source-length summary. Portal dispatch and Markdown-construct review remain.

Export now begins from a path-free, immutable plan derived from one exact
accepted library revision. A single-note plan binds the exact note revision; a
folder plan binds the exact live folder revision and its current live notes;
and a whole-library plan binds the exact library revision. Every plan contains
sorted stable note IDs, only referenced nondeleted attachment identities, and
checked Markdown/attachment byte totals. Revalidation immediately before I/O
rejects stale or modified plans. Human-readable Markdown export is deliberately
limited to one note without attachments so it cannot silently discard managed
bytes. It emits deterministic UTF-8 with a versioned metadata header, stable
identity, timestamps, pin/trash state, escaped tags, title, and exact body.

The version-1 rmac Notes bundle is one bounded streaming file. All integers are
little-endian. It starts with the eight-byte `RMNBNDL\0` magic, a `u16` bundle
version, and a `u64` accepted library revision. The canonical filtered library
manifest follows as `u64 length + 32-byte SHA-256 + bytes`, then a `u64` note
count. Each note entry is `u64 stable ID + u64 Markdown length + 32-byte
SHA-256 + deterministic UTF-8 Markdown`. A `u64` attachment count follows;
each attachment entry is `u64 stable ID + u64 length + 32-byte SHA-256 + exact
bytes`. The manifest retains the required folder and identity-sequence context
but excludes unrelated notes and unreferenced attachment tombstones. The file
contains no source/destination paths, drafts, recovery state, search data,
previews, diagnostics, or executable entries.

`rmac-notes-storage` caps the complete bundle at 16 GiB, streams managed files
in 64 KiB chunks through owner/single-link/no-follow validation, and checks
each complete length and SHA-256 against the accepted manifest. It writes only
to a create-new adjacent temporary file, caps and fingerprints that candidate,
rechecks the exact reviewed destination immediately before atomic replacement,
preserves an existing destination's permissions, syncs the file and parent,
and requires exact final readback. A changed attachment, stale library,
maintenance state, changed destination, or oversized output removes the
temporary candidate without replacement. A sync or readback failure after the
atomic rename is reported without claiming export success; the selected target
may already contain the complete candidate and is never silently rolled back
over a concurrent writer. Export never mutates the accepted Notes library. The
repository worker flushes a pending edit first, performs planning and all export
I/O off GPUI, redacts selected paths and hashes, and emits only a typed outcome
with revisions, counts, byte totals, and output fingerprint. XDG FileChooser
dispatch, live export review/progress UI, and Linux interaction/accessibility
evidence remain.

Bundle import now consumes that exact version-1 format through a separate
two-step review/accept boundary. Preparation requires an absolute canonical
portal-selected regular file outside the managed library, opens the final entry
without following it, caps the complete bundle at 16 GiB, and fingerprints every
byte while parsing. It requires the exact magic/version/revision, hashed
canonical manifest re-encoding, sorted note and attachment identities, exact
deterministic Markdown records, matching attachment lengths and hashes, and no
trailing data. Each PNG/JPEG/WebP attachment is allocated and fully decoded one
at a time under the existing 64 MiB compressed, axis, pixel, and decoded-memory
bounds; unsupported, malformed, tombstoned, or inconsistent attachment state is
rejected before a review is offered. Prepared paths and fingerprints are
redacted from diagnostics.

The review binds the exact accepted and source revisions and exposes only folder,
note, attachment, collision, and byte counts. The initial explicit policy is
`KeepBoth`: it never overwrites a destination record or reuses purged identity
history, remaps identities that could collide, preserves unused future source
identities when safe, retains destination sort order, and deterministically
suffixes colliding live folder names. Acceptance fully re-derives the complete
base-to-candidate mapping and rejects a stale accepted library or changed source.

Storage persists a private versioned intent containing exact base/candidate and
source fingerprints before staging attachments. It streams each reviewed source
range into a fresh private managed identity, reuses only an exact prior stage,
checks the complete source again before and after staging, and publishes the
single complete metadata candidate last. Startup with the exact base removes
only exact staged orphans; startup with the exact candidate verifies and keeps
them; substituted, linked, missing-after-acceptance, malformed, simultaneous, or
otherwise ambiguous state is preserved as blocking bundle-import maintenance.
The repository retains the path-private prepared source and exact plan through
Retry. The worker flushes pending edits, retains one review by request identity,
does parsing/planning/I/O off GPUI, rejects mismatched review acceptance without
consuming the valid review, and emits only safe counts plus the accepted
snapshot. The session projects the review and bundle maintenance distinctly.
XDG FileChooser dispatch and the live review/progress/collision UI remain.

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
source is not mutated. A bounded legacy discovery adapter now requires an
absolute non-symlink source, sorts
every entry, opens regular files no-follow, caps entries and aggregate bytes,
preserves empty folders and nested unclaimed files, normalizes only inside-root
legacy absolute pins, and rejects links, deeper trees, invalid metadata/sort/
timestamps, unreadable entries, and excessive files without partially
importing them. Its deterministic output feeds the planner directly, so the
application can scan once for review and scan again for the commit's exact-plan
comparison. Every referenced PNG/JPEG/WebP candidate must now pass the same
complete bounded decode as a new import during both review and fresh-reread
planning; signature-only malformed data and GIF remain unchanged in raw
recovery, keep their Markdown text, and produce an unsupported-reference
warning instead of authoritative attachment metadata. Linux runtime wiring
remains. The real store
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
debugging note content. The live Notes view wiring remains; the prototype's
1.5-second loop is still the running behavior until that integration lands and
is validated.

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

The GPUI-free runtime now also provides a disposable version-1 search index
built from an exact accepted library revision. It indexes only live notes and
their live attachment names; lowercases Unicode without changing the original
display text; returns stable note/attachment identities and original-byte
highlight spans; and deterministically ranks exact title, title prefix, tag,
title-contains, body, then attachment name with modified time and stable ID as
ties. Queries are limited to 1 KiB, results to 500, matches per result to 16,
and normalized index/search work to 128 MiB. Index building and searching check
cancellation at bounded intervals. The session has distinct empty, indexing,
results, no-match, and unavailable states, retains a selected stable ID when it
survives, and rejects both late generations and batches from a different
library revision. Debug output contains counts, revisions, IDs, and states but
not queries, note text, tags, or attachment names. The index is intentionally
not persisted and must be rebuilt after accepted snapshot changes. The live
Notes view must still dispatch work and render these states.

A dedicated search worker now keeps that work outside both GPUI and the
single-writer repository thread. Each job carries one exact accepted snapshot,
query generation, library revision, and shared cancellation token. The worker
reuses an index only for the same revision, otherwise performs a cancellable
rebuild before searching, and publishes typed started/result/failure events.
The session projection accepts those events only while the exact generation and
revision remain pending. Fixed 8-command and 16-event channels expose
backpressure instead of growing, cancelled queued jobs publish nothing, and
dropping the event endpoint cancels active work and deterministically joins the
thread even while command clients remain. The live view still needs to submit
the current accepted snapshot on each query/revision change and render the
projected states; no running application behavior has changed yet.

Known live-prototype gaps include path-based identity and pins, synchronous
scans/reads on the UI thread, a permanent 1.5-second save loop, no versioned
manifest/journal or aggregate bounds, no exact conflict preflight/readback, no
recovery records, silent scan/decode failures, no live permanent file/folder
deletion confirmation/action UI,
attachment copies outside a note transaction, no live portal import/export or
bundle-import review UI, no consumption of the derived cancellable index, and no
Linux accessibility/runtime evidence. Migration must preserve every readable
existing note and attachment; it must not delete the prototype library after a
partial import.

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
