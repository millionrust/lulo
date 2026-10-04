//! Separate compose window. Input changes stay on the GPUI thread; MIME,
//! attachment reads and mailbox writes run on worker threads.

use std::time::Duration;

use gpui::{
    div, prelude::FluentBuilder as _, px, App, AppContext as _, ClickEvent, Context, Entity,
    FocusHandle, Focusable as _, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    Role, StatefulInteractiveElement as _, Styled as _, Window,
};
use rmac_editor::InputState;
use rmac_mail::compose::{
    addresses, apply_completion, complete, completion_query, initial_draft, ComposeKind, Recipient,
};
use rmac_mail::Message;
use rmac_mail_mime::Draft;
use rmac_ui::{mac, AccessibleTextInput as _, InputEvent, Root, TextField};

use crate::{
    delivery::{ComposeAccount, DeliveryResult, DraftLocation, PendingAttachment},
    AttachFile, CloseWindow, SendMessage,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AddressField {
    To,
    Cc,
    Bcc,
}

pub fn open(
    kind: ComposeKind,
    message: Option<Message>,
    accounts: Vec<ComposeAccount>,
    candidates: Vec<Recipient>,
    cx: &mut App,
) {
    let options = rmac_ui::window_options_for_app(rmac_ui::app_id::MAIL, 640.0, 560.0, cx);
    match cx.open_window(options, |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        window.set_window_title("New Message");
        let view =
            cx.new(|cx| ComposeView::new(kind, message.as_ref(), accounts, candidates, window, cx));
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

/// `212 KB` / `2.4 MB`, matching the Mac's attachment chips.
fn human_size(bytes: usize) -> String {
    const KB: f64 = 1024.0;
    let bytes = bytes as f64;
    if bytes < KB {
        format!("{bytes:.0} B")
    } else if bytes < KB * KB {
        format!("{:.0} KB", bytes / KB)
    } else {
        format!("{:.1} MB", bytes / (KB * KB))
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
    candidates: Vec<Recipient>,
    completion: Option<(AddressField, Vec<Recipient>)>,
    attachments: Vec<PendingAttachment>,
    draft_location: Option<DraftLocation>,
    autosave_generation: u64,
    sent: bool,
}

impl ComposeView {
    fn new(
        kind: ComposeKind,
        message: Option<&Message>,
        accounts: Vec<ComposeAccount>,
        candidates: Vec<Recipient>,
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
        for input in [&subject, &body] {
            cx.subscribe(input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.status = None;
                    this.schedule_autosave(cx);
                    cx.notify();
                }
            })
            .detach();
        }
        for (field, input) in [
            (AddressField::To, &to),
            (AddressField::Cc, &cc),
            (AddressField::Bcc, &bcc),
        ] {
            cx.subscribe_in(
                input,
                window,
                move |this, entity, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => {
                        let value = entity.read(cx).value().to_string();
                        let query = completion_query(&value);
                        let suggestions = complete(query, &this.candidates);
                        this.completion = (!suggestions.is_empty()).then_some((field, suggestions));
                        this.status = None;
                        this.schedule_autosave(cx);
                        cx.notify();
                    }
                    InputEvent::PressEnter { .. } => {
                        if let Some((active, suggestions)) = this.completion.clone() {
                            if active == field {
                                if let Some(chosen) = suggestions.first() {
                                    let value = entity.read(cx).value().to_string();
                                    let updated = apply_completion(&value, chosen);
                                    entity.update(cx, |input, cx| {
                                        input.set_value(updated, window, cx)
                                    });
                                    this.completion = None;
                                    this.schedule_autosave(cx);
                                    cx.notify();
                                }
                            }
                        }
                    }
                    _ => {}
                },
            )
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
            candidates,
            completion: None,
            attachments: Vec::new(),
            draft_location: None,
            autosave_generation: 0,
            sent: false,
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
        draft.attachments = self
            .attachments
            .iter()
            .map(|attachment| rmac_mail_mime::Attachment {
                filename: attachment.filename.clone(),
                content_type: attachment.content_type.clone(),
                bytes: attachment.bytes.as_ref().clone(),
                content_id: None,
            })
            .collect();
        Ok(draft)
    }

    /// Whether there is anything worth keeping as a draft: an empty new
    /// message is not worth a row in Drafts, matching the Mac.
    fn worth_saving(&self, cx: &Context<Self>) -> bool {
        !self.to.read(cx).value().trim().is_empty()
            || !self.cc.read(cx).value().trim().is_empty()
            || !self.bcc.read(cx).value().trim().is_empty()
            || !self.subject.read(cx).value().trim().is_empty()
            || !self.body.read(cx).value().trim().is_empty()
            || !self.attachments.is_empty()
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
        let saved = self.draft_location;
        let (sender, receiver) = async_channel::bounded(1);
        if std::thread::Builder::new()
            .name("rmac-mail-send".into())
            .spawn(move || {
                let result = crate::delivery::deliver(&account, draft, saved);
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
                    let delivered = !matches!(result, DeliveryResult::Failed(_));
                    this.status = Some(match result {
                        DeliveryResult::Sent => "Sent".to_owned(),
                        DeliveryResult::Queued => {
                            "Saved to Outbox. Mail will send when the connection is available"
                                .to_owned()
                        }
                        DeliveryResult::Failed(message) => message.to_owned(),
                    });
                    if delivered {
                        this.sent = true;
                        this.draft_location = None;
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Debounced draft autosave: each edit arms a generation and schedules a
    /// timer; only the timer that still matches the latest edit writes.
    /// Mirrors Text Editor's recovery clock (`document_state.rs`) without a
    /// second long-lived worker thread.
    fn schedule_autosave(&mut self, cx: &mut Context<Self>) {
        if self.sent || !self.worth_saving(cx) {
            return;
        }
        let Some(account) = self.accounts.get(self.selected_account).cloned() else {
            return;
        };
        self.autosave_generation = self.autosave_generation.wrapping_add(1);
        let generation = self.autosave_generation;
        let from = account.address.clone();
        let to = self.to.read(cx).value().to_string();
        let cc = self.cc.read(cx).value().to_string();
        let bcc = self.bcc.read(cx).value().to_string();
        let subject = self.subject.read(cx).value().to_string();
        let body = self.body.read(cx).value().to_string();
        let attachments = self.attachments.clone();
        let existing = self.draft_location;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            let current = this
                .update(cx, |this, _| this.autosave_generation == generation)
                .unwrap_or(false);
            if !current {
                return;
            }
            let to = addresses(&to).unwrap_or_default();
            let cc = addresses(&cc).unwrap_or_default();
            let bcc = addresses(&bcc).unwrap_or_default();
            let location = cx
                .background_executor()
                .spawn(async move {
                    crate::delivery::save_draft(
                        &account,
                        existing,
                        &from,
                        &to,
                        &cc,
                        &bcc,
                        &subject,
                        &body,
                        &attachments,
                    )
                })
                .await;
            if let Some(location) = location {
                let _ = this.update(cx, |this, cx| {
                    if this.autosave_generation == generation {
                        this.draft_location = Some(location);
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    /// File ▸ Attach File… (⇧⌘A): the portal's open panel, then one worker
    /// read per chosen file. Reading never touches the GPUI thread.
    fn attach_files(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let chosen = rmac_portal::choose_mail_attachments().await;
            let paths = match chosen {
                Ok(paths) => paths,
                Err(error) => {
                    let _ = this.update(cx, |this, cx| {
                        this.status = Some(error.to_string());
                        cx.notify();
                    });
                    return;
                }
            };
            if paths.is_empty() {
                return;
            }
            let read = cx
                .background_executor()
                .spawn(async move {
                    paths
                        .iter()
                        .map(|path| crate::delivery::read_attachment(path))
                        .collect::<Vec<_>>()
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let mut failure = None;
                for result in read {
                    match result {
                        Ok(attachment) => this.attachments.push(attachment),
                        Err(message) => failure = Some(message),
                    }
                }
                this.status = failure;
                this.schedule_autosave(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn remove_attachment(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.attachments.len() {
            self.attachments.remove(index);
            self.schedule_autosave(cx);
            cx.notify();
        }
    }

    /// Closing without having sent still leaves a draft behind: flush the
    /// debounce immediately instead of waiting out the timer.
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.sent && self.worth_saving(cx) {
            if let Some(account) = self.accounts.get(self.selected_account).cloned() {
                let from = account.address.clone();
                let to = addresses(&self.to.read(cx).value()).unwrap_or_default();
                let cc = addresses(&self.cc.read(cx).value()).unwrap_or_default();
                let bcc = addresses(&self.bcc.read(cx).value()).unwrap_or_default();
                let subject = self.subject.read(cx).value().to_string();
                let body = self.body.read(cx).value().to_string();
                let attachments = self.attachments.clone();
                let existing = self.draft_location;
                cx.background_executor()
                    .spawn(async move {
                        crate::delivery::save_draft(
                            &account,
                            existing,
                            &from,
                            &to,
                            &cc,
                            &bcc,
                            &subject,
                            &body,
                            &attachments,
                        )
                    })
                    .detach();
            }
        }
        window.remove_window();
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
        address_field: Option<AddressField>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let dropdown = match address_field.zip(self.completion.clone()) {
            Some((field, (active, suggestions))) if active == field => {
                Some(self.completion_dropdown(field, &suggestions, cx))
            }
            _ => None,
        };
        div()
            .relative()
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
            .when_some(dropdown, |parent, dropdown| parent.child(dropdown))
    }

    /// Suggestions float under the field they belong to, like Finder's Go to
    /// Folder panel (`go_to_folder_controller.rs`).
    fn completion_dropdown(
        &self,
        field: AddressField,
        suggestions: &[Recipient],
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut list = div()
            .id("compose-address-suggestions")
            .role(Role::ListBox)
            .aria_label("Suggested Addresses")
            .absolute()
            .top(px(35.0))
            .left(px(84.0))
            .right(px(10.0))
            .flex()
            .flex_col()
            .py(px(4.0))
            .rounded(px(mac::radius_card()))
            .bg(mac::raised())
            .border_1()
            .border_color(mac::separator())
            .shadow_lg();
        for (index, recipient) in suggestions.iter().enumerate() {
            let chosen = recipient.clone();
            list = list.child(
                div()
                    .id(("compose-address-suggestion", index))
                    .role(Role::ListBoxOption)
                    .aria_label(chosen.label())
                    .px(px(10.0))
                    .py(px(5.0))
                    .text_size(px(12.0))
                    .text_color(mac::text())
                    .cursor_pointer()
                    .hover(|style| style.bg(mac::hover()))
                    .child(chosen.label())
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.accept_completion(field, &chosen, window, cx);
                    })),
            );
        }
        list
    }

    fn accept_completion(
        &mut self,
        field: AddressField,
        chosen: &Recipient,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = match field {
            AddressField::To => self.to.clone(),
            AddressField::Cc => self.cc.clone(),
            AddressField::Bcc => self.bcc.clone(),
        };
        let current = input.read(cx).value().to_string();
        let updated = apply_completion(&current, chosen);
        input.update(cx, |input, cx| input.set_value(updated, window, cx));
        self.completion = None;
        self.schedule_autosave(cx);
        cx.notify();
    }

    fn attachments_row(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if self.attachments.is_empty() {
            return None;
        }
        let mut row = div()
            .id("compose-attachments")
            .role(Role::ListBox)
            .aria_label("Attachments")
            .flex()
            .flex_wrap()
            .gap(px(6.0))
            .px(px(16.0))
            .py(px(8.0))
            .border_b_1()
            .border_color(mac::separator());
        for (index, attachment) in self.attachments.iter().enumerate() {
            let name = attachment.filename.clone();
            let label = format!("{name}, {}", human_size(attachment.size));
            row = row.child(
                div()
                    .id(("compose-attachment", index))
                    .role(Role::ListBoxOption)
                    .aria_label(label.clone())
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .px(px(8.0))
                    .py(px(4.0))
                    .rounded(px(mac::radius_control()))
                    .bg(mac::control_fill())
                    .text_size(px(12.0))
                    .text_color(mac::text())
                    .child(label)
                    .child(
                        div()
                            .id(("compose-attachment-remove", index))
                            .role(Role::Button)
                            .aria_label(format!("Remove {name}"))
                            .cursor_pointer()
                            .text_color(mac::text_secondary())
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.remove_attachment(index, cx);
                            }))
                            .child("✕"),
                    ),
            );
        }
        Some(row)
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
            .on_action(cx.listener(|this, _: &AttachFile, _, cx| this.attach_files(cx)))
            .on_action(cx.listener(|this, _: &CloseWindow, window, cx| this.close(window, cx)))
            .on_action(
                cx.listener(|this, _: &rmac_ui::RequestClose, window, cx| this.close(window, cx)),
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
                            .on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| this.attach_files(cx)),
                            )
                            .text_color(mac::text())
                            .child("Attach"),
                    ),
            )
            .child(self.field("To:", &self.to, Some(AddressField::To), cx))
            .child(self.field("Cc:", &self.cc, Some(AddressField::Cc), cx))
            .child(self.field("Bcc:", &self.bcc, Some(AddressField::Bcc), cx))
            .child(self.field("Subject:", &self.subject, None, cx))
            .when_some(self.attachments_row(cx), |parent, row| parent.child(row))
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
