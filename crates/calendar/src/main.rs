//! Calendar shell: Day/Week/Month/Year views (CAL-3/CAL-4), event editing
//! (CAL-5), and calendar-list management, ICS subscriptions, search and
//! Settings (CAL-6).
mod settings_window;
mod view;

use gpui::{App, AppContext as _, KeyBinding};
use rmac_ui::{app_id::CALENDAR, Root};
use view::CalendarView;

gpui::actions!(
    calendar,
    [
        ShowDay,
        ShowWeek,
        ShowMonth,
        ShowYear,
        GoToday,
        PreviousPeriod,
        NextPeriod,
        SelectPreviousDay,
        SelectNextDay,
        SelectPreviousWeek,
        SelectNextWeek,
        ToggleSidebar,
        NewEvent,
        SaveEvent,
        DeleteEvent,
        UndoEvent,
        RedoEvent,
        ShowInspector,
        DismissInspector,
        ShowInvitations,
        Search,
        NewCalendar,
        NewCalendarSubscription,
        RenameCalendar,
        DeleteCalendar,
        ShowSettings,
        CloseWindow,
    ]
);

fn main() {
    rmac_ui::application()
        .with_assets(rmac_ui::shared_assets())
        .run(|cx: &mut App| {
            rmac_ui::init_application(cx);
            cx.bind_keys([
                KeyBinding::new("cmd-1", ShowDay, Some("Calendar")),
                KeyBinding::new("cmd-2", ShowWeek, Some("Calendar")),
                KeyBinding::new("cmd-3", ShowMonth, Some("Calendar")),
                KeyBinding::new("cmd-4", ShowYear, Some("Calendar")),
                KeyBinding::new("cmd-t", GoToday, Some("Calendar")),
                KeyBinding::new("cmd-n", NewEvent, Some("Calendar")),
                KeyBinding::new("cmd-i", ShowInspector, Some("Calendar")),
                KeyBinding::new("delete", DeleteEvent, Some("Calendar")),
                KeyBinding::new("backspace", DeleteEvent, Some("Calendar")),
                KeyBinding::new("cmd-z", UndoEvent, Some("Calendar")),
                KeyBinding::new("shift-cmd-z", RedoEvent, Some("Calendar")),
                KeyBinding::new("escape", DismissInspector, Some("Calendar")),
                KeyBinding::new("cmd-left", PreviousPeriod, Some("Calendar")),
                KeyBinding::new("cmd-right", NextPeriod, Some("Calendar")),
                KeyBinding::new("left", SelectPreviousDay, Some("Calendar")),
                KeyBinding::new("right", SelectNextDay, Some("Calendar")),
                KeyBinding::new("up", SelectPreviousWeek, Some("Calendar")),
                KeyBinding::new("down", SelectNextWeek, Some("Calendar")),
                KeyBinding::new("ctrl-cmd-s", ToggleSidebar, Some("Calendar")),
                KeyBinding::new("cmd-f", Search, Some("Calendar")),
                KeyBinding::new("cmd-,", ShowSettings, Some("Calendar")),
                KeyBinding::new("cmd-w", CloseWindow, Some("Calendar")),
                KeyBinding::new("alt-cmd-w", rmac_ui::RequestClose, Some("Calendar")),
            ]);
            rmac_ui::install_app_menu(CALENDAR, cx);
            // Invitations arrive in CAL-8.
            for action in ["calendar::ShowInvitations"] {
                rmac_ui::set_menu_enabled(action, false, cx);
            }
            let mut options = rmac_ui::window_options_for_app(CALENDAR, 1100.0, 720.0, cx);
            options.window_min_size = Some(gpui::size(gpui::px(760.0), gpui::px(560.0)));
            let opened = cx.open_window(options, |window, cx| {
                rmac_ui::prepare_surface_window(window, cx);
                let view = cx.new(|cx| {
                    rmac_ui::observe_window_state(CALENDAR, window, cx);
                    CalendarView::new(window, cx)
                });
                let focus = view.read(cx).focus.clone();
                window.focus(&focus, cx);
                cx.new(|cx| Root::new(view, window, cx))
            });
            if let Err(error) = opened {
                eprintln!("rmac-calendar: could not open a window: {error}");
                cx.quit();
            }
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            cx.activate(true);
        });
}
