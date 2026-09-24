use super::*;

impl FinderView {
    pub(in crate::view) fn build_sort_menu(
        pos: Point<Pixels>,
        key: SortKey,
    ) -> rmac_ui::ContextMenu {
        let check = |candidate| {
            if key == candidate {
                rmac_ui::MenuCheck::On
            } else {
                rmac_ui::MenuCheck::None
            }
        };
        rmac_ui::ContextMenu::new(pos)
            .header("Sort By")
            .checked_item("Name", check(SortKey::Name), Box::new(SortByName))
            .checked_item("Date Modified", check(SortKey::Date), Box::new(SortByDate))
            .checked_item("Size", check(SortKey::Size), Box::new(SortBySize))
            .checked_item("Kind", check(SortKey::Kind), Box::new(SortByKind))
    }

    /// `compress_label` is Finder's "Compress “x”" / "Compress N Items",
    /// present exactly when items are selected.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::view) fn build_context_menu(
        pos: Point<Pixels>,
        compress_label: Option<String>,
        can_open_with: bool,
        can_paste: bool,
        trash_view: bool,
        applications_view: bool,
        undo_label: Option<String>,
        file_words: rmac_locale::FileVocabulary,
    ) -> rmac_ui::ContextMenu {
        let has_selection = compress_label.is_some();
        let mut m = rmac_ui::ContextMenu::new(pos);
        if let Some(label) = undo_label {
            m = m
                .command_item(label, rmac_ui::shortcuts::UNDO, Box::new(UndoOperation))
                .separator();
        }
        if trash_view {
            if has_selection {
                m = m
                    .item("Put Back", Box::new(RestoreItems))
                    .separator()
                    .danger_command_item(
                        "Delete Immediately…",
                        rmac_ui::shortcuts::DELETE_PERMANENT,
                        Box::new(DeletePermanently),
                    );
            }
            return m;
        }
        if applications_view {
            if has_selection {
                m = m
                    .command_item(
                        "Open",
                        rmac_ui::shortcuts::OPEN_SELECTION,
                        Box::new(OpenItems),
                    )
                    .separator()
                    .command_item("Get Info", rmac_ui::shortcuts::INFO, Box::new(GetInfo));
                return m;
            }
            return m
                .item("View as Icons", Box::new(ViewAsIcons))
                .item("View as List", Box::new(ViewAsList))
                .item("View as Columns", Box::new(ViewAsColumns))
                .item("View as Gallery", Box::new(ViewAsGallery))
                .separator()
                .item("Sort by Name", Box::new(SortByName))
                .item("Sort by Date Modified", Box::new(SortByDate))
                .item("Sort by Size", Box::new(SortBySize))
                .item("Sort by Kind", Box::new(SortByKind))
                .separator()
                .command_item(
                    "Select All",
                    rmac_ui::shortcuts::SELECT_ALL,
                    Box::new(SelectAll),
                );
        }
        if has_selection {
            let move_to_bin = format!("Move to {}", file_words.bin());
            m = m.command_item(
                "Open",
                rmac_ui::shortcuts::OPEN_SELECTION,
                Box::new(OpenItems),
            );
            if can_open_with {
                m = m.item("Open With…", Box::new(OpenWith));
            }
            m = m
                .separator()
                .command_item(
                    move_to_bin,
                    rmac_ui::shortcuts::DELETE,
                    Box::new(MoveToTrash),
                )
                .command_item("Get Info", rmac_ui::shortcuts::INFO, Box::new(GetInfo))
                .command_item("Rename", rmac_ui::shortcuts::ENTER, Box::new(RenameItem));
            if let Some(label) = compress_label {
                m = m.item(label, Box::new(Compress));
            }
            m = m
                .command_item(
                    "Duplicate",
                    rmac_ui::shortcuts::DUPLICATE,
                    Box::new(Duplicate),
                )
                .command_item("Quick Look", rmac_ui::shortcuts::SPACE, Box::new(QuickLook))
                .separator()
                .command_item("Copy", rmac_ui::shortcuts::COPY, Box::new(CopyItems));
        } else {
            m = m.command_item(
                "New Folder",
                rmac_ui::shortcuts::NEW_FOLDER,
                Box::new(NewFolder),
            );
            if can_paste {
                m = m.command_item(
                    "Paste Item",
                    rmac_ui::shortcuts::PASTE,
                    Box::new(PasteItems),
                );
            }
            m = m
                .separator()
                .item("View as Icons", Box::new(ViewAsIcons))
                .item("View as List", Box::new(ViewAsList))
                .item("View as Columns", Box::new(ViewAsColumns))
                .item("View as Gallery", Box::new(ViewAsGallery))
                .separator()
                .item("Sort by Name", Box::new(SortByName))
                .item("Sort by Date Modified", Box::new(SortByDate))
                .item("Sort by Size", Box::new(SortBySize))
                .item("Sort by Kind", Box::new(SortByKind))
                .separator()
                .command_item(
                    "Select All",
                    rmac_ui::shortcuts::SELECT_ALL,
                    Box::new(SelectAll),
                );
        }
        m
    }

    // ---- list ----

    pub(in crate::view) fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut bar = div()
            .h(px(30.0))
            .flex_none()
            .flex()
            .items_center()
            .px_2()
            .gap_1()
            .bg(rmac_ui::mac::chrome())
            .border_b_1()
            .border_color(sep());
        for (i, tab) in self.tabs.iter().enumerate() {
            let active = i == self.active;
            let name = tab
                .cwd
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Macintosh HD".into());
            bar = bar.child(
                div()
                    .id(SharedString::from(format!("tab-{i}")))
                    .flex()
                    .items_center()
                    .gap_1()
                    .h(px(22.0))
                    .px_2()
                    .rounded(px(rmac_ui::mac::radius_menu_item()))
                    .when(active, |el: Stateful<Div>| el.bg(rmac_ui::mac::raised()))
                    .when(!active, |el: Stateful<Div>| {
                        el.hover(|h| h.bg(rmac_ui::mac::hover()))
                    })
                    .child(
                        Button::new(SharedString::from(format!("tabname-{i}")), name)
                            .ghost()
                            .xsmall()
                            .selected(active)
                            .on_click(cx.listener(move |this, _, _, cx| this.select_tab(i, cx))),
                    )
                    .child(
                        Button::new(SharedString::from(format!("tabclose-{i}")), "")
                            .icon(Icon::new(IconName::Close).text_color(rmac_ui::mac::text()))
                            .ghost()
                            .xsmall()
                            .tooltip("Close Tab")
                            .on_click(cx.listener(move |this, _, _, cx| this.close_tab(i, cx))),
                    ),
            );
        }
        bar.child(div().flex_1()).child(
            Button::new("newtab", "")
                .icon(Icon::new(IconName::Plus).text_color(rmac_ui::mac::text()))
                .ghost()
                .xsmall()
                .tooltip("New Tab")
                .on_click(cx.listener(|this, _, _, cx| this.new_tab(cx))),
        )
    }
}
