# Phase 0 inventory

> Captured: 2026-07-10  
> Purpose: starting evidence for the Linux migration; not a defect list

## Baseline

| Item | Baseline |
|---|---|
| Rust | 1.94.1, now pinned |
| Rust source | approximately 12,500 lines after formatting |
| Workspace packages | 9: 7 binaries and 2 libraries |
| Unit tests | 3 |
| Doc tests | 2 ignored examples |
| Formatting | passing after Phase 0 cleanup |
| Strict Clippy | passing after Phase 0 cleanup |
| Workspace tests | passing on macOS arm64 |
| Runtime validation | macOS prototype only; Ubuntu 26.04 pending Phase 1 |
| Performance baseline | first-frame startup, idle CPU, and RSS captured on macOS arm64 |

The test count is the largest immediate quality gap. Passing tests currently
prove compilation and three narrow native/parser behaviors, not application
journeys.

## Large modules to split during vertical ports

Approximate post-format sizes:

| Module | Lines | Migration boundary |
|---|---:|---|
| System Settings `main.rs` | 2,576 | domain, service adapters, persistence, panes |
| Finder `main.rs` | 2,378 | file operations, navigation model, previews, render |
| Activity Monitor `main.rs` | 1,729 | metrics, process actions, histories, render |
| Notes `main.rs` | 1,624 | store, search/index, document model, render |
| Terminal `main.rs` | 1,247 | PTY/session, terminal model, selection, render |
| Text Editor `main.rs` | 981 | document/store, actions, render |
| App Drawer `main.rs` | 905 | app catalog, icon resolver, state, render |

These are not split in one refactor. Each is split when its vertical slice gains
the corresponding service interface and tests.

## macOS-only or macOS-shaped paths

### App Drawer

- scans `/Applications` and `/System/Applications` for `.app` bundles;
- reads bundle metadata with `defaults` and `PlistBuddy`;
- converts `.icns` with `sips`;
- reveals apps with `open -R`;
- stores cache under `~/Library/Caches`.

`rmac-apps` now provides the cross-platform catalog boundary. On Linux it scans
XDG application directories with user override/Hidden semantics, filters
desktop visibility and `TryExec`, expands `Exec` field codes without a shell,
and resolves icons through the active GTK, KDE, or GNOME theme. Theme lookup
honors XDG base-directory precedence, recursive inheritance, `hicolor` and
unthemed fallbacks, and fixed/scalable/threshold size metadata. Native catalog
watches refresh App Drawer after debounced install, removal, and desktop-entry
changes while preserving its active search, category, and selection. App reveal
uses `rmac-portal` OpenDirectory on Linux, with an `xdg-open` fallback, while
macOS retains Finder reveal.

### Finder

- `mdfind` for search, tags, and recents;
- `qlmanage` for Quick Look;
- `diskutil` for eject;
- `ditto` and `sips` for preview/thumbnail work;
- macOS pasteboard FFI for file copy/paste;
- macOS-shaped `stat` and `df` parsing.

Replacement: freedesktop MIME/default-app integration and a Linux preview
provider.

`rmac-search` now owns the search-provider boundary. Finder keeps Spotlight for
macOS, while Linux recursive filename search walks without following symlinked
directories, supports cancellation and result limits, and reads Recents from
the XDG `recently-used.xbel` bookmark store. Finder rejects stale background
results after navigation or a newer request and hides macOS tags on Linux.

`rmac-thumbnails` now owns image thumbnail generation. Linux decodes supported
images in-process and writes PNG thumbnails atomically under the XDG cache;
macOS retains `sips` for HEIC support. Cache keys include source size and
modification time, and Finder drops stale or off-directory mappings on reload.

`rmac-mounts` now owns Finder's Locations volumes and eject operations. Linux
parses `/proc/self/mountinfo`, decodes kernel path escapes, exposes only
user-facing media/GVFS mount points, and unmounts through `gio`; macOS retains
`/Volumes` and `diskutil`. Helper failures are reported in Finder instead of
being discarded, and root labels are platform-appropriate.

### System Settings

- `system_profiler` for Bluetooth, displays, and audio;
- `networksetup`, `route`, `ipconfig`, and `scutil` for network state;
- `pmset` and `ioreg` for battery and hardware details;
- `sw_vers`, macOS `sysctl` keys, and `diskutil` for system/storage details.

Replacement: NetworkManager, BlueZ, UPower, PipeWire/WirePlumber, standard
system information, and explicit capability detection. Since 2026-07-10 the
prototype gathers these read-only facts in a background snapshot after the
first frame; the Linux adapters must preserve that non-blocking boundary and
replace the all-at-once snapshot with service-specific updates where useful.

`rmac-network` now starts that migration for Wi-Fi. Linux reads radio/device and
access-point state from NetworkManager D-Bus, requests scans, and changes the
real `WirelessEnabled` property; macOS retains `networksetup`. System Settings
keeps all calls off the UI thread and surfaces service failures. The complete
pane-to-service contract is tracked in `docs/system-settings-audit.md`.

`rmac-bluetooth` backs the Bluetooth pane through BlueZ's D-Bus ObjectManager
on Linux. Adapter power/discoverability, bounded discovery, and known-device
connect/disconnect are real asynchronous operations with independent errors;
macOS keeps its read-only `system_profiler` development adapter. New-device
pairing remains gated on a confirmation-capable BlueZ agent.

### Text Editor

- RTF parsing is implemented through AppKit and intentionally unavailable on
  other platforms.

Decision required: keep RTF as a macOS development feature through 1.0 or adopt
a tested cross-platform read-only parser. Rich-text editing remains out of scope.

### Persistence paths

Activity Monitor, Terminal, and System Settings now use XDG configuration paths
on Linux and `~/Library/Application Support` on macOS. Notes intentionally uses
`~/Documents/rmac-notes`. Text Editor recovery uses XDG state storage on Linux
and Application Support on macOS.

`rmac-storage` supplies the shared atomic filesystem primitives and Text Editor
imports its former temporary-directory recovery file on launch.

## ignored data-changing errors

The prototype deliberately discards one remaining data-changing result:

- Finder native pasteboard writes (the AppKit API does not expose a useful
  per-item result).

Since 2026-07-10 Finder create/copy/rename/delete/trash and move paths report
typed failures in the UI. Cross-device fallback occurs only for `EXDEV`; copy
or source-removal failures retain recoverable paths, and injected faults cover
permission, copy, delete, and source-removal failures. Progress and
cancellation run through a background transfer worker: the UI reports completed
items, cancellation terminates `ditto` or the chunked portable fallback, and
source removal is skipped once cancellation is observed. Byte-level progress
and guided cleanup for partial cancelled destinations remain future work.

Notes note bodies, pin state, and sort state now use adjacent-temp atomic writes
with typed failures. Failed autosaves remain dirty and retryable; navigation and
window close stop when pending edits cannot be saved. Folder rename/delete,
note create/delete, and attachment copy failures remain visible, and attachment
copies cannot overwrite an existing destination.

Text Editor document and recovery writes now use adjacent-temp atomic
replacement with typed failures. Recovery timers are generation-checked and
write only while the buffer is dirty; clean transitions invalidate pending
timers. Recovery removal failures remain visible and block New/Close or an
explicit discard, while cancelling an Open dialog keeps the current draft
recoverable. Recovery state now lives under XDG state/Application Support; the
former temporary file is migrated atomically. Migration or cleanup failures
retain the legacy path, and destructive actions attempt both copies before
continuing.

System Settings now loads and saves its XDG/macOS configuration through typed,
adjacent-temp atomic storage. Missing configuration remains a normal first
launch; unreadable or malformed existing state is reported in the UI, and
persisted indices and slider values are bounded before rendering.

Activity Monitor column preferences now use XDG/macOS paths and typed,
adjacent-temp atomic storage owned by the root view. Missing preferences keep
the default columns; unreadable, empty, duplicated, or unknown column data is
reported, while the required Process Name column and canonical display order
are restored before the table is built.

Terminal profile preferences now use XDG/macOS paths and typed, adjacent-temp
atomic storage. Stable profile names replace fragile array indices; legacy
numeric preferences are imported and migrated on launch. Missing preferences
select the default profile, while unreadable, unknown, or out-of-range values
are reported in a non-disruptive overlay that does not alter PTY geometry.

The five persistence consumers above now share `rmac-storage` for adjacent-temp
replacement, directory durability, no-clobber copies, partial-copy cleanup, and
injectable filesystem faults. App-local adapters retain their domain operation
labels and parsing rules; the duplicated filesystem implementations and tests
were removed.

Migration rule: preference writes move to atomic `rmac-storage` operations. No
data-changing Linux path may add a new ignored error.

## polling and redraw inventory

- Finder's former 600 ms filesystem-event polling bridge was replaced on
  2026-07-10 with a bounded, event-triggered watcher channel. Event bursts are
  debounced into one reload and the UI task sleeps until the watched directory
  actually changes.
- App Drawer's former 120 ms redraw timer was removed on 2026-07-10. Search
  input and icon extraction completion now request the redraws they need. Its
  `rmac-apps` catalog watcher now sleeps between native filesystem events and
  coalesces event bursts into one background rescan.
- Terminal's former 33 ms redraw timer was replaced with a bounded PTY/model
  wake channel on 2026-07-10. Bursts coalesce and the UI task sleeps when idle;
  Linux confirmation remains part of the reference-PC performance pass.
- Activity Monitor refreshes on a two-second metric interval; this interval is
  domain work and remains appropriate. Since 2026-07-10 the collector requests
  CPU usage without static frequency data, loads user metadata once, and clones
  only the bounded visible process rows. Rendering still occurs after each
  completed sample.

## missing foundations

- no shared versioned-format and migration registry;
- no Linux system-service layer;
- no compositor event model or niri client;
- no UI semantic/accessibility test harness;
- no journey/integration tests;
- no release packaging or application metadata.

These map directly to Phases 1–3 and the first 20 pull requests in
`PLAN_V2.md`.

## Phase 0 completion status

Completed in the initial implementation pass:

- pinned Rust, rustfmt, and Clippy;
- repository-wide formatting;
- strict Clippy cleanup;
- Ubuntu/macOS CI definition;
- enforced advisory, dependency-source, and license policy;
- build/run and contribution documentation;
- architecture and platform-debt inventory;
- complete macOS workspace test run;
- repeatable seven-application performance harness and macOS baseline report.

Still requires external evidence:

- first successful Ubuntu CI run;
- wakeup, interactive frame-time, GPU, and energy measurement;
- Ubuntu reference-hardware performance baseline;
- Ubuntu 26.04 runtime/GPU/IME/accessibility validation in Phase 1.
