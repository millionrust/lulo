//! Display confirmation, persistence, primary-output, and rollback lifecycle.

use super::*;

impl Settings {
    pub(in crate::controller) fn begin_display_confirmation(
        &mut self,
        baseline: rmac_display::Snapshot,
        applied: rmac_display::Snapshot,
        cx: &mut Context<Self>,
    ) {
        self.display_generation = self.display_generation.wrapping_add(1);
        let generation = self.display_generation;
        self.display_confirmation = Some(DisplayConfirmation {
            baseline,
            applied,
            generation,
            seconds_remaining: DISPLAY_CONFIRMATION_SECONDS,
        });
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            for remaining in (0..DISPLAY_CONFIRMATION_SECONDS).rev() {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let update = this.update(cx, |this: &mut Settings, cx| {
                    let current = this.display_confirmation.as_ref().is_some_and(|pending| {
                        pending.generation == generation && this.display_generation == generation
                    });
                    if !current {
                        return false;
                    }
                    if remaining == 0 {
                        this.revert_display_change(cx);
                    } else if let Some(pending) = &mut this.display_confirmation {
                        pending.seconds_remaining = remaining;
                        cx.notify();
                    }
                    true
                });
                if !matches!(update, Ok(true)) || remaining == 0 {
                    break;
                }
            }
        })
        .detach();
    }

    pub(in crate::controller) fn keep_display_change(&mut self, cx: &mut Context<Self>) {
        if self.display_loading || self.display_busy || !self.display.can_persist {
            return;
        }
        let Some(pending) = self.display_confirmation.clone() else {
            return;
        };
        let Some(primary) = self
            .display
            .outputs
            .iter()
            .find(|output| output.primary && output.logical.is_some())
            .or_else(|| {
                self.display
                    .outputs
                    .iter()
                    .find(|output| output.logical.is_some())
            })
            .map(|output| output.id.clone())
        else {
            self.display_error = Some("No enabled display can be saved as Main".into());
            cx.notify();
            return;
        };
        let layout = match rmac_display::current_layout(&self.display, &primary) {
            Ok(layout) => layout,
            Err(error) => {
                self.display_error =
                    Some(format!("Could not prepare display layout: {error}").into());
                cx.notify();
                return;
            }
        };
        self.display_generation = self.display_generation.wrapping_add(1);
        self.display_confirmation = None;
        self.display_busy = true;
        self.display_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_display::persist_layout(&layout) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                match result {
                    Ok(snapshot) => {
                        this.finish_display_update(Ok(snapshot));
                        this.flush_display_stream_refresh(cx);
                    }
                    Err(error) => {
                        this.display_busy = false;
                        this.display_error =
                            Some(format!("Could not save display layout: {error}").into());
                        this.begin_display_confirmation(pending.baseline, pending.applied, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::controller) fn set_primary_display(
        &mut self,
        output: String,
        cx: &mut Context<Self>,
    ) {
        if self.display_loading
            || self.display_busy
            || self.display_confirmation.is_some()
            || !self.display.can_persist
        {
            return;
        }
        let layout = match rmac_display::current_layout(&self.display, &output) {
            Ok(layout) => layout,
            Err(error) => {
                self.display_error = Some(format!("Could not select Main display: {error}").into());
                cx.notify();
                return;
            }
        };
        self.display_busy = true;
        self.display_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_display::persist_layout(&layout) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_display_update(result);
                this.flush_display_stream_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::controller) fn revert_display_change(&mut self, cx: &mut Context<Self>) {
        if self.display_loading || self.display_busy || !self.display.can_configure {
            return;
        }
        let Some(pending) = self.display_confirmation.take() else {
            return;
        };
        self.display_generation = self.display_generation.wrapping_add(1);
        self.display_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rmac_display::restore_snapshot(&pending.baseline, &pending.applied)
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_display_update(result);
                this.flush_display_stream_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }
}
