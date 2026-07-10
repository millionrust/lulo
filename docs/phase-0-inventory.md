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

Replacement: `rmac-apps` using desktop entries, icon theme lookup, XDG cache,
and safe desktop-entry launch expansion.

### Finder

- `mdfind` for search, tags, and recents;
- `qlmanage` for Quick Look;
- `diskutil` for eject;
- `ditto` and `sips` for preview/thumbnail work;
- macOS pasteboard FFI for file copy/paste;
- macOS-shaped `stat` and `df` parsing.

Replacement: asynchronous `FileOperations`, a search-provider interface,
freedesktop MIME/default-app integration, portal support, mount service, and
Linux thumbnail/preview providers.

### System Settings

- `system_profiler` for Bluetooth, displays, and audio;
- `networksetup`, `route`, `ipconfig`, and `scutil` for network state;
- `pmset` and `ioreg` for battery and hardware details;
- `sw_vers`, macOS `sysctl` keys, and `diskutil` for system/storage details.

Replacement: NetworkManager, BlueZ, UPower, PipeWire/WirePlumber, standard
system information, and explicit capability detection.

### Text Editor

- RTF parsing is implemented through AppKit and intentionally unavailable on
  other platforms.

Decision required: keep RTF as a macOS development feature through 1.0 or adopt
a tested cross-platform read-only parser. Rich-text editing remains out of scope.

### Persistence paths

Activity Monitor and Terminal use `~/Library/Application Support`; Notes uses
`~/Documents/rmac-notes`. System Settings already contains an XDG config-path
branch.

Replacement: `rmac-storage` and XDG base directories, with migration/import
support where useful.

## ignored data-changing errors

The prototype deliberately discards several filesystem results. Highest-risk
areas:

- Finder trash, permanent delete, rename, move fallback, and clipboard copy;
- Notes folder rename/delete and pin/sort persistence;
- Text Editor recovery-file writes/removal;
- System Settings persistence;
- Activity Monitor column preferences;
- Terminal profile preferences.

Migration rule: destructive Finder operations are fixed first and gain typed
results, progress, cancellation, and fault-injection tests. Preference writes
move to atomic `rmac-storage` operations. No data-changing Linux path may add a
new ignored error.

## polling and redraw inventory

- App Drawer requests a redraw every 120 ms even though input observation is
  already installed. Remove this during the `rmac-apps` port.
- Terminal requests a redraw every 33 ms to observe PTY changes. Replace this
  with PTY/model change notifications before performance acceptance.
- Activity Monitor refreshes on a two-second metric interval; this interval is
  domain work and remains appropriate, but rendering should occur only after a
  completed refresh.

## missing foundations

- no Linux application catalog abstraction;
- no portal client crate;
- no shared atomic/versioned persistence;
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
