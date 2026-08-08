//! Privacy portal and security-coverage settings authority.

use super::*;

mod render;

impl Settings {
    pub(super) fn finish_privacy_update(
        &mut self,
        result: std::result::Result<rmac_privacy::Snapshot, rmac_privacy_linux::Error>,
    ) {
        self.privacy_loading = false;
        self.privacy_busy = None;
        match result {
            Ok(snapshot) => {
                self.privacy = Some(snapshot);
                self.privacy_error = None;
            }
            Err(error) => {
                self.privacy_error =
                    Some(format!("Could not update portal permissions: {error}").into());
            }
        }
    }
    pub(super) fn queue_privacy_stream_refresh(&mut self, cx: &mut Context<Self>) {
        if self.privacy_loading || self.privacy_busy.is_some() || self.privacy_stream_refreshing {
            self.privacy_refresh_pending = true;
            return;
        }
        self.privacy_refresh_pending = false;
        self.privacy_stream_refreshing = true;
        let generation = self.privacy_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_privacy_linux::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.privacy_stream_refreshing = false;
                if privacy_stream_snapshot_is_current(
                    generation,
                    this.privacy_generation,
                    this.privacy_loading,
                    this.privacy_busy.is_some(),
                ) {
                    match result {
                        Ok(snapshot) => {
                            this.finish_privacy_update(Ok(snapshot));
                            this.privacy_stream_error = None;
                        }
                        Err(error) => this.finish_privacy_update(Err(error)),
                    }
                } else {
                    this.privacy_refresh_pending = true;
                }
                this.run_pending_privacy_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn run_pending_privacy_refresh(&mut self, cx: &mut Context<Self>) {
        if self.privacy_refresh_pending
            && !self.privacy_loading
            && self.privacy_busy.is_none()
            && !self.privacy_stream_refreshing
        {
            self.queue_privacy_stream_refresh(cx);
        }
    }

    pub(super) fn refresh_privacy(&mut self, cx: &mut Context<Self>) {
        if self.privacy_loading || self.privacy_busy.is_some() || self.privacy_stream_refreshing {
            return;
        }
        self.privacy_generation = self.privacy_generation.wrapping_add(1);
        self.privacy_loading = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_privacy_linux::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_privacy_update(result);
                this.run_pending_privacy_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn refresh_security_coverage(&mut self, cx: &mut Context<Self>) {
        if self.security_coverage_loading {
            return;
        }
        self.security_coverage_loading = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let snapshot = cx
                .background_executor()
                .spawn(async { rmac_privacy_linux::security_coverage_snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.security_coverage = Some(snapshot);
                this.security_coverage_loading = false;
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn request_privacy_reset(
        &mut self,
        decision: rmac_privacy::PortalDecision,
        cx: &mut Context<Self>,
    ) {
        if self.privacy_busy.is_none()
            && !self.privacy_loading
            && !self.privacy_stream_refreshing
            && self.privacy.as_ref().is_some_and(|snapshot| {
                snapshot.can_reset && snapshot.decisions.contains(&decision)
            })
        {
            self.privacy_reset_confirmation = Some(decision);
            cx.notify();
        }
    }

    pub(super) fn cancel_privacy_reset(&mut self, cx: &mut Context<Self>) {
        if self.privacy_reset_confirmation.take().is_some() {
            cx.notify();
        }
    }

    pub(super) fn confirm_privacy_reset(&mut self, cx: &mut Context<Self>) {
        if self.privacy_loading || self.privacy_busy.is_some() || self.privacy_stream_refreshing {
            return;
        }
        let Some(decision) = self.privacy_reset_confirmation.take() else {
            return;
        };
        let resource = decision.resource;
        let app_id = decision.app_id.clone();
        self.privacy_generation = self.privacy_generation.wrapping_add(1);
        self.privacy_busy = Some((resource, app_id.clone()));
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_privacy_linux::reset_decision(&decision) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_privacy_update(result);
                this.run_pending_privacy_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }
}
