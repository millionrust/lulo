//! Separate compose window. Input changes stay on the GPUI thread; MIME and
//! mailbox writes are dispatched by `ComposeWorker`.

use gpui::{
    div, prelude::FluentBuilder as _, px, App, AppContext as _, ClickEvent, Context, Entity,
    FocusHandle, Focusable as _, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    Role, StatefulInteractiveElement as _, Styled as _, Window,
};
use rmac_editor::InputState;
use rmac_mail::compose::{addresses, initial_draft, ComposeKind};
use rmac_mail::Message;
use rmac_mail_mime::Draft;
use rmac_ui::{mac, AccessibleTextInput as _, InputEvent, Root, TextField};

use crate::{
    delivery::{ComposeAccount, DeliveryResult},
    AttachFile, CloseWindow, SendMessage,
};

pub fn open(
    kind: ComposeKind,
    message: Option<Message>,
    accounts: Vec<ComposeAccount>,
    cx: &mut App,
) {
    let options = rmac_ui::window_options_for_app(rmac_ui::app_id::MAIL, 640.0, 560.0, cx);
    match cx.open_window(options, |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        window.set_window_title("New Message");
        let view = cx.new(|cx| ComposeView::new(kind, message.as_ref(), accounts, window, cx));
        let focus = view.read(cx).to.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        cx.new(|cx| Root::new(view, window, cx))
    }) {
        Ok(handle) => {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
        }
        Err(error) => eprintln!("rmac-mail: could not open compose window: {error}"),
    }
}

struct ComposeView {
    focus: FocusHandle,
    to: Entity<InputState>,
    cc: Entity<InputState>,
    bcc: Entity<InputState>,
    subject: Entity<InputState>,
    body: Entity<InputState>,
    accounts: Vec<ComposeAccount>,
    selected_account: usize,
    pending: bool,
    quote: Draft,
    status: Option<String>,
}

impl ComposeView {
    fn new(
        kind: ComposeKind,
        message: Option<&Message>,
        accounts: Vec<ComposeAccount>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let from = accounts
            .first()
            .map(|account| account.address.as_str())
            .unwrap_or_default();
        let quote = initial_draft(kind, message, from);
        let to = cx.new(|cx| InputState::new(window, cx).placeholder("Add recipients"));
        let cc = cx.new(|cx| InputState::new(window, cx).placeholder("Add Cc recipients"));
        let bcc = cx.new(|cx| InputState::new(window, cx).placeholder("Add Bcc recipients"));
        let subject = cx.new(|cx| InputState::new(window, cx).placeholder("Subject"));
        let body = rmac_editor::multiline("Message", window, cx);
        to.update(cx, |input, cx| {
            input.set_value(quote.to.join(", "), window, cx)
        });
        cc.update(cx, |input, cx| {
            input.set_value(quote.cc.join(", "), window, cx)
        });
        subject.update(cx, |input, cx| {
            input.set_value(quote.subject.clone(), window, cx)
        });
        body.update(cx, |input, cx| {
            input.set_value(quote.text.clone(), window, cx)
        });
        for input in [&to, &cc, &bcc, &subject, &body] {
            cx.subscribe(input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.status = None;
                    cx.notify();
                }
            })
            .detach();
        }
        Self {
            focus: cx.focus_handle(),
            to,
            cc,
            bcc,
            subject,
            body,
            accounts,
            selected_account: 0,
            pending: false,
            quote,
            status: None,
        }
    }

    fn draft(&self, cx: &Context<Self>) -> Result<Draft, &'static str> {
        let mut draft = self.quote.clone();
        draft.from = self
            .accounts
            .get(self.selected_account)
            .map(|account| account.address.clone())
            .unwrap_or_default();
        draft.to = addresses(&self.to.read(cx).value())?;
        draft.cc = addresses(&self.cc.read(cx).value())?;
        draft.bcc = addresses(&self.bcc.read(cx).value())?;
        draft.subject = self.subject.read(cx).value().to_string();
        draft.text = self.body.read(cx).value().to_string();
        Ok(draft)
    }

    fn send(&mut self, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        let Ok(draft) = self.draft(cx) else {
            self.status = Some("Check the recipient addresses".into());
            cx.notify();
            return;
        };
        if draft.to.is_empty() && draft.cc.is_empty() && draft.bcc.is_empty() {
            self.status = Some("Add at least one recipient".into());
            cx.notify();
            return;
        }
        let Some(account) = self.accounts.get(self.selected_account).cloned() else {
            self.status = Some("Add an Internet Account to send mail".into());
            cx.notify();
            return;
        };
        self.pending = true;
        self.status = Some("Sending…".into());
        cx.notify();
        let (sender, receiver) = async_channel::bounded(1);
        if std::thread::Builder::new()
            .name("rmac-mail-send".into())
            .spawn(move || {
                let result = crate::delivery::deliver(&account, draft);
                let _ = sender.send_blocking(result);
            })
            .is_err()
        {
            self.pending = false;
            self.status = Some("Mail could not start the send worker".into());
            cx.notify();
            return;
        }
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            if let Ok(result) = receiver.recv().await {
                let _ = this.update(cx, |this, cx| {
                    this.pending = false;
                    this.status = Some(match result {
                        DeliveryResult::Sent => "Sent".to_owned(),
                        DeliveryResult::Queued => {
                            "Saved to Outbox. Mail will send when the connection is available"
                                .to_owned()
                        }
                        DeliveryResult::Failed(message) => message.to_owned(),
                    });
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn next_account(&mut self, cx: &mut Context<Self>) {
        if !self.accounts.is_empty() {
            self.selected_account = (self.selected_account + 1) % self.accounts.len();
            cx.notify();
        }
    }

    fn field(
        &self,
        label: &'static str,
        input: &Entity<InputState>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .h(px(35.0))
            .px(px(16.0))
            .flex()
            .items_center()
            .border_b_1()
            .border_color(mac::separator())
            .child(
                div()
                    .w(px(68.0))
                    .text_size(px(12.0))
                    .text_color(mac::text_secondary())
                    .child(label),
            )
            .child(
                div()
                    .id(format!(
                        "compose-{}",
                        label.trim_end_matches(':').to_lowercase()
                    ))
                    .role(Role::TextInput)
                    .aria_label(label.trim_end_matches(':'))
                    .accessible_text_input(input, cx)
                    .flex_1()
                    .child(TextField::new(input).appearance(false).px_0().py_0()),
            )
    }
}

impl Render for ComposeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(mac::window())
            .track_focus(&self.focus)
            .key_context("MailCompose")
            .on_action(cx.listener(|this, _: &SendMessage, _, cx| this.send(cx)))
            .on_action(cx.listener(|this, _: &AttachFile, _, cx| {
                this.status = Some("Attachment selection is unavailable".into());
                cx.notify();
            }))
            .on_action(cx.listener(|_, _: &CloseWindow, window, _| window.remove_window()))
            .on_action(
                cx.listener(|_, _: &rmac_ui::RequestClose, window, _| window.remove_window()),
            )
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(52.0))
                    .px(px(16.0))
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .border_b_1()
                    .border_color(mac::separator())
                    .child(
                        div()
                            .id("compose-send")
                            .role(Role::Button)
                            .aria_label("Send Message")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.send(cx)))
                            .rounded(px(mac::radius_control()))
                            .bg(mac::accent())
                            .text_color(mac::white())
                            .px(px(12.0))
                            .py(px(6.0))
                            .child("Send"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(13.0))
                            .text_color(mac::text())
                            .child("New Message"),
                    )
                    .child(
                        div()
                            .id("compose-attach")
                            .role(Role::Button)
                            .aria_label("Attach File")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.status = Some("Attachment selection is unavailable".into());
                                cx.notify();
                            }))
                            .text_color(mac::text())
                            .child("Attach"),
                    ),
            )
            .child(self.field("To:", &self.to, cx))
            .child(self.field("Cc:", &self.cc, cx))
            .child(self.field("Bcc:", &self.bcc, cx))
            .child(self.field("Subject:", &self.subject, cx))
            .child(
                div()
                    .id("compose-from")
                    .role(Role::Button)
                    .aria_label("From Account")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.next_account(cx)))
                    .h(px(35.0))
                    .px(px(16.0))
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(mac::separator())
                    .text_size(px(12.0))
                    .text_color(mac::text_secondary())
                    .child(format!(
                        "From: {} ▾",
                        self.accounts
                            .get(self.selected_account)
                            .map(|account| account.address.as_str())
                            .unwrap_or("No Account")
                    )),
            )
            .child(
                div()
                    .id("compose-body")
                    .role(Role::MultilineTextInput)
                    .aria_label("Message Body")
                    .accessible_text_input(&self.body, cx)
                    .flex_1()
                    .min_h(px(0.0))
                    .px(px(16.0))
                    .py(px(12.0))
                    .child(
                        TextField::new(&self.body)
                            .h_full()
                            .appearance(false)
                            .px_0()
                            .py_0(),
                    ),
            )
            .when_some(self.status.clone(), |parent, status| {
                parent.child(
                    div()
                        .px(px(16.0))
                        .py(px(8.0))
                        .text_size(px(12.0))
                        .text_color(mac::danger())
                        .child(status),
                )
            })
    }
}
