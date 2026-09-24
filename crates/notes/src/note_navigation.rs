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
        let mut sidebar = div()
            .id("notes-folders")
            .role(Role::List)
            .aria_label("Folders")
            .w(px(FOLDERS_W))
            .h_full()
            .flex_shrink_0()
            .v_flex()
            .pt_3()
            .px_2()
            .bg(mac::sidebar())
            .border_r_1()
            .border_color(mac::separator())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_2()
                    .pb_1()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(11.0))
                            .font_weight(mac::BOLD)
                            .text_color(mac::text_secondary())
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
                                    .disabled(!self.is_interactive_ready())
                                    .tooltip("New Folder")
                                    .on_click(cx.listener(|this, _, _, cx| this.create_folder(cx))),
                            ),
                    ),
            )
            .child(folder_row(
                "all-notes",
                "All Notes",
                IconName::Folder,
                all_count,
                current == FolderSelection::All,
                cx.listener(|this, _, window, cx| {
                    this.select_folder(FolderSelection::All, window, cx)
                }),
            ));

        for folder in self.session.folders() {
            let folder_id = folder.id;
            sidebar = sidebar.child(folder_row(
                ("folder", folder_id.get()),
                folder.name.clone(),
                IconName::Folder,
                self.session.folder_count(folder_id),
                current == FolderSelection::Folder(folder_id),
                cx.listener(move |this, _, window, cx| {
                    this.select_folder(FolderSelection::Folder(folder_id), window, cx)
                }),
            ));
        }

        sidebar.child(div().mt_2().child(folder_row(
            "trash-notes",
            "Recently Deleted",
            IconName::Delete,
            trash_count,
            current == FolderSelection::Trash,
            cx.listener(|this, _, window, cx| {
                this.select_folder(FolderSelection::Trash, window, cx)
            }),
        )))
    }

    pub(super) fn render_note_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let search_active = !self.search_query.read(cx).value().trim().is_empty();
        let in_trash =
            self.session.folder_selection() == rmac_notes_runtime::FolderSelection::Trash;
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
        for (note, search_hit) in notes {
            if sectioned {
                let section: SharedString = if note.pinned {
                    "Pinned".into()
                } else if sort_by_created {
                    date_section(note.created_unix_ms)
                } else {
                    date_section(note.modified_unix_ms)
                };
                if current_section.as_ref() != Some(&section) {
                    items.push(
                        div()
                            .px(px(20.0))
                            .pt_3()
                            .pb_1()
                            .text_size(rmac_ui::text_px(13.0))
                            .font_weight(mac::BOLD)
                            .text_color(mac::text())
                            .child(section.clone())
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
            items.push(
                div()
                    .id(("note", note.id.get()))
                    .role(Role::ListItem)
                    .aria_label(accessible_label)
                    .aria_selected(is_selected)
                    .mx(px(10.0))
                    .px(px(10.0))
                    .py(px(8.0))
                    .rounded(px(rmac_ui::mac::radius_control()))
                    // Notes selects in its dimmed yellow (S) and separates
                    // the other rows with inset hairlines.
                    .when(is_selected, |element: Stateful<Div>| {
                        element.bg(mac::notes_accent().opacity(0.62))
                    })
                    .when(!is_selected, |element: Stateful<Div>| {
                        element
                            .border_b_1()
                            .border_color(mac::separator())
                            .hover(|hover| hover.bg(mac::hover()))
                    })
                    .child(
                        div()
                            .v_flex()
                            .gap_0p5()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .when(note.pinned, |element| {
                                        element.child(
                                            Icon::new(IconName::Star)
                                                .text_color(if is_selected {
                                                    mac::on_accent()
                                                } else {
                                                    mac::notes_accent()
                                                })
                                                .with_size(Size::XSmall),
                                        )
                                    })
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_size(rmac_ui::text_px(13.0))
                                            .font_weight(mac::BOLD)
                                            .text_color(if is_selected {
                                                mac::on_accent()
                                            } else {
                                                mac::text()
                                            })
                                            .truncate()
                                            .child(styled_search_fragment(
                                                title_fragment,
                                                false,
                                                true,
                                            )),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1p5()
                                    .child(
                                        div()
                                            .text_size(rmac_ui::text_px(12.0))
                                            .font_weight(mac::MEDIUM)
                                            .text_color(if is_selected {
                                                mac::on_accent()
                                            } else {
                                                mac::text()
                                            })
                                            .child(date_label(note.modified_unix_ms)),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_size(rmac_ui::text_px(12.0))
                                            .text_color(if is_selected {
                                                mac::on_accent().opacity(0.82)
                                            } else {
                                                mac::text_secondary()
                                            })
                                            .truncate()
                                            .child(styled_search_fragment(
                                                body_fragment,
                                                true,
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
                                        .pt_0p5()
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
                    .on_click(cx.listener(move |this, _, window, cx| {
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
        div()
            .w(px(LIST_W))
            .h_full()
            .flex_shrink_0()
            .v_flex()
            .bg(mac::list())
            .border_r_1()
            .border_color(mac::separator())
            .child(
                div()
                    .h(px(42.0))
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_4()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(13.0))
                            .font_weight(mac::SEMIBOLD)
                            .text_color(mac::text_secondary())
                            .child(format!(
                                "{note_count} {}",
                                if search_active {
                                    if note_count == 1 {
                                        "Result"
                                    } else {
                                        "Results"
                                    }
                                } else if note_count == 1 {
                                    "Note"
                                } else {
                                    "Notes"
                                }
                            )),
                    )
                    .when(!search_active && in_trash && note_count != 0, |element| {
                        element.child(
                            Button::new("empty-trash", "Empty")
                                .destructive()
                                .xsmall()
                                .disabled(!self.is_interactive_ready())
                                .tooltip("Empty Recently Deleted…")
                                .on_click(cx.listener(|this, _, _, cx| this.begin_empty_trash(cx))),
                        )
                    }),
            )
            .child(
                div()
                    .id("notes-scroll")
                    .role(Role::List)
                    .aria_label(if search_active { "Results" } else { "Notes" })
                    .flex_1()
                    .overflow_y_scroll()
                    .py_1()
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
