//! Separate compose window. Input changes stay on the GPUI thread; MIME,
//! attachment reads and mailbox writes run on worker threads.

use std::time::Duration;

use gpui::{
    div, prelude::FluentBuilder as _, px, App, AppContext as _, ClickEvent, Context, Entity,
    EntityInputHandler as _, FocusHandle, Focusable as _, Global, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, Role, StatefulInteractiveElement as _, Styled as _,
    Window,
};
use rmac_editor::InputState;
use rmac_mail::compose::{
    addresses, apply_completion, complete, completion_query, format_lines, initial_draft_with,
    quote_text, ComposeKind, LineFormat, Recipient,
};
use rmac_mail::Message;
use rmac_mail_mime::Draft;
use rmac_ui::{mac, AccessibleTextInput as _, InputEvent, Root, TextField};

use crate::{
    delivery::{ComposeAccount, DeliveryResult, DraftLocation, PendingAttachment},
    AttachFile, CloseWindow, DecreaseIndentation, DecreaseQuoteLevel, Delete, IncreaseIndentation,
    IncreaseQuoteLevel, PasteAsQuotation, Redo, SaveDraft, SendMessage, StartSpeaking,
    StopSpeaking, TransformCapitalise, TransformLowercase, TransformUppercase, Undo,
};

/// View ▸ Cc Address Field and Bcc Address Field: which optional address
/// rows every compose window shows. The Mac shows Cc and hides Bcc until
/// asked; a draft that already has Bcc recipients shows it regardless.
struct ComposeFields {
    cc: bool,
    bcc: bool,
}

impl Global for ComposeFields {}

fn fields(cx: &App) -> (bool, bool) {
    cx.try_global::<ComposeFields>()
        .map_or((true, false), |fields| (fields.cc, fields.bcc))
}

/// Ticks View ▸ Cc/Bcc Address Field to match the current choice.
pub fn publish_field_state(cx: &mut App) {
    let (cc, bcc) = fields(cx);
    rmac_ui::set_menu_checked("mail::ToggleCcField", cc, cx);
    rmac_ui::set_menu_checked("mail::ToggleBccField", bcc, cx);
}

/// View ▸ Cc Address Field (`cc`) or Bcc Address Field: show or hide that
/// row in every compose window.
pub fn toggle_field(cc: bool, cx: &mut App) {
    let (current_cc, current_bcc) = fields(cx);
    let (cc_shown, bcc_shown) = if cc {
        (!current_cc, current_bcc)
    } else {
        (current_cc, !current_bcc)
    };
    cx.set_global(ComposeFields {
        cc: cc_shown,
        bcc: bcc_shown,
    });
    publish_field_state(cx);
    cx.refresh_windows();
}

/// Hands an Edit command to the focused text field (gpui-component's
/// `input::` actions), for the rows Mail's own actions own.
fn forward_to_text_field(name: &str, window: &mut Window, cx: &mut App) {
    if let Ok(action) = cx.build_action(name, None) {
        window.dispatch_action(action, cx);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AddressField {
    To,
    Cc,
    Bcc,
}

pub fn open(
    kind: ComposeKind,
    message: Option<Message>,
    prefill: Option<Draft>,
    accounts: Vec<ComposeAccount>,
    candidates: Vec<Recipient>,
    cx: &mut App,
) {
    // Not `window_options_for_app`: the compose window would then inherit
    // whatever size the main Mail window last saved under the same app_id
    // (the UIA-06/UIA-09 window-geometry-key bug).
    let options = rmac_ui::window_options_for_panel(rmac_ui::app_id::MAIL, 640.0, 560.0, cx);
    match cx.open_window(options, |window, cx| {
        rmac_ui::prepare_surface_window(window, cx);
        window.set_window_title("New Message");
        let view = cx.new(|cx| {
            ComposeView::new(
                kind,
                message.as_ref(),
                prefill,
                accounts,
                candidates,
                window,
                cx,
            )
        });
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
    /// Edit ▸ Writing Tools and its panel, for the message body (ADR 0024
    /// feature 2).
    writing_tools: Entity<rmac_writing_tools::WritingTools>,
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
    /// A draft opened with Bcc recipients keeps its Bcc row on screen.
    show_bcc: bool,
}

impl ComposeView {
    fn new(
        kind: ComposeKind,
        message: Option<&Message>,
        prefill: Option<Draft>,
        accounts: Vec<ComposeAccount>,
        candidates: Vec<Recipient>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let from = accounts
            .first()
            .map(|account| account.address.as_str())
            .unwrap_or_default();
        // Settings ▸ Composing decides how much of the original a reply
        // quotes; Settings ▸ Signatures adds the signature below.
        let (settings, _error) = rmac_mail::settings::load();
        let mut quote = initial_draft_with(kind, message, from, &settings.composing);
        // A `mailto:` launch (MAIL-8, `main.rs`) hands its parsed fields in
        // here instead of deriving them from `kind`/`message`; only the
        // fields it actually set override the blank/quoted draft.
        if let Some(prefill) = prefill {
            if !prefill.to.is_empty() {
                quote.to = prefill.to;
            }
            if !prefill.cc.is_empty() {
                quote.cc = prefill.cc;
            }
            if !prefill.bcc.is_empty() {
                quote.bcc = prefill.bcc;
            }
            if !prefill.subject.is_empty() {
                quote.subject = prefill.subject;
            }
            if !prefill.text.is_empty() {
                quote.text = prefill.text;
            }
            if !prefill.attachments.is_empty() {
                quote.attachments = prefill.attachments;
            }
        }
        // Forward as Attachment carries the original as a part from the
        // start; it joins the attachment row like any other.
        let initial_attachments: Vec<PendingAttachment> = std::mem::take(&mut quote.attachments)
            .into_iter()
            .map(|attachment| PendingAttachment {
                filename: attachment.filename,
                content_type: attachment.content_type,
                size: attachment.bytes.len(),
                bytes: std::sync::Arc::new(attachment.bytes),
            })
            .collect();
        let show_bcc = !quote.bcc.is_empty();
        // Every new message, reply and forward gets the chosen account's
        // automatic signature (Settings ▸ Signatures, MAIL-8) appended once,
        // right here when the compose window opens.
        let quote = settings.compose_draft_with_signature(quote, from);
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
        bcc.update(cx, |input, cx| {
            input.set_value(quote.bcc.join(", "), window, cx)
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
        // Lulo Intelligence may have been turned on in System Settings
        // since: Writing Tools look again when this window comes forward.
        cx.observe_window_activation(window, |_, window, cx| {
            if window.is_window_active() {
                rmac_writing_tools::refresh(cx);
                // Message ▸ Send Again becomes Send in a compose window.
                rmac_ui::set_menu_label("mail::SendMessage", "Send", cx);
                for action in [
                    "mail::SaveDraft",
                    "mail::PasteAsQuotation",
                    "mail::SendMessage",
                    "mail::Undo",
                    "mail::Redo",
                    "mail::Delete",
                ] {
                    rmac_ui::set_menu_enabled(action, true, cx);
                }
            }
        })
        .detach();
        Self {
            focus: cx.focus_handle(),
            to,
            cc,
            bcc,
            subject,
            body,
            writing_tools: cx.new(rmac_writing_tools::WritingTools::new),
            accounts,
            selected_account: 0,
            pending: false,
            quote,
            status: None,
            candidates,
            completion: None,
            attachments: initial_attachments,
            draft_location: None,
            autosave_generation: 0,
            sent: false,
            show_bcc,
        }
    }

    /// The address, subject or body field that has keyboard focus.
    fn focused_field(&self, window: &Window, cx: &App) -> Option<Entity<InputState>> {
        [&self.to, &self.cc, &self.bcc, &self.subject, &self.body]
            .into_iter()
            .find(|input| input.read(cx).focus_handle(cx).is_focused(window))
            .cloned()
    }

    /// File ▸ Save (⌘S): writes the draft to Drafts now rather than at the
    /// next autosave.
    fn save_now(&mut self, cx: &mut Context<Self>) {
        if self.sent || !self.worth_saving(cx) {
            return;
        }
        let Some(account) = self.accounts.get(self.selected_account).cloned() else {
            self.status = Some("Add an Internet Account to save drafts".into());
            cx.notify();
            return;
        };
        // Any autosave still waiting is superseded by this one.
        self.autosave_generation = self.autosave_generation.wrapping_add(1);
        let generation = self.autosave_generation;
        let from = account.address.clone();
        let to = addresses(&self.to.read(cx).value()).unwrap_or_default();
        let cc = addresses(&self.cc.read(cx).value()).unwrap_or_default();
        let bcc = addresses(&self.bcc.read(cx).value()).unwrap_or_default();
        let subject = self.subject.read(cx).value().to_string();
        let body = self.body.read(cx).value().to_string();
        let attachments = self.attachments.clone();
        let existing = self.draft_location;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
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
            let _ = this.update(cx, |this, cx| {
                match location {
                    Some(location) if this.autosave_generation == generation => {
                        this.draft_location = Some(location);
                        this.status = None;
                    }
                    Some(_) => {}
                    None => this.status = Some("Mail could not save the draft".into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Edit ▸ Paste as Quotation (⇧⌘V): the clipboard's text into the body
    /// as a quoted block.
    fn paste_as_quotation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let quoted = quote_text(&text);
        let focus = self.body.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        self.body.update(cx, |state, cx| {
            state.replace_text_in_range(None, &quoted, window, cx);
        });
        self.schedule_autosave(cx);
        cx.notify();
    }

    fn transform(
        &mut self,
        transformation: rmac_ui::TextTransformation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(field) = self.focused_field(window, cx) {
            if rmac_ui::transform_selection(&field, transformation, window, cx) {
                cx.notify();
            }
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

    /// Format ▸ Indentation and Quote Level: reformat every line the body's
    /// selection touches, keep those lines selected, and record one undo step.
    fn format_body(&mut self, format: LineFormat, window: &mut Window, cx: &mut Context<Self>) {
        self.body.update(cx, |state, cx| {
            let text = state.value().to_string();
            let Some((range, replaced)) = format_lines(&text, state.selected_range(), format)
            else {
                return;
            };
            let start = range.start;
            state.set_selected_range(range, cx);
            state.replace_text_in_range(None, &replaced, window, cx);
            state.set_selected_range(start..start + replaced.len(), cx);
        });
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if window.is_window_active() {
            // Edit ▸ Writing Tools works on the body while this window is
            // in front; Mail's main window turns it off again.
            rmac_writing_tools::publish_menu_state(!self.pending && !self.sent, cx);
        }
        let editable = !self.pending && !self.sent;
        let (cc_field, bcc_field) = fields(cx);
        let show_cc = cc_field || !self.cc.read(cx).value().trim().is_empty();
        let show_bcc = bcc_field || self.show_bcc;
        let root = div()
            .size_full()
            .bg(mac::window())
            .track_focus(&self.focus)
            .key_context("MailCompose")
            .on_action(cx.listener(|this, _: &SendMessage, _, cx| this.send(cx)))
            .on_action(cx.listener(|this, _: &AttachFile, _, cx| this.attach_files(cx)))
            .on_action(cx.listener(|this, _: &IncreaseIndentation, window, cx| {
                this.format_body(LineFormat::IncreaseIndent, window, cx)
            }))
            .on_action(cx.listener(|this, _: &DecreaseIndentation, window, cx| {
                this.format_body(LineFormat::DecreaseIndent, window, cx)
            }))
            .on_action(cx.listener(|this, _: &IncreaseQuoteLevel, window, cx| {
                this.format_body(LineFormat::IncreaseQuote, window, cx)
            }))
            .on_action(cx.listener(|this, _: &DecreaseQuoteLevel, window, cx| {
                this.format_body(LineFormat::DecreaseQuote, window, cx)
            }))
            .on_action(cx.listener(|this, _: &CloseWindow, window, cx| this.close(window, cx)))
            .on_action(
                cx.listener(|this, _: &rmac_ui::RequestClose, window, cx| this.close(window, cx)),
            )
            .on_action(cx.listener(|this, _: &SaveDraft, _, cx| this.save_now(cx)))
            .on_action(cx.listener(|this, _: &PasteAsQuotation, window, cx| {
                this.paste_as_quotation(window, cx)
            }))
            // Edit ▸ Undo, Redo and Delete act on the message list in the
            // viewer; here they edit the focused field.
            .on_action(cx.listener(|_, _: &Undo, window, cx| {
                forward_to_text_field("input::Undo", window, cx)
            }))
            .on_action(cx.listener(|_, _: &Redo, window, cx| {
                forward_to_text_field("input::Redo", window, cx)
            }))
            .on_action(cx.listener(|_, _: &Delete, window, cx| {
                forward_to_text_field("input::Delete", window, cx)
            }))
            .on_action(cx.listener(|this, _: &TransformUppercase, window, cx| {
                this.transform(rmac_ui::TextTransformation::Uppercase, window, cx)
            }))
            .on_action(cx.listener(|this, _: &TransformLowercase, window, cx| {
                this.transform(rmac_ui::TextTransformation::Lowercase, window, cx)
            }))
            .on_action(cx.listener(|this, _: &TransformCapitalise, window, cx| {
                this.transform(rmac_ui::TextTransformation::Capitalise, window, cx)
            }))
            .on_action(cx.listener(|this, _: &StartSpeaking, window, cx| {
                let field = this
                    .focused_field(window, cx)
                    .unwrap_or_else(|| this.body.clone());
                rmac_ui::start_speaking(&field, "mail::StopSpeaking", cx);
            }))
            .on_action(cx.listener(|_, _: &StopSpeaking, _, _| rmac_ui::stop_speaking()))
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
            .when(show_cc, |root| {
                root.child(self.field("Cc:", &self.cc, Some(AddressField::Cc), cx))
            })
            .when(show_bcc, |root| {
                root.child(self.field("Bcc:", &self.bcc, Some(AddressField::Bcc), cx))
            })
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
                            .context_menu(rmac_writing_tools::text_menu(
                                self.body.clone(),
                                editable,
                            ))
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
            .child(self.writing_tools.clone());
        let view = cx.entity().downgrade();
        rmac_writing_tools::on_actions(root, &self.writing_tools, move |_, cx| {
            let view = view.upgrade()?;
            let this = view.read(cx);
            (!this.pending && !this.sent).then(|| rmac_writing_tools::target(&this.body))
        })
    }
}
