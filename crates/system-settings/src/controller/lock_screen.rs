//! Lock Screen policy and immediate-session-lock lifecycle.

mod render;

use super::*;

impl Settings {
    pub(super) fn finish_lock_policy_update(
        &mut self,
        result: std::result::Result<
            rmac_shortcuts::lock_settings::Snapshot,
            rmac_shortcuts::lock_settings::Error,
        >,
    ) {
        self.lock_policy_loading = false;
        self.lock_policy_busy = false;
        match result {
            Ok(policy) => {
                self.lock_policy = Some(policy);
                self.lock_policy_error = None;
            }
            Err(error) => {
                self.lock_policy_error =
                    Some(format!("Could not update Lock Screen: {error}").into());
            }
        }
    }

    pub(super) fn apply_lock_policy_stream_update(
        &mut self,
        update: std::result::Result<rmac_shortcuts::lock_settings::Snapshot, String>,
    ) {
        self.lock_policy_loading = false;
        match update {
            Ok(policy) => {
                self.lock_policy = Some(policy);
                self.lock_policy_stream_error = None;
            }
            Err(error) => {
                self.lock_policy_stream_error =
                    Some(format!("Live Lock Screen updates unavailable: {error}").into());
            }
        }
    }

    pub(super) fn refresh_lock_policy(&mut self, cx: &mut Context<Self>) {
        if self.lock_policy_loading || self.lock_policy_busy {
            return;
        }
        self.lock_policy_loading = true;
        self.lock_policy_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_shortcuts::lock_settings::settings() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_lock_policy_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn set_lock_after(&mut self, seconds: Option<u32>, cx: &mut Context<Self>) {
        if self.lock_policy_loading || self.lock_policy_busy {
            return;
        }
        self.lock_policy_busy = true;
        self.lock_policy_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_shortcuts::lock_settings::set_lock_after(seconds) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_lock_policy_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn set_suspend_after(&mut self, seconds: Option<u32>, cx: &mut Context<Self>) {
        if self.lock_policy_loading || self.lock_policy_busy {
            return;
        }
        self.lock_policy_busy = true;
        self.lock_policy_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_shortcuts::lock_settings::set_suspend_after(seconds) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_lock_policy_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    /// Lock Screen ▸ Turn display off on battery / on power adapter when
    /// inactive.
    pub(super) fn set_display_off(
        &mut self,
        on_battery: bool,
        seconds: Option<u32>,
        cx: &mut Context<Self>,
    ) {
        if self.lock_policy_loading || self.lock_policy_busy {
            return;
        }
        self.lock_policy_busy = true;
        self.lock_policy_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(move || {
                rmac_shortcuts::lock_settings::set_display_off(on_battery, seconds)
            })
            .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_lock_policy_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    /// Lock Screen ▸ Require password after the display is turned off.
    pub(super) fn set_require_password(&mut self, seconds: Option<u32>, cx: &mut Context<Self>) {
        self.update_lock_policy(cx, move || {
            rmac_shortcuts::lock_settings::set_require_password(seconds)
        });
    }

    /// Battery ▸ Options… ▸ Prevent automatic sleeping on power adapter
    /// when the display is off.
    pub(super) fn set_prevent_sleep_on_adapter(&mut self, prevent: bool, cx: &mut Context<Self>) {
        self.update_lock_policy(cx, move || {
            rmac_shortcuts::lock_settings::set_prevent_sleep_on_adapter(prevent)
        });
    }

    fn update_lock_policy(
        &mut self,
        cx: &mut Context<Self>,
        mutation: impl FnOnce() -> std::result::Result<
                rmac_shortcuts::lock_settings::Snapshot,
                rmac_shortcuts::lock_settings::Error,
            > + Send
            + 'static,
    ) {
        if self.lock_policy_loading || self.lock_policy_busy {
            return;
        }
        self.lock_policy_busy = true;
        self.lock_policy_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(mutation).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_lock_policy_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn set_battery_options_sheet(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.battery_options_sheet != open {
            self.battery_options_sheet = open;
            cx.notify();
        }
    }

    pub(super) fn request_lock_screen(&mut self, cx: &mut Context<Self>) {
        if self.lock_request_busy {
            return;
        }
        self.lock_request_busy = true;
        self.lock_request_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            // `rmac_shortcuts::lock::request()` shells out (`systemctl`,
            // `swaylock`); GPUI's background executor is not safe to spawn
            // child processes from (LINUX-HW-07).
            let result = blocking::unblock(rmac_shortcuts::lock::request).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.lock_request_busy = false;
                this.lock_request_error = result
                    .err()
                    .map(|error| format!("Could not lock this session: {error}").into());
                cx.notify();
            });
        })
        .detach();
    }
}
