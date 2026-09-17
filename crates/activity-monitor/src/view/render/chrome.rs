//! System Monitor toolbar and column chooser presentation.

use super::*;
use crate::view::responsive_layout::ToolbarLayout;

impl MonitorView {
    pub(super) fn render_columns_menu(
        &self,
        layout: ToolbarLayout,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let visible = self.table.read(cx).delegate().visible.clone();
        let top = layout.columns_menu_top
            + if self.persistence_error.is_some() {
                34.0
            } else {
                0.0
            }
            + if self.process_action_feedback.is_some() {
                52.0
            } else {
                0.0
            };
        div()
            .absolute()
            .top(px(top))
            .right(px(16.0))
            .w(px(210.0))
            .bg(mac::window())
            .rounded(px(mac::radius_control()))
            .border_1()
            .border_color(mac::separator())
            .shadow_lg()
            .py_1()
            .child(
                div()
                    .px_3()
                    .py_1()
                    .text_size(rmac_ui::text_px(11.0))
                    .font_weight(mac::SEMIBOLD)
                    .text_color(mac::text_tertiary())
                    .child("COLUMNS"),
            )
            .children(ColKey::ALL.into_iter().map(|key| {
                let on = visible.contains(&key);
                let disabled = key.required();
                div()
                    .id(SharedString::from(key.id()))
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .h(px(26.0))
                    .px_3()
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(if disabled {
                        mac::text_tertiary()
                    } else {
                        mac::text()
                    })
                    .when(!disabled, |element: Stateful<gpui::Div>| {
                        element
                            .hover(|hover| hover.bg(mac::chrome()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_column(key, cx);
                            }))
                    })
                    .child(div().w(px(14.0)).text_color(mac::accent()).child(if on {
                        "✓"
                    } else {
                        ""
                    }))
                    .child(div().flex_1().child(key.title()))
            }))
    }
    pub(super) fn render_toolbar(
        &self,
        layout: ToolbarLayout,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        let selected = Tab::ALL
            .iter()
            .position(|tab| *tab == self.tab)
            .unwrap_or(0);
        let tabs = Tabs::new("activity-tabs", Tab::ALL.map(Tab::label))
            .selected(selected)
            .on_change(cx.listener(|this, index: &usize, _, cx| {
                this.select_tab(Tab::ALL[*index], cx);
            }));
        let has_selection = self.selected_proc(cx).is_some();
        let actions = div()
            .h_flex()
            .items_center()
            .gap_2()
            .child(
                Button::new("quit", "Quit")
                    .disabled(!has_selection)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.request_kill(false, cx);
                    })),
            )
            .child(
                Button::new("force-quit", "Force Quit")
                    .destructive()
                    .disabled(!has_selection)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.request_kill(true, cx);
                    })),
            )
            .when(self.tab.has_process_table(), |element| {
                element.child(
                    Button::new("columns", "Columns")
                        .selected(self.cols_menu_open)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.cols_menu_open = !this.cols_menu_open;
                            cx.notify();
                        })),
                )
            })
            .child(
                div()
                    .w(px(layout.search_width))
                    .child(SearchField::new(&self.search).small()),
            );

        if layout.compact {
            div()
                .v_flex()
                .gap_2()
                .px_4()
                .py_2()
                .border_b_1()
                .border_color(mac::separator())
                .child(div().w_full().flex().justify_center().child(tabs))
                .child(div().w_full().flex().justify_end().child(actions))
                .into_any_element()
        } else {
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .gap_3()
                .px_4()
                .py_2()
                .border_b_1()
                .border_color(mac::separator())
                .child(tabs)
                .child(actions)
                .into_any_element()
        }
    }
}
