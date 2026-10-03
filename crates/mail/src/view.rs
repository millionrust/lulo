use std::sync::Arc;

use gpui::{
    div, px, uniform_list, AnyElement, ClickEvent, Context, FocusHandle, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window,
};
use rmac_mail::{MailState, Mailbox, Message};
use rmac_mail_mime::{BlockKind, RichText};
use rmac_ui::mac;

use crate::{
    CloseWindow, NextMessage, PreviousMessage, ToggleRead, ToggleThreads, ToggleUnreadFilter,
};

const SIDEBAR: f32 = 220.0;
const LIST: f32 = 340.0;
const TOOLBAR: f32 = 52.0;

pub struct MailView {
    pub focus: FocusHandle,
    state: MailState,
}

#[derive(Clone)]
struct Row {
    id: &'static str,
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
    pub fn new(state: MailState, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            state,
        }
    }

    fn sync_menu(&self, cx: &mut Context<Self>) {
        rmac_ui::set_menu_checked("mail::ToggleThreads", self.state.threads, cx);
        rmac_ui::set_menu_checked("mail::ToggleUnreadFilter", self.state.unread_only, cx);
    }

    fn control(
        &self,
        id: &'static str,
        glyph: &'static str,
        label: &'static str,
        enabled: bool,
        selected: bool,
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
            .bg(if selected {
                mac::control_fill()
            } else {
                mac::material_clear()
            })
            .text_color(if enabled {
                mac::text()
            } else {
                mac::text_tertiary()
            })
            .text_size(px(17.0))
            .child(glyph);
        if enabled {
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
            true,
            self.state.unread_only,
            |this, cx| {
                this.state.unread_only = !this.state.unread_only;
                this.sync_menu(cx);
                cx.notify();
            },
            cx,
        ));
        bar = bar.child(div().w(px(LIST - 66.0)));
        for (id, glyph, label) in [
            ("mail-compose", "▣", "Compose"),
            ("mail-archive", "▤", "Archive"),
            ("mail-trash", "⌫", "Delete"),
            ("mail-junk", "⊗", "Junk"),
            ("mail-reply", "↶", "Reply"),
            ("mail-reply-all", "↞", "Reply All"),
            ("mail-forward", "↷", "Forward"),
            ("mail-flag", "⚑", "Flag"),
            ("mail-move", "▱", "Move"),
            ("mail-search", "⌕", "Search"),
        ] {
            bar = bar.child(self.control(id, glyph, label, false, false, |_, _| {}, cx));
        }
        bar.into_any_element()
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut list = div()
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

    fn list(&self, cx: &mut Context<Self>) -> AnyElement {
        let visible = self.state.visible();
        let rows: Arc<Vec<Row>> = Arc::new(
            visible
                .iter()
                .map(|&index| {
                    let message = &self.state.messages[index];
                    Row {
                        id: message.id,
                        sender: message.sender,
                        subject: message.subject,
                        preview: message.preview,
                        date: message.date,
                        unread: message.unread,
                        flagged: message.flagged,
                        attachment: message.attachment.is_some(),
                        thread_count: self.state.thread_count(message.thread_id),
                        selected: self.state.selected == Some(message.id),
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
                                let id = row.id;
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
                                            this.state.select(id);
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
                if let Some(message) = this
                    .state
                    .messages
                    .iter_mut()
                    .find(|message| Some(message.id) == this.state.selected)
                {
                    message.unread = !message.unread;
                    cx.notify();
                }
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
