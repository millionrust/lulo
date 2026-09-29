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
        sort_key: SortKey,
        compress_label: Option<String>,
        can_open_with: bool,
        can_paste: bool,
        trash_view: bool,
        applications_view: bool,
        undo_label: Option<String>,
        tag_checks: [rmac_ui::MenuCheck; 7],
        file_words: rmac_locale::FileVocabulary,
    ) -> rmac_ui::ContextMenu {
        let has_selection = compress_label.is_some();
        let colors = rmac_ui::theme::current().colors;
        let swatches = [
            colors.system_red.hsla(),
            colors.system_orange.hsla(),
            colors.system_yellow.hsla(),
            colors.system_green.hsla(),
            colors.system_blue.hsla(),
            colors.system_purple.hsla(),
            colors.system_gray.hsla(),
        ];
        let swatch = |index: usize| swatches[index];
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
                .submenu("View", Self::build_view_submenu(pos))
                .submenu("Sort By", Self::build_sort_submenu(pos, sort_key))
                .item("Show View Options", Box::new(ShowViewOptions))
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
            // The picker loads type handlers asynchronously and owns the
            // default-app controls, so this row opens it instead of building
            // a submenu from data that is not available to this menu builder.
            if can_open_with {
                m = m.item("Open With", Box::new(OpenWith));
            }
            m = m
                .separator()
                .command_item(
                    move_to_bin,
                    rmac_ui::shortcuts::DELETE,
                    Box::new(MoveToTrash),
                )
                .separator()
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
                .item("Make Alias", Box::new(MakeAlias))
                .command_item("Quick Look", rmac_ui::shortcuts::SPACE, Box::new(QuickLook))
                .separator()
                .command_item("Copy", rmac_ui::shortcuts::COPY, Box::new(CopyItems))
                .separator()
                .header("Tags")
                .checked_item_with_swatch("Red", tag_checks[0], swatch(0), Box::new(TagRed))
                .checked_item_with_swatch("Orange", tag_checks[1], swatch(1), Box::new(TagOrange))
                .checked_item_with_swatch("Yellow", tag_checks[2], swatch(2), Box::new(TagYellow))
                .checked_item_with_swatch("Green", tag_checks[3], swatch(3), Box::new(TagGreen))
                .checked_item_with_swatch("Blue", tag_checks[4], swatch(4), Box::new(TagBlue))
                .checked_item_with_swatch("Purple", tag_checks[5], swatch(5), Box::new(TagPurple))
                .checked_item_with_swatch("Gray", tag_checks[6], swatch(6), Box::new(TagGray));
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
                .command_item("Get Info", rmac_ui::shortcuts::INFO, Box::new(GetInfo))
                .separator()
                .submenu("View", Self::build_view_submenu(pos))
                .submenu("Sort By", Self::build_sort_submenu(pos, sort_key))
                .item("Show View Options", Box::new(ShowViewOptions))
                .separator()
                .command_item(
                    "Select All",
                    rmac_ui::shortcuts::SELECT_ALL,
                    Box::new(SelectAll),
                );
        }
        m
    }

    fn build_view_submenu(pos: Point<Pixels>) -> rmac_ui::ContextMenu {
        rmac_ui::ContextMenu::new(pos)
            .item("Icons", Box::new(ViewAsIcons))
            .item("List", Box::new(ViewAsList))
            .item("Columns", Box::new(ViewAsColumns))
            .item("Gallery", Box::new(ViewAsGallery))
    }

    fn build_sort_submenu(pos: Point<Pixels>, key: SortKey) -> rmac_ui::ContextMenu {
        let check = |candidate| {
            if key == candidate {
                rmac_ui::MenuCheck::On
            } else {
                rmac_ui::MenuCheck::None
            }
        };
        rmac_ui::ContextMenu::new(pos)
            .checked_item("Name", check(SortKey::Name), Box::new(SortByName))
            .checked_item("Date Modified", check(SortKey::Date), Box::new(SortByDate))
            .checked_item("Size", check(SortKey::Size), Box::new(SortBySize))
            .checked_item("Kind", check(SortKey::Kind), Box::new(SortByKind))
    }

    // ---- list ----

    pub(in crate::view) fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut bar = div()
            .id("finder-tabs")
            .role(Role::TabList)
            .aria_label("Finder window tabs")
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
                    .role(Role::Tab)
                    .aria_label(name.clone())
                    .aria_selected(active)
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
