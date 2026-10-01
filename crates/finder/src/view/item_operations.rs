use super::*;
use std::path::Path;

#[cfg(target_os = "macos")]
const FILE_TAG_XATTR: &str = "com.rmac.tag";
#[cfg(not(target_os = "macos"))]
const FILE_TAG_XATTR: &str = "user.rmac.tag";

const FILE_TAGS: [&str; 7] = ["red", "orange", "yellow", "green", "blue", "purple", "gray"];

impl FinderView {
    pub(super) fn menu_unavailable(&mut self, message: &'static str, cx: &mut Context<Self>) {
        self.menu_at = None;
        self.operation_notice = Some(message.into());
        cx.notify();
    }
    /// Finder-style color tags persisted in an app-owned extended attribute.
    /// This avoids touching file contents or ownership.
    pub(super) fn set_selected_tag(&mut self, tag: &'static str, cx: &mut Context<Self>) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        let remove_tag = FILE_TAGS
            .iter()
            .position(|candidate| *candidate == tag)
            .is_some_and(|index| self.selected_tag_checks()[index] == rmac_ui::MenuCheck::On);
        let mut failures = Vec::new();
        let mut updated = 0usize;
        for path in paths {
            let result = if remove_tag {
                clear_file_tag(&path)
            } else {
                write_file_tag(&path, tag)
            };
            if let Err(error) = result {
                failures.push(format!("{}: {error}", path.display()));
            } else {
                updated += 1;
                // FILES-05: keep the Linux tag index (there is no Spotlight
                // here) exactly in step with what Files itself just wrote,
                // without waiting for this folder to be listed again.
                #[cfg(any(target_os = "linux", test))]
                rmac_search::tag_index::record(&path, (!remove_tag).then_some(tag));
            }
        }
        if failures.is_empty() {
            self.operation_error = None;
            self.operation_notice = Some(
                format!(
                    "{} the {tag} tag {} {updated} item{}",
                    if remove_tag { "Removed" } else { "Applied" },
                    if remove_tag { "from" } else { "to" },
                    if updated == 1 { "" } else { "s" }
                )
                .into(),
            );
        } else {
            self.operation_notice = None;
            self.operation_error =
                Some(format!("Could not tag selected item(s): {}", failures.join("; ")).into());
        }
        cx.notify();
    }

    pub(super) fn selected_tag_checks(&self) -> [rmac_ui::MenuCheck; 7] {
        let paths = self.selected_paths();
        let values = paths
            .iter()
            .map(|path| read_file_tag(path).ok())
            .collect::<Vec<_>>();
        std::array::from_fn(|index| {
            let matches = values
                .iter()
                .filter(|value| value.as_deref() == Some(FILE_TAGS[index].as_bytes()))
                .count();
            if matches == 0 {
                rmac_ui::MenuCheck::None
            } else if matches == paths.len() {
                rmac_ui::MenuCheck::On
            } else {
                rmac_ui::MenuCheck::Mixed
            }
        })
    }

    // ---- operations ----
    pub(super) fn new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.block_mutation_during_transfer(cx) {
            return;
        }
        self.operation_error = None;
        let Some(journal) = self.operation_journal.clone() else {
            self.operation_error =
                Some("File-operation recovery is unavailable; New Folder is disabled".into());
            cx.notify();
            return;
        };
        let cwd = self.cwd.clone();
        let window_handle = window.window_handle();
        self.new_folder_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let path = unique_path(cwd.join("untitled folder"));
                    file_ops::create_folder(&file_ops::RealFileSystem, &path)
                        .map_err(|error| error.detail)?;
                    journal
                        .undo_store()
                        .archive_created_folder(&path)
                        .map_err(|error| {
                            format!(
                                "New folder was created, but Undo could not be recorded: {error}"
                            )
                        })?;
                    let availability = journal
                        .undo_store()
                        .latest()
                        .map_err(|error| error.to_string())?;
                    let entry = entry_for(&path).ok_or_else(|| {
                        "The folder was created but could not be displayed".to_owned()
                    })?;
                    Ok::<_, String>((path, entry, availability))
                })
                .await;
            let _ = cx.update_window(window_handle, |_, window, cx| {
                let _ = this.update(cx, |this: &mut FinderView, cx| {
                    this.new_folder_busy = false;
                    match result {
                        Ok((path, entry, availability)) => {
                            this.undo_available = availability;
                            if path.parent() == Some(this.cwd.as_path()) {
                                this.root_entries.retain(|item| item.path != path);
                                this.root_entries.push(entry.clone());
                                sort_entries(&mut this.root_entries, this.sort_key, this.sort_asc);
                                let group = this.current_options().group_by;
                                view_options::group_entries(&mut this.root_entries, group);
                                this.rebuild_list_entries();
                                if let Some(index) =
                                    this.entries.iter().position(|item| item.path == path)
                                {
                                    this.select_single(index);
                                    if this.view == ViewMode::Column {
                                        this.column_selection = Some(entry);
                                    }
                                    this.rename_start(window, cx);
                                }
                            }
                        }
                        Err(error) => this.operation_error = Some(error.into()),
                    }
                    cx.notify();
                });
            });
        })
        .detach();
    }

    pub(super) fn duplicate(&mut self, cx: &mut Context<Self>) {
        let mut tasks = Vec::new();
        for src in self.selected_paths() {
            let stem = src
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let ext = src.extension().map(|e| e.to_string_lossy().into_owned());
            let copy_name = match &ext {
                Some(e) => format!("{stem} copy.{e}"),
                None => format!("{stem} copy"),
            };
            let destination_dir = src.parent().unwrap_or(self.cwd.as_path());
            let dst = destination_dir.join(copy_name);
            tasks.push(file_ops::TransferTask {
                kind: file_ops::TransferKind::Copy,
                source: src,
                destination: dst,
            });
        }
        // Select the new copy once it lands, as Finder does.
        self.start_transfer_with_conflicts("Duplicating", tasks, false, false, cx);
    }

    /// File ▸ Make Alias: a symbolic link next to each selected item, named
    /// "<name> alias" as Finder names a fresh alias. Selects the last alias
    /// made, as Duplicate selects its copy.
    pub(super) fn make_alias(&mut self, cx: &mut Context<Self>) {
        if self.block_mutation_during_transfer(cx) {
            return;
        }
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        let mut destinations = BTreeSet::new();
        let mut failures = Vec::new();
        let mut last_destination = None;
        for src in paths {
            let stem = src
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let ext = src.extension().map(|e| e.to_string_lossy().into_owned());
            let alias_name = match &ext {
                Some(e) => format!("{stem} alias.{e}"),
                None => format!("{stem} alias"),
            };
            let destination_dir = src.parent().unwrap_or(self.cwd.as_path());
            let dst = unique_path_avoiding(destination_dir.join(alias_name), &destinations);
            destinations.insert(dst.clone());
            match std::os::unix::fs::symlink(&src, &dst) {
                Ok(()) => last_destination = Some(dst),
                Err(error) => failures.push(file_ops::Failure::message(
                    file_ops::Operation::CreateAlias,
                    &src,
                    Some(&dst),
                    error.to_string(),
                )),
            }
        }
        self.pending_select = last_destination;
        self.record_operation_failures(failures, cx);
        self.reload(cx);
    }

    /// "Delete Immediately" skips Trash entirely, so — like macOS — it must
    /// never run without the user confirming an unrecoverable delete first.
    /// No confirmation dialog is wired up for this action yet, so it refuses
    /// rather than deleting: silently permitting an unconfirmed, untrashed
    /// delete the moment something (a future keybinding or menu item) calls
    /// this is exactly the trap this guard exists to prevent.
    pub(super) fn delete_immediately(&mut self, cx: &mut Context<Self>) {
        if self.block_mutation_during_transfer(cx) {
            return;
        }
        if self.selected_paths().is_empty() {
            return;
        }
        self.operation_error =
            Some("Delete Immediately needs a confirmation step that isn't available yet".into());
        cx.notify();
    }
}

fn write_file_tag(path: &Path, tag: &str) -> Result<(), rustix::io::Errno> {
    rustix::fs::setxattr(
        path,
        FILE_TAG_XATTR,
        tag.as_bytes(),
        rustix::fs::XattrFlags::empty(),
    )
}

fn clear_file_tag(path: &Path) -> Result<(), rustix::io::Errno> {
    rustix::fs::removexattr(path, FILE_TAG_XATTR)
}

fn read_file_tag(path: &Path) -> Result<Vec<u8>, rustix::io::Errno> {
    let mut buffer = [0u8; 32];
    let length = rustix::fs::getxattr(path, FILE_TAG_XATTR, &mut buffer)?;
    Ok(buffer[..length].to_vec())
}

#[cfg(test)]
mod tag_tests {
    use super::{clear_file_tag, read_file_tag, write_file_tag};
    use std::fs;

    #[test]
    fn selected_tag_is_persisted_as_file_metadata() {
        let path = std::env::temp_dir().join(format!("rmac-tag-test-{}.txt", uuid::Uuid::new_v4()));
        fs::write(&path, "file contents remain unchanged").unwrap();

        if let Err(error) = write_file_tag(&path, "blue") {
            let _ = fs::remove_file(path);
            assert!(
                error == rustix::io::Errno::NOTSUP
                    || error == rustix::io::Errno::OPNOTSUPP
                    || error == rustix::io::Errno::PERM
                    || error == rustix::io::Errno::ACCESS,
                "unexpected extended attribute error: {error}"
            );
            return;
        }
        let stored = read_file_tag(&path).unwrap();

        assert_eq!(stored.as_slice(), b"blue");
        assert_eq!(fs::read(&path).unwrap(), b"file contents remain unchanged");
        clear_file_tag(&path).unwrap();
        assert!(read_file_tag(&path).is_err());
        let _ = fs::remove_file(path);
    }
}
