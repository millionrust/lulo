//! Renderer-independent wording and keyboard policy for Files recovery sheets.

use crate::{operation_journal, ConflictDecision};

#[cfg(any(target_os = "linux", test))]
use crate::trash_store;

pub(crate) struct RecoveryPresentation {
    pub(crate) message: String,
    pub(crate) action_label: &'static str,
}

pub(crate) fn recovery_presentation(
    action: &operation_journal::RecoveryAction,
) -> RecoveryPresentation {
    match action {
        operation_journal::RecoveryAction::PreserveCopy {
            complete,
            suggested_name,
        } => RecoveryPresentation {
            message: if *complete {
                format!(
                    "Files has a complete copy from an interrupted file operation. Preserve it as “{suggested_name}”. Existing items will not be changed."
                )
            } else {
                format!(
                    "The interrupted copy may be incomplete. Preserve it as “{suggested_name}” so you can inspect it. Existing items will not be changed."
                )
            },
            action_label: "Preserve Copy",
        },
        operation_journal::RecoveryAction::PreserveReplacementBackup {
            complete,
            suggested_name,
        } => RecoveryPresentation {
            message: if *complete {
                format!(
                    "Files retained the previous destination from an interrupted replacement. Preserve it as “{suggested_name}”. The replacement and other existing items will not be changed."
                )
            } else {
                format!(
                    "The item retained from an interrupted replacement may have changed or may be incomplete. Preserve it as “{suggested_name}” so you can inspect it. Existing items will not be changed."
                )
            },
            action_label: "Preserve Previous Item",
        },
        operation_journal::RecoveryAction::KeepExistingItems => RecoveryPresentation {
            message: "No staged recovery copy remains. Keep every existing item and clear only this recovery record. No file will be deleted.".to_string(),
            action_label: "Keep Existing Items",
        },
    }
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn trash_recovery_presentation(
    action: &trash_store::TrashRecoveryAction,
) -> RecoveryPresentation {
    match action {
        trash_store::TrashRecoveryAction::ReturnRemainingItem {
            may_be_partial: true,
        } => RecoveryPresentation {
            message: "Files found remaining data from an interrupted permanent deletion. It may be incomplete. Return it to Trash without deleting or replacing any existing item."
                .to_string(),
            action_label: "Return to Trash",
        },
        trash_store::TrashRecoveryAction::ReturnRemainingItem {
            may_be_partial: false,
        } => RecoveryPresentation {
            message: "Files found an item hidden by an interrupted permanent deletion. Return it to Trash without deleting or replacing any existing item."
                .to_string(),
            action_label: "Return to Trash",
        },
        trash_store::TrashRecoveryAction::ReturnRemainingItemAndRebuildMetadata {
            may_be_partial: true,
        } => RecoveryPresentation {
            message: "Files found remaining data from an interrupted permanent deletion, but its Trash metadata is missing. It may be incomplete. Return the exact reviewed data and rebuild its metadata using the recovery time."
                .to_string(),
            action_label: "Return and Rebuild",
        },
        trash_store::TrashRecoveryAction::ReturnRemainingItemAndRebuildMetadata {
            may_be_partial: false,
        } => RecoveryPresentation {
            message: "Files found an item hidden by an interrupted permanent deletion, but its Trash metadata is missing. Return the exact reviewed item and rebuild its metadata using the recovery time."
                .to_string(),
            action_label: "Return and Rebuild",
        },
        trash_store::TrashRecoveryAction::PreserveConflictingItems {
            rebuild_metadata: true,
        } => RecoveryPresentation {
            message: "Files found two different copies from an interrupted permanent deletion, and their Trash metadata is missing. Keep the visible copy unchanged, publish the exact hidden copy under a separate recovered name, and rebuild metadata for both. You can then compare, restore, or copy out either item."
                .to_string(),
            action_label: "Keep Both in Trash",
        },
        trash_store::TrashRecoveryAction::PreserveConflictingItems {
            rebuild_metadata: false,
        } => RecoveryPresentation {
            message: "Files found two different copies from an interrupted permanent deletion. Keep the visible copy unchanged and publish the exact hidden copy under a separate recovered name. Neither copy will be replaced or deleted, so you can compare, restore, or copy out either item."
                .to_string(),
            action_label: "Keep Both in Trash",
        },
        trash_store::TrashRecoveryAction::RebuildMetadata => RecoveryPresentation {
            message: "The exact reviewed item remains in Trash, but its metadata is missing. Rebuild only the metadata using the recovery time; the item data will not be changed."
                .to_string(),
            action_label: "Rebuild Metadata",
        },
        trash_store::TrashRecoveryAction::RemoveOrphanMetadata => RecoveryPresentation {
            message: "No file data remains for this transaction. Remove only its reviewed Trash metadata; no user file will be deleted."
                .to_string(),
            action_label: "Remove Metadata",
        },
        trash_store::TrashRecoveryAction::KeepExistingItems => RecoveryPresentation {
            message: "Keep every existing source, Trash, and destination item. Files will clear only the exact recovery record and will not delete, move, or replace a file."
                .to_string(),
            action_label: "Keep Existing Items",
        },
        trash_store::TrashRecoveryAction::RequiresManualRepair => RecoveryPresentation {
            message: "Files cannot prove a safe automatic repair for this state. Keep it for later; no item or recovery record will be changed."
                .to_string(),
            action_label: "Manual Repair Required",
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RecoveryKeyIntent {
    Close,
    Resolve,
}

pub(crate) fn recovery_key_intent(key: &str, busy: bool) -> Option<RecoveryKeyIntent> {
    if busy {
        return None;
    }
    match key {
        "escape" => Some(RecoveryKeyIntent::Close),
        "enter" => Some(RecoveryKeyIntent::Resolve),
        _ => None,
    }
}

pub(crate) fn conflict_key_intent(key: &str, busy: bool) -> Option<ConflictDecision> {
    if busy {
        return None;
    }
    match key {
        "escape" => Some(ConflictDecision::Skip),
        "enter" => Some(ConflictDecision::KeepBoth),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_keyboard_policy_blocks_shortcuts_while_busy() {
        assert_eq!(
            recovery_key_intent("escape", false),
            Some(RecoveryKeyIntent::Close)
        );
        assert_eq!(
            recovery_key_intent("enter", false),
            Some(RecoveryKeyIntent::Resolve)
        );
        assert_eq!(recovery_key_intent("space", false), None);
        assert_eq!(recovery_key_intent("escape", true), None);
        assert_eq!(recovery_key_intent("enter", true), None);
    }

    #[test]
    fn conflict_keyboard_policy_uses_safe_defaults_and_blocks_while_busy() {
        assert_eq!(
            conflict_key_intent("escape", false),
            Some(ConflictDecision::Skip)
        );
        assert_eq!(
            conflict_key_intent("enter", false),
            Some(ConflictDecision::KeepBoth)
        );
        assert_eq!(conflict_key_intent("space", false), None);
        assert_eq!(conflict_key_intent("escape", true), None);
        assert_eq!(conflict_key_intent("enter", true), None);
    }

    #[test]
    fn recovery_presentation_never_claims_a_partial_copy_is_complete() {
        let partial = recovery_presentation(&operation_journal::RecoveryAction::PreserveCopy {
            complete: false,
            suggested_name: "Recovered item".to_string(),
        });
        let complete = recovery_presentation(&operation_journal::RecoveryAction::PreserveCopy {
            complete: true,
            suggested_name: "Recovered item".to_string(),
        });
        let replacement = recovery_presentation(
            &operation_journal::RecoveryAction::PreserveReplacementBackup {
                complete: true,
                suggested_name: "Recovered item".to_string(),
            },
        );
        let existing = recovery_presentation(&operation_journal::RecoveryAction::KeepExistingItems);

        assert!(partial.message.contains("may be incomplete"));
        assert!(!complete.message.contains("may be incomplete"));
        assert!(complete.message.contains("complete copy"));
        assert!(replacement.message.contains("previous destination"));
        assert_eq!(replacement.action_label, "Preserve Previous Item");
        assert!(existing.message.contains("No file will be deleted"));
    }

    #[test]
    fn trash_recovery_presentations_never_overstate_safe_actions() {
        let partial =
            trash_recovery_presentation(&trash_store::TrashRecoveryAction::ReturnRemainingItem {
                may_be_partial: true,
            });
        let orphan =
            trash_recovery_presentation(&trash_store::TrashRecoveryAction::RemoveOrphanMetadata);
        let keep =
            trash_recovery_presentation(&trash_store::TrashRecoveryAction::KeepExistingItems);
        let conflicting = trash_recovery_presentation(
            &trash_store::TrashRecoveryAction::PreserveConflictingItems {
                rebuild_metadata: false,
            },
        );
        let manual =
            trash_recovery_presentation(&trash_store::TrashRecoveryAction::RequiresManualRepair);

        assert!(partial.message.contains("may be incomplete"));
        assert!(partial.message.contains("without deleting or replacing"));
        assert!(orphan.message.contains("no user file will be deleted"));
        assert!(keep
            .message
            .contains("clear only the exact recovery record"));
        assert!(keep.message.contains("will not delete, move, or replace"));
        assert_eq!(conflicting.action_label, "Keep Both in Trash");
        assert!(conflicting
            .message
            .contains("Neither copy will be replaced or deleted"));
        assert!(conflicting
            .message
            .contains("compare, restore, or copy out"));
        assert!(manual
            .message
            .contains("cannot prove a safe automatic repair"));
        assert!(manual
            .message
            .contains("no item or recovery record will be changed"));
    }
}
