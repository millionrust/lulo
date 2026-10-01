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
        // Return both commits the rename field (its own "Input" keymap
        // context) and, as the plain "Finder" context's Return shortcut for
        // File ▸ Rename, restarts one — both bindings match the same
        // keystroke, and the global one runs first (FILES-43). Restarting
        // here would blow away the field the person was just typing into,
        // so a rename already in progress makes this a no-op and lets the
        // field's own Return commit normally right after.
        if self.renaming.is_some() {
            return;
        }
        if self.block_mutation_during_transfer(cx) {
            return;
        }
        let Some(entry) = self.selected_entry() else {
            return;
        };
        self.begin_rename_path(entry.path.clone(), entry.name.to_string(), entry.is_dir, window, cx);
    }

    pub(super) fn rename_sidebar_path(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.renaming.is_some() || self.block_mutation_during_transfer(cx) {
            return;
        }
        let Some(entry) = entry_for(&path) else { return; };
        self.begin_rename_path(path, entry.name.to_string(), entry.is_dir, window, cx);
    }

    fn begin_rename_path(&mut self, path: PathBuf, name: String, is_dir: bool, window: &mut Window, cx: &mut Context<Self>) {
        let selection = rename_selection(&name, is_dir);
        let input = cx.new(|cx| InputState::new(window, cx).default_value(name));
        cx.subscribe_in(
            &input,
            window,
            |this, _input, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } => this.rename_commit(window, cx),
                InputEvent::Blur => this.renaming = None,
                _ => {}
            },
        )
        .detach();
        let focus = input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        let field = input.clone();
        self.renaming = Some((path, input));
        cx.notify();
        // Select on the field itself once it exists, as Finder does: the
        // whole name of a folder ("untitled folder"), or only the base name
        // of a file ("report" in "report.txt"), so typing replaces it. A
        // dispatched SelectAll would reach Files' own Select All (every
        // file) instead of the field.
        window.on_next_frame(move |window, cx| {
            window.focus(&focus, cx);
            field.update(cx, |state, cx| state.set_selected_range(selection, cx));
        });
    }

    fn rename_commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((path, input)) = self.renaming.take() else {
            return;
        };
        let new_name = input.read(cx).value().to_string();
        // Keep the item selected under its new name, and return focus to
        // the list, as Finder does — reload's own path-based selection
        // would otherwise look for the item under its old (now nonexistent)
        // path and find nothing.
        let scheduled = self.rename_path_to(&path, &new_name, cx);
        self.pending_select = scheduled
            .as_ref()
            .map(|(path, _)| path.clone())
            .or(Some(path.clone()));
        if self.is_removable_favourite(&path) {
            if let Some((destination, completion)) = scheduled.as_ref() {
                let source = path.clone();
                let destination = destination.clone();
                let completion = completion.clone();
                cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
                    if completion.recv().await == Ok(true) {
                        let _ = this.update(cx, |this: &mut FinderView, cx| {
                            if let Some(favourite) = this.favourite_extras.iter_mut().find(|item| item.as_path() == source.as_path()) {
                                *favourite = destination;
                                this.save_and_broadcast_favourites(cx);
                            }
                        });
                    }
                }).detach();
            }
        }
        window.focus(&self.focus, cx);
        if scheduled.is_none() {
            self.reload(cx);
        } else {
            cx.notify();
        }
    }

    /// Tab accepts the edit and advances to the item that followed it before
    /// the list is sorted again under the new name. Finder leaves list focus.
    pub(super) fn rename_next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((path, input)) = self.renaming.take() else {
            return;
        };
        let next_path = (|| {
            let index = self.entries.iter().position(|entry| entry.path == path)?;
            self.entries
                .get((index + 1) % self.entries.len())
                .map(|entry| entry.path.clone())
        })();
        let name = input.read(cx).value().to_string();
        let unchanged = path
            .file_name()
            .is_some_and(|old| old.to_string_lossy() == name.trim());
        let scheduled = self.rename_path_to(&path, &name, cx);
        window.focus(&self.focus, cx);
        if self.rename_conflict.is_some() || (scheduled.is_none() && !unchanged) {
            self.pending_select = Some(path);
            self.reload(cx);
            return;
        }
        if let Some((destination, _)) = scheduled {
            self.pending_select = next_path.or(Some(destination));
            cx.notify();
            return;
        }
        self.pending_select = next_path.or(Some(path));
        self.reload(cx);
    }

    /// Escape while renaming: cancel, discard the typed text and return
    /// focus to the list, as Finder does. The list's own key handler skips
    /// its navigation while `renaming` is set, so it calls this instead.
    pub(super) fn rename_cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.renaming.take().is_some() {
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }

    /// Rename `path` to `new_name` in its own folder, reporting a failure or
    /// a name clash visibly. Returns the destination when work was scheduled.
    ///
    /// Schedule the same background journaled transfer as a same-folder drag,
    /// so Command-Z can reverse the rename exactly. The receiver reports when
    /// the transfer and its UI reload have completed.
    pub(super) fn rename_path_to(
        &mut self,
        path: &Path,
        new_name: &str,
        cx: &mut Context<Self>,
    ) -> Option<(PathBuf, async_channel::Receiver<bool>)> {
        let entry = entry_for(path)?;
        let new_name = new_name.trim();
        if new_name.is_empty() || new_name == entry.name.as_ref() {
            return None;
        }
        if self.block_mutation_during_transfer(cx) {
            return None;
        }
        let destination = entry
            .path
            .parent()
            .unwrap_or(self.cwd.as_path())
            .join(new_name);
        let occupied = match std::fs::symlink_metadata(&destination) {
            Ok(_) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => {
                self.record_operation_failures(
                    vec![file_ops::Failure::message(
                        file_ops::Operation::Rename,
                        &entry.path,
                        Some(&destination),
                        error.to_string(),
                    )],
                    cx,
                );
                return None;
            }
        };
        if occupied {
            let stem = destination
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy();
            let extension = destination
                .extension()
                .map(|ext| format!(".{}", ext.to_string_lossy()))
                .unwrap_or_default();
            self.rename_conflict = Some(
                if extension.is_empty() {
                    format!("The name “{stem}” is already taken. Please choose a different name.")
                } else {
                    format!("The name “{stem}” with extension “{extension}” is already taken. Please choose a different name.")
                }
                .into(),
            );
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
        if self.operation_journal.is_none() {
            self.operation_error =
                Some("File-operation recovery is unavailable; Rename is disabled".into());
            cx.notify();
            return None;
        }
        let task = file_ops::TransferTask {
            kind: file_ops::TransferKind::Move,
            source: entry.path.clone(),
            destination: destination.clone(),
        };
        let (sender, receiver) = async_channel::bounded(1);
        self.start_transfer_with_retained(
            "Renaming",
            vec![task],
            TransferStartOptions {
                keep_unfinished_in_clipboard: false,
                retained_clipboard: Vec::new(),
                play_drop_sound: false,
                completion: Some(sender),
            },
            cx,
        );
        self.transfer.is_some().then_some((destination, receiver))
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
    use super::{rename_selection, rename_unavailable_reason};

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
