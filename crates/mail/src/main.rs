mod view;

use gpui::{App, AppContext as _, KeyBinding};
use rmac_mail::MailState;
use rmac_ui::{app_id::MAIL, Root};
use view::MailView;

gpui::actions!(
    mail,
    [
        ToggleThreads,
        ToggleUnreadFilter,
        ToggleRead,
        NextMessage,
        PreviousMessage,
        CloseWindow,
        NewMessage,
        Archive,
        Delete,
        Junk,
        Reply,
        ReplyAll,
        Forward,
        Flag,
        Move,
        Copy,
        Undo,
        Search
    ]
);

fn main() {
    // Parse the small fixture before starting the UI. MAIL-4 will deliver snapshots
    // from its workers; neither parsing nor mailbox I/O belongs in render().
    let fixture = MailState::fixture();
    rmac_ui::application()
        .with_assets(rmac_ui::shared_assets())
        .run(move |cx: &mut App| {
            rmac_ui::init_application(cx);
            cx.bind_keys([
                KeyBinding::new("cmd-shift-u", ToggleRead, Some("Mail")),
                KeyBinding::new("cmd-shift-l", Flag, Some("Mail")),
                KeyBinding::new("cmd-shift-j", Junk, Some("Mail")),
                KeyBinding::new("backspace", Delete, Some("Mail")),
                KeyBinding::new("cmd-z", Undo, Some("Mail")),
                KeyBinding::new("cmd-f", Search, Some("Mail")),
                KeyBinding::new("ctrl-cmd-m", Move, Some("Mail")),
                KeyBinding::new("down", NextMessage, Some("Mail")),
                KeyBinding::new("up", PreviousMessage, Some("Mail")),
                KeyBinding::new("cmd-w", CloseWindow, Some("Mail")),
                KeyBinding::new("alt-cmd-w", rmac_ui::RequestClose, Some("Mail")),
            ]);
            rmac_ui::install_app_menu(MAIL, cx);
            for action in [
                "mail::NewMessage",
                "mail::Reply",
                "mail::ReplyAll",
                "mail::Forward",
            ] {
                rmac_ui::set_menu_enabled(action, false, cx);
            }
            let mut options = rmac_ui::window_options_for_app(MAIL, 1200.0, 750.0, cx);
            options.window_min_size = Some(gpui::size(gpui::px(860.0), gpui::px(540.0)));
            let opened = cx.open_window(options, |window, cx| {
                rmac_ui::prepare_surface_window(window, cx);
                let view = cx.new(|cx| {
                    rmac_ui::observe_window_state(MAIL, window, cx);
                    MailView::new(fixture, window, cx)
                });
                let focus = view.read(cx).focus.clone();
                window.focus(&focus, cx);
                cx.new(|cx| Root::new(view, window, cx))
            });
            if let Err(error) = opened {
                eprintln!("rmac-mail: could not open a window: {error}");
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
