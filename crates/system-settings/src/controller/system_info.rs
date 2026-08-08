//! General/About system snapshot, hostname, accessibility, and diagnostics updates.

use super::*;

mod render;

impl Settings {
    pub(super) fn apply_system_snapshot(&mut self, snapshot: SystemSnapshot) {
        self.account = snapshot.account.into();
        match snapshot.sysinfo {
            Ok(sysinfo) => {
                self.sysinfo = sysinfo;
                self.system_data_error = None;
            }
            Err(error) => {
                self.system_data_error =
                    Some(format!("Could not read system information: {error}").into());
            }
        }
        match snapshot.storage {
            Ok(storage) => {
                self.storage = storage;
                self.storage_error = None;
                self.storage_stream_error = None;
            }
            Err(error) => {
                self.storage_error =
                    Some(format!("Could not read storage volumes: {error}").into());
            }
        }
        self.system_data_loading = false;
    }

    pub(super) fn queue_system_info_stream_refresh(&mut self, cx: &mut Context<Self>) {
        if self.system_data_loading || self.system_data_busy || self.system_data_stream_refreshing {
            self.system_data_refresh_pending = true;
            return;
        }
        self.system_data_refresh_pending = false;
        self.system_data_stream_refreshing = true;
        let generation = self.system_data_generation;
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_system_info::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.system_data_stream_refreshing = false;
                if system_info_stream_snapshot_is_current(
                    generation,
                    this.system_data_generation,
                    this.system_data_loading,
                    this.system_data_busy,
                ) {
                    match result {
                        Ok(snapshot) => {
                            this.sysinfo = snapshot;
                            this.system_data_error = None;
                            this.system_data_stream_error = None;
                            this.diagnostics_copied = false;
                        }
                        Err(error) => {
                            this.system_data_stream_error = Some(
                                format!("Could not refresh changed system information: {error}")
                                    .into(),
                            );
                        }
                    }
                } else {
                    this.system_data_refresh_pending = true;
                }
                this.run_pending_system_info_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn run_pending_system_info_refresh(&mut self, cx: &mut Context<Self>) {
        if self.system_data_refresh_pending
            && !self.system_data_loading
            && !self.system_data_busy
            && !self.system_data_stream_refreshing
        {
            self.queue_system_info_stream_refresh(cx);
        }
    }

    pub(super) fn start_hostname_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.system_data_busy || !self.sysinfo.hostname_mutable {
            return;
        }
        let hostname = self
            .sysinfo
            .static_hostname
            .clone()
            .unwrap_or_else(|| self.sysinfo.hostname.clone());
        let editor = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(hostname)
                .placeholder("studio-pc")
        });
        let focus = editor.read(cx).focus_handle(cx);
        window.focus(&focus);
        self.hostname_editor = Some(editor);
        self.system_data_error = None;
        cx.notify();
    }

    pub(super) fn cancel_hostname_edit(&mut self, cx: &mut Context<Self>) {
        if !self.system_data_busy {
            self.hostname_editor = None;
            self.system_data_error = None;
            cx.notify();
        }
    }

    pub(super) fn submit_hostname(&mut self, cx: &mut Context<Self>) {
        if self.system_data_busy {
            return;
        }
        let Some(editor) = self.hostname_editor.as_ref() else {
            return;
        };
        let hostname = editor.read(cx).value().trim().to_string();
        if let Err(error) = rmac_system_info::validate_static_hostname(&hostname) {
            self.system_data_error = Some(error.to_string().into());
            cx.notify();
            return;
        }
        self.system_data_busy = true;
        self.system_data_generation = self.system_data_generation.wrapping_add(1);
        self.system_data_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_system_info::set_static_hostname(&hostname) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.system_data_busy = false;
                match result {
                    Ok(snapshot) => {
                        this.sysinfo = snapshot;
                        this.hostname_editor = None;
                        this.system_data_error = None;
                        this.system_data_stream_error = None;
                        this.diagnostics_copied = false;
                    }
                    Err(error) => {
                        this.system_data_error = Some(error.to_string().into());
                    }
                }
                this.run_pending_system_info_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn refresh_system_info(&mut self, cx: &mut Context<Self>) {
        if self.system_data_busy {
            return;
        }
        self.system_data_busy = true;
        self.system_data_generation = self.system_data_generation.wrapping_add(1);
        self.system_data_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_system_info::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.system_data_busy = false;
                match result {
                    Ok(snapshot) => {
                        this.sysinfo = snapshot;
                        this.system_data_error = None;
                        this.system_data_stream_error = None;
                        this.diagnostics_copied = false;
                    }
                    Err(error) => {
                        this.system_data_error =
                            Some(format!("Could not refresh system information: {error}").into());
                    }
                }
                this.run_pending_system_info_refresh(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn refresh_screen_reader(&mut self, cx: &mut Context<Self>) {
        if self.screen_reader_loading {
            return;
        }
        self.screen_reader_loading = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let capability = cx
                .background_executor()
                .spawn(async { gather_screen_reader_capability() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.screen_reader = capability;
                this.screen_reader_loading = false;
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn copy_diagnostics(&mut self, cx: &mut Context<Self>) {
        let report = self.sysinfo.diagnostic_report();
        cx.write_to_clipboard(ClipboardItem::new_string(report));
        self.diagnostics_copied = true;
        cx.notify();
    }
}
