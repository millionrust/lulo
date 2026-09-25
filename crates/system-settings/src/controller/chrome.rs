//! Window chrome and category-sidebar rendering, measured from macOS 26
//! System Settings (design-lab/settings.html).

use super::*;
use gpui::{Stateful, WindowControlArea};
use gpui_component::InteractiveElementExt as _;

/// A region that moves the window when dragged and zooms it on a
/// double-click, like the Mac's toolbar and sidebar header.
fn drag_region(id: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .window_control_area(WindowControlArea::Drag)
        .on_double_click(|_, window, _| window.zoom_window())
}

impl Settings {
    fn sidebar_toggle(
        &self,
        layout: crate::responsive_layout::SettingsLayout,
        cx: &Context<Self>,
    ) -> Button {
        Button::new(rmac_system_settings::accessibility::SIDEBAR_TOGGLE_ID, "")
            .icon(
                Icon::new(if layout.sidebar_visible {
                    IconName::PanelLeftClose
                } else {
                    IconName::PanelLeftOpen
                })
                .text_color(style::toolbar_glyph(true)),
            )
            .ghost()
            .xsmall()
            .tooltip(if layout.sidebar_visible {
                rmac_system_settings::accessibility::HIDE_SIDEBAR_NAME
            } else {
                rmac_system_settings::accessibility::SHOW_SIDEBAR_NAME
            })
            .on_click(cx.listener(|this, _, _, cx| this.toggle_compact_sidebar(cx)))
    }

    /// The toolbar over the detail column: the back/forward capsule and the
    /// pane title. General (and any pane with a hero) shows no title, as on
    /// the Mac.
    pub(super) fn render_toolbar(
        &self,
        layout: crate::responsive_layout::SettingsLayout,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let can_back = self.can_go_back();
        let can_forward = self.can_go_forward();
        let segment = |id: &'static str, path: &'static str, enabled: bool| {
            div()
                .id(id)
                .w(px(style::CAPSULE_SEGMENT))
                .h(px(style::CAPSULE_SEGMENT))
                .flex()
                .items_center()
                .justify_center()
                .when(enabled, |segment| segment.cursor_pointer())
                .child(glyph(
                    path,
                    style::CAPSULE_GLYPH,
                    style::toolbar_glyph(enabled),
                ))
        };
        let capsule = div()
            .h(px(style::CAPSULE_HEIGHT))
            .flex_none()
            .flex()
            .items_center()
            .rounded(px(style::CAPSULE_HEIGHT / 2.0))
            .bg(style::capsule_fill())
            .border_1()
            .border_color(style::capsule_edge())
            .child(
                segment(
                    rmac_system_settings::accessibility::BACK_ID,
                    "icons/chevron-left.svg",
                    can_back,
                )
                .when(can_back, |back| {
                    back.on_click(cx.listener(|this, _, window, cx| this.go_back(window, cx)))
                }),
            )
            .child(
                div()
                    .w(px(1.0))
                    .h(px(style::CAPSULE_DIVIDER_HEIGHT))
                    .bg(style::capsule_divider()),
            )
            .child(
                segment("nav-forward", "icons/chevron-right.svg", can_forward)
                    .when(can_forward, |forward| {
                        forward.on_click(cx.listener(|this, _, _, cx| this.go_forward(cx)))
                    }),
            );
        let title = self.toolbar_title();
        let subtitle = self.toolbar_subtitle();

        drag_region("topbar")
            .h(px(style::TOOLBAR_HEIGHT))
            .flex_none()
            .w_full()
            .flex()
            .items_center()
            .pl(px(style::CAPSULE_LEADING))
            // With the sidebar hidden the lights and its toggle move here.
            .when(!layout.sidebar_visible, |bar| {
                bar.pl(px(rmac_ui::traffic_lights_origin(true)))
                    .gap_2()
                    .child(rmac_ui::traffic_lights())
                    .child(self.sidebar_toggle(layout, cx))
            })
            .child(capsule)
            .when_some(title, |bar, title| {
                // A subtitle turns the title into the two-line form (13 pt
                // bold over 11 pt secondary), as Battery shows its level.
                let two_line = subtitle.is_some();
                bar.child(
                    div()
                        .ml(px(style::TITLE_GAP))
                        .min_w_0()
                        .v_flex()
                        .child(
                            div()
                                .truncate()
                                .text_size(rmac_ui::text_px(if two_line {
                                    13.0
                                } else {
                                    style::TITLE_SIZE
                                }))
                                .font_weight(rmac_ui::mac::BOLD)
                                .text_color(style::title_text())
                                .child(title),
                        )
                        .when_some(subtitle, |block, subtitle| {
                            block.child(
                                div()
                                    .truncate()
                                    .text_size(rmac_ui::text_px(11.0))
                                    .text_color(secondary())
                                    .child(subtitle),
                            )
                        }),
                )
            })
    }

    fn toolbar_subtitle(&self) -> Option<SharedString> {
        (self.nav.is_empty() && self.current().name.as_ref() == "Battery")
            .then(|| self.battery_toolbar_subtitle())
            .flatten()
    }

    fn toolbar_title(&self) -> Option<SharedString> {
        match self.nav.last() {
            Some(subpage) => Some(self.subpage_title(subpage).into()),
            None if self.pane_has_hero() => None,
            None => Some(self.current().name.clone()),
        }
    }

    /// The floating sidebar panel: traffic lights, search, account and the
    /// category list (design-lab/settings.html).
    pub(super) fn render_sidebar(
        &self,
        layout: crate::responsive_layout::SettingsLayout,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let compact = layout.compact;
        let search = div()
            .id(rmac_system_settings::accessibility::SEARCH_ID)
            .role(Role::SearchInput)
            .aria_label("Search")
            .accessible_text_input(&self.search, cx)
            .mx(px(style::SIDEBAR_ROW_INSET))
            .h(px(style::SEARCH_HEIGHT))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(5.0))
            .pl(px(8.0))
            .pr(px(6.0))
            .rounded(px(style::SEARCH_HEIGHT / 2.0))
            .bg(style::search_fill())
            .child(glyph(
                "icons/search.svg",
                style::SEARCH_GLYPH,
                style::search_glyph(),
            ))
            .child(
                div()
                    .flex_1()
                    .child(SearchField::new(&self.search).appearance(false)),
            );
        let query = self.search.read(cx).value().to_string();
        let searching = !query.trim().is_empty();
        let search_matches = self.search_matches(cx);
        let no_search_results = searching && search_matches.is_empty();
        let active_search_result = self
            .search_selection
            .min(search_matches.len().saturating_sub(1));
        let highlight = if self.sidebar_focused && window.is_window_active() {
            style::sidebar_selection_focused()
        } else {
            style::sidebar_selection()
        };
        let highlight_text = if self.sidebar_focused && window.is_window_active() {
            white()
        } else {
            style::sidebar_text()
        };

        let account = div()
            .h(px(style::ACCOUNT_ROW_HEIGHT))
            .flex_none()
            .relative()
            .flex()
            .items_center()
            .child(
                div()
                    .absolute()
                    .left(px(style::SIDEBAR_ICON_X))
                    .top(px((style::ACCOUNT_ROW_HEIGHT - style::ACCOUNT_AVATAR) / 2.0))
                    .size(px(style::ACCOUNT_AVATAR))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(rmac_ui::mac::system_gray())
                    .child(glyph("icons/user.svg", 22.0, white())),
            )
            .child(
                div()
                    .absolute()
                    .left(px(style::ACCOUNT_TEXT_X))
                    .top(px(7.0))
                    .v_flex()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(13.0))
                            .line_height(px(16.0))
                            .font_weight(rmac_ui::mac::BOLD)
                            .text_color(style::sidebar_text())
                            .child(self.account.clone()),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(11.0))
                            .line_height(px(14.0))
                            .text_color(secondary())
                            .child(rmac_system_settings::accessibility::LOCAL_ACCOUNT_LABEL),
                    ),
            );

        let mut list = div()
            .id(rmac_system_settings::accessibility::SIDEBAR_ID)
            .flex_1()
            .min_h(px(0.0))
            .v_flex()
            .pt(px(style::LIST_TOP_GAP))
            .px(px(style::SIDEBAR_ROW_INSET))
            .pb(px(style::SIDEBAR_ROW_INSET))
            .overflow_y_scroll()
            // Up/Down move the highlighted category while a sidebar row has
            // focus (Tab into the list, then arrow like the Mac's own
            // System Settings sidebar). The search field's own Up/Down
            // handling lives on the root capture_key_down and stops
            // propagation before it reaches here.
            .capture_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                if searching {
                    return;
                }
                let handled = match event.keystroke.key.as_str() {
                    "down" => this.move_category_selection(1, cx),
                    "up" => this.move_category_selection(-1, cx),
                    _ => false,
                };
                if handled {
                    cx.stop_propagation();
                }
            }))
            .child(account)
            .when(no_search_results, |sidebar| {
                sidebar.child(
                    div()
                        .mt_6()
                        .v_flex()
                        .items_center()
                        .gap_1()
                        .text_center()
                        .child(
                            div()
                                .text_size(rmac_ui::text_px(13.0))
                                .font_weight(rmac_ui::mac::SEMIBOLD)
                                .text_color(style::sidebar_text())
                                .child("No Settings Found"),
                        )
                        .child(
                            div()
                                .text_size(rmac_ui::text_px(11.0))
                                .text_color(secondary())
                                .child("Try a different search."),
                        ),
                )
            });

        // While a pane filed under General is open, General stays selected.
        let current = self.current().name.clone();
        let sidebar_owner = category_parent(current.as_ref())
            .map(SharedString::from)
            .unwrap_or(current);
        let mut search_result_index = 0;
        for (si, section) in self.sections.iter().enumerate() {
            let matching: Vec<(usize, &Category)> = section
                .iter()
                .enumerate()
                .filter(|(_, category)| {
                    if searching {
                        crate::settings_search::matches(category, &query)
                    } else {
                        category_parent(category.name.as_ref()).is_none()
                    }
                })
                .collect();
            if matching.is_empty() {
                continue;
            }
            list = list.child(div().h(px(style::SIDEBAR_SECTION_GAP)).flex_none());
            for (ci, cat) in matching {
                let search_context = searching.then(|| {
                    crate::settings_search::match_hint(cat, &query)
                        .map(SharedString::from)
                        .unwrap_or_else(|| cat.desc.clone())
                });
                let selected = if searching {
                    search_result_index == active_search_result
                } else {
                    cat.name == sidebar_owner
                };
                search_result_index += 1;
                let text = if selected {
                    highlight_text
                } else {
                    style::sidebar_text()
                };
                list = list.child(
                    ListRow::new(
                        SharedString::from(format!("cat-{si}-{ci}")),
                        div()
                            .flex()
                            .items_center()
                            .gap(px(style::SIDEBAR_LABEL_X
                                - style::SIDEBAR_ICON_X
                                - style::SIDEBAR_ICON))
                            .child(tile(cat.icon, cat.color, style::SIDEBAR_ICON))
                            .child(
                                div()
                                    .min_w_0()
                                    .v_flex()
                                    .child(
                                        div()
                                            .text_size(rmac_ui::text_px(13.0))
                                            .text_color(text)
                                            .truncate()
                                            .child(cat.name.clone()),
                                    )
                                    .when_some(search_context, |content, context| {
                                        content.child(
                                            div()
                                                .max_w(px(if compact {
                                                    480.0
                                                } else {
                                                    style::SIDEBAR_PANEL_WIDTH
                                                        - 2.0 * style::SIDEBAR_ROW_INSET
                                                        - style::SIDEBAR_LABEL_X
                                                        - 6.0
                                                }))
                                                .overflow_hidden()
                                                .text_size(rmac_ui::text_px(10.0))
                                                .text_color(if selected {
                                                    highlight_text
                                                } else {
                                                    secondary()
                                                })
                                                .child(context),
                                        )
                                    }),
                            ),
                    )
                    // The row paints its own Tahoe fill: accent only while the
                    // sidebar has focus, grey otherwise, and no hover wash.
                    .selected(true)
                    .bg(if selected {
                        highlight
                    } else {
                        gpui::transparent_black()
                    })
                    .rounded(px(style::SIDEBAR_ROW_RADIUS))
                    .pl(px(style::SIDEBAR_ICON_X))
                    .pr(px(6.0))
                    .flex_none()
                    .h(px(if searching {
                        42.0
                    } else {
                        style::SIDEBAR_ROW_HEIGHT
                    }))
                    .on_activate(cx.listener(move |t, _, window, cx| {
                        // `select_position` moves focus into the chosen
                        // pane's content itself (macOS's own sidebar
                        // behaviour), so no separate re-focus is needed
                        // here for either path.
                        t.select_position((si, ci), window, cx);
                        if searching {
                            t.clear_search(window, cx);
                        }
                    })),
                );
            }
        }

        let lights_origin = rmac_ui::traffic_lights_origin(true) - style::SIDEBAR_INSET;
        let panel = div()
            .relative()
            .size_full()
            .v_flex()
            .rounded(px(style::SIDEBAR_RADIUS))
            .bg(style::sidebar_panel())
            .border_1()
            .border_color(style::sidebar_panel_edge())
            .overflow_hidden()
            .child(
                drag_region("sidebar-header")
                    .h(px(style::SEARCH_TOP))
                    .flex_none()
                    .w_full(),
            )
            .child(search)
            .child(list)
            .child(
                div()
                    .absolute()
                    .left(px(lights_origin))
                    .top(px(lights_origin))
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(rmac_ui::traffic_lights())
                    .when(compact, |lights| {
                        lights.child(self.sidebar_toggle(layout, cx))
                    }),
            );

        div()
            .h_full()
            .flex_shrink_0()
            .when(compact, |column| column.flex_1())
            .when(!compact, |column| column.w(px(style::SIDEBAR_COLUMN_WIDTH)))
            .pl(px(style::SIDEBAR_INSET))
            .py(px(style::SIDEBAR_INSET))
            .when(compact, |column| column.pr(px(style::SIDEBAR_INSET)))
            .child(panel)
    }
}
