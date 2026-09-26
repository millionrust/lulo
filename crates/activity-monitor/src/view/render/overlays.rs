//! Process confirmation and inspection overlays.

use super::*;

impl MonitorView {
    pub(super) fn render_confirm(&self, cx: &Context<Self>) -> Option<gpui::AnyElement> {
        use rmac_ui::DialogButtonKind::{Destructive, Normal, Primary};

        let request = self.pending_kill.current()?.clone();
        if request.kind == process_action::ActionKind::Quit {
            // Activity Monitor's ⊗ asks once and offers both signals:
            // Force Quit · Cancel · Quit.
            return Some(
                rmac_ui::alert(
                    "Are you sure you want to quit this process?",
                    format!(
                        "Do you really want to quit \u{201c}{}\u{201d} (PID {})?",
                        request.process.name, request.process.pid
                    ),
                    vec![
                        rmac_ui::dialog_button("kill-force", "Force Quit", Destructive)
                            .on_click(cx.listener(|this, _, _, cx| this.confirm_force_quit(cx)))
                            .into_any_element(),
                        rmac_ui::dialog_button("kill-cancel", "Cancel", Normal)
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_kill(cx)))
                            .into_any_element(),
                        rmac_ui::dialog_button("kill-confirm", "Quit", Primary)
                            .on_click(cx.listener(|this, _, _, cx| this.confirm_kill(cx)))
                            .into_any_element(),
                    ],
                )
                .into_any_element(),
            );
        }
        let verb = request.kind.label();
        let body = format!(
            "Do you want to {} the process \u{201c}{}\u{201d} (PID {})?",
            verb.to_lowercase(),
            request.process.name,
            request.process.pid
        );
        Some(
            rmac_ui::alert(
                format!("{verb} Process"),
                body,
                vec![
                    rmac_ui::dialog_button("kill-cancel", "Cancel", Normal)
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_kill(cx)))
                        .into_any_element(),
                    rmac_ui::dialog_button("kill-confirm", verb, Destructive)
                        .on_click(cx.listener(|this, _, _, cx| this.confirm_kill(cx)))
                        .into_any_element(),
                ],
            )
            .into_any_element(),
        )
    }

    pub(super) fn render_inspector(&self, cx: &Context<Self>) -> Option<impl IntoElement> {
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

        let card = div()
            .v_flex()
            .tab_group()
            .gap_1()
            .w(px(420.0))
            .p_5()
            .rounded(px(mac::radius_card()))
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

        // A hand-rolled scrim + card used to render here with no role and no
        // Tab trap, so Tab could leave the inspector for the process table
        // behind it. `rmac_ui::dialog` gives it `Role::Dialog`, and
        // `.tab_group()` on the card keeps Tab/Shift-Tab inside it, matching
        // `rmac_ui::alert`'s pattern.
        Some(rmac_ui::dialog("activity-monitor-inspector", card).into_any_element())
    }
}
