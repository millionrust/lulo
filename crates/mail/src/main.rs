mod compose_window;
mod delivery;
mod live;
mod settings_view;
mod view;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, AppContext as _, KeyBinding, WeakEntity};
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

/// Set only for development, the Mac-parity screenshots and the behaviour
/// suite's accessibility/performance scenarios — never for a real user.
/// With it unset, `main()` only ever builds `MailState` from live
/// `rmac-mail-storage` data (`live::load`), showing the empty state when
/// there are no accounts yet, exactly as `docs/design/calendar-mail.md` §3
/// requires: Mail never shows sample messages to a real person.
const FIXTURE_ENV: &str = "RMAC_MAIL_FIXTURE";

type ViewHandle = Rc<RefCell<Option<WeakEntity<MailView>>>>;

/// Starts GOA account discovery and per-account IMAP sync (MAIL-4) and
/// bridges its snapshot/new-mail/open-message events into the window.
/// `rmac_mail_runtime` has no GPUI dependency by design, so a plain
/// `std::sync::mpsc` sink is drained on its own thread and forwarded into
/// an `async_channel` a GPUI task can `.await` on the foreground thread.
/// Returns the runtime so organise actions can wake the right account's
/// worker right after queuing a journal entry (`view.rs::dispatch_persist`).
#[cfg(target_os = "linux")]
fn start_live_sync(
    accounts: Vec<delivery::ComposeAccount>,
    view_handle: ViewHandle,
    cx: &mut App,
) -> Option<Arc<rmac_mail_runtime::Runtime>> {
    use rmac_mail_runtime::linux::{self, DesktopSink, UiEvent};

    let root = delivery::data_root()?;
    let (std_tx, std_rx) = std::sync::mpsc::channel::<UiEvent>();
    let (async_tx, async_rx) = async_channel::unbounded();
    std::thread::spawn(move || {
        while let Ok(event) = std_rx.recv() {
            if async_tx.send_blocking(event).is_err() {
                break;
            }
        }
    });
    let sink: Arc<dyn rmac_mail_runtime::EventSink> = Arc::new(DesktopSink::new(Some(std_tx)));
    // GOA `ms_graph` accounts (Microsoft 365, Outlook.com) sync through
    // Microsoft Graph (MAIL-9); `linux::resolve_goa` routes them here.
    let factory = Arc::new(linux::ProviderFactory {
        imap: rmac_mail_runtime::ImapFactory::goa(),
        graph: Arc::new(rmac_mail_graph::GraphFactory::goa(root.clone())),
    });
    let runtime = Arc::new(rmac_mail_runtime::Runtime::new(root, factory, sink));
    // Seeds every mail-capable GOA account already configured (`watch`
    // emits `Added` for each on the first call), then reacts to Internet
    // Accounts changes for as long as Mail runs.
    linux::watch_goa(Arc::clone(&runtime));
    linux::watch_connectivity(Arc::clone(&runtime));
    cx.spawn(async move |cx| loop {
        let Ok(event) = async_rx.recv().await else {
            break;
        };
        let Some(handle) = view_handle.borrow().clone() else {
            continue;
        };
        match event {
            UiEvent::Snapshot(_) | UiEvent::NewMail(_) | UiEvent::AccountRemoved(_) => {
                let accounts = accounts.clone();
                let (mailboxes, messages) = cx
                    .background_executor()
                    .spawn(async move { live::load(&accounts) })
                    .await;
                let _ = handle.update(cx, |view, cx| {
                    view.refresh_live(mailboxes, messages, cx);
                });
            }
            UiEvent::OpenMessage {
                account,
                message_id,
            } => {
                let _ = handle.update(cx, |view, cx| {
                    if let Some(full_id) = view.message_id_for_row(account, message_id) {
                        view.open_message(&full_id, cx);
                    }
                });
            }
            UiEvent::Failure { .. } => {}
        }
    })
    .detach();
    Some(runtime)
}

#[cfg(not(target_os = "linux"))]
fn start_live_sync(
    _accounts: Vec<delivery::ComposeAccount>,
    _view_handle: ViewHandle,
    _cx: &mut App,
) -> Option<Arc<rmac_mail_runtime::Runtime>> {
    // GOA and the sync runtime are Linux session services; Lulo OS only
    // ships on Linux, but CI's macOS clippy pass still compiles this crate.
    None
}

fn main() {
    let fixture_mode = std::env::var(FIXTURE_ENV).is_ok_and(|value| value == "1");
    let accounts = if fixture_mode {
        vec![
            delivery::ComposeAccount {
                path: "/fixture/Google".to_owned(),
                id: rmac_mail::FIXTURE_GOOGLE_ACCOUNT,
                address: "jacob@example.com".to_owned(),
                provider: "google".to_owned(),
            },
            delivery::ComposeAccount {
                path: "/fixture/iCloud".to_owned(),
                id: rmac_mail::FIXTURE_ICLOUD_ACCOUNT,
                address: "jacob@icloud.example".to_owned(),
                provider: "imap_smtp".to_owned(),
            },
        ]
    } else {
        delivery::accounts()
    };
    let initial_state = if fixture_mode {
        let extra = std::env::var("RMAC_MAIL_FIXTURE_MESSAGES")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        MailState::fixture_with_extra(extra.min(100_000))
    } else {
        let (mailboxes, messages) = live::load(&accounts);
        MailState::new(mailboxes, messages)
    };
    // `Exec=/usr/bin/rmac-mail %u` (packaging/rmac-apps) hands a `mailto:`
    // URI here as the one argument when the session's mailto handler runs.
    let mailto_draft = std::env::args()
        .nth(1)
        .and_then(|argument| rmac_mail::mailto::parse(&argument));
    // Compose's address completion (MAIL-6) needs the fixture before it
    // moves into `MailView::new` below.
    let mailto_candidates = rmac_mail::compose::known_recipients(&initial_state.messages);
    let mailto_accounts = accounts.clone();
    let sync_accounts = accounts.clone();
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
            // `view_handle` is an empty slot `start_live_sync` only reads
            // from once an event actually arrives, by which time the
            // window below has filled it in; this breaks what would
            // otherwise be a circular dependency (the view needs the
            // runtime to wake workers after an organise action, and the
            // runtime's background loop needs a handle to the view).
            let view_handle: ViewHandle = Rc::new(RefCell::new(None));
            let runtime = if fixture_mode {
                None
            } else {
                start_live_sync(sync_accounts, Rc::clone(&view_handle), cx)
            };
            let view_handle_for_window = Rc::clone(&view_handle);
            let runtime_for_window = runtime.clone();
            let opened = cx.open_window(options, move |window, cx| {
                rmac_ui::prepare_surface_window(window, cx);
                rmac_ui::fit_to_display_after_first_frame(window, cx);
                let view = cx.new(|cx| {
                    rmac_ui::observe_window_state(MAIL, window, cx);
                    MailView::new(initial_state, accounts, runtime_for_window, window, cx)
                });
                *view_handle_for_window.borrow_mut() = Some(view.downgrade());
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
