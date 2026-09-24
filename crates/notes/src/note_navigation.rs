//! Notes folder navigation and searchable note-list projection.

use super::*;

impl NotesView {
    pub(super) fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use rmac_notes_runtime::FolderSelection;

        let snapshot = self.session.snapshot();
        // A single pass over every note, rather than two separate `.filter()`
        // scans: cheap while the library is small, but this sidebar count
        // recomputes on every render (not just first paint), so it should
        // not cost twice what it needs to as a real library grows.
        let (all_count, trash_count) = snapshot.map_or((0, 0), |snapshot| {
            snapshot.notes.iter().fold((0, 0), |(all, trash), note| {
                if note.deleted {
                    (all, trash + 1)
                } else {
                    (all + 1, trash)
                }
            })
        });
        let current = self.session.folder_selection();
        let has_selected_folder = matches!(current, FolderSelection::Folder(_));
        let mut rows = div()
            .id("notes-folders")
            .role(Role::List)
            .aria_label("Folders")
            .flex_1()
            .min_h(px(0.0))
            .overflow_y_scroll()
            .v_flex()
            // Rows sit 11 in from the panel's outer edges (1 pt of that is
            // the panel's rim).
            .px(px(SIDEBAR_ROW_INSET - 1.0))
            .pb(px(SIDEBAR_ROW_INSET))
            .child(
                div()
                    .h(px(SIDEBAR_SECTION_HEIGHT))
                    .mt(px(4.0))
                    .flex()
                    .items_center()
                    .justify_between()
                    .pl(px(SIDEBAR_SECTION_TEXT_X - SIDEBAR_ROW_INSET))
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(11.0))
                            .font_weight(mac::BOLD)
                            .text_color(sidebar_section_text())
                            .child("On My Computer"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_0p5()
                            .child(
                                Button::new("folder-actions", "")
                                    .icon(IconName::Ellipsis)
                                    .ghost()
                                    .with_size(Size::XSmall)
                                    .text_color(sidebar_section_text())
                                    .disabled(!self.is_interactive_ready() || !has_selected_folder)
                                    .tooltip("Folder Actions")
                                    .dropdown_menu(|menu, _, _| {
                                        menu.menu("Rename Folder…", Box::new(RenameSelectedFolder))
                                            .menu("Delete Folder…", Box::new(DeleteSelectedFolder))
                                    }),
                            )
                            .child(
                                Button::new("new-folder", "")
                                    .icon(IconName::Plus)
                                    .ghost()
                                    .with_size(Size::XSmall)
                                    .text_color(sidebar_section_text())
                                    .disabled(!self.is_interactive_ready())
                                    .tooltip("New Folder")
                                    .on_click(cx.listener(|this, _, _, cx| this.create_folder(cx))),
                            ),
                    ),
            )
            .child(folder_row(
                "all-notes",
                "All Notes",
                glyphs::FOLDER,
                all_count,
                current == FolderSelection::All,
                cx.listener(|this, _, window, cx| {
                    this.select_folder(FolderSelection::All, window, cx)
                }),
            ));

        for folder in self.session.folders() {
            let folder_id = folder.id;
            rows = rows.child(folder_row(
                ("folder", folder_id.get()),
                folder.name.clone(),
                glyphs::FOLDER,
                self.session.folder_count(folder_id),
                current == FolderSelection::Folder(folder_id),
                cx.listener(move |this, _, window, cx| {
                    this.select_folder(FolderSelection::Folder(folder_id), window, cx)
                }),
            ));
        }

        rows = rows.child(folder_row(
            "trash-notes",
            "Recently Deleted",
            glyphs::TRASH,
            trash_count,
            current == FolderSelection::Trash,
            cx.listener(|this, _, window, cx| {
                this.select_folder(FolderSelection::Trash, window, cx)
            }),
        ));

        // The traffic lights live inside the floating panel (window-relative
        // centres 26 / 49 / 72, y 26), so place the cluster by its centre.
        let traffic_left =
            TRAFFIC_LIGHT_FIRST_CENTRE - mac::traffic_light_hit_width() / 2.0 - SIDEBAR_INSET;
        let traffic_top =
            TRAFFIC_LIGHT_FIRST_CENTRE - mac::traffic_light_hit_height() / 2.0 - SIDEBAR_INSET;
        let titlebar = self.toolbar_drag(
            div()
                .id("notes-sidebar-titlebar")
                .h(px(TOOLBAR_HEIGHT - SIDEBAR_INSET))
                .flex_none()
                .relative()
                .child(
                    div()
                        .absolute()
                        .left(px(traffic_left - 1.0))
                        .top(px(traffic_top - 1.0))
                        .child(rmac_ui::traffic_lights()),
                ),
            cx,
        );

        div()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .flex_shrink_0()
            .relative()
            .child(
                div()
                    .id("notes-sidebar-panel")
                    .absolute()
                    .left(px(SIDEBAR_INSET))
                    .top(px(SIDEBAR_INSET))
                    .bottom(px(SIDEBAR_BOTTOM_INSET))
                    .right_0()
                    .v_flex()
                    .rounded(px(SIDEBAR_RADIUS))
                    .bg(sidebar_panel())
                    .border_1()
                    .border_color(sidebar_panel_edge())
                    .overflow_hidden()
                    .child(titlebar)
                    .child(rows),
            )
    }

    pub(super) fn render_note_list(
        &self,
        list_focused: bool,
        _window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let search_active = !self.search_query.read(cx).value().trim().is_empty();
        let selected = if search_active {
            self.search.selected()
        } else {
            self.session.selected_note_id()
        };
        let notes: Vec<(&NoteRecord, Option<&SearchHit>)> =
            if search_active && self.search.state() == SearchState::Results {
                self.session.snapshot().map_or_else(Vec::new, |snapshot| {
                    self.search
                        .hits()
                        .iter()
                        .filter_map(|hit| {
                            snapshot
                                .notes
                                .iter()
                                .find(|note| note.id == hit.note_id && !note.deleted)
                                .map(|note| (note, Some(hit)))
                        })
                        .collect()
                })
            } else if search_active {
                Vec::new()
            } else {
                self.session
                    .visible_notes()
                    .into_iter()
                    .map(|note| (note, None))
                    .collect()
            };
        let note_count = notes.len();
        // Notes groups a date-sorted list under Pinned / Today / Yesterday /
        // Previous 7 Days / … headers (S: sizes from platform knowledge).
        let sectioned = !search_active
            && self
                .session
                .snapshot()
                .is_some_and(|snapshot| snapshot.sort_order != SortOrder::Title);
        let sort_by_created = self
            .session
            .snapshot()
            .is_some_and(|snapshot| snapshot.sort_order == SortOrder::Created);
        let mut current_section: Option<SharedString> = None;
        let mut items = Vec::<AnyElement>::new();
        let section_of = |note: &NoteRecord| -> Option<SharedString> {
            sectioned.then(|| {
                if note.pinned {
                    "Pinned".into()
                } else if sort_by_created {
                    date_section(note.created_unix_ms)
                } else {
                    date_section(note.modified_unix_ms)
                }
            })
        };
        let sections = notes
            .iter()
            .map(|(note, _)| section_of(note))
            .collect::<Vec<_>>();
        let selected_flags = notes
            .iter()
            .map(|(note, _)| selected == Some(note.id))
            .collect::<Vec<_>>();
        for (index, (note, search_hit)) in notes.into_iter().enumerate() {
            let next_selected = selected_flags.get(index + 1).copied();
            // The last row of a section (or of the list) has no rule.
            let is_last = sections
                .get(index + 1)
                .is_none_or(|next| *next != sections[index]);
            if let Some(section) = sections[index].clone() {
                if current_section.as_ref() != Some(&section) {
                    // A 40 pt section row: the name 15 bold at x 16.5, then a
                    // full-width rule 29 below the row's top.
                    items.push(
                        div()
                            .h(px(SECTION_HEIGHT))
                            .flex_none()
                            .relative()
                            .child(
                                div()
                                    .absolute()
                                    .left(px(SECTION_TEXT_X))
                                    .top(px(1.0))
                                    .text_size(rmac_ui::text_px(15.0))
                                    .line_height(px(18.0))
                                    .font_weight(mac::BOLD)
                                    .text_color(section_text())
                                    .child(section.clone()),
                            )
                            .child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .right_0()
                                    .top(px(SECTION_RULE_Y))
                                    .h(px(1.0))
                                    .bg(section_rule()),
                            )
                            .into_any_element(),
                    );
                    current_section = Some(section);
                }
            }
            let note_id = note.id;
            let is_selected = selected == Some(note.id);
            let title_source = if note.title.trim().is_empty() {
                "New Note"
            } else {
                note.title.as_str()
            };
            let title_match = search_hit.and_then(|hit| {
                hit.matches
                    .iter()
                    .find(|search_match| matches!(search_match.field, SearchField::Title))
            });
            let title_fragment = title_match
                .and_then(|search_match| {
                    matched_search_fragment(
                        title_source,
                        search_match.span.start_byte..search_match.span.end_byte,
                        MAX_SEARCH_TITLE_FRAGMENT_CHARS,
                    )
                })
                .unwrap_or_else(|| {
                    plain_search_fragment(title_source, MAX_SEARCH_TITLE_FRAGMENT_CHARS)
                });
            let body_match = search_hit.and_then(|hit| {
                hit.matches
                    .iter()
                    .find(|search_match| matches!(search_match.field, SearchField::Body))
            });
            let mut body_fragment = body_match
                .and_then(|search_match| {
                    matched_search_fragment(
                        &note.body,
                        search_match.span.start_byte..search_match.span.end_byte,
                        MAX_SEARCH_DETAIL_FRAGMENT_CHARS,
                    )
                })
                .unwrap_or_else(|| {
                    plain_search_fragment(&note.body, MAX_SEARCH_DETAIL_FRAGMENT_CHARS)
                });
            if body_fragment.text().trim().is_empty() {
                body_fragment =
                    plain_search_fragment("No additional text", MAX_SEARCH_DETAIL_FRAGMENT_CHARS);
            }
            let tags = note
                .tags
                .iter()
                .enumerate()
                .map(|(index, tag)| {
                    let matched = search_hit.and_then(|hit| {
                        hit.matches.iter().find(|search_match| {
                            matches!(search_match.field, SearchField::Tag { index: found } if found == index)
                        })
                    });
                    matched
                        .and_then(|search_match| {
                            matched_search_fragment(
                                tag,
                                search_match.span.start_byte..search_match.span.end_byte,
                                MAX_SEARCH_LABEL_FRAGMENT_CHARS,
                            )
                        })
                        .unwrap_or_else(|| {
                            plain_search_fragment(tag, MAX_SEARCH_LABEL_FRAGMENT_CHARS)
                        })
                        .with_prefix("#")
                })
                .collect::<Vec<_>>();
            let attachment_matches = search_hit.map_or_else(Vec::new, |hit| {
                let Some(snapshot) = self.session.snapshot() else {
                    return Vec::new();
                };
                hit.matches
                    .iter()
                    .filter_map(|search_match| {
                        let SearchField::AttachmentName { attachment_id } = search_match.field
                        else {
                            return None;
                        };
                        let attachment = snapshot.attachments.iter().find(|attachment| {
                            attachment.id == attachment_id
                                && attachment.note_id == note.id
                                && !attachment.deleted
                        })?;
                        matched_search_fragment(
                            &attachment.display_name,
                            search_match.span.start_byte..search_match.span.end_byte,
                            MAX_SEARCH_LABEL_FRAGMENT_CHARS,
                        )
                        .map(|fragment| fragment.with_prefix("Photo: "))
                    })
                    .collect::<Vec<_>>()
            });
            let matches_truncated = search_hit.is_some_and(|hit| hit.matches_truncated);
            // No AccessKit `description` setter is exposed on `div()`, so the
            // date and preview join the title in one accessible name, the
            // same fold `presentation::folder_row` uses for its count.
            let mut accessible_label = String::new();
            if note.pinned {
                accessible_label.push_str("Pinned, ");
            }
            accessible_label.push_str(title_fragment.text());
            accessible_label.push_str(", ");
            accessible_label.push_str(&date_label(note.modified_unix_ms));
            accessible_label.push_str(", ");
            accessible_label.push_str(body_fragment.text());
            let title_colour = if is_selected {
                selection_text(list_focused)
            } else {
                mac::text()
            };
            let preview_colour = if is_selected {
                selection_preview(list_focused)
            } else {
                mac::text_secondary()
            };
            // Hairlines separate unselected neighbours, inset from the text.
            let rule = !is_selected && next_selected != Some(true) && !is_last;
            items.push(
                div()
                    .id(("note", note.id.get()))
                    .role(Role::ListItem)
                    .aria_label(accessible_label)
                    .aria_selected(is_selected)
                    .px(px(NOTE_ROW_INSET))
                    .py(px(0.5))
                    .relative()
                    .child(
                        div()
                            .min_h(px(NOTE_ROW_HEIGHT - 1.0))
                            .pl(px(NOTE_TEXT_X))
                            .pr(px(12.0))
                            .pt(px(11.25))
                            .pb(px(8.0))
                            .rounded(px(NOTE_ROW_RADIUS))
                            .when(is_selected, |element| {
                                element.bg(selection_fill(list_focused))
                            })
                            .v_flex()
                            .child(
                                div()
                                    .h(px(17.0))
                                    .text_size(rmac_ui::text_px(13.0))
                                    .line_height(px(17.0))
                                    .font_weight(mac::BOLD)
                                    .text_color(title_colour)
                                    .truncate()
                                    .child(styled_search_fragment_in(
                                        title_fragment,
                                        title_colour,
                                        true,
                                    )),
                            )
                            .child(
                                div()
                                    .h(px(15.0))
                                    .flex()
                                    .items_center()
                                    .gap(px(NOTE_TIME_GAP))
                                    .text_size(rmac_ui::text_px(12.0))
                                    .line_height(px(15.0))
                                    .child(
                                        div()
                                            .flex_none()
                                            .text_color(title_colour)
                                            .child(date_label(note.modified_unix_ms)),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w(px(0.0))
                                            .text_color(preview_colour)
                                            .truncate()
                                            .child(styled_search_fragment_in(
                                                body_fragment,
                                                preview_colour,
                                                false,
                                            )),
                                    ),
                            )
                            .when(!tags.is_empty(), |element| {
                                element.child(
                                    div()
                                        .flex()
                                        .flex_wrap()
                                        .gap_1()
                                        .pt_1()
                                        .children(tags.into_iter().map(tag_pill)),
                                )
                            })
                            .when(!attachment_matches.is_empty(), |element| {
                                element.child(div().v_flex().gap_0p5().pt_0p5().children(
                                    attachment_matches.into_iter().map(attachment_match_row),
                                ))
                            })
                            .when(matches_truncated, |element| {
                                element.child(
                                    div()
                                        .pt_0p5()
                                        .text_size(rmac_ui::text_px(10.0))
                                        .text_color(mac::text_tertiary())
                                        .child("More matches in this note"),
                                )
                            }),
                    )
                    .when(rule, |element| {
                        element.child(
                            div()
                                .absolute()
                                .left(px(NOTE_ROW_INSET + NOTE_TEXT_X))
                                .right(px(NOTE_ROW_INSET))
                                .bottom_0()
                                .h(px(1.0))
                                .bg(row_rule()),
                        )
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        // Clicking a note gives the list the keyboard, as on
                        // the Mac: the selection turns yellow until the
                        // editor takes focus again.
                        window.focus(&this.focus, cx);
                        if search_active {
                            this.select_search_result(note_id, window, cx)
                        } else {
                            this.select_note(note_id, window, cx)
                        }
                    }))
                    .into_any_element(),
            );
        }
        if items.is_empty() {
            let empty_message: SharedString = if search_active {
                match self.search.state() {
                    SearchState::Indexing => "Searching…".into(),
                    SearchState::NoMatches => "No matching notes".into(),
                    SearchState::Unavailable => self.search.failure().map_or_else(
                        || "Search is unavailable".into(),
                        |error| error.to_string().into(),
                    ),
                    SearchState::Empty | SearchState::Results => "Search is unavailable".into(),
                }
            } else if self.session.folder_selection() == rmac_notes_runtime::FolderSelection::Trash
            {
                "Recently Deleted is empty".into()
            } else {
                "No notes in this folder".into()
            };
            items.push(
                div()
                    .px_4()
                    .py_6()
                    .text_size(rmac_ui::text_px(13.0))
                    .text_color(mac::text_tertiary())
                    .child(empty_message)
                    .into_any_element(),
            );
        }
        let (title, subtitle): (SharedString, SharedString) = if search_active {
            (
                "Search".into(),
                format!(
                    "{note_count} {}",
                    if note_count == 1 { "result" } else { "results" }
                )
                .into(),
            )
        } else {
            let title: SharedString = match self.session.folder_selection() {
                rmac_notes_runtime::FolderSelection::All => "All Notes".into(),
                rmac_notes_runtime::FolderSelection::Trash => "Recently Deleted".into(),
                rmac_notes_runtime::FolderSelection::Folder(folder_id) => self
                    .session
                    .folders()
                    .iter()
                    .find(|folder| folder.id == folder_id)
                    .map(|folder| SharedString::from(folder.name.clone()))
                    .unwrap_or_else(|| "Notes".into()),
            };
            let subtitle = match note_count {
                0 => "No notes".to_string(),
                1 => "1 note".to_string(),
                count => format!("{count} notes"),
            };
            (title, subtitle.into())
        };
        div()
            .w(px(LIST_WIDTH))
            .h_full()
            .flex_shrink_0()
            .v_flex()
            .bg(list_fill())
            .child(self.render_list_toolbar(title, subtitle, cx))
            .child(
                div()
                    .id("notes-scroll")
                    .role(Role::List)
                    .aria_label(if search_active { "Results" } else { "Notes" })
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .pt(px(LIST_TOP_PADDING))
                    .pb(px(8.0))
                    .children(items),
            )
            .when(
                search_active && self.search.results_truncated(),
                |element| {
                    element.child(
                        div()
                            .px_3()
                            .py_1()
                            .text_size(rmac_ui::text_px(10.0))
                            .text_color(mac::text_tertiary())
                            .child("Showing the first 500 results"),
                    )
                },
            )
    }
}
