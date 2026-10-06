use super::*;

impl FinderView {
    pub(in crate::view) fn build_title_path_menu(
        &self,
        pos: Point<Pixels>,
    ) -> rmac_ui::ContextMenu {
        let mut menu = rmac_ui::ContextMenu::new(pos);
        for ancestor in self.cwd.ancestors() {
            let name = ancestor
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| rmac_finder::places::root_volume_name().to_owned());
            menu = menu.item(
                name,
                Box::new(GoToTitlePathAction {
                    path: ancestor.to_path_buf(),
                }),
            );
        }
        menu
    }

    pub(in crate::view) fn build_sidebar_menu(
        pos: Point<Pixels>,
        removable: bool,
        applications: bool,
    ) -> rmac_ui::ContextMenu {
        let mut menu = if applications {
            rmac_ui::ContextMenu::new(pos).item("Open", Box::new(GoApplications))
        } else {
            rmac_ui::ContextMenu::new(pos)
                .item("Open in New Window", Box::new(SidebarOpenWindow))
                .item("Open in New Tab", Box::new(SidebarOpenTab))
                .item("Show in Enclosing Folder", Box::new(SidebarShowEnclosing))
                .separator()
                .item("Get Info", Box::new(SidebarGetInfo))
                .item("Rename", Box::new(SidebarRename))
                .item("Add to Dock", Box::new(SidebarAddToDock))
        };
        if removable {
            menu = menu
                .separator()
                .item("Remove from Sidebar", Box::new(SidebarRemove));
        }
        menu
    }

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
            .checked_item("Kind", check(SortKey::Kind), Box::new(SortByKind))
            .checked_item(
                "Date Last Opened",
                check(SortKey::LastOpened),
                Box::new(SortByLastOpened),
            )
            .checked_item("Date Added", check(SortKey::Added), Box::new(SortByAdded))
            .checked_item("Date Modified", check(SortKey::Date), Box::new(SortByDate))
            .checked_item("Size", check(SortKey::Size), Box::new(SortBySize))
            .checked_item("Tags", check(SortKey::Tags), Box::new(SortByTags))
            .checked_item(
                "Date Created",
                check(SortKey::Created),
                Box::new(SortByCreated),
            )
    }

    /// `compress_label` is Finder's "Compress “x”" / "Compress N Items",
    /// present exactly when items are selected.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::view) fn build_context_menu(
        pos: Point<Pixels>,
        sort_key: SortKey,
        compress_label: Option<String>,
        copy_pathname_label: String,
        slideshow_label: String,
        selection_count: usize,
        selected_folder: bool,
        can_open_with: bool,
        open_with_association: Option<&rmac_apps::FileAssociation>,
        _can_paste: bool,
        trash_view: bool,
        applications_view: bool,
        icon_view: bool,
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
            if !selected_folder {
                m = m.command_item(
                    "Open",
                    rmac_ui::shortcuts::OPEN_SELECTION,
                    Box::new(OpenItems),
                );
            }
            if selection_count == 1 && selected_folder {
                m = m
                    .item("Open in New Tab", Box::new(OpenSelectionInNewTab))
                    .item("Open in New Window", Box::new(OpenSelectionInNewWindow));
            }
            if selection_count > 1 {
                m = m.item(
                    "New Folder with Selection",
                    Box::new(NewFolderWithSelection),
                );
            }
            if can_open_with {
                let mut submenu = rmac_ui::ContextMenu::new(pos);
                if let Some(association) = open_with_association {
                    for (index, handler) in association.handlers.iter().take(16).enumerate() {
                        submenu = submenu.item(
                            handler.name.clone(),
                            Box::new(OpenWithHandlerAction {
                                index,
                                make_default: false,
                            }),
                        );
                    }
                    submenu = submenu.separator();
                }
                m = m.submenu("Open With", submenu.item("Other…", Box::new(OpenWith)));
                let mut always = rmac_ui::ContextMenu::new(pos);
                if let Some(association) = open_with_association {
                    for (index, handler) in association.handlers.iter().take(16).enumerate() {
                        always = always.item(
                            handler.name.clone(),
                            Box::new(OpenWithHandlerAction {
                                index,
                                make_default: true,
                            }),
                        );
                    }
                    always = always.separator();
                }
                m = m.submenu(
                    "Always Open With",
                    always.item("Other…", Box::new(AlwaysOpenWithOther)),
                );
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
                .item("Show Inspector", Box::new(ShowInspector))
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
                .item("Duplicate Exactly", Box::new(DuplicateExactly))
                .item("Make Alias", Box::new(MakeAlias))
                .command_item("Quick Look", rmac_ui::shortcuts::SPACE, Box::new(QuickLook))
                .item(slideshow_label, Box::new(Slideshow))
                .separator()
                .command_item("Copy", rmac_ui::shortcuts::COPY, Box::new(CopyItems))
                .command_item(
                    copy_pathname_label,
                    rmac_ui::shortcuts::COPY_AS_PATHNAME,
                    Box::new(CopyAsPathname),
                )
                .item("Share…", Box::new(ShareItems))
                .separator()
                .submenu(
                    "label",
                    rmac_ui::ContextMenu::new(pos)
                        .checked_item_with_swatch("Red", tag_checks[0], swatch(0), Box::new(TagRed))
                        .checked_item_with_swatch(
                            "Orange",
                            tag_checks[1],
                            swatch(1),
                            Box::new(TagOrange),
                        )
                        .checked_item_with_swatch(
                            "Yellow",
                            tag_checks[2],
                            swatch(2),
                            Box::new(TagYellow),
                        )
                        .checked_item_with_swatch(
                            "Green",
                            tag_checks[3],
                            swatch(3),
                            Box::new(TagGreen),
                        )
                        .checked_item_with_swatch(
                            "Blue",
                            tag_checks[4],
                            swatch(4),
                            Box::new(TagBlue),
                        )
                        .checked_item_with_swatch(
                            "Purple",
                            tag_checks[5],
                            swatch(5),
                            Box::new(TagPurple),
                        )
                        .checked_item_with_swatch(
                            "Gray",
                            tag_checks[6],
                            swatch(6),
                            Box::new(TagGray),
                        ),
                )
                .separator()
                .item("Quick Actions", Box::new(QuickActions));
        } else {
            m = m.command_item(
                "New Folder",
                rmac_ui::shortcuts::NEW_FOLDER,
                Box::new(NewFolder),
            );
            m = m
                .separator()
                .command_item("Get Info", rmac_ui::shortcuts::INFO, Box::new(GetInfo))
                .separator()
                .submenu("View", Self::build_view_submenu(pos))
                .item("Use Groups", Box::new(UseGroups))
                .submenu("Sort By", Self::build_sort_submenu(pos, sort_key));
            // Clean Up / Clean Up By only make sense for Icon view's
            // auto-flowed grid (matches the View menu's own gating in
            // presentation.rs); List/Column/Gallery backgrounds never show
            // them on the Mac.
            if icon_view {
                m = m
                    .item("Clean Up", Box::new(CleanUp))
                    .submenu("Clean Up By", Self::build_clean_up_by_submenu(pos));
            }
            m = m
                .item("Show View Options", Box::new(ShowViewOptions))
                .separator()
                .item("Import from iPhone", Box::new(ImportFromIphone));
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
            .checked_item("Kind", check(SortKey::Kind), Box::new(SortByKind))
            .checked_item(
                "Date Last Opened",
                check(SortKey::LastOpened),
                Box::new(SortByLastOpened),
            )
            .checked_item("Date Added", check(SortKey::Added), Box::new(SortByAdded))
            .checked_item("Date Modified", check(SortKey::Date), Box::new(SortByDate))
            .checked_item("Size", check(SortKey::Size), Box::new(SortBySize))
            .checked_item("Tags", check(SortKey::Tags), Box::new(SortByTags))
            .checked_item(
                "Date Created",
                check(SortKey::Created),
                Box::new(SortByCreated),
            )
    }

    /// File ▸ Open With / Always Open With, triggered from the menu bar
    /// rather than a right click: the same handler list and "Other…" entry
    /// the context menu's submenu already offers (`build_context_menu`),
    /// as a standalone popup anchored under the toolbar instead of at a
    /// mouse position.
    pub(in crate::view) fn build_open_with_menu(
        pos: Point<Pixels>,
        always_default: bool,
        association: Option<&rmac_apps::FileAssociation>,
    ) -> rmac_ui::ContextMenu {
        let mut menu = rmac_ui::ContextMenu::new(pos).header(if always_default {
            "Always Open With"
        } else {
            "Open With"
        });
        if let Some(association) = association {
            for (index, handler) in association.handlers.iter().take(16).enumerate() {
                menu = menu.item(
                    handler.name.clone(),
                    Box::new(OpenWithHandlerAction {
                        index,
                        make_default: always_default,
                    }),
                );
            }
            menu = menu.separator();
        }
        let other: Box<dyn gpui::Action> = if always_default {
            Box::new(AlwaysOpenWithOther)
        } else {
            Box::new(OpenWith)
        };
        menu.item("Other…", other)
    }

    /// Go ▸ Recent Folders, as a standalone popup (the same reason as
    /// `build_open_with_menu`: the menu-bar protocol has no dynamic-submenu
    /// support, so the dynamic list renders as an in-app popup instead).
    pub(in crate::view) fn build_recent_folders_menu(
        pos: Point<Pixels>,
        recent: &[PathBuf],
    ) -> rmac_ui::ContextMenu {
        let mut menu = rmac_ui::ContextMenu::new(pos).header("Recent Folders");
        if recent.is_empty() {
            return menu.disabled_item("No Recent Folders", Box::new(ShowRecentFolders));
        }
        for (index, path) in recent.iter().enumerate() {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            menu = menu.item(name, Box::new(OpenRecentFolderAction { index }));
        }
        menu.separator()
            .item("Clear Menu", Box::new(ClearRecentFolders))
    }

    fn build_clean_up_by_submenu(pos: Point<Pixels>) -> rmac_ui::ContextMenu {
        rmac_ui::ContextMenu::new(pos)
            .item("Name", Box::new(CleanUpByName))
            .item("Kind", Box::new(CleanUpByKind))
            .item("Date Created", Box::new(CleanUpByCreated))
            .item("Date Modified", Box::new(CleanUpByDate))
            .item("Size", Box::new(CleanUpBySize))
            .item("Tags", Box::new(CleanUpByTags))
    }

    // ---- list ----

    pub(in crate::view) fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut bar = div()
            .id("finder-tabs")
            .role(Role::TabList)
            .aria_label("Finder window tabs")
            .flex_none()
            .flex()
            .items_center()
            .px_2()
            .gap_1()
            .bg(rmac_ui::mac::chrome())
            .border_b_1()
            .border_color(sep());
        // View ▸ Show All Tabs wraps every tab onto as many rows as it
        // takes, so none are scrolled out of sight; otherwise the strip is
        // a single fixed-height row, same as the Mac's default.
        bar = if self.show_all_tabs {
            bar.flex_wrap().py_1()
        } else {
            bar.h(px(30.0))
        };
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
                    .when(self.tabs.len() > 1, |el: Stateful<Div>| {
                        el.child(
                            Button::new(SharedString::from(format!("tabclose-{i}")), "")
                                .icon(Icon::new(IconName::Close).text_color(rmac_ui::mac::text()))
                                .ghost()
                                .xsmall()
                                .tooltip("Close Tab")
                                .on_click(cx.listener(move |this, _, _, cx| this.close_tab(i, cx))),
                        )
                    }),
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
