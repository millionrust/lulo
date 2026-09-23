//! rmac System Monitor — a fast, native, GPU-rendered process monitor.

mod columns;
mod cpu_ticks;
mod host_stats;
mod metrics;
mod process_action;
mod process_signal;
mod process_table;
mod sampling;
mod storage;
mod view;

use view::MonitorView;

gpui::actions!(
    activity_monitor,
    [
        QuitProcess,
        ForceQuitProcess,
        FocusSearch,
        CancelKill,
        ConfirmKill
    ]
);

fn main() {
    rmac_ui::boot_app(
        rmac_ui::app_id::SYSTEM_MONITOR,
        "System Monitor",
        // Activity Monitor's window on macOS 26.2 (measured).
        960.0,
        640.0,
        |window, cx| {
            let view = MonitorView::new(window, cx);
            cx.bind_keys([
                gpui::KeyBinding::new(
                    rmac_ui::shortcuts::FIND.keystroke,
                    FocusSearch,
                    Some("ActivityMonitor"),
                ),
                gpui::KeyBinding::new(
                    rmac_ui::shortcuts::DELETE.keystroke,
                    QuitProcess,
                    Some("ActivityMonitor"),
                ),
                gpui::KeyBinding::new(
                    rmac_ui::shortcuts::FORCE_DELETE.keystroke,
                    ForceQuitProcess,
                    Some("ActivityMonitor"),
                ),
                gpui::KeyBinding::new(
                    rmac_ui::shortcuts::ENTER.keystroke,
                    ConfirmKill,
                    Some("ActivityMonitor"),
                ),
                gpui::KeyBinding::new(
                    rmac_ui::shortcuts::ESCAPE.keystroke,
                    CancelKill,
                    Some("ActivityMonitor"),
                ),
            ]);
            window.focus(&view.focus, cx);
            view
        },
    );
}
