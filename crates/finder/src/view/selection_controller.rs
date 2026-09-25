use super::*;

impl FinderView {
    pub(super) fn select_single(&mut self, index: usize) {
        self.column_selection = None;
        self.selected.clear();
        self.selected.insert(index);
        self.anchor = Some(index);
    }

    pub(super) fn handle_click(&mut self, index: usize, command: bool, shift: bool) {
        self.column_selection = None;
        if command {
            if !self.selected.remove(&index) {
                self.selected.insert(index);
            }
            self.anchor = Some(index);
        } else if shift {
            if let Some(anchor) = self.anchor {
                let (low, high) = if anchor <= index {
                    (anchor, index)
                } else {
                    (index, anchor)
                };
                self.selected.clear();
                for selected in low..=high {
                    self.selected.insert(selected);
                }
            } else {
                self.select_single(index);
            }
        } else {
            self.select_single(index);
        }
    }

    pub(super) fn selected_paths(&self) -> Vec<PathBuf> {
        if self.view == ViewMode::Column && !self.applications_view && !self.trash_view {
            return self
                .column_selection
                .as_ref()
                .map(|entry| vec![entry.path.clone()])
                .unwrap_or_default();
        }
        self.selected
            .iter()
            .filter_map(|&index| self.entries.get(index))
            .map(|entry| entry.path.clone())
            .collect()
    }

    pub(super) fn selected_entry(&self) -> Option<&Entry> {
        if self.view == ViewMode::Column && !self.applications_view && !self.trash_view {
            return self.column_selection.as_ref();
        }
        self.selected
            .iter()
            .next()
            .and_then(|index| self.entries.get(*index))
    }

    pub(super) fn selection_count(&self) -> usize {
        if self.view == ViewMode::Column && !self.applications_view && !self.trash_view {
            usize::from(self.column_selection.is_some())
        } else {
            self.selected.len()
        }
    }

    /// Publish `self.clipboard` on the system clipboard (Copy, Cut, and
    /// the unfinished half of a move). A failure is shown; the items stay
    /// in this window's own clipboard, so Paste here still works.
    pub(super) fn write_clip_text(&self, cx: &mut Context<Self>) {
        let pending = pasteboard::write_file_list(self.clipboard.clone(), self.clip_cut);
        // macOS also offers the paths as text for text fields; on Linux
        // wl-copy already offers the URI list as text, and a second writer
        // would replace the file list.
        #[cfg(not(target_os = "linux"))]
        {
            let text = self
                .clipboard
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join("\n");
            if !text.is_empty() {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
            }
        }
        let has_files = !self.clipboard.is_empty();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = pending.wait().await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                match result {
                    Ok(()) => this.pasteboard_has_files = has_files,
                    Err(error) => {
                        this.operation_error = Some(
                            format!("The items were not put on the clipboard: {error}").into(),
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Ask whether the system clipboard holds files, so Paste is offered
    /// in a window that did not copy them. Runs when the window becomes
    /// active, never on a timer.
    pub(super) fn refresh_pasteboard_state(&self, cx: &mut Context<Self>) {
        let pending = pasteboard::has_file_list();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            // A failure here is reported by Paste itself, which reads again.
            let has_files = pending.wait().await.unwrap_or(false);
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                if this.pasteboard_has_files != has_files {
                    this.pasteboard_has_files = has_files;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn can_paste(&self) -> bool {
        !self.clipboard.is_empty() || self.pasteboard_has_files
    }

    pub(super) fn copy(&mut self, cx: &mut Context<Self>) {
        if self.applications_view {
            self.operation_error = Some("Applications cannot be copied from this view".into());
            cx.notify();
            return;
        }
        if self.trash_view {
            self.operation_error = Some("Restore items before copying them".into());
            cx.notify();
            return;
        }
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        self.clipboard = paths;
        self.clip_cut = false;
        self.write_clip_text(cx);
    }

    /// ⌥⌘C: put the selection's absolute paths on the clipboard as plain
    /// text, one per line, leaving the file clipboard (and Paste) alone —
    /// the Mac's Copy “x” as Pathname is a text copy, not a file one.
    pub(super) fn copy_as_pathname(&mut self, cx: &mut Context<Self>) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(pathname_clipboard_text(
            &paths,
        )));
    }

    pub(super) fn cut(&mut self, cx: &mut Context<Self>) {
        if self.applications_view {
            self.operation_error = Some("Applications cannot be moved from this view".into());
            cx.notify();
            return;
        }
        if self.trash_view {
            self.operation_error = Some("Use Restore to move an item out of Trash".into());
            cx.notify();
            return;
        }
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        self.clipboard = paths;
        self.clip_cut = true;
        self.write_clip_text(cx);
    }

    /// Paste reads the system clipboard first, so files copied in another
    /// window or another file manager are pasted, then runs the ordinary
    /// transfer (conflict sheet, cancellation, undo journal).
    pub(super) fn paste(&mut self, cx: &mut Context<Self>) {
        self.paste_with_kind(false, cx);
    }

    /// ⌥⌘V, Move Item Here: paste as a move even though the clipboard held
    /// a plain Copy, exactly as the Mac's Edit menu item does — ⌘C an item
    /// elsewhere, then ⌥⌘V it here instead of copying it.
    pub(super) fn move_item_here(&mut self, cx: &mut Context<Self>) {
        self.paste_with_kind(true, cx);
    }

    fn paste_with_kind(&mut self, force_move: bool, cx: &mut Context<Self>) {
        if self.block_mutation_during_transfer(cx) {
            return;
        }
        let pending = pasteboard::read_file_list();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let read = pending.wait().await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                this.paste_from(read, force_move, cx)
            });
        })
        .detach();
    }

    fn paste_from(
        &mut self,
        read: std::result::Result<Option<pasteboard::FileList>, pasteboard::PasteboardError>,
        force_move: bool,
        cx: &mut Context<Self>,
    ) {
        if self.block_mutation_during_transfer(cx) {
            return;
        }
        match read {
            Ok(Some(list)) => {
                self.pasteboard_has_files = true;
                // The Mac pasteboard cannot say "cut"; this window can,
                // when these are the items it cut.
                let cut = list.cut
                    || (self.clip_cut && pasteboard::same_files(&list.paths, &self.clipboard));
                self.clipboard = list.paths;
                self.clip_cut = cut;
            }
            Ok(None) => {
                self.pasteboard_has_files = false;
                // Something other than files was copied since: the system
                // clipboard is the authority, so the old items are not
                // pasted. (macOS keeps this window's own list; its text
                // copy replaces the file URLs.)
                #[cfg(target_os = "linux")]
                {
                    self.clipboard.clear();
                    self.clip_cut = false;
                }
            }
            Err(error) => {
                if self.clipboard.is_empty() {
                    self.operation_error = Some(format!("Nothing was pasted: {error}").into());
                    cx.notify();
                    return;
                }
            }
        }
        #[cfg(not(target_os = "linux"))]
        if self.clipboard.is_empty() {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                self.clipboard = text.lines().map(PathBuf::from).collect();
                self.clip_cut = false;
            }
        }
        // Items that vanished since they were copied are skipped; a
        // symbolic link counts as itself, whether or not its target exists.
        self.clipboard
            .retain(|path| path.is_absolute() && path.symlink_metadata().is_ok());
        if self.clipboard.is_empty() {
            self.clip_cut = false;
            self.operation_notice = Some("There are no files on the clipboard to paste".into());
            cx.notify();
            return;
        }
        let move_it = force_move || self.clip_cut;
        let kind = if move_it {
            file_ops::TransferKind::Move
        } else {
            file_ops::TransferKind::Copy
        };
        let mut tasks = Vec::new();
        for source in self.clipboard.clone() {
            if move_it && source.parent() == Some(self.cwd.as_path()) {
                continue;
            }
            let Some(name) = source.file_name().map(|name| name.to_owned()) else {
                continue;
            };
            tasks.push(file_ops::TransferTask {
                kind: kind.clone(),
                source,
                destination: self.cwd.join(name),
            });
        }
        if tasks.is_empty() {
            if move_it {
                let pasted = std::mem::take(&mut self.clipboard);
                self.clip_cut = false;
                self.clear_pasteboard_after_move(pasted, cx);
                self.operation_notice =
                    Some("The items are already in this folder; nothing was moved".into());
                cx.notify();
            }
            return;
        }
        self.start_transfer_with_conflicts(
            if move_it { "Moving" } else { "Copying" },
            tasks,
            move_it,
            false,
            cx,
        );
    }

    /// After a cut is pasted the clipboard is emptied, as in Nautilus,
    /// unless something else was copied in the meantime.
    pub(super) fn clear_pasteboard_after_move(&self, moved: Vec<PathBuf>, cx: &mut Context<Self>) {
        if moved.is_empty() {
            return;
        }
        let pending = pasteboard::clear_file_list_if(moved);
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = pending.wait().await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                match result {
                    Ok(()) => this.pasteboard_has_files = false,
                    Err(error) => {
                        this.operation_error = Some(
                            format!("The moved items are still on the clipboard: {error}").into(),
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn select_all(&mut self, cx: &mut Context<Self>) {
        let query = if self.search_summary.is_some() {
            String::new()
        } else {
            self.query.read(cx).value().to_lowercase()
        };
        self.selected = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| query.is_empty() || entry.name.to_lowercase().contains(&query))
            .map(|(index, _)| index)
            .collect();
        cx.notify();
    }

    pub(super) fn toggle_hidden(&mut self, cx: &mut Context<Self>) {
        self.show_hidden = !self.show_hidden;
        self.reload(cx);
    }

    pub(super) fn set_sort(&mut self, key: SortKey, cx: &mut Context<Self>) {
        if self.sort_key == key {
            self.sort_asc = !self.sort_asc;
        } else {
            self.sort_key = key;
            self.sort_asc = true;
        }
        sort_entries(&mut self.entries, self.sort_key, self.sort_asc);
        self.search_relevance_order = false;
        self.selected.clear();
        cx.notify();
    }

    /// Select exactly one item in the current listing, as a plain click does,
    /// and give the file view keyboard focus so selection-scoped commands
    /// (Rename, Move to Trash, Quick Look…) act on it.
    pub(super) fn accessible_select(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.entries.len() {
            return;
        }
        self.select_single(index);
        window.focus(&self.focus, cx);
        cx.notify();
    }
}

/// ⌥⌘C, Copy “x” as Pathname: the clipboard text for a selection, one
/// absolute path per line, in selection order.
pub(super) fn pathname_clipboard_text(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Publish one file item to assistive technology: its role, its name, its kind
/// (read after the name, as VoiceOver reads Finder's Kind column), whether
/// it is selected and where it sits in its list, plus the Click and Focus
/// actions a screen reader (or an AT-SPI test) uses to select it. `select`
/// runs the same selection a plain click makes, and works whether or not the
/// item is scrolled into view (GPUI's own Click fallback synthesizes a mouse
/// click at the item's on-screen centre).
#[allow(clippy::too_many_arguments)]
pub(super) fn accessible_item(
    element: Stateful<Div>,
    role: Role,
    entry: &Entry,
    selected: bool,
    position: usize,
    count: usize,
    entity: &Entity<FinderView>,
    select: impl Fn(&mut FinderView, &mut Window, &mut Context<FinderView>) + Clone + 'static,
) -> Stateful<Div> {
    let click_entity = entity.clone();
    let click_select = select.clone();
    let focus_entity = entity.clone();
    rmac_ui::accessibility::with_description(element, entry.kind.clone())
        .role(role)
        .aria_label(entry.name.clone())
        .aria_selected(selected)
        .aria_position_in_set(position + 1)
        .aria_size_of_set(count)
        .on_a11y_action(AccessibleAction::Click, move |_, window, cx| {
            focus_entity_update(&click_entity, window, cx, &click_select);
        })
        .on_a11y_action(AccessibleAction::Focus, move |_, window, cx| {
            focus_entity_update(&focus_entity, window, cx, &select);
        })
}

fn focus_entity_update(
    entity: &Entity<FinderView>,
    window: &mut Window,
    cx: &mut gpui::App,
    select: &impl Fn(&mut FinderView, &mut Window, &mut Context<FinderView>),
) {
    entity.update(cx, |this, cx| select(this, window, cx));
}
