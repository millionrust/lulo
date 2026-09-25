//! Shared typed row, control, icon, and visualization builders.

mod appearance_input;
mod bluetooth;
mod displays;
mod focus;
mod form;
mod locale;
mod network;
mod power;
mod shell;
mod sound;
mod storage;

use super::*;
pub(super) use appearance_input::*;
pub(super) use bluetooth::*;
pub(super) use displays::*;
pub(super) use focus::*;
pub(super) use form::*;
pub(super) use locale::*;
pub(super) use network::*;
pub(super) use power::*;
pub(super) use shell::*;
pub(super) use sound::*;
pub(super) use storage::*;
// ---- row / control builders ----------------------------------------------

/// A grouped-form row: 37 tall with 10 of padding, so rows plus their 1 pt
/// separators fall on the Mac's 38 pt pitch (design-lab/settings.html).
pub(super) fn row_base() -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .min_h(px(style::ROW_HEIGHT))
        .px(px(style::ROW_PADDING))
        .py(px(style::ROW_PADDING - 0.5))
}

/// Label (13 pt on a 16 pt line) with an optional 11 pt subtitle 2 below.
pub(super) fn text_block(title: SharedString, sub: Option<SharedString>) -> Div {
    let mut b = div().v_flex().flex_1().min_w_0().gap(px(2.0)).child(
        div()
            .text_size(rmac_ui::text_px(13.0))
            .line_height(px(16.0))
            .text_color(label())
            .child(title),
    );
    if let Some(s) = sub {
        b = b.child(
            div()
                .text_size(rmac_ui::text_px(11.0))
                .line_height(px(14.0))
                .text_color(secondary())
                .child(s),
        );
    }
    b
}

/// One entry of a [`form_popup`] menu: its label, whether it is the current
/// value, and the change it applies.
pub(super) type PopupChoice = (SharedString, bool, Rc<dyn Fn(&mut Window, &mut App)>);

/// Build a [`PopupChoice`].
pub(super) fn choice(
    label: impl Into<SharedString>,
    checked: bool,
    apply: impl Fn(&mut Window, &mut App) + 'static,
) -> PopupChoice {
    (label.into(), checked, Rc::new(apply))
}

/// The Tahoe grouped-form pop-up: the current value then the ⌃⌄ circle,
/// opening a menu with the current value ticked. `current` is shown when no
/// choice is selected (a saved value outside the presets).
pub(super) fn form_popup(
    id: impl Into<ElementId>,
    current: SharedString,
    choices: Vec<PopupChoice>,
    enabled: bool,
) -> PopUpButton {
    let choices = Rc::new(choices);
    PopUpButton::new(id, current)
        .form()
        .disabled(!enabled)
        .dropdown_menu(move |mut menu, _, _| {
            for (label, checked, apply) in choices.iter() {
                let apply = apply.clone();
                menu = menu.item(
                    PopupMenuItem::new(label.clone())
                        .checked(*checked)
                        .on_click(move |_, window, cx| apply(window, cx)),
                );
            }
            menu
        })
}

/// A form row with a label (and optional subtitle) and a trailing pop-up.
pub(super) fn popup_row(
    id: impl Into<ElementId>,
    title: impl Into<SharedString>,
    subtitle: Option<SharedString>,
    current: SharedString,
    choices: Vec<PopupChoice>,
    enabled: bool,
) -> AnyElement {
    row_base()
        .when(subtitle.is_some(), |row| row.items_start())
        .child(text_block(title.into(), subtitle))
        .child(form_popup(id, current, choices, enabled))
        .into_any_element()
}

/// The label of the selected choice, or `fallback` when none is selected.
pub(super) fn popup_value(choices: &[PopupChoice], fallback: &str) -> SharedString {
    choices
        .iter()
        .find(|(_, checked, _)| *checked)
        .map(|(label, _, _)| label.clone())
        .unwrap_or_else(|| SharedString::from(fallback.to_owned()))
}

/// Right-aligned push buttons under the last group, the way the Mac places
/// "Advanced…", "Options…" and similar pane actions.
pub(super) fn footer_buttons(buttons: Vec<AnyElement>) -> Div {
    div()
        .flex()
        .justify_end()
        .items_center()
        .gap(px(10.0))
        .mb(px(style::GROUP_GAP))
        .children(buttons)
}

/// Explanatory text under a group: 11 pt secondary, aligned with row labels
/// (the Mac's Mission Control and Bluetooth "discoverable" notes).
pub(super) fn footnote(text: impl Into<SharedString>) -> Div {
    div()
        .px(px(style::ROW_PADDING))
        .mb(px(style::GROUP_GAP))
        .text_size(rmac_ui::text_px(11.0))
        .line_height(px(14.0))
        .text_color(secondary())
        .child(text.into())
}

/// A grouped-form push button: 24 tall, radius 6, the measured fill.
pub(super) fn push_button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    Button::new(id, label)
        .h(px(24.0))
        .px(px(10.0))
        .bg(style::control_fill())
        .border_0()
        .text_color(label_color())
}

fn label_color() -> Hsla {
    label()
}

/// A read-only row with a right-aligned value.
pub(super) fn value_row(
    icon: &'static str,
    color: Hsla,
    title: SharedString,
    value: SharedString,
) -> AnyElement {
    row_base()
        .child(tile(icon, color, style::ROW_ICON))
        .child(text_block(title, None))
        .child(
            div()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(secondary())
                .child(value),
        )
        .into_any_element()
}

/// rather than a reflection of (or control over) real system hardware.
/// An informational note card, e.g. to flag a pane as simulated/demo state
pub(super) fn note_card(text: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .mb(px(style::GROUP_GAP))
        .px(px(style::ROW_PADDING))
        .py(px(style::ROW_PADDING))
        .rounded(px(style::GROUP_RADIUS))
        .bg(rmac_ui::mac::warning_background())
        .border_1()
        .border_color(rmac_ui::mac::warning_border())
        .child(glyph("icons/info.svg", 15.0, rmac_ui::mac::warning_text()))
        .child(
            div()
                .flex_1()
                // A flex item's default min-width is its content's
                // natural width, so a long single-line message (Language
                // & Region's XKB warning, SET-80) ran past the card's
                // right edge instead of wrapping.
                .min_w_0()
                .text_size(rmac_ui::text_px(11.5))
                .text_color(rmac_ui::mac::warning_text())
                .child(text.into()),
        )
}

/// A section head above a group: 13 pt bold, 10 in, 30 below the previous
/// group (its own 10 gap plus 20) and 10 above the next.
pub(super) fn section_header(title: impl Into<SharedString>) -> Div {
    div()
        .px(px(style::ROW_PADDING))
        .pt(px(style::SECTION_TOP))
        .pb(px(style::SECTION_BOTTOM))
        .text_size(rmac_ui::text_px(13.0))
        .line_height(px(16.0))
        .font_weight(rmac_ui::mac::BOLD)
        .text_color(style::heading_text())
        .child(title.into())
}

/// The first section head of a pane sits 3 below the toolbar instead of 30
/// below a group.
pub(super) fn first_section_header(title: impl Into<SharedString>) -> Div {
    section_header(title).pt(px(0.0))
}

pub(super) fn application_icon(
    icon: Option<&PathBuf>,
    fallback_icon: &'static str,
    fallback_color: Hsla,
) -> AnyElement {
    match icon {
        Some(icon) => img(icon.clone())
            .w(px(style::ROW_ICON))
            .h(px(style::ROW_ICON))
            .flex_none()
            .into_any_element(),
        None => tile(fallback_icon, fallback_color, style::ROW_ICON).into_any_element(),
    }
}

/// A clickable navigation row that pushes a subpage onto the back stack.
pub(super) fn nav_row(
    view: Entity<Settings>,
    icon: &'static str,
    color: Hsla,
    title: SharedString,
    value: Option<SharedString>,
    target: SubPage,
) -> AnyElement {
    let id = ElementId::from(SharedString::from(format!("nav-{title}")));
    nav_list_row(id, nav_content(icon, color, title, value))
        .on_activate(move |_, _, cx| {
            let target = target.clone();
            view.update(cx, |s, cx| s.push(target, cx));
        })
        .into_any_element()
}

/// A General-style row that opens another pane (Date & Time, Sharing, …)
/// filed under General on macOS 26.
pub(super) fn pane_nav_row(
    view: Entity<Settings>,
    icon: &'static str,
    color: Hsla,
    title: &'static str,
) -> AnyElement {
    let id = ElementId::from(SharedString::from(format!("pane-{title}")));
    nav_list_row(id, nav_content(icon, color, title.into(), None))
        .on_activate(move |_, window, cx| {
            view.update(cx, |s, cx| {
                s.select_category(title, window, cx);
            });
        })
        .into_any_element()
}

/// Icon tile 20 at 10, label at 40, optional value, chevron 14 from the edge.
fn nav_content(
    icon: &'static str,
    color: Hsla,
    title: SharedString,
    value: Option<SharedString>,
) -> Div {
    let mut content = div()
        .w_full()
        .flex()
        .items_center()
        .gap(px(style::NAV_ICON_GAP))
        .child(tile(icon, color, style::ROW_ICON))
        .child(text_block(title, None));
    if let Some(v) = value {
        content = content.child(
            div()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(secondary())
                .child(v),
        );
    }
    content.child(glyph(
        "icons/chevron-right.svg",
        style::NAV_CHEVRON,
        style::chevron(),
    ))
}

/// A 42 pt navigation row with no fill of its own (the group supplies it).
pub(super) fn nav_list_row(id: impl Into<ElementId>, content: impl IntoElement) -> ListRow {
    ListRow::new(id, content)
        .selected(true)
        .bg(gpui::transparent_black())
        .rounded(px(0.0))
        .h(px(style::NAV_ROW_HEIGHT))
        .pl(px(style::ROW_PADDING))
        .pr(px(style::NAV_TRAILING))
}

/// A grouped-form box: radius 12, the measured fill, no border, 10 below;
/// rows are divided by 1 pt separators inset 10 on both sides.
pub(super) fn card(rows: Vec<AnyElement>) -> Div {
    let mut c = div()
        .v_flex()
        .mb(px(style::GROUP_GAP))
        .rounded(px(style::GROUP_RADIUS))
        .bg(card_bg())
        .overflow_hidden();
    let n = rows.len();
    for (i, r) in rows.into_iter().enumerate() {
        c = c.child(r);
        if i + 1 < n {
            c = c.child(
                div()
                    .h(px(style::SEPARATOR))
                    .flex_none()
                    .bg(sep())
                    .mx(px(style::ROW_PADDING)),
            );
        }
    }
    c
}
