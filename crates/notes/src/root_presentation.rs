use super::*;

impl NotesView {
    pub(super) fn render_root(
        &mut self,
        leading_dialog: Option<AnyElement>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if window.is_window_active() {
            self.publish_menu_state(window, cx);
        }
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
                    .when(self.folders_visible, |element| {
                        element.child(self.render_sidebar(cx))
                    })
                    .child(self.render_note_list(list_focused, window, cx))
                    .child(div().w(px(1.0)).h_full().flex_none().bg(column_rule()))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .h_full()
                            .v_flex()
                            .bg(editor_fill())
                            .when(self.toolbar_visible, |element| {
                                element.child(self.render_toolbar(window, cx))
                            })
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
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let key = &event.keystroke;
                if key.key != "f"
                    || key.modifiers.platform
                    || key.modifiers.control
                    || key.modifiers.alt
                    || key.modifiers.shift
                {
                    return;
                }
                let text_field_focused = [
                    &this.search_query,
                    &this.folder_name_input,
                    &this.title,
                    &this.tags,
                    &this.body,
                    &this.note_find_input,
                    &this.note_replace_input,
                ]
                .iter()
                .any(|input| input.read(cx).focus_handle(cx).is_focused(window));
                if !text_field_focused {
                    window.toggle_fullscreen();
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|this, _: &ComposeNote, _, cx| this.create_note(cx)))
            .on_action(cx.listener(|this, _: &CreateFolder, _, cx| this.create_folder(cx)))
            .on_action(cx.listener(|this, _: &TrashOrRestore, _, cx| this.trash_or_restore(cx)))
            .on_action(cx.listener(|this, _: &DeleteSelectedNote, _, cx| {
                this.delete_selected_note_with_undo(cx)
            }))
            .on_action(cx.listener(|this, _: &CloseAll, window, cx| this.request_close(window, cx)))
            // Notes has exactly one window, which is recreated at launch.
            // Preserve its durable library through the same reviewed close
            // path used by the window controls.
            .on_action(cx.listener(|this, _: &QuitAndKeepWindows, window, cx| {
                this.request_close(window, cx)
            }))
            .on_action(cx.listener(|_, _: &FocusMainWindow, window, _| {
                window.activate_window();
            }))
            .on_action(cx.listener(|this, _: &PreviousRecentNote, window, cx| {
                this.navigate_recent_note(1, window, cx)
            }))
            .on_action(cx.listener(|this, _: &NextRecentNote, window, cx| {
                this.navigate_recent_note(-1, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ClearRecentNotes, _, cx| this.clear_recent_notes(cx)))
            .on_action(cx.listener(|_, _: &ToggleFullScreen, window, _| window.toggle_fullscreen()))
            .on_action(cx.listener(|this, _: &OpenRecentNote0, window, cx| {
                this.open_recent_note(0, window, cx)
            }))
            .on_action(cx.listener(|this, _: &OpenRecentNote1, window, cx| {
                this.open_recent_note(1, window, cx)
            }))
            .on_action(cx.listener(|this, _: &OpenRecentNote2, window, cx| {
                this.open_recent_note(2, window, cx)
            }))
            .on_action(cx.listener(|this, _: &OpenRecentNote3, window, cx| {
                this.open_recent_note(3, window, cx)
            }))
            .on_action(cx.listener(|this, _: &OpenRecentNote4, window, cx| {
                this.open_recent_note(4, window, cx)
            }))
            .on_action(cx.listener(|this, _: &OpenRecentNote5, window, cx| {
                this.open_recent_note(5, window, cx)
            }))
            .on_action(cx.listener(|this, _: &OpenRecentNote6, window, cx| {
                this.open_recent_note(6, window, cx)
            }))
            .on_action(cx.listener(|this, _: &OpenRecentNote7, window, cx| {
                this.open_recent_note(7, window, cx)
            }))
            .on_action(cx.listener(|this, _: &OpenRecentNote8, window, cx| {
                this.open_recent_note(8, window, cx)
            }))
            .on_action(cx.listener(|this, _: &OpenRecentNote9, window, cx| {
                this.open_recent_note(9, window, cx)
            }))
            .on_action(cx.listener(|this, _: &TogglePin, _, cx| this.toggle_pin(cx)))
            .on_action(cx.listener(|this, _: &DuplicateNote, _, cx| this.duplicate_note(cx)))
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
            .on_action(
                cx.listener(|this, _: &FindInNote, window, cx| this.toggle_note_find(window, cx)),
            )
            .on_action(cx.listener(|this, _: &FindAndReplace, window, cx| {
                this.open_note_replace(window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &FindInNoteNext, window, cx| this.note_find_next(window, cx)),
            )
            .on_action(cx.listener(|this, _: &FindInNotePrevious, window, cx| {
                this.note_find_previous(window, cx)
            }))
            .on_action(cx.listener(|this, _: &UseSelectionForFind, window, cx| {
                this.use_selection_for_find(window, cx)
            }))
            .on_action(cx.listener(|this, _: &JumpToSelection, window, cx| {
                this.jump_to_selection(window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &PastePlainText, window, cx| {
                    this.paste_plain_text(window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &MakeUppercase, window, cx| {
                this.transform_selection(TextTransform::Uppercase, window, cx)
            }))
            .on_action(cx.listener(|this, _: &MakeLowercase, window, cx| {
                this.transform_selection(TextTransform::Lowercase, window, cx)
            }))
            .on_action(cx.listener(|this, _: &Capitalise, window, cx| {
                this.transform_selection(TextTransform::Capitalise, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ExportNotes, _, cx| this.begin_export(cx)))
            .on_action(cx.listener(|this, _: &PrintNote, window, cx| this.print_note(window, cx)))
            .on_action(
                cx.listener(|this, _: &ExportNotePdf, window, cx| this.export_note_pdf(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ExportNoteMarkdown, _, cx| this.export_note_markdown(cx)),
            )
            .on_action(cx.listener(|this, _: &InsertChecklist, window, cx| {
                this.insert_checklist(window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleChecklistDone, window, cx| {
                this.toggle_checklist_line(window, cx)
            }))
            .on_action(cx.listener(|this, _: &TickAll, window, cx| {
                this.apply_checklist_bulk(ChecklistBulkAction::TickAll, window, cx)
            }))
            .on_action(cx.listener(|this, _: &UntickAll, window, cx| {
                this.apply_checklist_bulk(ChecklistBulkAction::UntickAll, window, cx)
            }))
            .on_action(cx.listener(|this, _: &MoveTickedToBottom, window, cx| {
                this.apply_checklist_bulk(ChecklistBulkAction::MoveTickedToBottom, window, cx)
            }))
            .on_action(cx.listener(|this, _: &DeleteTicked, window, cx| {
                this.apply_checklist_bulk(ChecklistBulkAction::DeleteTicked, window, cx)
            }))
            .on_action(cx.listener(|this, _: &MoveItemUp, window, cx| {
                this.move_current_list_item(true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &MoveItemDown, window, cx| {
                this.move_current_list_item(false, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &InsertTable, window, cx| this.insert_table(window, cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleBold, window, cx| this.toggle_bold(window, cx)))
            .on_action(
                cx.listener(|this, _: &ToggleItalic, window, cx| this.toggle_italic(window, cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleStrikethrough, window, cx| {
                this.toggle_strikethrough(window, cx)
            }))
            .on_action(cx.listener(|this, _: &SetStyleTitle, window, cx| {
                this.set_paragraph_style(ParagraphStyle::Title, window, cx)
            }))
            .on_action(cx.listener(|this, _: &SetStyleHeading, window, cx| {
                this.set_paragraph_style(ParagraphStyle::Heading, window, cx)
            }))
            .on_action(cx.listener(|this, _: &SetStyleSubheading, window, cx| {
                this.set_paragraph_style(ParagraphStyle::Subheading, window, cx)
            }))
            .on_action(cx.listener(|this, _: &SetStyleBody, window, cx| {
                this.set_paragraph_style(ParagraphStyle::Body, window, cx)
            }))
            .on_action(cx.listener(|this, _: &SetStyleMonospaced, window, cx| {
                this.set_paragraph_style(ParagraphStyle::Monospaced, window, cx)
            }))
            .on_action(cx.listener(|this, _: &InsertBulletedList, window, cx| {
                this.insert_list_marker(ListMarker::Bulleted, window, cx)
            }))
            .on_action(cx.listener(|this, _: &InsertNumberedList, window, cx| {
                this.insert_list_marker(ListMarker::Numbered, window, cx)
            }))
            .on_action(cx.listener(|this, _: &InsertDashedList, window, cx| {
                this.insert_list_marker(ListMarker::Dashed, window, cx)
            }))
            .on_action(cx.listener(|this, _: &InsertBlockQuote, window, cx| {
                this.insert_block_quote(window, cx)
            }))
            .on_action(cx.listener(|this, _: &InsertLink, window, cx| this.insert_link(window, cx)))
            .on_action(cx.listener(|this, _: &IncreaseIndent, window, cx| {
                this.change_indent(true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &DecreaseIndent, window, cx| {
                this.change_indent(false, window, cx)
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
            .on_action(cx.listener(|this, _: &ToggleFolders, _, cx| {
                this.folders_visible = !this.folders_visible;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleNoteCount, _, cx| {
                this.show_note_count = !this.show_note_count;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleToolbar, _, cx| {
                this.toolbar_visible = !this.toolbar_visible;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ShowListView, _, cx| {
                this.gallery_view = false;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ShowGalleryView, _, cx| {
                this.gallery_view = true;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &CollapseSection, _, cx| {
                this.set_selected_section_collapsed(true, cx);
            }))
            .on_action(cx.listener(|this, _: &ExpandSection, _, cx| {
                this.set_selected_section_collapsed(false, cx);
            }))
            .on_action(cx.listener(|this, _: &CollapseAllSections, _, cx| {
                this.set_all_sections_collapsed(true, cx);
            }))
            .on_action(cx.listener(|this, _: &ExpandAllSections, _, cx| {
                this.set_all_sections_collapsed(false, cx);
            }))
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| {
                this.note_zoom = (this.note_zoom + 1).min(12);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| {
                this.note_zoom = (this.note_zoom - 1).max(-5);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ZoomReset, _, cx| {
                this.note_zoom = 0;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ImportNote, _, cx| this.choose_text_note_import(cx)))
            .on_action(
                cx.listener(|this, _: &ImportMarkdown, _, cx| this.choose_text_note_import(cx)),
            )
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
