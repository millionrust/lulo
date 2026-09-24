# Keyboard-only operation audit (journey 8)

A code-level audit of `todo.md` journey 8, "Complete journeys 1-7 with the
keyboard only", against the shell surfaces and apps those journeys touch. This
is about a sighted person using a physical keyboard with no mouse — a
different bar than journey 9 (Orca + AT-SPI at 200%). Several apps have real,
live-confirmed AT-SPI gaps documented in `docs/journey-suite.md`
(`run-journey-terminal.py`, `run-journey-notes.py`, `run-journey-files.py`);
those are **not** repeated as journey-8 failures here unless the same control
is also unreachable from a real keyboard, independent of AT-SPI. This
document complements, and does not replace, `docs/accessibility-audit.md`
(shared-component roles/focus) and `docs/known-limitations.md` (the ⌃F2 gap).

Status key: **Works** (does what macOS does from the keyboard alone),
**Fixed** (this pass changed it), **Partial** (works but with a caveat —
usually "every control is a Tab stop but there's no roving arrow-key
selection," the same shape of gap `accessibility-audit.md` already tracks for
`List`/`ListRow`), **Missing** (no keyboard path exists at all), **Broken**
(a keyboard path exists but doesn't do the right thing), **S** (needs a live
run on the laptop to confirm — reasoned from source, not observed).

Files marked "excluded" below are other agents' territory this pass
(`crates/finder`, `crates/terminal`, `crates/notes`, `crates/text-editor`,
`shell/bins/rmac-wallpaper`, `shell/bins/rmac-menubar` +
`crates/rmac-app-menu`) — those rows are read-only findings, not fixes.

## Shell

| Surface | Task | Status | Evidence |
|---|---|---|---|
| Spotlight (`crates/launcher-app`) | ⌘Space opens it | Works | `crates/rmac-shortcuts/src/model.rs:19` registers `"launcher"` at `LOGO+space`/`Mod+Space` with the desktop-portal `GlobalShortcuts` session; live-confirmed reachable in `docs/journey-suite.md`'s journey-1 script |
| Spotlight | Type to filter | Works | Physical typing goes through GPUI's normal text-input path; only *injected* (AT-SPI) typing is blocked (`docs/known-limitations.md`'s `EditableText` gap, journey 9 concern) |
| Spotlight | Arrow keys move selection | Works | `crates/launcher-app/src/view/render.rs:825-834` maps `"down"`/`"up"` to `KeyCommand::ArrowDown`/`ArrowUp` |
| Spotlight | Return activates | Works | `render.rs:829-832` (`"enter"` → `KeyCommand::Return`/`AlternateReturn` with ⌘) |
| Spotlight | ⌘Y Quick Look | Works | `render.rs:797` (`"y"` under ⌘ → `quick_look_selected`) |
| Spotlight | Esc closes | Works | `render.rs:833` (`"escape"` → `KeyCommand::Escape`) |
| ⌘Tab app switcher (`shell/bins/rmac-app-switcher`) | Tab/⇧Tab cycles, arrows also cycle | Works | `shell/bins/rmac-app-switcher/src/main.rs:288-303` (`key_down`): `tab`/`shift-tab`, `left`/`right` all step; niri forwards the physical press via `packaging/rmac-session/shell.kdl:265-266` |
| ⌘Tab app switcher | Releasing ⌘ activates | Works | `main.rs:275-285` (`modifiers_changed` commits on ⌘ release) |
| ⌘Tab app switcher | Esc / ⌘. cancels | Works | `main.rs:297-298` |
| ⌘Tab app switcher | ⌘Q/⌘H act on the selected app | Works | `main.rs:300-301` |
| ⌃F3 Dock (`shell/bins/rmac-dock`, `crates/rmac-dock/src/keyboard.rs`) | Focus transfer, navigate, activate, open menu, Esc | Works (pre-existing) | `crates/rmac-dock/src/keyboard.rs` `Navigator`/`Key`, wired to niri's `Ctrl+F3` in `shell.kdl:263`; documented and scoped in `docs/known-limitations.md` |
| ⌃F2 menu bar | Move focus to the menu bar | **Missing** | Confirmed absent: no `Ctrl+F2` bind anywhere in `packaging/rmac-session/shell.kdl`'s `binds { }` block (only `Ctrl+F3` exists at line 263). `docs/known-limitations.md` already documents why it's not a small addition (needs the Dock's invisible-overlay-surface trick, a command endpoint, and a new "title highlighted, no menu open" state in `shell/bins/rmac-menubar`, which is excluded from this pass — menus agent's territory). **Not implemented here; left as the known limitation.** |
| Control Centre (`crates/quick-settings-app`) | Open from the keyboard | **Missing** | No niri bind and no `default_shortcuts()` entry for `"quick-settings"` (`crates/rmac-shortcuts/src/model.rs:19-24` only ships `launcher` and `lock`); the app does listen for `ShortcutId("quick-settings")` (`crates/quick-settings-app/src/main.rs:270`) but nothing ever sends it. Mouse-click-on-the-top-bar-icon is the only path today. Not just an unfilled `default_shortcuts()` entry: `packaging/rmac-session/shell.kdl:230-234`'s own comment says niri deliberately leaves every `Mod+<letter>` unbound so it passes through as the focused app's own Cmd+`<letter>` (Copy, New, …), so any letter chord here would shadow an app shortcut. macOS itself has no default keyboard shortcut for Control Center either (click/trackpad-only), so which non-conflicting chord to reserve is a product decision, not a mechanical one — same root cause for Notification Centre below |
| Control Centre | Tab/arrow between toggles and sliders once open | **Fixed** (`2a0ef183`: arrows/Tab move, ←→ adjust a focused slider, Return/Space activate, Esc backs out of a list). Earlier finding: | `crates/quick-settings-app/src/render.rs:189-195`'s root `capture_key_down` only handles `"escape"`; every pill/toggle/circle in `crates/quick-settings-app/src/render/{cards,controls}.rs` is built with a bare `.on_click(...)` and no `.track_focus`/`tab_index` anywhere in the crate (`grep` for both returns nothing). The brightness/volume sliders are drag-only (`render.rs:195-206`, `on_mouse_move`/`on_mouse_up`) with no arrow-key equivalent. This is a real "no pointer-only controls" violation, structurally larger than a binding fix — it needs the same kind of `div()`-rewrite-with-real-focus the shared-component audit already did for `Toggle`/`Checkbox` |
| Control Centre | Esc closes | Works | `render.rs:191-194` |
| Notification Centre (`crates/notification-center-app`) | Open from the keyboard | **Missing** | Same shape as Control Centre: `ShortcutId("notification-center")` is watched (`crates/notification-center-app/src/main.rs:246`) but `default_shortcuts()` never registers it and no niri bind exists |
| Notification Centre | Tab through notification action buttons | Partial | Buttons are real `rmac_ui::Button`s (`crates/notification-center-app/src/render/history.rs`), so each is its own Tab stop and Enter/Space activates it (same as `List`/`ListRow` in `accessibility-audit.md`) — but there's no roving arrow-key navigation between cards, only Tab-through-every-control |
| Notification Centre | Esc closes | **Fixed** (robustness) | `render.rs:238-246`'s root `div` had no `.track_focus`/`FocusHandle` at all — Esc worked only because GPUI happens to fall back to the dispatch-tree root when nothing is focused, unlike every sibling shell surface. Added a `FocusHandle`, focused on construction (`view.rs`, `view/lifecycle.rs`), matching `quick-settings-app`'s own pattern, so this no longer depends on that fallback |
| ⌘Q Quit (every app) | Works via the app menu; per-app keystroke was mostly missing | Partial → improving | The bold-name app menu's "Quit `<App>`" (`shell/bins/rmac-menubar/src/main.rs:2536-2547`, `app::quit`) works by mouse/AT-SPI click, but no app bound the literal `"cmd-q"` keystroke itself — niri deliberately passes Cmd+letter through to the focused app rather than intercepting it (`shell.kdl`'s binds comment), so each app is responsible for its own. This pass bound `cmd-q` to the existing `RequestClose` handler in System Settings and System Monitor, the two single-window apps in scope, where closing the one window *is* quitting; see the ⌘W/⌘Q table below. A real "quit the frontmost app regardless of which one" dispatcher (for apps outside this pass's scope) is still menu-bar/cross-cutting territory |
| ⌘H Hide (every app) | Works via the app menu | Works | Same synthesized app menu, `main.rs:2527` (`app::hide`); no per-app keystroke was added in this pass (out of scope — none of the three in-scope apps park windows) |
| ⌘M Minimize | Was missing everywhere | **Fixed** in System Settings and System Monitor | No `⌘M` binding existed anywhere (`grep -rn "cmd-m"` across the whole tree returned nothing) before this branch; the yellow traffic light already minimized on click through `WindowAction::Minimize`/`send_window_action` in `crates/rmac-ui/src/chrome.rs`, but both were private to that module. Added `rmac_ui::minimize_focused_window(cx)` as a public wrapper (`chrome.rs`, `lib.rs`), then a `Minimize` action + `"cmd-m"` `KeyBinding` + `.on_action` handler calling it in both System Settings (`controller.rs`, `controller/shell_render.rs`) and System Monitor (`main.rs`, `view/render.rs`), the same shape as the ⌘Q/⌘W wiring above. Every other app (Notes, Terminal, Files, Text Editor, Clock, …) still has no ⌘M — out of scope (excluded crates) or not attempted (in-scope apps not otherwise touched this pass) |
| ⌘W Close Window/Tab | Per-app, see table below | — | — |
| Lock screen (`crates/rmac-lock-provider-linux`) | Type password, Return submits, Esc cancels, switch selection | Works | Purpose-built XKB adapter, not GPUI: `src/keyboard.rs`'s `DecodedKey` (`Text`/`Backspace`/`Submit`/`Cancel`/`SelectPrevious`/`SelectNext`/`ToggleSelection`) is the single semantic action space. `src/pointer.rs:7-19`'s `PointerTarget::into_key` maps every mouse hit back onto the *same* `DecodedKey` variants, so nothing on the lock screen is pointer-only by construction |

### ⌘W and ⌘Q across the apps in scope

| App | Before this pass | After |
|---|---|---|
| Clock, Text Editor, Terminal (`CloseTab`), Preview, Weather, Player, Files (`CloseTab`) | ⌘W already bound (`rmac_ui::shortcuts::CLOSE.keystroke`); ⌘Q not bound (out of scope, unchanged) | Unchanged |
| **System Monitor** (`crates/activity-monitor`) | ⌘W **missing** — `rmac_ui::RequestClose` was already handled (`src/view/render.rs`) but no keystroke was ever bound to it; ⌘Q missing too | **Fixed**: `src/main.rs` now binds both `CLOSE.keystroke` and `"cmd-q"` to `rmac_ui::RequestClose` in `"ActivityMonitor"`'s context — a single window, so closing it is quitting |
| **System Settings** (`crates/system-settings`) | ⌘W **missing** — same shape: `RequestClose` fully handled (`src/controller/shell_render.rs`, the "close whatever sheet is open, else close the window" cascade) but unbound; ⌘Q missing too | **Fixed**: `src/controller.rs`'s `run()` now binds both `CLOSE.keystroke` and `"cmd-q"` to the same `RequestClose` handler in `"SystemSettings"`'s context, so ⌘Q still gets the sheet-aware guard rather than a bare `cx.quit()` |
| Notes (excluded) | Missing — `crates/notes/src/startup_controller.rs:15-20` binds ⌘N/⇧⌘N/⌘⌫/⌘F/⇧⌘E/⇧⌘L but never ⌘W | Documented only |
| **Setup Assistant** | Missing (no key infrastructure existed at all) | Not added — macOS's own Setup Assistant can't be closed with ⌘W/⌘Q either, and this app's close semantics already route through `Event::Close` on the native close button; see its own row below for what *was* fixed (Return + initial focus) |

## Apps

| App | Task | Status | Evidence |
|---|---|---|---|
| **System Settings** | Sidebar → content Tab order | **Fixed** (arrows) + Partial | Sidebar rows are built on the shared `ListRow`/`.on_activate` pattern (`crates/system-settings/src/controller/chrome.rs`), so each row is its own Tab stop and Return/Space activates it — Tab-through-every-row still has no roving arrow-key equivalent for the search-result list's own separate handling, but the plain (non-searching) category list previously had **no** Up/Down at all outside of an active search query. Added `Settings::move_category_selection` (`controller/navigation_state.rs`) and a `capture_key_down` on the sidebar's list container (`controller/chrome.rs`) so Up/Down move the highlighted category the same way the Mac's own sidebar does |
| System Settings | Visible focus ring | Partial (S) | Inherits the cross-cutting `Button`/`ListRow` ring caveat from `accessibility-audit.md` (gpui-component's own 1.5 px ring, not rmac's measured 3 pt one) — not independently re-checked here |
| System Settings | Dialogs/sheets trap focus | **Fixed** (rmac-ui) | Every dialog/sheet in the crate is built through `rmac_ui::dialog(...)`/`rmac_ui::alert(...)`, which now return a real `Dialog` (`crates/rmac-ui/src/components.rs`) that traps Tab/Shift-Tab for as long as it's on screen, the same `window.focus_next`/`focus_prev` + `contains_focused` technique `ContextMenu` already used — see `docs/accessibility-audit.md`'s updated "A real Tab-trap" section. No call site in this crate needed changing (`dialog()`/`alert()`'s public signatures are unchanged); `view_helpers/form.rs`'s `settings_sheet` needed its own return type loosened from `gpui::Stateful<Div>` to `impl IntoElement` since it named the old concrete type. Not independently re-verified live |
| System Settings | Dialog initial focus | **Fixed** (rmac-ui) | The same `Dialog` fix lands initial (and wrap-around) focus inside the dialog itself the frame it appears — the dialog's own container first, then its first tab stop (`dialog()`) or last/default one (`alert()`, since macOS puts the default action rightmost) once that content has actually been painted. Text-entry sheets that already called `window.focus(&field, cx)` themselves on open are unaffected (that focus is already inside the dialog, so the new self-heal is a no-op for them). Not independently re-verified live |
| System Settings | Esc / Return in dialogs | Works | Each dialog wires its own `capture_key_down` for both, e.g. `wifi/render/dialogs.rs` (`"escape"` cancels, `"enter"` submits via `submit_wifi_password`), matching the pattern in `shell_render.rs`'s root cascade — unaffected by the `Dialog` fix above, which only intercepts Tab/Shift-Tab and passes every other key through unchanged |
| System Settings | Focus lands in the new pane's content after picking a sidebar category | **Fixed** | `Settings` gained a second `content_focus: FocusHandle` (`controller/state.rs`), tracked (`tab_stop(false)`, a boundary rather than a stop of its own) on the detail column (`controller/shell_render.rs`). `select_position` (`controller/navigation_state.rs`) now moves focus there through a new `enter_content_focus`, which steps onto the pane's first real control — deliberately two frames, not one, the same reason as the `Dialog` fix above (`window.focus_next` reads the *last painted* frame's tab stops, which don't yet include a pane's content the instant it changes). Shift-Tab from that control returns to the sidebar for free, since the sidebar precedes the detail column in tab order. Up/Down browsing the sidebar's own highlight is unaffected — it now calls a new `select_position_keeping_focus` (the same state change, without moving focus), so arrowing through categories still keeps focus in the list rather than jumping into each pane's content on every arrow-press. `select_position`/`select_category`/`go_back` now take `window: &mut Window`; all call sites already had it in scope in their existing closures, just unnamed. Not independently re-verified live |
| System Settings | ⌘W / ⌘Q close | **Fixed** | Both bound to the existing `RequestClose` handler; `controller.rs` |
| **System Monitor** | Table/list Tab order + arrows | Works | Uses `rmac_ui::Table` (`accessibility-audit.md`: "Pass — gpui-component's own keyboard model") |
| System Monitor | Quit/Force Quit/Confirm/Cancel/Find | Works | `src/main.rs` binds ⌘F/⌘⌫/⇧⌘⌫/Return/Esc; handled in `src/view/render.rs` |
| System Monitor | Confirmation dialog traps focus, Esc/Return | **Fixed** (Escape) + **Fixed** (real trap, rmac-ui) | **Was broken**: the process table keeps its own `FocusHandle` while the "Quit Process?" dialog is open, and the vendored `DataTable`'s own key context also binds Escape (to clear its row selection) at a *deeper* context than the view's root — GPUI resolves the deepest match first, so Escape silently cleared the table's highlight instead of dismissing the dialog. Fixed by moving focus to the view's root `FocusHandle` when `request_kill` opens the dialog (`src/view.rs`), so Escape/Return now resolve at the root's `CancelKill`/`ConfirmKill` handlers. The dialog itself (`rmac_ui::alert`) now traps Tab for real too — see the System Settings dialog rows above and `docs/accessibility-audit.md`'s "A real Tab-trap" section — no `crates/activity-monitor` change was needed for that part |
| System Monitor | ⌘W / ⌘Q close | **Fixed** | Both bound to the existing `RequestClose` handler; `src/main.rs` |
| System Monitor | Visible focus ring while Tabbing | **Fixed** | `rmac_ui::Button`/`Table` draw one when focused; the 5 metric-tab pills (Cpu/Memory/Energy/Disk/Network, `view/render/chrome.rs`) are now real tab stops too (a keyed `FocusHandle` per pill, the same `window.use_keyed_state` technique `rmac-ui`'s own `Toggle`/`Checkbox`/`Radio` use), with Left/Right/Home/End moving focus and selection together across the strip (mirroring `rmac_ui::SegmentedControl`'s own roving convention; pure index math in `tab_roving_target`, unit tested) |
| System Monitor | Column headers clickable/sortable from the keyboard and AT-SPI | **Fixed** | `process_table.rs`'s `render_th` (AT-SPI `ColumnHeader`, but Accessible/Component interfaces only per `docs/journey-suite.md`'s journey 6) is now a tab stop; Space and `AccessibleAction::Click` both call a new `activate_sort`, reusing `ProcessTableDelegate::perform_sort` — the same path the table widget's own mouse-driven header click already takes, so the real sort order stays correct regardless of trigger. `aria_label` now folds in "sorted ascending"/"sorted descending" so a screen reader always reports the true order. Deliberately no new `on_click` on the header (would double-fire, and double-toggle the sort direction, alongside the table widget's own mouse click handling — see the commit for the full reasoning); the one honest gap left is that the table widget's own sort-chevron icon, driven by a private field only its mouse-click path can set, won't flip when a header is sorted from the keyboard or AT-SPI, though the data and `aria_label` both stay correct either way |
| **Setup Assistant** (`crates/setup-assistant`) | Initial focus on open | **Fixed** | Added a `focus: FocusHandle` field, created and focused in `SetupView::new` (`src/view.rs`), matching every other first-party app; before this pass the crate had zero `track_focus`/`FocusHandle`/`KeyBinding`/`actions!` usage anywhere (confirmed by grep) |
| Setup Assistant | Return activates the default button | **Fixed** | Added a `Continue` action bound to Return in the `"SetupAssistant"` key context, calling the same `continue_pressed()` every screen's own Continue button already calls (`src/main.rs`, `src/view.rs`). This specifically unblocks the **Welcome** screen, which had no other keyboard-reachable way to advance — its round "Get Started" button is a plain, unfocusable `div` (next row), and "Skip Setup" ends the whole assistant rather than proceeding |
| Setup Assistant | Tab/Shift-Tab through every field/button | **Fixed** | Text fields, language/region rows, Wi-Fi network rows, and all bottom-bar buttons were already real Tab stops (`ListRow`/`Button`, `tab_stop` by default). The three remaining plain `div()`s now are too, each with its own keyed `FocusHandle` (`window.use_keyed_state`, the technique `rmac-ui`'s own `Toggle`/`Checkbox`/`Radio` use) and a focus ring: Welcome's round "Get Started" button (role Button, name "Get Started" — GPUI's generic focused-click mapping already turns Space/Return into a click once it's a tab stop, so no new key handling was needed), Appearance's Light/Dark/Auto cards (role RadioButton with `aria_selected`, plus Left/Up/Right/Down/Home/End roving focus+selection across the three, mirroring `rmac_ui::RadioGroup`'s own ARIA radiogroup convention; pure index math in `appearance_roving_target`, unit tested), and Account's photo-picker frames (role RadioButton, `aria_selected`, and a real accessible name — "Current picture" / "Picture N") |
| Setup Assistant | Dialogs/confirmations trap focus, Esc/Return | N/A | No dialog/alert exists anywhere in this crate; the native window-close goes straight through `window.on_window_should_close` → `Event::Close` with no confirmation prompt to audit |
| Files (`crates/finder`, **excluded**) | Arrow-key file-list navigation with a real keyboard | Works (S) | `crates/finder/src/view/list_presentation.rs:800-828` has real `on_key_down` up/down handling independent of AT-SPI; this contradicts nothing in `docs/journey-suite.md`'s Files findings, which are specifically about the AT-SPI tree exposing no row nodes (an Orca/journey-9 concern), not about whether a physical keyboard can drive the list |
| Files (excluded) | Rename, ⌘↓ open, ⌘⌫ trash, ⌘Z undo, menus | Works | `crates/finder/src/view/startup/shortcuts.rs` binds all of it: `ENTER` → `RenameItem` (line 63), `OPEN_SELECTION` (⌘↓) → `OpenItems` (line 58), `DELETE` (⌘⌫) → `MoveToTrash` (line 33), `UNDO` (⌘Z) → `UndoOperation` (line 24), plus ⌘1-4 view modes, tab management, Space for Quick Look. `docs/journey-suite.md`'s journey-2 script's rename failure is specifically about driving it over AT-SPI with no row selection reachable that way — a different failure mode than a plain keyboard user Tabbing to a row and pressing Return, which this binding supports directly. One real gap found: the places sidebar has no arrow-key roving of its own (rows are `ListRow` Tab stops, per `accessibility-audit.md`, so Tab reaches every place one at a time but there's no Up/Down within it the way the file list has it, `list_presentation.rs:800-820`). Owned by the Files a11y agent this pass |
| Text Editor (`crates/text-editor`, **excluded**) | New/Open/Save/Find/Close Window | Works | `src/view/lifecycle.rs:49-89` binds all of NEW/OPEN/SAVE/SAVE_AS/FIND/FIND_NEXT/FIND_PREVIOUS/ESCAPE/CLOSE; `src/view/render.rs:77-122` wires `track_focus`+`key_context`+`RequestClose` routing for the traffic light. No gap found; documented only since the crate is excluded (polish agent) |
| Notes (`crates/notes`, **excluded**) | New/search/trash/export/checklist shortcuts | Works | `src/startup_controller.rs:15-20` | 
| Notes (excluded) | Arrow-key note-list navigation with a real keyboard | **Missing** | `grep` across the whole crate for `on_key_down`/`capture_key_down`/arrow-key handling on the note list returns nothing outside an unrelated icon name (`toolbar.rs:212`, `IconName::ArrowUp`). This is not just the AT-SPI pruning `docs/journey-suite.md` already found — there is genuinely no up/down key path to move between notes at the source level, only per-row `on_click`. Confirms and sharpens the existing journey-suite finding; owned by the Notes a11y agent |
| Notes (excluded) | ⌘W closes | Missing | See ⌘W table above |
| Terminal (`crates/terminal`, **excluded**) | Copy/Paste/Find/Zoom/Tabs/⌘W | Works | `src/controller/lifecycle.rs:20-88` binds all of it; ⌘W → `CloseTab` | 
| Terminal (excluded) | Typing a command with a real keyboard | Works (S) | The PTY itself takes real keyboard input normally; `docs/journey-suite.md`'s "typing is impossible" finding is specifically about AT-SPI's missing `EditableText` bridge and the *absence of a keyboard injector on the reference laptop* used for that automated script — a journey-9/AT-SPI concern, not a journey-8 physical-keyboard one. Still marked S because it wasn't independently re-verified live in this pass |

## What this pass fixed

Twelve commits, each small and independently reviewable:

1. `crates/activity-monitor/src/view/render/overlays.rs` — trap Tab inside
   the process inspector dialog (was a hand-rolled scrim/card with no role
   and no `.tab_group()`; now routed through `rmac_ui::dialog`).
2. `crates/system-settings/src/controller.rs` — bind ⌘W to the existing
   `rmac_ui::RequestClose` handler in `"SystemSettings"`'s key context.
3. `crates/activity-monitor/src/main.rs` — bind ⌘W to the existing
   `rmac_ui::RequestClose` handler in `"ActivityMonitor"`'s key context.
4. `crates/notification-center-app/{view.rs,view/lifecycle.rs,render.rs}` —
   give the panel a real `FocusHandle`, focused on construction, instead of
   relying on GPUI's no-focus fallback landing on the dispatch-tree root.
5. `crates/system-settings/{controller/chrome.rs,controller/navigation_state.rs}`
   — Up/Down now move the sidebar's highlighted category outside of an
   active search, the same way the Mac's own System Settings sidebar works.
6. `crates/setup-assistant/{main.rs,view.rs}` — add a `FocusHandle` (focused
   on construction) and a Return→`Continue` binding, unblocking the Welcome
   screen's previously-total keyboard dead end.
7. `crates/system-settings/src/controller.rs` — bind ⌘Q to the same
   `RequestClose` handler as ⌘W (single window, so closing it is quitting).
8. `crates/activity-monitor/src/main.rs` — same ⌘Q binding for System
   Monitor.
9. `crates/activity-monitor/{view.rs,view/render.rs,view/render/chrome.rs}`
   — fix Escape not closing the "Quit Process?" dialog: the process table
   kept keyboard focus while the dialog was open, so Escape resolved at the
   table's own deeper key context (clearing its row selection) instead of
   the dialog's `CancelKill`. `request_kill` now moves focus to the view's
   root `FocusHandle` when it opens the dialog.
10. `crates/rmac-ui/{src/chrome.rs,src/lib.rs}` — expose
    `minimize_focused_window(cx)`, a thin public wrapper around the private
    `send_window_action(WindowAction::Minimize, ...)` the yellow traffic
    light already called, so another crate can trigger the same minimize
    from a keystroke instead of only a click.
11. `crates/system-settings/{controller.rs,controller/shell_render.rs}` — add
    a `Minimize` action, bind `"cmd-m"` to it in `"SystemSettings"`'s key
    context, and call `rmac_ui::minimize_focused_window` from it.
12. `crates/activity-monitor/{main.rs,view/render.rs}` — the same ⌘M wiring
    for System Monitor.

### A later pass: the four gaps this document listed under "Larger gaps"

Four more commits, closing every gap the section above used to list by
name:

13. `crates/rmac-ui/{src/components.rs,src/lib.rs}` — `dialog()`/`alert()`
    now return a `Dialog` that really traps Tab/Shift-Tab (the same
    `window.focus_next`/`focus_prev` + `contains_focused` technique
    `ContextMenu` already used, generalized into a shared
    `cycle_focus_within`) and claims initial focus itself. Additive: no
    existing caller's code needed to change except System Settings'
    `settings_sheet`, whose declared return type named the old concrete
    type.
14. `crates/setup-assistant/src/view.rs` — the Welcome "Get Started" circle,
    the three Appearance cards, and every photo-picker frame get a real
    keyed `FocusHandle`, a focus ring, and a role/name/selected state;
    Appearance's cards get Left/Up/Right/Down/Home/End roving
    focus+selection.
15. `crates/activity-monitor/{process_table.rs,metrics.rs,view/render.rs,
    view/render/chrome.rs}` — column headers become real tab stops with
    Space and `AccessibleAction::Click` both sorting (reusing the table's
    own `perform_sort`, deliberately without a new `on_click` — see the
    row above for why); the five metric-tab pills become tab stops with
    Left/Right/Home/End roving.
16. `crates/system-settings/{controller/state.rs,controller/
    initialization/construction.rs,controller/navigation_state.rs,
    controller/shell_render.rs,controller/chrome.rs,controller/
    view_helpers.rs,controller/input/render.rs,controller/accessibility/
    render.rs}` — a second `content_focus: FocusHandle`, and
    `select_position` moving focus there (two frames, not one, for the
    same reason item 13's dialog fix is) whenever a category is actually
    chosen rather than merely browsed with Up/Down.

## What still needs a laptop check

- ⌘Q in System Settings reuses the `RequestClose` cascade, so with a sheet
  open it closes the sheet rather than quitting. The Mac quits. Decide
  whether that is acceptable.
- ⌘M in System Settings and System Monitor: confirm the window minimizes
  to the Dock the same way the yellow traffic light does.
- Every ⌘W/⌘Q fix: confirm the keystroke actually reaches the window (niri
  layer-shell focus quirks have bitten other shortcuts in this codebase
  before) and that `RequestClose`'s cascade still does the right thing (close
  the frontmost sheet, not the window, when one is open) when triggered by a
  key instead of a mouse click on the traffic light.
- System Settings sidebar Up/Down: confirm it doesn't fight the search
  field's own Up/Down handling when a query is present, and that it doesn't
  interfere with `PopUpButton`/`Slider` controls elsewhere in the window that
  also use arrow keys.
- System Monitor: confirm Escape now closes the kill-confirmation dialog
  reliably (the bug was live-reasoned from the vendored `DataTable`'s key
  context depth, not observed with a real keyboard), and that moving focus
  to the root on dialog-open doesn't visually disturb the table's row
  highlight in a way that reads as "selection lost."
- Setup Assistant: confirm Return submits every screen as expected —
  particularly the Wi-Fi and Account screens, where `continue_pressed()`
  branches into async work (`apply_then_continue`) rather than calling
  `handle(Event::Continue, ...)` directly — and that it doesn't fire
  unexpectedly while a text field has focus and the field's own widget
  wants Enter for something else.
- Notification Centre: confirm Esc still closes it now that focus is
  explicit rather than relying on the no-focus fallback.
- ⌘M in System Settings and System Monitor: confirm the keystroke reaches
  the window and that niri actually parks/minimizes it the same way clicking
  the yellow traffic light already does (same compositor path, `WindowAction
  ::Minimize`, but not independently re-run from a keystroke here).
- Control Centre / Notification Centre: confirm they are indeed unreachable
  from any physical key today (this pass found no bind, but niri config
  outside `packaging/rmac-session/shell.kdl` was not exhaustively searched).
- `rmac_ui::Dialog`'s new Tab trap: confirm Tab/Shift-Tab actually cycle
  within an open dialog/alert/sheet across a few real ones (a Wi-Fi password
  sheet, a destructive confirmation alert, System Monitor's Quit Process
  dialog) without ever reaching the window behind the scrim, that initial
  focus lands somewhere sensible (the container the first frame, a real
  control the next), and that this didn't regress any dialog's own
  Esc/Return handling (it shouldn't — the new trap only intercepts Tab —
  but this is unverified against a real event loop).
- Setup Assistant's Get Started circle, Appearance cards, and photo-picker
  frames: confirm Tab reaches all three, Space/Return activate the Get
  Started circle and photo frames, and Left/Right/Home/End move and select
  across the Appearance cards without also scrolling the page.
- System Monitor's column headers: confirm Space and Tab both reach and
  sort a header, that the sort order is correct (ascending first pick,
  toggling thereafter), and specifically confirm a real mouse click on a
  header still sorts exactly as before (this pass deliberately avoided
  adding a second `on_click` there to prevent a double-toggle, so this is
  the one regression risk most worth re-checking by hand) — and that the
  metric-tab pills' Left/Right/Home/End roving doesn't fight the table's
  own arrow-key row navigation once focus is inside it.
- System Settings: confirm choosing a category (click, Return on a focused
  sidebar row, a search result, or an in-pane "jump to Keyboard…" link)
  actually lands visible focus on the new pane's first real control, that
  Shift-Tab from there returns to the sidebar, and that Up/Down browsing
  the sidebar's own highlight still keeps focus in the list across several
  presses in a row (the two-frame focus technique this needed has no
  cargo-verified precedent in this crate).
- Everything marked **S** above.

## Larger gaps (not fixed here, listed per the brief)

- **⌃F2 (menu bar focus)**: known limitation, needs the Dock's invisible-
  overlay-surface mechanism plus a new menu-bar state; `shell/bins/rmac-menubar`
  is excluded from this pass.
- **Control Centre keyboard operation**: needs (a) a default global shortcut
  + niri bind + activation wiring to open it at all, and (b) rewriting its
  toggles/pills/sliders off bare `on_click` onto real `track_focus`/keyboard
  handling — the same "wrap `div()` directly" pattern
  `accessibility-audit.md` used for `Toggle`/`Checkbox`/`Radio`.
- **Notification Centre roving navigation**: opening it and Tab-through-every-
  button already work; there is no arrow-key navigation between notification
  cards, only Tab-through-every-control.
- **⌘Q / ⌘H as a true "act on the frontmost app regardless of which one"
  dispatcher**: this pass only bound ⌘Q locally in the two single-window
  apps it owns (System Settings, System Monitor), where "close this window"
  and "quit this app" are the same thing. A real system-wide ⌘Q/⌘H for
  multi-window-capable apps (Player, Preview) or apps outside this pass's
  scope needs a shared action wired through every app crate (10+), which is
  a cross-cutting workstream, not a mechanical per-app fix.
- **⌘M (Minimize) beyond System Settings and System Monitor**: those two now
  bind it (see the fix list and shell table above), reusing a new
  `rmac_ui::minimize_focused_window` helper. Every other app — Notes,
  Terminal, Files, Text Editor, Clock, Calculator, Preview, Weather, Player —
  still has no ⌘M; wiring it there is the same three-line
  action/binding/`on_action` shape used for System Settings and System
  Monitor, just not done for apps outside this pass's touched set.
**Fixed in a later pass** (kept here, struck through in spirit, for the
record — see the App rows above for each one's detail): Setup Assistant's
three plain-`div` controls; System Monitor's metric-tab pills and
column-header sorting; a real Tab-trap for `rmac_ui::dialog()`/`alert()`
(built on `Dialog`'s own `window.focus_next`/`focus_prev` +
`contains_focused`, the same technique `ContextMenu` already used —
**not** gpui-component's `focus_trap::FocusTrapElement`, so this stays
clear of ADR 0015); and System Settings' sidebar → pane content focus
handoff.
