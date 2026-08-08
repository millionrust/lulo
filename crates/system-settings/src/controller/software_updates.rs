//! Software Update snapshot, review, installation, progress, and recovery lifecycle.

use super::*;

mod render;

impl Settings {
    pub(super) fn finish_update_status(
        &mut self,
        result: std::result::Result<rmac_updates::Snapshot, rmac_updates::Error>,
    ) {
        self.updates_loading = false;
        self.updates_busy = false;
        match result {
            Ok(snapshot) => {
                self.updates = Some(snapshot);
                self.updates_error = None;
                self.updates_stream_error = None;
            }
            Err(error) => {
                self.updates_error = Some(format!("Could not check for updates: {error}").into());
            }
        }
    }

    pub(super) fn queue_update_stream_refresh(&mut self, cx: &mut Context<Self>) {
        if self.updates_loading || self.updates_busy || self.updates_stream_refreshing {
            self.updates_refresh_pending = true;
            return;
        }
        self.updates_refresh_pending = false;
        self.updates_stream_refreshing = true;
        let generation = self.updates_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = rmac_updates_linux::snapshot(rmac_updates::Request::cached()).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.updates_stream_refreshing = false;
                if update_stream_snapshot_is_current(
                    generation,
                    this.updates_generation,
                    this.updates_loading,
                    this.updates_busy,
                ) {
                    match result {
                        Ok(snapshot) => {
                            this.updates = Some(snapshot);
                            this.updates_error = None;
                            this.updates_stream_error = None;
                            this.updates_plan = None;
                        }
                        Err(error) => {
                            this.updates_stream_error = Some(
                                format!("Could not refresh changed package state: {error}").into(),
                            );
                        }
                    }
                } else {
                    this.updates_refresh_pending = true;
                }
                this.run_pending_update_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn run_pending_update_refresh(&mut self, cx: &mut Context<Self>) {
        if self.updates_refresh_pending
            && !self.updates_loading
            && !self.updates_busy
            && !self.updates_stream_refreshing
        {
            self.queue_update_stream_refresh(cx);
        }
    }

    pub(super) fn refresh_update_status(&mut self, cx: &mut Context<Self>) {
        if self.updates_loading || self.updates_busy {
            return;
        }
        self.updates_busy = true;
        self.updates_generation = self.updates_generation.wrapping_add(1);
        self.updates_error = None;
        self.updates_plan = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = rmac_updates_linux::snapshot(rmac_updates::Request::refresh()).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_update_status(result);
                this.run_pending_update_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn prepare_updates(&mut self, cx: &mut Context<Self>) {
        if self.updates_loading || self.updates_busy {
            return;
        }
        let Some(snapshot) = self.updates.as_ref() else {
            return;
        };
        if !snapshot.can_prepare_install() {
            return;
        }
        let cancellation = rmac_updates::Cancellation::default();
        self.updates_busy = true;
        self.updates_preparing = true;
        self.updates_installing = false;
        self.updates_generation = self.updates_generation.wrapping_add(1);
        self.updates_error = None;
        self.updates_plan = None;
        self.updates_progress = None;
        self.updates_result = None;
        self.updates_cancellation = Some(cancellation.clone());
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = rmac_updates_linux::prepare(cancellation).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.updates_busy = false;
                this.updates_preparing = false;
                this.updates_cancellation = None;
                match result {
                    Ok((snapshot, plan)) => {
                        this.updates = Some(snapshot);
                        this.updates_plan = Some(plan);
                        this.updates_error = None;
                        this.updates_stream_error = None;
                    }
                    Err(error) => {
                        this.updates_error =
                            Some(format!("Could not prepare updates: {error}").into());
                    }
                }
                this.run_pending_update_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn cancel_update_plan(&mut self, cx: &mut Context<Self>) {
        if !self.updates_busy {
            self.updates_plan = None;
            cx.notify();
        }
    }

    pub(super) fn cancel_update_operation(&mut self, cx: &mut Context<Self>) {
        if let Some(cancellation) = &self.updates_cancellation {
            cancellation.cancel();
            cx.notify();
        }
    }

    pub(super) fn confirm_update_plan(&mut self, cx: &mut Context<Self>) {
        if self.updates_busy {
            return;
        }
        let Some(plan) = self.updates_plan.take() else {
            return;
        };
        let cancellation = rmac_updates::Cancellation::default();
        let (progress_sender, progress_receiver) = async_channel::bounded(8);
        self.updates_busy = true;
        self.updates_preparing = false;
        self.updates_installing = true;
        self.updates_generation = self.updates_generation.wrapping_add(1);
        self.updates_error = None;
        self.updates_progress = Some(rmac_updates::InstallProgress::default());
        self.updates_result = None;
        self.updates_cancellation = Some(cancellation.clone());
        cx.notify();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(progress) = progress_receiver.recv().await {
                if this
                    .update(cx, |this: &mut Settings, cx| {
                        if this.updates_installing {
                            this.updates_progress = Some(progress);
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = rmac_updates_linux::install(plan, cancellation, progress_sender).await;
            let recovery = rmac_updates_linux::snapshot(rmac_updates::Request::refresh()).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.updates_busy = false;
                this.updates_installing = false;
                this.updates_cancellation = None;
                match result {
                    Ok(result) => match recovery {
                        Ok(snapshot) => {
                            this.updates_result = Some(result);
                            this.updates = Some(snapshot);
                            this.updates_error = None;
                            this.updates_stream_error = None;
                        }
                        Err(error) => {
                            this.updates_result = None;
                            this.updates = None;
                            this.updates_error = Some(
                                format!(
                                    "Updates were installed, but remaining updates could not be confirmed: {error}"
                                )
                                .into(),
                            );
                        }
                    },
                    Err(error) => match recovery {
                        Ok(snapshot) => {
                            this.updates = Some(snapshot);
                            this.updates_error =
                                Some(format!("Could not install updates: {error}").into());
                            this.updates_stream_error = None;
                        }
                        Err(recovery_error) => {
                            this.updates = None;
                            this.updates_error = Some(
                                format!(
                                    "Could not install updates: {error}. The current package state could not be confirmed: {recovery_error}"
                                )
                                .into(),
                            );
                        }
                    },
                }
                this.run_pending_update_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }
}
