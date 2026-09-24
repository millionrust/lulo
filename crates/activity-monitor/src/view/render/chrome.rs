//! Activity Monitor's unified toolbar and column chooser.
//!
//! Measured on macOS 26.2 (design-lab/apps.html, 960 × 640 window): a 52 pt
//! toolbar with the lights centred 26 from the corner, a two-line title at
//! x 111 ("Activity Monitor" 13 bold over an 11 pt subtitle), a ⊗ ⓘ capsule
//! at x 258 (70 wide), a ⋯▾ capsule at x 340 (46 wide), the five-tab capsule
//! at x 403 (5 × 77) and a 38 pt search circle 8 from the right edge.

use super::*;
use crate::view::responsive_layout::ToolbarLayout;

/// `rmac_ui::toolbar` insets its content 12 from the window edge.
const TOOLBAR_CONTENT_INSET: f32 = 12.0;
const TITLE_X: f32 = 111.0;
const STOP_GROUP_X: f32 = 258.0;
/// One icon slot inside the ⊗ ⓘ capsule: two slots plus the capsule's 4 pt
/// side padding make the measured 70.
const ICON_SLOT: f32 = 31.0;
/// `rmac_ui::toolbar_group`'s horizontal padding on each side.
const GROUP_PADDING: f32 = 4.0;
const ACTIONS_WIDTH: f32 = 46.0;
const GROUP_GAP: f32 = 12.0;
const TABS_GAP: f32 = 17.0;
/// Five tabs plus the capsule padding span the measured 387.
const TAB_WIDTH: f32 = (387.0 - 2.0 * GROUP_PADDING) / 5.0;
const TAB_PILL_HEIGHT: f32 = 27.0;
const SEARCH_DIAMETER: f32 = 38.0;
const TOOLBAR_RIGHT_INSET: f32 = 8.0;

/// The raised pill of the selected tab: #444655 over the #212332 capsule.
fn selected_tab_fill() -> gpui::Hsla {
    gpui::hsla(0.0, 0.0, 1.0, 0.14)
}

/// Wrap an icon-only `rmac_ui::Button` with an outer accessible node.
///
/// The wrapped component library's `Button` only sets `aria_label` from
/// `.label()`, which would also draw visible text on a glyph-only control
/// (see docs/accessibility-audit.md's "Blocked" note on `Button`), so the
/// accessible name and AT-SPI Click action live on this wrapper instead. The
/// inner button keeps its own mouse click, hover, focus ring, and disabled
/// styling exactly as before; `activate` runs the identical state change a
/// mouse click already runs, so both input paths do the same thing.
fn accessible_icon_button(
    id: &'static str,
    name: &'static str,
    expanded: Option<bool>,
    button: impl IntoElement,
    view: Entity<MonitorView>,
    activate: fn(&mut MonitorView, &mut Context<MonitorView>),
) -> impl IntoElement {
    div()
        .id(SharedString::from(format!("{id}-a11y")))
        .role(Role::Button)
        .aria_label(name)
        .when_some(expanded, |element, expanded| {
            element.aria_expanded(expanded)
        })
        .on_a11y_action(AccessibleAction::Click, move |_data, _window, cx| {
            view.update(cx, |this, cx| activate(this, cx));
        })
        .child(button)
}

impl MonitorView {
    pub(super) fn render_columns_menu(
        &self,
        layout: ToolbarLayout,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let visible = self.table.read(cx).delegate().visible.clone();
        div()
            .absolute()
            .top(px(layout.columns_menu_top))
            .left(px(STOP_GROUP_X
                + 2.0 * (ICON_SLOT + GROUP_PADDING)
                + GROUP_GAP))
            .w(px(210.0))
            .bg(mac::material_popover())
            .rounded(px(mac::radius_menu()))
            .border_1()
            .border_color(mac::separator())
            .shadow_lg()
            .py_1()
            .child(
                div()
                    .px_3()
                    .py_1()
                    .text_size(rmac_ui::text_px(11.0))
                    .font_weight(mac::BOLD)
                    .text_color(mac::text_tertiary())
                    .child("Columns"),
            )
            .children(ColKey::ALL.into_iter().map(|key| {
                let on = visible.contains(&key);
                let disabled = key.required();
                div()
                    .id(SharedString::from(key.id()))
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .h(px(24.0))
                    .mx_1()
                    .px_2()
                    .rounded(px(mac::radius_menu_item()))
                    .text_size(rmac_ui::text_px(13.0))
                    .text_color(if disabled {
                        mac::text_tertiary()
                    } else {
                        mac::text()
                    })
                    .when(!disabled, |element: Stateful<gpui::Div>| {
                        element
                            .hover(|hover| hover.bg(mac::accent()).text_color(mac::on_accent()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_column(key, cx);
                            }))
                    })
                    .child(div().w(px(14.0)).child(if on { "✓" } else { "" }))
                    .child(div().flex_1().child(key.title()))
            }))
    }

    fn toolbar_icon(
        &self,
        id: &'static str,
        icon: IconName,
        tooltip: &'static str,
        enabled: bool,
    ) -> Button {
        Button::new(id, "")
            .icon(Icon::new(icon).text_color(if enabled {
                mac::text()
            } else {
                mac::text_tertiary()
            }))
            .ghost()
            .disabled(!enabled)
            .tooltip(tooltip)
    }

    pub(super) fn render_toolbar(
        &self,
        layout: ToolbarLayout,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let has_selection = self.selected_proc(cx).is_some();
        let view = cx.entity();
        let tabs = Tab::ALL.into_iter().map(|tab| {
            let selected = tab == self.tab;
            div()
                .id(SharedString::from(format!("tab-{}", tab.label())))
                .role(Role::Tab)
                .aria_label(tab.label())
                .aria_selected(selected)
                .w(px(TAB_WIDTH))
                .h(px(TAB_PILL_HEIGHT))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(TAB_PILL_HEIGHT / 2.0))
                .text_size(rmac_ui::text_px(13.0))
                .text_color(mac::text())
                .when(selected, |element| element.bg(selected_tab_fill()))
                .when(!selected, |element| {
                    element.hover(|hover| hover.bg(mac::hover()))
                })
                .child(tab.label())
                .on_click(cx.listener(move |this, _, _, cx| this.select_tab(tab, cx)))
        });
        let search_open = self.search_open || !self.search.read(cx).value().is_empty();
        let search = if search_open {
            let query = self.search.read(cx).value().to_string();
            let view = cx.entity();
            div()
                .w(px(layout.search_width))
                .flex_none()
                .child(rmac_ui::toolbar_group(
                    div()
                        .id("monitor-search")
                        .role(Role::TextInput)
                        .aria_label("Search")
                        .aria_value(SharedString::from(query))
                        .w_full()
                        .child(SearchField::new(&self.search).appearance(false).small())
                        .on_a11y_action(AccessibleAction::SetValue, {
                            let view = view.clone();
                            move |data, window, cx| {
                                let Some(accesskit::ActionData::Value(text)) = data else {
                                    return;
                                };
                                let text = text.to_string();
                                view.update(cx, |this, cx| {
                                    this.set_search_from_assistive_technology(text, window, cx);
                                });
                            }
                        })
                        .on_a11y_action(AccessibleAction::ReplaceSelectedText, {
                            move |data, window, cx| {
                                let Some(accesskit::ActionData::Value(text)) = data else {
                                    return;
                                };
                                let text = text.to_string();
                                view.update(cx, |this, cx| {
                                    this.set_search_from_assistive_technology(text, window, cx);
                                });
                            }
                        }),
                ))
                .into_any_element()
        } else {
            div()
                .id("search-toggle")
                .role(Role::Button)
                .aria_label("Search")
                .size(px(SEARCH_DIAMETER))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(mac::material_clear())
                .border_1()
                .border_color(mac::separator())
                .child(Icon::new(IconName::Search).text_color(mac::text()))
                .on_click(cx.listener(|this, _, window, cx| this.focus_search(window, cx)))
                .into_any_element()
        };

        let row = div()
            .size_full()
            .flex()
            .items_center()
            .pl(px(TITLE_X - TOOLBAR_CONTENT_INSET))
            .pr(px(TOOLBAR_RIGHT_INSET))
            // Narrow windows give the title's room to the controls, as the
            // Mac's toolbar drops its title before its items.
            .when(!layout.compact, |row| {
                row.child(
                    div()
                        .w(px(STOP_GROUP_X - TITLE_X))
                        .min_w_0()
                        .v_flex()
                        .justify_center()
                        .child(
                            div()
                                .truncate()
                                .text_size(rmac_ui::text_px(13.0))
                                .font_weight(mac::BOLD)
                                .text_color(mac::text())
                                .child("System Monitor"),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(rmac_ui::text_px(11.0))
                                .text_color(mac::text_secondary())
                                .child("All Processes"),
                        ),
                )
            })
            .child(rmac_ui::toolbar_group(
                div()
                    .flex()
                    .items_center()
                    .child(
                        div().w(px(ICON_SLOT)).flex().justify_center().child(
                            accessible_icon_button(
                                "stop",
                                "Quit Process",
                                None,
                                self.toolbar_icon(
                                    "stop",
                                    IconName::CircleX,
                                    "Quit Process",
                                    has_selection,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.request_kill(false, cx);
                                    },
                                )),
                                view.clone(),
                                |this, cx| this.request_kill(false, cx),
                            ),
                        ),
                    )
                    .child(
                        div().w(px(ICON_SLOT)).flex().justify_center().child(
                            accessible_icon_button(
                                "inspect",
                                "Inspect Process",
                                None,
                                self.toolbar_icon(
                                    "inspect",
                                    IconName::Info,
                                    "Inspect Process",
                                    has_selection,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.inspect_selected(cx);
                                    },
                                )),
                                view.clone(),
                                |this, cx| this.inspect_selected(cx),
                            ),
                        ),
                    ),
            ))
            .child(
                div().pl(px(GROUP_GAP)).child(rmac_ui::toolbar_group(
                    div()
                        .w(px(ACTIONS_WIDTH - 2.0 * GROUP_PADDING))
                        .flex()
                        .justify_center()
                        .child(accessible_icon_button(
                            "columns",
                            "Columns",
                            Some(self.cols_menu_open),
                            Button::new("columns", "")
                                .icon(Icon::new(IconName::Ellipsis).text_color(mac::text()))
                                .ghost()
                                .selected(self.cols_menu_open)
                                .disabled(!self.tab.has_process_table())
                                .tooltip("Columns")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.toggle_columns_menu(cx);
                                })),
                            view.clone(),
                            |this, cx| this.toggle_columns_menu(cx),
                        )),
                )),
            )
            .child(div().pl(px(TABS_GAP)).child(rmac_ui::toolbar_group(
                div().flex().items_center().children(tabs),
            )))
            .child(div().flex_1().min_w(px(8.0)))
            .child(search);
        rmac_ui::toolbar(row)
    }
}
