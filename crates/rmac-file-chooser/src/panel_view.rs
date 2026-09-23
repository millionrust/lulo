//! Rendering of the Open/Save panel at the measured macOS 26 geometry
//! (design-lab/file-chooser.html, `rmac_file_chooser::metrics`).

use std::path::PathBuf;

use gpui::{
    div, prelude::FluentBuilder as _, px, svg, AnyElement, ClickEvent, Context, Div,
    Focusable as _, Hsla, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled, Svg, Window,
};
use rmac_file_chooser::browser::{Location, ViewMode};
use rmac_file_chooser::metrics::{self as m, dark};
use rmac_file_chooser::request::Mode;
use rmac_finder::listing::SortKey;
use rmac_ui::TextField;

use crate::panel::*;

/// Panel colours: measured dark values; light mode uses the shared tokens
/// until it is measured.
#[derive(Clone, Copy)]
struct Colors {
    panel: Hsla,
    sheet: Hsla,
    sidebar: Hsla,
    sidebar_rim: Hsla,
    pill: Hsla,
    hairline: Hsla,
    control: Hsla,
    text: Hsla,
    text_sidebar: Hsla,
    text_file: Hsla,
    label: Hsla,
    section: Hsla,
    sidebar_selected: Hsla,
    glyph_disabled: Hsla,
    glyph: Hsla,
    search_rim: Hsla,
    default: Hsla,
    default_disabled: Hsla,
    text_disabled: Hsla,
    field_rim: Hsla,
    focus_ring: Hsla,
    goto_head: Hsla,
    goto_list: Hsla,
    goto_hairline: Hsla,
    goto_close: Hsla,
    folder: Hsla,
    selection: Hsla,
    on_selection: Hsla,
    menu: Hsla,
}

fn hex(value: u32) -> Hsla {
    gpui::rgb(value).into()
}

impl Colors {
    fn current() -> Self {
        let dark_mode =
            rmac_ui::theme::current().color_scheme == rmac_appearance::ResolvedColorScheme::Dark;
        let accent = rmac_ui::mac::accent();
        if dark_mode {
            Self {
                panel: hex(dark::PANEL),
                sheet: hex(dark::SHEET),
                sidebar: hex(dark::SIDEBAR),
                sidebar_rim: hex(dark::SIDEBAR_RIM),
                pill: hex(dark::PILL),
                hairline: hex(dark::HAIRLINE),
                control: hex(dark::CONTROL),
                text: hex(dark::TEXT),
                text_sidebar: hex(dark::TEXT_SIDEBAR),
                text_file: hex(dark::TEXT_FILE),
                label: hex(dark::LABEL),
                section: hex(dark::SECTION),
                sidebar_selected: hex(dark::SIDEBAR_SELECTED),
                glyph_disabled: hex(dark::GLYPH_DISABLED),
                glyph: hex(dark::GLYPH),
                search_rim: hex(dark::SEARCH_RIM),
                default: hex(dark::DEFAULT),
                default_disabled: hex(dark::DEFAULT_DISABLED),
                text_disabled: hex(dark::TEXT_DISABLED),
                field_rim: hex(dark::FIELD_RIM),
                focus_ring: hex(dark::FOCUS_RING),
                goto_head: hex(dark::GOTO_HEAD),
                goto_list: hex(dark::GOTO_LIST),
                goto_hairline: hex(dark::GOTO_HAIRLINE),
                goto_close: hex(dark::GOTO_CLOSE),
                folder: rmac_ui::mac::folder_blue(),
                selection: accent,
                on_selection: rmac_ui::mac::on_accent(),
                menu: rmac_ui::mac::raised(),
            }
        } else {
            let separator = rmac_ui::mac::separator();
            Self {
                panel: rmac_ui::mac::window(),
                sheet: rmac_ui::mac::sheet(),
                sidebar: rmac_ui::mac::sidebar(),
                sidebar_rim: separator,
                pill: rmac_ui::mac::sidebar_selection(),
                hairline: separator,
                control: rmac_ui::mac::button_secondary(),
                text: rmac_ui::mac::text(),
                text_sidebar: rmac_ui::mac::text(),
                text_file: rmac_ui::mac::text(),
                label: rmac_ui::mac::text_secondary(),
                section: rmac_ui::mac::text_tertiary(),
                sidebar_selected: accent,
                glyph_disabled: rmac_ui::mac::text_tertiary(),
                glyph: rmac_ui::mac::text(),
                search_rim: separator,
                default: accent,
                default_disabled: rmac_ui::mac::control_fill(),
                text_disabled: rmac_ui::mac::text_tertiary(),
                field_rim: separator,
                focus_ring: rmac_ui::mac::accent_border(),
                goto_head: rmac_ui::mac::sheet(),
                goto_list: rmac_ui::mac::window(),
                goto_hairline: separator,
                goto_close: rmac_ui::mac::text_secondary(),
                folder: rmac_ui::mac::folder_blue(),
                selection: accent,
                on_selection: rmac_ui::mac::on_accent(),
                menu: rmac_ui::mac::raised(),
            }
        }
    }
}

fn at(x: f32, y: f32, width: f32, height: f32) -> Div {
    div()
        .absolute()
        .left(px(x))
        .top(px(y))
        .w(px(width))
        .h(px(height))
}

fn glyph(path: &'static str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(path)
        .w(px(size))
        .h(px(size))
        .flex_none()
        .text_color(color)
}

fn label(text: impl Into<SharedString>, size: f32, color: Hsla) -> Div {
    div()
        .text_size(rmac_ui::text_px(size))
        .text_color(color)
        .truncate()
        .child(text.into())
}

/// A 24 pt plate inset 1 pt inside a 26 pt control frame.
fn plate(fill: Hsla) -> Div {
    div()
        .absolute()
        .left(px(m::PLATE_INSET))
        .top(px(m::PLATE_INSET))
        .right(px(m::PLATE_INSET))
        .bottom(px(m::PLATE_INSET))
        .rounded(px(m::PLATE_RADIUS))
        .bg(fill)
}

/// Up/down chevrons of a pop-up button, 15 pt from its right edge.
fn popup_chevrons(color: Hsla) -> Div {
    div()
        .absolute()
        .right(px(10.0))
        .top(px(4.0))
        .w(px(10.0))
        .h(px(18.0))
        .flex()
        .flex_col()
        .items_center()
        .child(glyph("icons/chevron-up.svg", 9.0, color))
        .child(glyph("icons/chevron-down.svg", 9.0, color))
}

impl Panel {
    #[allow(clippy::too_many_arguments)]
    fn button(
        &self,
        id: &'static str,
        text: SharedString,
        x: f32,
        y: f32,
        width: f32,
        primary: bool,
        enabled: bool,
        colors: Colors,
    ) -> gpui::Stateful<Div> {
        let (fill, ink) = match (primary, enabled) {
            (true, true) => (colors.default, colors.on_selection),
            (true, false) => (colors.default_disabled, colors.text_disabled),
            (false, _) => (colors.control, colors.text),
        };
        at(x, y, width, m::CONTROL_HEIGHT).id(id).child(
            plate(fill)
                .flex()
                .items_center()
                .justify_center()
                .child(label(text, 13.0, ink)),
        )
    }

    fn render_sidebar(&self, height: f32, colors: Colors, cx: &mut Context<Self>) -> AnyElement {
        let mut column = div().flex().flex_col().pt(px(m::SIDEBAR_FIRST_ROW));
        for (index, section) in self.sections.iter().enumerate() {
            if section.places.is_empty() {
                continue;
            }
            if index > 0 {
                column = column.child(
                    div()
                        .mt(px(m::SIDEBAR_HEADER_GAP))
                        .h(px(m::SIDEBAR_HEADER_HEIGHT))
                        .pl(px(m::SIDEBAR_HEADER_LEFT))
                        .flex()
                        .items_center()
                        .child(
                            label(section.title.clone(), 11.0, colors.section)
                                .font_weight(rmac_ui::mac::SEMIBOLD),
                        ),
                );
            }
            for (row, place) in section.places.iter().enumerate() {
                let selected = *self.browser.location() == place.location;
                let ink = if selected {
                    colors.sidebar_selected
                } else {
                    colors.text_sidebar
                };
                let location = place.location.clone();
                column = column.child(
                    div()
                        .id(SharedString::from(format!("place-{index}-{row}")))
                        .relative()
                        .ml(px(m::SIDEBAR_PILL_INSET))
                        .w(px(m::SIDEBAR_PILL_WIDTH))
                        .h(px(m::SIDEBAR_ROW_HEIGHT))
                        .flex_none()
                        .rounded(px(m::SIDEBAR_PILL_RADIUS))
                        .when(selected, |row| row.bg(colors.pill))
                        .flex()
                        .items_center()
                        .child(div().w(px(m::SIDEBAR_ICON_LEFT)).flex_none())
                        .child(glyph(place.icon, m::SIDEBAR_ICON, ink))
                        .child(
                            div()
                                .w(px(m::SIDEBAR_LABEL_LEFT
                                    - m::SIDEBAR_ICON_LEFT
                                    - m::SIDEBAR_ICON))
                                .flex_none(),
                        )
                        .child(
                            label(place.name.clone(), 13.0, ink)
                                .flex_1()
                                .min_w(px(0.0))
                                .pr(px(4.0)),
                        )
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            window.focus(&this.focus, cx);
                            this.navigate(location.clone(), cx);
                        })),
                );
            }
        }
        at(
            m::SIDEBAR_INSET,
            m::SIDEBAR_INSET,
            m::SIDEBAR_WIDTH,
            height - 2.0 * m::SIDEBAR_INSET,
        )
        .rounded(px(m::SIDEBAR_RADIUS))
        .bg(colors.sidebar)
        .border_1()
        .border_color(colors.sidebar_rim.opacity(0.5))
        .overflow_hidden()
        .child(
            div()
                .id("sidebar-scroll")
                .size_full()
                .overflow_y_scroll()
                .child(column),
        )
        .into_any_element()
    }

    fn render_toolbar(
        &self,
        top: f32,
        width: f32,
        colors: Colors,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let back = if self.browser.can_go_back() {
            colors.glyph
        } else {
            colors.glyph_disabled
        };
        let forward = if self.browser.can_go_forward() {
            colors.glyph
        } else {
            colors.glyph_disabled
        };
        let saving = self.mode() == Mode::Save;
        let where_width = if saving {
            m::WHERE_WIDTH_SAVE
        } else {
            m::WHERE_WIDTH_OPEN
        };
        let search_x = width - (m::PANEL_WIDTH - m::SEARCH_X);
        let view_glyph = match self.browser.view {
            ViewMode::Icons => "icons/layout-grid.svg",
            ViewMode::List => "icons/list.svg",
        };
        let search_focused = self.search.read(cx).focus_handle(cx).is_focused(window);
        let mut elements = vec![
            at(m::NAV_X, top, m::NAV_WIDTH, m::CONTROL_HEIGHT)
                .child(plate(colors.control))
                .child(
                    at(0.0, 0.0, 24.0, m::CONTROL_HEIGHT)
                        .id("back")
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(glyph("icons/chevron-left.svg", 14.0, back))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.go_back(cx))),
                )
                .child(at(24.0, 6.0, 1.0, 14.0).bg(colors.glyph_disabled.opacity(0.5)))
                .child(
                    at(25.0, 0.0, 24.0, m::CONTROL_HEIGHT)
                        .id("forward")
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(glyph("icons/chevron-right.svg", 14.0, forward))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.go_forward(cx))),
                )
                .into_any_element(),
            at(m::VIEW_X, top, m::VIEW_WIDTH, m::CONTROL_HEIGHT)
                .id("view-menu")
                .child(plate(colors.control))
                .child(at(14.0, 5.0, 16.0, 16.0).child(glyph(view_glyph, 16.0, colors.glyph)))
                .child(at(46.0, 7.0, 12.0, 12.0).child(glyph(
                    "icons/chevron-down.svg",
                    12.0,
                    colors.glyph,
                )))
                .on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_menu(MenuKind::View, cx)),
                )
                .into_any_element(),
            at(m::SORT_X, top, m::SORT_WIDTH, m::CONTROL_HEIGHT)
                .id("sort-menu")
                .child(plate(colors.control))
                .child(at(14.0, 5.0, 16.0, 16.0).child(glyph(
                    "icons/ellipsis.svg",
                    16.0,
                    colors.glyph,
                )))
                .child(at(44.0, 7.0, 12.0, 12.0).child(glyph(
                    "icons/chevron-down.svg",
                    12.0,
                    colors.glyph,
                )))
                .on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_menu(MenuKind::Sort, cx)),
                )
                .into_any_element(),
            self.where_popup(m::WHERE_X, top, where_width, colors, cx),
        ];
        if saving {
            elements.push(
                at(m::DISCLOSURE_X, top, m::DISCLOSURE_WIDTH, m::CONTROL_HEIGHT)
                    .id("collapse")
                    .child(plate(colors.control))
                    .child(at(7.0, 7.0, 12.0, 12.0).child(glyph(
                        "icons/chevron-up.svg",
                        12.0,
                        colors.text,
                    )))
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.toggle_expanded(window, cx)
                    }))
                    .into_any_element(),
            );
        }
        elements.push(
            at(search_x, top, m::SEARCH_WIDTH, m::CONTROL_HEIGHT)
                .rounded(px(m::CONTROL_HEIGHT / 2.0))
                .border_1()
                .border_color(if search_focused {
                    colors.focus_ring
                } else {
                    colors.search_rim
                })
                .child(at(7.0, 6.0, 14.0, 14.0).child(glyph(
                    "icons/search.svg",
                    14.0,
                    colors.label,
                )))
                .child(
                    at(24.0, 1.0, m::SEARCH_WIDTH - 30.0, 24.0).child(
                        TextField::new(&self.search)
                            .appearance(false)
                            .text_size(rmac_ui::text_px(13.0)),
                    ),
                )
                .into_any_element(),
        );
        elements
    }

    fn where_popup(
        &self,
        x: f32,
        y: f32,
        width: f32,
        colors: Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (title, icon) = match self.browser.location() {
            Location::Folder(path) => (
                rmac_file_chooser::browser::display_name(path),
                if path.parent().is_none() {
                    "icons/hard-drive.svg"
                } else {
                    "icons/folder-artwork.svg"
                },
            ),
            Location::Recents => ("Recents".to_owned(), "icons/clock.svg"),
            Location::Search(_) => ("Search".to_owned(), "icons/search.svg"),
        };
        at(x, y, width, m::CONTROL_HEIGHT)
            .id("where")
            .child(plate(colors.control))
            .child(at(15.0, 5.0, 16.0, 16.0).child(glyph(icon, 16.0, colors.folder)))
            .child(at(34.0, 4.0, width - 60.0, 18.0).child(label(title, 13.0, colors.text)))
            .child(popup_chevrons(colors.text))
            .on_click(
                cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_menu(MenuKind::Where, cx)),
            )
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn simple_popup(
        &self,
        id: SharedString,
        x: f32,
        y: f32,
        width: f32,
        title: SharedString,
        kind: MenuKind,
        colors: Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        at(x, y, width, m::CONTROL_HEIGHT)
            .id(id)
            .child(plate(colors.control))
            .child(at(12.0, 4.0, width - 36.0, 18.0).child(label(title, 13.0, colors.text)))
            .child(popup_chevrons(colors.text))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_menu(kind, cx)))
            .into_any_element()
    }

    fn checkbox(
        &self,
        index: usize,
        x: f32,
        y: f32,
        colors: Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(choice) = self.choices.get(index) else {
            return div().into_any_element();
        };
        let checked = choice.checked();
        at(x, y, 220.0, m::CONTROL_HEIGHT)
            .id(SharedString::from(format!("choice-{index}")))
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(
                div()
                    .w(px(14.0))
                    .h(px(14.0))
                    .flex_none()
                    .rounded(px(3.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(checked, |box_| box_.bg(colors.selection))
                    .when(!checked, |box_| {
                        box_.bg(colors.control)
                            .border_1()
                            .border_color(colors.field_rim)
                    })
                    .when(checked, |box_| {
                        box_.child(label("✓", 10.0, colors.on_selection))
                    }),
            )
            .child(label(choice.label.clone(), 13.0, colors.text))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.set_choice(index, String::new(), cx)
            }))
            .into_any_element()
    }

    fn name_field(
        &self,
        x: f32,
        y: f32,
        colors: Colors,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(name) = self.name.as_ref() else {
            return div().into_any_element();
        };
        let focused = name.read(cx).focus_handle(cx).is_focused(window);
        at(x, y, m::FIELD_WIDTH, m::CONTROL_HEIGHT)
            .rounded(px(m::PLATE_RADIUS))
            .border_1()
            .border_color(colors.field_rim)
            .when(focused, |field| {
                field
                    .border_color(colors.focus_ring)
                    .shadow(vec![gpui::BoxShadow {
                        color: colors.focus_ring,
                        offset: gpui::point(px(0.0), px(0.0)),
                        blur_radius: px(0.0),
                        spread_radius: px(2.0),
                        inset: false,
                    }])
            })
            .child(
                at(4.0, 1.0, m::FIELD_WIDTH - 8.0, 24.0).child(
                    TextField::new(name)
                        .appearance(false)
                        .text_size(rmac_ui::text_px(13.0)),
                ),
            )
            .into_any_element()
    }

    /// Save header rows at the given label/field columns; returns the y of
    /// the next row.
    fn save_header(
        &self,
        label_x: f32,
        field_x: f32,
        colors: Colors,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Vec<AnyElement>, f32) {
        let mut elements = Vec::new();
        let mut y = m::HEADER_FIRST_ROW;
        let row_label = |text: SharedString, y: f32| {
            at(label_x, y + 4.0, m::LABEL_WIDTH, 18.0)
                .flex()
                .justify_end()
                .child(label(text, 13.0, colors.label))
                .into_any_element()
        };
        elements.push(row_label("Save As:".into(), y));
        elements.push(self.name_field(field_x, y, colors, window, cx));
        y += m::HEADER_ROW_PITCH;
        if self.shows_format_row() {
            elements.push(row_label("File Format:".into(), y));
            elements.push(self.simple_popup(
                "format".into(),
                field_x,
                y,
                m::FIELD_WIDTH,
                self.filter_name(),
                MenuKind::Filter,
                colors,
                cx,
            ));
            y += m::HEADER_ROW_PITCH;
        }
        for (index, choice) in self.choices.iter().enumerate() {
            if choice.is_checkbox() {
                elements.push(self.checkbox(index, field_x, y, colors, cx));
            } else {
                elements.push(row_label(format!("{}:", choice.label).into(), y));
                let title = choice
                    .options
                    .iter()
                    .find(|(key, _)| *key == choice.selected)
                    .map(|(_, label)| label.clone())
                    .unwrap_or_default();
                elements.push(self.simple_popup(
                    SharedString::from(format!("choice-popup-{index}")),
                    field_x,
                    y,
                    m::FIELD_WIDTH,
                    title.into(),
                    MenuKind::Choice(index),
                    colors,
                    cx,
                ));
            }
            y += m::HEADER_ROW_PITCH;
        }
        (elements, y)
    }

    fn render_compact(
        &self,
        width: f32,
        colors: Colors,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let (mut elements, where_y) =
            self.save_header(m::COMPACT_LABEL_X, m::COMPACT_FIELD_X, colors, window, cx);
        elements.push(
            at(m::COMPACT_LABEL_X, where_y + 4.0, m::LABEL_WIDTH, 18.0)
                .flex()
                .justify_end()
                .child(label("Where:", 13.0, colors.label))
                .into_any_element(),
        );
        elements.push(self.where_popup(
            m::COMPACT_FIELD_X,
            where_y,
            m::WHERE_WIDTH_SAVE,
            colors,
            cx,
        ));
        elements.push(
            at(
                m::COMPACT_DISCLOSURE_X,
                where_y,
                m::DISCLOSURE_WIDTH,
                m::CONTROL_HEIGHT,
            )
            .id("expand")
            .child(plate(colors.control))
            .child(at(7.0, 7.0, 12.0, 12.0).child(glyph(
                "icons/chevron-down.svg",
                12.0,
                colors.text,
            )))
            .on_click(
                cx.listener(|this, _: &ClickEvent, window, cx| this.toggle_expanded(window, cx)),
            )
            .into_any_element(),
        );
        let buttons_y = where_y + m::COMPACT_BUTTONS_BELOW_WHERE;
        let offset = width - m::COMPACT_WIDTH;
        if let Some(notice) = self.notice.clone() {
            elements.push(
                at(
                    20.0,
                    buttons_y + 4.0,
                    m::COMPACT_CANCEL_X + offset - 28.0,
                    18.0,
                )
                .child(label(notice, 11.0, colors.label))
                .into_any_element(),
            );
        }
        elements.push(
            self.button(
                "cancel",
                "Cancel".into(),
                m::COMPACT_CANCEL_X + offset,
                buttons_y,
                m::BUTTON_WIDTH,
                false,
                true,
                colors,
            )
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.cancel(window, cx)))
            .into_any_element(),
        );
        let enabled = self.can_accept(cx);
        elements.push(
            self.button(
                "accept",
                self.request.accept_label().into(),
                m::COMPACT_SAVE_X + offset,
                buttons_y,
                m::BUTTON_WIDTH,
                true,
                enabled,
                colors,
            )
            .when(enabled, |button| {
                button.on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| this.accept(window, cx)),
                )
            })
            .into_any_element(),
        );
        elements
    }

    fn render_icons(
        &self,
        content_width: f32,
        colors: Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let columns = self.icon_columns(content_width);
        let first_x = m::ICON_FIRST_X - (m::ICON_LABEL_WIDTH - m::ICON_SIZE) / 2.0;
        let rows = self.browser.rows().len().div_ceil(columns.max(1));
        let mut grid = div()
            .relative()
            .w(px(content_width))
            .h(px(m::ICON_FIRST_Y + rows as f32 * m::ICON_PITCH_Y));
        for (index, row) in self.browser.rows().iter().enumerate() {
            let column = index % columns.max(1);
            let line = index / columns.max(1);
            let selected = self.browser.is_selected(index);
            let (icon, tint) = if row.item.is_dir {
                ("icons/folder-artwork.svg", colors.folder)
            } else {
                ("icons/file-fill.svg", colors.label)
            };
            grid = grid.child(
                at(
                    first_x + column as f32 * m::ICON_PITCH_X,
                    m::ICON_FIRST_Y + line as f32 * m::ICON_PITCH_Y,
                    m::ICON_LABEL_WIDTH,
                    m::ICON_PITCH_Y - 8.0,
                )
                .id(SharedString::from(format!("tile-{index}")))
                .flex()
                .flex_col()
                .items_center()
                .when(!row.enabled, |tile| tile.opacity(0.4))
                .child(
                    div()
                        .p(px(3.0))
                        .rounded(px(6.0))
                        .when(selected, |plate| plate.bg(colors.pill))
                        .child(glyph(icon, m::ICON_SIZE, tint)),
                )
                .child(
                    div()
                        .mt(px(m::ICON_LABEL_GAP - 3.0))
                        .max_w(px(m::ICON_LABEL_WIDTH))
                        .px(px(4.0))
                        .rounded(px(4.0))
                        .when(selected, |name| name.bg(colors.selection))
                        .text_size(rmac_ui::text_px(13.0))
                        .line_height(px(m::ICON_LABEL_LINE))
                        .text_center()
                        .line_clamp(2)
                        .text_color(if selected {
                            colors.on_selection
                        } else {
                            colors.text_file
                        })
                        .child(row.item.name.clone()),
                )
                .on_click(cx.listener(
                    move |this, event: &ClickEvent, window, cx| {
                        let modifiers = event.modifiers();
                        this.click_row(
                            index,
                            modifiers.platform,
                            modifiers.shift,
                            event.click_count() >= 2,
                            window,
                            cx,
                        )
                    },
                )),
            );
        }
        grid.into_any_element()
    }

    fn render_list(
        &self,
        content_width: f32,
        colors: Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        const DATE_W: f32 = 184.0;
        const SIZE_W: f32 = 80.0;
        const KIND_W: f32 = 150.0;
        let name_width = (content_width - DATE_W - SIZE_W - KIND_W - 46.0).max(80.0);
        let header_cell = |title: &'static str, key: SortKey, width: f32| {
            let active = self.browser.sort == key;
            div()
                .id(title)
                .w(px(width))
                .flex_none()
                .flex()
                .items_center()
                .gap(px(4.0))
                .child(
                    label(title, 11.0, colors.label)
                        .when(active, |cell| cell.font_weight(rmac_ui::mac::SEMIBOLD)),
                )
                .when(active, |cell| {
                    cell.child(glyph(
                        if self.browser.ascending {
                            "icons/chevron-up.svg"
                        } else {
                            "icons/chevron-down.svg"
                        },
                        9.0,
                        colors.label,
                    ))
                })
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_sort(key, cx)))
        };
        let mut list = div().flex().flex_col().w(px(content_width)).child(
            div()
                .h(px(m::LIST_ROW_HEIGHT))
                .flex()
                .items_center()
                .pl(px(22.0))
                .border_b_1()
                .border_color(colors.hairline)
                .child(header_cell("Name", SortKey::Name, name_width + 24.0))
                .child(header_cell("Date Modified", SortKey::Date, DATE_W))
                .child(header_cell("Size", SortKey::Size, SIZE_W))
                .child(header_cell("Kind", SortKey::Kind, KIND_W)),
        );
        for (index, row) in self.browser.rows().iter().enumerate() {
            let selected = self.browser.is_selected(index);
            let ink = if selected {
                colors.on_selection
            } else {
                colors.text_file
            };
            let secondary = if selected {
                colors.on_selection
            } else {
                colors.label
            };
            let (icon, tint) = if selected {
                (
                    if row.item.is_dir {
                        "icons/folder-artwork.svg"
                    } else {
                        "icons/file-fill.svg"
                    },
                    colors.on_selection,
                )
            } else if row.item.is_dir {
                ("icons/folder-artwork.svg", colors.folder)
            } else {
                ("icons/file-fill.svg", colors.label)
            };
            list = list.child(
                div()
                    .id(SharedString::from(format!("row-{index}")))
                    .h(px(m::LIST_ROW_HEIGHT))
                    .mx(px(10.0))
                    .rounded(px(5.0))
                    .flex()
                    .items_center()
                    .pl(px(12.0))
                    .when(selected, |line| line.bg(colors.selection))
                    .when(!selected && index % 2 == 1, |line| {
                        line.bg(colors.text.opacity(0.03))
                    })
                    .when(!row.enabled, |line| line.opacity(0.4))
                    .child(glyph(icon, 16.0, tint))
                    .child(
                        label(row.item.name.clone(), 13.0, ink)
                            .w(px(name_width))
                            .ml(px(8.0)),
                    )
                    .child(label(row.item.date_label(), 13.0, secondary).w(px(DATE_W)))
                    .child(label(row.item.size_label(), 13.0, secondary).w(px(SIZE_W)))
                    .child(label(row.item.kind.clone(), 13.0, secondary).w(px(KIND_W)))
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        let modifiers = event.modifiers();
                        this.click_row(
                            index,
                            modifiers.platform,
                            modifiers.shift,
                            event.click_count() >= 2,
                            window,
                            cx,
                        )
                    })),
            );
        }
        list.into_any_element()
    }

    fn render_menu(
        &self,
        width: f32,
        height: f32,
        colors: Colors,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let kind = self.menu?;
        let items = self.menu_items(kind);
        if items.is_empty() {
            return None;
        }
        const ITEM: f32 = 22.0;
        const SEPARATOR: f32 = 11.0;
        let menu_height = items
            .iter()
            .map(|item| {
                if item.action.is_some() {
                    ITEM
                } else {
                    SEPARATOR
                }
            })
            .sum::<f32>()
            + 10.0;
        let (x, anchor_y, menu_width) = self.menu_anchor(kind, width, height);
        let menu_height = menu_height.min(height - 8.0);
        let mut y = anchor_y + m::CONTROL_HEIGHT + 2.0;
        if y + menu_height > height - 4.0 {
            y = (anchor_y - menu_height - 2.0).max(4.0);
        }
        if y + menu_height > height - 4.0 {
            y = (height - 4.0 - menu_height).max(4.0);
        }
        let mut column = div().flex().flex_col().py(px(5.0));
        for (index, item) in items.into_iter().enumerate() {
            match item.action {
                None => {
                    column = column.child(
                        div()
                            .h(px(SEPARATOR))
                            .flex()
                            .items_center()
                            .px(px(10.0))
                            .child(div().h(px(1.0)).w_full().bg(colors.hairline)),
                    );
                }
                Some(action) => {
                    column = column.child(
                        div()
                            .id(SharedString::from(format!("menu-item-{index}")))
                            .h(px(ITEM))
                            .mx(px(5.0))
                            .px(px(6.0))
                            .rounded(px(5.0))
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .hover(|row| row.bg(colors.selection))
                            .child(div().w(px(12.0)).flex_none().when(item.checked, |mark| {
                                mark.child(label("✓", 12.0, colors.text))
                            }))
                            .when_some(item.icon, |row, icon| {
                                row.child(glyph(icon, 16.0, colors.folder))
                            })
                            .child(label(item.label.clone(), 13.0, colors.text).flex_1())
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.run_menu(action.clone(), cx)
                            })),
                    );
                }
            }
        }
        Some(
            div()
                .absolute()
                .inset_0()
                .id("menu-scrim")
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _: &MouseDownEvent, _, cx| {
                        this.menu = None;
                        cx.notify();
                    }),
                )
                .child(
                    at(x, y, menu_width, menu_height)
                        .id("menu")
                        .occlude()
                        .rounded(px(8.0))
                        .bg(colors.menu)
                        .border_1()
                        .border_color(colors.hairline)
                        .shadow_lg()
                        .overflow_y_scroll()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(column),
                )
                .into_any_element(),
        )
    }

    /// Left edge, control top, and width of the control that owns `kind`.
    fn menu_anchor(&self, kind: MenuKind, width: f32, height: f32) -> (f32, f32, f32) {
        let bottom_row = height - m::BOTTOM_ROW;
        let compact = self.is_compact();
        let field_x = if compact {
            m::COMPACT_FIELD_X
        } else {
            m::WHERE_X
        };
        let header_row = |index: usize| m::HEADER_FIRST_ROW + m::HEADER_ROW_PITCH * index as f32;
        let toolbar = self.toolbar_top();
        match kind {
            MenuKind::Where if compact => (
                m::COMPACT_FIELD_X,
                header_row(self.header_rows()),
                m::WHERE_WIDTH_SAVE,
            ),
            MenuKind::Where => (m::WHERE_X, toolbar, m::WHERE_WIDTH_OPEN),
            MenuKind::View => (m::VIEW_X, toolbar, 160.0),
            MenuKind::Sort => (m::SORT_X, toolbar, 200.0),
            MenuKind::Filter if self.mode() == Mode::Save => {
                (field_x, header_row(1), m::FIELD_WIDTH)
            }
            MenuKind::Filter => (m::NAV_X, bottom_row, 240.0),
            MenuKind::Choice(index) if self.mode() == Mode::Save => (
                field_x,
                header_row(1 + usize::from(self.shows_format_row()) + index),
                m::FIELD_WIDTH,
            ),
            MenuKind::Choice(_) => (m::NAV_X, bottom_row, 240.0f32.min(width - m::NAV_X)),
        }
    }

    fn toolbar_top(&self) -> f32 {
        if self.mode() == Mode::Save {
            m::expanded_toolbar_top(self.header_rows())
        } else {
            m::TOOLBAR_TOP
        }
    }

    fn render_goto(
        &self,
        width: f32,
        height: f32,
        colors: Colors,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let sheet = self.goto.as_ref()?;
        let x = ((width - m::GOTO_WIDTH) / 2.0).max(0.0);
        let y = ((height - m::GOTO_HEIGHT) / 2.0).max(0.0);
        let mut list = div().flex().flex_col().pt(px(4.0));
        if sheet.error {
            list = list.child(
                div()
                    .h(px(24.0))
                    .px(px(15.0))
                    .flex()
                    .items_center()
                    .child(label("The folder can’t be found.", 13.0, colors.label)),
            );
        }
        for (index, path) in sheet.suggestions.iter().enumerate() {
            let highlighted = sheet.highlighted == Some(index);
            let target: PathBuf = path.clone();
            list = list.child(
                div()
                    .id(SharedString::from(format!("goto-{index}")))
                    .h(px(24.0))
                    .mx(px(6.0))
                    .px(px(9.0))
                    .rounded(px(5.0))
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .when(highlighted, |row| row.bg(colors.selection))
                    .child(glyph(
                        "icons/folder-artwork.svg",
                        16.0,
                        if highlighted {
                            colors.on_selection
                        } else {
                            colors.folder
                        },
                    ))
                    .child(label(
                        rmac_file_chooser::goto::display(path, &self.home),
                        13.0,
                        if highlighted {
                            colors.on_selection
                        } else {
                            colors.text
                        },
                    ))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.goto_pick(target.clone(), window, cx)
                    })),
            );
        }
        Some(
            div()
                .absolute()
                .inset_0()
                .bg(gpui::black().opacity(0.1))
                .child(
                    at(x, y, m::GOTO_WIDTH, m::GOTO_HEIGHT)
                        .id("goto-sheet")
                        .occlude()
                        .rounded(px(m::GOTO_RADIUS))
                        .overflow_hidden()
                        .bg(colors.goto_list)
                        .border_1()
                        .border_color(colors.goto_hairline)
                        .child(
                            at(0.0, 0.0, m::GOTO_WIDTH, m::GOTO_HEAD_HEIGHT).bg(colors.goto_head),
                        )
                        .child(
                            at(
                                m::GOTO_FIELD_X,
                                m::GOTO_FIELD_Y,
                                m::GOTO_FIELD_WIDTH,
                                m::GOTO_FIELD_HEIGHT,
                            )
                            .child(
                                TextField::new(&sheet.input)
                                    .appearance(false)
                                    .text_size(rmac_ui::text_px(15.0)),
                            ),
                        )
                        .child(
                            at(
                                m::GOTO_CLOSE_X,
                                m::GOTO_CLOSE_Y,
                                m::GOTO_CLOSE_SIZE,
                                m::GOTO_CLOSE_SIZE,
                            )
                            .id("goto-close")
                            .rounded_full()
                            .bg(colors.goto_close)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(label("✕", 10.0, colors.goto_head))
                            .on_click(cx.listener(
                                |this, _: &ClickEvent, window, cx| this.cancel(window, cx),
                            )),
                        )
                        .child(
                            at(0.0, m::GOTO_HEAD_HEIGHT - 1.0, m::GOTO_WIDTH, 1.0)
                                .bg(colors.goto_hairline),
                        )
                        .child(
                            at(
                                0.0,
                                m::GOTO_HEAD_HEIGHT,
                                m::GOTO_WIDTH,
                                m::GOTO_HEIGHT - m::GOTO_HEAD_HEIGHT,
                            )
                            .id("goto-list")
                            .overflow_y_scroll()
                            .child(list),
                        ),
                )
                .into_any_element(),
        )
    }

    fn render_replace(
        &self,
        width: f32,
        height: f32,
        colors: Colors,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let target = self.replace.as_ref()?;
        let name = target
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let card_width = (width - 32.0).min(300.0);
        let card_height = 104.0f32.min(height - 8.0);
        Some(
            div()
                .absolute()
                .inset_0()
                .bg(gpui::black().opacity(0.1))
                .child(
                    at(
                        (width - card_width) / 2.0,
                        ((height - card_height) / 2.0).max(4.0),
                        card_width,
                        card_height,
                    )
                    .id("replace")
                    .occlude()
                    .rounded(px(m::SAVE_RADIUS / 2.0))
                    .bg(colors.sheet)
                    .border_1()
                    .border_color(colors.hairline)
                    .p(px(14.0))
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(13.0))
                            .font_weight(rmac_ui::mac::BOLD)
                            .text_color(colors.text)
                            .child(format!(
                                "“{name}” already exists. Do you want to replace it?"
                            )),
                    )
                    .child(
                        div()
                            .mt_auto()
                            .flex()
                            .justify_end()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .id("replace-no")
                                    .relative()
                                    .w(px(m::BUTTON_WIDTH))
                                    .h(px(m::CONTROL_HEIGHT))
                                    .child(
                                        plate(colors.default)
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .child(label("Cancel", 13.0, colors.on_selection)),
                                    )
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.dismiss_replace(cx)
                                    })),
                            )
                            .child(
                                div()
                                    .id("replace-yes")
                                    .relative()
                                    .w(px(m::BUTTON_WIDTH))
                                    .h(px(m::CONTROL_HEIGHT))
                                    .child(
                                        plate(colors.control)
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .child(label("Replace", 13.0, colors.text)),
                                    )
                                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                        this.confirm_replace(window, cx)
                                    })),
                            ),
                    ),
                )
                .into_any_element(),
        )
    }

    fn render_browser(
        &self,
        width: f32,
        height: f32,
        colors: Colors,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut elements = vec![self.render_sidebar(height, colors, cx)];
        let top = self.toolbar_top();
        if self.mode() == Mode::Save {
            let (header, _) = self.save_header(m::EXPANDED_LABEL_X, m::WHERE_X, colors, window, cx);
            elements.extend(header);
        }
        elements.extend(self.render_toolbar(top, width, colors, window, cx));
        let content_top = top + m::TOOLBAR_TO_HAIRLINE + 0.5;
        let content_bottom = height - m::BOTTOM_HAIRLINE;
        let content_width = width - m::CONTENT_LEFT;
        elements.push(
            at(
                m::CONTENT_LEFT,
                top + m::TOOLBAR_TO_HAIRLINE,
                content_width,
                0.5,
            )
            .bg(colors.hairline)
            .into_any_element(),
        );
        let body = match self.browser.view {
            ViewMode::Icons => self.render_icons(content_width, colors, cx),
            ViewMode::List => self.render_list(content_width, colors, cx),
        };
        elements.push(
            at(
                m::CONTENT_LEFT,
                content_top,
                content_width,
                (content_bottom - content_top).max(0.0),
            )
            .id("content")
            .overflow_y_scroll()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus, cx);
                }),
            )
            .child(body)
            .when_some(self.notice.clone(), |content, notice| {
                content.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(label(notice, 13.0, colors.label)),
                )
            })
            .into_any_element(),
        );
        elements.push(
            at(m::CONTENT_LEFT, content_bottom, content_width, 0.5)
                .bg(colors.hairline)
                .into_any_element(),
        );

        // Bottom row.
        let row_y = height - m::BOTTOM_ROW;
        let mut accessory_x = m::NAV_X;
        if self.mode() == Mode::Save {
            elements.push(
                self.button(
                    "new-folder",
                    "New Folder".into(),
                    m::NAV_X,
                    row_y,
                    m::NEW_FOLDER_WIDTH,
                    false,
                    true,
                    colors,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.new_folder(cx)))
                .into_any_element(),
            );
        } else {
            if self.request.filters.len() > 1 {
                elements.push(self.simple_popup(
                    "filter".into(),
                    accessory_x,
                    row_y,
                    200.0,
                    self.filter_name(),
                    MenuKind::Filter,
                    colors,
                    cx,
                ));
                accessory_x += 208.0;
            }
            for (index, choice) in self.choices.iter().enumerate() {
                if choice.is_checkbox() {
                    elements.push(self.checkbox(index, accessory_x, row_y, colors, cx));
                    accessory_x += 140.0;
                } else {
                    let title = choice
                        .options
                        .iter()
                        .find(|(key, _)| *key == choice.selected)
                        .map(|(_, label)| label.clone())
                        .unwrap_or_default();
                    elements.push(self.simple_popup(
                        SharedString::from(format!("choice-popup-{index}")),
                        accessory_x,
                        row_y,
                        150.0,
                        title.into(),
                        MenuKind::Choice(index),
                        colors,
                        cx,
                    ));
                    accessory_x += 158.0;
                }
            }
        }
        let right = width - m::PANEL_WIDTH;
        elements.push(
            self.button(
                "cancel",
                "Cancel".into(),
                m::CANCEL_X + right,
                row_y,
                m::BUTTON_WIDTH,
                false,
                true,
                colors,
            )
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.cancel(window, cx)))
            .into_any_element(),
        );
        let enabled = self.can_accept(cx);
        elements.push(
            self.button(
                "accept",
                self.request.accept_label().into(),
                m::DEFAULT_X + right,
                row_y,
                m::BUTTON_WIDTH,
                true,
                enabled,
                colors,
            )
            .when(enabled, |button| {
                button.on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| this.accept(window, cx)),
                )
            })
            .into_any_element(),
        );
        elements
    }
}

impl Render for Panel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Colors::current();
        let viewport = window.viewport_size();
        let width = f32::from(viewport.width);
        let height = f32::from(viewport.height);
        let content_width = width - m::CONTENT_LEFT;
        let background = if self.mode() == Mode::Save {
            colors.sheet
        } else {
            colors.panel
        };
        let elements = if self.is_compact() {
            self.render_compact(width, colors, window, cx)
        } else {
            self.render_browser(width, height, colors, window, cx)
        };
        let menu = self.render_menu(width, height, colors, cx);
        let goto = self.render_goto(width, height, colors, cx);
        let replace = self.render_replace(width, height, colors, cx);
        let home = self.home.clone();
        div()
            .id("file-chooser")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(background)
            .font_family(rmac_ui::UI_FONT)
            .text_size(rmac_ui::text_px(13.0))
            .text_color(colors.text)
            .on_action(cx.listener(|this, _: &Accept, window, cx| this.accept(window, cx)))
            .on_action(cx.listener(|this, _: &Cancel, window, cx| this.cancel(window, cx)))
            .on_action(
                cx.listener(|this, _: &rmac_ui::RequestClose, window, cx| this.cancel(window, cx)),
            )
            .on_action(cx.listener(|this, _: &GoToFolder, window, cx| this.open_goto(window, cx)))
            .on_action({
                let desktop = home.join("Desktop");
                cx.listener(move |this, _: &GoDesktop, _, cx| {
                    let target = if desktop.is_dir() {
                        desktop.clone()
                    } else {
                        this.home.clone()
                    };
                    this.navigate(Location::Folder(target), cx)
                })
            })
            .on_action(cx.listener(move |this, _: &GoHome, _, cx| {
                this.navigate(Location::Folder(home.clone()), cx)
            }))
            .on_action(cx.listener(|this, _: &GoEnclosing, _, cx| this.go_enclosing(cx)))
            .on_action(cx.listener(|this, _: &GoBack, _, cx| this.go_back(cx)))
            .on_action(cx.listener(|this, _: &GoForward, _, cx| this.go_forward(cx)))
            .on_action(
                cx.listener(|this, _: &ViewAsIcons, _, cx| this.set_view(ViewMode::Icons, cx)),
            )
            .on_action(cx.listener(|this, _: &ViewAsList, _, cx| this.set_view(ViewMode::List, cx)))
            .on_action(
                cx.listener(|this, _: &FocusSearch, window, cx| this.focus_search(window, cx)),
            )
            .on_action(cx.listener(|this, _: &NewFolder, _, cx| this.new_folder(cx)))
            .on_action(cx.listener(|this, _: &SelectAllItems, window, cx| {
                if this.focus.is_focused(window) {
                    let multiple = this.request.multiple;
                    this.browser.select_all(multiple);
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleHidden, _, cx| this.toggle_hidden(cx)))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if this.goto.is_some() {
                    match event.keystroke.key.as_str() {
                        "down" => this.goto_move(1, cx),
                        "up" => this.goto_move(-1, cx),
                        _ => return,
                    }
                    cx.stop_propagation();
                    return;
                }
                if this.list_key(event, content_width, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .children(elements)
            .children(menu)
            .children(goto)
            .children(replace)
    }
}
