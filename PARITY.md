# rmac — head-to-head parity roadmap

Goal: bring **every** app to real functional parity with its macOS counterpart,
the way Finder was done. Process per app: **Codex audit → write `crates/<app>/SPEC.md`
→ build the gaps → screenshot/behavior-verify → commit each step.**

Status legend: ✅ parity-ish · 🟡 partial · ⬜ mockup

## App status
- **Finder** ✅ — two audit→build passes done (multi-select, file ops, shortcuts,
  context menus, tabs, columns, Quick Look, DnD, tags, Get Info). Residual: native
  pasteboard / drag-OUT to Finder (needs objc), recursive Spotlight search.
- **Terminal** 🟡 — real PTY, colors, resize ✅. Gaps below.
- **Notes** 🟡 — create/edit/save/search ✅. Gaps below.
- **Activity Monitor** 🟡 — live read-only table. Gaps below.
- **Text Editor** 🟡 — plain text only. Gaps below.
- **App Drawer** 🟡 — scan/icons/search/launch ✅. Gaps below.
- **System Settings** ⬜ — navigation + search work; panes are static. Gaps below.

## Execution order (highest impact-per-effort first)

### 1. Terminal → work-usable  (biggest functional gap in the most-used app)
- [ ] Scrollback viewport (render grid `display_offset`, scroll wheel → scroll history)
- [ ] Mouse text selection (drag cell range, render highlight)
- [ ] Copy selection → clipboard; Paste clipboard → PTY (⌘C/⌘V)
- [ ] Tabs / new window; bold/underline/italic cell rendering; option/meta + function keys
- [ ] Profiles (font, colors, cursor, bell)

### 2. Activity Monitor → tool, not viewer
- [ ] Process selection + Quit / Force Quit (signal via sysinfo/kill)
- [ ] Search/filter; configurable update interval; PID column sortable
- [ ] Tabbed panes: CPU / Memory / Energy / Disk / Network (sysinfo data)
- [ ] Per-pane bottom graphs (sparkline/history)

### 3. Notes → real notes
- [ ] Real folders (create/rename/delete) + per-note folder; tags
- [ ] Rich blocks: checklists, headings, bullet lists (and a format bar)
- [ ] Attachments (image/PDF references) in the note model + editor

### 4. Text Editor → TextEdit
- [ ] Find/replace UI + operations
- [ ] Dirty-state tracking, safe new/close prompts, autosave
- [ ] Rich-text mode + minimal style spans; RTF/HTML import/export

### 5. App Drawer → Launchpad/App Library
- [ ] Keyboard navigation (arrows / Return / Esc)
- [ ] Grid/list view toggle
- [ ] Category metadata + category filter

### 6. System Settings → real controls  (largest; mostly mockup today)
- [ ] Interactive controls per pane (toggles/sliders) with persisted state
- [ ] Detail subpage navigation + back/forward history
- [ ] Wire safe panes to real macOS-readable state (Appearance, Sound, etc.)

## Notes
- Each "→" milestone is roughly a focused session. Verify the *look and behavior*
  with screenshots / interaction, never just a clean compile.
- Reuse the infra already built for Finder: gpui actions + keybindings, context
  menus (gpui-component PopupMenu), custom SVG asset pipeline, `rmac_ui::mac` tokens.
