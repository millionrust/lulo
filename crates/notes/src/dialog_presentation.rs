//! Notes confirmation, import, export, and move dialog projection.

use super::*;

impl NotesView {
    pub(super) fn render_purge_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        use rmac_ui::DialogButtonKind::{Destructive, Normal};

        let dialog = self.purge_dialog?;
        let (title, message, confirm_label) = match dialog {
            PurgeDialog::Note {
                note_id,
                attachment_count,
                attachment_bytes,
                ..
            } => {
                let note_title = self
                    .session
                    .snapshot()
                    .and_then(|snapshot| snapshot.notes.iter().find(|note| note.id == note_id))
                    .map_or_else(|| "this note".to_string(), |note| display_title(&note.title).to_string());
                (
                    "Delete this note permanently?",
                    format!(
                        "“{note_title}” and {attachment_count} {} ({}) will be permanently deleted. This cannot be undone. If attachment cleanup needs attention, Notes will pause further edits.",
                        if attachment_count == 1 { "attachment" } else { "attachments" },
                        format_storage_bytes(attachment_bytes)
                    ),
                    "Delete Note",
                )
            }
            PurgeDialog::EmptyTrash {
                note_count,
                attachment_count,
                attachment_bytes,
                ..
            } => (
                "Permanently delete all notes?",
                format!(
                    "{note_count} {} and {attachment_count} {} ({}) will be permanently deleted. This cannot be undone. If attachment cleanup needs attention, Notes will pause further edits.",
                    if note_count == 1 { "note" } else { "notes" },
                    if attachment_count == 1 { "attachment" } else { "attachments" },
                    format_storage_bytes(attachment_bytes)
                ),
                "Empty Trash",
            ),
        };
        Some(
            rmac_ui::alert(
                title,
                message,
                vec![
                    rmac_ui::dialog_button("cancel-purge", "Cancel", Normal)
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_purge(cx)))
                        .into_any_element(),
                    rmac_ui::dialog_button("confirm-purge", confirm_label, Destructive)
                        .on_click(cx.listener(|this, _, _, cx| this.confirm_purge(cx)))
                        .into_any_element(),
                ],
            )
            .into_any_element(),
        )
    }

    pub(super) fn render_attachment_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        use rmac_ui::DialogButtonKind::{Destructive, Normal};

        let dialog = self.attachment_dialog?;
        let (attachment_id, expected_revision, byte_len) = match dialog {
            AttachmentDialog::Remove {
                attachment_id,
                attachment_revision,
                byte_len,
                ..
            }
            | AttachmentDialog::CollectOrphan {
                attachment_id,
                attachment_revision,
                byte_len,
            } => (attachment_id, attachment_revision, byte_len),
        };
        let name = self
            .session
            .snapshot()
            .and_then(|snapshot| {
                snapshot.attachments.iter().find(|attachment| {
                    attachment.id == attachment_id && attachment.revision == expected_revision
                })
            })
            .map_or_else(
                || "this photo".to_string(),
                |attachment| attachment.display_name.clone(),
            );
        let (title, message, confirm_label) = match dialog {
            AttachmentDialog::Remove { .. } => (
                "Remove this photo?",
                format!(
                    "Remove “{name}” ({}) from this note? Notes will first save the note without the reference, then delete its managed local copy in a separate verified cleanup. The original imported file is not changed.",
                    format_storage_bytes(byte_len)
                ),
                "Remove Photo",
            ),
            AttachmentDialog::CollectOrphan { .. } => (
                "Clean up this removed photo?",
                format!(
                    "“{name}” ({}) is no longer referenced by any note. Delete its managed local copy? The original imported file is not changed.",
                    format_storage_bytes(byte_len)
                ),
                "Delete Managed Copy",
            ),
        };
        Some(
            rmac_ui::alert(
                title,
                message,
                vec![
                    rmac_ui::dialog_button("cancel-attachment-action", "Cancel", Normal)
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_attachment_dialog(cx)))
                        .into_any_element(),
                    rmac_ui::dialog_button("confirm-attachment-action", confirm_label, Destructive)
                        .on_click(cx.listener(|this, _, _, cx| this.confirm_attachment_dialog(cx)))
                        .into_any_element(),
                ],
            )
            .into_any_element(),
        )
    }

    pub(super) fn render_export_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        use rmac_ui::DialogButtonKind::{Normal, Primary};

        match self.export_dialog? {
            ExportDialog::Complete(outcome) => {
                let format = match outcome.format {
                    ExportFormat::Markdown => "Markdown file",
                    ExportFormat::RmacBundle => "Notes bundle",
                };
                Some(
                    rmac_ui::alert(
                        "Export complete",
                        format!(
                            "Notes verified the final {format}: {} {}, {} {}, {} total.",
                            outcome.note_count,
                            if outcome.note_count == 1 {
                                "note"
                            } else {
                                "notes"
                            },
                            outcome.attachment_count,
                            if outcome.attachment_count == 1 {
                                "attachment"
                            } else {
                                "attachments"
                            },
                            format_storage_bytes(outcome.output.byte_len)
                        ),
                        vec![rmac_ui::dialog_button("dismiss-export", "Done", Primary)
                            .on_click(cx.listener(|this, _, _, cx| this.dismiss_export_dialog(cx)))
                            .into_any_element()],
                    )
                    .into_any_element(),
                )
            }
            ExportDialog::Review(review) => {
                let snapshot = self.session.snapshot()?;
                let note_scope = self.session.selected_note().map(|note| ExportScope::Note {
                    note_id: note.id,
                    expected_note_revision: note.revision,
                });
                let folder_scope = match self.session.folder_selection() {
                    rmac_notes_runtime::FolderSelection::Folder(folder_id) => snapshot
                        .folders
                        .iter()
                        .find(|folder| folder.id == folder_id && !folder.deleted)
                        .map(|folder| ExportScope::Folder {
                            folder_id,
                            expected_folder_revision: folder.revision,
                        }),
                    _ => None,
                };
                let library_scope = ExportScope::Library {
                    expected_library_revision: snapshot.revision,
                };
                let can_markdown = matches!(review.scope, ExportScope::Note { .. })
                    && review.attachment_count == 0;
                let scope_detail = match review.scope {
                    ExportScope::Note { .. } => "The selected note is bound to its exact revision.",
                    ExportScope::Folder { .. } => {
                        "Only live notes in the current folder are included."
                    }
                    ExportScope::Library { .. } => {
                        "The complete library includes live and Recently Deleted notes."
                    }
                };
                let reviewed_bytes = review
                    .markdown_bytes
                    .saturating_add(review.attachment_bytes);
                let mut scope_buttons = Vec::<AnyElement>::new();
                if let Some(scope) = note_scope {
                    scope_buttons.push(
                        Button::new("export-this-note", "This Note")
                            .selected(review.scope == scope)
                            .w_full()
                            .on_click(
                                cx.listener(|this, _, _, cx| this.review_selected_note_export(cx)),
                            )
                            .into_any_element(),
                    );
                }
                if let Some(scope) = folder_scope {
                    scope_buttons.push(
                        Button::new("export-current-folder", "Current Folder")
                            .selected(review.scope == scope)
                            .w_full()
                            .on_click(
                                cx.listener(|this, _, _, cx| this.review_current_folder_export(cx)),
                            )
                            .into_any_element(),
                    );
                }
                scope_buttons.push(
                    Button::new("export-library", "Entire Library")
                        .selected(review.scope == library_scope)
                        .w_full()
                        .on_click(cx.listener(|this, _, _, cx| this.review_library_export(cx)))
                        .into_any_element(),
                );
                let card = div()
                    .w(px(440.0))
                    .p(px(20.0))
                    .v_flex()
                    .gap_3()
                    .rounded(px(rmac_ui::mac::radius_card()))
                    .bg(mac::window())
                    .border_1()
                    .border_color(mac::separator())
                    .shadow_xl()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(17.0))
                            .font_weight(mac::BOLD)
                            .child("Export Notes"),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(mac::text_secondary())
                            .child(scope_detail),
                    )
                    .child(div().v_flex().gap_1().children(scope_buttons))
                    .child(
                        div()
                            .p_3()
                            .rounded(px(rmac_ui::mac::radius_control()))
                            .bg(mac::control_fill())
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(mac::text_secondary())
                            .child(format!(
                                "{} {}, {} {}, {} of note and attachment content",
                                review.note_count,
                                if review.note_count == 1 { "note" } else { "notes" },
                                review.attachment_count,
                                if review.attachment_count == 1 {
                                    "attachment"
                                } else {
                                    "attachments"
                                },
                                format_storage_bytes(reviewed_bytes)
                            )),
                    )
                    .when(!can_markdown, |element| {
                        element.child(
                            div()
                                .text_size(rmac_ui::text_px(11.0))
                                .text_color(mac::text_tertiary())
                                .child(
                                    "Use a Notes bundle for folders, the library, or notes with attachments.",
                                ),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                rmac_ui::dialog_button("cancel-export", "Cancel", Normal)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.dismiss_export_dialog(cx)
                                    })),
                            )
                            .when(can_markdown, |element| {
                                element.child(
                                    rmac_ui::dialog_button(
                                        "export-markdown",
                                        "Export Markdown…",
                                        Normal,
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.choose_export_destination(ExportFormat::Markdown, cx)
                                    })),
                                )
                            })
                            .child(
                                rmac_ui::dialog_button(
                                    "export-bundle",
                                    "Export Bundle…",
                                    Primary,
                                )
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.choose_export_destination(ExportFormat::RmacBundle, cx)
                                })),
                            ),
                    );
                Some(rmac_ui::dialog("export-dialog", card).into_any_element())
            }
        }
    }

    pub(super) fn render_markdown_import_dialog(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        use rmac_ui::DialogButtonKind::{Normal, Primary};

        let review_request_id = self.note_import_request_id?;
        if self.markdown_import_action_request_id.is_some() {
            if matches!(self.session.phase(), SessionPhase::Pending { .. }) {
                return None;
            }
            let card = div()
                .w(px(420.0))
                .p(px(20.0))
                .v_flex()
                .gap_3()
                .rounded(px(rmac_ui::mac::radius_card()))
                .bg(mac::window())
                .border_1()
                .border_color(mac::separator())
                .shadow_xl()
                .child(
                    div()
                        .text_size(rmac_ui::text_px(17.0))
                        .font_weight(mac::BOLD)
                        .child("Importing Markdown note…"),
                )
                .child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(mac::text_secondary())
                        .child(
                            "Notes is committing the exact reviewed text candidate and verifying durable readback.",
                        ),
                );
            return Some(rmac_ui::dialog("markdown-import-progress", card).into_any_element());
        }

        let (reviewed_request_id, base_library_revision, review) = self.markdown_import_review?;
        if reviewed_request_id != review_request_id {
            return None;
        }
        let attention = if review.attention_count() == 0 {
            "No linked images, raw HTML, tables, tasks, footnotes, or frontmatter were recognized."
                .to_string()
        } else {
            format!(
                "Review found {} linked {}, {} raw HTML {}, {} {}, {} task-list {}, {} footnote {}, and {} frontmatter {}.",
                review.image_count,
                if review.image_count == 1 { "image" } else { "images" },
                review.raw_html_count,
                if review.raw_html_count == 1 { "construct" } else { "constructs" },
                review.table_count,
                if review.table_count == 1 { "table" } else { "tables" },
                review.task_count,
                if review.task_count == 1 { "item" } else { "items" },
                review.footnote_count,
                if review.footnote_count == 1 { "construct" } else { "constructs" },
                review.frontmatter_count,
                if review.frontmatter_count == 1 { "block" } else { "blocks" },
            )
        };
        let card = div()
            .w(px(470.0))
            .p(px(20.0))
            .v_flex()
            .gap_3()
            .rounded(px(rmac_ui::mac::radius_card()))
            .bg(mac::window())
            .border_1()
            .border_color(mac::separator())
            .shadow_xl()
            .child(
                div()
                    .text_size(rmac_ui::text_px(17.0))
                    .font_weight(mac::BOLD)
                    .child("Import Markdown Note"),
            )
            .child(
                div()
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(mac::text_secondary())
                    .child(format!(
                        "This review is bound to Notes library revision {base_library_revision}. The selected source decoded as {}.",
                        review.encoding.label()
                    )),
            )
            .child(
                div()
                    .p_3()
                    .rounded(px(rmac_ui::mac::radius_control()))
                    .bg(mac::control_fill())
                    .v_flex()
                    .gap_1()
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(mac::text_secondary())
                    .child(format!(
                        "{} source; {} decoded text; {} {}; {} {}",
                        format_storage_bytes(review.source_bytes),
                        format_storage_bytes(review.decoded_bytes),
                        review.heading_count,
                        if review.heading_count == 1 { "heading" } else { "headings" },
                        review.link_count,
                        if review.link_count == 1 { "link" } else { "links" },
                    ))
                    .child(attention),
            )
            .child(
                div()
                    .p_3()
                    .rounded(px(rmac_ui::mac::radius_control()))
                    .bg(mac::control_fill())
                    .text_size(rmac_ui::text_px(12.0))
                    .child(
                        "Import preserves the decoded Markdown characters and line endings as editable note source. Notes does not download linked images, execute raw HTML, or turn frontmatter, tables, tasks, or footnotes into active data. The original file is unchanged.",
                    ),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        rmac_ui::dialog_button("cancel-markdown-import", "Cancel", Normal)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.discard_markdown_import_review(cx)
                            })),
                    )
                    .child(
                        rmac_ui::dialog_button(
                            ("accept-markdown-import", review_request_id),
                            "Import as Markdown Source",
                            Primary,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.accept_markdown_import(cx))),
                    ),
            );
        Some(rmac_ui::dialog("markdown-import-review", card).into_any_element())
    }

    pub(super) fn render_bundle_import_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        use rmac_ui::DialogButtonKind::{Normal, Primary};

        if let Some(completion) = self.bundle_import_completion {
            let title = if completion.maintenance_pending {
                "Import accepted"
            } else {
                "Import complete"
            };
            let maintenance = if completion.maintenance_pending {
                " The imported library is durable, but verified storage maintenance is still pending. Editing remains paused until recovery finishes."
            } else {
                " Notes verified the accepted library and its imported attachments."
            };
            return Some(
                rmac_ui::alert(
                    title,
                    format!(
                        "Imported {} {}, {} {}, and {} {} ({} of attachments).{maintenance}",
                        completion.folder_count,
                        if completion.folder_count == 1 {
                            "folder"
                        } else {
                            "folders"
                        },
                        completion.note_count,
                        if completion.note_count == 1 {
                            "note"
                        } else {
                            "notes"
                        },
                        completion.attachment_count,
                        if completion.attachment_count == 1 {
                            "attachment"
                        } else {
                            "attachments"
                        },
                        format_storage_bytes(completion.attachment_bytes),
                    ),
                    vec![
                        rmac_ui::dialog_button("dismiss-bundle-import", "Done", Primary)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.dismiss_bundle_import_completion(cx)
                            }))
                            .into_any_element(),
                    ],
                )
                .into_any_element(),
            );
        }

        let review_request_id = self.bundle_review_request_id?;
        if self.bundle_action_request_id.is_some() {
            if matches!(self.session.phase(), SessionPhase::Pending { .. }) {
                return None;
            }
            let card = div()
                .w(px(420.0))
                .p(px(20.0))
                .v_flex()
                .gap_3()
                .rounded(px(rmac_ui::mac::radius_card()))
                .bg(mac::window())
                .border_1()
                .border_color(mac::separator())
                .shadow_xl()
                .child(
                    div()
                        .text_size(rmac_ui::text_px(17.0))
                        .font_weight(mac::BOLD)
                        .child("Applying Notes bundle…"),
                )
                .child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(mac::text_secondary())
                        .child(
                            "Notes is verifying the reviewed source and committing attachments before publishing metadata.",
                        ),
                );
            return Some(rmac_ui::dialog("bundle-import-progress", card).into_any_element());
        }

        let Some((reviewed_request_id, review)) = self.bundle_review else {
            let card = div()
                .w(px(420.0))
                .p(px(20.0))
                .v_flex()
                .gap_3()
                .rounded(px(rmac_ui::mac::radius_card()))
                .bg(mac::window())
                .border_1()
                .border_color(mac::separator())
                .shadow_xl()
                .child(
                    div()
                        .text_size(rmac_ui::text_px(17.0))
                        .font_weight(mac::BOLD)
                        .child("Reviewing Notes bundle…"),
                )
                .child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(mac::text_secondary())
                        .child(
                            "Checking the versioned manifest, note records, hashes, and bounded image payloads. No library changes have been made.",
                        ),
                );
            return Some(rmac_ui::dialog("bundle-review-progress", card).into_any_element());
        };
        if reviewed_request_id != review_request_id {
            return None;
        }
        let collision_detail = if review.identity_collisions == 0
            && review.folder_name_collisions == 0
        {
            "No stable-identity or live folder-name collisions were found.".to_string()
        } else {
            format!(
                "{} stable-identity collision{} will be remapped, and {} live folder-name collision{} will receive a deterministic imported suffix.",
                review.identity_collisions,
                if review.identity_collisions == 1 { "" } else { "s" },
                review.folder_name_collisions,
                if review.folder_name_collisions == 1 { "" } else { "s" },
            )
        };
        let card = div()
            .w(px(460.0))
            .p(px(20.0))
            .v_flex()
            .gap_3()
            .rounded(px(rmac_ui::mac::radius_card()))
            .bg(mac::window())
            .border_1()
            .border_color(mac::separator())
            .shadow_xl()
            .child(
                div()
                    .text_size(rmac_ui::text_px(17.0))
                    .font_weight(mac::BOLD)
                    .child("Import Notes Bundle"),
            )
            .child(
                div()
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(mac::text_secondary())
                    .child(format!(
                        "This review is bound to library revision {} and bundle library revision {}.",
                        review.base_library_revision, review.source_library_revision
                    )),
            )
            .child(
                div()
                    .p_3()
                    .rounded(px(rmac_ui::mac::radius_control()))
                    .bg(mac::control_fill())
                    .v_flex()
                    .gap_1()
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(mac::text_secondary())
                    .child(format!(
                        "{} {}, {} {}, and {} {}",
                        review.folder_count,
                        if review.folder_count == 1 { "folder" } else { "folders" },
                        review.note_count,
                        if review.note_count == 1 { "note" } else { "notes" },
                        review.attachment_count,
                        if review.attachment_count == 1 {
                            "attachment"
                        } else {
                            "attachments"
                        },
                    ))
                    .child(format!(
                        "{} bundle source; {} of attachment payloads",
                        format_storage_bytes(review.source_bytes),
                        format_storage_bytes(review.attachment_bytes),
                    )),
            )
            .child(
                div()
                    .text_size(rmac_ui::text_px(12.0))
                    .text_color(mac::text_secondary())
                    .child(collision_detail),
            )
            .child(
                div()
                    .p_3()
                    .rounded(px(rmac_ui::mac::radius_control()))
                    .bg(mac::control_fill())
                    .text_size(rmac_ui::text_px(12.0))
                    .child(
                        "Keep Both never overwrites an existing note, folder, attachment, or purged identity. Imported records are safely renamed or remapped when needed.",
                    ),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        rmac_ui::dialog_button("cancel-bundle-import", "Cancel", Normal)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.discard_bundle_import_review(cx)
                            })),
                    )
                    .child(
                        rmac_ui::dialog_button(
                            ("accept-bundle-import", review_request_id),
                            "Import and Keep Both",
                            Primary,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.accept_bundle_import(cx))),
                    ),
            );
        Some(rmac_ui::dialog("bundle-import-review", card).into_any_element())
    }

    pub(super) fn render_move_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        use rmac_ui::DialogButtonKind::Normal;

        let dialog = self.move_dialog?;
        let note_title = self
            .session
            .snapshot()
            .and_then(|snapshot| snapshot.notes.iter().find(|note| note.id == dialog.note_id))
            .map_or_else(|| "Note".into(), |note| display_title(&note.title));
        let mut rows = vec![Button::new("move-to-all", "All Notes")
            .selected(dialog.current_folder.is_none())
            .w_full()
            .on_click(cx.listener(|this, _, _, cx| this.move_note_to(None, cx)))
            .into_any_element()];
        rows.extend(self.session.folders().into_iter().map(|folder| {
            let folder_id = folder.id;
            Button::new(("move-to-folder", folder_id.get()), folder.name.clone())
                .selected(dialog.current_folder == Some(folder_id))
                .w_full()
                .on_click(cx.listener(move |this, _, _, cx| this.move_note_to(Some(folder_id), cx)))
                .into_any_element()
        }));
        let card = div()
            .w(px(380.0))
            .max_h(px(480.0))
            .p(px(20.0))
            .v_flex()
            .gap_3()
            .rounded(px(rmac_ui::mac::radius_card()))
            .bg(mac::window())
            .border_1()
            .border_color(mac::separator())
            .shadow_xl()
            .child(
                div()
                    .text_size(rmac_ui::text_px(15.0))
                    .font_weight(mac::BOLD)
                    .child("Move Note"),
            )
            .child(
                div()
                    .text_size(rmac_ui::text_px(13.0))
                    .text_color(mac::text_secondary())
                    .truncate()
                    .child(note_title),
            )
            .child(
                div()
                    .id("move-note-destinations")
                    .max_h(px(330.0))
                    .overflow_y_scroll()
                    .v_flex()
                    .gap_1()
                    .children(rows),
            )
            .child(
                div().flex().justify_end().child(
                    rmac_ui::dialog_button("cancel-move-note", "Cancel", Normal)
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_move_note(cx))),
                ),
            );
        Some(rmac_ui::dialog("move-note-dialog", card).into_any_element())
    }

    pub(super) fn render_folder_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        use rmac_ui::DialogButtonKind::{Destructive, Normal, Primary};

        let dialog = self.folder_dialog?;
        let folder_id = match dialog {
            FolderDialog::Rename(folder_id) | FolderDialog::Delete(folder_id) => folder_id,
        };
        let folder = self
            .session
            .folders()
            .into_iter()
            .find(|folder| folder.id == folder_id)?;
        match dialog {
            FolderDialog::Rename(_) => {
                let card = div()
                    .w(px(360.0))
                    .p(px(20.0))
                    .v_flex()
                    .gap_3()
                    .rounded(px(rmac_ui::mac::radius_card()))
                    .bg(mac::window())
                    .border_1()
                    .border_color(mac::separator())
                    .shadow_xl()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(15.0))
                            .font_weight(mac::BOLD)
                            .child("Rename Folder"),
                    )
                    .child(TextField::new(&self.folder_name_input).cleanable(true))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                rmac_ui::dialog_button("cancel-folder-rename", "Cancel", Normal)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.cancel_folder_dialog(cx)),
                                    ),
                            )
                            .child(
                                rmac_ui::dialog_button("commit-folder-rename", "Rename", Primary)
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.commit_folder_rename(cx)),
                                    ),
                            ),
                    );
                Some(rmac_ui::dialog("rename-folder-dialog", card).into_any_element())
            }
            FolderDialog::Delete(_) => {
                let count = self.session.folder_count(folder_id);
                let message = format!(
                    "Delete “{}”? {} {} will move to All Notes. The notes and their attachments will not be deleted.",
                    folder.name,
                    count,
                    if count == 1 { "note" } else { "notes" }
                );
                Some(
                    rmac_ui::alert(
                        "Delete this folder?",
                        message,
                        vec![
                            rmac_ui::dialog_button("cancel-folder-delete", "Cancel", Normal)
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.cancel_folder_dialog(cx)),
                                )
                                .into_any_element(),
                            rmac_ui::dialog_button(
                                "confirm-folder-delete",
                                "Delete Folder",
                                Destructive,
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.confirm_folder_delete(cx)))
                            .into_any_element(),
                        ],
                    )
                    .into_any_element(),
                )
            }
        }
    }
}
