# rmac — head-to-head parity roadmap

Goal: bring **every** app to real functional parity with its macOS counterpart,
the way Finder was done. Process per app: **Codex audit → write `crates/<app>/SPEC.md`
→ build the gaps → screenshot/behavior-verify → commit each step.**

Status legend: ✅ parity-ish · 🟡 partial · ⬜ mockup

## App status
- **Finder** ✅ — multi-select, file ops, shortcuts, context menus, tabs, columns,
  Quick Look, DnD, tags, Get Info, recursive Spotlight search, native
  NSPasteboard file copy/paste, and a **status bar** (item count + free space).
- **Terminal** ✅ — real PTY, true-color, resize, scrollback, mouse selection,
  copy/paste, find, font-zoom, clear, tabs, **9 color profiles** (picker via
  chip / ⌘⇧P / right-click), and a **right-click menu**.
- **System Settings** ✅ — navigation + search, interactive persisted controls,
  subpage history, live **Battery / Displays / Network** panes, a **Storage**
  subpage (real `df`), and a real **audio device list** in Sound (output/input
  device names via `system_profiler`, system default marked).
- **Notes** ✅ — create/edit/save/search, folders, tags, markdown preview,
  image/PDF attachments, a word/character-count footer, and **pinned notes**
  (Pinned/Notes sections, persisted).
- **Activity Monitor** ✅ — live table, process selection + Quit/Force Quit,
  search, five tabbed panes with sparklines, sortable columns, a **column
  chooser** (five extra real columns, persisted), **per-core CPU bars**, a
  double-click **process inspector**, and a real **per-interface Network table**
  (interface name, cumulative + per-interval rates, busiest-first).
- **Text Editor** ✅ — find/replace, dirty-state + safe prompts, autosave, a
  **formatted RTF viewer** (NSAttributedString → styled runs), and a **status
  bar** (live cursor Ln/Col + word/char counts).
- **App Drawer** ✅ — scan/icons/search/launch, keyboard nav, grid/list toggle,
  category filter, and a right-click menu (Open / Reveal in Finder).

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
- Shared infra: gpui actions + keybindings, context menus (gpui-component
  PopupMenu), custom SVG asset pipeline, `rmac_ui::mac` tokens, and `objc2`
  AppKit interop (macOS-gated) for the pasteboard and RTF readers.
