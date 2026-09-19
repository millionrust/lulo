//! Exact adapters from Files' live controller models to public semantics.

use rmac_finder::accessibility::{
    project_files_accessibility, AccessibilityProjectionError, AccessibleDialog,
    AccessibleDialogAction, AccessibleDialogOption, AccessibleGallery, AccessibleGalleryItem,
    AccessibleLiveRegion, AccessibleProgress, DialogActionKind, DialogFocus, DialogKind,
    FilesAccessibilitySnapshot, LivePoliteness, ProgressUnit,
};

use super::*;

impl FinderView {
    /// Pinned GPUI cannot publish an accessibility tree. This exact projection
    /// is intentionally dormant until the A5/A6 framework decision provides an
    /// adapter; keeping it attached to live state prevents a future adapter
    /// from reconstructing file-operation policy.
    #[allow(dead_code)]
    pub(super) fn accessibility_snapshot(
        &self,
        cx: &Context<Self>,
    ) -> Result<FilesAccessibilitySnapshot, AccessibilityProjectionError> {
        project_files_accessibility(
            self.accessible_gallery(cx),
            self.accessible_dialogs(),
            self.accessible_live_regions(),
        )
    }

    fn accessible_gallery(&self, cx: &Context<Self>) -> Option<AccessibleGallery> {
        if self.view != ViewMode::Gallery {
            return None;
        }
        let query = if self.search_summary.is_some() {
            String::new()
        } else {
            self.query.read(cx).value().to_lowercase()
        };
        let visible_indices = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                (query.is_empty() || entry.name.to_lowercase().contains(&query)).then_some(index)
            })
            .collect::<Vec<_>>();
        let selection_count = self
            .selected
            .iter()
            .filter(|index| visible_indices.contains(index))
            .count();
        let active_entry_index = self
            .anchor
            .filter(|index| self.selected.contains(index) && visible_indices.contains(index))
            .or_else(|| {
                self.selected
                    .iter()
                    .copied()
                    .find(|index| visible_indices.contains(index))
            });
        let active_item = active_entry_index
            .and_then(|index| visible_indices.iter().position(|visible| *visible == index));
        let items = visible_indices
            .iter()
            .filter_map(|index| self.entries.get(*index).map(|entry| (*index, entry)))
            .map(|(index, entry)| AccessibleGalleryItem {
                stable_id: format!("gallery-item-{index}"),
                name: sanitize_dialog_name(entry.name.as_ref()),
                description: format!("{} · {} · {}", entry.kind, entry.size, entry.modified),
                selected: self.selected.contains(&index),
                is_directory: entry.is_dir,
            })
            .collect();
        let active_entry = active_entry_index.and_then(|index| self.entries.get(index));
        let preview_name = active_entry.map(|entry| {
            if selection_count > 1 {
                format!("{selection_count} items selected")
            } else {
                sanitize_dialog_name(entry.name.as_ref())
            }
        });
        let preview_description = active_entry.map_or_else(
            || {
                if visible_indices.is_empty() {
                    "No matching items. Try a different search."
                } else {
                    "Select an item in the filmstrip to preview it."
                }
                .to_string()
            },
            |entry| {
                if selection_count > 1 {
                    "The focused item is shown".to_string()
                } else {
                    format!("{} · {} · {}", entry.kind, entry.size, entry.modified)
                }
            },
        );
        let has_selection = selection_count != 0;
        let can_open_with = selection_count == 1 && active_entry.is_some_and(|entry| !entry.is_dir);
        let actions = if self.trash_view {
            vec![
                dialog_action("gallery-restore", "Restore", DialogActionKind::Default)
                    .disabled(!has_selection),
                dialog_action(
                    "gallery-delete-permanently",
                    "Delete Permanently",
                    DialogActionKind::Destructive,
                )
                .disabled(!has_selection),
            ]
        } else {
            vec![
                dialog_action("gallery-open", "Open", DialogActionKind::Default)
                    .disabled(!has_selection),
                dialog_action("gallery-open-with", "Open With", DialogActionKind::Normal)
                    .disabled(!can_open_with),
                dialog_action("gallery-rename", "Rename", DialogActionKind::Normal)
                    .disabled(selection_count != 1),
                dialog_action("gallery-duplicate", "Duplicate", DialogActionKind::Normal)
                    .disabled(!has_selection),
                dialog_action("gallery-quick-look", "Quick Look", DialogActionKind::Normal)
                    .disabled(!has_selection),
                dialog_action("gallery-copy", "Copy", DialogActionKind::Normal)
                    .disabled(!has_selection),
                dialog_action("gallery-cut", "Cut", DialogActionKind::Normal)
                    .disabled(!has_selection),
                dialog_action("gallery-get-info", "Get Info", DialogActionKind::Normal)
                    .disabled(selection_count != 1),
                dialog_action(
                    "gallery-move-to-trash",
                    "Move to Trash",
                    DialogActionKind::Normal,
                )
                .disabled(!has_selection),
                dialog_action(
                    "gallery-delete-immediately",
                    "Delete Immediately",
                    DialogActionKind::Destructive,
                )
                .disabled(!has_selection),
                dialog_action("gallery-paste", "Paste Item", DialogActionKind::Normal)
                    .disabled(self.clipboard.is_empty()),
                dialog_action("gallery-new-folder", "New Folder", DialogActionKind::Normal),
            ]
        };
        Some(AccessibleGallery {
            items,
            active_item,
            selection_count,
            preview_name,
            preview_description,
            actions,
        })
    }

    fn accessible_dialogs(&self) -> Vec<AccessibleDialog> {
        let mut dialogs = Vec::new();
        if let Some(index) = self.info {
            if let Some(entry) = self.entries.get(index) {
                dialogs.push(self.accessible_info_dialog(entry));
            }
        }
        if let Some(dialog) = self.accessible_conflict_dialog() {
            dialogs.push(dialog);
        }
        if let Some(dialog) = self.accessible_recovery_dialog() {
            dialogs.push(dialog);
        }
        #[cfg(any(target_os = "linux", test))]
        if let Some(dialog) = self.accessible_trash_recovery_dialog() {
            dialogs.push(dialog);
        }
        #[cfg(any(target_os = "linux", test))]
        if let Some(dialog) = self.accessible_delete_dialog() {
            dialogs.push(dialog);
        }
        if let Some(dialog) = self.accessible_open_with_dialog() {
            dialogs.push(dialog);
        }
        if let Some(dialog) = self.accessible_quick_look_dialog() {
            dialogs.push(dialog);
        }
        dialogs
    }

    fn accessible_info_dialog(&self, entry: &Entry) -> AccessibleDialog {
        let name = sanitize_dialog_name(entry.name.as_ref());
        let description = file_info(entry)
            .into_iter()
            .map(|(key, value)| format!("{key}: {value}"))
            .collect::<Vec<_>>()
            .join("\n");
        AccessibleDialog {
            kind: DialogKind::GetInfo,
            title: format!("{name} Info"),
            description,
            actions: vec![dialog_action(
                "info-close",
                "Close",
                DialogActionKind::Normal,
            )],
            options: Vec::new(),
            initial_focus: DialogFocus::Action(0),
            document_text: None,
            status: None,
        }
    }

    fn accessible_conflict_dialog(&self) -> Option<AccessibleDialog> {
        let batch = self.conflict_batch.as_ref()?;
        let conflict = batch.conflicts.front()?;
        let current = batch
            .conflict_total
            .saturating_sub(batch.conflicts.len())
            .saturating_add(1);
        let busy = self.conflict_busy;
        let replace_available =
            conflict.destination_snapshot.is_some() && conflict.source != conflict.destination;
        Some(AccessibleDialog {
            kind: DialogKind::Conflict,
            title: format!(
                "An Item With This Name Already Exists ({current} of {})",
                batch.conflict_total
            ),
            description: conflict_prompt(conflict),
            actions: vec![
                dialog_action("conflict-skip", "Skip", DialogActionKind::Normal).disabled(busy),
                dialog_action("conflict-replace", "Replace", DialogActionKind::Destructive)
                    .disabled(busy || !replace_available)
                    .busy(busy),
                dialog_action(
                    "conflict-keep-both",
                    if busy { "Checking…" } else { "Keep Both" },
                    DialogActionKind::Default,
                )
                .disabled(busy)
                .busy(busy),
            ],
            options: Vec::new(),
            // Skip is the non-mutating initial target; Return remains the
            // controller's explicit Keep Both shortcut.
            initial_focus: DialogFocus::Action(0),
            document_text: None,
            status: busy.then(|| polite_status("conflict-status", "Checking conflict state…")),
        })
    }

    fn accessible_recovery_dialog(&self) -> Option<AccessibleDialog> {
        if !self.recovery_open {
            return None;
        }
        let review = self.recovery_reviews.first()?;
        let presentation = recovery_presentation(&review.action);
        let busy = self.recovery_busy;
        Some(AccessibleDialog {
            kind: DialogKind::Recovery,
            title: format!(
                "Recover File Operation (1 of {})",
                self.recovery_reviews.len()
            ),
            description: presentation.message,
            actions: vec![
                dialog_action("recovery-later", "Later", DialogActionKind::Normal).disabled(busy),
                dialog_action(
                    "recovery-confirm",
                    if busy {
                        "Resolving…"
                    } else {
                        presentation.action_label
                    },
                    DialogActionKind::Default,
                )
                .disabled(busy)
                .busy(busy),
            ],
            options: Vec::new(),
            initial_focus: DialogFocus::Action(0),
            document_text: None,
            status: busy.then(|| polite_status("recovery-status", "Resolving recovery…")),
        })
    }

    #[cfg(any(target_os = "linux", test))]
    fn accessible_trash_recovery_dialog(&self) -> Option<AccessibleDialog> {
        if !self.trash_recovery_open || self.recovery_open {
            return None;
        }
        let review = self.trash_recovery_reviews.first()?;
        let presentation = trash_recovery_presentation(&review.action);
        let busy = self.trash_recovery_busy;
        let resolvable = !matches!(
            &review.action,
            trash_store::TrashRecoveryAction::RequiresManualRepair
        );
        Some(AccessibleDialog {
            kind: DialogKind::TrashRecovery,
            title: format!(
                "Recover Trash Operation (1 of {})",
                self.trash_recovery_reviews.len()
            ),
            description: presentation.message,
            actions: vec![
                dialog_action("trash-recovery-later", "Later", DialogActionKind::Normal)
                    .disabled(busy),
                dialog_action(
                    "trash-recovery-confirm",
                    if busy {
                        "Resolving…"
                    } else {
                        presentation.action_label
                    },
                    DialogActionKind::Default,
                )
                .disabled(busy || !resolvable)
                .busy(busy),
            ],
            options: Vec::new(),
            initial_focus: DialogFocus::Action(0),
            document_text: None,
            status: busy.then(|| polite_status("trash-recovery-status", "Resolving recovery…")),
        })
    }

    #[cfg(any(target_os = "linux", test))]
    fn accessible_delete_dialog(&self) -> Option<AccessibleDialog> {
        let confirmation = self.delete_confirmation.as_ref()?;
        let count = confirmation.items.len();
        let name = confirmation
            .items
            .first()
            .and_then(|item| item.original_path.file_name())
            .map(|name| sanitize_dialog_name(&name.to_string_lossy()));
        Some(AccessibleDialog {
            kind: DialogKind::PermanentDelete,
            title: if count == 1 {
                "Delete Item Permanently?"
            } else {
                "Delete Items Permanently?"
            }
            .to_string(),
            description: permanent_delete_prompt(count, name.as_deref()),
            actions: vec![
                dialog_action(
                    "permanent-delete-cancel",
                    "Cancel",
                    DialogActionKind::Normal,
                ),
                dialog_action(
                    "permanent-delete-confirm",
                    "Delete",
                    DialogActionKind::Destructive,
                ),
            ],
            options: Vec::new(),
            initial_focus: DialogFocus::Action(0),
            document_text: None,
            status: None,
        })
    }

    fn accessible_open_with_dialog(&self) -> Option<AccessibleDialog> {
        let picker = self.open_with.as_ref()?;
        let name = picker
            .path
            .file_name()
            .map(|name| sanitize_dialog_name(&name.to_string_lossy()))
            .unwrap_or_else(|| "this file".into());
        let mut description = "Finding compatible applications…".to_string();
        let mut options = Vec::new();
        let mut can_open = false;
        let mut selected_is_default = false;
        if let Some(association) = &picker.association {
            description = format!(
                "Choose an application for “{name}” ({})",
                association.mime_type
            );
            if association.handlers.is_empty() {
                description
                    .push_str(". No installed application advertises support for this file type.");
            } else {
                can_open = true;
                let selected = picker.selected.min(association.handlers.len() - 1);
                options = association
                    .handlers
                    .iter()
                    .enumerate()
                    .map(|(index, application)| {
                        let is_default = association.default_application_id.as_deref()
                            == Some(application.id.as_str());
                        if index == selected {
                            selected_is_default = is_default;
                        }
                        AccessibleDialogOption {
                            stable_id: application.id.clone(),
                            name: sanitize_dialog_name(&application.name),
                            selected: index == selected,
                            is_default,
                        }
                    })
                    .collect();
            }
        }
        let checked = selected_is_default || picker.make_default;
        let busy = picker.busy;
        let mut actions = Vec::new();
        if can_open {
            actions.push(
                dialog_action(
                    "open-with-default",
                    if selected_is_default {
                        "This application is already the default"
                    } else {
                        "Always open this file type with this application"
                    },
                    DialogActionKind::Toggle,
                )
                .checked(checked)
                .disabled(selected_is_default || busy),
            );
        }
        actions.extend([
            dialog_action(
                "open-with-cancel",
                if can_open { "Cancel" } else { "Close" },
                DialogActionKind::Normal,
            )
            .disabled(busy),
            dialog_action(
                "open-with-confirm",
                if busy { "Opening…" } else { "Open" },
                DialogActionKind::Default,
            )
            .disabled(busy || !can_open)
            .busy(busy),
        ]);
        let status = picker
            .error
            .as_ref()
            .map(|error| assertive_status("open-with-error", error.as_ref()))
            .or_else(|| busy.then(|| polite_status("open-with-status", "Opening application…")))
            .or_else(|| {
                picker
                    .association
                    .is_none()
                    .then(|| polite_status("open-with-status", "Finding compatible applications…"))
            });
        Some(AccessibleDialog {
            kind: DialogKind::OpenWith,
            title: "Open With".to_string(),
            description,
            actions,
            options,
            initial_focus: if can_open {
                DialogFocus::Option(
                    picker
                        .selected
                        .min(picker.association.as_ref()?.handlers.len() - 1),
                )
            } else {
                DialogFocus::Action(0)
            },
            document_text: None,
            status,
        })
    }

    fn accessible_quick_look_dialog(&self) -> Option<AccessibleDialog> {
        let panel = self.quick_look.as_ref()?;
        let path = panel.paths.get(panel.current)?;
        let name = path
            .file_name()
            .map(|name| sanitize_dialog_name(&name.to_string_lossy()))
            .unwrap_or_else(|| root_volume_name().into());
        let count = panel.paths.len();
        let position = panel.current + 1;
        let mut document_text = None;
        let (description, status) = if let Some(error) = &panel.error {
            (
                error.to_string(),
                Some(assertive_status("quick-look-error", error.as_ref())),
            )
        } else {
            match panel.content.as_ref() {
                None => (
                    "Loading preview…".to_string(),
                    Some(polite_status("quick-look-status", "Loading preview…")),
                ),
                Some(quick_look::Content::Image { .. }) => ("Image preview".to_string(), None),
                Some(quick_look::Content::Media { kind, .. }) => (
                    match kind {
                        rmac_thumbnails::MediaKind::Pdf => "PDF · first page",
                        rmac_thumbnails::MediaKind::Video => "Video · preview frame",
                        rmac_thumbnails::MediaKind::Audio => "Audio waveform · first 30 seconds",
                    }
                    .to_string(),
                    None,
                ),
                Some(quick_look::Content::MediaUnavailable { kind }) => (
                    format!("{} preview capability unavailable", kind.label()),
                    None,
                ),
                Some(quick_look::Content::Text { text, truncated }) => {
                    document_text = Some(text.clone());
                    (
                        if *truncated {
                            "Showing the first 64 KB of text"
                        } else {
                            "Text preview"
                        }
                        .to_string(),
                        None,
                    )
                }
                Some(quick_look::Content::Folder { items, truncated }) => (
                    if *truncated {
                        format!("At least {items} items. Folder summary")
                    } else {
                        format!(
                            "{items} item{}. Folder summary",
                            if *items == 1 { "" } else { "s" }
                        )
                    },
                    None,
                ),
                Some(quick_look::Content::Link { target }) => {
                    (format!("Symbolic link to\n{target}"), None)
                }
                Some(quick_look::Content::Unsupported) => (
                    "No Preview Available. Press Return after closing Quick Look to open the item."
                        .to_string(),
                    None,
                ),
            }
        };
        let mut actions = vec![dialog_action(
            "quick-look-close",
            "Close",
            DialogActionKind::Normal,
        )];
        if count > 1 {
            actions.push(
                dialog_action(
                    "quick-look-previous",
                    "Previous item",
                    DialogActionKind::Normal,
                )
                .disabled(panel.current == 0),
            );
            actions.push(
                dialog_action("quick-look-next", "Next item", DialogActionKind::Normal)
                    .disabled(position >= count),
            );
        }
        Some(AccessibleDialog {
            kind: DialogKind::QuickLook,
            title: name,
            description: if count > 1 {
                format!("{position} of {count}. {description}")
            } else {
                description
            },
            actions,
            options: Vec::new(),
            initial_focus: DialogFocus::Action(0),
            document_text,
            status,
        })
    }

    fn accessible_live_regions(&self) -> Vec<AccessibleLiveRegion> {
        let mut regions = Vec::new();
        if let Some(notice) = &self.operation_notice {
            regions.push(AccessibleLiveRegion {
                id: "operation-notice".to_string(),
                text: notice.to_string(),
                politeness: LivePoliteness::Polite,
                progress: None,
                actions: vec![dialog_action(
                    "dismiss-operation-notice",
                    "Dismiss",
                    DialogActionKind::Normal,
                )],
            });
        }
        if let Some(error) = &self.operation_error {
            let recovery_pending = self.pending_operations != 0;
            #[cfg(any(target_os = "linux", test))]
            let recovery_pending = recovery_pending || self.trash_pending != 0;
            regions.push(AccessibleLiveRegion {
                id: "operation-error".to_string(),
                text: rmac_ui::user_error_message(
                    rmac_ui::ErrorSurface::Files,
                    error.as_ref(),
                    recovery_pending,
                )
                .to_string(),
                politeness: LivePoliteness::Assertive,
                progress: None,
                actions: vec![dialog_action(
                    if recovery_pending {
                        "review-operation-error"
                    } else {
                        "dismiss-operation-error"
                    },
                    if recovery_pending {
                        "Review"
                    } else {
                        "Dismiss"
                    },
                    if recovery_pending {
                        DialogActionKind::Default
                    } else {
                        DialogActionKind::Normal
                    },
                )],
            });
        }
        #[cfg(any(target_os = "linux", test))]
        if let Some(operation) = &self.trash_operation {
            let cancelling = operation.cancelling;
            regions.push(AccessibleLiveRegion {
                id: "trash-progress".to_string(),
                text: trash_progress_text(
                    operation.label.as_ref(),
                    operation.processed,
                    operation.total,
                ),
                politeness: LivePoliteness::Polite,
                progress: nonzero_progress(
                    operation.processed,
                    operation.total,
                    ProgressUnit::Items,
                ),
                actions: vec![dialog_action(
                    "cancel-trash",
                    cancel_progress_label(cancelling),
                    DialogActionKind::Normal,
                )
                .disabled(cancelling)],
            });
        }
        if let Some(undo) = &self.undo_operation {
            regions.push(AccessibleLiveRegion {
                id: "undo-progress".to_string(),
                text: undo_progress_text(undo),
                politeness: LivePoliteness::Polite,
                progress: None,
                actions: vec![dialog_action(
                    "cancel-undo",
                    cancel_progress_label(undo.cancelling),
                    DialogActionKind::Normal,
                )
                .disabled(undo.cancelling)],
            });
        }
        if let Some(transfer) = &self.transfer {
            let progress = if transfer.bytes_total > 0 {
                nonzero_progress(
                    transfer.bytes_processed,
                    transfer.bytes_total.max(transfer.bytes_processed),
                    ProgressUnit::Bytes,
                )
            } else {
                nonzero_progress(transfer.processed, transfer.total, ProgressUnit::Items)
            };
            regions.push(AccessibleLiveRegion {
                id: "transfer-progress".to_string(),
                text: transfer_progress_text(transfer),
                politeness: LivePoliteness::Polite,
                progress,
                actions: vec![dialog_action(
                    "cancel-transfer",
                    cancel_progress_label(transfer.cancelling),
                    DialogActionKind::Normal,
                )
                .disabled(transfer.cancelling)],
            });
        }
        regions
    }
}

fn dialog_action(id: &str, name: &str, kind: DialogActionKind) -> AccessibleDialogAction {
    AccessibleDialogAction::new(id, name, kind)
}

fn polite_status(id: &str, text: &str) -> AccessibleLiveRegion {
    AccessibleLiveRegion {
        id: id.to_string(),
        text: text.to_string(),
        politeness: LivePoliteness::Polite,
        progress: None,
        actions: Vec::new(),
    }
}

fn assertive_status(id: &str, text: &str) -> AccessibleLiveRegion {
    AccessibleLiveRegion {
        politeness: LivePoliteness::Assertive,
        ..polite_status(id, text)
    }
}

fn nonzero_progress(
    current: impl TryInto<u64>,
    total: impl TryInto<u64>,
    unit: ProgressUnit,
) -> Option<AccessibleProgress> {
    let current = current.try_into().ok()?;
    let total = total.try_into().ok()?;
    (total != 0).then_some(AccessibleProgress {
        current,
        total,
        unit,
    })
}

pub(super) fn trash_progress_text(label: &str, processed: usize, total: usize) -> String {
    format!("{label} — {processed} of {total} items")
}

pub(super) fn undo_progress_text(undo: &ActiveUndo) -> String {
    match undo.phase {
        file_ops::TransferPhase::Scanning => format!("{} — Checking items", undo.label),
        file_ops::TransferPhase::Copying if undo.bytes_processed != 0 => {
            format!(
                "{} — Restoring {}",
                undo.label,
                human_size(undo.bytes_processed)
            )
        }
        file_ops::TransferPhase::Copying => format!("{} — Restoring item", undo.label),
        file_ops::TransferPhase::Finishing => format!("{} — Finishing safely", undo.label),
    }
}

pub(super) fn transfer_progress_text(transfer: &ActiveTransfer) -> String {
    match transfer.phase {
        file_ops::TransferPhase::Scanning => format!(
            "{} — Scanning {} of {} items",
            transfer.label, transfer.processed, transfer.total
        ),
        file_ops::TransferPhase::Copying if transfer.bytes_total > 0 => format!(
            "{} — {} of {} · {} of {}",
            transfer.label,
            transfer.processed,
            transfer.total,
            human_size(transfer.bytes_processed),
            human_size(transfer.bytes_total.max(transfer.bytes_processed))
        ),
        file_ops::TransferPhase::Copying => format!(
            "{} — Copying {} of {} items",
            transfer.label, transfer.processed, transfer.total
        ),
        file_ops::TransferPhase::Finishing => format!(
            "{} — Finishing {} of {} items",
            transfer.label, transfer.processed, transfer.total
        ),
    }
}

pub(super) const fn cancel_progress_label(cancelling: bool) -> &'static str {
    if cancelling {
        "Cancelling…"
    } else {
        "Cancel"
    }
}
