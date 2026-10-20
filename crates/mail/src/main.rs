// A GUI app on Windows: no console behind it (ADR 0023). Without this
// Windows made a console host for the process before `main` ran, about
// 200 ms of every launch on the reference laptop (WIN-OS-99).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
mod accounts_win;
mod compose_window;
mod delivery;
mod live;
mod settings_view;
mod view;

use std::borrow::Cow;
use std::sync::Arc;

use gpui::{App, AssetSource, KeyBinding, SharedString};
use rmac_mail::compose::ComposeKind;
use rmac_mail::MailState;
use rmac_ui::app_id::MAIL;
use view::MailView;

gpui::actions!(
    mail,
    [
        ToggleThreads,
        ToggleMessageFilter,
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
        ShowSettings,
        IncreaseIndentation,
        DecreaseIndentation,
        IncreaseQuoteLevel,
        DecreaseQuoteLevel,
        // MROBOT-03: the rest of Mac Mail's menus.
        ShowAccounts,
        OpenMessage,
        CloseAll,
        SaveDraft,
        SaveAs,
        SaveAttachments,
        ExportAsPdf,
        PrintMessage,
        Redo,
        PasteAsQuotation,
        JumpToTop,
        TransformUppercase,
        TransformLowercase,
        TransformCapitalise,
        StartSpeaking,
        StopSpeaking,
        SortByAttachments,
        SortByDate,
        SortByFlags,
        SortByFrom,
        SortBySubject,
        SortByTo,
        SortByUnread,
        SortAscending,
        SortDescending,
        FilterUnread,
        FilterFlagged,
        FilterToMe,
        FilterCcMe,
        FilterAttachments,
        ToggleColumnLayout,
        ToggleBottomPreview,
        ToggleDateAndTime,
        ToggleToCcLabel,
        ToggleContactPhoto,
        ToggleHighlightConversations,
        ExpandAllConversations,
        CollapseAllConversations,
        ToggleCcField,
        ToggleBccField,
        ShowRawSource,
        ToggleSidebar,
        ToggleToolbar,
        GetAllNewMail,
        MarkAllAsRead,
        EraseDeletedItems,
        EraseDeletedItemsNow,
        EraseJunkMail,
        NewMailbox,
        NewSmartMailbox,
        EditSmartMailbox,
        DuplicateSmartMailbox,
        RenameMailbox,
        DeleteMailbox,
        ExportMailbox,
        MailboxSheetConfirm,
        MailboxSheetCancel,
        MailboxSheetNextField,
        GoToFavourite0,
        GoToFavourite1,
        GoToFavourite2,
        GoToFavourite3,
        MoveToFavourite0,
        MoveToFavourite2,
        MoveToFavourite3,
        PreviousMailbox,
        NextMailbox,
        PreviousUnreadMailbox,
        NextUnreadMailbox,
        ForwardAsAttachment,
        Redirect,
        ClearFlag,
        MoveAgain,
        ShowMessageViewer
    ]
);

/// How many accounts Get New Mail ▸ and Erase Deleted Items ▸ list, and
/// how many mailboxes Message ▸ Move to ▸ / Copy to ▸ list: each row needs
/// an action of its own.
pub const ACCOUNT_ROWS: usize = 8;
pub const MAILBOX_ROWS: usize = 24;

gpui::actions!(
    mail,
    [
        GetNewMail0,
        GetNewMail1,
        GetNewMail2,
        GetNewMail3,
        GetNewMail4,
        GetNewMail5,
        GetNewMail6,
        GetNewMail7,
        EraseDeletedItemsAccount0,
        EraseDeletedItemsAccount1,
        EraseDeletedItemsAccount2,
        EraseDeletedItemsAccount3,
        EraseDeletedItemsAccount4,
        EraseDeletedItemsAccount5,
        EraseDeletedItemsAccount6,
        EraseDeletedItemsAccount7,
        MoveToMailbox0,
        MoveToMailbox1,
        MoveToMailbox2,
        MoveToMailbox3,
        MoveToMailbox4,
        MoveToMailbox5,
        MoveToMailbox6,
        MoveToMailbox7,
        MoveToMailbox8,
        MoveToMailbox9,
        MoveToMailbox10,
        MoveToMailbox11,
        MoveToMailbox12,
        MoveToMailbox13,
        MoveToMailbox14,
        MoveToMailbox15,
        MoveToMailbox16,
        MoveToMailbox17,
        MoveToMailbox18,
        MoveToMailbox19,
        MoveToMailbox20,
        MoveToMailbox21,
        MoveToMailbox22,
        MoveToMailbox23,
        CopyToMailbox0,
        CopyToMailbox1,
        CopyToMailbox2,
        CopyToMailbox3,
        CopyToMailbox4,
        CopyToMailbox5,
        CopyToMailbox6,
        CopyToMailbox7,
        CopyToMailbox8,
        CopyToMailbox9,
        CopyToMailbox10,
        CopyToMailbox11,
        CopyToMailbox12,
        CopyToMailbox13,
        CopyToMailbox14,
        CopyToMailbox15,
        CopyToMailbox16,
        CopyToMailbox17,
        CopyToMailbox18,
        CopyToMailbox19,
        CopyToMailbox20,
        CopyToMailbox21,
        CopyToMailbox22,
        CopyToMailbox23
    ]
);

// MROBOT-03 round two: the rest of Mac Mail's menus.
gpui::actions!(
    mail,
    [
        NewViewerWindow,
        QuickLookAttachments,
        FindInMessage,
        FindNext,
        FindPrevious,
        JumpToSelection,
        CloseFindBar,
        ShowAllHeaders,
        SortBySize,
        ToggleMessageSize,
        FlagOrange,
        FlagRed,
        FlagPurple,
        FlagBlue,
        FlagYellow,
        FlagGreen,
        FlagGrey,
        ToggleReplyToField,
        PriorityLow,
        PriorityNormal,
        PriorityHigh,
        ShowSpellingAndGrammar,
        CheckDocumentNow,
        CheckSpellingWhileTyping,
        CheckSpellingBeforeSending,
        CheckSpellingNever,
        ToggleCheckGrammarWithSpelling,
        ToggleCorrectSpellingAutomatically,
        ToggleSmartQuotes,
        ToggleSmartDashes,
        ToggleTextReplacement,
        AddLink,
        RemoveLink
    ]
);

/// Mail's assets: the shared ones, and Quick Look's own glyphs for File ▸
/// Quick Look Attachments….
struct MailAssets;

impl AssetSource for MailAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        if let Some(asset) = rmac_quick_look::asset(path) {
            return Ok(Some(asset));
        }
        rmac_ui::shared_assets().load(path)
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        let mut assets: Vec<SharedString> = rmac_quick_look::asset_paths(path)
            .into_iter()
            .map(SharedString::from)
            .collect();
        assets.extend(rmac_ui::shared_assets().list(path)?);
        Ok(assets)
    }
}

/// Set only for development, the Mac-parity screenshots and the behaviour
/// suite's accessibility/performance scenarios — never for a real user.
/// With it unset, `main()` only ever builds `MailState` from live
/// `rmac-mail-storage` data (`live::load`), showing the empty state when
/// there are no accounts yet, exactly as `docs/design/calendar-mail.md` §3
/// requires: Mail never shows sample messages to a real person.
const FIXTURE_ENV: &str = "RMAC_MAIL_FIXTURE";

/// Starts GOA account discovery and per-account IMAP sync (MAIL-4) and
/// bridges its snapshot/new-mail/open-message events into the window.
/// `rmac_mail_runtime` has no GPUI dependency by design, so a plain
/// `std::sync::mpsc` sink is drained on its own thread and forwarded into
/// an `async_channel` a GPUI task can `.await` on the foreground thread.
/// Returns the runtime so organise actions can wake the right account's
/// worker right after queuing a journal entry (`view.rs::dispatch_persist`).
/// Bridges the sync runtime's events into the window: a plain
/// `std::sync::mpsc` sink is drained on its own thread and forwarded into an
/// `async_channel` a GPUI task can `.await` on the foreground thread.
#[cfg(any(target_os = "linux", windows))]
fn forward_events(
    accounts: Vec<delivery::ComposeAccount>,
    cx: &mut App,
) -> std::sync::mpsc::Sender<rmac_mail_runtime::UiEvent> {
    use rmac_mail_runtime::UiEvent;

    #[cfg(windows)]
    let _ = &accounts;
    let (std_tx, std_rx) = std::sync::mpsc::channel::<UiEvent>();
    let (async_tx, async_rx) = async_channel::unbounded();
    std::thread::spawn(move || {
        while let Ok(event) = std_rx.recv() {
            if async_tx.send_blocking(event).is_err() {
                break;
            }
        }
    });
    cx.spawn(async move |cx| loop {
        let Ok(event) = async_rx.recv().await else {
            break;
        };
        match event {
            UiEvent::Snapshot(_) | UiEvent::NewMail(_) | UiEvent::AccountRemoved(_) => {
                // Windows lets accounts come and go while Mail runs, so each
                // reload asks the store again.
                #[cfg(not(windows))]
                let accounts = accounts.clone();
                let (mailboxes, messages) = cx
                    .background_executor()
                    .spawn(async move {
                        #[cfg(windows)]
                        let accounts = delivery::accounts();
                        live::load(&accounts)
                    })
                    .await;
                // Every viewer window shows the same mail.
                refresh_viewers(mailboxes, messages, cx);
            }
            UiEvent::OpenMessage {
                account,
                message_id,
            } => {
                if let Some(handle) = cx.update(|_| view::viewers().into_iter().next()) {
                    handle.update(cx, |view: &mut MailView, cx| {
                        if let Some(full_id) = view.message_id_for_row(account, message_id) {
                            view.open_message(&full_id, cx);
                        }
                    });
                }
            }
            UiEvent::Failure { .. } => {}
        }
    })
    .detach();
    std_tx
}

/// Hands a fresh load to every viewer window.
#[cfg(any(target_os = "linux", windows))]
fn refresh_viewers(
    mailboxes: Vec<rmac_mail::Mailbox>,
    messages: Vec<rmac_mail::Message>,
    cx: &mut gpui::AsyncApp,
) {
    cx.update(|cx| {
        for view in view::viewers() {
            let (mailboxes, messages) = (mailboxes.clone(), messages.clone());
            view.update(cx, |view, cx| view.refresh_live(mailboxes, messages, cx));
        }
    });
}

/// Starts GOA account discovery and per-account IMAP sync (MAIL-4) and
/// bridges its snapshot/new-mail/open-message events into the window.
/// Returns the runtime so organise actions can wake the right account's
/// worker right after queuing a journal entry (`view.rs::dispatch_persist`).
#[cfg(target_os = "linux")]
fn start_live_sync(
    accounts: Vec<delivery::ComposeAccount>,
    cx: &mut App,
) -> Option<Arc<rmac_mail_runtime::Runtime>> {
    use rmac_mail_runtime::linux::{self, DesktopSink};

    let root = delivery::data_root()?;
    let std_tx = forward_events(accounts, cx);
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
    Some(runtime)
}

/// Windows: the accounts in Lulo's own store (secrets in the Credential
/// Manager) sync through the same runtime; new messages arrive as toasts.
#[cfg(windows)]
fn start_live_sync(
    accounts: Vec<delivery::ComposeAccount>,
    cx: &mut App,
) -> Option<Arc<rmac_mail_runtime::Runtime>> {
    use rmac_mail_runtime::windows::{self, ToastSink};

    let root = delivery::data_root()?;
    let std_tx = forward_events(accounts, cx);
    let sink: Arc<dyn rmac_mail_runtime::EventSink> = Arc::new(ToastSink::new(Some(std_tx)));
    let factory = Arc::new(rmac_mail_runtime::ProviderFactory {
        imap: windows::imap_factory(),
        graph: Arc::new(rmac_mail_graph::GraphFactory::windows(root.clone())),
    });
    let runtime = Arc::new(rmac_mail_runtime::Runtime::new(root, factory, sink));
    let seeded = Arc::clone(&runtime);
    std::thread::spawn(move || windows::seed_accounts(&seeded));
    Some(runtime)
}

#[cfg(not(any(target_os = "linux", windows)))]
fn start_live_sync(
    _accounts: Vec<delivery::ComposeAccount>,
    _cx: &mut App,
) -> Option<Arc<rmac_mail_runtime::Runtime>> {
    // GOA and the sync runtime are session services of Lulo OS and the
    // Windows account store; CI's macOS clippy pass still compiles this crate.
    None
}

/// A later launch of Mail on Windows (a `mailto:` link, or the browser coming
/// back from a Google or Microsoft sign-in) reaches the running Mail here.
#[cfg(windows)]
fn handle_launch(arguments: Vec<String>, cx: &mut App) {
    if let Some(first) = arguments.first() {
        if let Some(draft) = rmac_mail::mailto::parse(first) {
            compose_window::open(
                ComposeKind::New,
                None,
                Some(draft),
                delivery::accounts(),
                Vec::new(),
                cx,
            );
        } else if !first.starts_with("--") {
            let _ = rmac_accounts_linux::oauth::callback::deliver(first);
        }
    }
    rmac_ui::activate_app_window(cx);
}

/// Windows: when accounts are added or removed in Settings, start or stop
/// their sync workers and show the new list.
#[cfg(windows)]
fn watch_account_changes(runtime: Option<Arc<rmac_mail_runtime::Runtime>>) {
    use std::cell::RefCell;
    use std::collections::HashSet;
    use std::rc::Rc;

    let known: Rc<RefCell<HashSet<String>>> = Rc::new(RefCell::new(
        delivery::accounts()
            .into_iter()
            .map(|account| account.path)
            .collect(),
    ));
    accounts_win::set_on_change(move |cx| {
        let accounts = delivery::accounts();
        let paths: HashSet<String> = accounts
            .iter()
            .map(|account| account.path.clone())
            .collect();
        if let Some(runtime) = &runtime {
            for gone in known.borrow().difference(&paths) {
                runtime.remove_account(gone);
            }
            let runtime = Arc::clone(runtime);
            std::thread::spawn(move || rmac_mail_runtime::windows::seed_accounts(&runtime));
        }
        *known.borrow_mut() = paths;
        for view in view::viewers() {
            let shown = accounts.clone();
            view.update(cx, |view: &mut MailView, cx| view.set_accounts(shown, cx));
        }
        cx.spawn(async move |cx| {
            let (mailboxes, messages) = cx
                .background_executor()
                .spawn(async move { live::load(&accounts) })
                .await;
            refresh_viewers(mailboxes, messages, cx);
        })
        .detach();
    });
}

fn main() {
    rmac_ui::startup_mark("mail_main");
    // One Mail per user on Windows: a later launch hands its `mailto:` link
    // (or sign-in return) to the running one instead of opening a second.
    #[cfg(windows)]
    if rmac_ui::hand_off_to_running_instance(MAIL, &[std::env::args().skip(1).collect()]) {
        rmac_ui::focus_running_app(MAIL);
        return;
    }
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
        .with_assets(MailAssets)
        .run(move |cx: &mut App| {
            rmac_ui::init_application(cx);
            cx.bind_keys([
                KeyBinding::new("cmd-shift-u", ToggleRead, Some("Mail")),
                KeyBinding::new("cmd-o", OpenMessage, Some("Mail")),
                KeyBinding::new("alt-cmd-w", CloseAll, None),
                KeyBinding::new("cmd-s", SaveDraft, Some("MailCompose")),
                KeyBinding::new("cmd-shift-s", SaveAs, Some("Mail")),
                KeyBinding::new("cmd-p", PrintMessage, Some("Mail")),
                KeyBinding::new("cmd-shift-z", Redo, Some("Mail")),
                KeyBinding::new("cmd-shift-v", PasteAsQuotation, Some("MailCompose")),
                KeyBinding::new("alt-cmd-f", Search, Some("Mail")),
                KeyBinding::new("cmd-f", FindInMessage, Some("Mail")),
                KeyBinding::new("cmd-g", FindNext, Some("Mail")),
                KeyBinding::new("cmd-shift-g", FindPrevious, Some("Mail")),
                KeyBinding::new("cmd-j", JumpToSelection, Some("Mail")),
                KeyBinding::new("escape", CloseFindBar, Some("MailFind")),
                KeyBinding::new("alt-cmd-n", NewViewerWindow, Some("Mail")),
                KeyBinding::new("cmd-y", QuickLookAttachments, Some("Mail")),
                KeyBinding::new("cmd-shift-h", ShowAllHeaders, Some("Mail")),
                KeyBinding::new("alt-cmd-r", ToggleReplyToField, None),
                KeyBinding::new("cmd-;", CheckDocumentNow, Some("MailCompose")),
                KeyBinding::new("cmd-k", AddLink, Some("MailCompose")),
                KeyBinding::new("alt-cmd-k", RemoveLink, Some("MailCompose")),
                KeyBinding::new("ctrl-cmd-t", JumpToTop, Some("Mail")),
                KeyBinding::new("cmd-l", ToggleMessageFilter, Some("Mail")),
                KeyBinding::new("alt-cmd-b", ToggleBccField, None),
                KeyBinding::new("alt-cmd-u", ShowRawSource, Some("Mail")),
                KeyBinding::new("ctrl-cmd-s", ToggleSidebar, Some("Mail")),
                KeyBinding::new("alt-cmd-t", ToggleToolbar, Some("Mail")),
                KeyBinding::new("cmd-shift-n", GetAllNewMail, None),
                KeyBinding::new("cmd-shift-backspace", EraseDeletedItems, Some("Mail")),
                KeyBinding::new(
                    "alt-cmd-shift-backspace",
                    EraseDeletedItemsNow,
                    Some("Mail"),
                ),
                KeyBinding::new("alt-cmd-j", EraseJunkMail, Some("Mail")),
                KeyBinding::new("escape", MailboxSheetCancel, Some("MailMailboxSheet")),
                KeyBinding::new("tab", MailboxSheetNextField, Some("MailMailboxSheet")),
                KeyBinding::new("enter", MailboxSheetConfirm, Some("MailMailboxSheet")),
                KeyBinding::new("cmd-1", GoToFavourite0, Some("Mail")),
                KeyBinding::new("cmd-2", GoToFavourite1, Some("Mail")),
                KeyBinding::new("cmd-3", GoToFavourite2, Some("Mail")),
                KeyBinding::new("cmd-4", GoToFavourite3, Some("Mail")),
                KeyBinding::new("ctrl-cmd-1", MoveToFavourite0, Some("Mail")),
                KeyBinding::new("ctrl-cmd-3", MoveToFavourite2, Some("Mail")),
                KeyBinding::new("ctrl-cmd-4", MoveToFavourite3, Some("Mail")),
                KeyBinding::new("ctrl-cmd-[", PreviousMailbox, Some("Mail")),
                KeyBinding::new("ctrl-cmd-]", NextMailbox, Some("Mail")),
                KeyBinding::new("ctrl-alt-cmd-[", PreviousUnreadMailbox, Some("Mail")),
                KeyBinding::new("ctrl-alt-cmd-]", NextUnreadMailbox, Some("Mail")),
                KeyBinding::new("cmd-shift-e", Redirect, Some("Mail")),
                KeyBinding::new("ctrl-cmd-a", Archive, Some("Mail")),
                KeyBinding::new("cmd-shift-d", SendMessage, Some("Mail")),
                KeyBinding::new("cmd-backspace", Delete, Some("Mail")),
                KeyBinding::new("cmd-0", ShowMessageViewer, None),
                KeyBinding::new("cmd-shift-l", Flag, Some("Mail")),
                KeyBinding::new("cmd-shift-j", Junk, Some("Mail")),
                KeyBinding::new("backspace", Delete, Some("Mail")),
                KeyBinding::new("cmd-z", Undo, Some("Mail")),
                KeyBinding::new("cmd-z", Undo, Some("MailCompose")),
                KeyBinding::new("cmd-shift-z", Redo, Some("MailCompose")),
                KeyBinding::new("ctrl-cmd-m", Move, Some("Mail")),
                KeyBinding::new("cmd-n", NewMessage, Some("Mail")),
                KeyBinding::new("cmd-r", Reply, Some("Mail")),
                KeyBinding::new("cmd-shift-r", ReplyAll, Some("Mail")),
                KeyBinding::new("cmd-shift-f", Forward, Some("Mail")),
                KeyBinding::new("cmd-shift-d", SendMessage, Some("MailCompose")),
                KeyBinding::new("cmd-shift-a", AttachFile, Some("MailCompose")),
                KeyBinding::new("cmd-]", IncreaseIndentation, Some("MailCompose")),
                KeyBinding::new("cmd-[", DecreaseIndentation, Some("MailCompose")),
                KeyBinding::new("cmd-'", IncreaseQuoteLevel, Some("MailCompose")),
                KeyBinding::new("alt-cmd-'", DecreaseQuoteLevel, Some("MailCompose")),
                KeyBinding::new("down", NextMessage, Some("Mail")),
                KeyBinding::new("up", PreviousMessage, Some("Mail")),
                KeyBinding::new("cmd-w", CloseWindow, Some("Mail")),
                KeyBinding::new("alt-cmd-w", rmac_ui::RequestClose, Some("Mail")),
                KeyBinding::new("cmd-,", ShowSettings, Some("Mail")),
            ]);
            #[cfg(windows)]
            rmac_ui::install_app_instance(MAIL, handle_launch, cx);
            #[cfg(not(windows))]
            rmac_ui::install_app_menu(MAIL, cx);
            // Edit ▸ Writing Tools in compose windows; hidden until Lulo
            // Intelligence is known to be on with its model present.
            rmac_writing_tools::init(cx);
            cx.observe_new(|_: &mut MailView, window, cx| {
                if let Some(window) = window {
                    cx.observe_window_activation(window, |_, window, cx| {
                        if window.is_window_active() {
                            rmac_writing_tools::refresh(cx);
                        }
                    })
                    .detach();
                }
            })
            .detach();
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
            // Printing goes through the desktop Print portal, which only the
            // Lulo OS session on Linux has.
            #[cfg(not(target_os = "linux"))]
            rmac_ui::set_menu_hidden("mail::PrintMessage", true, cx);
            cx.on_action(|_: &CloseAll, cx| view::close_all(cx));
            cx.on_action(|_: &ShowMessageViewer, cx| view::show_viewer(cx));
            cx.on_action(|_: &ToggleCcField, cx| {
                compose_window::toggle_field(compose_window::Field::Cc, cx)
            });
            cx.on_action(|_: &ToggleBccField, cx| {
                compose_window::toggle_field(compose_window::Field::Bcc, cx)
            });
            cx.on_action(|_: &ToggleReplyToField, cx| {
                compose_window::toggle_field(compose_window::Field::ReplyTo, cx)
            });
            cx.on_action(|_: &CheckSpellingWhileTyping, cx| {
                compose_window::set_spelling_mode(compose_window::SpellingMode::WhileTyping, cx)
            });
            cx.on_action(|_: &CheckSpellingBeforeSending, cx| {
                compose_window::set_spelling_mode(compose_window::SpellingMode::BeforeSending, cx)
            });
            cx.on_action(|_: &CheckSpellingNever, cx| {
                compose_window::set_spelling_mode(compose_window::SpellingMode::Never, cx)
            });
            cx.on_action(|_: &ToggleCheckGrammarWithSpelling, cx| {
                compose_window::change_assist(
                    |settings| {
                        settings.check_grammar_with_spelling = !settings.check_grammar_with_spelling
                    },
                    cx,
                )
            });
            cx.on_action(|_: &ToggleCorrectSpellingAutomatically, cx| {
                compose_window::change_assist(
                    |settings| {
                        settings.correct_spelling_automatically =
                            !settings.correct_spelling_automatically
                    },
                    cx,
                )
            });
            cx.on_action(|_: &ToggleSmartQuotes, cx| {
                compose_window::change_assist(
                    |settings| settings.smart_quotes = !settings.smart_quotes,
                    cx,
                )
            });
            cx.on_action(|_: &ToggleSmartDashes, cx| {
                compose_window::change_assist(
                    |settings| settings.smart_dashes = !settings.smart_dashes,
                    cx,
                )
            });
            cx.on_action(|_: &ToggleTextReplacement, cx| {
                compose_window::change_assist(
                    |settings| settings.text_replacement = !settings.text_replacement,
                    cx,
                )
            });
            compose_window::publish_field_state(cx);
            let runtime = if fixture_mode {
                None
            } else {
                start_live_sync(sync_accounts, cx)
            };
            #[cfg(windows)]
            watch_account_changes(runtime.clone());
            if view::open_viewer(initial_state, accounts, runtime, cx).is_none() {
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
