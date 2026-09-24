//! Notes editor, attachment, and Markdown-preview projection.

use super::*;

impl NotesView {
    fn render_attachments(&self, note: &NoteRecord, cx: &mut Context<Self>) -> Option<AnyElement> {
        let snapshot = self.session.snapshot()?;
        let attachments = note
            .attachments
            .iter()
            .filter_map(|attachment_id| {
                snapshot
                    .attachments
                    .iter()
                    .find(|attachment| attachment.id == *attachment_id && !attachment.deleted)
            })
            .collect::<Vec<_>>();
        if attachments.is_empty() {
            return None;
        }
        let selected = self.selected_attachment;
        let can_remove = !note.deleted
            && selected.is_some_and(|attachment_id| {
                attachments
                    .iter()
                    .any(|attachment| attachment.id == attachment_id)
            });
        let preview = match self.preview.state() {
            PreviewState::Loading { attachment_id, .. } if selected == Some(*attachment_id) => {
                centered_attachment_state("Loading preview…", None, cx)
            }
            PreviewState::Ready { image, .. }
                if selected == Some(image.attachment_id()) && self.preview_image.is_some() =>
            {
                div()
                    .size_full()
                    .rounded(px(rmac_ui::mac::radius_control()))
                    .overflow_hidden()
                    .bg(mac::control_fill())
                    .child(
                        img(self
                            .preview_image
                            .as_ref()
                            .expect("preview image checked")
                            .clone())
                        .size_full()
                        .object_fit(ObjectFit::Contain),
                    )
                    .into_any_element()
            }
            PreviewState::Ready { image, .. } if selected == Some(image.attachment_id()) => {
                centered_attachment_state("Preview unavailable", Some("Try Again"), cx)
            }
            PreviewState::Unavailable { attachment_id, .. } if selected == Some(*attachment_id) => {
                centered_attachment_state("Preview unavailable", Some("Try Again"), cx)
            }
            _ if self.preview_worker.is_none() => {
                centered_attachment_state("Preview unavailable", None, cx)
            }
            _ => centered_attachment_state("Loading preview…", None, cx),
        };
        let rows = attachments.into_iter().map(|attachment| {
            let attachment_id = attachment.id;
            div()
                .v_flex()
                .gap_0p5()
                .child(
                    Button::new(
                        ("attachment-preview", attachment_id.get()),
                        attachment.display_name.clone(),
                    )
                    .selected(selected == Some(attachment_id))
                    .xsmall()
                    .w_full()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_attachment_preview(attachment_id, cx)
                    })),
                )
                .child(
                    div()
                        .px_2()
                        .text_size(rmac_ui::text_px(10.0))
                        .text_color(mac::text_tertiary())
                        .child(format_storage_bytes(attachment.byte_len)),
                )
        });
        Some(
            div()
                .mx(px(EDITOR_INSET))
                .mb_2()
                .h(px(174.0))
                .flex_none()
                .flex()
                .gap_3()
                .p_2()
                .rounded(px(rmac_ui::mac::radius_menu()))
                .border_1()
                .border_color(mac::separator())
                .bg(mac::window())
                .child(div().w(px(260.0)).h_full().child(preview))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .v_flex()
                        .gap_1()
                        .child(
                            div()
                                .id("attachment-list")
                                .flex_1()
                                .min_h(px(0.0))
                                .overflow_y_scroll()
                                .v_flex()
                                .gap_1()
                                .children(rows),
                        )
                        .when(can_remove, |element| {
                            element.child(
                                Button::new("remove-attachment", "Remove Photo…")
                                    .destructive()
                                    .xsmall()
                                    .disabled(
                                        !self.is_interactive_ready()
                                            || self.latest_local_generation.is_some(),
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.begin_attachment_removal(cx)
                                    })),
                            )
                        }),
                )
                .into_any_element(),
        )
    }

    pub(super) fn render_editor(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(note) = self.session.selected_note() else {
            return centered_state("No Note Selected", "Choose a note or create a new one.");
        };
        let editable =
            self.is_interactive_ready() && !note.deleted && !self.markdown_preview_visible;
        let attachments = self.render_attachments(note, cx);
        let body_value = self.body.read(cx).value().to_string();
        let body = if self.markdown_preview_visible {
            self.render_markdown_preview(cx)
        } else {
            div()
                .id("notes-body")
                .role(Role::TextInput)
                .aria_label("Body")
                .aria_value(body_value)
                .on_a11y_action(AccessibleAction::SetValue, self.assistive_body_listener(cx))
                .on_a11y_action(
                    AccessibleAction::ReplaceSelectedText,
                    self.assistive_body_listener(cx),
                )
                .flex_1()
                .min_h(px(0.0))
                .px(px(EDITOR_INSET))
                .pb_4()
                .text_size(rmac_ui::text_px(BODY_SIZE))
                .line_height(px(BODY_LINE))
                .text_color(editor_text())
                .child(
                    TextField::new(&self.body)
                        .h_full()
                        .appearance(false)
                        .disabled(!editable)
                        .text_size(rmac_ui::text_px(BODY_SIZE))
                        .line_height(px(BODY_LINE))
                        .px_0()
                        .py_0(),
                )
                .into_any_element()
        };
        // macOS 26 Notes: the edit date centred 8 below the toolbar, then the
        // title in Notes' Title style and the body directly beneath it. Tags
        // (a rmac field; Notes keeps #tags inline) sit at the foot.
        div()
            .size_full()
            .v_flex()
            .bg(editor_fill())
            .child(
                div()
                    .pt(px(DATE_TOP))
                    .h(px(DATE_TOP + DATE_LINE))
                    .flex_none()
                    .flex()
                    .justify_center()
                    .text_size(rmac_ui::text_px(DATE_SIZE))
                    .line_height(px(DATE_LINE))
                    .text_color(editor_date())
                    .child(full_date_label(note.modified_unix_ms)),
            )
            .child(
                div()
                    .id("notes-title")
                    .role(Role::TextInput)
                    .aria_label("Title")
                    .aria_value(self.title.read(cx).value().to_string())
                    .on_a11y_action(
                        AccessibleAction::SetValue,
                        self.assistive_title_listener(cx),
                    )
                    .on_a11y_action(
                        AccessibleAction::ReplaceSelectedText,
                        self.assistive_title_listener(cx),
                    )
                    .flex_none()
                    .px(px(EDITOR_INSET))
                    .pt(px(TITLE_TOP))
                    .text_size(rmac_ui::text_px(TITLE_SIZE))
                    .line_height(px(TITLE_LINE))
                    .font_weight(mac::BOLD)
                    .text_color(editor_text())
                    .child(
                        TextField::new(&self.title)
                            .appearance(false)
                            .disabled(!editable)
                            .text_size(rmac_ui::text_px(TITLE_SIZE))
                            .line_height(px(TITLE_LINE))
                            .h(px(TITLE_LINE))
                            .px_0()
                            .py_0(),
                    ),
            )
            .when_some(attachments, |element, attachments| {
                element.child(attachments)
            })
            .child(body)
            .child(
                div()
                    .id("notes-tags")
                    .role(Role::TextInput)
                    .aria_label("Tags")
                    .aria_value(self.tags.read(cx).value().to_string())
                    .on_a11y_action(AccessibleAction::SetValue, self.assistive_tags_listener(cx))
                    .on_a11y_action(
                        AccessibleAction::ReplaceSelectedText,
                        self.assistive_tags_listener(cx),
                    )
                    .flex_none()
                    .h(px(30.0))
                    .mx(px(EDITOR_INSET))
                    .flex()
                    .items_center()
                    .gap_1()
                    .border_t_1()
                    .border_color(row_rule())
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(mac::text_secondary())
                    .child(div().text_color(sidebar_selected_text()).child("#"))
                    .child(
                        div().flex_1().min_w(px(0.0)).child(
                            TextField::new(&self.tags)
                                .appearance(false)
                                .cleanable(true)
                                .small()
                                .disabled(!editable),
                        ),
                    ),
            )
            .into_any_element()
    }

    fn render_markdown_preview(&self, cx: &mut Context<Self>) -> AnyElement {
        match self.markdown_preview.state() {
            MarkdownPreviewState::Empty => centered_state(
                "Preview unavailable",
                "Switch back to Edit, then try Preview again.",
            ),
            MarkdownPreviewState::Loading { .. } => centered_state(
                "Formatting preview…",
                "Parsing this exact saved note without loading linked content.",
            ),
            MarkdownPreviewState::Unavailable { error, .. } => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .v_flex()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .text_size(rmac_ui::text_px(14.0))
                                .font_weight(mac::BOLD)
                                .child("Preview unavailable"),
                        )
                        .child(
                            div()
                                .text_size(rmac_ui::text_px(12.0))
                                .text_color(mac::text_secondary())
                                .child(error.to_string()),
                        )
                        .child(
                            Button::new("retry-markdown-preview", "Try Again")
                                .small()
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.retry_markdown_preview(cx)),
                                ),
                        ),
                )
                .into_any_element(),
            MarkdownPreviewState::Ready { document, .. } => render_markdown_document(document),
        }
    }
}

fn centered_attachment_state(
    message: &'static str,
    retry_label: Option<&'static str>,
    cx: &mut Context<NotesView>,
) -> AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(rmac_ui::mac::radius_control()))
        .bg(mac::control_fill())
        .child(
            div()
                .v_flex()
                .items_center()
                .gap_2()
                .text_size(rmac_ui::text_px(12.0))
                .text_color(mac::text_secondary())
                .child(message)
                .when_some(retry_label, |element, label| {
                    element.child(
                        Button::new("retry-attachment-preview", label)
                            .xsmall()
                            .on_click(
                                cx.listener(|this, _, _, cx| this.retry_attachment_preview(cx)),
                            ),
                    )
                }),
        )
        .into_any_element()
}
