//! Calendar shell: Day/Week/Month/Year views (CAL-3/CAL-4), event editing
//! (CAL-5), calendar-list management, ICS subscriptions, search and
//! Settings (CAL-6), and invitations and deep links (CAL-8).
mod deep_link;
mod settings_window;
mod view;

pub use deep_link::DeepLink;
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
    let args: Vec<String> = std::env::args().skip(1).collect();
    let deep_link = deep_link::parse(&args);
    rmac_ui::application()
        .with_assets(rmac_ui::shared_assets())
        .run(move |cx: &mut App| {
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
            let mut options = rmac_ui::window_options_for_app(CALENDAR, 1100.0, 720.0, cx);
            options.window_min_size = Some(gpui::size(gpui::px(760.0), gpui::px(560.0)));
            let opened = cx.open_window(options, move |window, cx| {
                rmac_ui::prepare_surface_window(window, cx);
                rmac_ui::fit_to_display_after_first_frame(window, cx);
                let view = cx.new(|cx| {
                    rmac_ui::observe_window_state(CALENDAR, window, cx);
                    CalendarView::new(window, cx, deep_link)
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
