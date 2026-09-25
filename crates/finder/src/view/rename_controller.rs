use super::*;

impl FinderView {
    /// File ▸ Rename and Return: rename the one selected item in place, as
    /// Finder does. The global menu cannot grey the item out per selection,
    /// so an unusable request says why instead of doing nothing.
    pub(super) fn rename_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(reason) = rename_unavailable_reason(
            self.selection_count(),
            self.trash_view,
            self.applications_view,
            self.file_words,
        ) {
            self.operation_notice = Some(reason.into());
            cx.notify();
            return;
        }
        self.rename_start(window, cx);
    }

    pub(super) fn rename_start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.block_mutation_during_transfer(cx) {
            return;
        }
        let Some(entry) = self.selected_entry() else {
            return;
        };
        let path = entry.path.clone();
        let name = entry.name.to_string();
        let selection = rename_selection(&name, entry.is_dir);
        let input = cx.new(|cx| InputState::new(window, cx).default_value(name));
        cx.subscribe(&input, |this, _input, event: &InputEvent, cx| match event {
            InputEvent::PressEnter { .. } => this.rename_commit(cx),
            InputEvent::Blur => this.renaming = None,
            _ => {}
        })
        .detach();
        let focus = input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        self.renaming = Some((path, input));
        cx.notify();
        // Select on the field itself once it exists, as Finder does: the
        // whole name of a folder ("untitled folder"), or only the base name
        // of a file ("report" in "report.txt"), so typing replaces it. A
        // dispatched SelectAll would reach Files' own Select All (every
        // file) instead of the field.
        let field = input.clone();
        window.on_next_frame(move |window, cx| {
            window.focus(&focus, cx);
            field.update(cx, |state, cx| state.set_selected_range(selection, cx));
        });
    }

    fn rename_commit(&mut self, cx: &mut Context<Self>) {
        let Some((path, input)) = self.renaming.take() else {
            return;
        };
        let new_name = input.read(cx).value().to_string();
        self.rename_path_to(&path, &new_name, cx);
        self.reload(cx);
    }

    /// Rename `path` to `new_name` in its own folder, reporting a failure or
    /// a name clash visibly. Returns the new path when the item was renamed.
    fn rename_path_to(
        &mut self,
        path: &Path,
        new_name: &str,
        cx: &mut Context<Self>,
    ) -> Option<PathBuf> {
        let entry = entry_for(path)?;
        let new_name = new_name.trim();
        if new_name.is_empty() || new_name == entry.name.as_ref() {
            return None;
        }
        let destination = entry
            .path
            .parent()
            .unwrap_or(self.cwd.as_path())
            .join(new_name);
        if destination.exists() {
            self.record_operation_failures(
                vec![file_ops::Failure::message(
                    file_ops::Operation::Rename,
                    &entry.path,
                    Some(&destination),
                    "an item with that name already exists",
                )],
                cx,
            );
            return None;
        }
        match file_ops::rename(&file_ops::RealFileSystem, &entry.path, &destination) {
            Ok(()) => Some(destination),
            Err(failure) => {
                self.record_operation_failures(vec![failure], cx);
                None
            }
        }
    }

    /// Get Info's editable Name & Extension field, as in Finder's Info
    /// window: Return renames the item and the panel follows it.
    pub(super) fn info_name_field(
        &mut self,
        entry: &Entry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(entry.name.to_string()));
        cx.subscribe(&input, |this, _input, event: &InputEvent, cx| {
            if let InputEvent::PressEnter { .. } = event {
                this.info_rename_commit(cx);
            }
        })
        .detach();
        self.info_name = Some((entry.path.clone(), input));
    }

    fn info_rename_commit(&mut self, cx: &mut Context<Self>) {
        let Some((path, input)) = self.info_name.clone() else {
            return;
        };
        if self.info.as_ref().map(|entry| &entry.path) != Some(&path) {
            return;
        }
        let new_name = input.read(cx).value().to_string();
        if let Some(destination) = self.rename_path_to(&path, &new_name, cx) {
            if let Some(entry) = entry_for(&destination) {
                self.info_details = file_info(&entry);
                self.info = Some(entry);
            }
            self.info_name = Some((destination, input));
        }
        self.reload(cx);
    }
}

/// Why File ▸ Rename cannot run for this selection, if it cannot.
/// The part of a name Rename selects: all of a folder's name, and a file's
/// name up to its last extension (a leading dot is part of the name).
fn rename_selection(name: &str, is_dir: bool) -> std::ops::Range<usize> {
    match name.rfind('.') {
        Some(dot) if dot > 0 && !is_dir => 0..dot,
        _ => 0..name.len(),
    }
}

fn rename_unavailable_reason(
    selection_count: usize,
    trash_view: bool,
    applications_view: bool,
    file_words: rmac_locale::FileVocabulary,
) -> Option<String> {
    if trash_view {
        Some(format!(
            "Put items back from the {} before renaming them",
            file_words.bin()
        ))
    } else if applications_view {
        Some("Applications cannot be renamed from Files".to_owned())
    } else if selection_count != 1 {
        Some("Select one item to rename it".to_owned())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::rename_unavailable_reason;

    #[test]
    fn rename_selects_a_folder_whole_and_a_file_up_to_its_extension() {
        assert_eq!(rename_selection("untitled folder", true), 0..15);
        assert_eq!(rename_selection("report.txt", false), 0..6);
        assert_eq!(rename_selection("Archive.tar.gz", false), 0..11);
        assert_eq!(rename_selection(".bashrc", false), 0..7);
        assert_eq!(rename_selection("Photos.library", true), 0..14);
        assert_eq!(rename_selection("README", false), 0..6);
    }

    #[test]
    fn rename_needs_exactly_one_item_in_an_ordinary_folder() {
        let words = rmac_locale::FileVocabulary::for_locale("en_US.UTF-8");
        assert_eq!(rename_unavailable_reason(1, false, false, words), None);
        assert!(rename_unavailable_reason(0, false, false, words).is_some());
        assert!(rename_unavailable_reason(2, false, false, words).is_some());
        assert!(rename_unavailable_reason(1, true, false, words).is_some());
        assert!(rename_unavailable_reason(1, false, true, words).is_some());
    }
}
