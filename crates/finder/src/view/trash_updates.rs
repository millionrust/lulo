use super::*;

impl FinderView {
    pub(super) fn reload_trash(&mut self, cx: &mut Context<Self>) {
        #[cfg(any(target_os = "linux", all(test, unix)))]
        {
            self.cancel_search();
            self.result_title = Some(self.file_words.bin().into());
            self.search_summary = None;
            self.search_relevance_order = false;
            self.selected.clear();
            self.anchor = None;
            self.renaming = None;
            self.trash_generation = self.trash_generation.wrapping_add(1);
            let generation = self.trash_generation;
            let key = self.sort_key;
            let asc = self.sort_asc;
            let Some(store) = self.trash_store.clone() else {
                self.entries.clear();
                self.trash_items.clear();
                self.operation_error = Some(
                    if self.trash_loading {
                        "Files is still verifying Trash recovery"
                    } else {
                        "Trash is unavailable"
                    }
                    .into(),
                );
                cx.notify();
                return;
            };
            cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let items = store.list()?;
                        let mut entries = Vec::with_capacity(items.len());
                        for item in &items {
                            let mut entry = entry_for(item.data_path()).ok_or_else(|| {
                                std::io::Error::new(
                                    std::io::ErrorKind::WouldBlock,
                                    "Trash changed while it was listed",
                                )
                            })?;
                            entry.name = item
                                .original_path
                                .file_name()
                                .unwrap_or(item.name.as_os_str())
                                .to_string_lossy()
                                .into_owned()
                                .into();
                            entry.modified = deletion_label(&item.deleted_at).into();
                            entries.push(entry);
                        }
                        sort_entries(&mut entries, key, asc);
                        Ok::<_, std::io::Error>((items, entries))
                    })
                    .await;
                let _ = this.update(cx, |this: &mut FinderView, cx| {
                    if !this.trash_view || this.trash_generation != generation {
                        return;
                    }
                    match result {
                        Ok((items, entries)) => {
                            this.trash_items = items;
                            this.entries = entries;
                            this.free_bytes = None;
                            // Trash just verified cleanly: a banner left over
                            // from an earlier transient hiccup (the items
                            // still arrived in Trash; only the housekeeping
                            // check that ran afterwards failed) would
                            // otherwise claim Trash "isn't available" forever,
                            // even though this listing just proved it is.
                            if should_clear_stale_trash_error(
                                this.operation_error.as_deref(),
                                this.trash_pending,
                            ) {
                                this.operation_error = None;
                            }
                        }
                        Err(error) => {
                            this.entries.clear();
                            this.trash_items.clear();
                            this.operation_error = Some(
                                match error.kind() {
                                    std::io::ErrorKind::WouldBlock => {
                                        "Trash is busy or changed; try again"
                                    }
                                    _ => "Trash could not be verified safely",
                                }
                                .into(),
                            );
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }
        #[cfg(not(any(target_os = "linux", all(test, unix))))]
        {
            self.entries.clear();
            self.operation_error = Some("Trash browsing is available on Linux".into());
            cx.notify();
        }
    }
}

/// A Trash item's XDG `DeletionDate` (local time, `YYYY-MM-DDThh:mm:ss`) in
/// the list's date style ("Today at 11:19 AM"); an unreadable one is shown
/// as written.
#[cfg(any(target_os = "linux", test))]
pub(super) fn deletion_label(raw: &str) -> String {
    chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S")
        .ok()
        .and_then(|local| local.and_local_timezone(chrono::Local).earliest())
        .map(|time| rmac_finder::listing::date_label(std::time::SystemTime::from(time)))
        .unwrap_or_else(|| raw.to_owned())
}

/// Whether a banner currently shown should be cleared because Trash just
/// listed cleanly. `operation_error` is a single shared slot for unrelated
/// failures too (sidebar favourites, applications…), so this only clears a
/// message that actually mentions Trash — never something else that happens
/// to be showing at the same time.
#[cfg(any(target_os = "linux", test))]
fn should_clear_stale_trash_error(current: Option<&str>, trash_pending: usize) -> bool {
    trash_pending == 0 && current.is_some_and(|error| error.to_ascii_lowercase().contains("trash"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_trash_errors_clear_once_trash_verifies_cleanly() {
        assert!(should_clear_stale_trash_error(
            Some("Trash isn't available right now—try again."),
            0,
        ));
        assert!(should_clear_stale_trash_error(
            Some("Trash recovery data could not be verified; Trash actions are disabled"),
            0,
        ));
    }

    #[test]
    fn unrelated_errors_are_left_alone() {
        assert!(!should_clear_stale_trash_error(
            Some("Could not load applications: permission denied"),
            0,
        ));
        assert!(!should_clear_stale_trash_error(None, 0));
    }

    #[test]
    fn a_genuinely_pending_review_is_not_cleared() {
        assert!(!should_clear_stale_trash_error(
            Some("Trash isn't available right now—try again."),
            1,
        ));
    }
}
