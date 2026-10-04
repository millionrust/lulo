use std::sync::Arc;

use gpui::{
    div, prelude::FluentBuilder as _, px, uniform_list, AnyElement, AppContext as _, ClickEvent,
    Context, Entity, FocusHandle, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Role, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window,
};
use rmac_editor::InputState;
use rmac_mail::{compose::ComposeKind, MailState, Mailbox, Message, OrganizeAction, SearchScope};
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
}

#[derive(Clone)]
struct Row {
    id: String,
    sender: &'static str,
    subject: &'static str,
    preview: &'static str,
    date: &'static str,
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
        Self {
            focus: cx.focus_handle(),
            state,
            search_input,
            search_open: false,
            destination_menu: false,
            copy_destination: false,
            accounts,
        }
    }

    fn sync_menu(&self, cx: &mut Context<Self>) {
        rmac_ui::set_menu_checked("mail::ToggleThreads", self.state.threads, cx);
        rmac_ui::set_menu_checked("mail::ToggleUnreadFilter", self.state.unread_only, cx);
        rmac_ui::set_menu_enabled("mail::Undo", self.state.can_undo(), cx);
        rmac_ui::set_menu_enabled(
            "mail::Archive",
            self.state
                .selected_message()
                .is_some_and(|message| message.mailbox.account() == "Google"),
            cx,
        );
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

    fn perform(&mut self, action: OrganizeAction, cx: &mut Context<Self>) {
        if self.state.apply(action) {
            self.destination_menu = false;
            self.sync_menu(cx);
            cx.notify();
        }
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
            // Archive has no destination on iCloud (special_mailbox returns
            // None), so grey it out there instead of a silent no-op click.
            let available = self.state.selected_message().is_some_and(|message| {
                action != OrganizeAction::Archive || message.mailbox.account() != "iCloud"
            });
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
                move |this, cx| this.perform(action, cx),
                cx,
            ));
        }
        for (id, glyph, label) in [
            ("mail-reply", "↶", "Reply"),
            ("mail-reply-all", "↞", "Reply All"),
            ("mail-forward", "↷", "Forward"),
        ] {
            bar = bar.child(self.control(id, glyph, label, ControlMode::Disabled, |_, _| {}, cx));
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
        let mut account = "";
        for mailbox in Mailbox::ALL {
            if mailbox.account() != account {
                account = mailbox.account();
                list = list.child(
                    div()
                        .h(px(30.0))
                        .pt(px(10.0))
                        .pl(px(8.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(11.0))
                        .text_color(mac::text_secondary())
                        .child(account),
                );
            }
            let selected = self.state.mailbox == mailbox;
            let count = self.state.unread_count(mailbox);
            let label = if count > 0 {
                format!("{}, {} unread", mailbox.label(), count)
            } else {
                mailbox.label().to_owned()
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
                    .child(div().w(px(16.0)).child(mailbox_icon(mailbox)))
                    .child(mailbox.label())
                    .child(div().flex_1())
                    .child(if count > 0 {
                        count.to_string()
                    } else {
                        String::new()
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.state.select_mailbox(mailbox);
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
        for mailbox in Mailbox::ALL
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
                            OrganizeAction::Copy(mailbox)
                        } else {
                            OrganizeAction::Move(mailbox)
                        };
                        this.perform(action, cx);
                    })),
            );
        }
        panel.into_any_element()
    }

    fn list(&self, cx: &mut Context<Self>) -> AnyElement {
        let visible = self.state.visible();
        let rows: Arc<Vec<Row>> = Arc::new(
            visible
                .iter()
                .map(|&index| {
                    let message = &self.state.messages[index];
                    Row {
                        id: message.id.clone(),
                        sender: message.sender,
                        subject: message.subject,
                        preview: message.preview,
                        date: message.date,
                        unread: message.unread,
                        flagged: message.flagged,
                        attachment: message.attachment.is_some(),
                        thread_count: self.state.thread_count(message.thread_id),
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
                                                            .child(row.sender),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_size(px(10.0))
                                                            .text_color(mac::text_secondary())
                                                            .child(row.date),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .flex()
                                                    .gap(px(4.0))
                                                    .text_size(px(12.0))
                                                    .text_color(mac::text())
                                                    .child(row.subject)
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
                                                    .child(row.preview),
                                            ),
                                    )
                                    .on_click(move |_: &ClickEvent, _, cx| {
                                        let _ = view.update(cx, |this, cx| {
                                            this.state.select(&id);
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

    fn viewer(&self) -> AnyElement {
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
        body = body.child(self.rich_body(&message.body));
        if let Some((name, size)) = message.attachment {
            body = body.child(
                div()
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
                    .child("▤")
                    .child(format!("{name} · {size}")),
            );
        }
        body.into_any_element()
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
                    .child(message.initials),
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
                            .child(message.sender),
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
                            .child(message.subject),
                    ),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(mac::text_secondary())
                    .child(message.date),
            )
            .into_any_element()
    }
}

impl Render for MailView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                    this.accounts.clone(),
                    this.compose_candidates(),
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &Reply, _, cx| {
                compose_window::open(
                    ComposeKind::Reply,
                    this.state.selected_message().cloned(),
                    this.accounts.clone(),
                    this.compose_candidates(),
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &ReplyAll, _, cx| {
                compose_window::open(
                    ComposeKind::ReplyAll,
                    this.state.selected_message().cloned(),
                    this.accounts.clone(),
                    this.compose_candidates(),
                    cx,
                )
            }))
            .on_action(cx.listener(|this, _: &Forward, _, cx| {
                compose_window::open(
                    ComposeKind::Forward,
                    this.state.selected_message().cloned(),
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
                    .child(self.viewer()),
            )
            .child(self.sidebar(cx))
            .when(self.destination_menu, |root| {
                root.child(self.destinations(cx))
            })
    }
}

fn mailbox_icon(mailbox: Mailbox) -> &'static str {
    match mailbox {
        Mailbox::AllInboxes | Mailbox::GoogleInbox | Mailbox::IcloudInbox => "▤",
        Mailbox::Flagged => "⚑",
        Mailbox::Drafts | Mailbox::GoogleDrafts => "✎",
        Mailbox::Sent | Mailbox::GoogleSent | Mailbox::IcloudSent => "➤",
        Mailbox::GoogleJunk | Mailbox::IcloudJunk => "⊗",
        Mailbox::GoogleTrash | Mailbox::IcloudTrash => "⌫",
        Mailbox::GoogleArchive => "▥",
        Mailbox::GoogleReceipts => "▱",
    }
}
