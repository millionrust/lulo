# Notes application specification

This file describes the behavior currently implemented by `rmac-notes`. It is
not a claim of iCloud, Apple Notes database, or cross-device compatibility.

## Core journeys

- Browse the virtual All Notes collection or a real local folder, search the
  loaded note list, select a note, and edit its title, body, and tags.
- Create, rename, and delete local folders; folder deletion requires explicit
  confirmation.
- Create and delete notes, pin notes, and sort by edited time, created time, or
  title.
- Insert Markdown-oriented formatting, switch to the rendered preview, and
  attach a user-selected local file.
- Surface storage failures without presenting an unsuccessful mutation as
  complete.

## Platform authorities and persistence

- Notes are Markdown files under `~/Documents/rmac-notes`; subdirectories are
  folders. The filesystem is the authority, not a cloud service.
- Tags use the app's trailing Markdown comment representation. Pin and sort
  preferences use the local `.pinned` and `.sort` files.
- The editor saves after a bounded debounce through the typed `storage` layer.
  Attachments are copies selected through the Linux file portal path used by
  the application.
- There is currently no external-change reconciliation, version history,
  transactional multi-file store, import/export contract, or cloud sync.

## Failure states

- Directory creation, reads, writes, copies, renames, and deletes report typed
  storage failures in the visible error banner.
- A failed save leaves the failure visible and must not be represented as a
  successful save. A failed folder delete or rename leaves the in-memory view
  recoverable through a reload.
- Missing metadata files fall back to unpinned notes and edited-time sorting
  while exposing non-absence read failures.

## Keyboard map

- `Cmd-N`: new note.
- `Shift-Cmd-N`: new folder.
- `Cmd-Backspace`: delete the selected note.
- `Shift-Cmd-P`: toggle preview.
- Text fields retain their normal editing, selection, and focus behavior;
  context-menu commands dispatch the same application actions as pointer use.

## Visual and interaction states

- Three-column layout: folders, filtered note list, and selected-note editor.
- Selected, hovered, pinned, preview, empty-selection, confirmation, context
  menu, and storage-error states are visually distinct.
- Notes uses live rmac light/dark, contrast, motion, accent, and application text
  preferences. Folder/list labels, metadata, status, tags, empty state, and
  banners scale from 100% through 130%.
- Note title/body editing and Markdown preview typography are content fonts and
  remain independent from application text scaling.

## Remaining hardening gates

Transactional storage, crash recovery evidence, external conflict handling,
import/export, attachment lifecycle, complete keyboard traversal, accessible
roles/names/states/actions, Orca announcements, and Linux scale/contrast visual
evidence remain required by roadmap item G2.
