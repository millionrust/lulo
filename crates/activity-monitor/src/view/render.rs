use gpui::{
    div, prelude::FluentBuilder as _, px, Context, InteractiveElement as _, IntoElement,
    ParentElement, Render, SharedString, Stateful, StatefulInteractiveElement as _, Styled, Window,
};
use gpui_component::StyledExt as _;
use rmac_ui::{mac, Button, SearchField, Table, Tabs};
use sysinfo::Pid;

use crate::columns::ColKey;
use crate::metrics::{format_bytes, format_duration, format_mem, format_rate, Tab};
use crate::{process_action, CancelKill, ConfirmKill, FocusSearch, ForceQuitProcess, QuitProcess};

use super::MonitorView;

impl MonitorView {
    /// The column-chooser dropdown: a checklist of every available column.
    fn render_columns_menu(&self, cx: &Context<Self>) -> impl IntoElement {
        let visible = self.table.read(cx).delegate().visible.clone();
        let top =
            96.0 + if self.persistence_error.is_some() {
                34.0
            } else {
                0.0
            } + if self.process_action_feedback.is_some() {
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
            .rounded(px(8.0))
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

    fn stat_card(&self, label: &str, value: String, accent: gpui::Hsla) -> impl IntoElement {
        div()
            .v_flex()
            .gap_1()
            .px_4()
            .py_3()
            .min_w(px(150.0))
            .rounded(px(10.0))
            .bg(mac::chrome())
            .border_1()
            .border_color(mac::separator())
            .child(
                div()
                    .text_size(rmac_ui::text_px(11.0))
                    .font_weight(mac::SEMIBOLD)
                    .text_color(mac::text_tertiary())
                    .child(label.to_uppercase()),
            )
            .child(
                div()
                    .text_size(rmac_ui::text_px(24.0))
                    .font_weight(mac::SEMIBOLD)
                    .text_color(accent)
                    .child(value),
            )
    }

    fn sparkline(&self, samples: &[f32], accent: gpui::Hsla) -> impl IntoElement {
        let max = samples.iter().cloned().fold(1.0f32, f32::max);
        let bars: Vec<_> = samples
            .iter()
            .map(|&value| {
                let fraction = (value / max).clamp(0.02, 1.0);
                div()
                    .flex_1()
                    .h(px(40.0 * fraction))
                    .min_w(px(2.0))
                    .rounded(px(1.0))
                    .bg(accent)
            })
            .collect();
        div()
            .h(px(44.0))
            .w_full()
            .flex()
            .items_end()
            .gap(px(1.0))
            .px_1()
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap(px(1.0))
                    .size_full()
                    .children(bars),
            )
    }

    fn render_core_bars(&self) -> impl IntoElement {
        let blue = gpui::rgb(0x007aff);
        let cores = self.sampler.aggregates.per_core.clone();
        div()
            .v_flex()
            .gap_2()
            .pt_3()
            .child(
                div()
                    .text_size(rmac_ui::text_px(11.0))
                    .font_weight(mac::SEMIBOLD)
                    .text_color(mac::text_tertiary())
                    .child("CPU CORES"),
            )
            .child(div().flex().flex_wrap().gap_x_4().gap_y_2().children(
                cores.into_iter().enumerate().map(|(index, usage)| {
                    let fraction = (usage / 100.0).clamp(0.0, 1.0);
                    div()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .w(px(160.0))
                        .child(
                            div()
                                .w(px(48.0))
                                .text_size(rmac_ui::text_px(11.0))
                                .text_color(mac::text_secondary())
                                .child(format!("Core {}", index + 1)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .h(px(6.0))
                                .rounded(px(3.0))
                                .bg(mac::chrome())
                                .child(
                                    div()
                                        .h_full()
                                        .w(gpui::relative(fraction))
                                        .rounded(px(3.0))
                                        .bg(blue),
                                ),
                        )
                        .child(
                            div()
                                .w(px(34.0))
                                .text_size(rmac_ui::text_px(11.0))
                                .text_color(mac::text())
                                .child(format!("{usage:.0}%")),
                        )
                }),
            ))
    }

    /// Network is system-wide because the current authority has no reliable
    /// per-process network accounting.
    fn render_network_pane(&self) -> impl IntoElement {
        let teal = gpui::rgb(0x32ade6);
        let figure = |value: String, color: gpui::Hsla| {
            div()
                .w(px(110.0))
                .text_size(rmac_ui::text_px(12.0))
                .text_color(color)
                .text_right()
                .child(value)
        };

        let header = div()
            .h_flex()
            .items_center()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(mac::separator())
            .child(
                div()
                    .flex_1()
                    .text_size(rmac_ui::text_px(11.0))
                    .font_weight(mac::SEMIBOLD)
                    .text_color(mac::text_tertiary())
                    .child("INTERFACE"),
            )
            .children(
                ["RCVD", "SENT", "↓ RATE", "↑ RATE"]
                    .into_iter()
                    .map(|label| {
                        div()
                            .w(px(110.0))
                            .text_size(rmac_ui::text_px(11.0))
                            .font_weight(mac::SEMIBOLD)
                            .text_color(mac::text_tertiary())
                            .text_right()
                            .child(label)
                    }),
            );

        let rows: Vec<gpui::AnyElement> = self
            .sampler
            .interfaces
            .iter()
            .enumerate()
            .map(|(index, interface)| {
                let active = interface.recv_rate + interface.sent_rate > 0.0;
                div()
                    .h_flex()
                    .items_center()
                    .px_3()
                    .py_1p5()
                    .when(index % 2 == 1, |element| element.bg(mac::hover()))
                    .child(
                        div()
                            .flex_1()
                            .h_flex()
                            .items_center()
                            .gap_2()
                            .child(div().size(px(7.0)).rounded_full().bg(if active {
                                teal.into()
                            } else {
                                mac::text_tertiary()
                            }))
                            .child(
                                div()
                                    .text_size(rmac_ui::text_px(13.0))
                                    .text_color(mac::text())
                                    .child(interface.name.clone()),
                            ),
                    )
                    .child(figure(format_bytes(interface.total_recv), mac::text()))
                    .child(figure(format_bytes(interface.total_sent), mac::text()))
                    .child(figure(
                        format_rate(interface.recv_rate),
                        if active {
                            teal.into()
                        } else {
                            mac::text_secondary()
                        },
                    ))
                    .child(figure(
                        format_rate(interface.sent_rate),
                        if active {
                            teal.into()
                        } else {
                            mac::text_secondary()
                        },
                    ))
                    .into_any_element()
            })
            .collect();

        div().flex_1().min_h(px(0.0)).px_4().pb_4().child(
            div()
                .id("net-iface-table")
                .size_full()
                .min_h(px(0.0))
                .overflow_y_scroll()
                .border_1()
                .border_color(mac::separator())
                .rounded(px(8.0))
                .bg(mac::window())
                .child(header)
                .children(rows),
        )
    }

    fn render_summary(&self, cx: &Context<Self>) -> impl IntoElement {
        let blue = gpui::rgb(0x007aff).into();
        let green = gpui::rgb(0x28b463).into();
        let orange = gpui::rgb(0xff9500).into();
        let purple = gpui::rgb(0xaf52de).into();
        let teal = gpui::rgb(0x32ade6).into();

        let (cards, samples, accent): (Vec<gpui::AnyElement>, &[f32], gpui::Hsla) = match self.tab {
            Tab::Cpu => {
                let red = gpui::rgb(0xff3b30).into();
                let mut cards = vec![
                    self.stat_card(
                        "CPU Load",
                        format!("{:.1}%", self.sampler.aggregates.cpu_total),
                        blue,
                    )
                    .into_any_element(),
                    self.stat_card(
                        "Cores",
                        self.sampler.aggregates.per_core.len().to_string(),
                        mac::text(),
                    )
                    .into_any_element(),
                    self.stat_card(
                        "Processes",
                        self.table.read(cx).delegate().all_rows.len().to_string(),
                        mac::text(),
                    )
                    .into_any_element(),
                ];
                if let Some((user, system, idle)) = self.sampler.cpu_split {
                    cards.push(
                        self.stat_card("System", format!("{system:.1}%"), red)
                            .into_any_element(),
                    );
                    cards.push(
                        self.stat_card("User", format!("{user:.1}%"), blue)
                            .into_any_element(),
                    );
                    cards.push(
                        self.stat_card("Idle", format!("{idle:.1}%"), mac::text_secondary())
                            .into_any_element(),
                    );
                }
                (cards, self.sampler.history.cpu.as_slice(), blue)
            }
            Tab::Memory => {
                let cards = vec![
                    self.stat_card(
                        "Memory Used",
                        format!(
                            "{} / {}",
                            format_mem(self.sampler.aggregates.mem_used),
                            format_mem(self.sampler.aggregates.mem_total)
                        ),
                        green,
                    )
                    .into_any_element(),
                    self.stat_card(
                        "Available",
                        format_mem(self.sampler.aggregates.mem_available),
                        mac::text(),
                    )
                    .into_any_element(),
                    self.stat_card(
                        "Swap",
                        format!(
                            "{} / {}",
                            format_mem(self.sampler.aggregates.swap_used),
                            format_mem(self.sampler.aggregates.swap_total)
                        ),
                        mac::text(),
                    )
                    .into_any_element(),
                ];
                (cards, self.sampler.history.mem.as_slice(), green)
            }
            Tab::Energy => {
                let cards = vec![
                    self.stat_card(
                        "Energy Impact",
                        format!("{:.1}", self.sampler.aggregates.energy_total),
                        orange,
                    )
                    .into_any_element(),
                    self.stat_card(
                        "CPU Load",
                        format!("{:.1}%", self.sampler.aggregates.cpu_total),
                        mac::text(),
                    )
                    .into_any_element(),
                ];
                (cards, self.sampler.history.energy.as_slice(), orange)
            }
            Tab::Disk => {
                let cards = vec![
                    self.stat_card(
                        "Reads",
                        format_rate(self.sampler.aggregates.disk_read_rate),
                        purple,
                    )
                    .into_any_element(),
                    self.stat_card(
                        "Writes",
                        format_rate(self.sampler.aggregates.disk_write_rate),
                        purple,
                    )
                    .into_any_element(),
                ];
                (cards, self.sampler.history.disk.as_slice(), purple)
            }
            Tab::Network => {
                let cards = vec![
                    self.stat_card(
                        "Receiving",
                        format_rate(self.sampler.aggregates.net_recv_rate),
                        teal,
                    )
                    .into_any_element(),
                    self.stat_card(
                        "Sending",
                        format_rate(self.sampler.aggregates.net_sent_rate),
                        teal,
                    )
                    .into_any_element(),
                ];
                (cards, self.sampler.history.net.as_slice(), teal)
            }
        };

        let samples = samples.to_vec();
        div()
            .v_flex()
            .gap_3()
            .p_4()
            .border_b_1()
            .border_color(mac::separator())
            .child(div().h_flex().gap_3().children(cards))
            .child(self.sparkline(&samples, accent))
            .when(
                matches!(self.tab, Tab::Cpu) && !self.sampler.aggregates.per_core.is_empty(),
                |element| element.child(self.render_core_bars()),
            )
            .when(
                matches!(self.tab, Tab::Memory) && self.sampler.aggregates.mem_total > 0,
                |element| element.child(self.render_mem_pressure()),
            )
    }

    fn render_mem_pressure(&self) -> impl IntoElement {
        let fraction = (self.sampler.aggregates.mem_used as f32
            / self.sampler.aggregates.mem_total as f32)
            .clamp(0.0, 1.0);
        let (color, label): (gpui::Hsla, &str) = if fraction < 0.60 {
            (gpui::rgb(0x28b463).into(), "Normal")
        } else if fraction < 0.80 {
            (gpui::rgb(0xff9500).into(), "Elevated")
        } else {
            (gpui::rgb(0xff3b30).into(), "High")
        };
        div()
            .v_flex()
            .gap_2()
            .pt_1()
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(11.0))
                            .font_weight(mac::SEMIBOLD)
                            .text_color(mac::text_tertiary())
                            .child("MEMORY PRESSURE"),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(11.0))
                            .font_weight(mac::SEMIBOLD)
                            .text_color(color)
                            .child(label),
                    ),
            )
            .child(
                div()
                    .h(px(10.0))
                    .w_full()
                    .rounded(px(5.0))
                    .bg(mac::chrome())
                    .child(
                        div()
                            .h_full()
                            .w(gpui::relative(fraction))
                            .rounded(px(5.0))
                            .bg(color),
                    ),
            )
    }

    fn render_toolbar(&self, cx: &Context<Self>) -> impl IntoElement {
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
            .child(
                div()
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
                            .w(px(220.0))
                            .child(SearchField::new(&self.search).small()),
                    ),
            )
    }

    fn render_confirm(&self, cx: &Context<Self>) -> Option<impl IntoElement> {
        use rmac_ui::DialogButtonKind::{Destructive, Normal, Primary};

        let request = self.pending_kill.clone()?;
        let verb = request.kind.label();
        let body = format!(
            "Do you want to {} the process \u{201c}{}\u{201d} (PID {})?",
            verb.to_lowercase(),
            request.process.name,
            request.process.pid
        );
        let confirm_kind = if request.kind == process_action::ActionKind::ForceQuit {
            Destructive
        } else {
            Primary
        };
        Some(rmac_ui::alert(
            format!("{verb} Process"),
            body,
            vec![
                rmac_ui::dialog_button("kill-cancel", "Cancel", Normal)
                    .on_click(cx.listener(|this, _, _, cx| this.cancel_kill(cx)))
                    .into_any_element(),
                rmac_ui::dialog_button("kill-confirm", verb, confirm_kind)
                    .on_click(cx.listener(|this, _, _, cx| this.confirm_kill(cx)))
                    .into_any_element(),
            ],
        ))
    }

    fn render_inspector(&self, cx: &Context<Self>) -> Option<impl IntoElement> {
        let pid = self.inspect_pid?;
        let state = self.table.read(cx);
        let delegate = state.delegate();
        let row = delegate
            .rows
            .iter()
            .chain(delegate.all_rows.iter())
            .find(|row| row.pid == pid)?;
        let path = delegate
            .system
            .process(Pid::from_u32(pid))
            .and_then(|process| {
                process
                    .exe()
                    .map(|path| path.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "—".into());

        let info_row = |label: &str, value: String| {
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .gap_4()
                .py_1p5()
                .border_b_1()
                .border_color(mac::separator())
                .child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(mac::text_secondary())
                        .child(label.to_string()),
                )
                .child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .font_weight(mac::MEDIUM)
                        .text_color(mac::text())
                        .child(value),
                )
        };

        let dialog = div()
            .v_flex()
            .gap_1()
            .w(px(420.0))
            .p_5()
            .rounded(px(12.0))
            .bg(mac::window())
            .border_1()
            .border_color(mac::separator())
            .shadow_lg()
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .justify_between()
                    .pb_2()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(16.0))
                            .font_weight(mac::SEMIBOLD)
                            .text_color(mac::text())
                            .child(row.name.clone()),
                    )
                    .child(Button::new("inspect-close", "Done").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.inspect_pid = None;
                            cx.notify();
                        },
                    ))),
            )
            .child(info_row("Process ID (PID)", row.pid.to_string()))
            .child(info_row(
                "Parent PID",
                row.ppid
                    .map(|parent| parent.to_string())
                    .unwrap_or_else(|| "—".into()),
            ))
            .child(info_row("User", row.user.to_string()))
            .child(info_row("Status", row.status.to_string()))
            .child(info_row("% CPU", format!("{:.1}", row.cpu)))
            .child(info_row("Memory", format_mem(row.mem)))
            .child(info_row("Virtual Memory", format_mem(row.vmem)))
            .child(info_row("Disk I/O", format_mem(row.disk)))
            .child(info_row("Run Time", format_duration(row.run_time)))
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .pt_2()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(mac::text_secondary())
                            .child("Path"),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(11.0))
                            .text_color(mac::text())
                            .child(path),
                    ),
            );

        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(mac::scrim())
                .child(dialog),
        )
    }
}

impl Render for MonitorView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let persistence_error = self.persistence_error.clone();
        let process_feedback = self.process_action_feedback.clone();
        div()
            .track_focus(&self.focus)
            .key_context("ActivityMonitor")
            .on_action(cx.listener(|this, _: &QuitProcess, _, cx| {
                this.request_kill(false, cx);
            }))
            .on_action(cx.listener(|this, _: &ForceQuitProcess, _, cx| {
                this.request_kill(true, cx);
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
            .size_full()
            .v_flex()
            .bg(mac::window())
            .text_color(mac::text())
            .child(rmac_ui::title_bar("System Monitor"))
            .child(self.render_toolbar(cx))
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
                        .cursor_pointer()
                        .child(div().flex_1().child(message))
                        .child("Dismiss")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.persistence_error = None;
                            cx.notify();
                        })),
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
                        .cursor_pointer()
                        .child(
                            div()
                                .v_flex()
                                .flex_1()
                                .gap_1()
                                .child(div().font_weight(mac::SEMIBOLD).child(feedback.title))
                                .child(feedback.detail),
                        )
                        .child("Dismiss")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.process_action_feedback = None;
                            cx.notify();
                        })),
                )
            })
            .child(self.render_summary(cx))
            .when(self.tab.has_process_table(), |monitor| {
                monitor.child(
                    div()
                        .flex_1()
                        .px_4()
                        .pb_4()
                        .child(Table::new(&self.table).stripe(true).bordered(true)),
                )
            })
            .when(!self.tab.has_process_table(), |monitor| {
                monitor.child(self.render_network_pane())
            })
            .when(
                self.cols_menu_open && self.tab.has_process_table(),
                |monitor| monitor.child(self.render_columns_menu(cx)),
            )
            .children(self.render_confirm(cx))
            .children(self.render_inspector(cx))
    }
}
