# rmac — head-to-head parity roadmap

Goal: bring **every** app to real functional parity with its macOS counterpart,
the way Finder was done. Process per app: **Codex audit → write `crates/<app>/SPEC.md`
→ build the gaps → screenshot/behavior-verify → commit each step.**

Status legend: ✅ parity-ish · 🟡 partial · ⬜ mockup

The executable component gallery is now the Phase B visual contract: 16 planned
shared controls, 80 state specimens, deterministic 100/150/200% logical
previews, and documented keyboard journeys. Native Linux scaling and Orca
evidence remains gated by the reference-PC framework work.

## App status
- **Finder** ✅ — multi-select, file ops, shortcuts, context menus, tabs, columns,
  Quick Look, DnD, **tag-search sidebar** (live `mdfind`), **real Recents**
  (Spotlight last-used query), Get Info, recursive Spotlight search, native
  NSPasteboard file copy/paste, and a **status bar** (item count + free space).
  (Toolbar Share/Tag/⋯ are decorative; the ⋯ opens the item context menu.)
- **Terminal** 🟡 — real PTY, true-color, bounded dynamic resize and up to
  10,000-line scrollback per tab under a tested 512 MiB aggregate base-grid
  ceiling, mouse selection, bounded copy/paste, find, font zoom, clear, tabs,
  **9 color profiles**, and a right-click menu. Child exit is event-driven and
  truthful; exited input is refused, and stable-ID tab/window close now reviews
  a kernel-reported foreground process group before hangup. Bracketed
  paste is mode-correct and termination-safe; unprotected multiline paste has a
  content-redacted exact-session review. Traditional xterm arrows, Home/End,
  editing keys, F1–F20, modifiers, Meta/control input, and application-cursor
  mode are encoded and tested; incomplete enhanced keyboard modes are not
  partially advertised. IME, enhanced Kitty/keypad/mouse input, hyperlinks/
  shell integration, per-tab UI state, bounds for dynamic terminal extras,
  accessibility, Linux interaction/visual evidence, and measured resident/
  idle/active performance keep G3 partial.
- **System Settings** 🟡 — navigation + search, subpage history, real Linux
  **Network / VPN / Sound / Battery / Displays** services, and validated,
  atomically persisted niri controls for **Keyboard / Mouse / Trackpad**. The
  **Appearance** pane writes the recoverable rmac theme authority instead of
  local demo state, and all seven apps consume its live light/dark/accent/
  contrast/motion tokens. The Storage subpage reports real filesystem usage.
  General hides unimplemented Apple-only continuity, warranty, password,
  startup-disk, and backup controls instead of persisting demo state. Assistant
  and Screen Time stay out of navigation, while required unfinished panes show
  an explicit unavailable state. Sound exposes only system-backed controls and
  hides local-only alert/startup/UI-effect state. System Settings no longer
  reads or writes a private settings file for service-owned state. About now
  has typed privacy-safe Linux facts, interactive-polkit hostname mutation,
  authoritative refresh, and a redacted clipboard report. Software Update now
  reports a bounded live PackageKit update list with security/blocked status,
  refresh timeout, and truthful unavailable/error states. Storage directly
  measures system/removable/network volumes, isolates per-volume failures, and
  warns conservatively when space is low. Date & Time now exposes timedated
  clock/timezone/NTP state plus validated polkit-backed timezone and automatic
  time changes with live service refresh; installation, cleanup actions, manual
  clock editing, and Linux runtime evidence keep overall parity partial.
- **Notes** 🟡 — the prototype provides create/edit/save/search, folders, tags,
  Markdown preview, image attachments, word/character counts, and persisted
  pins/sorting. G2 hardening remains: versioned transactional identity/store,
  migration, recovery/conflicts, safe attachment lifecycle, import/export,
  cancellable indexing, app trash/restore, and Linux accessibility/runtime
  evidence. The authoritative destination is `crates/notes/SPEC.md`.
- **Activity Monitor** ✅ — live table, process selection + Quit/Force Quit,
  search, five tabbed panes with sparklines, sortable columns, a **column
  chooser** (five extra real columns, persisted), **per-core CPU bars**, a
  double-click **process inspector**, and a real **per-interface Network table**
  (interface name, cumulative + per-interval rates, busiest-first).
- **Text Editor** ✅ — find/replace, dirty-state + safe prompts, autosave, a
  **formatted RTF viewer** (NSAttributedString → styled runs), and a **status
  bar** (live cursor Ln/Col + word/char counts).
- **App Drawer** 🟡 — localized XDG discovery, `TryExec`, inherited icons,
  search, shell-free launch, keyboard navigation, grid/list and category views,
  live refresh, metadata/action search, explicit empty states, and context
  actions (Open / declared desktop actions / Show in Folder). Strict-focus and
  Orca interaction evidence on Linux, plus measured cold-cache behavior,
  remain.

## Shared component library (`rmac-ui`)
Every app now shares one macOS-fidelity component layer instead of per-app
modals, third-party menus, or the framework's native prompt:
- **Alert / dialog** — `rmac_ui::alert` (scrim + card + primary/normal/destructive
  pill buttons). Replaced GPUI's `window.prompt()` in the Text Editor (recover /
  unsaved-changes guard / save error) and Notes (folder delete).
- **ContextMenu** — our own right-click popover (dispatches GPUI actions itself,
  dismisses on click-away via a `DismissMenu` action). Replaced gpui-component's
  `PopupMenu` in Terminal, Notes, Finder, and App Drawer.
- **Traffic lights** — the OS lights are hidden off-screen; `rmac_ui::title_bar` /
  `toolbar` draw our own red/yellow/green controls (glyph-on-hover) wired to
  `remove_window` / `minimize_window` / `zoom_window`. Adopted by all 7 apps.

## Framework-blocked (GPUI 0.2.2 limitations — not faked)
These need capabilities GPUI doesn't expose; documented honestly rather than mocked:
- **Native drag-OUT to other apps** (Finder drag-to-Finder, App Drawer drag-to-Dock):
  GPUI's `NSView` only `registerForDraggedTypes` (receives drops) — there's no
  `NSDraggingSource`/`beginDraggingSession`. App Drawer ships *Reveal in Finder*
  as the honest bridge; Finder ships native pasteboard copy/paste.
- **Rich-text *editing* (RTF authoring)**: GPUI has no editable rich-text widget
  and can't embed a native `NSTextView`. The Text Editor ships a read-only
  formatted RTF *viewer* instead. (Per-run font *size* also isn't carried by
  GPUI's `TextRun`.)
- **GPU / cache per-process metrics** (Activity Monitor): require Metal/IOKit not
  linked; no fabricated columns were added. (macOS Activity Monitor also has no
  GPU/Cache *tab* — its five tabs already exist here.)

## Layout/paint gotchas learned (GPUI)
- **Absolute overlays must be the LAST child** — GPUI paints children in order,
  so an absolute panel added before an opaque sibling is hidden behind it (this
  silently broke the Terminal profile picker until moved to render last).
- **Flex children default to content-size min-height** — a scroll area in a
  flex column won't shrink (pushing later siblings off-screen) without
  `min_h(0)` on it and every flex ancestor; fixed the Finder/Notes bottom bars.
- Title-bar buttons (gpui-component `Button`) DO receive clicks; raw `div`s in an
  `absolute` container inside the `TitleBar` don't — use a `Button` in flex flow.

## Notes
- Verify *look and behavior* with screenshots / real synthetic clicks (CGEvent),
  never just a clean compile. Native interop (pasteboard, RTF) is covered by
  round-trip unit tests against the live system APIs.
- Shared infra: gpui actions + keybindings, our own `rmac_ui::ContextMenu`
  + `rmac_ui::alert`, custom SVG asset pipeline, `rmac_ui::mac` tokens, and `objc2`
  AppKit interop (macOS-gated) for the pasteboard and RTF readers.
