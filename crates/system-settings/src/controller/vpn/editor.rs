//! VPN profile editing and persistence lifecycle.

use super::*;

impl Settings {
    pub(in crate::controller) fn start_vpn_edit(
        &mut self,
        id: rmac_network::VpnProfileId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.vpn_loading
            || self.vpn_refreshing
            || self.vpn_busy.is_some()
            || self.vpn_import_busy
            || self.vpn_import_preview.is_some()
            || self.vpn_editor_loading.is_some()
            || self.vpn_editor_busy
            || self.vpn_editor.is_some()
            || self.vpn_delete_preparing.is_some()
            || self.vpn_delete_busy
            || self.vpn_delete_preview.is_some()
        {
            return;
        }
        self.vpn_generation = self.vpn_generation.wrapping_add(1);
        self.vpn_editor_loading = Some(id.clone());
        self.vpn_error = None;
        let window_handle = window.window_handle();
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_network::vpn_profile_configuration(&id) })
                .await;
            let _ = cx.update_window(window_handle, |_, window, cx| {
                let _ = this.update(cx, |this: &mut Settings, cx| {
                    this.vpn_editor_loading = None;
                    match result {
                        Ok(configuration) => {
                            let name = cx.new(|cx| {
                                InputState::new(window, cx)
                                    .default_value(configuration.name.clone())
                                    .placeholder("VPN connection name")
                            });
                            let username = cx.new(|cx| {
                                InputState::new(window, cx)
                                    .default_value(configuration.username.clone())
                                    .placeholder("Optional account name")
                            });
                            let timeout = cx.new(|cx| {
                                InputState::new(window, cx)
                                    .default_value(configuration.timeout.to_string())
                                    .placeholder("0")
                            });
                            this.vpn_editor = Some(VpnEditorState {
                                persistent: configuration.persistent,
                                configuration,
                                name,
                                username,
                                timeout,
                                validation_error: None,
                            });
                        }
                        Err(error) => {
                            this.vpn_error =
                                Some(format!("Could not open VPN details: {error}").into());
                        }
                    }
                    cx.notify();
                });
            });
        })
        .detach();
    }

    pub(in crate::controller) fn set_vpn_editor_persistent(
        &mut self,
        persistent: bool,
        cx: &mut Context<Self>,
    ) {
        if self.vpn_editor_busy
            || self.vpn_secret_preparing
            || self.vpn_secret_busy
            || self.vpn_secret_preview.is_some()
        {
            return;
        }
        if let Some(editor) = &mut self.vpn_editor {
            editor.persistent = persistent;
            editor.validation_error = None;
            cx.notify();
        }
    }

    pub(in crate::controller) fn cancel_vpn_edit(&mut self, cx: &mut Context<Self>) {
        if !self.vpn_editor_busy
            && self.vpn_editor_loading.is_none()
            && !self.vpn_secret_preparing
            && !self.vpn_secret_busy
            && self.vpn_secret_preview.is_none()
        {
            self.vpn_editor = None;
            self.vpn_error = None;
            cx.notify();
        }
    }

    pub(in crate::controller) fn submit_vpn_edit(&mut self, cx: &mut Context<Self>) {
        if self.vpn_editor_busy
            || self.vpn_editor_loading.is_some()
            || self.vpn_secret_preparing
            || self.vpn_secret_busy
            || self.vpn_secret_preview.is_some()
        {
            return;
        }
        let Some(editor) = &self.vpn_editor else {
            return;
        };
        let name = editor.name.read(cx).value();
        let username = editor.username.read(cx).value();
        let timeout_text = editor.timeout.read(cx).value();
        let timeout = match timeout_text.trim().parse::<u32>() {
            Ok(timeout) => timeout,
            Err(_) => {
                if let Some(editor) = &mut self.vpn_editor {
                    editor.validation_error = Some(
                        "Enter a whole connection timeout from 0 to 4294967295 seconds".into(),
                    );
                }
                cx.notify();
                return;
            }
        };
        let edit = match rmac_network::VpnProfileEdit::new(
            &editor.configuration,
            &name,
            &username,
            editor.persistent,
            timeout,
        ) {
            Ok(edit) => edit,
            Err(error) => {
                if let Some(editor) = &mut self.vpn_editor {
                    editor.validation_error = Some(error.to_string().into());
                }
                cx.notify();
                return;
            }
        };
        if edit.is_unchanged(&editor.configuration) {
            self.vpn_editor = None;
            cx.notify();
            return;
        }
        self.vpn_generation = self.vpn_generation.wrapping_add(1);
        self.vpn_editor_busy = true;
        self.vpn_error = None;
        if let Some(editor) = &mut self.vpn_editor {
            editor.validation_error = None;
        }
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let (result, recovery) = cx
                .background_executor()
                .spawn(async move {
                    let result = rmac_network::update_vpn_profile(&edit);
                    let recovery = result
                        .as_ref()
                        .err()
                        .and_then(|_| rmac_network::vpn_snapshot().ok());
                    (result, recovery)
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.vpn_editor_busy = false;
                if let Some(snapshot) = recovery {
                    this.vpn = snapshot;
                }
                match result {
                    Ok(snapshot) => {
                        this.vpn = snapshot;
                        this.vpn_editor = None;
                        this.vpn_error = None;
                        this.vpn_stream_error = None;
                    }
                    Err(error) => {
                        let message: SharedString =
                            format!("Could not save VPN details: {error}").into();
                        this.vpn_error = Some(message.clone());
                        if let Some(editor) = &mut this.vpn_editor {
                            editor.validation_error = Some(message);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
