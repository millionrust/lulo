use super::*;
use std::path::Path;

#[cfg(target_os = "macos")]
const FILE_TAG_XATTR: &str = "com.rmac.tag";
#[cfg(not(target_os = "macos"))]
const FILE_TAG_XATTR: &str = "user.rmac.tag";

const FILE_TAGS: [&str; 7] = ["red", "orange", "yellow", "green", "blue", "purple", "gray"];

impl FinderView {
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
        let path = unique_path(self.cwd.join("untitled folder"));
        if let Err(failure) = file_ops::create_folder(&file_ops::RealFileSystem, &path) {
            self.record_operation_failures(vec![failure], cx);
            return;
        }
        if let Some(journal) = self.operation_journal.as_ref() {
            if let Err(error) = journal.undo_store().archive_created_folder(&path) {
                self.operation_error = Some(format!("New folder was created, but Undo could not be recorded: {error}").into());
            } else {
                self.undo_available = journal.undo_store().latest().ok().flatten();
            }
        }

        let Some(entry) = entry_for(&path) else {
            self.operation_error = Some("The folder was created but could not be displayed".into());
            self.reload(cx);
            return;
        };
        self.entries.push(entry.clone());
        sort_entries(&mut self.entries, self.sort_key, self.sort_asc);
        let Some(index) = self.entries.iter().position(|entry| entry.path == path) else {
            self.reload(cx);
            return;
        };
        self.selected.clear();
        self.selected.insert(index);
        self.anchor = Some(index);
        if self.view == ViewMode::Column {
            self.column_selection = Some(entry);
        }
        self.rename_start(window, cx);
    }

    pub(super) fn duplicate(&mut self, cx: &mut Context<Self>) {
        let mut tasks = Vec::new();
        let mut destinations = BTreeSet::new();
        let mut new_selection = Vec::new();
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
            let dst = unique_path_avoiding(destination_dir.join(copy_name), &destinations);
            destinations.insert(dst.clone());
            new_selection.push(dst.clone());
            tasks.push(file_ops::TransferTask {
                kind: file_ops::TransferKind::Copy,
                source: src,
                destination: dst,
            });
        }
        // Select the new copy once it lands, as Finder does.
        self.pending_select = None;
        self.pending_select_many = new_selection;
        self.start_transfer("Duplicating", tasks, false, cx);
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
