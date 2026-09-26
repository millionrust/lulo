# Shared-component accessibility audit

This is a code-level audit of every shared interactive primitive in
`crates/rmac-ui` and `shell/crates/rmac-shell-ui`, checked against the
`todo.md` "Accessibility gates" checklist:

1. stable accessible identity, role, name, state, value, actions;
2. correct tab order and a visible focus ring;
3. full keyboard operation, no pointer-only controls;
4. announcements for asynchronous status and errors;
5. no clipping at 200% text/UI scaling;
6. usable high-contrast colours;
7. reduced motion read from the Settings portal.

It complements, and does not replace, `docs/accessibility-release-audit.md`
(the I3 evidence gate: real Orca observations on the reference laptop). A
component reading correctly in this document's terms is a precondition for
that gate, not a substitute for it — nothing here was verified with a screen
reader.

Status key: **Pass** (meets the criterion today), **Fixed** (this audit
changed it), **Partial** (meets some but not all of the criterion), **Gap**
(does not meet it; fixable in rmac-ui), **Blocked** (needs a change in
`gpui-component`/`gpui-kit`, not fixable from rmac-ui alone), **S**
(unmeasured — a real value is needed before this can be marked Pass), **N/A**
(component does not exist yet, or the criterion does not apply).

## The central finding

Nearly every shared control in `crates/rmac-ui/src/controls.rs` was built by
wrapping `gpui_component::button::Button` (`ComponentButton`) purely for its
click/hover/focus plumbing, then painting rmac's own pixel-accurate visuals
inside it. `Button`'s accessibility semantics are hardcoded in its own
`render()` (`accesskit::Role::Button`/`Role::Link`, `aria_label` only when
`.label(text)` was called, `aria_selected` from its own `selected` field) and
are **not exposed as public hooks** — there is no `.role()`, `.aria_toggled()`,
or `.aria_expanded()` a wrapper can call from another crate. `gpui_component`'s
own `checkbox.rs`, `radio.rs`, `menu/menu_item.rs`, `tab/tab.rs`, `list/list.rs`,
and `table/table.rs` all build directly on `gpui::div()` instead, and get full
role/state/name control that way.

The first pass on this branch fixed `Toggle`, `Checkbox`, and `Radio` by doing
the same: building directly on `div()` with
`role()`/`aria_toggled()`/`aria_selected()`/`aria_label()` and manual focus
handling (see "Fixes applied" below). A second pass applies the same
technique to `ContextMenu`'s items, `List`/`Tree`'s container role and
loading/error announcements, `ListRow`/`TreeRow`, and the grouped-form
`PopUpButton` trigger (see "Fixes applied (second pass)" below), and adds a
new `RadioGroup` container for roving arrow-key selection between radios. The
plain (non-form) `PopUpButton` variant is the one remaining `Gap` of this
shape — it has no current caller, and rebuilding its two-button
`DropdownButton` visual without a way to verify it on the laptop was judged a
worse risk than leaving it exactly as it was. `Tabs` and `SegmentedControl`'s
selected/unselected buttons keep wrapping `Button` because `.selected(bool)`
(which does map to `aria_selected`) is sufficient for a two-state tab
strip/segmented control; `SegmentedControl` just wasn't calling it (fixed in
the first pass).

Everything that already builds on `div()` directly —
`shell/crates/rmac-shell-ui/src/text_field.rs`'s `TextField`,
`rmac_top_bar::accessibility` and its sibling shell surfaces — was unaffected
by this and already has correct role/name/value.

## Component × criterion

| Component | Identity/role/name/state/actions | Tab order & focus ring | Keyboard operation | Async announcements | 200% scale | High contrast | Reduced motion |
|---|---|---|---|---|---|---|---|
| `Button` (`controls.rs:99`) | Pass — gpui-component sets `Role::Button`/`Role::Link`, `aria_label` from the button's own `.label()`, `aria_selected` (`gpui-component/button/button.rs:464`) | Partial — tab-stop via `track_focus`; ring is gpui-component's own 1.5 px default (`styled.rs:551`), not rmac's measured 3 pt accent ring (S; see "Focus ring width" below) | Pass — Space/Return activate via GPUI's generic focused-click mapping (`gpui/elements/div.rs:2760`) | N/A | Pass — label/icon reflow in a flex row, no fixed width | Pass — reads `mac::*` theme tokens, which strengthen under Increase Contrast (`theme.rs` `increased_contrast_strengthens_non_text_boundaries_and_focus`) | N/A (no animation) |
| `Toggle` (switch) (`controls.rs:477`) | **Fixed** — now `Role::Switch`, `aria_toggled`, `aria_label` from the label or (new) the tooltip. Previously `Role::Button` via the wrapped `Button`, no toggled state, and **no name at all** for a label-less switch | **Fixed** — `track_focus` + `mac::focus_ring_shadow()` when disabled is false and focused; ring now uses the measured 3 pt token | Pass (unchanged) — Space/Return toggle once focused, same generic mapping | N/A | Pass — fixed switch width is the measured macOS metric itself, not a clip risk | Pass | N/A |
| `Checkbox` (`controls.rs:654`) | **Fixed** — now `Role::CheckBox` + `aria_toggled` (On/Off/Mixed) + `aria_label` fallback. Previously `Role::Button`, no toggled state, name only from luck (content-derived) | **Fixed** — same as Toggle | Pass (unchanged) | N/A | Pass | Pass | N/A |
| `Radio` (`controls.rs:784`) | **Fixed** — now `Role::RadioButton` + `aria_selected` (matches `gpui-component/radio.rs:165`'s own convention) + `aria_label` fallback | **Fixed** — same as Toggle | Pass for a standalone radio; use the new `RadioGroup` (below) for a set — a bare `Radio` still only does Space/Return | N/A | Pass | Pass | N/A |
| `RadioGroup` (`controls.rs`, new) | **Fixed** (new component) — `Role::RadioGroup` with an optional `aria_label`, wrapping one `Radio` per option | **Fixed** — each radio keeps its own focus ring; arrow keys move focus to the target radio's own focus handle | **Fixed** — Up/Left/Down/Right/Home/End move to and select another option, matching the ARIA radiogroup convention. Previously there was no grouping component at all, so Tab was the only way to reach a later radio in a set and arrow keys did nothing | N/A | Pass | Pass | N/A |
| `SegmentedControl` (`controls.rs:1647`) | **Fixed** — segments now call `.selected(index == selected)`, so `aria_selected` finally reflects which segment is chosen (it was always `false` before) | Partial — same ring caveat as `Button` (S) | **Fixed** — Left/Right/Home/End now call the control's own `wrapped_selection` helper, which existed but was never wired to a key handler, so arrow keys previously did nothing | N/A | Pass | Pass | N/A |
| `PopUpButton` (`controls.rs:304`) | **Fixed for `.form()`** (the only variant with a current caller) — its trigger is rebuilt directly on `div()` as `PopUpButtonTrigger`, reporting `Role::ComboBox`, `aria_value` set to the selected option, and `aria_expanded` tracking the popover's real open state. The plain (non-form) variant is unchanged and remains **Gap**: it still wraps `Button`/`DropdownButton`, neither of which sets `Role::ComboBox`/`aria_expanded`, and has no current caller to verify a rebuild against. Either way, the **opened menu itself** is a `gpui_component::menu::PopupMenu`, which does get `Role::Menu` with `Role::MenuItem` rows and `aria_selected`/`aria_label` (`menu/popup_menu.rs:1319`, `menu/menu_item.rs:97`) — Pass | **Fixed for `.form()`** — its own keyboard focus ring; non-form variant keeps the `Button` ring caveat (S) | Partial — the opened `PopupMenu` supports arrow/Home/End/Esc internally (gpui-component); **the trigger itself only opens on mouse-down** — `gpui_component::popover::Popover::render()` wires its open/close toggle to `on_mouse_down` directly with no `on_click`, so GPUI's generic focused-Enter/Space-to-click mapping never fires for it. This is true of every `Popover`-based dropdown trigger in the app today, not something this pass introduced or could fix from rmac-ui (**Blocked**; see "What still needs gpui-kit/gpui-component work" below) | N/A | Pass | Pass | N/A |
| `ContextMenu` (`components.rs`) | **Fixed** — the panel has `Role::Menu`; items now report `Role::MenuItem` (`Role::MenuItemCheckBox` + `aria_toggled` for a checked item) with an explicit `aria_label` of just the item's label (previously content-derived, which also read out the checkmark glyph and shortcut hint), and a disabled item is now actually marked disabled instead of staying in keyboard navigation with no handler behind it | Pass — menu keeps and traps focus (`ContextMenuState`, `move_context_menu_focus`) | Pass — Up/Down/Tab navigate, Escape (`DismissMenu`) closes, type-select exists (`type_select_match`). rmac-ui's `ContextMenu` has no submenu concept, so there is no Left/Right/`aria_expanded` submenu contract to give it | N/A | Pass — width is `min_w`, content reflows | Pass | N/A |
| `alert`/`alert_with_icon`/`dialog` (`components.rs`) | **Fixed** — `dialog()` now sets `Role::Dialog`; `alert_with_icon` narrows to `Role::AlertDialog` and sets the title (or message) as `aria_label`. Previously no role at all | **Fixed** (real trap) — `dialog()`/`alert()` now return a `Dialog` that really traps Tab/Shift-Tab (see "A real Tab-trap" below, which replaces the `.tab_group()`-only description this row used to give — `.tab_group()` only ever set ordering priority, not an enforced trap; default button is rightmost per macOS convention, and initial focus now lands there too) | Pass — Escape dismisses via `DismissMenu`'s key binding context; buttons are ordinary `Button`s | N/A (an alert appearing is itself the "announcement"; `Role::AlertDialog` is the accesskit signal for that) | Pass — fixed 260 pt card width is the measured macOS metric | Pass | N/A |
| `Toast` (`feedback.rs`) | **Fixed** — now `Role::Alert` (accesskit's live-region equivalent, matching `gpui-component/alert.rs:192`) with title+message combined into one `aria_label`. Previously **no announcement mechanism of any kind** — a toast appearing was invisible to a screen reader unless focus happened to land on it | N/A (not a focus target) | N/A | **Fixed** — see identity column | Pass — `pr_20()` reserves room for the dismiss button; message wraps in a flex column | Pass | N/A |
| `EmptyState` (`feedback.rs`) | **Fixed** — the `error` variant now announces as `Role::Alert` with the title and message combined into its accessible name, the same pattern `Toast` uses. A non-error empty state stays a silent, unlabeled container | N/A | N/A | **Fixed** (see identity) | Pass | Pass | N/A |
| `Spinner` / `Progress` (`feedback.rs`) | **Fixed** — both report `Role::ProgressIndicator`; determinate `Progress` sets `aria_numeric_value`/`aria_min_numeric_value`/`aria_max_numeric_value` (0.0–1.0), indeterminate leaves them unset per the ARIA convention for a busy indicator | N/A | N/A | **Fixed** | Pass | Pass | N/A |
| `List`/`ListRow` (`controls.rs`) | **Fixed** — `ListRow` is rebuilt directly on `div()` the same way `Toggle`/`Checkbox`/`Radio` were: `Role::ListItem` with a real `aria_selected` (previously always `true` regardless of the row's actual selection state, because the shared `painted()` helper forced the wrapped `Button`'s own `selected` flag on for every row so it could paint rmac's colours through Button's "selected" style branch). `List` gained an optional `.label()` for the container's own `Role::List` + `aria_label`, and its Loading/Empty/Unavailable messages now announce as `Role::Status`, a real load failure as `Role::Alert` | **Fixed** — `ListRow`'s own keyboard focus ring | Pass — plain click/Enter/Space per row; no roving tabindex, so a long list Tabs through every row rather than arrowing between them (acceptable for a short list, S for a long one) | **Fixed** (state-message divs, see identity) | Pass | Pass | N/A |
| `Tree`/`TreeRow` (`controls.rs`) | **Fixed** — `TreeRow` now reports `Role::TreeItem` with `aria_expanded` on branch rows (omitted on leaves, per ARIA) and a 1-based `aria_level` from its depth; `Tree`'s container can take the same `.label()` as `List`, reporting `Role::Tree` | **Fixed** — inherited from `ListRow` | Pass — Left/Right collapse/expand a branch row (wired in `TreeRow::render`'s own `on_key_down`), Enter/Space activate a leaf | N/A | Pass — label truncates in its flex row | Pass | N/A |
| `Table` (`controls.rs:1574`, wraps `gpui_component::table::DataTable`) | Pass — gpui-component sets `Role::Table`/`Role::RowGroup`/`Role::Row`/`Role::ColumnHeader`/`Role::Cell` throughout (`table/table.rs`); rmac adds no styling that bypasses this | Pass | Pass (gpui-component's own keyboard model) | N/A | S — virtualized; not independently checked here | Pass | N/A |
| `Tabs` (`controls.rs:1115`) | Pass — each tab is a `Button` with `.selected(index == selected)` called correctly (unlike `SegmentedControl` before this audit), so `aria_selected` is accurate | Partial — same ring caveat as `Button` (S) | Pass — Left/Right/Home/End wired via `next_tab_index` (pre-existing, unaffected by this audit) | N/A | Pass | Pass | N/A |
| `Slider`/`SliderState` (`controls.rs:898`, wraps `gpui_component::slider::Slider`) | Pass — gpui-component sets `Role::Slider`, `aria_numeric_value`/min/max, orientation, and `on_a11y_action(Increment/Decrement)` (`slider.rs:608-627`) | S — not independently re-verified; gpui-component owns this control's focus handling entirely | Pass (gpui-component's own arrow-key model) | N/A | Pass | Pass | N/A |
| `TextField`/`SearchField` (`controls.rs:950`, `:1049`, wraps `gpui_component::input::Input`) | Pass — gpui-component sets a role from `input.rs:398`'s `accessibility_role` (varies by kind) | S — not independently re-verified | Pass | Gap — the inline `.error(message)` text has no role/live-region; a validation error appearing under a field is not announced | Pass | Pass | N/A |
| Traffic lights (`chrome.rs`) | **Fixed** — `Role::Button` + `aria_label` ("Close window"/"Minimize"/"Enter Full Screen") added; previously plain divs with **no accessible identity of any kind** | **Fixed** — now a Tab stop (skipped for the full-screen light on fixed-size windows) with the shared focus ring; previously **not reachable from the keyboard at all** | **Fixed** — see tab order; mouse-only before this audit | N/A | Pass — fixed 16 pt hit boxes are the measured AX frame size, not app content | Pass | N/A |
| `shell-ui::TextField` (`shell/crates/rmac-shell-ui/src/text_field.rs:257`) | Pass — `Role::TextInput`, `aria_label(name)`, `aria_value(text)` already set (`text_field.rs:259-261`) | **Fixed** — `track_focus` made this field a tab stop with no visible indicator of that; it now draws a `BoxShadow` ring reading live from two new `rmac_shell_ui::tokens` functions, `focus_ring()`/`focus_ring_width()` (mirroring `rmac-ui`'s `mac::focus_ring()`/`theme().focus.ring_width`), rather than widening `TextFieldStyle` (which has exactly two call sites — this file and `shell/bins/rmac-wallpaper/src/linux_wayland/desktop.rs` — and reading the shared live token avoided touching the latter at all) | Pass — full AppKit-style editing keys (`key_down`, `Motion`) | N/A | S — wraps at a fixed `wrap_width`; not independently re-verified | S | N/A |
| `Stepper` | N/A — does not exist in rmac-ui yet. When built, it should follow `Slider`'s pattern: `Role::SpinButton`, `aria_numeric_value`, `on_a11y_action(Increment/Decrement)` (see `gpui-component/input/number_input.rs:304` for the same pattern already used for numeric fields) | N/A | N/A | N/A | N/A | N/A | N/A |
| Sidebars / toolbars (`chrome.rs` `toolbar`/`toolbar_group`, `mac::sidebar`) | N/A — these are layout/background helpers, not separately interactive; their rows are `ListRow` (see above) | N/A | N/A | N/A | Pass | Pass | N/A |

## Focus ring width (cross-cutting, S)

`rmac_design::Metrics::focus_ring_width` (3.0 pt regular, 4.0 pt high-contrast)
and the matching accent-based color were already measured
(`design-lab/chrome.html`: "focus ring 3 wide (2 outside, 1 inside)") but
**nothing painted them**: `FocusTokens { ring_width, ring_offset }` existed in
`rmac-ui`'s theme with no color and no drawing code anywhere in `rmac-ui` or
`rmac-shell-ui` (grepped for `is_focused`/`focus_ring`/`FocusTokens` — the only
hits were the field definitions and shell-ui's own unrelated `is_focused`
checks). This audit adds `theme.rs`'s `ColorTokens::focus_ring` and
`mac::focus_ring_shadow()` (a solid `BoxShadow` at the measured width) and uses
it on the three rewritten controls and the traffic lights.

Controls that still wrap `gpui-component`'s `Button` (`Button` itself, the
non-form `PopUpButton`, `Tabs`, `SegmentedControl`'s segments) keep getting *a*
ring for free — `Button::render` calls a private
`FocusableExt::focus_ring(is_focused, px(0.), ..)` (`gpui-component/styled.rs`)
— but it is hardcoded to a 1.5 px border, not rmac's measured 3 pt accent ring,
and that extension trait is `pub(crate)` inside `gpui-component`, so rmac-ui
cannot override its width or color. **Blocked**: getting the exact measured
ring on every `Button`-wrapped control needs either a `gpui-component` change
(expose the ring color/width, or a way to opt out and draw rmac's own) or
converting each of those controls off `Button` the way `Toggle`/`Checkbox`/
`Radio` were converted here.

## Reduced motion (cross-cutting)

The plumbing is correct end to end: `org.freedesktop.appearance`'s
`reduced-motion` key → `rmac-appearance-portal` (`REDUCED_MOTION_KEY`) →
`rmac_appearance::Model::reduced_motion` → `rmac_design::motion::MotionTokens`
(`reduced_motion`, feeding `fast`/`standard`/`deliberate` durations and
`spatial_motion`) → `rmac-ui`'s own `theme::MotionTokens`. This is a single
gate, tested (`theme::tests::reduced_motion_disables_spatial_transitions`).

However, grepping `crates/rmac-ui` and `shell/crates/rmac-shell-ui` for
`with_animation`/`Animation::new` finds **no shared component that currently
animates anything** — the only `Duration`-based code is interaction delays
(the green-button zoom-menu hover open/close timers in `chrome.rs`, window
state debounce), not visual motion. So there is nothing live to violate today,
but there is also nothing exercising the gate. This needs re-checking the
first time a shared control (a sheet presentation, a toast enter/exit, a
disclosure triangle rotation) adds an actual animation — it must read
`crate::theme::current().motion` rather than a literal duration.

## High contrast (cross-cutting)

Pass at the token level: every color a shared component draws comes from
`mac::*` → `crate::theme::current().colors`, and `theme.rs` has passing tests
asserting WCAG-level contrast ratios for text/background pairs and that
Increase Contrast strengthens non-text boundaries and the focus ring width
(`increased_contrast_strengthens_non_text_boundaries_and_focus`). No shared
component was found hardcoding a raw color instead of a `mac::` token.

## Clipping at 200% scale (cross-cutting)

No shared component in `rmac-ui`/`rmac-shell-ui` was found with a hardcoded
pixel *content* width that would clip text at 200% — sizes that are fixed
(switch tracks, checkbox/radio boxes, traffic-light hit boxes, the alert card
width) are macOS's own measured control metrics, not text containers, and
text-bearing containers use `flex`/`max_w`/`truncate` rather than a fixed
height that would clip a taller glyph run. This was a static read-through, not
a rendered check at 200% — mark **S** for anything not called out by name
above.

## Fixes applied (first pass)

Five commits on this branch, each buildable and tested independently
(`cargo check -p rmac-ui`, `cargo test -p rmac-ui --lib` — 45/45 passing;
`cargo fmt -p rmac-ui` clean; `cargo clippy -p rmac-ui --lib` shows the same
two pre-existing warnings as `master`, both outside the files this audit
touched, and no new ones):

1. **Add a shared keyboard-focus ring color and shadow helper**
   (`theme.rs`, `mac.rs`) — `ColorTokens::focus_ring` and
   `mac::focus_ring_shadow()`.
2. **Give Toggle, Checkbox, and Radio their real accessible roles**
   (`controls.rs`) — rebuilt on `div()` with correct role/state/name/focus;
   also fixed `SegmentedControl`'s missing `aria_selected` and missing
   arrow-key navigation.
3. **Make the window traffic lights keyboard- and AT-reachable**
   (`chrome.rs`) — role, name, Tab stop, focus ring.
4. **Name dialogs and menus for assistive technology** (`components.rs`) —
   `Role::Dialog`/`Role::AlertDialog` with a computed name, `Role::Menu` on
   the context-menu panel.
5. **Announce toasts as they appear** (`feedback.rs`) — `Role::Alert` with a
   combined title+message name.

## Compile risk (first pass)

Low. All five commits are additive within `rmac-ui` (no public API removed;
`Toggle`/`Checkbox`/`Radio`'s builder methods are unchanged, only their
`render()` bodies changed the underlying element type from
`gpui_component::button::Button` to `gpui::Stateful<gpui::Div>`, and both
implement `IntoElement`, so every call site — `system-settings`, `finder`,
`setup-assistant`, `clock`, etc. — compiles unchanged since they only ever see
`impl IntoElement`). `SegmentedControl`'s public surface is unchanged. No
`Cargo.toml` changed, no new dependency.

Only `crates/rmac-ui` was rebuilt and tested here (`CARGO_TARGET_DIR` pointed
at the repository's existing `target/`, per the local agent constraints); a
full workspace build, the Linux/niri target, and `shell/crates/rmac-shell-ui`
were not rebuilt since no code in the latter changed.

## Fixes applied (second pass)

Six more commits on this branch, closing the `Gap`s the first pass called out
by name plus adding `RadioGroup`. **No `cargo` command was run for this
pass** (the constraint on this machine ruled it out); each commit was checked
with `rustfmt --edition 2021 --check` and a close manual read against the
pinned `gpui`/`gpui-component` source (fetched from the local Cargo git
checkouts) for every type/method/trait bound used, not compiled or tested.
`scripts/check-gpui-component-imports.sh` passes (50 files, none growing).

1. **Give `Spinner`, `Progress`, and `EmptyState` errors real accessible
   roles** (`feedback.rs`) — `Role::ProgressIndicator` with
   `aria_numeric_value`/min/max when determinate; `EmptyState`'s error variant
   as `Role::Alert`.
2. **Give `List`, `Tree`, `ListRow`, and `TreeRow` real accessible roles**
   (`controls.rs`) — `List`/`Tree` gain an optional `.label()` and a
   `Role::Status`/`Role::Alert` split on their state messages;
   `ListRow`/`TreeRow` are rebuilt on `div()` the way `Toggle`/`Checkbox`/
   `Radio` were, plus the new `RadioGroup` container.
3. **Give context-menu items real menu-item roles** (`components.rs`) —
   `Role::MenuItem`/`Role::MenuItemCheckBox`, a real disabled flag, and an
   explicit item name.
4. **Give the grouped-form `PopUpButton` trigger a real combo-box role**
   (`controls.rs`) — `Role::ComboBox`, `aria_value`, `aria_expanded`; see
   "Compile risk (second pass)" below for why this one carries more
   uncertainty than the others.
5. **Keep the `PopUpButton` trigger's `Selectable` impl off the
   `gpui_component` guard** (`controls.rs`) — a same-day fixup: the previous
   commit's fully-qualified `impl gpui_component::Selectable` and a doc
   comment spelling out the crate name both tripped
   `check-gpui-component-imports.sh`, which counts every line containing that
   text, comments included.
6. **Draw a visible focus ring on the shell `TextField`**
   (`shell/crates/rmac-shell-ui`) — reads `rmac_shell_ui::tokens::focus_ring()`/
   `focus_ring_width()` (new) live rather than widening `TextFieldStyle`.

## Compile risk (second pass)

Low for items 1–3 and 6: same shape as the first pass (additive fields,
`render()` body swapped from a wrapped `Button` to `div()`, public builder
signatures unchanged, verified by reading every call site in `crates/`
and `shell/` for each type touched).

**Moderate** for item 4, `PopUpButton`: this one couldn't be checked against
an existing working pattern in this codebase the way the others could, because
no shared component here previously used `gpui_component::popover::Popover`
directly. The new `PopUpButtonTrigger`/`PopUpMenuCache` code mirrors
`gpui-component`'s own `menu::dropdown_menu::DropdownMenuPopover::render`
line for line (read from the pinned git checkout) for the "build the menu
once, listen for its `DismissEvent`, rebuild after dismiss" caching, since
`Popover::content`'s own doc comment warns it is called on every render and
building a fresh `PopupMenu` entity each time would leak one per frame. Two
things specifically need laptop confirmation:
- That the mirrored caching actually behaves like the original once compiled
  (types were checked by hand against the pinned `gpui-component` source, not
  by the compiler).
- That a disabled trigger's `on_mouse_down` + `cx.stop_propagation()` still
  stops `Popover`'s own wrapping div from toggling open, the same way a
  disabled `Button`'s identical pattern already does today for every other
  disabled dropdown in the app (checked by reading `Button::render`'s
  `on_mouse_down` handler, not by running it).

The plain (non-form) `PopUpButton` variant was deliberately left as `Button`/
`DropdownButton`-wrapped and unconverted, both because it has no current
caller to break and because rebuilding its two-button visual without a way to
compare it on the laptop was judged a worse risk than leaving the documented
`Gap`.

## A real Tab-trap for `dialog()`/`alert()` (third pass)

`docs/keyboard-audit.md` found the exact shape of `.tab_group()`'s limit
this document's own "alert/dialog" row used to gloss over: it only sets
*ordering priority* among a group's own tab stops (confirmed by reading
`gpui`'s `tab_stop.rs`), not an enforced trap — Tab could still walk out of
an open dialog onto the window's other tab stops behind the scrim, and
`dialog()` itself (used by every Wi-Fi/Bluetooth/VPN/update sheet in System
Settings, and by Notes'/Finder's/Text Editor's own dialogs) never even
called `.tab_group()`; only `alert()` did.

`dialog()`/`alert()`/`alert_with_icon()` now return a new `Dialog`
(`#[derive(IntoElement)]` + `RenderOnce`, the same shape as `Button`/
`ListRow`/`Toggle`) instead of a plain `gpui::Stateful<gpui::Div>`. Inside
its own `render()` — which GPUI calls with a live `window`/`cx`, unlike the
free functions that used to build the div eagerly — it:

- gets a persistent, per-dialog `FocusHandle` via `window.use_keyed_state`
  keyed by the dialog's own id (the same pattern `Radio`/`RadioGroup`
  already use for a stable handle across renders without the caller owning
  a field for it);
- traps Tab/Shift-Tab with a new `cycle_focus_within`, which generalizes
  `ContextMenu`'s own proven `move_context_menu_focus` (`window.focus_next`/
  `focus_prev`, wrapping back in via `contains_focused` rather than letting
  focus escape) — `ContextMenu` now calls the same shared function;
- claims focus itself if it currently isn't inside the dialog, landing on
  the dialog's own container the first frame and stepping onto its first
  tab stop (`dialog()`) or last one (`alert()`, since macOS puts the
  default action rightmost — `.initial_focus_last()`) once that content has
  actually been painted at least once. This is deliberately two frames, not
  one: `window.focus_next`/`focus_prev` read the *last painted* frame's tab
  stops, which don't yet include a dialog's own content the instant it
  first appears.

`dialog()`'s own public signature is unchanged (`fn dialog(id, content) ->
Dialog`), so none of its ~13 existing callers across `crates/terminal`,
`crates/activity-monitor`, `crates/text-editor`, `crates/system-settings`,
`crates/notes`, and `crates/finder` needed to change — `Dialog` implements
`IntoElement`/`.into_any_element()` like every other `rmac-ui` control, and
gained its own `.capture_key_down(...)` builder method so the several
System Settings sheets that already chain their own Escape/Return handler
onto the result keep compiling and working unchanged (both listeners run;
this one only ever calls `cx.stop_propagation()` for Tab/Shift-Tab, so it
never swallows a caller's own Escape/Enter). The one exception was System
Settings' `view_helpers/form.rs::settings_sheet`, whose declared return
type named the old concrete `gpui::Stateful<Div>`, loosened to `impl
IntoElement`. Not independently re-verified live.

## What still needs gpui-kit/gpui-component work (Blocked)

- `gpui_component::button::Button`'s accessibility fields (role, `aria_label`,
  `aria_selected`) are set internally in its own `render()` with no public
  setters. The plain (non-form) `PopUpButton` variant is still stuck at
  `Role::Button` until either `gpui-component` exposes these, or it is
  converted off `Button` the way its `.form()` sibling was in the second
  pass.
- `gpui_component::popover::Popover`'s open/close toggle is wired to
  `on_mouse_down` directly inside its own `render()`, with no `on_click`
  registered — so GPUI's generic focused-Enter/Space-to-click mapping never
  fires for it. This affects every `Popover`-based dropdown trigger in the
  app today (the form `PopUpButton` included, both before and after the
  second pass), not something fixable from rmac-ui without either a
  `gpui-component` change or dropping `Popover` for a hand-built overlay the
  way `ContextMenu` already is.
- The same crate's `FocusableExt::focus_ring` (used internally by `Button`,
  and by extension every control still wrapping it) is `pub(crate)` and
  hardcodes a 1.5 px ring, so those controls cannot be moved onto rmac's
  measured 3 pt ring without either an upstream change or dropping `Button`.
- `gpui_component::tooltip::ManagedTooltipExt` (the ergonomic
  `.tooltip("text")` used throughout `rmac-ui`) is `pub(crate)` too; this
  audit's `Toggle` rewrite works around it by calling the public
  `gpui_component::tooltip::Tooltip::new(text).build(window, cx)` directly
  through GPUI's own `.tooltip(build_fn)`, which is the pattern any future
  `div()`-based control needing a tooltip should reuse.

These are consistent with ADR 0015's direction (drop `gpui-component`'s styled
widgets for `gpui-base` behaviour underneath rmac-owned visuals): once that
migration lands, rmac-ui's controls will own this layer directly instead of
routing around a pinned dependency's private internals.

## Follow-through into app-owned rows (not shared components)

This audit is scoped to `crates/rmac-ui`/`shell/crates/rmac-shell-ui`, but the
`div()`-rewrite pattern it proved for `Toggle`/`Checkbox`/`Radio` — and the
`ListRow`/`TreeRow` `Role::ListItem`/`Role::TreeItem` gap it called out above
as proven-but-unconverted — is the exact fix journey 3 and 4's live
acceptance tests (`docs/journey-suite.md`) found missing in two app crates
that build their own rows directly on `div()` rather than through
`rmac-ui`: `crates/terminal`'s tab strip (now `Role::Tab`/`Role::TabList`/
`Role::Menu`/`Role::MenuItem`) and `crates/notes`'s folder/note rows (now
`Role::ListItem`/`Role::List`). Neither app crate depends on the
`gpui_component::button::Button` internals this document's "Blocked" section
describes, so both were fixable without any upstream change — see
`docs/journey-suite.md`'s journey 3/4 sections for what changed and what a
live re-run still needs to confirm.

## A second `Table` gap: `gpui_component::table::{TableState, TableDelegate}`

The "Table" row above (`controls.rs:1574`) is Pass because it describes
`gpui_component`'s *declarative* `Table`/`TableRow`/`TableCell` builder
(`table/table.rs`), which does set `Role::Table`/`Role::Row`/
`Role::ColumnHeader`/`Role::Cell` on its own elements. `gpui_component` ships
a **second**, unrelated table API in the same module —
`table::{TableState, TableDelegate}`, the virtualized/data-driven table
`rmac_ui::Table<D: TableDelegate>` also wraps — and that one sets **no**
AT-SPI role anywhere in its own code (`table/state.rs`, `table/delegate.rs`,
`table/data_table.rs`: zero `role(`/`aria_` calls). Its default
`render_tr`/`render_th`/`render_header` are plain, roleless `div()`s, and the
framework renders the `TableDelegate` implementation's own returned element
directly as the row/header/cell — there is no wrapping role applied on top.
This is fixable entirely from the `TableDelegate` implementation (the trait
methods return ordinary `gpui::Div`s the implementor already owns), unlike
the `Button`-wrapping gaps above — it is a **Gap**, not **Blocked** — but
every current `TableDelegate` implementation needs its own fix; none of this
is inherited by using `rmac_ui::Table` instead of the raw types.

`crates/activity-monitor`'s `ProcessTableDelegate` (System Monitor's process
table) hit exactly this: journey 6 of `docs/journey-suite.md` found its live
AT-SPI tree had no table, row, or cell for any process at all. That crate's
own `accessibility.rs` already had a complete, unit-tested projection
(`project_process_table`) that nothing called. This was fixed in the same
session as this note — see `docs/journey-suite.md`'s "Journey 6" section for
the full list of changes (row `Role::Row`/`aria_label`/`aria_selected` plus
`AccessibleAction::Click`/`Focus`, header `Role::ColumnHeader`, the table
container's `Role::Table`, the search field's `Role::TextInput` wrap, tab
`Role::Tab`, and an outer-wrapper naming pattern for icon-only toolbar
buttons that reuses the same `Button`-can't-set-`aria_label`-without-a-
visible-label limitation documented above). Left unfixed there, and worth
folding into whichever future pass picks up the `Gap`s above: per-cell
`Role::Cell`, and the column-chooser popover's checkbox-like rows (the same
`ListRow`-shaped gap already tracked in the "List/ListRow" row of the table
above).

A later pass closed one more gap this section left standing:
`render_th`'s `Role::ColumnHeader` cells reported the Accessible/Component
AT-SPI interfaces only, with no `Action` and no keyboard path either
(`docs/journey-suite.md`'s journey 6, "Remaining real product bugs" #2).
Each header is now a real tab stop; Space and `AccessibleAction::Click`
both call a new `activate_sort`, which reuses `ProcessTableDelegate::
perform_sort` — the same path the table widget's own mouse-driven header
click already takes, so the real sort order stays correct regardless of
how a header was activated. `aria_label` folds in "sorted ascending"/
"sorted descending" so the true order is always announced. Deliberately no
new `on_click` on the header div: the table widget's own wrapper already
calls back into its private sort method on a real mouse click (bubbled up
from this div), and GPUI's generic Space/Return-activates-a-focused-click
mapping only re-fires a div's own click listeners, not an ancestor's — a
second `on_click` here would double-fire, and double-toggle the sort
direction, on every mouse click. The one honest gap this leaves: the table
widget's own sort-chevron icon is driven by a private field only its
mouse-click path can set, so it won't flip when a header is sorted from
the keyboard or AT-SPI — the data and `aria_label` both stay correct
either way. `crates/activity-monitor/src/view/render/chrome.rs`'s five
metric-tab pills (Cpu/Memory/Energy/Disk/Network), plain `div().on_click
(...)` with no way to reach them from the keyboard, were fixed the same
pass: each is a real tab stop, with Left/Right/Home/End roving focus and
selection together across the strip.

### Off-screen rows were unreachable — fixed in `rmac_ui::Table` itself

Every fix above still left one gap this document didn't call out: even a
`TableDelegate` implementation that does everything right (real
`Role::Row`/`aria_label`/`aria_selected` on each row, as
`ProcessTableDelegate` does) only produces an AccessKit node for a row while
`gpui-component`'s virtualized `TableState`/`DataTable` has it painted.
`render_tr`/`render_th` are only ever called for the visible range; a row
that has never scrolled into view has no element at all, so no screen
reader can select it. Journey 6 of `docs/journey-suite.md` found exactly
this: with ~227 real processes and ~19 painted at a time, most of System
Monitor's process list was outside AT-SPI's reach, and select → Quit
Process → confirm was impossible for a process outside the current
viewport.

Unlike the per-`TableDelegate` gaps documented above, this one could not be
fixed by any one delegate — the model rows a delegate never paints never
reach its own `render_tr` at all. It is fixed instead in the shared layer,
`rmac_ui::Table<D>::render` (`controls.rs`), so every `Table` user gets it
for free:

- The table is now wrapped in a `Role::RowGroup` element (previously
  `Table::render` returned `DataTable` directly with no role of its own).
- While an AT client is listening (`Window::is_a11y_active`, the same gate
  Terminal's own accessibility projection uses — commit `ba012c90` — so
  idle CPU stays flat), that wrapper publishes one synthetic `Role::Row`
  AccessKit node per model row outside the table's current painted range
  (`TableState::visible_range().rows()`), up to
  `rmac_ui::accessibility::MAX_ACCESSIBLE_TABLE_ROWS` (4096) — an honest,
  bounded cap rather than unbounded per-frame cost for a pathologically
  large table.
- Each synthetic row gets its 1-based row index and the table's row count
  (`node.set_row_index`/`set_row_count`, so a screen reader can announce
  "row 145 of 300"), its name from `TableDelegate::cell_text(row, 0, cx)`
  (the first column), its selected state, and — since it isn't painted —
  the table's own bounds as a stand-in position, via
  `A11ySubtreeBuilder::parent_node().bounds()`. Painted rows are
  untouched: they keep using their own real element and real bounds, so no
  row is ever described twice.
- Each synthetic row advertises the `Click` and `Focus` actions, registered
  via `Window::on_a11y_action` with a deterministic, per-table, per-row
  node id (`rmac_ui::accessibility::table_row_node_id`, salted with the
  `TableState` entity's id so two tables never collide). Either action
  calls `TableState::set_selected_row`, which both scrolls the row into
  view and selects it, and emits `TableEvent::SelectRow` — the same event
  keyboard navigation already emits — so a consumer's own selection
  bookkeeping (e.g. `ProcessTableDelegate::selected_pid`, kept in sync via
  `MonitorView`'s existing `TableEvent::SelectRow` subscription) stays
  correct regardless of how the row was selected.
- `ProcessTableDelegate::cell_text` now returns real per-cell text (it
  previously used the trait's empty-string default), and
  `ProcessTableDelegate::render_tr` now also sets `aria_row_index`/
  `aria_row_count` on painted rows, so a screen reader announces "row N of
  M" the same way whether or not a given row happens to be painted.

The pure row-selection and node-id logic
(`offscreen_table_row_indices`, `table_row_node_id`) lives in
`rmac_ui::accessibility` and is unit-tested there. This closes the System
Monitor accept-flow blocker `docs/beta-checklist.md` row 3/6 described;
see that file and `docs/journey-suite.md` journey 6 for the live
confirmation.
