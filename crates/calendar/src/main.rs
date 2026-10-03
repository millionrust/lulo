//! Calendar shell and read-only Week view (CAL-3).
mod view;

use gpui::{App, AppContext as _, KeyBinding};
use rmac_ui::{app_id::CALENDAR, Root};
use view::CalendarView;

gpui::actions!(
    calendar,
    [
        ShowDay, ShowWeek, ShowMonth, ShowYear,
        GoToday, PreviousPeriod, NextPeriod,
        ToggleSidebar, NewEvent, ShowInvitations, Search,
        CloseWindow,
    ]
);

fn main() {
    rmac_ui::application().run(|cx: &mut App| {
        rmac_ui::init_application(cx);
        cx.bind_keys([
            KeyBinding::new("cmd-1", ShowDay, Some("Calendar")),
            KeyBinding::new("cmd-2", ShowWeek, Some("Calendar")),
            KeyBinding::new("cmd-3", ShowMonth, Some("Calendar")),
            KeyBinding::new("cmd-4", ShowYear, Some("Calendar")),
            KeyBinding::new("cmd-t", GoToday, Some("Calendar")),
            KeyBinding::new("cmd-left", PreviousPeriod, Some("Calendar")),
            KeyBinding::new("cmd-right", NextPeriod, Some("Calendar")),
            KeyBinding::new("ctrl-cmd-s", ToggleSidebar, Some("Calendar")),
            KeyBinding::new("cmd-w", CloseWindow, Some("Calendar")),
        ]);
        rmac_ui::install_app_menu(CALENDAR, cx);
        // Event creation, invitations, and search arrive in CAL-5/6/8.
        for action in ["calendar::NewEvent", "calendar::ShowInvitations", "calendar::Search"] {
            rmac_ui::set_menu_enabled(action, false, cx);
        }
        let mut options = rmac_ui::window_options_for_app(CALENDAR, 1100.0, 720.0, cx);
        options.window_min_size = Some(gpui::size(gpui::px(760.0), gpui::px(560.0)));
        let opened = cx.open_window(options, |window, cx| {
            rmac_ui::prepare_surface_window(window, cx);
            let view = cx.new(|cx| {
                rmac_ui::observe_window_state(CALENDAR, window, cx);
                CalendarView::new(cx)
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
            if cx.windows().is_empty() { cx.quit(); }
        }).detach();
        cx.activate(true);
    });
}
