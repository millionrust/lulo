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
mod view_filter;

use view::MonitorView;

gpui::actions!(
    activity_monitor,
    [
        QuitProcess,
        ForceQuitProcess,
        FocusSearch,
        FilterProcesses,
        FindNext,
        FindPrevious,
        UseSelectionForFind,
        JumpToSelection,
        InspectProcess,
        ClearCpuHistory,
        Close,
        CloseAll,
        ShowAllProcesses,
        ShowMyProcesses,
        ShowSystemProcesses,
        ShowOtherUsersProcesses,
        ShowActiveProcesses,
        ShowInactiveProcesses,
        ShowSelectedProcesses,
        ShowMainWindow,
        TogglePidColumn,
        ToggleUserColumn,
        ToggleCpuColumn,
        ToggleThreadsColumn,
        ToggleMemoryColumn,
        RefreshEverySecond,
        RefreshEveryTwoSeconds,
        RefreshEveryFiveSeconds,
        CancelKill,
        ConfirmKill,
        Minimize
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
            // Route menu-bar commands to the monitor even while focus is in
            // the top bar or has just returned from a dismissed confirmation.
            rmac_ui::register_menu_target(window, &view.focus, cx);
            cx.bind_keys([
                gpui::KeyBinding::new(
                    rmac_ui::shortcuts::FIND.keystroke,
                    FocusSearch,
                    Some("ActivityMonitor"),
                ),
                gpui::KeyBinding::new("alt-cmd-f", FilterProcesses, Some("ActivityMonitor")),
                gpui::KeyBinding::new("cmd-g", FindNext, Some("ActivityMonitor")),
                gpui::KeyBinding::new("shift-cmd-g", FindPrevious, Some("ActivityMonitor")),
                gpui::KeyBinding::new("cmd-e", UseSelectionForFind, Some("ActivityMonitor")),
                gpui::KeyBinding::new("cmd-j", JumpToSelection, Some("ActivityMonitor")),
                gpui::KeyBinding::new("cmd-i", InspectProcess, Some("ActivityMonitor")),
                gpui::KeyBinding::new("cmd-k", ClearCpuHistory, Some("ActivityMonitor")),
                gpui::KeyBinding::new("alt-cmd-q", QuitProcess, Some("ActivityMonitor")),
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
                // ⌘W closes the window; already handled by on_action in
                // view/render.rs, just never had a keystroke bound to it.
                gpui::KeyBinding::new(
                    rmac_ui::shortcuts::CLOSE.keystroke,
                    Close,
                    Some("ActivityMonitor"),
                ),
                gpui::KeyBinding::new("alt-cmd-w", CloseAll, Some("ActivityMonitor")),
                // System Monitor is a single window, so ⌘Q and ⌘W both just
                // need to close it -- reuse the same handler.
                gpui::KeyBinding::new("cmd-q", rmac_ui::RequestClose, Some("ActivityMonitor")),
                // ⌘M minimizes the window, the same as the yellow traffic
                // light.
                gpui::KeyBinding::new("cmd-m", Minimize, Some("ActivityMonitor")),
                gpui::KeyBinding::new("cmd-1", ShowMainWindow, Some("ActivityMonitor")),
            ]);
            window.focus(&view.focus, cx);
            view
        },
    );
}
