# Files fidelity and safety specification

The installed product is **Files** (`org.rmac.Files`, `rmac-files`). This
document may use another desktop as a visual reference, but installed labels,
metadata, icons, and identifiers remain original rmac work.

## File-operation invariants

- Copy, duplicate, move, rename, trash, restore, and permanent delete run away
  from the GPUI thread. A visible operation owns progress, cancellation, and a
  dismissible result.
- No operation overwrites an item merely because it appeared after a UI
  conflict check. Copy creates every destination node exclusively. Same-volume
  move and rename use an atomic no-replace primitive and fail closed where that
  primitive is unavailable.
- A directory is never copied into itself or a real descendant, including a
  descendant reached through a symlinked destination parent.
- Recursive copy uses `symlink_metadata`, recreates a symlink itself, and never
  traverses the symlink target. Dangling and relative links retain their exact
  link payload. Sockets, FIFOs, devices, and other special files are refused
  during both planning and copy rather than opened or interpreted as regular
  file content.
- Cross-volume move is copy-then-source-removal only for the kernel's
  cross-device result. Any other rename failure stops without copying. Source
  removal begins only after a durable hidden copy on the destination volume,
  journal persistence, identity revalidation, and a final cancellation check.
  After source removal, the copy is atomically published with no-replace
  semantics. A racing final name therefore leaves the complete staged copy
  recoverable and never overwrites the conflicting entry.
- Ordinary copy uses the same typed, private, versioned journal and
  destination-volume staging boundary as cross-volume move. No partial copy is
  exposed at its requested final name. After durable staging, Files revalidates
  the staged identity and original source identity, publishes with one atomic
  no-replace rename, fsyncs the destination parent, and only then removes the
  journal record. A version-1 record with no operation field remains a move for
  upgrade compatibility. Each new copy or cross-volume move persists
  deterministic SHA-256 manifests for both the source tree and completed
  destination stage over raw relative path bytes, no-follow device/inode/mode/
  size/timestamps, and exact symlink payloads. Each manifest is bounded to
  1,000,000 entries, 256 levels, and 128 MiB of path/link bytes; each directory
  identity is rechecked after its sorted children. A second destination digest
  ignores only the published tree root's ctime, which is expected to change
  during rename, while continuing to bind every nested identity. Live
  publication, recovery review acceptance, and restart recovery rescan these
  manifests, so a nested source or hidden-stage mutation invalidates the entire
  copy even when the corresponding root directory metadata did not change. An
  unchanged manifest-proven file, symlink, or directory can finish
  automatically, including a rename completed just before stage persistence. A
  changed source or stage, conflicting final name, or partial stage remains
  available for explicit recovery review rather than being guessed or
  discarded.
- An honored cancellation never removes the move source. Once a same-volume
  source rename or cross-volume source removal begins, Files completes or
  durably retains that transaction instead of claiming it was cancelled.
  Partial destinations remain visible for explicit recovery and are never
  silently deleted after a path race.
- Paste and drag/drop conflicts are preflighted away from GPUI and presented
  sequentially with explicit Keep Both, Replace, and Skip choices. Each review
  binds bounded no-follow source and existing-destination tree snapshots and
  revalidates them off-thread at acceptance. Keep Both chooses an unoccupied
  numbered name without clobbering another item; Skip leaves both reviewed
  items unchanged and retains skipped cut items on the clipboard. Enter chooses
  Keep Both, Escape chooses Skip, and duplicate activation is blocked while a
  choice is being revalidated.
- Copy and move replacement never use delete-then-copy. The reviewed snapshots
  are revalidated again while preparing a private journal. Copy replacement
  creates a durable hidden destination-volume stage. A same-volume move
  atomically renames the exact source into that private stage without duplicate
  capacity; a cross-volume move durably copies there, revalidates source/stage/
  previous destination, and only then removes the exact source. One kernel
  atomic exchange publishes the new item while retaining the previous
  destination at the hidden name. Cancellation is honored only before that
  exchange and, for a cross-volume move, before source removal. If it arrives
  after the completed-stage record is durable, Files first converts that record
  into a non-destructive retained-copy review so restart cannot resume the
  cancelled replacement. The exchanged stage is fsynced and persisted before
  the previous destination is removed without following symlinks. Restart
  recovery never removes a source that still exists, but infers a same-volume
  source rename, cross-volume source removal, or exchange interrupted before
  its stage write; it resumes identity-bound partial directory cleanup and
  keeps changed source/destination/backup state for an exact review. Batch-only
  conflicts still disable Replace because no existing destination snapshot was
  reviewed. User-facing Undo is not implemented yet.
- Every destructive or multi-step operation has a durable, versioned journal
  whose recovery distinguishes prepared, destination-complete,
  source-removed, replacement-exchanged, published, committed, and ambiguous
  states. Journal files
  preserve raw Unix path bytes, use private modes, are atomically replaced and
  fsynced, and bind no-follow device/inode/type/size/time identities. Startup
  finishes only identity-proven publication or journal cleanup; unknown,
  substituted, partial, or conflicting states remain visible and fail closed.
- Ambiguous records open a sequential recovery sheet before new transfers are
  allowed. Its review is bound to the exact journal, source, staged copy,
  destination, and proposed visible recovery name. A second validation occurs
  off GPUI at acceptance. Complete and possibly-partial copies use distinct
  wording; both are preserved with one atomic no-replace rename. When no staged
  copy exists, acceptance clears only the exact record and keeps every existing
  item. Recovery never offers deletion or replacement, persists its intent
  before publication, survives interruption, caps one scan at 512 records, and
  redacts private paths/names from `Debug`.
- The recovery sheet uses shared semantic buttons, opens automatically, moves
  sequentially through every record, supports Enter/Escape while idle, blocks
  dismissal and duplicate activation while busy, and reports completion,
  partial-copy preservation, changed review state, and name races truthfully.
- Each journal record has a private empty no-follow lock file held with the
  kernel's exclusive advisory lock from preparation through commit. Other
  Files processes use a nonblocking exclusive probe: active records and their
  atomic temporary writes are excluded from recovery/review, while unrelated
  transfers continue. Review acceptance reacquires the same lock before its
  identity checks. A lock with no record is removed only after the kernel
  proves its owner is gone; lock identity, type, mode, and size are revalidated
  before unlink. Unsupported or malformed locking fails closed.
- Linux move-to-Trash follows the freedesktop Trash specification and runs off
  GPUI through a Files-owned private transaction authority. It resolves the
  source's actual mount, uses the home Trash only on that filesystem, otherwise
  selects a valid sticky `.Trash/$UID` or private `.Trash-$UID`, and never
  copies across volumes. The `.trashinfo` file is written and fsynced before
  one atomic no-replace rename; home identities are absolute and mounted-volume
  identities are top-directory-relative, with raw path bytes percent-encoded.
  Source, data, and metadata identities plus bounded no-follow SHA-256 tree
  manifests survive prepared, metadata-published, and data-moved stages. Both
  rename parents and all private state are fsynced. The bounded UI bridge shows
  completed top-level items and supports cooperative cancellation during tree
  scanning and between items; cancellation before rename removes only the
  transaction's exact metadata and record while retaining the source. Startup
  completes only identity-proven interrupted work, retains changed state for
  manual recovery, and serializes Files processes with a private nonblocking
  kernel lock. Bounded Trash enumeration accepts only private user-owned roots
  and safe regular metadata, strictly parses one percent-decoded absolute or
  mount-relative `Path` plus a valid deletion timestamp, rejects traversal,
  NUL, malformed escapes, and excess inventory, and binds the exact data tree
  and metadata identities without exposing paths through diagnostics. Restore
  snapshots that identity into the same private journal, binds the real
  destination parent, refuses an occupied or changed destination, and uses one
  no-replace rename before removing only the exact `.trashinfo`. Prepared,
  data-restored, and info-removed stages recover every unambiguous crash
  boundary; data, metadata, parent, and destination races remain pending
  without replacement or deletion. Permanent delete begins only after a
  destructive confirmation names the item or bounded count, states that the
  action is immediate and cannot be undone, and never exposes a private Trash
  path. The accepted item snapshot is identity- and manifest-bound in a durable
  delete-prepared record. After a final cancellation and identity check, Files
  atomically renames the exact data to a journal-derived hidden sibling on the
  same filesystem, fsyncs the parent, persists the data-staged boundary, then
  removes the staged tree without following symlinks. Data-removed and
  info-removed boundaries are persisted and fsynced before the record is
  cleared. Restart recovery infers a rename completed before its stage write,
  resumes an identity-bound partially removed directory, and finishes exact
  metadata cleanup. Changed regular data, substituted metadata or staging
  paths, malformed records, and ambiguous states remain pending without
  deleting the changed entry. Cancellation is honored before destructive
  staging and between top-level items; once one staged tree begins deletion it
  is completed or retained durably for recovery.
- Trash is a virtual Files sidebar location, not a browsable implementation
  directory. Its rows use the original item name and deletion timestamp while
  retaining the bound private data identity internally. List, icon, and gallery
  presentation remain available; column view is refused because it would
  traverse ordinary filesystem parents. Double-click/Open, Copy, Cut,
  Duplicate, Rename, Quick Look, Get Info, internal drag/drop, and external
  drops do not expose or mutate private Trash paths. The item context menu
  offers Restore and the explicitly confirmed Delete Permanently action only.
  Both run through the journaled authority away from GPUI with bounded
  progress, cooperative cancellation, startup recovery, truthful result
  banners, and a generation-guarded refresh; Restore additionally guarantees
  no-clobber publication. An empty verified inventory renders a dedicated
  Trash empty state.
- Changed Trash, Restore, and permanent-delete records open a sequential
  recovery sheet before Trash actions can resume. Review captures the exact
  private journal identity plus bounded source, destination, Trash-data,
  delete-stage, and metadata snapshots; acceptance reacquires the process lock
  and recaptures every tree and metadata hash before changing anything. A
  changed or partially deleted hidden stage can be returned to Trash through a
  persisted return intent and one atomic no-replace rename. If no data remains,
  explicit review can remove only the identity-bound orphan metadata. Other
  states can keep every existing item and clear only the exact record. Wording
  distinguishes possibly incomplete data and never claims an orphan metadata
  cleanup deletes a user file. States for which Files cannot prove a safe
  automatic action expose a disabled manual-repair result and retain both
  items and journal. The sheet uses shared semantic controls, supports
  Enter/Escape while idle, blocks duplicate resolution while busy, processes
  records in stable order, and redacts all paths from `Debug`.
- Before creating any batch destination, a cancellable no-follow scan is
  bounded to 1,000,000 entries and 256 levels. It accounts for each regular
  file's logical bytes plus 4 KiB per destination entry, groups requirements by
  destination device, and preserves five percent of each volume up to a
  512 MiB reserve. Sparse files use logical size because the current copier
  materializes their holes. Same-volume moves require no duplicate capacity.
  If the mount boundary changes after preflight, a move refuses the newly
  required copy and asks for a retry.
- Visible progress distinguishes scanning, copying, and finishing. It reports
  copied bytes from accepted writes and completed top-level items. The
  background/UI bridge is bounded; intermediate snapshots may coalesce, but
  the terminal outcome is delivered with backpressure.
- Low-space preflight and mid-operation `ENOSPC` preserve the move source,
  retain truthful partial-destination recovery, and never claim completion.

Authoritative values (points). Sources: AppKit/NSColor, HIG, measured on light mode.

## Chrome
- Unified toolbar height: **52 pt**. Bg ≈ `#f6f6f6` (vibrancy), bottom hairline `#e5e5e5`.
- Traffic lights: 12 pt glyphs, 20 pt center spacing, vertically centered (center.y ≈ 26).
  → `traffic_light_position ≈ (19, 19)`, content left gutter ≈ 80 pt.
- Toolbar is **draggable** (start_window_move on drag).

## List view
- Row height **24 pt**; body text **13 pt**; selected row text → white.
- Column header **11 pt**, `secondaryLabel`; header height ~26 pt; bottom hairline.
- Disclosure indent per level **16 pt**; chevron ~10 pt, leading the icon. Icon→text gap **6 pt**.
- Alternating row stripes: `#ffffff` / `#f4f5f5`.

## Colors (light)
| token | hex |
|---|---|
| list bg | `#ffffff` |
| toolbar | `#f6f6f6` |
| sidebar | `#e9e9ed` |
| alt row | `#f4f5f5` |
| selected row (focused) | `#0063e1` (white text) |
| accent / systemBlue (folder tint) | `#007aff` |
| separator | `#e5e5e5` |
| label / secondary / tertiary | `#272727` / `#808080` / `#bfbfbf` |
| drive icon tint | `#808080` (gray) |

## Sidebar
- Sections: **Favorites**, **iCloud**, **Locations** (+ Tags). 11 pt semibold gray headers.
- Row ~28 pt, icon 18 px, text 13 pt. Selected = gray `#d8d8dc` rounded (unfocused) / accent (focused).
- Icon tint rule: folders/locations = **blue**; physical drives/hardware = **gray**.
- SF Symbol → bundled SVG: folder→folder-fill, Applications→layout-grid, Downloads→download,
  Recents→clock, Macintosh HD→hard-drive(gray), iCloud→cloud, home→house, drive→hard-drive(gray).

## Kind column
- Real Finder uses `UTType.localizedDescription`. Approximate by extension:
  Folder, Plain Text Document, PDF document, PNG image, JSON document, Markdown Document,
  Application, ZIP archive, Document (generic).

## Date
- `medium` date + `short` time + relative: "Today at 11:12 AM", "Yesterday at 1:30 PM",
  "18 Apr 2026 at 2:42 PM" (day-first, abbreviated month).

## Toolbar controls
- Left: back/forward chevrons. Title (left, 13 pt semibold) after nav.
- Right: view segmented control (grid/list[active]/columns/gallery), share, tag, more, Search field.

## Accessibility text size
- Finder-owned labels and chrome follow rmac's bounded 100%, 115%, and 130% text preference.
- Standard row and toolbar metrics stay faithful to the values above; their existing vertical room fits 130% glyphs.
- Tab-close and new-tab hit boxes are enlarged to avoid clipping their scaled symbols.
