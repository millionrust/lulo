use super::*;

impl FinderView {
    pub(in crate::view) fn render_recovery(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if !self.recovery_open {
            return None;
        }
        let review = self.recovery_reviews.first()?;
        let presentation = recovery_presentation(&review.action);
        let title = format!(
            "Recover File Operation (1 of {})",
            self.recovery_reviews.len()
        );
        let busy = self.recovery_busy;
        let buttons = vec![
            rmac_ui::dialog_button("recovery-later", "Later", rmac_ui::DialogButtonKind::Normal)
                .disabled(busy)
                .on_click(cx.listener(|this, _, _, cx| this.close_recovery(cx)))
                .into_any_element(),
            rmac_ui::dialog_button(
                "recovery-confirm",
                if busy {
                    "Resolving…"
                } else {
                    presentation.action_label
                },
                rmac_ui::DialogButtonKind::Primary,
            )
            .busy(busy)
            .disabled(busy)
            .on_click(cx.listener(|this, _, _, cx| this.resolve_current_recovery(cx)))
            .into_any_element(),
        ];
        Some(rmac_ui::alert(title, presentation.message, buttons).into_any_element())
    }

    pub(in crate::view) fn render_conflict(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let batch = self.conflict_batch.as_ref()?;
        let conflict = batch.conflicts.front()?;
        let current = batch
            .conflict_total
            .saturating_sub(batch.conflicts.len())
            .saturating_add(1);
        let title = format!(
            "An Item With This Name Already Exists ({current} of {})",
            batch.conflict_total
        );
        let busy = self.conflict_busy;
        let replace_available =
            conflict.destination_snapshot.is_some() && conflict.source != conflict.destination;
        let buttons = vec![
            rmac_ui::dialog_button("conflict-skip", "Skip", rmac_ui::DialogButtonKind::Normal)
                .disabled(busy)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.resolve_current_conflict(ConflictDecision::Skip, cx)
                }))
                .into_any_element(),
            rmac_ui::dialog_button(
                "conflict-replace",
                "Replace",
                rmac_ui::DialogButtonKind::Destructive,
            )
            .busy(busy)
            .disabled(busy || !replace_available)
            .on_click(cx.listener(|this, _, _, cx| {
                this.resolve_current_conflict(ConflictDecision::Replace, cx)
            }))
            .into_any_element(),
            rmac_ui::dialog_button(
                "conflict-keep-both",
                if busy { "Checking…" } else { "Keep Both" },
                rmac_ui::DialogButtonKind::Primary,
            )
            .busy(busy)
            .disabled(busy)
            .on_click(cx.listener(|this, _, _, cx| {
                this.resolve_current_conflict(ConflictDecision::KeepBoth, cx)
            }))
            .into_any_element(),
        ];
        Some(rmac_ui::alert(title, conflict_prompt(conflict), buttons).into_any_element())
    }

    #[cfg(any(target_os = "linux", test))]
    pub(in crate::view) fn render_trash_recovery(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if !self.trash_recovery_open || self.recovery_open {
            return None;
        }
        let review = self.trash_recovery_reviews.first()?;
        let presentation = trash_recovery_presentation(&review.action);
        let title = format!(
            "Recover Trash Operation (1 of {})",
            self.trash_recovery_reviews.len()
        );
        let busy = self.trash_recovery_busy;
        let resolvable = !matches!(
            &review.action,
            trash_store::TrashRecoveryAction::RequiresManualRepair
        );
        let buttons = vec![
            rmac_ui::dialog_button(
                "trash-recovery-later",
                "Later",
                rmac_ui::DialogButtonKind::Normal,
            )
            .disabled(busy)
            .on_click(cx.listener(|this, _, _, cx| this.close_trash_recovery(cx)))
            .into_any_element(),
            rmac_ui::dialog_button(
                "trash-recovery-confirm",
                if busy {
                    "Resolving…"
                } else {
                    presentation.action_label
                },
                rmac_ui::DialogButtonKind::Primary,
            )
            .busy(busy)
            .disabled(busy || !resolvable)
            .on_click(cx.listener(|this, _, _, cx| this.resolve_current_trash_recovery(cx)))
            .into_any_element(),
        ];
        Some(rmac_ui::alert(title, presentation.message, buttons).into_any_element())
    }

    #[cfg(any(target_os = "linux", test))]
    pub(in crate::view) fn render_delete_confirmation(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let confirmation = self.delete_confirmation.as_ref()?;
        let count = confirmation.items.len();
        let name = confirmation
            .items
            .first()
            .and_then(|item| item.original_path.file_name())
            .map(|name| sanitize_dialog_name(&name.to_string_lossy()));
        let (title, message, confirm) = if confirmation.empty_trash {
            let (title, message) = empty_trash_prompt(self.file_words.bin());
            (title, message, format!("Empty {}", self.file_words.bin()))
        } else {
            (
                if count == 1 {
                    "Delete Item Permanently?".to_owned()
                } else {
                    "Delete Items Permanently?".to_owned()
                },
                permanent_delete_prompt(count, name.as_deref()),
                "Delete".to_owned(),
            )
        };
        let buttons = vec![
            rmac_ui::dialog_button(
                "permanent-delete-cancel",
                "Cancel",
                rmac_ui::DialogButtonKind::Normal,
            )
            .on_click(cx.listener(|this, _, _, cx| this.cancel_permanent_delete(cx)))
            .into_any_element(),
            rmac_ui::dialog_button(
                "permanent-delete-confirm",
                confirm,
                rmac_ui::DialogButtonKind::Destructive,
            )
            .on_click(cx.listener(|this, _, _, cx| this.confirm_permanent_delete(cx)))
            .into_any_element(),
        ];
        Some(rmac_ui::alert(title, message, buttons).into_any_element())
    }
}
