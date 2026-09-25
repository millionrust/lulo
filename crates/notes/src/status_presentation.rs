use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BannerActions {
    Pending,
    OrphanCleanup,
    UndoTrash,
}

impl NotesView {
    pub(super) fn render_status_banner_with(
        &self,
        pending_override: Option<AnyElement>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let orphan_waiting =
            !self.attachment_action_pending() && self.first_orphaned_attachment().is_some();
        let (message, actions) = match self.session.phase() {
            SessionPhase::Pending { reason, .. } => {
                (pending_message(*reason), Some(BannerActions::Pending))
            }
            SessionPhase::Maintenance { .. } => (
                "Notes recovered the library but maintenance still needs attention. Editing is paused."
                    .to_string(),
                None,
            ),
            _ => match &self.message {
                Some(message) => (
                    message.to_string(),
                    if self.pending_undo_trash.is_some() {
                        Some(BannerActions::UndoTrash)
                    } else {
                        orphan_waiting.then_some(BannerActions::OrphanCleanup)
                    },
                ),
                None if orphan_waiting => (
                    "A removed photo is still stored until its managed copy is cleaned up."
                        .to_string(),
                    Some(BannerActions::OrphanCleanup),
                ),
                None => return None,
            },
        };
        let mut banner = div()
            .h(px(38.0))
            .flex_none()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .bg(mac::error_background())
            .border_b_1()
            .border_color(mac::error_border())
            .text_size(rmac_ui::text_px(12.0))
            .text_color(mac::danger())
            .child(div().flex_1().child(message));
        match actions {
            Some(BannerActions::Pending) => {
                if let Some(action) = pending_override {
                    banner = banner.child(action);
                } else {
                    banner = banner
                        .child(
                            Button::new("retry-pending", "Retry")
                                .xsmall()
                                .on_click(cx.listener(|this, _, _, cx| this.retry_pending(cx))),
                        )
                        .child(
                            Button::new("discard-pending", "Discard")
                                .xsmall()
                                .on_click(cx.listener(|this, _, _, cx| this.discard_pending(cx))),
                        );
                }
            }
            Some(BannerActions::OrphanCleanup) => {
                banner = banner.child(
                    Button::new("review-orphan-cleanup", "Clean Up…")
                        .xsmall()
                        .on_click(cx.listener(|this, _, _, cx| this.begin_orphan_cleanup(cx))),
                );
            }
            Some(BannerActions::UndoTrash) => {
                banner = banner.child(
                    Button::new("undo-trash-note", "Undo")
                        .xsmall()
                        .on_click(cx.listener(|this, _, _, cx| this.undo_delete_note(cx))),
                );
            }
            None => {}
        }
        Some(banner.into_any_element())
    }
}
