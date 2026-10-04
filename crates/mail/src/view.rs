use std::sync::Arc;

use gpui::{
    div, prelude::FluentBuilder as _, px, uniform_list, AnyElement, AppContext as _, ClickEvent,
    Context, Entity, FocusHandle, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Role, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window,
};
use rmac_editor::InputState;
use rmac_mail::{
    compose::ComposeKind, MailState, Mailbox, Message, OrganizeAction, SearchScope, SpecialUse,
};
use rmac_mail_mime::{BlockKind, RichText};
use rmac_ui::{mac, AccessibleTextInput as _, InputEvent, TextField};

use crate::{
    compose_window, delivery::ComposeAccount, Archive, CloseWindow, Copy, Delete, Flag, Forward,
    Junk, Move, NewMessage, NextMessage, PreviousMessage, Reply, ReplyAll, Search, ToggleRead,
    ToggleThreads, ToggleUnreadFilter, Undo,
};

const SIDEBAR: f32 = 220.0;
const LIST: f32 = 340.0;
const TOOLBAR: f32 = 52.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ControlMode {
    Disabled,
    Enabled,
    Selected,
}

pub struct MailView {
    pub focus: FocusHandle,
    state: MailState,
    search_input: Entity<InputState>,
    search_open: bool,
    destination_menu: bool,
    copy_destination: bool,
    accounts: Vec<ComposeAccount>,
    /// `None` under `RMAC_MAIL_FIXTURE=1` and in tests; otherwise the live
    /// sync runtime, kept only to wake the right account's worker right
    /// after an organise action queues its journal entry.
    runtime: Option<Arc<rmac_mail_runtime::Runtime>>,
}

#[derive(Clone)]
struct Row {
    id: String,
    sender: String,
    subject: String,
    preview: String,
    date: String,
    unread: bool,
    flagged: bool,
    attachment: bool,
    thread_count: usize,
    selected: bool,
}

impl MailView {
    pub fn new(
        state: MailState,
        accounts: Vec<ComposeAccount>,
        runtime: Option<Arc<rmac_mail_runtime::Runtime>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder("Search Mail"));
        cx.subscribe(&search_input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let query = this.search_input.read(cx).value().to_owned();
                this.state.set_search(&query);
                cx.notify();
            }
        })
        .detach();
        let mut view = Self {
            focus: cx.focus_handle(),
            state,
            search_input,
            search_open: false,
            destination_menu: false,
            copy_destination: false,
            accounts,
            runtime,
        };
        view.ensure_body_loaded(cx);
        view
    }

    /// Replaces the mailbox/message list with a freshly loaded one, keeping
    /// the current selection, mailbox and search — called after a sync
    /// snapshot or new-mail event (`main.rs`'s background event loop).
    pub fn refresh_live(&mut self, mailboxes: Vec<Mailbox>, messages: Vec<Message>, cx: &mut Context<Self>) {
        self.state.refresh_live(mailboxes, messages);
        self.ensure_body_loaded(cx);
        self.sync_menu(cx);
        cx.notify();
    }

    /// The `Message::id` for a live notification's `(account, row_id)`
    /// pair — `rmac_mail_runtime`'s `NewMail`/`OpenMessage` events only
    /// carry the account and the storage row id, not Mail's own composite
    /// id string.
    pub fn message_id_for_row(&self, account: uuid::Uuid, row_id: i64) -> Option<String> {
        self.state
            .messages
            .iter()
            .find(|message| {
                message.row_id == Some(row_id)
                    && matches!(&message.mailbox, Mailbox::Real(real) if real.account == account)
            })
            .map(|message| message.id.clone())
    }

    /// Selects `id` (for example from a clicked new-mail notification),
    /// opening its mailbox first if needed.
    pub fn open_message(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(message) = self.state.messages.iter().find(|message| message.id == id) {
            self.state.mailbox = message.mailbox.clone();
        }
        self.state.select(id);
        self.ensure_body_loaded(cx);
        self.dispatch_persist(cx);
        self.sync_menu(cx);
        cx.notify();
    }

    fn sync_menu(&self, cx: &mut Context<Self>) {
        rmac_ui::set_menu_checked("mail::ToggleThreads", self.state.threads, cx);
        rmac_ui::set_menu_checked("mail::ToggleUnreadFilter", self.state.unread_only, cx);
        rmac_ui::set_menu_enabled("mail::Undo", self.state.can_undo(), cx);
        rmac_ui::set_menu_enabled("mail::Archive", self.can_archive_selected(), cx);
        for action in [
            "mail::Delete",
            "mail::Junk",
            "mail::Flag",
            "mail::Move",
            "mail::Copy",
            "mail::ToggleRead",
        ] {
            rmac_ui::set_menu_enabled(action, self.state.selected_message().is_some(), cx);
        }
    }

    /// Whether the selected message's account has an Archive mailbox at
    /// all (iCloud, for example, has none) — decides whether the toolbar's
    /// Archive button is a real action or greyed out.
    fn can_archive_selected(&self) -> bool {
        self.state.selected_message().is_some_and(|message| {
            matches!(&message.mailbox, Mailbox::Real(real) if self
                .state
                .find_special(real.account, SpecialUse::Archive)
                .is_some())
        })
    }

    fn perform(&mut self, action: OrganizeAction, cx: &mut Context<Self>) {
        if self.state.apply(action) {
            self.destination_menu = false;
            self.dispatch_persist(cx);
            self.sync_menu(cx);
            cx.notify();
        }
    }

    /// Replays whatever `Persist` the last mutating `MailState` call queued
    /// into that account's storage journal, off the GPUI thread, and wakes
    /// its sync worker so the change reaches the server promptly instead of
    /// waiting for the next IDLE timeout. A no-op for fixture data, which
    /// never produces a `Persist`.
    fn dispatch_persist(&mut self, cx: &mut Context<Self>) {
        let Some(persist) = self.state.take_persist() else {
            return;
        };
        let runtime = self.runtime.clone();
        cx.background_executor()
            .spawn(async move {
                crate::live::persist(persist.clone());
                if let Some(runtime) = runtime {
                    runtime.sync_now(&persist.account_path);
                }
            })
            .detach();
    }

    /// Fetches and parses the selected message's real body in the
    /// background, the first time it is opened. A fixture message or one
    /// already loaded is a no-op (MAIL-10: never parses MIME for rows
    /// nobody has opened).
    fn ensure_body_loaded(&mut self, cx: &mut Context<Self>) {
        let Some(message) = self.state.selected_message() else {
            return;
        };
        if message.body_loaded {
            return;
        }
        let Mailbox::Real(real) = message.mailbox.clone() else {
            return;
        };
        let Some(row_id) = message.row_id else {
            return;
        };
        let id = message.id.clone();
        cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_executor()
                .spawn(async move { crate::live::load_body(real.account, real.mailbox_id, row_id) })
                .await;
            if let Some((body, attachment)) = loaded {
                let _ = this.update(cx, |this, cx| {
                    this.state.set_loaded_body(&id, body, attachment);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Address completion candidates for a freshly opened compose window:
    /// every distinct sender this mailbox has seen (`compose::known_recipients`).
    fn compose_candidates(&self) -> Vec<rmac_mail::compose::Recipient> {
        rmac_mail::compose::known_recipients(&self.state.messages)
    }

    fn control(
        &self,
        id: &'static str,
        glyph: &'static str,
        label: &'static str,
        mode: ControlMode,
        click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut button = div()
            .id(id)
            .role(Role::Button)
            .aria_label(label)
            .w(px(30.0))
            .h(px(28.0))
            .rounded(px(mac::radius_control()))
            .flex()
            .items_center()
            .justify_center()
            .bg(if mode == ControlMode::Selected {
                mac::control_fill()
            } else {
                mac::material_clear()
            })
            .text_color(if mode != ControlMode::Disabled {
                mac::text()
            } else {
                mac::text_tertiary()
            })
            .text_size(px(17.0))
            .child(glyph);
        if mode != ControlMode::Disabled {
            button =
                button.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| click(this, cx)));
        }
        button.into_any_element()
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut bar = div()
            .h(px(TOOLBAR))
            .pl(px(SIDEBAR + 8.0))
            .pr(px(10.0))
            .flex()
            .items_center()
            .gap(px(3.0))
            .border_b_1()
            .border_color(mac::separator());
        bar = bar.child(self.control(
            "mail-filter",
            "☷",
            "Filter Unread",
            if self.state.unread_only {
                ControlMode::Selected
            } else {
                ControlMode::Enabled
            },
            |this, cx| {
                this.state.unread_only = !this.state.unread_only;
                this.sync_menu(cx);
                cx.notify();
            },
            cx,
        ));
        bar = bar.child(div().w(px(LIST - 66.0)));
        bar = bar.child(self.control(
            "mail-compose",
            "▣",
            "Compose",
            ControlMode::Enabled,
            |this, cx| {
                compose_window::open(
                    ComposeKind::New,
                    None,
                    None,
                    this.accounts.clone(),
                    this.compose_candidates(),
                    cx,
                );
            },
            cx,
        ));
        for (id, glyph, label, action) in [
            ("mail-archive", "▤", "Archive", OrganizeAction::Archive),
            ("mail-trash", "⌫", "Move to Bin", OrganizeAction::Delete),
            ("mail-junk", "⊗", "Junk or Not Junk", OrganizeAction::Junk),
        ] {
            // Some accounts (iCloud, for example) have no Archive mailbox,
            // so grey the button out there instead of a silent no-op click.
            let available =
                action != OrganizeAction::Archive || self.can_archive_selected();
            let mode = if available {
                ControlMode::Enabled
            } else {
                ControlMode::Disabled
            };
            bar = bar.child(self.control(
                id,
                glyph,
                label,
                mode,
                move |this, cx| this.perform(action.clone(), cx),
                cx,
            ));
        }
        let has_selection = self.state.selected_message().is_some();
        for (id, glyph, label, kind) in [
            ("mail-reply", "↶", "Reply", ComposeKind::Reply),
            ("mail-reply-all", "↞", "Reply All", ComposeKind::ReplyAll),
            ("mail-forward", "↷", "Forward", ComposeKind::Forward),
        ] {
            let mode = if has_selection {
                ControlMode::Enabled
            } else {
                ControlMode::Disabled
            };
            bar = bar.child(self.control(
                id,
                glyph,
                label,
                mode,
                move |this, cx| {
                    compose_window::open(
                        kind,
                        this.state.selected_message().cloned(),
                        None,
                        this.accounts.clone(),
                        this.compose_candidates(),
                        cx,
                    );
                },
                cx,
            ));
        }
        bar = bar.child(self.control(
            "mail-flag",
            "⚑",
            "Flag",
            ControlMode::Enabled,
            |this, cx| this.perform(OrganizeAction::Flag, cx),
            cx,
        ));
        bar = bar.child(self.control(
            "mail-move",
            "▱",
            "Move or Copy to Mailbox",
            ControlMode::Enabled,
            |this, cx| {
                this.destination_menu = !this.destination_menu;
                cx.notify();
            },
            cx,
        ));
        bar = bar.child(self.control(
            "mail-search",
            "⌕",
            "Search",
            ControlMode::Enabled,
            |this, cx| {
                this.search_open = !this.search_open;
                cx.notify();
            },
            cx,
        ));
        if self.search_open {
            bar = bar.child(
                div()
                    .id("mail-search-field")
                    .role(Role::SearchInput)
                    .aria_label("Search Mail")
                    .accessible_text_input(&self.search_input, cx)
                    .w(px(170.0))
                    .h(px(28.0))
                    .rounded(px(mac::radius_control()))
                    .bg(mac::control_fill())
                    .child(
                        TextField::new(&self.search_input)
                            .appearance(false)
                            .cleanable(true)
                            .small(),
                    ),
            );
            let all = self.state.search_scope == SearchScope::AllMailboxes;
            bar = bar.child(self.control(
                "mail-search-scope",
                if all { "All" } else { "Here" },
                if all {
                    "Search All Mailboxes"
                } else {
                    "Search Current Mailbox"
                },
                ControlMode::Enabled,
                |this, cx| {
                    this.state.search_scope =
                        if this.state.search_scope == SearchScope::AllMailboxes {
                            SearchScope::CurrentMailbox
                        } else {
                            SearchScope::AllMailboxes
                        };
                    cx.notify();
                },
                cx,
            ));
        }
        bar.into_any_element()
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut list = div()
            .id("mail-sidebar-scroll")
            .role(Role::ListBox)
            .aria_label("Mailboxes")
            .absolute()
            .top(px(48.0))
            .left(px(10.0))
            .right(px(10.0))
            .bottom(px(12.0))
            .overflow_y_scroll()
            .flex()
            .flex_col();
        let mut account = String::new();
        for mailbox in self.state.mailboxes.clone() {
            if mailbox.account() != account {
                account = mailbox.account().to_owned();
                list = list.child(
                    div()
                        .h(px(30.0))
                        .pt(px(10.0))
                        .pl(px(8.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(11.0))
                        .text_color(mac::text_secondary())
                        .child(account.clone()),
                );
            }
            let selected = self.state.mailbox == mailbox;
            let count = self.state.unread_count(&mailbox);
            let label = if count > 0 {
                format!("{}, {} unread", mailbox.label(), count)
            } else {
                mailbox.label()
            };
            list = list.child(
                div()
                    .id(format!("mailbox-{mailbox:?}"))
                    .role(Role::ListBoxOption)
                    .aria_label(label)
                    .aria_selected(selected)
                    .h(px(27.0))
                    .px(px(8.0))
                    .rounded(px(mac::radius_control()))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .bg(if selected {
                        mac::accent()
                    } else {
                        mac::material_clear()
                    })
                    .text_color(if selected { mac::white() } else { mac::text() })
                    .text_size(px(12.0))
                    .child(div().w(px(16.0)).child(mailbox_icon(&mailbox)))
                    .child(mailbox.label())
                    .child(div().flex_1())
                    .child(if count > 0 {
                        count.to_string()
                    } else {
                        String::new()
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.state.select_mailbox(mailbox.clone());
                        this.ensure_body_loaded(cx);
                        this.dispatch_persist(cx);
                        this.sync_menu(cx);
                        cx.notify();
                    })),
            );
        }
        div()
            .absolute()
            .left(px(8.0))
            .top(px(8.0))
            .bottom(px(8.0))
            .w(px(SIDEBAR - 8.0))
            .rounded(px(mac::radius_large_surface()))
            .bg(mac::material_sidebar())
            .border_1()
            .border_color(mac::separator())
            .child(rmac_ui::traffic_lights())
            .child(list)
            .into_any_element()
    }

    fn destinations(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(selected) = self.state.selected_message() else {
            return div().into_any_element();
        };
        let account = selected.mailbox.account();
        let mut panel = div()
            .id("mail-destination-menu")
            .role(Role::Menu)
            .aria_label(if self.copy_destination {
                "Copy to Mailbox"
            } else {
                "Move to Mailbox"
            })
            .absolute()
            .top(px(48.0))
            .right(px(104.0))
            .w(px(196.0))
            .p(px(6.0))
            .rounded(px(mac::radius_card()))
            .bg(mac::material_sidebar())
            .border_1()
            .border_color(mac::separator())
            .flex()
            .flex_col();
        for mailbox in self
            .state
            .mailboxes
            .clone()
            .into_iter()
            .filter(|mailbox| mailbox.is_real() && mailbox.account() == account)
        {
            let label = format!("{} · {}", mailbox.account(), mailbox.label());
            panel = panel.child(
                div()
                    .id(format!("mail-destination-{mailbox:?}"))
                    .role(Role::MenuItem)
                    .aria_label(label.clone())
                    .h(px(27.0))
                    .px(px(8.0))
                    .rounded(px(mac::radius_control()))
                    .text_size(px(12.0))
                    .text_color(mac::text())
                    .flex()
                    .items_center()
                    .child(label)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        let action = if this.copy_destination {
                            OrganizeAction::Copy(mailbox.clone())
                        } else {
                            OrganizeAction::Move(mailbox.clone())
                        };
                        this.perform(action, cx);
                    })),
            );
        }
        panel.into_any_element()
    }

    fn list(&self, cx: &mut Context<Self>) -> AnyElement {
        let visible = self.state.visible();
        // `MailState::thread_count` scans every message; calling it once
        // per visible row would make building the list O(n²) in a 10 000
        // -message mailbox (MAIL-10). One pass here keeps it O(n).
        let mut thread_counts: std::collections::HashMap<&str, usize> =
            std::collections::HashMap::with_capacity(self.state.messages.len());
        for message in &self.state.messages {
            *thread_counts.entry(message.thread_id.as_str()).or_insert(0) += 1;
        }
        let rows: Arc<Vec<Row>> = Arc::new(
            visible
                .iter()
                .map(|&index| {
                    let message = &self.state.messages[index];
                    Row {
                        id: message.id.clone(),
                        sender: message.sender.clone(),
                        subject: message.subject.clone(),
                        preview: message.preview.clone(),
                        date: message.date.clone(),
                        unread: message.unread,
                        flagged: message.flagged,
                        attachment: message.attachment.is_some(),
                        thread_count: thread_counts
                            .get(message.thread_id.as_str())
                            .copied()
                            .unwrap_or(1),
                        selected: self.state.selected.as_deref() == Some(message.id.as_str()),
                    }
                })
                .collect(),
        );
        let view = cx.entity().downgrade();
        let count = rows.len();
        let unread = visible
            .iter()
            .filter(|&&index| self.state.messages[index].unread)
            .count();
        div()
            .id("mail-conversations")
            .role(Role::ListBox)
            .aria_label(format!("{count} conversations, {unread} unread"))
            .w(px(LIST))
            .h_full()
            .flex()
            .flex_col()
            .bg(mac::list())
            .border_r_1()
            .border_color(mac::separator())
            .child(
                div()
                    .h(px(64.0))
                    .px(px(15.0))
                    .pt(px(10.0))
                    .flex()
                    .flex_col()
                    .border_b_1()
                    .border_color(mac::separator())
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .text_size(px(20.0))
                            .text_color(mac::text())
                            .child(self.state.mailbox.label()),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(mac::text_secondary())
                            .child(format!("{count} conversations, {unread} unread")),
                    ),
            )
            .child(
                uniform_list(
                    "mail-message-list",
                    rows.len(),
                    move |range, _window, _cx| {
                        range
                            .map(|index| {
                                let row = &rows[index];
                                let id = row.id.clone();
                                let view = view.clone();
                                let accessible = format!(
                                    "{}{}{}, {}, {}, {}{}",
                                    if row.unread { "Unread, " } else { "" },
                                    if row.flagged { "flagged, " } else { "" },
                                    row.sender,
                                    row.subject,
                                    row.date,
                                    row.preview,
                                    if row.attachment { ", 1 attachment" } else { "" }
                                );
                                div()
                                    .id(format!("mail-message-{}", row.id))
                                    .role(Role::ListBoxOption)
                                    .aria_label(accessible)
                                    .aria_selected(row.selected)
                                    .h(px(78.0))
                                    .px(px(13.0))
                                    .pt(px(7.0))
                                    .flex()
                                    .gap(px(7.0))
                                    .bg(if row.selected {
                                        mac::accent_subtle()
                                    } else {
                                        mac::material_clear()
                                    })
                                    .border_b_1()
                                    .border_color(mac::separator())
                                    .child(
                                        div()
                                            .w(px(9.0))
                                            .pt(px(9.0))
                                            .text_size(px(12.0))
                                            .text_color(mac::system_blue())
                                            .child(if row.unread {
                                                "●"
                                            } else if row.flagged {
                                                "⚑"
                                            } else {
                                                ""
                                            }),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .overflow_hidden()
                                            .flex()
                                            .flex_col()
                                            .child(
                                                div()
                                                    .flex()
                                                    .justify_between()
                                                    .gap(px(5.0))
                                                    .child(
                                                        div()
                                                            .font_weight(if row.unread {
                                                                FontWeight::BOLD
                                                            } else {
                                                                FontWeight::NORMAL
                                                            })
                                                            .text_size(px(13.0))
                                                            .text_color(mac::text())
                                            .child(row.sender.clone()),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_size(px(10.0))
                                                            .text_color(mac::text_secondary())
                                                            .child(row.date.clone()),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .flex()
                                                    .gap(px(4.0))
                                                    .text_size(px(12.0))
                                                    .text_color(mac::text())
                                                    .child(row.subject.clone())
                                                    .child(if row.thread_count > 1 {
                                                        format!("({})", row.thread_count)
                                                    } else {
                                                        String::new()
                                                    })
                                                    .child(if row.attachment { "⌁" } else { "" }),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(11.0))
                                                    .text_color(mac::text_secondary())
                                                    .overflow_hidden()
                                                    .child(row.preview.clone()),
                                            ),
                                    )
                                    .on_click(move |_: &ClickEvent, _, cx| {
                                        let _ = view.update(cx, |this, cx| {
                                            this.state.select(&id);
                                            this.ensure_body_loaded(cx);
                                            this.dispatch_persist(cx);
                                            this.sync_menu(cx);
                                            cx.notify();
                                        });
                                    })
                            })
                            .collect()
                    },
                )
                .flex_1(),
            )
            .into_any_element()
    }

    fn rich_body(&self, body: &RichText) -> AnyElement {
        let mut content = div().flex().flex_col().gap(px(11.0));
        for (index, block) in body.blocks.iter().enumerate() {
            let mut line = div()
                .id(format!("mail-block-{index}"))
                .flex()
                .flex_wrap()
                .text_size(px(13.0))
                .text_color(mac::text());
            if block.kind == BlockKind::Quote {
                line = line
                    .border_l_3()
                    .border_color(mac::separator())
                    .pl(px(10.0))
                    .text_color(mac::text_secondary());
            }
            if block.kind == BlockKind::ListItem {
                line = line.child("• ");
            }
            for span in &block.spans {
                let mut piece = div().child(SharedString::from(span.text.clone()));
                if span.bold {
                    piece = piece.font_weight(FontWeight::BOLD);
                }
                if span.italic {
                    piece = piece.italic();
                }
                if span.underline || span.link.is_some() {
                    piece = piece.underline();
                }
                if span.link.is_some() {
                    piece = piece.text_color(mac::system_blue());
                }
                line = line.child(piece);
            }
            content = content.child(line);
        }
        content.into_any_element()
    }

    fn viewer(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(message) = self.state.selected_message() else {
            return div()
                .flex_1()
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(mac::text_secondary())
                .child("No message selected")
                .into_any_element();
        };
        let mut body = div()
            .id("mail-viewer-scroll")
            .role(Role::Document)
            .aria_label(format!("{}, {}", message.sender, message.subject))
            .flex_1()
            .h_full()
            .overflow_y_scroll()
            .px(px(24.0))
            .py(px(20.0))
            .flex()
            .flex_col()
            .gap(px(16.0))
            .bg(mac::window())
            .child(self.header(message));
        if !message.body.blocked_remote_images.is_empty() {
            body = body.child(
                div()
                    .py(px(8.0))
                    .border_b_1()
                    .border_color(mac::separator())
                    .text_size(px(12.0))
                    .text_color(mac::text_secondary())
                    .child("This message contains remote content. Images are blocked."),
            );
        }
        if message.body_loaded {
            body = body.child(self.rich_body(&message.body));
            if let Some(attachment) = message.attachment.clone() {
                body = body.child(self.attachment_chip(attachment, cx));
            }
        } else {
            body = body.child(
                div()
                    .text_size(px(13.0))
                    .text_color(mac::text_secondary())
                    .child("Loading message…"),
            );
        }
        body.into_any_element()
    }

    /// An attachment chip. A `.ics` invite (MAIL-8) is also a button that
    /// stages the attachment and hands it to Calendar through the OpenURI
    /// portal, which routes to Calendar because `text/calendar` is
    /// registered to `org.rmac.Calendar.desktop` (`packaging/rmac-apps`).
    fn attachment_chip(
        &self,
        attachment: rmac_mail::MessageAttachment,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_calendar_invite = rmac_mail::ics::has_ics_extension(&attachment.filename);
        let mut chip = div()
            .id("mail-attachment")
            .w(px(170.0))
            .h(px(46.0))
            .rounded(px(mac::radius_card()))
            .bg(mac::control_fill())
            .px(px(10.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .text_size(px(12.0))
            .text_color(mac::text())
            .child(if is_calendar_invite { "▦" } else { "▤" })
            .child(format!(
                "{} · {}",
                attachment.filename, attachment.size_label
            ));
        if is_calendar_invite {
            chip = chip
                .role(Role::Button)
                .aria_label(format!("Add {} to Calendar", attachment.filename))
                .cursor_pointer()
                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                    let filename = attachment.filename.clone();
                    let bytes = attachment.bytes.clone();
                    cx.spawn(async move |_, _cx| {
                        let staged = rmac_mail::ics::stage_for_handoff(&filename, &bytes);
                        match staged {
                            Ok(path) => {
                                if let Err(error) = rmac_portal::open_item(&path).await {
                                    eprintln!(
                                        "rmac-mail: could not hand {filename} to Calendar: {error}"
                                    );
                                }
                            }
                            Err(error) => {
                                eprintln!(
                                    "rmac-mail: could not stage {filename} for Calendar: {error}"
                                );
                            }
                        }
                    })
                    .detach();
                }));
        }
        chip.into_any_element()
    }

    fn header(&self, message: &Message) -> AnyElement {
        let mut details = format!("To: {}", message.to);
        if !message.cc.is_empty() {
            details.push_str(&format!("  ·  Cc: {}", message.cc));
        }
        div()
            .flex()
            .gap(px(10.0))
            .pb(px(16.0))
            .border_b_1()
            .border_color(mac::separator())
            .child(
                div()
                    .w(px(40.0))
                    .h(px(40.0))
                    .rounded_full()
                    .bg(mac::system_gray())
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(mac::white())
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(message.initials.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(15.0))
                            .text_color(mac::text())
                            .child(message.sender.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(mac::text_secondary())
                            .child(details),
                    )
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(13.0))
                            .text_color(mac::text())
                            .child(message.subject.clone()),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(mac::text_secondary())
                    .child(message.date.clone()),
            )
            .into_any_element()
    }

    /// Shown instead of the three-pane window when GOA has no mail
    /// account yet. Mail must never show fixture or sample data to a real
    /// user — this, not a fake mailbox, is what someone with no account
    /// sees (`docs/design/calendar-mail.md` §3).
    fn empty_state(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .size_full()
            .relative()
            .bg(mac::window())
            .track_focus(&self.focus)
            .key_context("Mail")
            .child(rmac_ui::traffic_lights())
            .child(
                div()
                    .id("mail-empty-state")
                    .role(Role::Group)
                    .aria_label("No Mail Accounts")
                    .size_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(14.0))
                    .child(
                        div()
                            .text_size(px(34.0))
                            .text_color(mac::text_tertiary())
                            .child("✉"),
                    )
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(17.0))
                            .text_color(mac::text())
                            .child("No Mail Accounts"),
                    )
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(mac::text_secondary())
                            .child("Add an account to send and receive mail."),
                    )
                    .child(
                        div()
                            .id("mail-empty-add-account")
                            .role(Role::Button)
                            .aria_label("Add Account…")
                            .cursor_pointer()
                            .mt(px(6.0))
                            .px(px(16.0))
                            .h(px(28.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(mac::radius_control()))
                            .bg(mac::accent())
                            .text_color(mac::white())
                            .text_size(px(13.0))
                            .child("Add Account…")
                            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                                cx.spawn(async move |_, _| {
                                    if let Err(error) =
                                        std::process::Command::new("rmac-system-settings")
                                            .arg("--pane")
                                            .arg("internet-accounts")
                                            .spawn()
                                    {
                                        eprintln!(
                                            "rmac-mail: could not open System Settings: {error}"
                                        );
                                    }
                                })
                                .detach();
                            })),
                    ),
            )
            .into_any_element()
    }
}

impl Render for MailView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.accounts.is_empty() {
            return self.empty_state(cx);
        }
        self.sync_menu(cx);
        div()
            .size_full()
            .relative()
            .bg(mac::window())
            .track_focus(&self.focus)
            .key_context("Mail")
            .on_action(cx.listener(|this, _: &NewMessage, _, cx| {
                compose_window::open(
                    ComposeKind::New,
                    None,
                    None,
                    this.accounts.clone(),
                    this.compose_candidates(),
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &Reply, _, cx| {
                compose_window::open(
                    ComposeKind::Reply,
                    this.state.selected_message().cloned(),
                    None,
                    this.accounts.clone(),
                    this.compose_candidates(),
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &ReplyAll, _, cx| {
                compose_window::open(
                    ComposeKind::ReplyAll,
                    this.state.selected_message().cloned(),
                    None,
                    this.accounts.clone(),
                    this.compose_candidates(),
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &Forward, _, cx| {
                compose_window::open(
                    ComposeKind::Forward,
                    this.state.selected_message().cloned(),
                    None,
                    this.accounts.clone(),
                    this.compose_candidates(),
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &ToggleThreads, _, cx| {
                this.state.threads = !this.state.threads;
                this.sync_menu(cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleUnreadFilter, _, cx| {
                this.state.unread_only = !this.state.unread_only;
                this.sync_menu(cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleRead, _, cx| {
                let action = if this
                    .state
                    .selected_message()
                    .is_some_and(|message| message.unread)
                {
                    OrganizeAction::MarkRead
                } else {
                    OrganizeAction::MarkUnread
                };
                this.perform(action, cx);
            }))
            .on_action(
                cx.listener(|this, _: &Archive, _, cx| this.perform(OrganizeAction::Archive, cx)),
            )
            .on_action(
                cx.listener(|this, _: &Delete, _, cx| this.perform(OrganizeAction::Delete, cx)),
            )
            .on_action(cx.listener(|this, _: &Junk, _, cx| this.perform(OrganizeAction::Junk, cx)))
            .on_action(cx.listener(|this, _: &Flag, _, cx| this.perform(OrganizeAction::Flag, cx)))
            .on_action(cx.listener(|this, _: &Undo, _, cx| {
                if this.state.undo() {
                    this.dispatch_persist(cx);
                    this.sync_menu(cx);
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &Move, _, cx| {
                this.copy_destination = false;
                this.destination_menu = true;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Copy, _, cx| {
                this.copy_destination = true;
                this.destination_menu = true;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Search, window, cx| {
                this.search_open = true;
                this.search_input
                    .update(cx, |state, cx| state.focus(window, cx));
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &NextMessage, _, cx| {
                this.state.select_next(1);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &PreviousMessage, _, cx| {
                this.state.select_next(-1);
                cx.notify();
            }))
            .on_action(cx.listener(|_, _: &CloseWindow, window, _| window.remove_window()))
            .on_action(
                cx.listener(|_, _: &rmac_ui::RequestClose, window, _| window.remove_window()),
            )
            .on_action(cx.listener(|_, _: &crate::ShowSettings, _, cx| {
                cx.defer(crate::settings_view::show);
            }))
            .child(self.toolbar(cx))
            .child(
                div()
                    .absolute()
                    .left(px(SIDEBAR + 8.0))
                    .right_0()
                    .top(px(TOOLBAR))
                    .bottom_0()
                    .flex()
                    .child(self.list(cx))
                    .child(self.viewer(cx)),
            )
            .child(self.sidebar(cx))
            .when(self.destination_menu, |root| {
                root.child(self.destinations(cx))
            })
            .into_any_element()
    }
}

fn mailbox_icon(mailbox: &Mailbox) -> &'static str {
    match mailbox {
        Mailbox::AllInboxes => "▤",
        Mailbox::Flagged => "⚑",
        Mailbox::Drafts => "✎",
        Mailbox::Sent => "➤",
        Mailbox::Real(real) => match real.special_use {
            Some(SpecialUse::Inbox) => "▤",
            Some(SpecialUse::Drafts) => "✎",
            Some(SpecialUse::Sent) => "➤",
            Some(SpecialUse::Junk) => "⊗",
            Some(SpecialUse::Trash) => "⌫",
            Some(SpecialUse::Archive) => "▥",
            None => "▱",
        },
    }
}
