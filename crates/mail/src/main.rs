mod compose_window;
mod delivery;
mod settings_view;
mod view;

use gpui::{App, AppContext as _, KeyBinding};
use rmac_mail::compose::ComposeKind;
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
        Search,
        SendMessage,
        AttachFile,
        ShowSettings
    ]
);

/// The sole mailbox address the fixture (MAIL-5..MAIL-8) knows about.
/// MAIL-4's account runtime replaces this with the account actually chosen
/// for the message being composed.
const FIXTURE_ACCOUNT_ADDRESS: &str = "jacob@example.com";

fn main() {
    // Parse the small fixture before starting the UI. MAIL-4 will deliver snapshots
    // from its workers; neither parsing nor mailbox I/O belongs in render().
    let fixture = MailState::fixture();
    let accounts = delivery::accounts();
    // `Exec=/usr/bin/rmac-mail %u` (packaging/rmac-apps) hands a `mailto:`
    // URI here as the one argument when the session's mailto handler runs.
    let mailto_draft = std::env::args()
        .nth(1)
        .and_then(|argument| rmac_mail::mailto::parse(&argument));
    // Compose's address completion (MAIL-6) needs the fixture before it
    // moves into `MailView::new` below.
    let mailto_candidates = rmac_mail::compose::known_recipients(&fixture.messages);
    let mailto_accounts = accounts.clone();
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
                KeyBinding::new("cmd-n", NewMessage, Some("Mail")),
                KeyBinding::new("cmd-r", Reply, Some("Mail")),
                KeyBinding::new("cmd-shift-r", ReplyAll, Some("Mail")),
                KeyBinding::new("cmd-shift-f", Forward, Some("Mail")),
                KeyBinding::new("cmd-shift-d", SendMessage, Some("MailCompose")),
                KeyBinding::new("cmd-shift-a", AttachFile, Some("MailCompose")),
                KeyBinding::new("down", NextMessage, Some("Mail")),
                KeyBinding::new("up", PreviousMessage, Some("Mail")),
                KeyBinding::new("cmd-w", CloseWindow, Some("Mail")),
                KeyBinding::new("alt-cmd-w", rmac_ui::RequestClose, Some("Mail")),
                KeyBinding::new("cmd-,", ShowSettings, Some("Mail")),
            ]);
            rmac_ui::install_app_menu(MAIL, cx);
            for action in [
                "mail::NewMessage",
                "mail::Reply",
                "mail::ReplyAll",
                "mail::Forward",
                "mail::Archive",
                "mail::Delete",
                "mail::Junk",
                "mail::Flag",
                "mail::Move",
                "mail::Search",
            ] {
                rmac_ui::set_menu_enabled(action, false, cx);
            }
            let mut options = rmac_ui::window_options_for_app(MAIL, 1200.0, 750.0, cx);
            options.window_min_size = Some(gpui::size(gpui::px(860.0), gpui::px(540.0)));
            let opened = cx.open_window(options, |window, cx| {
                rmac_ui::prepare_surface_window(window, cx);
                let view = cx.new(|cx| {
                    rmac_ui::observe_window_state(MAIL, window, cx);
                    MailView::new(fixture, accounts, window, cx)
                });
                let focus = view.read(cx).focus.clone();
                window.focus(&focus, cx);
                cx.new(|cx| Root::new(view, window, cx))
            });
            if let Err(error) = opened {
                eprintln!("rmac-mail: could not open a window: {error}");
                cx.quit();
            }
            if let Some(draft) = mailto_draft {
                compose_window::open(
                    ComposeKind::New,
                    None,
                    Some(draft),
                    mailto_accounts,
                    mailto_candidates,
                    cx,
                );
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
