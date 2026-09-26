use gpui::{
    accesskit, div, prelude::FluentBuilder as _, px, AccessibleAction, Context, Entity,
    InteractiveElement as _, IntoElement, ParentElement, Render, Role, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled, Window,
};
use gpui_component::{Icon, IconName, StyledExt as _};
use rmac_ui::{mac, Button, SearchField, Table};
use sysinfo::Pid;

use crate::columns::ColKey;
use crate::metrics::{format_bytes, format_duration, format_mem, format_rate, Tab};
use crate::{
    process_action, CancelKill, ConfirmKill, FocusSearch, ForceQuitProcess, Minimize, QuitProcess,
};

use super::MonitorView;

/// Measured Activity Monitor table row (and header row) height.
const TABLE_ROW_HEIGHT: f32 = 24.0;

mod chrome;
mod metrics_panes;
mod overlays;

impl Render for MonitorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if window.is_window_active() {
            self.publish_menu_state(cx);
        }
        let layout =
            super::responsive_layout::toolbar_layout(f32::from(window.bounds().size.width));
        let persistence_error = self.persistence_error.clone();
        let process_feedback = self.process_action_feedback.clone();
        div()
            .track_focus(&self.focus)
            .key_context("ActivityMonitor")
            .on_action(cx.listener(|this, _: &QuitProcess, window, cx| {
                this.request_kill(false, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ForceQuitProcess, window, cx| {
                this.request_kill(true, window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                this.focus_search(window, cx);
            }))
            .on_action(cx.listener(|this, _: &ConfirmKill, _, cx| {
                this.confirm_kill(cx);
            }))
            .on_action(cx.listener(|this, _: &CancelKill, _, cx| {
                this.cancel_kill(cx);
            }))
            .on_action(cx.listener(|_, _: &rmac_ui::RequestClose, window, _| {
                window.remove_window();
            }))
            .on_action(cx.listener(|_, _: &Minimize, _, cx| {
                rmac_ui::minimize_focused_window(cx);
            }))
            .size_full()
            .v_flex()
            .font_features(mac::tabular_font_features())
            .bg(mac::window())
            .text_color(mac::text())
            .child(self.render_toolbar(layout, window, cx))
            .when_some(persistence_error, |monitor, message| {
                monitor.child(
                    div()
                        .id("persistence-error")
                        .h(px(34.0))
                        .flex_none()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .bg(mac::error_background())
                        .border_b_1()
                        .border_color(mac::error_border())
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(mac::danger())
                        .child(div().flex_1().child(message))
                        .child(
                            Button::new("dismiss-persistence-error", "Dismiss")
                                .ghost()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.persistence_error = None;
                                    cx.notify();
                                })),
                        ),
                )
            })
            .when_some(process_feedback, |monitor, feedback| {
                let (background, border, text) = if feedback.success {
                    (mac::accent_subtle(), mac::accent_border(), mac::text())
                } else {
                    (mac::error_background(), mac::error_border(), mac::danger())
                };
                monitor.child(
                    div()
                        .id("process-action-feedback")
                        .role(Role::Alert)
                        .aria_label(SharedString::from(format!(
                            "{}. {}",
                            feedback.title, feedback.detail
                        )))
                        .min_h(px(52.0))
                        .flex_none()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_2()
                        .bg(background)
                        .border_b_1()
                        .border_color(border)
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(text)
                        .child(
                            div()
                                .v_flex()
                                .flex_1()
                                .gap_1()
                                .child(div().font_weight(mac::SEMIBOLD).child(feedback.title))
                                .child(feedback.detail),
                        )
                        .child(
                            Button::new("dismiss-process-feedback", "Dismiss")
                                .ghost()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.process_action_feedback = None;
                                    cx.notify();
                                })),
                        ),
                )
            })
            .when(self.tab.has_process_table(), |monitor| {
                // Activity Monitor's table: full-width 24 pt rows with every
                // other row striped, a 5 pt gap under the header. The table's
                // own role/name live on this wrapper; `rmac_ui::Table` marks
                // its own row group and publishes every off-screen row to
                // assistive technology (see docs/accessibility-audit.md),
                // and each painted row/header cell's role lives in
                // `process_table.rs`.
                let row_count = self.table.read(cx).delegate().rows.len();
                let column_count = self.table.read(cx).delegate().visible.len();
                let query = self.search.read(cx).value().to_string();
                // A search that matches nothing says so, instead of leaving
                // an empty table that reads as a machine with no processes.
                if row_count == 0 && !query.is_empty() {
                    return monitor.child(
                        div()
                            .id("process-table")
                            .flex_1()
                            .min_h(px(0.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(rmac_ui::EmptyState::new("No Matching Processes").message(
                                format!("No process name, PID or path contains “{query}”."),
                            )),
                    );
                }
                monitor.child(
                    div()
                        .id("process-table")
                        .role(Role::Table)
                        .aria_label("Processes")
                        .aria_row_count(row_count)
                        .aria_column_count(column_count)
                        .flex_1()
                        .min_h(px(0.0))
                        .text_size(rmac_ui::text_px(13.0))
                        .child(
                            Table::new(&self.table)
                                .stripe(true)
                                .bordered(false)
                                .row_height(TABLE_ROW_HEIGHT),
                        ),
                )
            })
            .when(!self.tab.has_process_table(), |monitor| {
                monitor.child(self.render_network_pane())
            })
            .child(self.render_bottom_panel(cx))
            .when(
                self.cols_menu_open && self.tab.has_process_table(),
                |monitor| monitor.child(self.render_columns_menu(layout, cx)),
            )
            .when(self.filter_menu_open, |monitor| {
                monitor.child(self.render_filter_menu(cx))
            })
            .children(self.render_confirm(cx))
            .children(self.render_inspector(cx))
    }
}
