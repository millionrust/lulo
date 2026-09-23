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
                .mx(px(44.0))
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
        let words = self.body.read(cx).value().split_whitespace().count();
        let characters = self.body.read(cx).value().chars().count();
        let attachments = self.render_attachments(note, cx);
        let body = if self.markdown_preview_visible {
            self.render_markdown_preview(cx)
        } else {
            div()
                .flex_1()
                .min_h(px(0.0))
                .px(px(44.0))
                .pt_2()
                .pb_4()
                .text_size(px(14.0))
                .line_height(px(20.0))
                .text_color(mac::text())
                .child(
                    TextField::new(&self.body)
                        .h_full()
                        .appearance(false)
                        .disabled(!editable),
                )
                .into_any_element()
        };
        div()
            .size_full()
            .v_flex()
            .bg(mac::window())
            .child(
                div()
                    .pt_3()
                    .pb_1()
                    .flex()
                    .items_center()
                    .px(px(44.0))
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(mac::text_secondary())
                    // Notes centres the note's full edit date above the text.
                    .child(div().flex_1())
                    .child(
                        div()
                            .flex_none()
                            .child(full_date_label(note.modified_unix_ms)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .justify_end()
                            .items_center()
                            .gap_1()
                            .child(
                                Button::new("edit-markdown", "Edit")
                                    .xsmall()
                                    .selected(!self.markdown_preview_visible)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if this.markdown_preview_visible {
                                            this.toggle_markdown_preview(cx)
                                        }
                                    })),
                            )
                            .child(
                                Button::new("preview-markdown", "Preview")
                                    .xsmall()
                                    .selected(self.markdown_preview_visible)
                                    .busy(matches!(
                                        self.markdown_preview.state(),
                                        MarkdownPreviewState::Loading { .. }
                                    ))
                                    .disabled(
                                        !self.markdown_preview_visible
                                            && (!self.is_interactive_ready()
                                                || self.latest_local_generation.is_some()),
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if !this.markdown_preview_visible {
                                            this.toggle_markdown_preview(cx)
                                        }
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .px(px(44.0))
                    .pt_1()
                    .text_size(px(24.0))
                    .line_height(px(30.0))
                    .font_weight(mac::BOLD)
                    .text_color(mac::text())
                    .child(
                        TextField::new(&self.title)
                            .appearance(false)
                            .disabled(!editable),
                    ),
            )
            .child(
                div()
                    .mx(px(44.0))
                    .mt_1()
                    .mb_2()
                    .h(px(28.0))
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .rounded(px(rmac_ui::mac::radius_segmented()))
                    .bg(mac::control_fill())
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(mac::notes_accent())
                    .child("#")
                    .child(
                        TextField::new(&self.tags)
                            .appearance(false)
                            .cleanable(true)
                            .small()
                            .disabled(!editable),
                    ),
            )
            .when_some(attachments, |element, attachments| {
                element.child(attachments)
            })
            .child(body)
            .child(
                div()
                    .h(px(24.0))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .border_t_1()
                    .border_color(mac::separator())
                    .text_size(rmac_ui::text_px(11.0))
                    .text_color(mac::text_tertiary())
                    .child(format!("{words} words"))
                    .child("•")
                    .child(format!("{characters} characters")),
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
