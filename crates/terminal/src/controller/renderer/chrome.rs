//! Terminal title-bar, tab-strip, and profile-picker projection.

use super::*;

impl TerminalView {
    pub(super) fn render_tabs(
        &self,
        tab_title_max_width: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let tab_count = self.tabs.len();
        let active_tab = self.active;
        let history_limit = scrollback_limit_for_tab_count(tab_count);
        let mut bar = div()
            .id("terminal-tabs")
            .h(px(32.0))
            .flex_none()
            .flex()
            .items_center()
            .overflow_x_scroll()
            .px_2()
            .gap_1()
            .bg(rmac_ui::mac::chrome())
            .border_b_1()
            .border_color(rmac_ui::mac::separator());
        for index in 0..tab_count {
            let is_active = index == active_tab;
            let title = self.tabs[index]
                .tab_title()
                .unwrap_or_else(|| format!("Terminal {}", index + 1));
            let label = self.tabs[index]
                .tab_state_label()
                .map_or(title.clone(), |state| format!("{title} — {state}"));
            bar = bar.child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_1()
                    .h(px(26.0))
                    .px_2()
                    .rounded(px(5.0))
                    .when(is_active, |element: Div| element.bg(hsla(active().bg)))
                    .child(
                        div()
                            .id(("tabname", index))
                            .max_w(px(tab_title_max_width))
                            .truncate()
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(if is_active {
                                hsla(active().fg)
                            } else {
                                rmac_ui::mac::text_secondary()
                            })
                            .child(label)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_tab(index, window, cx);
                            })),
                    )
                    .child(
                        div()
                            .id(("tabclose", index))
                            .text_size(rmac_ui::text_px(13.0))
                            .text_color(rmac_ui::mac::text_secondary())
                            .child("×")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.request_close_tab(index, window, cx);
                            })),
                    ),
            );
        }
        bar.child(div().flex_1())
            .child(
                div()
                    .flex_none()
                    .px_1()
                    .text_size(rmac_ui::text_px(10.0))
                    .text_color(rmac_ui::mac::text_secondary())
                    .child(format!("{history_limit} history lines/tab")),
            )
            .child(
                div()
                    .id("newtab")
                    .flex_none()
                    .w(px(22.0))
                    .h(px(26.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.0))
                    .text_size(rmac_ui::text_px(15.0))
                    .text_color(rmac_ui::mac::text_secondary())
                    .child("+")
                    .on_click(cx.listener(|this, _, window, cx| this.new_tab(window, cx))),
            )
    }

    /// The profile chip in the toolbar — shows the active scheme; click to
    /// open the picker (matching Terminal.app's profile switcher).
    pub(super) fn profile_chip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let name = PROFILES[self.profile].name;
        Button::new("profile-chip", format!("{name}  ▼"))
            .ghost()
            .small()
            .selected(self.picker_open)
            .on_click(cx.listener(|this, _, _, cx| {
                this.picker_open = !this.picker_open;
                cx.notify();
            }))
    }

    pub(super) fn render_picker(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active_index = self.profile;
        div()
            .absolute()
            .top(px(34.0))
            .right_2()
            .w(px(190.0))
            .bg(rmac_ui::mac::window())
            .rounded(px(8.0))
            .border_1()
            .border_color(rmac_ui::mac::separator())
            .shadow_lg()
            .py_1()
            .children(PROFILES.iter().enumerate().map(|(index, profile)| {
                let is_active = index == active_index;
                div()
                    .id(("profrow", index))
                    .flex()
                    .items_center()
                    .gap_2()
                    .h(px(26.0))
                    .px_2()
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(rmac_ui::mac::text())
                    .hover(|hovered| {
                        hovered
                            .bg(rmac_ui::mac::accent())
                            .text_color(rmac_ui::mac::on_accent())
                    })
                    .child(
                        div()
                            .w(px(14.0))
                            .h(px(14.0))
                            .rounded(px(3.0))
                            .border_1()
                            .border_color(rmac_ui::mac::separator())
                            .bg(hsla(profile.bg)),
                    )
                    .child(div().flex_1().child(profile.name))
                    .when(is_active, |element: Stateful<Div>| {
                        element.child(div().text_color(rmac_ui::mac::accent()).child("✓"))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.set_profile(index, cx)))
            }))
    }
}
