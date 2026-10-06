//! Terminal Find, context-menu, session, hyperlink, and error overlays.

use super::*;

impl TerminalView {
    pub(super) fn render_find_panel(
        &self,
        find_width: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let status = self.find_status_label();
        let view = cx.entity();
        div()
            .absolute()
            .top(px(40.0))
            .right(px(12.0))
            .w(px(find_width))
            .h(px(30.0))
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .rounded(px(rmac_ui::mac::radius_segmented()))
            .bg(rmac_ui::mac::raised())
            .key_context("TerminalFind")
            .on_action(cx.listener(|this, _: &FindNext, _, cx| this.find_step(true, cx)))
            .on_action(cx.listener(|this, _: &FindPrevious, _, cx| this.find_step(false, cx)))
            .child(
                div()
                    .id("terminal-find-field")
                    .role(Role::SearchInput)
                    .aria_label("Find")
                    .accessible_text_input(&self.search, cx)
                    .flex_1()
                    .child(SearchField::new(&self.search).appearance(false)),
            )
            .when_some(status, |panel, status| {
                panel.child(
                    div()
                        .flex_none()
                        .text_xs()
                        .text_color(rmac_ui::mac::text_secondary())
                        .child(status),
                )
            })
            .child(
                div()
                    .id("find-previous")
                    .role(Role::Button)
                    .aria_label("Previous Match")
                    .text_color(rmac_ui::mac::text_secondary())
                    .child("‹")
                    .on_click(cx.listener(|this, _, _, cx| this.find_step(false, cx))),
            )
            .child(
                div()
                    .id("find-next")
                    .role(Role::Button)
                    .aria_label("Next Match")
                    .text_color(rmac_ui::mac::text_secondary())
                    .child("›")
                    .on_click(cx.listener(|this, _, _, cx| this.find_step(true, cx))),
            )
            .child(
                div()
                    .id("find-close")
                    .role(Role::Button)
                    .aria_label("Close Find")
                    .text_color(rmac_ui::mac::text_secondary())
                    .child("×")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.close_find(window, cx);
                    }))
                    .on_a11y_action(AccessibleAction::Click, move |_, window, cx| {
                        view.update(cx, |this, cx| this.close_find(window, cx));
                    }),
            )
    }

    fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.capture_active_search_query(cx);
        self.tabs[self.active].ui.search_open = false;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub(super) fn render_context_menu(&self, state: rmac_ui::ContextMenuState) -> impl IntoElement {
        rmac_ui::ContextMenu::new(state.position())
            .command_item("Copy", rmac_ui::shortcuts::COPY, Box::new(Copy))
            .command_item("Paste", rmac_ui::shortcuts::PASTE, Box::new(Paste))
            .command_item(
                "Select All",
                rmac_ui::shortcuts::SELECT_ALL,
                Box::new(SelectAll),
            )
            .separator()
            .command_item(
                "Previous Prompt",
                rmac_ui::shortcuts::PREVIOUS_MARK,
                Box::new(PreviousPrompt),
            )
            .command_item(
                "Next Prompt",
                rmac_ui::shortcuts::NEXT_MARK,
                Box::new(NextPrompt),
            )
            .item("Select Command", Box::new(SelectCommand))
            .command_item(
                "Select Command Output",
                rmac_ui::shortcuts::SELECT_COMMAND_OUTPUT,
                Box::new(SelectCommandOutput),
            )
            .separator()
            .command_item("Clear", rmac_ui::shortcuts::CLEAR, Box::new(Clear))
            .separator()
            .item("Profiles…", Box::new(ShowProfiles))
            .render(&state)
    }

    pub(super) fn render_session_status(
        &self,
        message: SharedString,
        has_terminal_error: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id("session-status")
            .absolute()
            .left(px(8.0))
            .right(px(8.0))
            .bottom(px(if has_terminal_error { 54.0 } else { 8.0 }))
            .h_flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .rounded(px(rmac_ui::mac::radius_segmented()))
            .bg(rmac_ui::mac::raised())
            .text_size(rmac_ui::text_px(12.0))
            .text_color(rmac_ui::mac::text())
            .shadow_lg()
            .child(div().flex_1().child(message))
            .child(
                Button::new("new-tab-after-exit", "New Tab")
                    .small()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.new_tab(window, cx);
                    })),
            )
    }

    pub(super) fn render_hyperlink_status(
        &self,
        message: SharedString,
        has_terminal_error: bool,
    ) -> impl IntoElement {
        div()
            .id("hyperlink-status")
            .absolute()
            .left(px(8.0))
            .bottom(px(if has_terminal_error { 54.0 } else { 8.0 }))
            .max_w(px(460.0))
            .px_3()
            .py_2()
            .rounded(px(rmac_ui::mac::radius_segmented()))
            .bg(rmac_ui::mac::raised())
            .text_size(rmac_ui::text_px(12.0))
            .text_color(rmac_ui::mac::text_secondary())
            .shadow_lg()
            .truncate()
            .child(message)
    }

    pub(super) fn render_terminal_error(
        &self,
        message: SharedString,
        operation_error_visible: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id("terminal-error")
            .absolute()
            .left(px(8.0))
            .right(px(8.0))
            .bottom(px(8.0))
            .h_flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .rounded(px(rmac_ui::mac::radius_segmented()))
            .bg(rmac_ui::mac::danger())
            .text_size(rmac_ui::text_px(12.0))
            .text_color(rmac_ui::mac::on_danger())
            .shadow_lg()
            .child(div().flex_1().child(message))
            .child(
                Button::new("dismiss-terminal-error", "Dismiss")
                    .ghost()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if operation_error_visible {
                            this.operation_error = None;
                        } else {
                            this.persistence_error = None;
                        }
                        cx.notify();
                    })),
            )
    }

    /// View ▸ Show All Tabs (⇧⌘\): an Exposé-style grid of this window's
    /// tabs, like Preview/Text Editor's own. A terminal tab has no page
    /// image to thumbnail, so each tile shows its title, state and colour
    /// profile swatch instead — enough to tell tabs apart at a glance.
    pub(super) fn render_all_tabs(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if !self.show_all_tabs || self.tabs.len() < 2 {
            return None;
        }
        const TILE_W: f32 = 160.0;
        const TILE_H: f32 = 110.0;
        Some(
            div()
                .id("terminal-all-tabs")
                .role(Role::Group)
                .aria_label("All Tabs")
                .absolute()
                .inset_0()
                .flex()
                .flex_wrap()
                .items_start()
                .content_start()
                .gap(px(16.0))
                .p(px(32.0))
                .bg(rmac_ui::mac::scrim())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.show_all_tabs = false;
                    cx.notify();
                }))
                .children((0..self.tabs.len()).map(|index| {
                    let title = self.tabs[index]
                        .tab_title()
                        .unwrap_or_else(|| format!("Terminal {}", index + 1));
                    let state = self.tabs[index].tab_state_label();
                    let swatch = profiles::resolved(self.tab_profiles[index]).bg;
                    let is_active = index == self.active;
                    div()
                        .id(("terminal-all-tabs-tile", index))
                        .w(px(TILE_W))
                        .h(px(TILE_H))
                        .v_flex()
                        .gap(px(4.0))
                        .p(px(6.0))
                        .rounded(px(rmac_ui::mac::radius_card()))
                        .when(is_active, |tile| tile.bg(hsla(active().selection)))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.select_tab(index, window, cx);
                            this.show_all_tabs = false;
                            cx.notify();
                        }))
                        .child(
                            div()
                                .flex_1()
                                .rounded(px(rmac_ui::mac::radius_control()))
                                .bg(hsla(swatch))
                                .border_1()
                                .border_color(rmac_ui::mac::separator()),
                        )
                        .child(
                            div()
                                .text_size(rmac_ui::text_px(11.0))
                                .text_color(rmac_ui::mac::white())
                                .truncate()
                                .child(title),
                        )
                        .when_some(state, |tile, state| {
                            tile.child(
                                div()
                                    .text_size(rmac_ui::text_px(10.0))
                                    .text_color(rmac_ui::mac::text_secondary())
                                    .child(state),
                            )
                        })
                })),
        )
    }
}
