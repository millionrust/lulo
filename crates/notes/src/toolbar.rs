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

/// Whether the window is too narrow for the folder list beside the note
/// list and an editor column its toolbar fits in.
pub(super) fn narrow_window(window: &Window) -> bool {
    !folder_list_fits(f32::from(rmac_ui::window_content_size(window).width))
}

/// Whether a window `width` wide has room for the folder list, the note
/// list and an editor column.
fn folder_list_fits(width: f32) -> bool {
    width >= SIDEBAR_WIDTH + LIST_WIDTH + 1.0 + EDITOR_MIN_WIDTH
}

/// What the editor column's toolbar has room for. As in the Mac's
/// toolbars, a narrow window first collapses the search field to its
/// magnifier button, then moves items from the right into a » menu: the ⋯
/// capsule's, then Search itself. A search the user opened (focused, or
/// holding a query) keeps its field, and the ⋯ capsule and then the format
/// capsule give way to it instead. In a half-screen window Search and More
/// were cut off at the window's edge (Windows robot).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ToolbarFit {
    pub search: SearchSlot,
    /// The ⋯ capsule's items are in the » menu.
    pub more_in_overflow: bool,
    /// The format capsule's items are in the » menu.
    pub format_in_overflow: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SearchSlot {
    /// The search field, at least `SEARCH_MIN_WIDTH` wide.
    Field,
    /// The field, opened in a narrow window: at least `SEARCH_NARROW_WIDTH`.
    NarrowField,
    /// The magnifier button that opens the field.
    Button,
    /// A Search row in the » menu.
    InOverflow,
}

impl ToolbarFit {
    pub(super) fn overflow(self) -> bool {
        self.more_in_overflow || self.format_in_overflow || self.search == SearchSlot::InOverflow
    }
}

pub(super) fn toolbar_fit(
    editor_width: f32,
    format_width: f32,
    more_width: f32,
    searching: bool,
) -> ToolbarFit {
    // Compose, the room either side of the format capsule, the margin.
    let frame = COMPOSE_LEFT + CAPSULE_HEIGHT + 2.0 * TOOLBAR_SPACER + TRAILING_MARGIN;
    // `search` 0 is no search item; `overflow` adds the » at the far end.
    let fits = |format: f32, more: f32, search: f32, overflow: bool| {
        let search = if search > 0.0 {
            SEARCH_GAP + search
        } else {
            0.0
        };
        let overflow = match (overflow, more + search > 0.0) {
            (false, _) => 0.0,
            (true, true) => TOOLBAR_SPACER + CAPSULE_HEIGHT,
            (true, false) => CAPSULE_HEIGHT,
        };
        frame + format + more + search + overflow <= editor_width
    };
    let fit = |search, more_in_overflow, format_in_overflow| ToolbarFit {
        search,
        more_in_overflow,
        format_in_overflow,
    };
    if fits(format_width, more_width, SEARCH_MIN_WIDTH, false) {
        return fit(SearchSlot::Field, false, false);
    }
    if searching {
        return if fits(format_width, more_width, SEARCH_NARROW_WIDTH, false) {
            fit(SearchSlot::NarrowField, false, false)
        } else if fits(format_width, 0.0, SEARCH_NARROW_WIDTH, true) {
            fit(SearchSlot::NarrowField, true, false)
        } else {
            fit(SearchSlot::NarrowField, true, true)
        };
    }
    if fits(format_width, more_width, CAPSULE_HEIGHT, false) {
        fit(SearchSlot::Button, false, false)
    } else if fits(format_width, 0.0, CAPSULE_HEIGHT, true) {
        fit(SearchSlot::Button, true, false)
    } else if fits(format_width, 0.0, 0.0, true) {
        fit(SearchSlot::InOverflow, true, false)
    } else {
        fit(SearchSlot::InOverflow, true, true)
    }
}

/// A capsule's width around buttons `buttons` wide: its 0.5 padding and
/// 1 px edge on each side.
fn capsule_width(buttons: f32) -> f32 {
    buttons + 3.0
}

/// Give an icon-only component button an accessibility name and a Click
/// action. The inner button keeps its current mouse behavior; disabled
/// controls do not publish a Click action.
pub(super) fn accessible_icon_button(
    id: &'static str,
    name: &'static str,
    enabled: bool,
    button: impl IntoElement,
    view: Entity<NotesView>,
    activate: impl Fn(&mut NotesView, &mut Window, &mut Context<NotesView>) + 'static,
) -> impl IntoElement {
    let activate = std::rc::Rc::new(activate);
    let a11y_activate = activate.clone();
    let keyboard_view = view.clone();
    rmac_ui::KeyboardAction::new(
        SharedString::from(format!("{id}-keyboard")),
        div()
            .id(SharedString::from(format!("{id}-a11y")))
            .role(Role::Button)
            .aria_label(name)
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .when(enabled, move |element| {
                element.on_a11y_action(AccessibleAction::Click, move |_, window, cx| {
                    view.update(cx, |this, cx| a11y_activate(this, window, cx));
                })
            })
            .child(button),
        move |window, cx| {
            keyboard_view.update(cx, |this, cx| activate(this, window, cx));
        },
    )
    .disabled(!enabled)
}

impl NotesView {
    /// Whether the folder list is on screen: the user's choice, unless the
    /// window is too narrow for it and the user has not asked for it since.
    pub(super) fn folders_shown(&self) -> bool {
        self.folders_visible && (!self.window_narrow || self.folders_shown_while_narrow)
    }

    /// View ▸ Show/Hide Folders and the toolbar's Folders button.
    pub(super) fn toggle_folders(&mut self, cx: &mut Context<Self>) {
        if self.window_narrow {
            if self.folders_shown() {
                self.folders_shown_while_narrow = false;
            } else {
                self.folders_visible = true;
                self.folders_shown_while_narrow = true;
            }
        } else {
            self.folders_visible = !self.folders_visible;
            self.folders_shown_while_narrow = false;
        }
        cx.notify();
    }

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
                cx.listener(|this, event: &gpui::MouseDownEvent, _, _| {
                    this.dragging = Some(event.position)
                }),
            )
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, _| this.dragging = None),
            )
            .on_mouse_move(
                cx.listener(|this, event: &gpui::MouseMoveEvent, window, _| {
                    if event.pressed_button == Some(gpui::MouseButton::Left)
                        && this.dragging.is_some_and(|press| {
                            let delta = event.position - press;
                            delta.x.abs() > px(4.0) || delta.y.abs() > px(4.0)
                        })
                    {
                        this.dragging = None;
                        window.start_window_move();
                    }
                }),
            )
            .on_click(|event, _, cx| {
                if event.click_count() == 2 {
                    rmac_ui::double_click_title_bar_action(cx);
                }
            })
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
        let locked = selected.is_some_and(|note| note.lock.is_some());
        let current_folder = selected.and_then(|note| note.folder_id);
        // The ⋯ menu's Move to and Recent Notes submenus.
        let folders: Vec<(FolderId, String)> = self
            .session
            .folders()
            .into_iter()
            .map(|folder| (folder.id, folder.name.clone()))
            .collect();
        let recent: Vec<String> = self
            .recent_notes
            .iter()
            .filter_map(|id| {
                self.session
                    .snapshot()?
                    .notes
                    .iter()
                    .find(|note| note.id == *id && !note.deleted)
                    .map(|note| {
                        if note.lock.is_some() {
                            "Locked Note".to_owned()
                        } else {
                            display_title(&note.title).to_string()
                        }
                    })
            })
            .take(10)
            .collect();
        let large_attachments = self.attachment_view_large;
        let has_note = selected.is_some();
        let note_save_pending = self.latest_local_generation.is_some();
        let attachment_busy = self.attachment_chooser_open || self.attachment_action_pending();
        let preview_visible = self.markdown_preview_visible;
        let maths_results_mode = self.maths_results_mode;
        // The search field narrows with the editor column, up to the Mac's
        // 326 pt at full width.
        let editor_width = f32::from(rmac_ui::window_content_size(window).width)
            - if self.folders_shown() {
                SIDEBAR_WIDTH
            } else {
                0.0
            }
            - LIST_WIDTH
            - 1.0;
        let search_width = (editor_width - 360.0).clamp(SEARCH_MIN_WIDTH, SEARCH_MAX_WIDTH);
        let shown = |item| !self.hidden_toolbar_items.contains(&item);
        let format_width = capsule_width(
            CAPSULE_BUTTON_WIDTH
                + [
                    (ToolbarItem::Checklist, CAPSULE_BUTTON_WIDTH),
                    (ToolbarItem::InsertTable, CAPSULE_BUTTON_WIDTH),
                    (ToolbarItem::MathsResults, CAPSULE_BUTTON_WIDTH + 14.0),
                    (ToolbarItem::Media, CAPSULE_BUTTON_WIDTH),
                ]
                .into_iter()
                .filter(|(item, _)| shown(*item))
                .map(|(_, width)| width)
                .sum::<f32>(),
        );
        let more_width = capsule_width(
            CAPSULE_BUTTON_WIDTH
                + if shown(ToolbarItem::MoveNote) {
                    CAPSULE_BUTTON_WIDTH
                } else {
                    0.0
                },
        );
        let searching = !self.search_query.read(cx).value().is_empty()
            || self
                .search_query
                .read(cx)
                .focus_handle(cx)
                .is_focused(window);
        let fit = toolbar_fit(editor_width, format_width, more_width, searching);

        let compose_button = glyph_button("compose", glyphs::COMPOSE, CAPSULE_HEIGHT, "New Note")
            .disabled(!ready)
            .bg(capsule_fill())
            .border_1()
            .border_color(capsule_edge())
            .h(px(CAPSULE_HEIGHT))
            .on_click(cx.listener(|this, _, _, cx| this.create_note(cx)));
        // `rmac_ui::Button` currently exposes its tooltip but not an
        // accessibility-only name. Keep the glyph-only visual control and
        // publish a named, actionable wrapper so AT clients can reliably
        // identify and activate New Note.
        let view = cx.entity();
        let compose = accessible_icon_button(
            "compose",
            "New Note",
            ready,
            compose_button,
            view.clone(),
            |this, _, cx| this.create_note(cx),
        );

        let format = (!fit.format_in_overflow).then(|| {
            capsule("format-capsule")
                .child(
                    PopUpButton::new("note-format", "Format")
                        .menu_button(glyphs::FORMAT, TOOLBAR_GLYPH)
                        .w(px(CAPSULE_BUTTON_WIDTH))
                        .h(px(CAPSULE_HEIGHT - 2.0))
                        .rounded(px(CAPSULE_HEIGHT / 2.0))
                        .text_color(toolbar_glyph())
                        .disabled(!ready || deleted || !has_note || preview_visible)
                        .dropdown_menu(|menu, _, _| {
                            menu.menu("Title", Box::new(SetStyleTitle))
                                .menu("Heading", Box::new(SetStyleHeading))
                                .menu("Subheading", Box::new(SetStyleSubheading))
                                .menu("Body", Box::new(SetStyleBody))
                                .menu("Monostyled", Box::new(SetStyleMonospaced))
                                .separator()
                                .menu("Bold", Box::new(ToggleBold))
                                .menu("Italic", Box::new(ToggleItalic))
                                .menu("Strikethrough", Box::new(ToggleStrikethrough))
                                .separator()
                                .menu("Bulleted List", Box::new(InsertBulletedList))
                                .menu("Dashed List", Box::new(InsertDashedList))
                                .menu("Numbered List", Box::new(InsertNumberedList))
                        }),
                )
                // View ▸ Customise Toolbar… (NOT-MENU-054): each optional
                // button after Format can be hidden; Format itself, like the
                // Mac's own undraggable items, always stays.
                .when(
                    !self.hidden_toolbar_items.contains(&ToolbarItem::Checklist),
                    |el| {
                        el.child(accessible_icon_button(
                            "checklist",
                            "Checklist",
                            ready && !deleted && has_note && !preview_visible,
                            glyph_button(
                                "checklist",
                                glyphs::CHECKLIST,
                                CAPSULE_BUTTON_WIDTH,
                                "Checklist",
                            )
                            .disabled(!ready || deleted || !has_note || preview_visible)
                            .on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.insert_checklist(window, cx)
                                }),
                            ),
                            view.clone(),
                            |this, window, cx| this.insert_checklist(window, cx),
                        ))
                    },
                )
                .when(
                    !self
                        .hidden_toolbar_items
                        .contains(&ToolbarItem::InsertTable),
                    |el| {
                        el.child(accessible_icon_button(
                            "insert-table",
                            "Table",
                            ready && !deleted && has_note && !preview_visible,
                            glyph_button(
                                "insert-table",
                                glyphs::TABLE,
                                CAPSULE_BUTTON_WIDTH,
                                "Table",
                            )
                            .disabled(!ready || deleted || !has_note || preview_visible)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.insert_table(window, cx)),
                            ),
                            view.clone(),
                            |this, window, cx| this.insert_table(window, cx),
                        ))
                    },
                )
                .when(
                    !self
                        .hidden_toolbar_items
                        .contains(&ToolbarItem::MathsResults),
                    |el| {
                        el.child(
                            // NOT-TOOLBAR-001: the Mac's toolbar button between
                            // Format and Table shows the current Format ▸ Maths
                            // Results mode; this keeps a fixed face (like the
                            // Format/Media buttons beside it) and shows the live
                            // mode as the dropdown's checkmark instead.
                            PopUpButton::new("note-maths-results", "Off")
                                .w(px(CAPSULE_BUTTON_WIDTH + 14.0))
                                .h(px(CAPSULE_HEIGHT - 2.0))
                                .rounded(px(CAPSULE_HEIGHT / 2.0))
                                .text_color(toolbar_glyph())
                                .disabled(!ready)
                                .dropdown_menu(move |menu, _, _| {
                                    menu.menu_with_check(
                                        "Off",
                                        maths_results_mode == MathsResultsMode::Off,
                                        Box::new(MathsResultsOff),
                                    )
                                    .menu_with_check(
                                        "Suggest Results",
                                        maths_results_mode == MathsResultsMode::SuggestResults,
                                        Box::new(MathsResultsSuggest),
                                    )
                                    .menu_with_check(
                                        "Insert Results",
                                        maths_results_mode == MathsResultsMode::InsertResults,
                                        Box::new(MathsResultsInsert),
                                    )
                                }),
                        )
                    },
                )
                .when(
                    !self.hidden_toolbar_items.contains(&ToolbarItem::Media),
                    |el| {
                        el.child(
                            PopUpButton::new("note-media", "Media")
                                .menu_button(glyphs::ATTACH, TOOLBAR_GLYPH)
                                .w(px(CAPSULE_BUTTON_WIDTH))
                                .h(px(CAPSULE_HEIGHT - 2.0))
                                .rounded(px(CAPSULE_HEIGHT / 2.0))
                                .text_color(toolbar_glyph())
                                .disabled(
                                    !ready
                                        || deleted
                                        || !has_note
                                        || note_save_pending
                                        || attachment_busy,
                                )
                                .dropdown_menu(|menu, _, _| {
                                    menu.menu("Add Photo…", Box::new(AddPhoto))
                                }),
                        )
                    },
                )
        });

        // The ⋯ menu's items, as the Mac's (toolbar_More): pin and lock,
        // then Find in Note, Move to, Recent Notes, Maths Results and
        // Attachment View, then Delete Note. Also in the » menu when the ⋯
        // capsule has no room.
        macro_rules! more_items {
            ($menu:expr, $window:expr, $cx:expr, $folders:expr, $recent:expr) => {{
                let menu = $menu;
                if deleted {
                    menu.menu("Recover Note", Box::new(TrashOrRestore))
                        .menu("Delete Permanently…", Box::new(DeleteNotePermanently))
                } else {
                    let folders = $folders.clone();
                    let recent = $recent.clone();
                    menu.menu(
                        if pinned { "Unpin Note" } else { "Pin Note" },
                        Box::new(TogglePin),
                    )
                    .menu(
                        if locked { "Remove Lock" } else { "Lock Note" },
                        Box::new(ToggleLockNote),
                    )
                    .separator()
                    .menu("Find in Note", Box::new(FindInNote))
                    .submenu("Move to", $window, $cx, move |menu, _, _| {
                        let mut menu = menu.menu_with_check(
                            "All Notes",
                            current_folder.is_none(),
                            Box::new(MoveNoteToFolderAction { folder_id: None }),
                        );
                        for (id, name) in &folders {
                            menu = menu.menu_with_check(
                                name.clone(),
                                current_folder == Some(*id),
                                Box::new(MoveNoteToFolderAction {
                                    folder_id: Some(*id),
                                }),
                            );
                        }
                        menu
                    })
                    .submenu("Recent Notes", $window, $cx, move |menu, _, _| {
                        let actions: [Box<dyn gpui::Action>; 10] = [
                            Box::new(OpenRecentNote0),
                            Box::new(OpenRecentNote1),
                            Box::new(OpenRecentNote2),
                            Box::new(OpenRecentNote3),
                            Box::new(OpenRecentNote4),
                            Box::new(OpenRecentNote5),
                            Box::new(OpenRecentNote6),
                            Box::new(OpenRecentNote7),
                            Box::new(OpenRecentNote8),
                            Box::new(OpenRecentNote9),
                        ];
                        let mut menu = menu;
                        for (title, action) in recent.iter().zip(actions) {
                            menu = menu.menu(title.clone(), action);
                        }
                        menu.separator()
                            .menu("Clear Menu", Box::new(ClearRecentNotes))
                    })
                    .submenu("Maths Results", $window, $cx, move |menu, _, _| {
                        menu.menu_with_check(
                            "Off",
                            maths_results_mode == MathsResultsMode::Off,
                            Box::new(MathsResultsOff),
                        )
                        .menu_with_check(
                            "Suggest Results",
                            maths_results_mode == MathsResultsMode::SuggestResults,
                            Box::new(MathsResultsSuggest),
                        )
                        .menu_with_check(
                            "Insert Results",
                            maths_results_mode == MathsResultsMode::InsertResults,
                            Box::new(MathsResultsInsert),
                        )
                    })
                    .submenu("Attachment View", $window, $cx, move |menu, _, _| {
                        menu.menu_with_check(
                            "Set All to Small",
                            !large_attachments,
                            Box::new(SetAllAttachmentsSmall),
                        )
                        .menu_with_check(
                            "Set All to Large",
                            large_attachments,
                            Box::new(SetAllAttachmentsLarge),
                        )
                    })
                    .menu("Delete Note", Box::new(TrashOrRestore))
                }
            }};
        }
        let more = (!fit.more_in_overflow).then(|| {
            capsule("note-capsule")
                .when(
                    !self.hidden_toolbar_items.contains(&ToolbarItem::MoveNote),
                    |el| {
                        el.child(accessible_icon_button(
                            "move-note",
                            "Move Note…",
                            ready && !deleted && has_note,
                            glyph_button(
                                "move-note",
                                glyphs::FOLDER,
                                CAPSULE_BUTTON_WIDTH,
                                "Move Note…",
                            )
                            .disabled(!ready || deleted || !has_note)
                            .on_click(cx.listener(|this, _, _, cx| this.begin_move_note(cx))),
                            view.clone(),
                            |this, _, cx| this.begin_move_note(cx),
                        ))
                    },
                )
                .child(
                    PopUpButton::new("note-more", "More")
                        .menu_button(glyphs::MORE, TOOLBAR_GLYPH)
                        .w(px(CAPSULE_BUTTON_WIDTH))
                        .h(px(CAPSULE_HEIGHT - 2.0))
                        .rounded(px(CAPSULE_HEIGHT / 2.0))
                        .text_color(toolbar_glyph())
                        .disabled(!ready || !has_note)
                        .dropdown_menu({
                            let folders = folders.clone();
                            let recent = recent.clone();
                            move |menu, window, cx| more_items!(menu, window, cx, folders, recent)
                        }),
                )
        });
        // » at the far end: what the narrow toolbar has no room for, as rows.
        let format_over = fit.format_in_overflow;
        let more_over = fit.more_in_overflow;
        let search_over = fit.search == SearchSlot::InOverflow;
        let checklist_shown = shown(ToolbarItem::Checklist);
        let table_shown = shown(ToolbarItem::InsertTable);
        let media_shown = shown(ToolbarItem::Media);
        let overflow = fit.overflow().then(|| {
            PopUpButton::new("note-overflow", "More Toolbar Items")
                .menu_button(glyphs::OVERFLOW, TOOLBAR_GLYPH)
                .w(px(CAPSULE_HEIGHT))
                .h(px(CAPSULE_HEIGHT))
                .rounded(px(CAPSULE_HEIGHT / 2.0))
                .text_color(toolbar_glyph())
                .bg(capsule_fill())
                .border_1()
                .border_color(capsule_edge())
                .disabled(!ready)
                .dropdown_menu(move |menu, window, cx| {
                    let mut menu = menu;
                    let mut rows = false;
                    let editing = has_note && !deleted && !preview_visible;
                    if format_over && editing {
                        menu = menu
                            .menu("Title", Box::new(SetStyleTitle))
                            .menu("Heading", Box::new(SetStyleHeading))
                            .menu("Subheading", Box::new(SetStyleSubheading))
                            .menu("Body", Box::new(SetStyleBody))
                            .menu("Monostyled", Box::new(SetStyleMonospaced))
                            .separator()
                            .menu("Bold", Box::new(ToggleBold))
                            .menu("Italic", Box::new(ToggleItalic))
                            .menu("Strikethrough", Box::new(ToggleStrikethrough))
                            .separator()
                            .menu("Bulleted List", Box::new(InsertBulletedList))
                            .menu("Dashed List", Box::new(InsertDashedList))
                            .menu("Numbered List", Box::new(InsertNumberedList));
                        if checklist_shown || table_shown || media_shown {
                            menu = menu.separator();
                        }
                        if checklist_shown {
                            menu = menu.menu("Checklist", Box::new(InsertChecklist));
                        }
                        if table_shown {
                            menu = menu.menu("Table", Box::new(InsertTable));
                        }
                        if media_shown {
                            menu = menu.menu("Add Photo…", Box::new(AddPhoto));
                        }
                        rows = true;
                    }
                    if more_over && has_note {
                        if rows {
                            menu = menu.separator();
                        }
                        menu = more_items!(menu, window, cx, folders, recent);
                        rows = true;
                    }
                    if search_over {
                        if rows {
                            menu = menu.separator();
                        }
                        menu = menu.menu("Search", Box::new(FocusSearch));
                    }
                    menu
                })
        });

        let search = match fit.search {
            SearchSlot::InOverflow => None,
            SearchSlot::Button => Some(
                accessible_icon_button(
                    "search-button",
                    "Search",
                    true,
                    glyph_button("search-button", glyphs::SEARCH, CAPSULE_HEIGHT, "Search")
                        .bg(capsule_fill())
                        .border_1()
                        .border_color(capsule_edge())
                        .h(px(CAPSULE_HEIGHT))
                        .on_click(cx.listener(|this, _, window, cx| this.focus_search(window, cx))),
                    view.clone(),
                    |this, window, cx| this.focus_search(window, cx),
                )
                .into_any_element(),
            ),
            SearchSlot::Field | SearchSlot::NarrowField => Some(
                div()
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
                    // UIA-17: a fixed `.w()` here (with the row's other items all
                    // `flex_none`) let the capsule's computed width — only a rough
                    // estimate of the format/more capsules' real width, which
                    // varies with how many optional toolbar items (Checklist,
                    // Table, Maths Results, Media, Move Note) are visible — run
                    // past the window's right edge whenever the estimate was a
                    // little short, instead of giving way. `flex_shrink` plus a
                    // `min_w` floor lets the search capsule itself absorb that
                    // error and any narrower window, down to the Mac's measured
                    // minimum, before anything clips.
                    .flex_shrink(1.0)
                    .flex_grow(0.0)
                    .flex_basis(px(search_width))
                    .min_w(px(if fit.search == SearchSlot::NarrowField {
                        SEARCH_NARROW_WIDTH
                    } else {
                        SEARCH_MIN_WIDTH
                    }))
                    .max_w(px(SEARCH_MAX_WIDTH))
                    .h(px(CAPSULE_HEIGHT))
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
                    )
                    .into_any_element(),
            ),
        };

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
            .child(div().flex_1().min_w(px(TOOLBAR_SPACER)))
            .children(format)
            .child(div().flex_1().min_w(px(TOOLBAR_SPACER)))
            .children(more)
            .when_some(search, |bar, search| {
                bar.child(div().w(px(SEARCH_GAP)).flex_none()).child(search)
            })
            .when_some(overflow, |bar, overflow| {
                bar.when(!more_over || !search_over, |bar| {
                    bar.child(div().w(px(TOOLBAR_SPACER)).flex_none())
                })
                .child(overflow)
            });
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
        let gallery_view = self.gallery_view;
        let attachments_browser_visible = self.attachments_browser_visible;
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
                    .left(px(3.0))
                    .top(px(CAPSULE_TOP))
                    .child(accessible_icon_button(
                        "toggle-folders",
                        "Folders",
                        true,
                        glyph_button("toggle-folders", glyphs::FOLDER, CAPSULE_HEIGHT, "Folders")
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_folders(cx))),
                        cx.entity(),
                        |this, _, cx| this.toggle_folders(cx),
                    )),
            )
            .child(
                div()
                    .absolute()
                    .left(px(LIST_TITLE_X + 39.0))
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
                        PopUpButton::new("view-options", "View Options")
                            .menu_button(glyphs::MORE, TOOLBAR_GLYPH)
                            .w(px(CAPSULE_HEIGHT))
                            .h(px(CAPSULE_HEIGHT))
                            .rounded(px(CAPSULE_HEIGHT / 2.0))
                            .text_color(toolbar_glyph())
                            .bg(capsule_fill())
                            .border_1()
                            .border_color(capsule_edge())
                            .disabled(!ready)
                            .dropdown_menu(move |menu, _, _| {
                                let menu = menu
                                    .menu_with_check(
                                        "as List",
                                        !gallery_view,
                                        Box::new(ShowListView),
                                    )
                                    .menu_with_check(
                                        "as Gallery",
                                        gallery_view,
                                        Box::new(ShowGalleryView),
                                    )
                                    .menu(
                                        if attachments_browser_visible {
                                            "Hide Attachments Browser"
                                        } else {
                                            "Show Attachments Browser"
                                        },
                                        Box::new(ToggleAttachmentsBrowser),
                                    )
                                    .separator()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every optional item shown: Format, Checklist, Table, Maths Results
    /// and Media; Move Note and More.
    const FORMAT: f32 = 207.0;
    const MORE: f32 = 79.0;

    fn editor_width(window: f32, folders: bool) -> f32 {
        window - if folders { SIDEBAR_WIDTH } else { 0.0 } - LIST_WIDTH - 1.0
    }

    #[test]
    fn a_wide_window_shows_every_item() {
        let fit = toolbar_fit(editor_width(1366.0, true), FORMAT, MORE, false);
        assert_eq!(fit.search, SearchSlot::Field);
        assert!(!fit.overflow());
    }

    #[test]
    fn a_narrowing_window_collapses_search_then_moves_items_into_the_overflow_menu() {
        // A 1000-wide window: the search field collapses to its button.
        let fit = toolbar_fit(editor_width(1000.0, true), FORMAT, MORE, false);
        assert_eq!(fit.search, SearchSlot::Button);
        assert!(!fit.overflow());
        // Narrower: the ⋯ capsule goes into », the search button stays.
        let fit = toolbar_fit(FORMAT + 175.0, FORMAT, MORE, false);
        assert_eq!(fit.search, SearchSlot::Button);
        assert!(fit.more_in_overflow && !fit.format_in_overflow);
        // A half-screen window on a 1366-wide PC hides the folder list;
        // Search joins More in the » menu and the format capsule stays.
        let fit = toolbar_fit(editor_width(683.0, false), FORMAT, MORE, false);
        assert_eq!(fit.search, SearchSlot::InOverflow);
        assert!(fit.more_in_overflow && !fit.format_in_overflow);
    }

    #[test]
    fn an_open_search_keeps_its_field_and_the_capsules_give_way() {
        let fit = toolbar_fit(editor_width(1000.0, true), FORMAT, MORE, true);
        assert_eq!(fit.search, SearchSlot::NarrowField);
        assert!(fit.more_in_overflow && !fit.format_in_overflow);
        let fit = toolbar_fit(editor_width(683.0, false), FORMAT, MORE, true);
        assert_eq!(fit.search, SearchSlot::NarrowField);
        assert!(fit.more_in_overflow && fit.format_in_overflow);
    }

    #[test]
    fn what_shows_fits_the_editor_column() {
        // Narrower than the narrowest layout (compose, a field or », and »)
        // nothing more can give way.
        let narrowest = COMPOSE_LEFT
            + 2.0 * CAPSULE_HEIGHT
            + 2.0 * TOOLBAR_SPACER
            + TRAILING_MARGIN
            + SEARCH_GAP
            + SEARCH_NARROW_WIDTH
            + TOOLBAR_SPACER;
        for width in (200..1200).step_by(7) {
            let width = width as f32;
            if width < narrowest {
                continue;
            }
            for searching in [false, true] {
                let fit = toolbar_fit(width, FORMAT, MORE, searching);
                let format = if fit.format_in_overflow { 0.0 } else { FORMAT };
                let more = if fit.more_in_overflow { 0.0 } else { MORE };
                let search = match fit.search {
                    SearchSlot::Field => SEARCH_GAP + SEARCH_MIN_WIDTH,
                    SearchSlot::NarrowField => SEARCH_GAP + SEARCH_NARROW_WIDTH,
                    SearchSlot::Button => SEARCH_GAP + CAPSULE_HEIGHT,
                    SearchSlot::InOverflow => 0.0,
                };
                let overflow = if !fit.overflow() {
                    0.0
                } else if more + search > 0.0 {
                    TOOLBAR_SPACER + CAPSULE_HEIGHT
                } else {
                    CAPSULE_HEIGHT
                };
                let needed = COMPOSE_LEFT
                    + CAPSULE_HEIGHT
                    + 2.0 * TOOLBAR_SPACER
                    + TRAILING_MARGIN
                    + format
                    + more
                    + search
                    + overflow;
                assert!(needed <= width, "{width} {searching} {fit:?}");
            }
        }
    }

    #[test]
    fn a_half_screen_window_hides_the_folder_list() {
        assert!(!folder_list_fits(683.0));
        assert!(folder_list_fits(1366.0));
    }
}
