//! VPN snapshot, activation, cancellation, and recovery lifecycle.

use super::*;

impl Settings {
    pub(super) fn finish_vpn_update(
        &mut self,
        result: std::result::Result<rmac_network::VpnSnapshot, rmac_network::Error>,
    ) {
        self.vpn_loading = false;
        self.vpn_refreshing = false;
        self.vpn_busy = None;
        self.vpn_cancellation = None;
        match result {
            Ok(snapshot) => {
                self.vpn = snapshot;
                self.vpn_error = None;
                self.vpn_stream_error = None;
            }
            Err(error) => {
                self.vpn_error = Some(format!("Could not update VPN: {error}").into());
            }
        }
    }

    pub(super) fn finish_vpn_stream_update(
        &mut self,
        result: std::result::Result<rmac_network::VpnSnapshot, rmac_network::Error>,
    ) {
        match result {
            Ok(snapshot) => {
                self.vpn = snapshot;
                self.vpn_stream_error = None;
            }
            Err(_) => {
                self.vpn_stream_error =
                    Some("Live VPN state could not be refreshed from NetworkManager".into());
            }
        }
    }

    pub(super) fn refresh_vpn(&mut self, cx: &mut Context<Self>) {
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
        self.vpn_refreshing = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_network::vpn_snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_vpn_update(result);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn set_vpn_enabled(
        &mut self,
        id: rmac_network::VpnProfileId,
        enabled: bool,
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
        self.vpn_busy = Some(id.clone());
        self.vpn_error = None;
        let cancellation = rmac_network::VpnCancellation::new();
        self.vpn_cancellation = enabled.then_some(cancellation.clone());
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let (result, recovery) = cx
                .background_executor()
                .spawn(async move {
                    let result = rmac_network::set_vpn_enabled(&id, enabled, &cancellation);
                    let recovery = result
                        .as_ref()
                        .err()
                        .and_then(|_| rmac_network::vpn_snapshot().ok());
                    (result, recovery)
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_vpn_mutation_update(result, recovery);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn cancel_vpn_activation(&mut self, cx: &mut Context<Self>) {
        if let Some(cancellation) = &self.vpn_cancellation {
            cancellation.cancel();
            cx.notify();
        }
    }

    pub(super) fn finish_vpn_mutation_update(
        &mut self,
        result: std::result::Result<rmac_network::VpnSnapshot, rmac_network::Error>,
        recovery: Option<rmac_network::VpnSnapshot>,
    ) {
        self.vpn_busy = None;
        self.vpn_cancellation = None;
        if let Some(snapshot) = recovery {
            self.vpn = snapshot;
        }
        match result {
            Ok(snapshot) => {
                self.vpn = snapshot;
                self.vpn_error = None;
                self.vpn_stream_error = None;
            }
            Err(error) if error.is_cancelled() => {
                self.vpn_error = None;
            }
            Err(error) => {
                self.vpn_error = Some(format!("Could not update VPN: {error}").into());
            }
        }
    }

    pub(super) fn choose_vpn_import(
        &mut self,
        capability: rmac_network::VpnImportCapabilityId,
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
        self.vpn_import_busy = true;
        self.vpn_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = match rmac_portal::choose_vpn_configuration().await {
                Ok(Some(path)) => {
                    cx.background_executor()
                        .spawn(async move {
                            rmac_network::preview_vpn_import(&capability, &path)
                                .map(Some)
                                .map_err(|error| error.to_string())
                        })
                        .await
                }
                Ok(None) => Ok(None),
                Err(error) => Err(error.to_string()),
            };
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.vpn_import_busy = false;
                match result {
                    Ok(Some(preview)) => {
                        this.vpn_import_preview = Some(preview);
                        this.vpn_error = None;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        this.vpn_error =
                            Some(format!("Could not import VPN configuration: {error}").into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn finish_vpn_import(&mut self, keep: bool, cx: &mut Context<Self>) {
        if self.vpn_import_busy
            || self.vpn_busy.is_some()
            || self.vpn_editor_loading.is_some()
            || self.vpn_editor_busy
            || self.vpn_editor.is_some()
            || self.vpn_delete_preparing.is_some()
            || self.vpn_delete_busy
            || self.vpn_delete_preview.is_some()
        {
            return;
        }
        let Some(preview) = self
            .vpn_import_preview
            .as_ref()
            .map(|preview| preview.id.clone())
        else {
            return;
        };
        self.vpn_generation = self.vpn_generation.wrapping_add(1);
        self.vpn_import_busy = true;
        self.vpn_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_network::finish_vpn_import(&preview, keep) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.vpn_import_busy = false;
                match result {
                    Ok(snapshot) => {
                        this.vpn = snapshot;
                        this.vpn_import_preview = None;
                        this.vpn_error = None;
                        this.vpn_stream_error = None;
                    }
                    Err(error) => {
                        this.vpn_error = Some(
                            format!(
                                "Could not {} VPN import: {error}",
                                if keep { "save" } else { "cancel" }
                            )
                            .into(),
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn start_vpn_edit(
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

    pub(super) fn set_vpn_editor_persistent(&mut self, persistent: bool, cx: &mut Context<Self>) {
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

    pub(super) fn cancel_vpn_edit(&mut self, cx: &mut Context<Self>) {
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

    pub(super) fn submit_vpn_edit(&mut self, cx: &mut Context<Self>) {
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

    pub(super) fn request_vpn_secret_clear(
        &mut self,
        configuration: rmac_network::VpnProfileConfiguration,
        cx: &mut Context<Self>,
    ) {
        if self.vpn_editor.is_none()
            || self.vpn_editor_busy
            || self.vpn_secret_preparing
            || self.vpn_secret_busy
            || self.vpn_secret_preview.is_some()
            || !configuration.supports_vpn_options
        {
            return;
        }
        self.vpn_generation = self.vpn_generation.wrapping_add(1);
        self.vpn_secret_preparing = true;
        self.vpn_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_network::prepare_vpn_secret_clear(&configuration) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.vpn_secret_preparing = false;
                match result {
                    Ok(preview) => {
                        this.vpn_secret_preview = Some(preview);
                        this.vpn_error = None;
                    }
                    Err(error) => {
                        this.vpn_error = Some(
                            format!("Could not prepare saved authentication removal: {error}")
                                .into(),
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn cancel_vpn_secret_clear(&mut self, cx: &mut Context<Self>) {
        if self.vpn_secret_busy || self.vpn_secret_preparing {
            return;
        }
        if self.vpn_secret_preview.take().is_some() {
            self.vpn_error = None;
            cx.notify();
        }
    }

    pub(super) fn confirm_vpn_secret_clear(&mut self, cx: &mut Context<Self>) {
        if self.vpn_secret_busy || self.vpn_secret_preparing {
            return;
        }
        let Some(preview) = self
            .vpn_secret_preview
            .as_ref()
            .map(|preview| preview.id.clone())
        else {
            return;
        };
        self.vpn_generation = self.vpn_generation.wrapping_add(1);
        self.vpn_secret_busy = true;
        self.vpn_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let (result, recovery) = cx
                .background_executor()
                .spawn(async move {
                    let result = rmac_network::clear_vpn_profile_secrets(&preview);
                    let recovery = result
                        .as_ref()
                        .err()
                        .and_then(|_| rmac_network::vpn_snapshot().ok());
                    (result, recovery)
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.vpn_secret_busy = false;
                this.vpn_secret_preview = None;
                if let Some(snapshot) = recovery {
                    this.vpn = snapshot;
                }
                match result {
                    Ok(snapshot) => {
                        this.vpn = snapshot;
                        this.vpn_error = None;
                        this.vpn_stream_error = None;
                    }
                    Err(error) => {
                        let message: SharedString =
                            format!("Could not forget saved VPN authentication: {error}").into();
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

    pub(super) fn request_vpn_delete(
        &mut self,
        id: rmac_network::VpnProfileId,
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
        self.vpn_delete_preparing = Some(id.clone());
        self.vpn_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { rmac_network::prepare_vpn_delete(&id) })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.vpn_delete_preparing = None;
                match result {
                    Ok(preview) => {
                        this.vpn_delete_preview = Some(preview);
                        this.vpn_error = None;
                    }
                    Err(error) => {
                        this.vpn_error =
                            Some(format!("Could not prepare VPN deletion: {error}").into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn cancel_vpn_delete(&mut self, cx: &mut Context<Self>) {
        if self.vpn_delete_busy || self.vpn_delete_preparing.is_some() {
            return;
        }
        if self.vpn_delete_preview.take().is_some() {
            self.refresh_vpn(cx);
        }
    }

    pub(super) fn confirm_vpn_delete(&mut self, cx: &mut Context<Self>) {
        if self.vpn_delete_busy || self.vpn_delete_preparing.is_some() {
            return;
        }
        let Some(preview) = self
            .vpn_delete_preview
            .as_ref()
            .map(|preview| preview.id.clone())
        else {
            return;
        };
        self.vpn_generation = self.vpn_generation.wrapping_add(1);
        self.vpn_delete_busy = true;
        self.vpn_error = None;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let (result, recovery) = cx
                .background_executor()
                .spawn(async move {
                    let result = rmac_network::delete_vpn_profile(&preview);
                    let recovery = result
                        .as_ref()
                        .err()
                        .and_then(|_| rmac_network::vpn_snapshot().ok());
                    (result, recovery)
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.vpn_delete_busy = false;
                this.vpn_delete_preview = None;
                if let Some(snapshot) = recovery {
                    this.vpn = snapshot;
                }
                match result {
                    Ok(snapshot) => {
                        this.vpn = snapshot;
                        this.vpn_error = None;
                        this.vpn_stream_error = None;
                    }
                    Err(error) => {
                        this.vpn_error =
                            Some(format!("Could not delete VPN profile: {error}").into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
