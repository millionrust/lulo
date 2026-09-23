//! Notes toolbar projection and interaction wiring.

use super::*;

impl NotesView {
    pub(super) fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ready = self.is_interactive_ready();
        let selected = self.session.selected_note();
        let deleted = selected.is_some_and(|note| note.deleted);
        let pinned = selected.is_some_and(|note| note.pinned);
        let note_save_pending = self.latest_local_generation.is_some();
        let attachment_busy = self.attachment_chooser_open || self.attachment_action_pending();
        let note_import_busy = self.note_import_chooser_open
            || self.note_import_request_id.is_some()
            || self.markdown_import_action_request_id.is_some();
        let export_busy = self.export_chooser_open || self.export_request_id.is_some();
        let bundle_import_busy = self.bundle_chooser_open
            || self.bundle_review_request_id.is_some()
            || self.bundle_action_request_id.is_some();
        let sort_order = self
            .session
            .snapshot()
            .map(|snapshot| snapshot.sort_order)
            .unwrap_or(SortOrder::Edited);
        let row = div()
            .size_full()
            .flex()
            .items_center()
            .child(div().w(px(FOLDERS_W - 76.0)))
            .child(
                div()
                    .w(px(LIST_W))
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_1()
                    .pr_3()
                    .child(
                        Button::new("sort", "")
                            .icon(IconName::SortDescending)
                            .ghost()
                            .with_size(Size::Medium)
                            .disabled(!ready)
                            .tooltip("Sort Notes")
                            .dropdown_menu(move |menu, _, _| {
                                menu.menu_with_check(
                                    "Date Edited",
                                    sort_order == SortOrder::Edited,
                                    Box::new(SortByEdited),
                                )
                                .menu_with_check(
                                    "Date Created",
                                    sort_order == SortOrder::Created,
                                    Box::new(SortByCreated),
                                )
                                .menu_with_check(
                                    "Title",
                                    sort_order == SortOrder::Title,
                                    Box::new(SortByTitle),
                                )
                            }),
                    )
                    .child(
                        Button::new("import-note", "")
                            .icon(IconName::File)
                            .ghost()
                            .with_size(Size::Medium)
                            .busy(note_import_busy)
                            .disabled(!ready)
                            .tooltip(if note_import_busy {
                                "Importing Note…"
                            } else {
                                "Import Note…"
                            })
                            .on_click(
                                cx.listener(|this, _, _, cx| this.choose_text_note_import(cx)),
                            ),
                    )
                    .child(
                        Button::new("import-bundle", "")
                            .icon(IconName::FolderOpen)
                            .ghost()
                            .with_size(Size::Medium)
                            .busy(bundle_import_busy)
                            .disabled(!ready || note_save_pending)
                            .tooltip(if bundle_import_busy {
                                "Importing Notes Bundle…"
                            } else if note_save_pending {
                                "Saving Note…"
                            } else {
                                "Import Notes Bundle…"
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.choose_bundle_import(cx))),
                    )
                    .child(
                        Button::new("compose", "")
                            .icon(IconName::Plus)
                            .ghost()
                            .with_size(Size::Medium)
                            .disabled(!ready)
                            .tooltip("New Note")
                            .on_click(cx.listener(|this, _, _, cx| this.create_note(cx))),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_1()
                    .pr_4()
                    .child(
                        div()
                            .w(px(220.0))
                            .h(px(28.0))
                            .flex()
                            .items_center()
                            .gap_1()
                            .px_2()
                            .rounded(px(rmac_ui::mac::radius_segmented()))
                            .bg(mac::control_fill())
                            .child(
                                Icon::new(IconName::Search)
                                    .with_size(Size::XSmall)
                                    .text_color(mac::text_tertiary()),
                            )
                            .child(
                                TextField::new(&self.search_query)
                                    .appearance(false)
                                    .cleanable(true)
                                    .small()
                                    .disabled(self.session.snapshot().is_none()),
                            ),
                    )
                    .child(
                        Button::new("export-notes", "")
                            .icon(IconName::ExternalLink)
                            .ghost()
                            .with_size(Size::Medium)
                            .busy(export_busy)
                            .disabled(
                                !ready || self.session.snapshot().is_none() || note_save_pending,
                            )
                            .tooltip(if export_busy {
                                "Exporting Notes…"
                            } else if note_save_pending {
                                "Saving Note…"
                            } else {
                                "Export…"
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.begin_export(cx))),
                    )
                    .child(
                        Button::new("checklist", "")
                            .icon(IconName::CircleCheck)
                            .ghost()
                            .with_size(Size::Medium)
                            .disabled(
                                !ready
                                    || deleted
                                    || selected.is_none()
                                    || self.markdown_preview_visible,
                            )
                            .tooltip("Checklist")
                            .on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.insert_checklist(window, cx)
                                }),
                            ),
                    )
                    .child(
                        Button::new("add-image", "")
                            .icon(IconName::GalleryVerticalEnd)
                            .ghost()
                            .with_size(Size::Medium)
                            .busy(attachment_busy)
                            .disabled(!ready || deleted || selected.is_none() || note_save_pending)
                            .tooltip(if attachment_busy {
                                "Updating Attachments…"
                            } else if note_save_pending {
                                "Saving Note…"
                            } else {
                                "Add Photo…"
                            })
                            .on_click(
                                cx.listener(|this, _, _, cx| this.choose_image_attachment(cx)),
                            ),
                    )
                    .child(
                        Button::new("move-note", "")
                            .icon(IconName::Folder)
                            .ghost()
                            .with_size(Size::Medium)
                            .disabled(!ready || deleted || selected.is_none())
                            .tooltip("Move Note…")
                            .on_click(cx.listener(|this, _, _, cx| this.begin_move_note(cx))),
                    )
                    .child(
                        Button::new("pin", "")
                            .icon(IconName::Star)
                            .ghost()
                            .selected(pinned)
                            .with_size(Size::Medium)
                            .disabled(!ready || deleted || selected.is_none())
                            .tooltip(if pinned { "Unpin Note" } else { "Pin Note" })
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_pin(cx))),
                    )
                    .child(
                        Button::new("trash", "")
                            .icon(if deleted {
                                IconName::ArrowUp
                            } else {
                                IconName::Delete
                            })
                            .ghost()
                            .with_size(Size::Medium)
                            .disabled(!ready || selected.is_none())
                            .tooltip(if deleted {
                                "Restore Note"
                            } else {
                                "Move to Trash"
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.trash_or_restore(cx))),
                    )
                    .when(deleted, |element| {
                        element.child(
                            Button::new("delete-permanently", "")
                                .icon(IconName::Delete)
                                .destructive()
                                .with_size(Size::Medium)
                                .disabled(!ready)
                                .tooltip("Delete Note Permanently…")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.begin_permanent_note_delete(cx)
                                })),
                        )
                    }),
            );
        rmac_ui::toolbar(row)
    }
}
