//! Date & Time snapshots, timezone mutation, and reviewed manual-clock lifecycle.

use super::*;

mod render;

impl Settings {
    pub(super) fn finish_time_update(
        &mut self,
        result: std::result::Result<rmac_time::Snapshot, rmac_time::Error>,
    ) {
        self.time_loading = false;
        self.time_busy = false;
        match result {
            Ok(snapshot) => {
                if snapshot.ntp_enabled {
                    self.clock_editor = None;
                    self.clock_confirmation = None;
                }
                self.time = Some(snapshot);
                self.time_error = None;
                self.time_stream_error = None;
            }
            Err(error) => {
                self.time_error = Some(format!("Could not update date and time: {error}").into());
            }
        }
    }

    pub(super) fn queue_time_stream_refresh(&mut self, cx: &mut Context<Self>) {
        if self.time_loading || self.time_busy || self.time_stream_refreshing {
            self.time_refresh_pending = true;
            return;
        }
        self.time_refresh_pending = false;
        self.time_stream_refreshing = true;
        let generation = self.time_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_time_linux::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.time_stream_refreshing = false;
                if time_stream_snapshot_is_current(
                    generation,
                    this.time_generation,
                    this.time_loading,
                    this.time_busy,
                ) {
                    match result {
                        Ok(snapshot) => {
                            if snapshot.ntp_enabled {
                                this.clock_editor = None;
                                this.clock_confirmation = None;
                            }
                            this.time = Some(snapshot);
                            this.time_error = None;
                            this.time_stream_error = None;
                        }
                        Err(_) => {
                            this.time_stream_error =
                                Some("Could not refresh the changed date and time state".into());
                        }
                    }
                } else {
                    this.time_refresh_pending = true;
                }
                this.run_pending_time_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn run_pending_time_refresh(&mut self, cx: &mut Context<Self>) {
        if self.time_refresh_pending
            && !self.time_loading
            && !self.time_busy
            && !self.time_stream_refreshing
        {
            self.queue_time_stream_refresh(cx);
        }
    }

    pub(super) fn refresh_time(&mut self, cx: &mut Context<Self>) {
        if self.time_loading || self.time_busy {
            return;
        }
        self.time_busy = true;
        self.time_generation = self.time_generation.wrapping_add(1);
        self.time_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_time_linux::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_time_update(result);
                this.run_pending_time_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn set_automatic_time(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.time_loading || self.time_busy {
            return;
        }
        self.time_busy = true;
        self.time_generation = self.time_generation.wrapping_add(1);
        self.time_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_time_linux::set_ntp(enabled) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_time_update(result);
                this.run_pending_time_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn start_timezone_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.time_busy
            || self.timezone_editor.is_some()
            || self.clock_editor.is_some()
            || self.clock_confirmation.is_some()
        {
            return;
        }
        let Some(snapshot) = &self.time else {
            return;
        };
        let timezone = snapshot.timezone.clone();
        let editor = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(timezone)
                .placeholder("Asia/Kolkata")
        });
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        self.timezone_editor = Some(editor);
        self.time_error = None;
        cx.notify();
    }

    pub(super) fn cancel_timezone_edit(&mut self, cx: &mut Context<Self>) {
        if !self.time_busy {
            self.timezone_editor = None;
            self.time_error = None;
            cx.notify();
        }
    }

    pub(super) fn submit_timezone(&mut self, cx: &mut Context<Self>) {
        if self.time_busy {
            return;
        }
        let (Some(editor), Some(snapshot)) = (&self.timezone_editor, &self.time) else {
            return;
        };
        let timezone = editor.read(cx).value().trim().to_string();
        if let Err(error) = snapshot.validate_timezone(&timezone) {
            self.time_error = Some(error.to_string().into());
            cx.notify();
            return;
        }
        self.time_busy = true;
        self.time_generation = self.time_generation.wrapping_add(1);
        self.time_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_time_linux::set_timezone(&timezone) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if result.is_ok() {
                    this.timezone_editor = None;
                }
                this.finish_time_update(result);
                this.run_pending_time_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn start_clock_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.time_busy
            || self.clock_editor.is_some()
            || self.timezone_editor.is_some()
            || self.clock_confirmation.is_some()
        {
            return;
        }
        let Some(snapshot) = &self.time else {
            return;
        };
        if snapshot.ntp_enabled {
            self.time_error =
                Some("Turn off automatic time before setting the clock manually".into());
            cx.notify();
            return;
        }
        let current = current_system_time_usec().unwrap_or(snapshot.time_usec);
        let Some(value) = snapshot.clock_input_at(current) else {
            self.time_error = Some("The current system time could not be edited safely".into());
            cx.notify();
            return;
        };
        let editor = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(value)
                .placeholder("2026-07-18 11:30:00 +05:30")
        });
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        self.clock_editor = Some(editor);
        self.clock_confirmation = None;
        self.time_error = None;
        cx.notify();
    }

    pub(super) fn cancel_clock_edit(&mut self, cx: &mut Context<Self>) {
        if !self.time_busy {
            self.clock_editor = None;
            self.clock_confirmation = None;
            self.time_error = None;
            cx.notify();
        }
    }

    pub(super) fn cancel_clock_confirmation(&mut self, cx: &mut Context<Self>) {
        if !self.time_busy {
            self.clock_confirmation = None;
            cx.notify();
        }
    }

    pub(super) fn prepare_clock_change(&mut self, cx: &mut Context<Self>) {
        if self.time_busy {
            return;
        }
        let (Some(editor), Some(snapshot)) = (&self.clock_editor, &self.time) else {
            return;
        };
        if snapshot.ntp_enabled {
            self.time_error =
                Some("Turn off automatic time before setting the clock manually".into());
            cx.notify();
            return;
        }
        match rmac_time::ClockTarget::parse(&editor.read(cx).value()) {
            Ok(target) => {
                self.clock_confirmation = Some(target);
                self.time_error = None;
            }
            Err(error) => {
                self.time_error = Some(error.to_string().into());
            }
        }
        cx.notify();
    }

    pub(super) fn confirm_clock_change(&mut self, cx: &mut Context<Self>) {
        if self.time_busy {
            return;
        }
        let Some(target) = self.clock_confirmation.clone() else {
            return;
        };
        self.time_busy = true;
        self.clock_setting = true;
        self.time_generation = self.time_generation.wrapping_add(1);
        self.time_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_time_linux::set_time(&target) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.clock_setting = false;
                this.clock_confirmation = None;
                if result.is_ok() {
                    this.clock_editor = None;
                }
                this.finish_time_update(result);
                this.time_refresh_pending = true;
                this.run_pending_time_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }
}
