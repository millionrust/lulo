use super::*;

impl FinderView {
    pub(in crate::view) fn build_context_menu(
        pos: Point<Pixels>,
        has_selection: bool,
        can_open_with: bool,
        can_paste: bool,
        trash_view: bool,
        undo_label: Option<String>,
    ) -> rmac_ui::ContextMenu {
        let mut m = rmac_ui::ContextMenu::new(pos);
        if let Some(label) = undo_label {
            m = m
                .command_item(label, rmac_ui::shortcuts::UNDO, Box::new(UndoOperation))
                .separator();
        }
        if trash_view {
            if has_selection {
                m = m
                    .item("Restore", Box::new(RestoreItems))
                    .separator()
                    .danger_command_item(
                        "Delete Permanently…",
                        rmac_ui::shortcuts::DELETE_PERMANENT,
                        Box::new(DeletePermanently),
                    );
            }
            return m;
        }
        if has_selection {
            m = m.command_item(
                "Open",
                rmac_ui::shortcuts::OPEN_SELECTION,
                Box::new(OpenItems),
            );
            if can_open_with {
                m = m.item("Open With…", Box::new(OpenWith));
            }
            m = m
                .command_item("Rename", rmac_ui::shortcuts::ENTER, Box::new(RenameItem))
                .command_item(
                    "Duplicate",
                    rmac_ui::shortcuts::DUPLICATE,
                    Box::new(Duplicate),
                )
                .separator()
                .command_item("Copy", rmac_ui::shortcuts::COPY, Box::new(CopyItems))
                .command_item("Cut", rmac_ui::shortcuts::CUT, Box::new(CutItems));
        }
        if can_paste {
            m = m.command_item(
                "Paste Item",
                rmac_ui::shortcuts::PASTE,
                Box::new(PasteItems),
            );
        }
        m = m.separator().command_item(
            "New Folder",
            rmac_ui::shortcuts::NEW_FOLDER,
            Box::new(NewFolder),
        );
        if has_selection {
            m = m
                .separator()
                .command_item(
                    "Move to Trash",
                    rmac_ui::shortcuts::DELETE,
                    Box::new(MoveToTrash),
                )
                .danger_item("Delete Immediately", Box::new(DeleteItem));
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
                    .rounded(px(5.0))
                    .when(active, |el: Stateful<Div>| el.bg(rmac_ui::mac::raised()))
                    .when(!active, |el: Stateful<Div>| {
                        el.hover(|h| h.bg(rmac_ui::mac::hover()))
                    })
                    .child(
                        div()
                            .id(SharedString::from(format!("tabname-{i}")))
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(label())
                            .child(name)
                            .on_click(cx.listener(move |this, _, _, cx| this.select_tab(i, cx))),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("tabclose-{i}")))
                            .w(px(18.0))
                            .h(px(18.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(3.0))
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(secondary())
                            .hover(|h| h.bg(rmac_ui::mac::control_fill_hover()))
                            .child("×")
                            .on_click(cx.listener(move |this, _, _, cx| this.close_tab(i, cx))),
                    ),
            );
        }
        bar.child(div().flex_1()).child(
            div()
                .id("newtab")
                .w(px(26.0))
                .h(px(26.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.0))
                .text_size(rmac_ui::text_px(16.0))
                .text_color(secondary())
                .hover(|h| h.bg(rmac_ui::mac::hover()))
                .child("+")
                .on_click(cx.listener(|this, _, _, cx| this.new_tab(cx))),
        )
    }
}
