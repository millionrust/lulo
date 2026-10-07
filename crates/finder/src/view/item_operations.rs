use super::*;
use std::path::Path;

#[cfg(target_os = "macos")]
const FILE_TAG_XATTR: &str = "com.rmac.tag";
#[cfg(not(target_os = "macos"))]
const FILE_TAG_XATTR: &str = "user.rmac.tag";

const FILE_TAGS: [&str; 7] = ["red", "orange", "yellow", "green", "blue", "purple", "gray"];

impl FinderView {
    pub(super) fn add_paths_to_dock(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        if paths.is_empty() {
            return;
        }
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let resolved = blocking::unblock(move || {
                paths
                    .into_iter()
                    .map(|path| {
                        std::fs::canonicalize(path)?
                            .into_os_string()
                            .into_string()
                            .map_err(|_| {
                                std::io::Error::new(
                                    std::io::ErrorKind::InvalidInput,
                                    "The path cannot be added to the Dock",
                                )
                            })
                    })
                    .collect::<std::io::Result<Vec<_>>>()
            })
            .await;
            let result = match resolved {
                Ok(paths) => {
                    use rmac_dock_system::Backend as _;
                    let backend = rmac_dock_system::SystemBackend;
                    let mut result = Ok(());
                    for path in paths {
                        let command = rmac_dock::StackCommand::Add(
                            rmac_shell_settings::DockStackKind::Path { path },
                        );
                        if let Err(error) = backend.update_stacks(&command).await {
                            result = Err(error.detail);
                            break;
                        }
                    }
                    result
                }
                Err(error) => Err(error.to_string()),
            };
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                match result {
                    Ok(()) => this.operation_notice = Some("Added to Dock".into()),
                    Err(error) => {
                        this.operation_error =
                            Some(format!("Could not add to Dock: {error}").into())
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// File ▸ Show Original resolves a symbolic-link alias and reveals its
    /// target in the enclosing folder.
    pub(super) fn show_original(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.selected_paths().into_iter().next() else {
            return;
        };
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let target = blocking::unblock(move || {
                let metadata = std::fs::symlink_metadata(&path)?;
                if !metadata.file_type().is_symlink() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "The selected item is not an alias",
                    ));
                }
                std::fs::canonicalize(path)
            })
            .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| match target {
                Ok(target) => {
                    if let Some(parent) = target.parent() {
                        this.pending_select = Some(target.clone());
                        this.navigate(parent.to_path_buf(), cx);
                    }
                }
                Err(error) => {
                    this.operation_error = Some(error.to_string().into());
                    cx.notify();
                }
            });
        })
        .detach();
    }

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
                #[cfg(any(target_os = "linux", all(test, unix)))]
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
    pub(super) fn new_folder_with_selection(&mut self, cx: &mut Context<Self>) {
        if self.block_mutation_during_transfer(cx) {
            return;
        }
        let paths = self.selected_paths();
        let Some(parent) = paths
            .first()
            .and_then(|path| path.parent())
            .map(Path::to_path_buf)
        else {
            self.operation_notice = Some("Select items to put in a new folder".into());
            cx.notify();
            return;
        };
        if paths
            .iter()
            .any(|path| path.parent() != Some(parent.as_path()))
        {
            self.operation_error = Some("Select items from one folder".into());
            cx.notify();
            return;
        }
        let Some(journal) = self.operation_journal.clone() else {
            self.operation_error = Some("File-operation recovery is unavailable".into());
            cx.notify();
            return;
        };
        self.new_folder_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(move || {
                let folder = unique_path(parent.join("New Folder With Items"));
                file_ops::create_folder(&file_ops::RealFileSystem, &folder)
                    .map_err(|error| error.detail)?;
                journal
                    .undo_store()
                    .archive_created_folder(&folder)
                    .map_err(|error| error.to_string())?;
                Ok::<_, String>((folder, paths))
            })
            .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                this.new_folder_busy = false;
                match result {
                    Ok((folder, paths)) => {
                        let tasks = paths
                            .into_iter()
                            .filter_map(|source| {
                                let name = source.file_name()?;
                                Some(file_ops::TransferTask {
                                    kind: file_ops::TransferKind::Move,
                                    destination: folder.join(name),
                                    source,
                                })
                            })
                            .collect();
                        this.pending_select = Some(folder);
                        this.start_transfer_with_conflicts("Moving", tasks, false, false, cx);
                    }
                    Err(error) => {
                        this.operation_error = Some(error.into());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

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

#[cfg(not(windows))]
fn write_file_tag(path: &Path, tag: &str) -> Result<(), rustix::io::Errno> {
    rustix::fs::setxattr(
        path,
        FILE_TAG_XATTR,
        tag.as_bytes(),
        rustix::fs::XattrFlags::empty(),
    )
}

#[cfg(not(windows))]
fn clear_file_tag(path: &Path) -> Result<(), rustix::io::Errno> {
    rustix::fs::removexattr(path, FILE_TAG_XATTR)
}

#[cfg(not(windows))]
fn read_file_tag(path: &Path) -> Result<Vec<u8>, rustix::io::Errno> {
    let mut buffer = [0u8; 32];
    let length = rustix::fs::getxattr(path, FILE_TAG_XATTR, &mut buffer)?;
    Ok(buffer[..length].to_vec())
}

// Windows has no xattrs. An NTFS alternate data stream named after the path
// (`<path>:lulo.tags`) is the simplest honest equivalent (ADR 0023 phase 4):
// one extra, unindexed stream per tagged file, written and read with plain
// `std::fs`, and removed with `remove_file` on the stream's own name — NTFS
// deletes exactly that stream, not the file it rides on. It is lost if the
// file moves to a non-NTFS volume (a USB stick formatted FAT/exFAT), which
// is the same honest limitation xattr tags already have crossing to a
// filesystem without extended attributes.
#[cfg(windows)]
fn tag_stream_path(path: &Path) -> std::path::PathBuf {
    let mut stream = path.as_os_str().to_owned();
    stream.push(":lulo.tags");
    std::path::PathBuf::from(stream)
}

#[cfg(windows)]
fn write_file_tag(path: &Path, tag: &str) -> std::io::Result<()> {
    std::fs::write(tag_stream_path(path), tag.as_bytes())
}

#[cfg(windows)]
fn clear_file_tag(path: &Path) -> std::io::Result<()> {
    std::fs::remove_file(tag_stream_path(path))
}

#[cfg(windows)]
fn read_file_tag(path: &Path) -> std::io::Result<Vec<u8>> {
    std::fs::read(tag_stream_path(path))
}

/// The one Finder-style colour tag on `path`, by name (e.g. "blue"), for
/// sorting and the Clean Up By/Sort By ▸ Tags commands. `None` covers both
/// "no tag" and an unreadable xattr; both sort the same way.
pub(super) fn file_tag_label(path: &Path) -> Option<SharedString> {
    let raw = read_file_tag(path).ok()?;
    FILE_TAGS
        .iter()
        .find(|tag| tag.as_bytes() == raw.as_slice())
        .map(|tag| SharedString::from(*tag))
}

#[cfg(all(test, not(windows)))]
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

// Windows: the alternate-data-stream tag, round-tripped on a real NTFS file.
// GitHub's `windows-latest` runner's `target`/temp volumes are NTFS, so this
// is a real check, not a skip.
#[cfg(all(test, windows))]
mod tag_tests {
    use super::{clear_file_tag, read_file_tag, write_file_tag};
    use std::fs;

    #[test]
    fn selected_tag_is_persisted_in_an_alternate_data_stream() {
        let path = std::env::temp_dir().join(format!("rmac-tag-test-{}.txt", uuid::Uuid::new_v4()));
        fs::write(&path, "file contents remain unchanged").unwrap();

        write_file_tag(&path, "blue").unwrap();
        let stored = read_file_tag(&path).unwrap();

        assert_eq!(stored.as_slice(), b"blue");
        assert_eq!(fs::read(&path).unwrap(), b"file contents remain unchanged");
        clear_file_tag(&path).unwrap();
        assert!(read_file_tag(&path).is_err());
        let _ = fs::remove_file(path);
    }
}
