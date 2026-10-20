//! Mail's menu commands beyond the toolbar's (MROBOT-03): the rest of Mac
//! Mail's File, Edit, View, Mailbox, Message and Window menus, and the
//! live state each row shows.

use std::path::{Path, PathBuf};

use gpui::{
    div, px, relative, AnyElement, App, Context, Focusable as _, InteractiveElement as _,
    IntoElement, ParentElement as _, SharedString, Styled as _, Window,
};
use rmac_mail::{compose::ComposeKind, Erase, Mailbox, SortKey, SpecialUse};
use rmac_ui::{mac, DialogButtonKind, MenuItem};

use super::MailView;
use crate::{ACCOUNT_ROWS, MAILBOX_ROWS};

/// File ▸ Close All (⌥⌘W): every Mail window closes the way its own Close
/// does, so an unsent message is kept in Drafts.
pub fn close_all(cx: &mut App) {
    for handle in cx.windows() {
        let _ = handle.update(cx, |_, window, cx| {
            window.dispatch_action(Box::new(rmac_ui::RequestClose), cx);
        });
    }
}

/// Mail ▸ Accounts…: Internet Accounts, where Lulo keeps mail accounts (as
/// macOS 26 opens Internet Accounts in System Settings). On Windows, Mail's
/// own Settings ▸ Accounts holds them.
pub(super) fn open_internet_accounts(cx: &mut App) {
    #[cfg(windows)]
    crate::settings_view::show_accounts(cx);
    #[cfg(not(windows))]
    cx.spawn(async move |_| {
        if let Err(error) = std::process::Command::new("rmac-system-settings")
            .arg("--pane")
            .arg("internet-accounts")
            .spawn()
        {
            eprintln!("rmac-mail: could not open System Settings: {error}");
        }
    })
    .detach();
}

const SORT_ACTIONS: [(&str, SortKey); 8] = [
    ("mail::SortByAttachments", SortKey::Attachments),
    ("mail::SortByDate", SortKey::Date),
    ("mail::SortByFlags", SortKey::Flags),
    ("mail::SortByFrom", SortKey::From),
    ("mail::SortBySize", SortKey::Size),
    ("mail::SortBySubject", SortKey::Subject),
    ("mail::SortByTo", SortKey::To),
    ("mail::SortByUnread", SortKey::Unread),
];

/// The favourite each Go to / Move to Favourite Mailbox row stands for,
/// in sidebar order.
const FAVOURITES: [Mailbox; 4] = [
    Mailbox::AllInboxes,
    Mailbox::Flagged,
    Mailbox::Drafts,
    Mailbox::Sent,
];

/// Which View ▸ Filter row a toggle changes.
#[derive(Clone, Copy)]
enum FilterRow {
    Unread,
    Flagged,
    ToMe,
    CcMe,
    Attachments,
}

/// Which View ▸ option a toggle changes.
#[derive(Clone, Copy)]
enum ViewOption {
    ColumnLayout,
    BottomPreview,
    DateAndTime,
    ToCcLabel,
    ContactPhoto,
    HighlightConversations,
    MessageSize,
    Sidebar,
    Toolbar,
}

/// A home for saved files when the panel opens: the person's Documents.
pub(super) fn save_directory() -> PathBuf {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let documents = home.join("Documents");
    if documents.is_dir() {
        documents
    } else {
        home
    }
}

pub(super) fn write_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use rmac_storage::Backend as _;
    rmac_storage::FileSystem.write_atomic(path, bytes)
}

macro_rules! indexed {
    ($root:expr, $cx:expr, $method:ident, [$($action:ident => $index:expr),* $(,)?]) => {
        $root$(.on_action($cx.listener(
            |this: &mut MailView, _: &crate::$action, window, cx| this.$method($index, window, cx),
        )))*
    };
}

impl MailView {
    /// Every menu row's live state: checkmarks, labels, what is enabled and
    /// the rows built from the mailbox list.
    pub(super) fn sync_menu(&self, cx: &mut Context<Self>) {
        use rmac_ui::{set_menu_checked as check, set_menu_enabled as enable};
        let state = &self.state;
        let selected = state.selected_message();
        let has_selection = selected.is_some();
        check("mail::ToggleThreads", state.threads, cx);
        for (action, key) in SORT_ACTIONS {
            check(action, state.sort == key, cx);
        }
        let by_date = state.sort == SortKey::Date;
        rmac_ui::set_menu_label(
            "mail::SortAscending",
            if by_date {
                "Oldest Message on Top"
            } else {
                "Ascending"
            },
            cx,
        );
        rmac_ui::set_menu_label(
            "mail::SortDescending",
            if by_date {
                "Newest Message on Top"
            } else {
                "Descending"
            },
            cx,
        );
        check("mail::SortAscending", !state.descending, cx);
        check("mail::SortDescending", state.descending, cx);
        let filter = state.filter;
        check("mail::ToggleMessageFilter", filter.enabled, cx);
        for (action, on) in [
            ("mail::FilterUnread", filter.unread),
            ("mail::FilterFlagged", filter.flagged),
            ("mail::FilterToMe", filter.to_me),
            ("mail::FilterCcMe", filter.cc_me),
            ("mail::FilterAttachments", filter.attachments),
        ] {
            check(action, on, cx);
            // As on the Mac, the criteria apply while the filter is on.
            enable(action, filter.enabled, cx);
        }
        for (action, on) in [
            ("mail::ToggleColumnLayout", self.column_layout),
            ("mail::ToggleBottomPreview", self.bottom_preview),
            ("mail::ToggleDateAndTime", self.show_date_time),
            ("mail::ToggleToCcLabel", self.show_to_cc),
            ("mail::ToggleContactPhoto", self.show_contact_photo),
            (
                "mail::ToggleHighlightConversations",
                self.highlight_conversations,
            ),
        ] {
            check(action, on, cx);
        }
        // Highlighting marks a conversation's messages while they are not
        // grouped into one row.
        enable("mail::ToggleHighlightConversations", !state.threads, cx);
        rmac_ui::set_menu_label(
            "mail::ToggleSidebar",
            if self.sidebar_hidden {
                "Show Sidebar"
            } else {
                "Hide Sidebar"
            },
            cx,
        );
        rmac_ui::set_menu_label(
            "mail::ToggleToolbar",
            if self.toolbar_hidden {
                "Show Toolbar"
            } else {
                "Hide Toolbar"
            },
            cx,
        );
        enable("mail::ExpandAllConversations", state.threads, cx);
        enable("mail::CollapseAllConversations", state.threads, cx);

        enable("mail::NewMessage", true, cx);
        enable("mail::Search", true, cx);
        enable("mail::Archive", self.can_archive_selected(), cx);
        for action in [
            "mail::Junk",
            "mail::Flag",
            "mail::Move",
            "mail::Copy",
            "mail::ToggleRead",
            "mail::OpenMessage",
            "mail::SaveAs",
            "mail::ExportAsPdf",
            "mail::PrintMessage",
            "mail::Reply",
            "mail::ReplyAll",
            "mail::Forward",
            "mail::ForwardAsAttachment",
            "mail::Redirect",
            "mail::ShowRawSource",
            "mail::JumpToTop",
        ] {
            enable(action, has_selection, cx);
        }
        rmac_ui::set_menu_label(
            "mail::ToggleRead",
            if selected.is_some_and(|message| message.unread) {
                "Mark as Read"
            } else {
                "Mark as Unread"
            },
            cx,
        );
        rmac_ui::set_menu_label(
            "mail::Junk",
            if selected
                .is_some_and(|message| message.mailbox.special_use() == Some(SpecialUse::Junk))
            {
                "Mark as Not Junk"
            } else {
                "Move to Junk"
            },
            cx,
        );
        enable(
            "mail::ClearFlag",
            selected.is_some_and(|message| message.flagged),
            cx,
        );
        enable(
            "mail::SaveAttachments",
            selected.is_some_and(|message| {
                message.attachment.is_some() && message.body_state == rmac_mail::BodyState::Loaded
            }),
            cx,
        );
        enable("mail::MoveAgain", state.move_again_target().is_some(), cx);
        for (index, action) in [
            (0, "mail::MoveToFavourite0"),
            (2, "mail::MoveToFavourite2"),
            (3, "mail::MoveToFavourite3"),
        ] {
            enable(
                action,
                state.favourite_target(&FAVOURITES[index]).is_some(),
                cx,
            );
        }
        enable(
            "mail::PreviousMailbox",
            state.adjacent_mailbox(-1, false).is_some(),
            cx,
        );
        enable(
            "mail::NextMailbox",
            state.adjacent_mailbox(1, false).is_some(),
            cx,
        );
        enable(
            "mail::PreviousUnreadMailbox",
            state.adjacent_mailbox(-1, true).is_some(),
            cx,
        );
        enable(
            "mail::NextUnreadMailbox",
            state.adjacent_mailbox(1, true).is_some(),
            cx,
        );
        enable("mail::MarkAllAsRead", state.has_unread_visible(), cx);
        let syncing = self.runtime.is_some() && !self.accounts.is_empty();
        enable("mail::GetAllNewMail", syncing, cx);
        let bin = Erase {
            special: SpecialUse::Trash,
            account: None,
        };
        let any_deleted = state.erasable_count(bin) > 0;
        enable("mail::EraseDeletedItems", any_deleted, cx);
        enable("mail::EraseDeletedItemsNow", any_deleted, cx);
        enable(
            "mail::EraseJunkMail",
            state.erasable_count(Erase {
                special: SpecialUse::Junk,
                account: None,
            }) > 0,
            cx,
        );
        if self.active {
            // A compose window re-labels and enables these while it is in
            // front (`compose_window.rs`).
            enable("mail::Undo", state.can_undo(), cx);
            enable("mail::Redo", state.can_redo(), cx);
            enable("mail::Delete", has_selection, cx);
            rmac_ui::set_menu_label("mail::SendMessage", "Send Again", cx);
            enable(
                "mail::SendMessage",
                selected
                    .is_some_and(|message| message.mailbox.special_use() == Some(SpecialUse::Sent)),
                cx,
            );
            enable("mail::SaveDraft", false, cx);
            enable("mail::PasteAsQuotation", false, cx);
        }
        self.publish_dynamic_rows(syncing, cx);
        self.sync_tools_menu(cx);
        self.sync_mailbox_menu(cx);
    }

    /// Message ▸ Move to ▸ / Copy to ▸, Mailbox ▸ Get New Mail ▸ and Erase
    /// Deleted Items ▸, built from the mailboxes and accounts Mail has now.
    fn publish_dynamic_rows(&self, syncing: bool, cx: &mut Context<Self>) {
        let destinations = self.state.destinations();
        for (prefix, menu) in [
            ("mail::MoveToMailbox", "mail::MoveToMenu"),
            ("mail::CopyToMailbox", "mail::CopyToMenu"),
        ] {
            let mut rows: Vec<MenuItem> = destinations
                .iter()
                .take(MAILBOX_ROWS)
                .enumerate()
                .map(|(index, mailbox)| {
                    MenuItem::new(mailbox.label(), format!("{prefix}{index}"), "")
                })
                .collect();
            if rows.is_empty() {
                rows.push(MenuItem::new("No Mailboxes", format!("{prefix}0"), "").enabled(false));
            }
            rmac_ui::set_menu_children(menu, rows, cx);
        }
        let accounts = self.accounts.iter().take(ACCOUNT_ROWS);
        let mut get_new: Vec<MenuItem> = accounts
            .clone()
            .enumerate()
            .map(|(index, account)| {
                MenuItem::new(
                    account.address.clone(),
                    format!("mail::GetNewMail{index}"),
                    "",
                )
                .enabled(syncing)
            })
            .collect();
        if get_new.is_empty() {
            get_new.push(MenuItem::new("No Accounts", "mail::GetNewMail0", "").enabled(false));
        }
        rmac_ui::set_menu_children("mail::GetNewMailMenu", get_new, cx);
        let any_deleted = self.state.erasable_count(Erase {
            special: SpecialUse::Trash,
            account: None,
        }) > 0;
        let mut erase = vec![
            MenuItem::new("In All Accounts…", "mail::EraseDeletedItems", "⇧⌘⌫")
                .enabled(any_deleted)
                .with_alternate(
                    MenuItem::new("In All Accounts", "mail::EraseDeletedItemsNow", "⌥⇧⌘⌫")
                        .enabled(any_deleted),
                ),
        ];
        for (index, account) in accounts.enumerate() {
            let count = self.state.erasable_count(Erase {
                special: SpecialUse::Trash,
                account: Some(account.id),
            });
            let mut row = MenuItem::new(
                format!("{}…", account.address),
                format!("mail::EraseDeletedItemsAccount{index}"),
                "",
            )
            .enabled(count > 0);
            if index == 0 {
                row = row.separated();
            }
            erase.push(row);
        }
        rmac_ui::set_menu_children("mail::EraseDeletedItemsMenu", erase, cx);
    }

    /// What every selection change needs: the body loaded, the read mark
    /// saved and the menus brought up to date.
    pub(super) fn after_selection(&mut self, cx: &mut Context<Self>) {
        self.find_current = 0;
        self.find_scroll = true;
        self.load_all_headers(cx);
        self.ensure_body_loaded(cx);
        self.dispatch_persist(cx);
        self.sync_menu(cx);
        cx.notify();
    }

    fn open_mailbox(&mut self, mailbox: Mailbox, cx: &mut Context<Self>) {
        self.state.select_mailbox(mailbox);
        self.viewer_scroll.set_offset(gpui::point(px(0.0), px(0.0)));
        self.after_selection(cx);
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        self.sync_menu(cx);
        cx.notify();
    }

    fn sort_by(&mut self, key: SortKey, cx: &mut Context<Self>) {
        self.state.set_sort(key);
        self.changed(cx);
    }

    fn toggle_filter_row(&mut self, row: FilterRow, cx: &mut Context<Self>) {
        let filter = &mut self.state.filter;
        let flag = match row {
            FilterRow::Unread => &mut filter.unread,
            FilterRow::Flagged => &mut filter.flagged,
            FilterRow::ToMe => &mut filter.to_me,
            FilterRow::CcMe => &mut filter.cc_me,
            FilterRow::Attachments => &mut filter.attachments,
        };
        *flag = !*flag;
        self.changed(cx);
    }

    fn toggle_view_option(&mut self, option: ViewOption, cx: &mut Context<Self>) {
        let flag = match option {
            ViewOption::ColumnLayout => &mut self.column_layout,
            ViewOption::BottomPreview => &mut self.bottom_preview,
            ViewOption::DateAndTime => &mut self.show_date_time,
            ViewOption::ToCcLabel => &mut self.show_to_cc,
            ViewOption::ContactPhoto => &mut self.show_contact_photo,
            ViewOption::HighlightConversations => &mut self.highlight_conversations,
            ViewOption::MessageSize => &mut self.show_size,
            ViewOption::Sidebar => &mut self.sidebar_hidden,
            ViewOption::Toolbar => &mut self.toolbar_hidden,
        };
        *flag = !*flag;
        self.changed(cx);
    }

    fn compose(&mut self, kind: ComposeKind, prefill: Option<rmac_mail_mime::Draft>, cx: &mut App) {
        crate::compose_window::open(
            kind,
            self.state.selected_message().cloned(),
            prefill,
            self.accounts.clone(),
            self.compose_candidates(),
            cx,
        );
    }

    /// The selected message's original source when its body is cached, off
    /// the UI thread; `None` for fixture data and header-only messages.
    pub(super) fn load_raw_source(&self, cx: &mut Context<Self>) -> gpui::Task<Option<Vec<u8>>> {
        let target = self
            .state
            .selected_message()
            .and_then(|message| match &message.mailbox {
                Mailbox::Real(real) => message.row_id.map(|row_id| (real.account, row_id)),
                _ => None,
            });
        cx.background_executor().spawn(async move {
            let (account, row_id) = target?;
            crate::live::raw_source(account, row_id)
        })
    }

    /// Message ▸ Forward as Attachment: the original message rides along
    /// as a `message/rfc822` part — its real source when cached.
    fn forward_as_attachment(&mut self, cx: &mut Context<Self>) {
        let Some(message) = self.state.selected_message().cloned() else {
            return;
        };
        let raw = self.load_raw_source(cx);
        cx.spawn(async move |this, cx| {
            let prefill = raw.await.map(|bytes| rmac_mail_mime::Draft {
                attachments: vec![rmac_mail_mime::Attachment {
                    filename: format!("{}.eml", rmac_mail::safe_file_stem(&message.subject)),
                    content_type: "message/rfc822".to_owned(),
                    bytes,
                    content_id: None,
                }],
                ..rmac_mail_mime::Draft::default()
            });
            let _ = this.update(cx, |this, cx| {
                this.compose(ComposeKind::ForwardAsAttachment, prefill, cx)
            });
        })
        .detach();
    }

    /// File ▸ Save As…: the message as an `.eml` file, its original source
    /// when Mail has it.
    fn save_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(message) = self.state.selected_message().cloned() else {
            return;
        };
        let raw = self.load_raw_source(cx);
        let name = format!("{}.eml", rmac_mail::safe_file_stem(&message.subject));
        let chosen = cx.prompt_for_new_path(&save_directory(), Some(&name));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = chosen.await else {
                return;
            };
            let bytes = raw
                .await
                .unwrap_or_else(|| rmac_mail::message_source(&message).into_bytes());
            let written = cx
                .background_executor()
                .spawn(async move { write_file(&path, &bytes) })
                .await;
            if let Err(error) = written {
                let _ = this.update(cx, |this, cx| {
                    this.notice = Some(format!("Mail couldn’t save the message: {error}.").into());
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// File ▸ Save Attachments…: the selected message's attachment, under
    /// its own name.
    fn save_attachments(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(attachment) = self
            .state
            .selected_message()
            .and_then(|message| message.attachment.clone())
        else {
            return;
        };
        let name = rmac_mail::safe_file_stem(&attachment.filename);
        let chosen = cx.prompt_for_new_path(&save_directory(), Some(&name));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = chosen.await else {
                return;
            };
            let bytes = attachment.bytes;
            let written = cx
                .background_executor()
                .spawn(async move { write_file(&path, &bytes) })
                .await;
            if let Err(error) = written {
                let _ = this.update(cx, |this, cx| {
                    this.notice =
                        Some(format!("Mail couldn’t save the attachment: {error}.").into());
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// File ▸ Export as PDF…: the message laid out as File ▸ Print… lays it
    /// out, straight to a PDF file.
    fn export_pdf(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(message) = self.state.selected_message() else {
            return;
        };
        let (title, text) = rmac_mail::printable_message(message);
        let name = format!("{}.pdf", rmac_mail::safe_file_stem(&title));
        let chosen = cx.prompt_for_new_path(&save_directory(), Some(&name));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = chosen.await else {
                return;
            };
            let written = cx
                .background_executor()
                .spawn(async move {
                    let pdf = rmac_print::render_pdf(&text, rmac_print::PageLayout::default())
                        .map_err(|error| error.to_string())?;
                    write_file(&path, &pdf).map_err(|error| error.to_string())
                })
                .await;
            if let Err(error) = written {
                let _ = this.update(cx, |this, cx| {
                    this.notice = Some(format!("Mail couldn’t export the PDF: {error}.").into());
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// File ▸ Print… (⌘P) through the desktop Print portal, as Notes and
    /// Text Editor print.
    #[cfg(target_os = "linux")]
    fn print(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use std::sync::atomic::Ordering;

        let Some(message) = self.state.selected_message() else {
            return;
        };
        let (title, text) = rmac_mail::printable_message(message);
        let raw_window =
            raw_window_handle::HasWindowHandle::window_handle(window).map(|handle| handle.as_raw());
        let raw_display = raw_window_handle::HasDisplayHandle::display_handle(window)
            .map(|handle| handle.as_raw());
        let (Ok(raw_window), Ok(raw_display)) = (raw_window, raw_display) else {
            self.notice = Some("Mail couldn’t open the print dialog.".into());
            cx.notify();
            return;
        };
        let generation = self.print_generation.fetch_add(1, Ordering::AcqRel) + 1;
        let request = rmac_print_linux::PrintDocument {
            window: raw_window,
            display: raw_display,
            window_generation: 1,
            document_generation: generation,
            current_document_generation: self.print_generation.clone(),
            title,
            text,
            rich: None,
        };
        cx.spawn_in(window, async move |this, cx| {
            if let Err(error) = rmac_print_linux::print_document(request).await {
                let _ = this.update(cx, |this, cx| {
                    this.notice = Some(format!("Mail couldn’t print the message: {error}.").into());
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// View ▸ Message ▸ Raw Source (⌥⌘U): the message's source in a window
    /// of its own.
    fn show_raw_source(&mut self, cx: &mut Context<Self>) {
        let Some(message) = self.state.selected_message().cloned() else {
            return;
        };
        let live = message.row_id.is_some() && message.mailbox.is_real();
        let raw = self.load_raw_source(cx);
        cx.spawn(async move |_, cx| {
            let source = match raw.await {
                Some(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                None if live => {
                    "Mail hasn’t downloaded this message yet. Open it to download it, then show its source again.".to_owned()
                }
                None => rmac_mail::message_source(&message),
            };
            cx.update(|cx| super::windows::open_source(&message.subject, source, cx));
        })
        .detach();
    }

    /// Mailbox ▸ Get All New Mail / Get New Mail ▸ account: wakes each
    /// account's sync worker now.
    fn get_new_mail(&mut self, only: Option<usize>) {
        let Some(runtime) = self.runtime.clone() else {
            return;
        };
        for (index, account) in self.accounts.iter().enumerate() {
            if only.is_none_or(|only| only == index) {
                runtime.sync_now(&account.path);
            }
        }
    }

    fn get_new_mail_for(&mut self, index: usize, _: &mut Window, _: &mut Context<Self>) {
        self.get_new_mail(Some(index));
    }

    fn erase(&mut self, erase: Erase, cx: &mut Context<Self>) {
        self.erase_confirm = None;
        if self.state.erase(erase) > 0 {
            self.dispatch_persist(cx);
        }
        self.after_selection(cx);
    }

    fn ask_to_erase(&mut self, erase: Erase, cx: &mut Context<Self>) {
        if self.state.erasable_count(erase) > 0 {
            self.erase_confirm = Some(erase);
            cx.notify();
        }
    }

    fn erase_account(&mut self, index: usize, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(account) = self.accounts.get(index) {
            let erase = Erase {
                special: SpecialUse::Trash,
                account: Some(account.id),
            };
            self.ask_to_erase(erase, cx);
        }
    }

    fn go_to_favourite(&mut self, index: usize, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(mailbox) = self.state.favourites().get(index).cloned() {
            self.open_mailbox(mailbox, cx);
        }
    }

    fn move_to_favourite(&mut self, index: usize, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(target) = self.state.favourite_target(&FAVOURITES[index]) {
            self.perform(rmac_mail::OrganizeAction::Move(target), cx);
        }
    }

    fn move_to_mailbox(&mut self, index: usize, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(target) = self.state.destinations().get(index).cloned() {
            self.perform(rmac_mail::OrganizeAction::Move(target), cx);
        }
    }

    fn copy_to_mailbox(&mut self, index: usize, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(target) = self.state.destinations().get(index).cloned() {
            self.perform(rmac_mail::OrganizeAction::Copy(target), cx);
        }
    }

    fn step_mailbox(&mut self, step: i32, unread_only: bool, cx: &mut Context<Self>) {
        if let Some(mailbox) = self.state.adjacent_mailbox(step, unread_only) {
            self.open_mailbox(mailbox, cx);
        }
    }

    /// The list and viewer, arranged as View ▸ Use Column Layout and Show
    /// Bottom Preview ask, beside the sidebar unless it is hidden.
    pub(super) fn content(&self, list: AnyElement, viewer: AnyElement) -> AnyElement {
        let left = if self.sidebar_hidden {
            0.0
        } else {
            super::SIDEBAR + 8.0
        };
        let top = match (self.toolbar_hidden, self.sidebar_hidden) {
            (false, _) => super::TOOLBAR,
            // The window controls need a strip of their own.
            (true, true) => 36.0,
            (true, false) => 0.0,
        };
        let pane = div()
            .absolute()
            .left(px(left))
            .right_0()
            .top(px(top))
            .bottom_0()
            .flex();
        if self.bottom_preview {
            pane.flex_col()
                .child(
                    div()
                        .w_full()
                        .h(relative(0.42))
                        .flex_none()
                        .border_b_1()
                        .border_color(mac::separator())
                        .child(list),
                )
                .child(div().flex_1().min_h(px(0.0)).flex().child(viewer))
                .into_any_element()
        } else if self.column_layout {
            pane.child(div().flex_1().min_w(px(0.0)).h_full().child(list))
                .child(div().w(px(460.0)).flex_none().h_full().flex().child(viewer))
                .into_any_element()
        } else {
            pane.child(div().w(px(super::LIST)).h_full().flex_none().child(list))
                .child(viewer)
                .into_any_element()
        }
    }

    pub(super) fn erase_alert(&self, erase: Erase, cx: &mut Context<Self>) -> AnyElement {
        let place = match (erase.special, erase.account) {
            (SpecialUse::Junk, _) => "in Junk mailboxes".to_owned(),
            (_, None) => "in all accounts".to_owned(),
            (_, Some(id)) => self
                .accounts
                .iter()
                .find(|account| account.id == id)
                .map_or_else(
                    || "in this account".to_owned(),
                    |account| format!("in “{}”", account.address),
                ),
        };
        let what = if erase.special == SpecialUse::Junk {
            "all messages"
        } else {
            "all deleted messages"
        };
        rmac_ui::alert_cancel_default(
            format!("Are you sure you want to permanently erase {what} {place}?"),
            "You can’t undo this action.",
            vec![
                rmac_ui::dialog_button("mail-erase-cancel", "Cancel", DialogButtonKind::Normal)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.erase_confirm = None;
                        cx.notify();
                    }))
                    .into_any_element(),
                rmac_ui::dialog_button(
                    "mail-erase-confirm",
                    "Erase",
                    DialogButtonKind::Destructive,
                )
                .on_click(cx.listener(move |this, _, _, cx| this.erase(erase, cx)))
                .into_any_element(),
            ],
        )
        .into_any_element()
    }

    pub(super) fn notice_alert(&self, notice: SharedString, cx: &mut Context<Self>) -> AnyElement {
        rmac_ui::alert(
            notice,
            "",
            vec![
                rmac_ui::dialog_button("mail-notice-ok", "OK", DialogButtonKind::Primary)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.notice = None;
                        cx.notify();
                    }))
                    .into_any_element(),
            ],
        )
        .into_any_element()
    }

    /// The handlers for every menu command this module adds.
    pub(super) fn on_menu_actions(&self, root: gpui::Div, cx: &mut Context<Self>) -> gpui::Div {
        use crate::*;
        let root = root
            .on_action(cx.listener(|_, _: &ShowAccounts, _, cx| open_internet_accounts(cx)))
            .on_action(cx.listener(|this, _: &OpenMessage, _, cx| {
                if let Some(message) = this.state.selected_message().cloned() {
                    super::windows::open_message(message, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SaveAs, window, cx| this.save_as(window, cx)))
            .on_action(cx.listener(|this, _: &SaveAttachments, window, cx| {
                this.save_attachments(window, cx)
            }))
            .on_action(cx.listener(|this, _: &ExportAsPdf, window, cx| this.export_pdf(window, cx)))
            .on_action(cx.listener(|this, _: &PrintMessage, window, cx| {
                #[cfg(target_os = "linux")]
                this.print(window, cx);
                #[cfg(not(target_os = "linux"))]
                let _ = (this, window, cx);
            }))
            .on_action(cx.listener(|this, _: &Redo, _, cx| {
                if this.state.redo() {
                    this.dispatch_persist(cx);
                    this.changed(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &JumpToTop, _, cx| {
                this.viewer_scroll.set_offset(gpui::point(px(0.0), px(0.0)));
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &TransformUppercase, window, cx| {
                this.transform_search(rmac_ui::TextTransformation::Uppercase, window, cx)
            }))
            .on_action(cx.listener(|this, _: &TransformLowercase, window, cx| {
                this.transform_search(rmac_ui::TextTransformation::Lowercase, window, cx)
            }))
            .on_action(cx.listener(|this, _: &TransformCapitalise, window, cx| {
                this.transform_search(rmac_ui::TextTransformation::Capitalise, window, cx)
            }))
            .on_action(cx.listener(|this, _: &StartSpeaking, window, cx| {
                // The search field while it has focus, else the open message.
                if this
                    .search_input
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
                {
                    rmac_ui::start_speaking(&this.search_input, "mail::StopSpeaking", cx);
                } else if let Some(message) = this.state.selected_message() {
                    rmac_ui::speak(message.body.plain_text(), "mail::StopSpeaking", cx);
                }
            }))
            .on_action(cx.listener(|_, _: &StopSpeaking, _, _| rmac_ui::stop_speaking()))
            .on_action(cx.listener(|this, _: &SortByAttachments, _, cx| {
                this.sort_by(SortKey::Attachments, cx)
            }))
            .on_action(cx.listener(|this, _: &SortByDate, _, cx| this.sort_by(SortKey::Date, cx)))
            .on_action(cx.listener(|this, _: &SortByFlags, _, cx| this.sort_by(SortKey::Flags, cx)))
            .on_action(cx.listener(|this, _: &SortByFrom, _, cx| this.sort_by(SortKey::From, cx)))
            .on_action(
                cx.listener(|this, _: &SortBySubject, _, cx| this.sort_by(SortKey::Subject, cx)),
            )
            .on_action(cx.listener(|this, _: &SortByTo, _, cx| this.sort_by(SortKey::To, cx)))
            .on_action(
                cx.listener(|this, _: &SortByUnread, _, cx| this.sort_by(SortKey::Unread, cx)),
            )
            .on_action(cx.listener(|this, _: &SortAscending, _, cx| {
                this.state.descending = false;
                this.changed(cx);
            }))
            .on_action(cx.listener(|this, _: &SortDescending, _, cx| {
                this.state.descending = true;
                this.changed(cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleMessageFilter, _, cx| {
                this.state.filter.enabled = !this.state.filter.enabled;
                this.changed(cx);
            }))
            .on_action(cx.listener(|this, _: &FilterUnread, _, cx| {
                this.toggle_filter_row(FilterRow::Unread, cx)
            }))
            .on_action(cx.listener(|this, _: &FilterFlagged, _, cx| {
                this.toggle_filter_row(FilterRow::Flagged, cx)
            }))
            .on_action(cx.listener(|this, _: &FilterToMe, _, cx| {
                this.toggle_filter_row(FilterRow::ToMe, cx)
            }))
            .on_action(cx.listener(|this, _: &FilterCcMe, _, cx| {
                this.toggle_filter_row(FilterRow::CcMe, cx)
            }))
            .on_action(cx.listener(|this, _: &FilterAttachments, _, cx| {
                this.toggle_filter_row(FilterRow::Attachments, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleColumnLayout, _, cx| {
                this.toggle_view_option(ViewOption::ColumnLayout, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleBottomPreview, _, cx| {
                this.toggle_view_option(ViewOption::BottomPreview, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleDateAndTime, _, cx| {
                this.toggle_view_option(ViewOption::DateAndTime, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleToCcLabel, _, cx| {
                this.toggle_view_option(ViewOption::ToCcLabel, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleContactPhoto, _, cx| {
                this.toggle_view_option(ViewOption::ContactPhoto, cx)
            }))
            .on_action(
                cx.listener(|this, _: &ToggleHighlightConversations, _, cx| {
                    this.toggle_view_option(ViewOption::HighlightConversations, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
                this.toggle_view_option(ViewOption::Sidebar, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleToolbar, _, cx| {
                this.toggle_view_option(ViewOption::Toolbar, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleMessageSize, _, cx| {
                this.toggle_view_option(ViewOption::MessageSize, cx)
            }))
            .on_action(cx.listener(|this, _: &SortBySize, _, cx| this.sort_by(SortKey::Size, cx)))
            .on_action(cx.listener(|this, _: &NewViewerWindow, _, cx| this.new_viewer(cx)))
            .on_action(cx.listener(|this, _: &QuickLookAttachments, _, cx| this.quick_look(cx)))
            .on_action(
                cx.listener(|this, _: &FindInMessage, window, cx| this.open_find(window, cx)),
            )
            .on_action(cx.listener(|this, _: &FindNext, _, cx| this.step_find(1, cx)))
            .on_action(cx.listener(|this, _: &FindPrevious, _, cx| this.step_find(-1, cx)))
            .on_action(cx.listener(|this, _: &JumpToSelection, _, cx| this.jump_to_find(cx)))
            .on_action(
                cx.listener(|this, _: &CloseFindBar, window, cx| this.close_find(window, cx)),
            )
            .on_action(cx.listener(|this, _: &ShowAllHeaders, _, cx| this.toggle_all_headers(cx)))
            .on_action(cx.listener(|this, _: &FlagOrange, _, cx| {
                this.flag_with(rmac_mail::FlagColour::Orange, cx)
            }))
            .on_action(cx.listener(|this, _: &FlagRed, _, cx| {
                this.flag_with(rmac_mail::FlagColour::Red, cx)
            }))
            .on_action(cx.listener(|this, _: &FlagPurple, _, cx| {
                this.flag_with(rmac_mail::FlagColour::Purple, cx)
            }))
            .on_action(cx.listener(|this, _: &FlagBlue, _, cx| {
                this.flag_with(rmac_mail::FlagColour::Blue, cx)
            }))
            .on_action(cx.listener(|this, _: &FlagYellow, _, cx| {
                this.flag_with(rmac_mail::FlagColour::Yellow, cx)
            }))
            .on_action(cx.listener(|this, _: &FlagGreen, _, cx| {
                this.flag_with(rmac_mail::FlagColour::Green, cx)
            }))
            .on_action(cx.listener(|this, _: &FlagGrey, _, cx| {
                this.flag_with(rmac_mail::FlagColour::Grey, cx)
            }))
            .on_action(cx.listener(|this, _: &ExpandAllConversations, _, cx| {
                this.state.expand_all_conversations();
                this.changed(cx);
            }))
            .on_action(cx.listener(|this, _: &CollapseAllConversations, _, cx| {
                this.state.collapse_all_conversations();
                this.changed(cx);
            }))
            .on_action(cx.listener(|this, _: &ShowRawSource, _, cx| this.show_raw_source(cx)))
            .on_action(cx.listener(|this, _: &GetAllNewMail, _, _| this.get_new_mail(None)))
            .on_action(cx.listener(|this, _: &MarkAllAsRead, _, cx| {
                if this.state.mark_all_read() > 0 {
                    this.dispatch_persist(cx);
                    this.changed(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &EraseDeletedItems, _, cx| {
                this.ask_to_erase(
                    Erase {
                        special: SpecialUse::Trash,
                        account: None,
                    },
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &EraseDeletedItemsNow, _, cx| {
                this.erase(
                    Erase {
                        special: SpecialUse::Trash,
                        account: None,
                    },
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &EraseJunkMail, _, cx| {
                this.ask_to_erase(
                    Erase {
                        special: SpecialUse::Junk,
                        account: None,
                    },
                    cx,
                )
            }))
            .on_action(
                cx.listener(|this, _: &PreviousMailbox, _, cx| this.step_mailbox(-1, false, cx)),
            )
            .on_action(cx.listener(|this, _: &NextMailbox, _, cx| this.step_mailbox(1, false, cx)))
            .on_action(
                cx.listener(|this, _: &PreviousUnreadMailbox, _, cx| {
                    this.step_mailbox(-1, true, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &NextUnreadMailbox, _, cx| this.step_mailbox(1, true, cx)),
            )
            .on_action(cx.listener(|this, _: &SendMessage, _, cx| {
                let sent = this
                    .state
                    .selected_message()
                    .is_some_and(|message| message.mailbox.special_use() == Some(SpecialUse::Sent));
                if sent {
                    this.compose(ComposeKind::SendAgain, None, cx);
                }
            }))
            .on_action(
                cx.listener(|this, _: &ForwardAsAttachment, _, cx| this.forward_as_attachment(cx)),
            )
            .on_action(cx.listener(|this, _: &Redirect, _, cx| {
                if this.state.selected_message().is_some() {
                    this.compose(ComposeKind::Redirect, None, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ClearFlag, _, cx| {
                if this
                    .state
                    .selected_message()
                    .is_some_and(|message| message.flagged)
                {
                    this.perform(rmac_mail::OrganizeAction::Flag, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &MoveAgain, _, cx| {
                if let Some(target) = this.state.move_again_target() {
                    this.perform(rmac_mail::OrganizeAction::Move(target), cx);
                }
            }));
        let root = indexed!(root, cx, go_to_favourite, [
            GoToFavourite0 => 0, GoToFavourite1 => 1, GoToFavourite2 => 2, GoToFavourite3 => 3,
        ]);
        let root = indexed!(root, cx, move_to_favourite, [
            MoveToFavourite0 => 0, MoveToFavourite2 => 2, MoveToFavourite3 => 3,
        ]);
        let root = indexed!(root, cx, get_new_mail_for, [
            GetNewMail0 => 0, GetNewMail1 => 1, GetNewMail2 => 2, GetNewMail3 => 3,
            GetNewMail4 => 4, GetNewMail5 => 5, GetNewMail6 => 6, GetNewMail7 => 7,
        ]);
        let root = indexed!(root, cx, erase_account, [
            EraseDeletedItemsAccount0 => 0, EraseDeletedItemsAccount1 => 1,
            EraseDeletedItemsAccount2 => 2, EraseDeletedItemsAccount3 => 3,
            EraseDeletedItemsAccount4 => 4, EraseDeletedItemsAccount5 => 5,
            EraseDeletedItemsAccount6 => 6, EraseDeletedItemsAccount7 => 7,
        ]);
        let root = indexed!(root, cx, move_to_mailbox, [
            MoveToMailbox0 => 0, MoveToMailbox1 => 1, MoveToMailbox2 => 2, MoveToMailbox3 => 3,
            MoveToMailbox4 => 4, MoveToMailbox5 => 5, MoveToMailbox6 => 6, MoveToMailbox7 => 7,
            MoveToMailbox8 => 8, MoveToMailbox9 => 9, MoveToMailbox10 => 10,
            MoveToMailbox11 => 11, MoveToMailbox12 => 12, MoveToMailbox13 => 13,
            MoveToMailbox14 => 14, MoveToMailbox15 => 15, MoveToMailbox16 => 16,
            MoveToMailbox17 => 17, MoveToMailbox18 => 18, MoveToMailbox19 => 19,
            MoveToMailbox20 => 20, MoveToMailbox21 => 21, MoveToMailbox22 => 22,
            MoveToMailbox23 => 23,
        ]);
        indexed!(root, cx, copy_to_mailbox, [
            CopyToMailbox0 => 0, CopyToMailbox1 => 1, CopyToMailbox2 => 2, CopyToMailbox3 => 3,
            CopyToMailbox4 => 4, CopyToMailbox5 => 5, CopyToMailbox6 => 6, CopyToMailbox7 => 7,
            CopyToMailbox8 => 8, CopyToMailbox9 => 9, CopyToMailbox10 => 10,
            CopyToMailbox11 => 11, CopyToMailbox12 => 12, CopyToMailbox13 => 13,
            CopyToMailbox14 => 14, CopyToMailbox15 => 15, CopyToMailbox16 => 16,
            CopyToMailbox17 => 17, CopyToMailbox18 => 18, CopyToMailbox19 => 19,
            CopyToMailbox20 => 20, CopyToMailbox21 => 21, CopyToMailbox22 => 22,
            CopyToMailbox23 => 23,
        ])
    }

    /// Edit ▸ Transformations on the search field, the one text field the
    /// viewer has.
    fn transform_search(
        &mut self,
        transformation: rmac_ui::TextTransformation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let search = self.search_input.clone();
        if search.read(cx).focus_handle(cx).is_focused(window)
            && rmac_ui::transform_selection(&search, transformation, window, cx)
        {
            cx.notify();
        }
    }
}
