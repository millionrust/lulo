use super::*;
use gpui_component::tooltip::Tooltip;

/// One Tahoe toolbar capsule (design-lab/finder.html): 36 tall, full radius,
/// a faint glass fill and a light edge around 36 × 36 buttons.
fn capsule(id: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(CAPSULE_HEIGHT))
        .flex_none()
        .flex()
        .items_center()
        .px(px(CAPSULE_PADDING))
        .rounded(px(CAPSULE_HEIGHT / 2.0))
        .bg(capsule_fill())
        .border_1()
        .border_color(capsule_edge())
}

/// A 36 × 36 capsule button. The selected state is the 34 × 28 pill the view
/// control draws behind its current mode.
fn capsule_button(
    id: &'static str,
    glyph: &'static str,
    glyph_size: f32,
    tooltip: &'static str,
    selected: bool,
    enabled: bool,
) -> Stateful<Div> {
    let colour = if enabled {
        toolbar_glyph()
    } else {
        toolbar_glyph().opacity(0.35)
    };
    div()
        .id(id)
        .w(px(CAPSULE_BUTTON))
        .h(px(CAPSULE_BUTTON))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .when(enabled, |button| button.cursor_pointer())
        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
        .child(
            div()
                .w(px(CAPSULE_PILL_WIDTH))
                .h(px(CAPSULE_PILL_HEIGHT))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(CAPSULE_PILL_HEIGHT / 2.0))
                .when(selected, |pill| pill.bg(capsule_selected()))
                .child(icon(glyph, glyph_size, colour)),
        )
}

/// The 1 × 20 rule between two unselected neighbours in a capsule; beside
/// the selected pill it is hidden, as in Finder.
fn capsule_divider(visible: bool) -> Div {
    div()
        .w(px(1.0))
        .h(px(CAPSULE_DIVIDER_HEIGHT))
        .mx(px(-0.5))
        .flex_none()
        .when(visible, |rule| rule.bg(capsule_divider()))
}

impl FinderView {
    pub(in crate::view) fn render_toolbar(
        &self,
        layout: crate::view::responsive_layout::ResponsiveLayout,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let can_go_back = self.trash_view || self.applications_view || !self.back.is_empty();
        let can_go_forward = !self.fwd.is_empty();
        let navigation_control = capsule("navigation")
            .child(
                capsule_button(
                    "back",
                    "icons/chevron-left.svg",
                    TOOLBAR_CHEVRON_GLYPH,
                    "Back",
                    false,
                    can_go_back,
                )
                .on_click(cx.listener(|this, _, _, cx| this.go_back(cx))),
            )
            .child(capsule_divider(true))
            .child(
                capsule_button(
                    "fwd",
                    "icons/chevron-right.svg",
                    TOOLBAR_CHEVRON_GLYPH,
                    "Forward",
                    false,
                    can_go_forward,
                )
                .on_click(cx.listener(|this, _, _, cx| this.go_forward(cx))),
            );

        let modes = [
            ("v-icon", "icons/layout-grid.svg", "Icon", ViewMode::Icon),
            ("v-list", "icons/list.svg", "List", ViewMode::List),
            ("v-col", "icons/columns-3.svg", "Columns", ViewMode::Column),
            ("v-gal", "icons/gallery.svg", "Gallery", ViewMode::Gallery),
        ];
        let mut view_control = capsule("view-control");
        for (index, (id, glyph, tooltip, mode)) in modes.into_iter().enumerate() {
            if index > 0 {
                let previous = modes[index - 1].3;
                view_control =
                    view_control.child(capsule_divider(self.view != mode && self.view != previous));
            }
            view_control = view_control.child(
                capsule_button(id, glyph, TOOLBAR_GLYPH, tooltip, self.view == mode, true)
                    .on_click(cx.listener(move |this, _, _, cx| this.select_view_mode(mode, cx))),
            );
        }

        // Finder's "Group" pop-up. Files has no grouping, so it offers the
        // Sort By choices that the pop-up also carries.
        let sort_control = capsule("sort")
            .w(px(GROUP_CAPSULE_WIDTH))
            .justify_center()
            .gap(px(1.0))
            .cursor_pointer()
            .tooltip(|window, cx| Tooltip::new("Sort By").build(window, cx))
            .child(icon("icons/group.svg", TOOLBAR_GLYPH, toolbar_glyph()))
            .child(icon("icons/chevron-down.svg", 12.0, toolbar_glyph()))
            .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                this.menu_purpose = MenuPurpose::Sort;
                this.menu_at = Some(rmac_ui::ContextMenuState::open(
                    event.position(),
                    &this.focus,
                    window,
                    cx,
                ));
                cx.notify();
            }));

        // Finder's action capsule also holds Share and Tags; Files has no
        // backend for either, so only the Action (…) menu is shown.
        let action_control = capsule("actions").child(
            capsule_button(
                "more",
                "icons/ellipsis.svg",
                TOOLBAR_GLYPH,
                "Action",
                false,
                true,
            )
            .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                this.menu_purpose = MenuPurpose::Context;
                this.menu_at = Some(rmac_ui::ContextMenuState::open(
                    event.position(),
                    &this.focus,
                    window,
                    cx,
                ));
                cx.notify();
            })),
        );

        // Search is a 36 pt circle that opens into a field, as in Finder; it
        // stays open while it holds a query.
        let search_open = self.search_open
            || !self.query.read(cx).value().is_empty()
            || self.search_summary.is_some();
        let search: gpui::AnyElement = if search_open {
            capsule("search")
                .w(px(layout.search_width))
                .gap(px(6.0))
                .pl(px(10.0))
                .pr(px(10.0))
                .child(icon("icons/search.svg", 14.0, chrome_text()))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(SearchField::new(&self.query).appearance(false).small()),
                )
                .into_any_element()
        } else {
            capsule("search")
                .child(
                    capsule_button(
                        "search-button",
                        "icons/search.svg",
                        18.0,
                        "Search",
                        false,
                        true,
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.search_open = true;
                        this.query.update(cx, |state, cx| state.focus(window, cx));
                        cx.notify();
                    })),
                )
                .into_any_element()
        };

        // Without the sidebar the traffic lights move into the toolbar at the
        // same window-relative centres.
        let leading = (!layout.sidebar_visible).then(|| {
            div()
                .h_full()
                .flex_none()
                .flex()
                .items_center()
                .pl(px(rmac_ui::traffic_lights_origin(true)))
                .child(rmac_ui::traffic_lights())
        });

        div()
            .id("toolbar")
            .h(px(TOOLBAR_HEIGHT))
            .flex_none()
            .w_full()
            .flex()
            .items_center()
            .pl(px(TOOLBAR_LEADING_GAP))
            .pr(px(TRAILING_MARGIN))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|t, _, _, _| t.dragging = true),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|t, _, _, _| t.dragging = false),
            )
            .on_mouse_move(cx.listener(|t, _, window, _| {
                if t.dragging {
                    t.dragging = false;
                    window.start_window_move();
                }
            }))
            .when_some(leading, |toolbar, leading| {
                toolbar.child(leading.mr(px(TOOLBAR_LEADING_GAP)))
            })
            .child(navigation_control)
            .when(layout.title_visible, |toolbar| {
                toolbar.child(
                    div()
                        .ml(px(TITLE_GAP))
                        .min_w(px(0.0))
                        .truncate()
                        .text_size(rmac_ui::text_px(TITLE_SIZE))
                        .font_weight(rmac_ui::mac::BOLD)
                        .text_color(toolbar_glyph())
                        .child(self.title()),
                )
            })
            .child(div().flex_1())
            .when(layout.view_control_visible, |toolbar| {
                toolbar
                    .child(view_control)
                    .child(div().w(px(VIEW_TO_GROUP_GAP)).flex_none())
                    .child(sort_control)
                    .child(div().w(px(TRAILING_GAP)).flex_none())
            })
            .child(action_control)
            .child(div().w(px(TRAILING_GAP)).flex_none())
            .child(search)
    }

    pub(in crate::view) fn title(&self) -> SharedString {
        if let Some(rt) = &self.result_title {
            return rt.clone();
        }
        // Column view titles the window after the deepest selected folder.
        if self.view == ViewMode::Column && !self.applications_view && !self.trash_view {
            if let Some(name) = self
                .col_stack
                .last()
                .filter(|path| **path != self.cwd)
                .and_then(|path| path.file_name())
            {
                return name.to_string_lossy().into_owned().into();
            }
        }
        self.cwd
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| root_volume_name().to_string())
            .into()
    }
}
