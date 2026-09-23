//! Terminal title bar, tab bar, and profile-picker projection.
//!
//! Measured on macOS 26.2 (design-lab/apps.html): the title bar shows a
//! folder proxy icon at x 81 and "<title> — <cols>×<rows>" in 13 pt bold
//! secondary text at x 100. With two or more tabs a 36 pt bar follows: a
//! 28 pt capsule track from x 8 to 7 pt short of a 28 pt "+" circle whose
//! right edge sits 8 from the window, then a 7 pt gap and a 1 pt base line.
//! Tabs share the track equally; the active one is a 24 pt pill inset 2.

use super::*;

/// Left edge of the proxy icon's 18 pt frame, from the window edge.
const PROXY_ICON_X: f32 = 81.0;
const PROXY_ICON_FRAME: f32 = 18.0;
/// `rmac_ui::title_bar_content` insets its content 12 from the window edge.
const TITLE_BAR_CONTENT_INSET: f32 = 12.0;
const TAB_TRACK_HEIGHT: f32 = 28.0;
const TAB_TRACK_INSET: f32 = 8.0;
const TAB_TRACK_GAP: f32 = 7.0;
const ACTIVE_TAB_INSET: f32 = 2.0;

/// White at `alpha`, laid over the tinted title bar the way the Mac's tab
/// track (5 %) and active tab (17 %) are.
fn veil(alpha: f32) -> Hsla {
    gpui::hsla(0.0, 0.0, 1.0, alpha)
}

/// The working-directory folder drawn at the Mac's proxy-icon size.
fn folder_proxy_icon() -> impl IntoElement {
    let blue: Hsla = gpui::rgb(0x6fb6f2).into();
    div()
        .w(px(PROXY_ICON_FRAME))
        .h(px(PROXY_ICON_FRAME))
        .flex_none()
        .flex()
        .flex_col()
        .justify_center()
        .px(px(1.0))
        .child(div().w(px(7.0)).h(px(2.0)).rounded_t(px(1.5)).bg(blue))
        .child(div().w_full().h(px(12.0)).rounded(px(2.0)).bg(blue))
}

impl TerminalView {
    /// The title-bar row: proxy icon, then the tab title and grid size the
    /// way Terminal titles a window ("jake — -zsh — 80×24").
    pub(super) fn render_title(&self, title: String, max_width: f32) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .pl(px(PROXY_ICON_X - TITLE_BAR_CONTENT_INSET))
            .child(folder_proxy_icon())
            .child(
                div()
                    .pl(px(1.0))
                    .max_w(px(max_width))
                    .truncate()
                    .text_size(rmac_ui::text_px(13.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rmac_ui::mac::text_secondary())
                    .child(format!("{title} — {}×{}", self.cols, self.rows)),
            )
    }

    pub(super) fn render_tabs(
        &self,
        tab_title_max_width: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let tab_count = self.tabs.len();
        let active_tab = self.active;
        let mut track = div()
            .id("terminal-tabs")
            .flex_1()
            .min_w_0()
            .h(px(TAB_TRACK_HEIGHT))
            .flex()
            .items_center()
            .rounded(px(TAB_TRACK_HEIGHT / 2.0))
            .bg(veil(0.05));
        for index in 0..tab_count {
            let is_active = index == active_tab;
            let title = self.tabs[index]
                .tab_title()
                .unwrap_or_else(|| format!("Terminal {}", index + 1));
            let label = self.tabs[index]
                .tab_state_label()
                .map_or(title.clone(), |state| format!("{title} — {state}"));
            let group: SharedString = format!("terminal-tab-{index}").into();
            track = track.child(
                div()
                    .id(("tab", index))
                    .group(group.clone())
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .relative()
                    .when(is_active, |element| {
                        element
                            .m(px(ACTIVE_TAB_INSET))
                            .h(px(TAB_TRACK_HEIGHT - 2.0 * ACTIVE_TAB_INSET))
                            .rounded(px(TAB_TRACK_HEIGHT / 2.0 - ACTIVE_TAB_INSET))
                            .bg(veil(0.17))
                            .border_t_1()
                            .border_b_1()
                            .border_color(veil(0.14))
                    })
                    .child(
                        div()
                            .max_w(px(tab_title_max_width))
                            .truncate()
                            .text_size(rmac_ui::text_px(13.0))
                            .font_weight(if is_active {
                                FontWeight::BOLD
                            } else {
                                FontWeight::NORMAL
                            })
                            .text_color(if is_active {
                                rmac_ui::mac::text()
                            } else {
                                rmac_ui::mac::text_tertiary()
                            })
                            .child(label),
                    )
                    .child(
                        // Terminal reveals a tab's close button on hover.
                        div()
                            .id(("tabclose", index))
                            .absolute()
                            .left(px(6.0))
                            .top_0()
                            .bottom_0()
                            .w(px(16.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .opacity(0.0)
                            .group_hover(group, |style| style.opacity(1.0))
                            .text_size(rmac_ui::text_px(13.0))
                            .text_color(rmac_ui::mac::text_secondary())
                            .child("×")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                // Keep the click from also selecting the tab.
                                cx.stop_propagation();
                                this.request_close_tab(index, window, cx);
                            })),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select_tab(index, window, cx);
                    })),
            );
        }
        div()
            .h(px(TAB_BAR_HEIGHT))
            .flex_none()
            .bg(rmac_ui::mac::chrome())
            .border_b_1()
            .border_color(rmac_ui::mac::separator())
            .child(
                div()
                    .h(px(TAB_TRACK_HEIGHT))
                    .px(px(TAB_TRACK_INSET))
                    .flex()
                    .items_center()
                    .gap(px(TAB_TRACK_GAP))
                    .child(track)
                    .child(
                        div()
                            .id("newtab")
                            .flex_none()
                            .size(px(TAB_TRACK_HEIGHT))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .border_1()
                            .border_color(veil(0.14))
                            .hover(|hovered| hovered.bg(veil(0.08)))
                            .text_size(rmac_ui::text_px(18.0))
                            .text_color(rmac_ui::mac::text_secondary())
                            .child("+")
                            .on_click(cx.listener(|this, _, window, cx| this.new_tab(window, cx))),
                    ),
            )
    }

    pub(super) fn render_picker(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active_index = self.profile;
        div()
            .absolute()
            .top(px(TITLE_BAR_HEIGHT + 2.0))
            .right_2()
            .w(px(190.0))
            .bg(rmac_ui::mac::window())
            .rounded(px(rmac_ui::mac::radius_control()))
            .border_1()
            .border_color(rmac_ui::mac::separator())
            .shadow_lg()
            .py_1()
            .children(PROFILES.iter().enumerate().map(|(index, _)| {
                let profile = profiles::resolved(index);
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
                            .rounded(px(rmac_ui::mac::radius_menu_item()))
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
