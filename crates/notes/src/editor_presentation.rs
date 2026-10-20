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
                .h(px(if self.attachment_view_large {
                    174.0
                } else {
                    120.0
                }))
                .flex_none()
                .flex()
                .gap_3()
                .p_2()
                .rounded(px(rmac_ui::mac::radius_menu()))
                .border_1()
                .border_color(mac::separator())
                .bg(mac::window())
                .child(
                    div()
                        .w(px(if self.attachment_view_large {
                            260.0
                        } else {
                            150.0
                        }))
                        .h_full()
                        .child(preview),
                )
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

    /// File ▸ Lock Note: a locked note whose password is closed. Its title,
    /// body, tags and attachments are encrypted in the library and absent
    /// from the view, so there is nothing to render but the lock.
    fn render_locked_note(&self, note_id: NoteId, cx: &mut Context<Self>) -> AnyElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(mac::window())
            .child(
                div()
                    .w(px(320.0))
                    .v_flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(32.0))
                            .text_color(mac::text_secondary())
                            .child("🔒"),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(15.0))
                            .font_weight(mac::SEMIBOLD)
                            .child("This note is locked."),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(mac::text_secondary())
                            .child("Enter the notes password to view it."),
                    )
                    .child(
                        Button::new("unlock-note", "View Note…").on_click(cx.listener(
                            move |this, _, window, cx| this.request_unlock(note_id, window, cx),
                        )),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn render_editor(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(note) = self.session.selected_note() else {
            return centered_state("No Note Selected", "Choose a note or create a new one.");
        };
        if self.session.is_note_closed(note) {
            return self.render_locked_note(note.id, cx);
        }
        let editable =
            self.is_interactive_ready() && !note.deleted && !self.markdown_preview_visible;
        let light_background = self.note_has_light_background();
        let body_size = BODY_SIZE + f32::from(self.note_zoom);
        let title_size = TITLE_SIZE + f32::from(self.note_zoom);
        let title_line = TITLE_LINE + f32::from(self.note_zoom);
        let attachments = self.render_attachments(note, cx);
        let body = if self.markdown_preview_visible {
            self.render_markdown_preview(cx)
        } else {
            let text_color = note_editor_text(light_background);
            let paper = note_editor_fill(light_background);
            let zoom = body_size / BODY_SIZE;
            let show_highlights = self.show_highlights;
            let smart_lists = self.smart_lists;
            self.body.update(cx, |editor, cx| {
                editor.set_editable(editable, cx);
                editor.set_zoom(zoom, cx);
                editor.set_appearance(text_color, mac::text_selection(), mac::text_caret(), cx);
                editor.set_paper_color(paper, cx);
                editor.set_link_color(notes_accent(), cx);
                editor.set_decoration_colors(checkbox_ring(), notes_accent(), checkbox_ring(), cx);
                editor.set_show_highlights(show_highlights, cx);
                editor.set_smart_lists(smart_lists);
            });
            let editor = self.body.read(cx);
            let selection = editor.selected_range();
            let head = editor.cursor();
            let anchor = if head == selection.start {
                selection.end
            } else {
                selection.start
            };
            let document = editor.document().clone();
            let focus = gpui::Focusable::focus_handle(editor, cx);
            let rich = self.body.clone();
            div()
                .id("notes-body")
                .role(Role::MultilineTextInput)
                .aria_label("Body")
                .accessible_text(
                    move || document.text(),
                    (anchor, head),
                    focus,
                    move |range, _window, cx| {
                        rich.update(cx, |editor, cx| editor.select_range(range, cx));
                    },
                )
                // Tab at a list item's start nests it, as in Notes; anywhere
                // else it moves on to the Tags field after the body. Return
                // or Tab takes a result Suggest Results offers.
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    let modifiers = event.keystroke.modifiers;
                    if modifiers.shift
                        || modifiers.control
                        || modifiers.alt
                        || modifiers.platform
                        || !this.body_focused(window, cx)
                    {
                        return;
                    }
                    let key = event.keystroke.key.as_str();
                    if matches!(key, "enter" | "tab") && this.maths_suggestion.is_some() {
                        if this.accept_maths_suggestion(cx) {
                            window.prevent_default();
                            cx.stop_propagation();
                        }
                        return;
                    }
                    if key == "escape" && this.maths_suggestion.take().is_some() {
                        window.prevent_default();
                        cx.stop_propagation();
                        cx.notify();
                        return;
                    }
                    if key != "tab" || this.body.read(cx).at_list_item_start() {
                        return;
                    }
                    window.prevent_default();
                    cx.stop_propagation();
                    window.focus(&this.tags.read(cx).focus_handle(cx), cx);
                }))
                .on_a11y_action(AccessibleAction::SetValue, self.assistive_body_listener(cx))
                .on_a11y_action(
                    AccessibleAction::ReplaceSelectedText,
                    self.assistive_body_listener(cx),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, event: &MouseDownEvent, window, cx| {
                        this.open_body_menu(event.position, window, cx)
                    }),
                )
                .flex_1()
                .min_h(px(0.0))
                .pb_4()
                .child(self.body.clone())
                .into_any_element()
        };
        // macOS 26 Notes: the edit date centred 8 below the toolbar, then the
        // title in Notes' Title style and the body directly beneath it. Tags
        // (a rmac field; Notes keeps #tags inline) sit at the foot.
        div()
            .size_full()
            .v_flex()
            .bg(note_editor_fill(light_background))
            .text_color(note_editor_text(light_background))
            .child(
                div()
                    .pt(px(DATE_TOP))
                    .h(px(DATE_TOP + DATE_LINE))
                    .flex_none()
                    .flex()
                    .justify_center()
                    .text_size(rmac_ui::text_px(DATE_SIZE))
                    .line_height(px(DATE_LINE))
                    .text_color(note_editor_date(light_background))
                    .child(full_date_label(note.modified_unix_ms)),
            )
            .child(
                div()
                    .id("notes-title")
                    .role(Role::TextInput)
                    .aria_label("Title")
                    .accessible_text_input(&self.title, cx)
                    // Tab in the title goes on into the body, as the Mac's
                    // single text view does.
                    .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        let modifiers = event.keystroke.modifiers;
                        if event.keystroke.key != "tab"
                            || modifiers.shift
                            || modifiers.control
                            || modifiers.alt
                            || modifiers.platform
                            || !this.title.read(cx).focus_handle(cx).is_focused(window)
                        {
                            return;
                        }
                        window.prevent_default();
                        cx.stop_propagation();
                        this.focus_body(window, cx);
                    }))
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
                    .text_size(rmac_ui::text_px(title_size))
                    .line_height(px(title_line))
                    .font_weight(mac::BOLD)
                    .text_color(note_editor_text(light_background))
                    .child(
                        TextField::new(&self.title)
                            .appearance(false)
                            .disabled(!editable)
                            .text_size(rmac_ui::text_px(title_size))
                            .line_height(px(title_line))
                            .h(px(title_line))
                            .px_0()
                            .py_0(),
                    ),
            )
            .when_some(attachments, |element, attachments| {
                element.child(attachments)
            })
            .when(self.note_find_open, |element| {
                element.child(self.render_note_find_bar(cx))
            })
            .child(body)
            .child(
                div()
                    .id("notes-tags")
                    .role(Role::TextInput)
                    .aria_label("Tags")
                    .accessible_text_input(&self.tags, cx)
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
                    .text_color(note_editor_date(light_background))
                    .child(
                        div()
                            .text_color(note_editor_text(light_background))
                            .child("#"),
                    )
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
            MarkdownPreviewState::Ready { document, .. } => {
                let chips = self
                    .session
                    .selected_note()
                    .and_then(|note| self.attachment_chip_paths.get(&note.id));
                render_markdown_document(
                    document,
                    self.note_has_light_background(),
                    self.show_highlights,
                    chips,
                    cx,
                )
            }
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
