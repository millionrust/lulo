use super::*;

impl NotesView {
    pub(super) fn render_root(
        &mut self,
        leading_dialog: Option<AnyElement>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let content = match self.session.phase() {
            SessionPhase::Starting if self.message.is_none() => centered_state(
                "Opening Notes…",
                "Checking the private library and recovery state.",
            ),
            SessionPhase::Starting => centered_state(
                "Notes could not start",
                self.message
                    .clone()
                    .unwrap_or_else(|| "The private Notes worker is unavailable.".into()),
            ),
            SessionPhase::MigrationReview(review) => self.render_migration_review(review, cx),
            SessionPhase::Failed(error) => {
                centered_state("Notes could not open", error.to_string())
            }
            SessionPhase::Stopped if !self.closing => centered_state(
                "Notes stopped",
                "Close and reopen the app to reconnect to the private library.",
            ),
            SessionPhase::Ready if self.recovery_review_is_blocking() => self
                .session
                .draft_review()
                .map(|review| self.render_draft_review(review, cx))
                .unwrap_or_else(|| {
                    centered_state("Recovery unavailable", "Close and reopen Notes safely.")
                }),
            SessionPhase::Ready
            | SessionPhase::Maintenance { .. }
            | SessionPhase::Pending { .. }
            | SessionPhase::Stopped => {
                // This is the frame the performance harness must time
                // launch-to-interactive against: the library list and the
                // selected note (if any) are both on screen, not the
                // "Opening Notes…" placeholder above. Safe to call every
                // render; only the first call after main()'s
                // defer_content_ready() writes the benchmark marker.
                rmac_ui::mark_content_ready(window);
                // Notes on macOS 26: a floating folder panel, the note list
                // and the editor, each with its own part of the 52 pt toolbar
                // (design-lab/apps.html).
                let list_focused = self.focus.is_focused(window);
                div()
                    .size_full()
                    .flex()
                    .bg(window_frame())
                    .child(self.render_sidebar(cx))
                    .child(self.render_note_list(list_focused, window, cx))
                    .child(div().w(px(1.0)).h_full().flex_none().bg(column_rule()))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .h_full()
                            .v_flex()
                            .bg(editor_fill())
                            .child(self.render_toolbar(window, cx))
                            .when_some(self.render_status_banner(cx), |element, banner| {
                                element.child(banner)
                            })
                            .child(div().flex_1().min_h(px(0.0)).child(self.render_editor(cx))),
                    )
                    .into_any_element()
            }
        };
        let folder_dialog = self.render_folder_dialog(cx);
        let purge_dialog = self.render_purge_dialog(cx);
        let move_dialog = self.render_move_dialog(cx);
        let attachment_dialog = self.render_attachment_dialog(cx);
        let export_dialog = self.render_export_dialog(cx);
        let markdown_import_dialog = self.render_markdown_import_dialog(cx);
        let bundle_import_dialog = self.render_bundle_import_dialog(cx);

        div()
            .track_focus(&self.focus)
            .key_context("Notes")
            .on_action(cx.listener(|this, _: &ComposeNote, _, cx| this.create_note(cx)))
            .on_action(cx.listener(|this, _: &CreateFolder, _, cx| this.create_folder(cx)))
            .on_action(cx.listener(|this, _: &TrashOrRestore, _, cx| this.trash_or_restore(cx)))
            .on_action(cx.listener(|this, _: &TogglePin, _, cx| this.toggle_pin(cx)))
            .on_action(
                cx.listener(|this, _: &SortByEdited, _, cx| this.set_sort(SortOrder::Edited, cx)),
            )
            .on_action(
                cx.listener(|this, _: &SortByCreated, _, cx| this.set_sort(SortOrder::Created, cx)),
            )
            .on_action(
                cx.listener(|this, _: &SortByTitle, _, cx| this.set_sort(SortOrder::Title, cx)),
            )
            .on_action(
                cx.listener(|this, _: &FocusSearch, window, cx| this.focus_search(window, cx)),
            )
            .on_action(cx.listener(|this, _: &ExportNotes, _, cx| this.begin_export(cx)))
            .on_action(cx.listener(|this, _: &PrintNote, window, cx| this.print_note(window, cx)))
            .on_action(
                cx.listener(|this, _: &ExportNotePdf, window, cx| this.export_note_pdf(window, cx)),
            )
            .on_action(cx.listener(|this, _: &InsertChecklist, window, cx| {
                this.insert_checklist(window, cx)
            }))
            .on_action(cx.listener(|this, _: &RenameSelectedFolder, window, cx| {
                this.begin_folder_rename(window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &DeleteSelectedFolder, _, cx| this.begin_folder_delete(cx)),
            )
            .on_action(cx.listener(|this, _: &MoveSelectedNote, _, cx| this.begin_move_note(cx)))
            .on_action(cx.listener(|this, _: &DeleteNotePermanently, _, cx| {
                this.begin_permanent_note_delete(cx)
            }))
            .on_action(
                cx.listener(|this, _: &EmptyRecentlyDeleted, _, cx| this.begin_empty_trash(cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleMarkdownPreview, _, cx| {
                this.toggle_markdown_preview(cx)
            }))
            .on_action(cx.listener(|this, _: &ImportNote, _, cx| this.choose_text_note_import(cx)))
            .on_action(
                cx.listener(|this, _: &ImportNotesBundle, _, cx| this.choose_bundle_import(cx)),
            )
            .on_action(cx.listener(|this, _: &AddPhoto, _, cx| this.choose_image_attachment(cx)))
            .on_action(cx.listener(|this, _: &rmac_ui::RequestClose, window, cx| {
                this.request_close(window, cx)
            }))
            .size_full()
            .bg(mac::window())
            .text_color(mac::text())
            .child(content)
            .when_some(leading_dialog, |element, dialog| element.child(dialog))
            .when_some(folder_dialog, |element, dialog| element.child(dialog))
            .when_some(purge_dialog, |element, dialog| element.child(dialog))
            .when_some(move_dialog, |element, dialog| element.child(dialog))
            .when_some(attachment_dialog, |element, dialog| element.child(dialog))
            .when_some(export_dialog, |element, dialog| element.child(dialog))
            .when_some(markdown_import_dialog, |element, dialog| {
                element.child(dialog)
            })
            .when_some(bundle_import_dialog, |element, dialog| {
                element.child(dialog)
            })
            .into_any_element()
    }
}
