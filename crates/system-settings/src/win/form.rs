//! The grouped-form building blocks the Windows panes share, drawn to the
//! same measurements as Lulo OS's Settings (`controller/view_helpers.rs`,
//! design-lab/settings.html) from the same style tokens.

use std::rc::Rc;

use gpui::{
    div, prelude::FluentBuilder as _, px, svg, AnyElement, App, Div, ElementId, Hsla, IntoElement,
    ParentElement as _, SharedString, Styled as _, Svg, Window,
};
use rmac_ui::{Button, PopUpButton, PopupMenuItem, StyledExt as _};

use super::style;

pub(super) fn label() -> Hsla {
    style::label_text()
}

pub(super) fn secondary() -> Hsla {
    style::secondary_text()
}

pub(super) fn glyph(path: &'static str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(path)
        .w(px(size))
        .h(px(size))
        .text_color(color)
        .flex_none()
}

/// A coloured rounded-square icon tile, as the sidebar and hero draw them.
pub(super) fn tile(path: &'static str, background: Hsla, size: f32) -> Div {
    div()
        .w(px(size))
        .h(px(size))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(size * 0.28))
        .bg(background)
        .when(background.l < 0.2, |tile| {
            tile.border_1()
                .border_color(gpui::hsla(0.0, 0.0, 1.0, 0.18))
        })
        .child(glyph(path, size * 0.62, gpui::white()))
}

/// A grouped-form row on the Mac's 38 pt pitch.
pub(super) fn row_base() -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.0))
        .min_h(px(style::ROW_HEIGHT))
        .px(px(style::ROW_PADDING))
        .py(px(style::ROW_PADDING - 0.5))
}

/// Label (13 pt) with an optional 11 pt subtitle.
pub(super) fn text_block(title: impl Into<SharedString>, subtitle: Option<SharedString>) -> Div {
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
        .when_some(subtitle, |block, subtitle| {
            block.child(
                div()
                    .text_size(rmac_ui::text_px(11.0))
                    .line_height(px(14.0))
                    .text_color(secondary())
                    .child(subtitle),
            )
        })
}

/// A read-only row: label on the left, value on the right.
pub(super) fn value_row(
    title: impl Into<SharedString>,
    value: impl Into<SharedString>,
) -> AnyElement {
    row_base()
        .child(text_block(title, None))
        .child(
            div()
                .max_w(px(300.0))
                .text_right()
                .text_size(rmac_ui::text_px(13.0))
                .text_color(secondary())
                .child(value.into()),
        )
        .into_any_element()
}

/// Rows in one rounded group, separated by inset hairlines.
pub(super) fn card(rows: Vec<AnyElement>) -> Div {
    let count = rows.len();
    let mut group = div()
        .v_flex()
        .mb(px(style::GROUP_GAP))
        .rounded(px(style::GROUP_RADIUS))
        .bg(style::group_fill())
        .overflow_hidden();
    for (index, row) in rows.into_iter().enumerate() {
        group = group.child(row);
        if index + 1 < count {
            group = group.child(
                div()
                    .h(px(style::SEPARATOR))
                    .flex_none()
                    .bg(style::group_separator())
                    .mx(px(style::ROW_PADDING)),
            );
        }
    }
    group
}

/// A 13 pt bold section head above a group.
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

/// The first section head of a pane, right under the toolbar.
pub(super) fn first_section_header(title: impl Into<SharedString>) -> Div {
    section_header(title).pt(px(0.0))
}

/// 11 pt explanatory text under a group.
pub(super) fn footnote(text: impl Into<SharedString>) -> Div {
    div()
        .px(px(style::ROW_PADDING))
        .mb(px(style::GROUP_GAP))
        .text_size(rmac_ui::text_px(11.0))
        .line_height(px(14.0))
        .text_color(secondary())
        .child(text.into())
}

/// A note that needs the user: something could not be read or saved.
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
                .min_w_0()
                .text_size(rmac_ui::text_px(11.5))
                .text_color(rmac_ui::mac::warning_text())
                .child(text.into()),
        )
}

/// A grouped-form push button.
pub(super) fn push_button(id: impl Into<ElementId>, title: impl Into<SharedString>) -> Button {
    Button::new(id, title)
        .h(px(24.0))
        .px(px(10.0))
        .bg(style::control_fill())
        .border_0()
        .text_color(label())
}

/// Right-aligned push buttons under a group.
pub(super) fn footer_buttons(buttons: Vec<AnyElement>) -> Div {
    div()
        .flex()
        .justify_end()
        .items_center()
        .gap(px(10.0))
        .mb(px(style::GROUP_GAP))
        .children(buttons)
}

/// One choice of a [`popup_row`]: its label, whether it is current, and
/// what choosing it does.
pub(super) type Choice = (SharedString, bool, Rc<dyn Fn(&mut Window, &mut App)>);

pub(super) fn choice(
    title: impl Into<SharedString>,
    checked: bool,
    apply: impl Fn(&mut Window, &mut App) + 'static,
) -> Choice {
    (title.into(), checked, Rc::new(apply))
}

/// A form row with a trailing Tahoe pop-up showing the current choice.
pub(super) fn popup_row(
    id: impl Into<ElementId>,
    title: impl Into<SharedString>,
    choices: Vec<Choice>,
    enabled: bool,
) -> AnyElement {
    let current = choices
        .iter()
        .find(|(_, checked, _)| *checked)
        .map(|(title, _, _)| title.clone())
        .unwrap_or_default();
    let choices = Rc::new(choices);
    row_base()
        .child(text_block(title, None))
        .child(
            PopUpButton::new(id, current)
                .form()
                .disabled(!enabled)
                .dropdown_menu(move |mut menu, _, _| {
                    for (title, checked, apply) in choices.iter() {
                        let apply = apply.clone();
                        menu = menu.item(
                            PopupMenuItem::new(title.clone())
                                .checked(*checked)
                                .on_click(move |_, window, cx| apply(window, cx)),
                        );
                    }
                    menu
                }),
        )
        .into_any_element()
}
