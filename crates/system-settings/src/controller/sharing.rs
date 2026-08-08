//! Sharing snapshot and reviewed service-mutation lifecycle.

mod render;

use super::*;

impl Settings {
    pub(super) fn finish_sharing_update(
        &mut self,
        result: std::result::Result<rmac_sharing::Snapshot, rmac_sharing::Error>,
    ) {
        self.sharing_loading = false;
        self.sharing_busy = false;
        match result {
            Ok(snapshot) => {
                self.sharing = Some(snapshot);
                self.sharing_error = None;
            }
            Err(error) => {
                self.sharing_error = Some(format!("Could not update Sharing: {error}").into());
            }
        }
    }

    pub(super) fn refresh_sharing(&mut self, cx: &mut Context<Self>) {
        if self.sharing_loading || self.sharing_busy {
            return;
        }
        self.sharing_busy = true;
        self.sharing_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_sharing_linux::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_sharing_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn confirm_remote_login(&mut self, cx: &mut Context<Self>) {
        if self.sharing_busy {
            return;
        }
        let Some(enabled) = self.sharing_confirmation else {
            return;
        };
        self.sharing_busy = true;
        self.sharing_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_sharing_linux::set_remote_login(enabled) })
                .await;
            let succeeded = result.is_ok();
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if succeeded {
                    this.sharing_confirmation = None;
                }
                this.finish_sharing_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn confirm_file_sharing(&mut self, cx: &mut Context<Self>) {
        if self.sharing_busy {
            return;
        }
        let Some(enabled) = self.file_sharing_confirmation else {
            return;
        };
        self.sharing_busy = true;
        self.sharing_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_sharing_linux::set_file_sharing(enabled) })
                .await;
            let succeeded = result.is_ok();
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if succeeded {
                    this.file_sharing_confirmation = None;
                }
                this.finish_sharing_update(result);
                cx.notify();
            });
        })
        .detach();
    }
}
