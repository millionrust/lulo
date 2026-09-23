//! The macOS 26 form grammar shared by Network, Notifications, Focus, Lock
//! Screen, Privacy & Security, Keyboard, Mouse, Trackpad, the General
//! subpages, Accessibility and Spotlight (design-lab/settings.html, second
//! wave): 52 pt icon rows, status dots, header cards, section notes, stepped
//! sliders, the tab bar, table wells, the storage bar and sheets.

use super::*;

/// A click handler shared by several elements of one control.
pub(in crate::controller) type FormHandler = Rc<dyn Fn(&mut Window, &mut App)>;
/// A handler that receives the picked index (slider step, tab, well row).
pub(in crate::controller) type IndexHandler = Rc<dyn Fn(usize, &mut Window, &mut App)>;

// ---- icon rows ---------------------------------------------------------------

/// A 26 pt coloured icon tile for icon rows and header cards.
pub(in crate::controller) fn tile26(icon: &'static str, color: Hsla) -> AnyElement {
    tile(icon, color, style::LARGE_ICON).into_any_element()
}

/// An application's own icon at `size`, else a tinted tile.
pub(in crate::controller) fn app_icon(
    icon: Option<&PathBuf>,
    fallback: &'static str,
    color: Hsla,
    size: f32,
) -> AnyElement {
    match icon {
        Some(icon) => img(icon.clone())
            .w(px(size))
            .h(px(size))
            .flex_none()
            .into_any_element(),
        None => tile(fallback, color, size).into_any_element(),
    }
}

/// Title (13 on 16) over an optional subtitle element (11 on 14, 2 below),
/// which may be plain text or a [`status_line`].
pub(in crate::controller) fn large_text(
    title: impl Into<SharedString>,
    subtitle: Option<AnyElement>,
) -> Div {
    div()
        .v_flex()
        .flex_1()
        .min_w_0()
        .gap(px(2.0))
        .child(
            div()
                .text_size(rmac_ui::text_px(13.0))
                .line_height(px(16.0))
                .text_color(label())
                .child(title.into()),
        )
        .when_some(subtitle, |block, subtitle| block.child(subtitle))
}

/// 11 pt secondary text for a [`large_text`] subtitle.
pub(in crate::controller) fn subtitle_text(text: impl Into<SharedString>) -> AnyElement {
    div()
        .text_size(rmac_ui::text_px(11.0))
        .line_height(px(14.0))
        .text_color(secondary())
        .child(text.into())
        .into_any_element()
}

/// The Network status line: an 8 pt dot, 4 pt, then 11 pt secondary text.
pub(in crate::controller) fn status_line(color: Hsla, text: impl Into<SharedString>) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(style::STATUS_DOT_GAP))
        .child(
            div()
                .size(px(style::STATUS_DOT))
                .flex_none()
                .rounded_full()
                .bg(color),
        )
        .child(
            div()
                .text_size(rmac_ui::text_px(11.0))
                .line_height(px(14.0))
                .text_color(secondary())
                .child(text.into()),
        )
        .into_any_element()
}

/// A 52 pt icon row: the icon at x + 11, the text at x + 48. Callers append
/// the trailing controls with `.child`.
pub(in crate::controller) fn large_row(
    icon: AnyElement,
    title: impl Into<SharedString>,
    subtitle: Option<AnyElement>,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(style::LARGE_ICON_GAP))
        .min_h(px(style::LARGE_ROW_HEIGHT))
        .pl(px(style::LARGE_ICON_X))
        .pr(px(style::ROW_PADDING))
        .py(px(style::ROW_PADDING))
        .child(icon)
        .child(large_text(title, subtitle))
}

/// A trailing secondary value (Focus "On", Privacy counts, sizes).
pub(in crate::controller) fn trailing_value(text: impl Into<SharedString>) -> AnyElement {
    div()
        .flex_none()
        .text_size(rmac_ui::text_px(13.0))
        .text_color(secondary())
        .child(text.into())
        .into_any_element()
}

/// The 14 pt disclosure chevron of navigation rows.
pub(in crate::controller) fn row_chevron() -> AnyElement {
    glyph(
        "icons/chevron-right.svg",
        style::NAV_CHEVRON,
        style::chevron(),
    )
    .into_any_element()
}

/// A clickable 52 pt icon row with an optional value and the chevron.
pub(in crate::controller) fn large_nav_row(
    id: impl Into<ElementId>,
    icon: AnyElement,
    title: impl Into<SharedString>,
    subtitle: Option<AnyElement>,
    value: Option<SharedString>,
    on_activate: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let content = div()
        .w_full()
        .flex()
        .items_center()
        .gap(px(style::LARGE_ICON_GAP))
        .child(icon)
        .child(large_text(title, subtitle))
        .when_some(value, |row, value| row.child(trailing_value(value)))
        .child(row_chevron());
    ListRow::new(id, content)
        .selected(true)
        .bg(gpui::transparent_black())
        .rounded(px(0.0))
        .h(px(style::LARGE_ROW_HEIGHT))
        .pl(px(style::LARGE_ICON_X))
        .pr(px(style::NAV_TRAILING))
        .on_activate(move |_, window, cx| on_activate(window, cx))
        .into_any_element()
}

/// A clickable 42 pt row with a 20 pt icon, like General's list, for
/// destinations that are not subpage enums (Accessibility, Privacy).
pub(in crate::controller) fn icon_nav_row(
    id: impl Into<ElementId>,
    icon: AnyElement,
    title: impl Into<SharedString>,
    value: Option<SharedString>,
    on_activate: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let content = div()
        .w_full()
        .flex()
        .items_center()
        .gap(px(style::NAV_ICON_GAP))
        .child(icon)
        .child(text_block(title.into(), None))
        .when_some(value, |row, value| row.child(trailing_value(value)))
        .child(row_chevron());
    nav_list_row(id, content)
        .on_activate(move |_, window, cx| on_activate(window, cx))
        .into_any_element()
}

/// A 42 pt row with a 20 pt icon, a label and trailing controls (Spotlight
/// results, Focus apps, Camera apps, Storage categories).
pub(in crate::controller) fn icon_row(icon: AnyElement, title: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(style::NAV_ICON_GAP))
        .min_h(px(style::NAV_ROW_HEIGHT))
        .px(px(style::ROW_PADDING))
        .child(icon)
        .child(text_block(title.into(), None))
}

// ---- cards, heads and notes --------------------------------------------------

/// A group holding free-form children (no separators), radius 12, 10 below.
pub(in crate::controller) fn group() -> Div {
    div()
        .v_flex()
        .mb(px(style::GROUP_GAP))
        .rounded(px(style::GROUP_RADIUS))
        .bg(card_bg())
        .overflow_hidden()
}

/// A 1 pt separator inset 10 on both sides, for groups built by hand.
pub(in crate::controller) fn row_separator() -> Div {
    div()
        .h(px(style::SEPARATOR))
        .flex_none()
        .bg(sep())
        .mx(px(style::ROW_PADDING))
}

/// The header card that opens Notifications, Privacy & Security,
/// Accessibility and Spotlight: a 26 pt icon, the 13 pt title and an 11 pt
/// description, 65 pt tall.
pub(in crate::controller) fn header_card(
    icon: AnyElement,
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    trailing: Option<AnyElement>,
) -> Div {
    group().child(
        div()
            .flex()
            .items_start()
            .gap(px(style::LARGE_ICON_GAP))
            .min_h(px(style::HEADER_CARD_HEIGHT))
            .pl(px(style::LARGE_ICON_X))
            .pr(px(style::ROW_PADDING))
            .py(px(style::ROW_PADDING))
            .child(div().pt(px(style::HEADER_ICON_TOP)).child(icon))
            .child(
                div()
                    .v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(1.0))
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(13.0))
                            .line_height(px(16.0))
                            .text_color(label())
                            .child(title.into()),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(11.0))
                            .line_height(px(14.0))
                            .text_color(secondary())
                            .child(description.into()),
                    ),
            )
            .when_some(trailing, |row, trailing| row.child(trailing)),
    )
}

/// A section head with its explanatory note: the head, 2 pt, the 11 pt note,
/// then 10 pt to the group. `first` drops the 20 pt above the head.
pub(in crate::controller) fn section_with_note(
    title: impl Into<SharedString>,
    note: impl Into<SharedString>,
    first: bool,
) -> Div {
    let head = if first {
        first_section_header(title)
    } else {
        section_header(title)
    };
    div()
        .v_flex()
        .child(head.pb(px(style::SECTION_NOTE_GAP)))
        .child(
            div()
                .px(px(style::ROW_PADDING))
                .mb(px(style::SECTION_BOTTOM))
                .text_size(rmac_ui::text_px(11.0))
                .line_height(px(14.0))
                .text_color(style::note_text())
                .child(note.into()),
        )
}

/// A bold title with an optional note inside the top of a group (Spotlight's
/// "Results from System", Language & Region's "Preferred Languages").
pub(in crate::controller) fn group_heading(
    title: impl Into<SharedString>,
    note: Option<SharedString>,
) -> Div {
    div()
        .v_flex()
        .gap(px(2.0))
        .px(px(style::ROW_PADDING))
        .pt(px(style::ROW_PADDING))
        .pb(px(8.0))
        .child(
            div()
                .text_size(rmac_ui::text_px(13.0))
                .line_height(px(16.0))
                .font_weight(rmac_ui::mac::BOLD)
                .text_color(label())
                .child(title.into()),
        )
        .when_some(note, |block, note| {
            block.child(
                div()
                    .text_size(rmac_ui::text_px(11.0))
                    .line_height(px(14.0))
                    .text_color(secondary())
                    .child(note),
            )
        })
}

/// A centred 11 pt placeholder inside a group ("No Schedules").
pub(in crate::controller) fn group_placeholder(text: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .justify_center()
        .py(px(style::ROW_PADDING))
        .text_size(rmac_ui::text_px(11.0))
        .line_height(px(14.0))
        .text_color(secondary())
        .child(text.into())
}

/// A row whose only content is right-aligned push buttons (Keyboard
/// Shortcuts…, Add Schedule…, Display Settings…).
pub(in crate::controller) fn button_row(buttons: Vec<AnyElement>) -> AnyElement {
    row_base()
        .justify_end()
        .children(buttons)
        .into_any_element()
}

// ---- rows with controls --------------------------------------------------------

/// A label, optional subtitle and a trailing switch.
pub(in crate::controller) fn switch_row(
    id: impl Into<ElementId>,
    title: impl Into<SharedString>,
    subtitle: Option<SharedString>,
    checked: bool,
    enabled: bool,
    on_change: impl Fn(bool, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let has_subtitle = subtitle.is_some();
    row_base()
        .when(has_subtitle, |row| row.items_start())
        .child(text_block(title.into(), subtitle))
        .child(
            Toggle::new(id)
                .checked(checked)
                .disabled(!enabled)
                .on_click(move |value, window, cx| on_change(*value, window, cx)),
        )
        .into_any_element()
}

/// A label with a secondary value and an optional push button ("Time zone …
/// Set…", "Input Sources … Edit…").
pub(in crate::controller) fn value_button_row(
    title: impl Into<SharedString>,
    subtitle: Option<SharedString>,
    value: Option<SharedString>,
    button: Option<AnyElement>,
) -> AnyElement {
    let has_subtitle = subtitle.is_some();
    row_base()
        .when(has_subtitle, |row| row.items_start())
        .child(text_block(title.into(), subtitle))
        .when_some(value, |row, value| row.child(trailing_value(value)))
        .when_some(button, |row, button| row.child(button))
        .into_any_element()
}

/// A label with plain-text value only, for facts (About, Software Update).
pub(in crate::controller) fn fact_row(
    title: impl Into<SharedString>,
    value: impl Into<SharedString>,
) -> AnyElement {
    value_button_row(title, None, Some(value.into()), None)
}

/// A 16 pt radio circle, the accent with a white dot when selected.
fn radio_circle(selected: bool) -> Div {
    div()
        .size(px(style::RADIO))
        .flex_none()
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(if selected {
            style::control_accent()
        } else {
            style::control_off()
        })
        .when(selected, |circle| {
            circle.child(div().size(px(6.0)).rounded_full().bg(gpui::white()))
        })
}

/// The Mac's inline radio row: the label, then the choices right-aligned 14
/// apart ("Login window shows", "Temperature").
pub(in crate::controller) fn radio_row(
    id: &'static str,
    title: impl Into<SharedString>,
    choices: Vec<PopupChoice>,
    enabled: bool,
) -> AnyElement {
    let options = choices
        .into_iter()
        .enumerate()
        .map(|(index, (label, checked, apply))| {
            div()
                .id(SharedString::from(format!("{id}-{index}")))
                .flex()
                .items_center()
                .gap(px(6.0))
                .when(enabled && !checked, |option| {
                    option
                        .cursor_pointer()
                        .on_click(move |_, window, cx| apply(window, cx))
                })
                .when(!enabled, |option| option.opacity(0.5))
                .child(radio_circle(checked))
                .child(
                    div()
                        .text_size(rmac_ui::text_px(13.0))
                        .text_color(label_text())
                        .child(label),
                )
        });
    row_base()
        .child(text_block(title.into(), None))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(style::RADIO_GAP))
                .children(options),
        )
        .into_any_element()
}

fn label_text() -> Hsla {
    label()
}

/// A 16 pt checkbox drawn like the Mac's (accent with a white tick).
pub(in crate::controller) fn form_checkbox(checked: bool) -> Div {
    div()
        .size(px(style::RADIO))
        .flex_none()
        .rounded(px(4.0))
        .flex()
        .items_center()
        .justify_center()
        .bg(if checked {
            style::control_accent()
        } else {
            style::control_off()
        })
        .when(checked, |check| {
            check.child(glyph("icons/check.svg", 11.0, gpui::white()))
        })
}

// ---- stepped sliders -----------------------------------------------------------

/// A discrete slider over `steps` positions: the 6 pt track filled to the
/// knob, a 20 × 16 capsule knob, tick dots under the track and the 11 pt end
/// labels. Clicking anywhere picks the nearest step. `selected` None (a
/// value between presets) draws no knob.
#[allow(clippy::too_many_arguments)]
pub(in crate::controller) fn stepped_slider(
    id: &'static str,
    steps: usize,
    selected: Option<usize>,
    start_label: impl Into<SharedString>,
    end_label: impl Into<SharedString>,
    width: f32,
    enabled: bool,
    on_pick: IndexHandler,
) -> Div {
    let steps = steps.max(2);
    let usable = width - style::SLIDER_KNOB_WIDTH;
    let knob_x = selected.map(|step| usable * step.min(steps - 1) as f32 / (steps - 1) as f32);
    let track = div()
        .absolute()
        .left(px(0.0))
        .right(px(0.0))
        .top(px((style::SLIDER_KNOB_HEIGHT - style::SLIDER_TRACK) / 2.0))
        .h(px(style::SLIDER_TRACK))
        .rounded_full()
        .bg(style::control_off())
        .when_some(knob_x, |track, x| {
            track.child(
                div()
                    .absolute()
                    .left(px(0.0))
                    .top(px(0.0))
                    .h_full()
                    .w(px(x + style::SLIDER_KNOB_WIDTH / 2.0))
                    .rounded_full()
                    .bg(style::control_accent()),
            )
        });
    let knob = knob_x.map(|x| {
        div()
            .absolute()
            .left(px(x))
            .top(px(0.0))
            .w(px(style::SLIDER_KNOB_WIDTH))
            .h(px(style::SLIDER_KNOB_HEIGHT))
            .rounded_full()
            .bg(style::slider_knob())
            .shadow_sm()
    });
    // One invisible hit zone per step, centred on it, laid over the track.
    let zones = (0..steps).map(|step| {
        let pick = on_pick.clone();
        div()
            .id(SharedString::from(format!("{id}-step-{step}")))
            .flex_1()
            .h_full()
            .when(enabled, |zone| {
                zone.cursor_pointer()
                    .on_click(move |_, window, cx| pick(step, window, cx))
            })
    });
    let ticks = (0..steps).map(|_| div().w(px(1.0)).h(px(2.0)).bg(style::chevron()));
    div()
        .w(px(width))
        .flex_none()
        .v_flex()
        .when(!enabled, |slider| slider.opacity(0.5))
        .child(
            div()
                .relative()
                .w_full()
                .h(px(style::SLIDER_KNOB_HEIGHT))
                .child(track)
                .children(knob)
                .child(div().absolute().inset_0().flex().children(zones)),
        )
        .child(
            div()
                .flex()
                .justify_between()
                .px(px(style::SLIDER_KNOB_WIDTH / 2.0 - 0.5))
                .mt(px(1.0))
                .children(ticks),
        )
        .child(
            div()
                .flex()
                .justify_between()
                .mt(px(1.0))
                .text_size(rmac_ui::text_px(11.0))
                .line_height(px(14.0))
                .text_color(label())
                .child(start_label.into())
                .child(end_label.into()),
        )
}

/// A form row with a label on the left and a 242 pt [`stepped_slider`] on
/// the right (Trackpad and Mouse "Tracking speed"), 51 pt tall.
#[allow(clippy::too_many_arguments)]
pub(in crate::controller) fn stepped_slider_row(
    id: &'static str,
    title: impl Into<SharedString>,
    steps: usize,
    selected: Option<usize>,
    start_label: impl Into<SharedString>,
    end_label: impl Into<SharedString>,
    enabled: bool,
    on_pick: IndexHandler,
) -> AnyElement {
    row_base()
        .items_start()
        .child(text_block(title.into(), None))
        .child(stepped_slider(
            id,
            steps,
            selected,
            start_label,
            end_label,
            style::SLIDER_WIDTH,
            enabled,
            on_pick,
        ))
        .into_any_element()
}

// ---- tabs ----------------------------------------------------------------------

/// Trackpad's tab bar: equal segments across the content width, 24 tall,
/// radius 6, the selected one filled with the control accent.
pub(in crate::controller) fn tab_bar(
    id: &'static str,
    labels: &[&'static str],
    selected: usize,
    on_select: IndexHandler,
) -> Div {
    let tabs = labels.iter().enumerate().map(|(index, name)| {
        let select = on_select.clone();
        let is_selected = index == selected;
        div()
            .id(SharedString::from(format!("{id}-{index}")))
            .flex_1()
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(style::TAB_RADIUS))
            .text_size(rmac_ui::text_px(13.0))
            .when(is_selected, |tab| {
                tab.bg(style::control_accent()).text_color(gpui::white())
            })
            .when(!is_selected, |tab| {
                tab.text_color(label())
                    .cursor_pointer()
                    .on_click(move |_, window, cx| select(index, window, cx))
            })
            .child(*name)
    });
    div()
        .flex()
        .h(px(style::TAB_HEIGHT))
        .mb(px(style::TAB_GAP_BELOW))
        .rounded(px(style::TAB_RADIUS))
        .bg(style::tab_fill())
        .children(tabs)
}

// ---- table wells ---------------------------------------------------------------

/// One 24 pt row of a [`well`]; `selected` rows take the grey selection.
pub(in crate::controller) fn well_row(
    id: impl Into<ElementId>,
    selected: bool,
    content: impl IntoElement,
    on_click: Option<FormHandler>,
) -> AnyElement {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(8.0))
        .h(px(style::WELL_ROW_HEIGHT))
        .px(px(style::ROW_PADDING))
        .when(selected, |row| row.bg(style::sidebar_selection()))
        .when_some(on_click, |row, click| {
            row.cursor_pointer()
                .on_click(move |_, window, cx| click(window, cx))
        })
        .child(content)
        .into_any_element()
}

/// A table inside a group (Login Items' "Open at Login", Spotlight's
/// excluded folders): an optional 28 pt column header, 24 pt rows, then the
/// 24 pt +/− bar under a 1 pt rule. A None handler leaves that glyph dimmed.
pub(in crate::controller) fn well(
    header: Option<AnyElement>,
    rows: Vec<AnyElement>,
    empty_rows: usize,
    add: Option<FormHandler>,
    remove: Option<FormHandler>,
) -> Div {
    let bar_button = |id: &'static str, glyph_text: &'static str, handler: Option<FormHandler>| {
        let enabled = handler.is_some();
        div()
            .id(id)
            .w(px(22.0))
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .text_size(rmac_ui::text_px(13.0))
            .text_color(if enabled { label() } else { style::chevron() })
            .when_some(handler, |button, handler| {
                button
                    .cursor_pointer()
                    .on_click(move |_, window, cx| handler(window, cx))
            })
            .child(glyph_text)
    };
    let fillers = (rows.len()..rows.len().max(empty_rows))
        .map(|_| div().h(px(style::WELL_ROW_HEIGHT)).into_any_element());
    group()
        .when_some(header, |well, header| {
            well.child(
                div()
                    .flex()
                    .items_center()
                    .h(px(style::WELL_HEADER_HEIGHT))
                    .px(px(style::ROW_PADDING))
                    .text_size(rmac_ui::text_px(11.0))
                    .text_color(secondary())
                    .child(header),
            )
            .child(div().h(px(style::SEPARATOR)).bg(sep()))
        })
        .children(rows)
        .children(fillers)
        .child(div().h(px(style::SEPARATOR)).bg(sep()))
        .child(
            div()
                .flex()
                .items_center()
                .h(px(style::WELL_BAR_HEIGHT))
                .pl(px(4.0))
                .child(bar_button("well-add", "+", add))
                .child(div().w(px(1.0)).h(px(12.0)).bg(sep()))
                .child(bar_button("well-remove", "−", remove)),
        )
}

// ---- circled buttons -------------------------------------------------------------

/// The 17 pt circled (i) at the end of a row (Sharing, Storage, VPN).
pub(in crate::controller) fn info_button(
    id: impl Into<ElementId>,
    tooltip: &'static str,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    circled_button(id, "icons/info.svg", tooltip, on_click)
}

/// The circled magnifier that reveals a login item's file.
pub(in crate::controller) fn reveal_button(
    id: impl Into<ElementId>,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    circled_button(id, "icons/search.svg", "Show in Files", on_click)
}

fn circled_button(
    id: impl Into<ElementId>,
    icon: &'static str,
    tooltip: &'static str,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .size(px(style::INFO_BUTTON))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .tooltip(move |window, cx| gpui_component::tooltip::Tooltip::new(tooltip).build(window, cx))
        .on_click(move |_, window, cx| on_click(window, cx))
        .child(glyph(icon, style::INFO_BUTTON, label()))
        .into_any_element()
}

// ---- storage ---------------------------------------------------------------------

/// The Storage bar: one segment per (colour, fraction of the volume), 1 pt
/// gaps, then the free space with its size centred on it.
pub(in crate::controller) fn storage_bar(
    segments: &[(Hsla, f32)],
    free_fraction: f32,
    free_label: impl Into<SharedString>,
) -> Div {
    let bars = segments
        .iter()
        .filter(|(_, fraction)| *fraction > 0.0)
        .map(|(color, fraction)| {
            div()
                .h_full()
                .flex_none()
                .w(gpui::relative(fraction.clamp(0.0, 1.0)))
                .bg(*color)
        });
    div()
        .mx(px(style::ROW_PADDING))
        .h(px(style::STORAGE_BAR_HEIGHT))
        .flex()
        .gap(px(1.0))
        .rounded(px(style::STORAGE_BAR_RADIUS))
        .overflow_hidden()
        .bg(style::storage_gap())
        .children(bars)
        .child(
            div()
                .h_full()
                .flex_1()
                .min_w(px(0.0))
                .when(free_fraction <= 0.0, |free| free.w(px(0.0)))
                .flex()
                .items_center()
                .justify_center()
                .bg(style::storage_free())
                .text_size(rmac_ui::text_px(11.0))
                .text_color(label())
                .child(free_label.into()),
        )
}

/// The legend under the Storage bar: 8 pt dots and 11 pt names.
pub(in crate::controller) fn storage_legend(items: Vec<(Hsla, SharedString)>) -> Div {
    div()
        .flex()
        .flex_wrap()
        .gap(px(12.0))
        .px(px(style::ROW_PADDING))
        .pt(px(6.0))
        .pb(px(style::ROW_PADDING))
        .children(items.into_iter().map(|(color, name)| {
            div()
                .flex()
                .items_center()
                .gap(px(style::STATUS_DOT_GAP))
                .child(div().size(px(style::STATUS_DOT)).rounded_full().bg(color))
                .child(
                    div()
                        .text_size(rmac_ui::text_px(11.0))
                        .line_height(px(14.0))
                        .text_color(secondary())
                        .child(name),
                )
        }))
}

// ---- sheets ----------------------------------------------------------------------

/// A Settings sheet (Keyboard Shortcuts): a `width` × `height` card, radius
/// 26, centred over the window with the scrim. `sidebar` fills the 200 pt
/// panel inset 8; `body` starts at x 228, y 20; `footer` is the 65 pt strip
/// under a 1 pt rule, its buttons right-aligned.
pub(in crate::controller) fn settings_sheet(
    id: &'static str,
    width: f32,
    height: f32,
    sidebar: Option<AnyElement>,
    body: AnyElement,
    footer: Vec<AnyElement>,
) -> gpui::Stateful<Div> {
    let has_sidebar = sidebar.is_some();
    let content_left = if has_sidebar {
        style::SIDEBAR_INSET + style::SHEET_SIDEBAR_WIDTH
    } else {
        0.0
    };
    let card = div()
        .relative()
        .w(px(width))
        .h(px(height))
        .rounded(px(style::SHEET_RADIUS))
        .bg(style::sheet_fill())
        .border_1()
        .border_color(style::sheet_edge())
        .shadow_xl()
        .overflow_hidden()
        .when_some(sidebar, |card, sidebar| {
            card.child(
                div()
                    .absolute()
                    .left(px(style::SIDEBAR_INSET))
                    .top(px(style::SIDEBAR_INSET))
                    .bottom(px(style::SIDEBAR_INSET))
                    .w(px(style::SHEET_SIDEBAR_WIDTH))
                    .rounded(px(style::SHEET_RADIUS - style::SIDEBAR_INSET))
                    .bg(style::sheet_sidebar())
                    .border_1()
                    .border_color(style::sidebar_panel_edge())
                    .pt(px(style::ROW_PADDING))
                    .px(px(style::ROW_PADDING))
                    .child(sidebar),
            )
        })
        .child(
            div()
                .id(SharedString::from(format!("{id}-body")))
                .absolute()
                .left(px(content_left + style::DETAIL_INSET))
                .right(px(style::DETAIL_INSET))
                .top(px(style::DETAIL_INSET))
                .bottom(px(style::SHEET_FOOTER_HEIGHT))
                .overflow_y_scroll()
                .child(body),
        )
        .child(
            div()
                .absolute()
                .left(px(content_left))
                .right(px(0.0))
                .bottom(px(0.0))
                .h(px(style::SHEET_FOOTER_HEIGHT))
                .border_t_1()
                .border_color(sep())
                .flex()
                .items_center()
                .justify_end()
                .gap(px(10.0))
                .pl(px(style::DETAIL_INSET))
                .pr(px(12.0))
                .children(footer),
        );
    rmac_ui::dialog(id, card)
}

/// A 32 pt row of a sheet's sidebar (the Keyboard Shortcuts categories).
pub(in crate::controller) fn sheet_sidebar_row(
    id: impl Into<ElementId>,
    icon: AnyElement,
    title: impl Into<SharedString>,
    selected: bool,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(style::SIDEBAR_LABEL_X
            - style::SIDEBAR_ICON_X
            - style::SIDEBAR_ICON))
        .h(px(style::SIDEBAR_ROW_HEIGHT))
        .pl(px(style::SIDEBAR_ICON_X))
        .rounded(px(style::SIDEBAR_ROW_RADIUS))
        .when(selected, |row| row.bg(style::sidebar_selection()))
        .when(!selected, |row| row.cursor_pointer())
        .on_click(move |_, window, cx| on_click(window, cx))
        .child(icon)
        .child(
            div()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(style::sidebar_text())
                .child(title.into()),
        )
        .into_any_element()
}

/// The sheet's default button: the accent capsule ("Done").
pub(in crate::controller) fn sheet_default_button(
    id: &'static str,
    title: &'static str,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    rmac_ui::dialog_button(id, title, rmac_ui::DialogButtonKind::Primary)
        .on_click(move |_, window, cx| on_click(window, cx))
        .into_any_element()
}
