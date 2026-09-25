//! Notes toolbar projection and interaction wiring.
//!
//! macOS 26 Notes splits its 52 pt toolbar by column (design-lab/apps.html):
//! the folder name and View Options sit above the note list, and the editor
//! column carries compose, a centred format capsule, a ⋯ capsule and search.

use super::*;

/// A Tahoe toolbar capsule: 36 tall, full radius, a faint fill and edge.
pub(super) fn capsule(id: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(CAPSULE_HEIGHT))
        .flex_none()
        .flex()
        .items_center()
        .px(px(0.5))
        .rounded(px(CAPSULE_HEIGHT / 2.0))
        .bg(capsule_fill())
        .border_1()
        .border_color(capsule_edge())
}

/// A glyph button inside a capsule (38 × 34), or a free 36 circle.
pub(super) fn glyph_button(
    id: &'static str,
    path: &'static str,
    width: f32,
    tooltip: &'static str,
) -> Button {
    Button::new(id, "")
        .ghost()
        .icon(Icon::empty().path(path))
        .with_size(Size::Size(px(TOOLBAR_GLYPH / 0.75)))
        .tooltip(tooltip)
        .w(px(width))
        .h(px(CAPSULE_HEIGHT - 2.0))
        .rounded(px(CAPSULE_HEIGHT / 2.0))
        .text_color(toolbar_glyph())
}

impl NotesView {
    /// Pointer handlers that turn a press-and-drag on a toolbar's empty area
    /// into a window move.
    pub(super) fn toolbar_drag(
        &self,
        element: Stateful<Div>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        element
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, _| this.dragging = true),
            )
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, _| this.dragging = false),
            )
            .on_mouse_move(cx.listener(|this, _, window, _| {
                if this.dragging {
                    this.dragging = false;
                    window.start_window_move();
                }
            }))
    }

    /// The editor column's toolbar.
    pub(super) fn render_toolbar(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let ready = self.is_interactive_ready();
        let selected = self.session.selected_note();
        let deleted = selected.is_some_and(|note| note.deleted);
        let pinned = selected.is_some_and(|note| note.pinned);
        let has_note = selected.is_some();
        let note_save_pending = self.latest_local_generation.is_some();
        let attachment_busy = self.attachment_chooser_open || self.attachment_action_pending();
        let preview_visible = self.markdown_preview_visible;
        // The search field narrows with the editor column, up to the Mac's
        // 326 pt at full width.
        let editor_width = f32::from(rmac_ui::window_content_size(window).width)
            - SIDEBAR_WIDTH
            - LIST_WIDTH
            - 1.0;
        let search_width = (editor_width - 280.0).clamp(SEARCH_MIN_WIDTH, SEARCH_MAX_WIDTH);

        let compose = glyph_button("compose", glyphs::COMPOSE, CAPSULE_HEIGHT, "New Note")
            .disabled(!ready)
            .bg(capsule_fill())
            .border_1()
            .border_color(capsule_edge())
            .h(px(CAPSULE_HEIGHT))
            .on_click(cx.listener(|this, _, _, cx| this.create_note(cx)));

        let format = capsule("format-capsule")
            .child(
                glyph_button(
                    "checklist",
                    glyphs::CHECKLIST,
                    CAPSULE_BUTTON_WIDTH,
                    "Checklist",
                )
                .disabled(!ready || deleted || !has_note || preview_visible)
                .on_click(cx.listener(|this, _, window, cx| this.insert_checklist(window, cx))),
            )
            .child(
                glyph_button(
                    "add-image",
                    glyphs::ATTACH,
                    CAPSULE_BUTTON_WIDTH,
                    "Add Photo…",
                )
                .busy(attachment_busy)
                .disabled(!ready || deleted || !has_note || note_save_pending)
                .on_click(cx.listener(|this, _, _, cx| this.choose_image_attachment(cx))),
            );

        let more = capsule("note-capsule")
            .child(
                glyph_button(
                    "move-note",
                    glyphs::FOLDER,
                    CAPSULE_BUTTON_WIDTH,
                    "Move Note…",
                )
                .disabled(!ready || deleted || !has_note)
                .on_click(cx.listener(|this, _, _, cx| this.begin_move_note(cx))),
            )
            .child(
                glyph_button("note-more", glyphs::MORE, CAPSULE_BUTTON_WIDTH, "More")
                    .disabled(!ready || !has_note)
                    .dropdown_menu(move |menu, _, _| {
                        let menu = if deleted {
                            menu.menu("Recover Note", Box::new(TrashOrRestore))
                                .menu("Delete Permanently…", Box::new(DeleteNotePermanently))
                        } else {
                            menu.menu(
                                if pinned { "Unpin Note" } else { "Pin Note" },
                                Box::new(TogglePin),
                            )
                            .menu("Move Note…", Box::new(MoveSelectedNote))
                            .menu("Delete", Box::new(TrashOrRestore))
                            .separator()
                            .menu("Add Photo…", Box::new(AddPhoto))
                        };
                        menu.separator().menu(
                            if preview_visible {
                                "Edit Note"
                            } else {
                                "Show Markdown Preview"
                            },
                            Box::new(ToggleMarkdownPreview),
                        )
                    }),
            );

        let search = div()
            .id("notes-search")
            .role(Role::SearchInput)
            .aria_label("Search")
            .accessible_text_input(&self.search_query, cx)
            .on_a11y_action(
                AccessibleAction::SetValue,
                self.assistive_search_listener(cx),
            )
            .on_a11y_action(
                AccessibleAction::ReplaceSelectedText,
                self.assistive_search_listener(cx),
            )
            .w(px(search_width))
            .h(px(CAPSULE_HEIGHT))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.0))
            .pl(px(12.0))
            .pr(px(6.0))
            .rounded(px(CAPSULE_HEIGHT / 2.0))
            .bg(capsule_fill())
            .border_1()
            .border_color(capsule_edge())
            .text_size(rmac_ui::text_px(13.0))
            .child(glyph(glyphs::SEARCH, 15.0, search_placeholder()))
            .child(
                div().flex_1().min_w(px(0.0)).child(
                    TextField::new(&self.search_query)
                        .appearance(false)
                        .cleanable(true)
                        .small()
                        .disabled(self.session.snapshot().is_none()),
                ),
            );

        let bar = div()
            .id("notes-editor-toolbar")
            .role(Role::Toolbar)
            .aria_label("Toolbar")
            .h(px(TOOLBAR_HEIGHT))
            .w_full()
            .flex_none()
            .flex()
            .items_center()
            .pl(px(COMPOSE_LEFT))
            .pr(px(TRAILING_MARGIN))
            .child(compose)
            .child(div().flex_1().min_w(px(8.0)))
            .child(format)
            .child(div().flex_1().min_w(px(8.0)))
            .child(more)
            .child(div().w(px(SEARCH_GAP)).flex_none())
            .child(search);
        self.toolbar_drag(bar, cx)
    }

    /// The note list's part of the toolbar: the folder's name and note
    /// count, and View Options (sort, import, export).
    pub(super) fn render_list_toolbar(
        &self,
        title: SharedString,
        subtitle: SharedString,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let ready = self.is_interactive_ready();
        let note_save_pending = self.latest_local_generation.is_some();
        let has_library = self.session.snapshot().is_some();
        let in_trash =
            self.session.folder_selection() == rmac_notes_runtime::FolderSelection::Trash;
        let trash_has_notes = in_trash && !self.session.visible_notes().is_empty();
        let sort_order = self
            .session
            .snapshot()
            .map(|snapshot| snapshot.sort_order)
            .unwrap_or(SortOrder::Edited);
        let bar = div()
            .id("notes-list-toolbar")
            .h(px(TOOLBAR_HEIGHT))
            .w_full()
            .flex_none()
            .relative()
            .child(
                div()
                    .absolute()
                    .left(px(LIST_TITLE_X))
                    .top(px(10.75))
                    .right(px(LIST_MORE_RIGHT + CAPSULE_HEIGHT + 8.0))
                    .v_flex()
                    .child(
                        div()
                            .h(px(17.0))
                            .text_size(rmac_ui::text_px(13.0))
                            .line_height(px(17.0))
                            .font_weight(mac::BOLD)
                            .text_color(list_title())
                            .truncate()
                            .child(title),
                    )
                    .child(
                        div()
                            .h(px(13.0))
                            .text_size(rmac_ui::text_px(11.0))
                            .line_height(px(13.0))
                            .text_color(list_subtitle())
                            .truncate()
                            .child(subtitle),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .right(px(LIST_MORE_RIGHT))
                    .top(px(CAPSULE_TOP))
                    .child(
                        glyph_button("view-options", glyphs::MORE, CAPSULE_HEIGHT, "View Options")
                            .h(px(CAPSULE_HEIGHT))
                            .bg(capsule_fill())
                            .border_1()
                            .border_color(capsule_edge())
                            .disabled(!ready)
                            .dropdown_menu(move |menu, _, _| {
                                let menu = menu
                                    .menu_with_check(
                                        "Sort by Date Edited",
                                        sort_order == SortOrder::Edited,
                                        Box::new(SortByEdited),
                                    )
                                    .menu_with_check(
                                        "Sort by Date Created",
                                        sort_order == SortOrder::Created,
                                        Box::new(SortByCreated),
                                    )
                                    .menu_with_check(
                                        "Sort by Title",
                                        sort_order == SortOrder::Title,
                                        Box::new(SortByTitle),
                                    )
                                    .separator()
                                    .menu("Import Note…", Box::new(ImportNote));
                                let menu = if note_save_pending {
                                    menu
                                } else {
                                    menu.menu("Import Notes Bundle…", Box::new(ImportNotesBundle))
                                };
                                let menu = if has_library && !note_save_pending {
                                    menu.menu("Export Notes…", Box::new(ExportNotes))
                                } else {
                                    menu
                                };
                                if trash_has_notes {
                                    menu.separator().menu(
                                        "Empty Recently Deleted…",
                                        Box::new(EmptyRecentlyDeleted),
                                    )
                                } else {
                                    menu
                                }
                            }),
                    ),
            );
        self.toolbar_drag(bar, cx)
    }
}
