use super::*;

impl NotesView {
    pub(super) fn render_migration_review(
        &self,
        review: &rmac_notes_runtime::MigrationReviewSummary,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(mac::window())
            .child(
                div()
                    .w(px(460.0))
                    .p_6()
                    .v_flex()
                    .gap_3()
                    .rounded(px(rmac_ui::mac::radius_card()))
                    .bg(mac::raised())
                    .border_1()
                    .border_color(mac::separator())
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(20.0))
                            .font_weight(mac::BOLD)
                            .child("Bring your existing notes into rmac Notes?"),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(13.0))
                            .text_color(mac::text_secondary())
                            .child(format!(
                                "Notes found {} notes in {} folders, with {} managed attachments and {} recovery files. The source stays untouched.",
                                review.notes,
                                review.folders,
                                review.managed_attachments,
                                review.recovery_files
                            )),
                    )
                    .when(!review.warnings.is_empty(), |element| {
                        element.child(
                            div()
                                .text_size(rmac_ui::text_px(12.0))
                                .text_color(mac::warning_text())
                                .child(format!(
                                    "{} item{} need recovery attention after import.",
                                    review.warnings.len(),
                                    if review.warnings.len() == 1 { "" } else { "s" }
                                )),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                Button::new("start-empty", "Not Now")
                                    .on_click(cx.listener(|this, _, _, cx| this.start_empty(cx))),
                            )
                            .child(
                                Button::new("accept-migration", "Import Notes")
                                    .primary()
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.accept_migration(cx)),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn render_draft_review(
        &self,
        review: &rmac_notes_runtime::DraftReviewSummary,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let busy = self.recovery_decision.is_some() || self.recovery_copy_pending.is_some();
        let warning_count = review
            .malformed
            .saturating_add(review.quarantined)
            .saturating_add(review.cleanup_pending);
        let mut card = div()
            .w(px(500.0))
            .p_6()
            .v_flex()
            .gap_3()
            .rounded(px(rmac_ui::mac::radius_card()))
            .bg(mac::raised())
            .border_1()
            .border_color(mac::separator());

        if let Some(draft) = review.drafts.first() {
            let note_id = draft.note_id;
            let note_title = self
                .session
                .snapshot()
                .and_then(|snapshot| snapshot.notes.iter().find(|note| note.id == note_id))
                .map(|note| display_title(&note.title))
                .unwrap_or_else(|| "Deleted or unavailable note".into());
            let (heading, detail, decision, action_label) = match draft.kind {
                DraftRecoveryKind::Applicable => (
                    "Recover unsaved changes?",
                    "This recovery copy matches the durable note and can be restored safely.",
                    RecoveryDecision::RestoreOriginal,
                    "Restore",
                ),
                DraftRecoveryKind::Conflict => (
                    "Keep both versions?",
                    "The durable note changed after this recovery copy was written. Preserve the recovered text as a new note to avoid overwriting either version.",
                    RecoveryDecision::PreserveCopy,
                    "Keep as New Note",
                ),
                DraftRecoveryKind::Orphaned => (
                    "Preserve recovered text?",
                    "The original note is no longer available. Preserve this recovery copy as a new note before continuing.",
                    RecoveryDecision::PreserveCopy,
                    "Keep as New Note",
                ),
            };
            card = card
                .child(
                    div()
                        .text_size(rmac_ui::text_px(20.0))
                        .font_weight(mac::BOLD)
                        .child(heading),
                )
                .child(
                    div()
                        .text_size(rmac_ui::text_px(14.0))
                        .font_weight(mac::SEMIBOLD)
                        .child(note_title),
                )
                .child(
                    div()
                        .text_size(rmac_ui::text_px(13.0))
                        .text_color(mac::text_secondary())
                        .child(detail),
                )
                .child(
                    div()
                        .text_size(rmac_ui::text_px(11.0))
                        .text_color(mac::text_tertiary())
                        .child(format!(
                            "{} recovery {} remaining",
                            review.drafts.len(),
                            if review.drafts.len() == 1 {
                                "copy"
                            } else {
                                "copies"
                            }
                        )),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("close-recovery", "Review Later")
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.request_close(window, cx)
                                })),
                        )
                        .child(
                            Button::new("discard-recovery", "Discard Recovery")
                                .destructive()
                                .disabled(busy)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.discard_draft(note_id, cx)
                                })),
                        )
                        .child(
                            Button::new("restore-recovery", action_label)
                                .primary()
                                .busy(busy)
                                .disabled(busy)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.restore_draft(note_id, decision, cx)
                                })),
                        ),
                );
        } else {
            let cannot_continue = review.unavailable || review.excessive;
            let detail = if cannot_continue {
                "Notes could not enumerate every recovery record safely. The records remain untouched; close Notes and resolve the storage problem before editing."
            } else {
                "No recoverable note text remains. Any malformed records were isolated and will not be treated as valid note content."
            };
            card = card
                .child(
                    div()
                        .text_size(rmac_ui::text_px(20.0))
                        .font_weight(mac::BOLD)
                        .child(if cannot_continue {
                            "Recovery needs attention"
                        } else {
                            "Recovery review complete"
                        }),
                )
                .child(
                    div()
                        .text_size(rmac_ui::text_px(13.0))
                        .text_color(mac::text_secondary())
                        .child(detail),
                )
                .when(warning_count != 0, |element| {
                    element.child(
                        div()
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(mac::warning_text())
                            .child(format!(
                                "{warning_count} recovery record operation{} reported attention.",
                                if warning_count == 1 { "" } else { "s" }
                            )),
                    )
                })
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("close-recovery-notice", "Close Notes").on_click(
                                cx.listener(|this, _, window, cx| this.request_close(window, cx)),
                            ),
                        )
                        .when(!cannot_continue, |element| {
                            element.child(
                                Button::new("continue-recovery", "Continue")
                                    .primary()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.continue_after_recovery_notice(cx)
                                    })),
                            )
                        }),
                );
        }

        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(mac::window())
            .child(card)
            .into_any_element()
    }
}
