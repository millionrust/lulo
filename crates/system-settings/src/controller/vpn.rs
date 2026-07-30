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
    pub(super) fn render_vpn(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let connected = self
            .vpn
            .profiles
            .iter()
            .filter(|profile| profile.state == rmac_network::VpnState::Connected)
            .count();
        let status = match connected {
            0 => "No VPN Connected".to_string(),
            1 => "1 VPN Connected".to_string(),
            count => format!("{count} VPNs Connected"),
        };
        let refresh_view = view.clone();
        let refresh_label = if self.vpn_delete_busy || self.vpn_delete_preparing.is_some() {
            "Deleting…"
        } else if self.vpn_secret_busy {
            "Forgetting…"
        } else if self.vpn_secret_preparing {
            "Preparing…"
        } else if self.vpn_editor_busy {
            "Saving…"
        } else if self.vpn_editor_loading.is_some() {
            "Opening…"
        } else if self.vpn_editor.is_some() {
            "Editing…"
        } else if self.vpn_import_busy {
            "Importing…"
        } else if self.vpn_busy.is_some() {
            "Updating…"
        } else if self.vpn_refreshing {
            "Refreshing…"
        } else {
            "Refresh"
        };
        let mut cards = vec![card(vec![value_row(
            "icons/key.svg",
            if connected > 0 {
                hsl(0x34c759)
            } else {
                secondary()
            },
            "Status".into(),
            status.into(),
        )])];
        cards.push(
            div()
                .flex()
                .items_center()
                .justify_between()
                .px_1()
                .pt_2()
                .pb_1()
                .child(
                    div()
                        .text_size(rmac_ui::text_px(12.0))
                        .font_weight(rmac_ui::mac::SEMIBOLD)
                        .text_color(secondary())
                        .child("VPN Configurations"),
                )
                .child(
                    div()
                        .id("vpn-refresh")
                        .px_2()
                        .py_1()
                        .rounded(px(6.0))
                        .text_size(rmac_ui::text_px(12.0))
                        .text_color(accent())
                        .cursor_pointer()
                        .hover(|hover| hover.bg(rmac_ui::mac::hover()))
                        .child(refresh_label)
                        .on_click(move |_, _, cx| {
                            refresh_view.update(cx, |settings, cx| settings.refresh_vpn(cx));
                        }),
                ),
        );
        if self.vpn_loading {
            cards.push(note_card("Loading VPN configurations from the system…"));
            return self.pane(cards);
        }
        if !self.vpn.available {
            cards.push(note_card(
                "The system VPN service is not available on this computer.",
            ));
            return self.pane(cards);
        }
        if self.vpn_editor_loading.is_some() {
            cards.push(note_card("Opening current VPN profile details…"));
        }
        if self.vpn_editor.is_some() {
            cards.extend(self.render_vpn_editor(cx));
        }
        if self.vpn.profiles.is_empty() {
            cards.push(note_card(
                "No VPN configurations are installed. Import a configuration below.",
            ));
        } else {
            let rows = self
                .vpn
                .profiles
                .iter()
                .enumerate()
                .map(|(index, profile)| {
                    let id = profile.id.clone();
                    let switch_id = id.clone();
                    let profile_view = view.clone();
                    let delete_id = id.clone();
                    let delete_view = view.clone();
                    let edit_id = id.clone();
                    let edit_view = view.clone();
                    let applying = self.vpn_busy.as_ref() == Some(&id);
                    let connecting = applying && self.vpn_cancellation.is_some();
                    let preparing_delete = self.vpn_delete_preparing.as_ref() == Some(&id);
                    let subtitle = if connecting {
                        format!("{} · Connecting…", profile.service)
                    } else if applying {
                        format!("{} · Disconnecting…", profile.service)
                    } else {
                        format!("{} · {}", profile.service, profile.state.label())
                    };
                    let control = if connecting {
                        Button::new(("vpn-stop", index), "Stop")
                            .on_click(move |_, _, cx| {
                                profile_view
                                    .update(cx, |settings, cx| settings.cancel_vpn_activation(cx));
                            })
                            .into_any_element()
                    } else {
                        Toggle::new(ElementId::from(SharedString::from(format!(
                            "vpn-profile-{index}"
                        ))))
                        .checked(profile.state.is_enabled())
                        .disabled(
                            self.vpn_busy.is_some()
                                || self.vpn_refreshing
                                || self.vpn_import_busy
                                || self.vpn_import_preview.is_some()
                                || self.vpn_editor_loading.is_some()
                                || self.vpn_editor_busy
                                || self.vpn_editor.is_some()
                                || self.vpn_delete_preparing.is_some()
                                || self.vpn_delete_busy
                                || self.vpn_delete_preview.is_some(),
                        )
                        .on_click(move |enabled, _, cx| {
                            let id = switch_id.clone();
                            profile_view.update(cx, |settings, cx| {
                                settings.set_vpn_enabled(id, *enabled, cx)
                            });
                        })
                        .into_any_element()
                    };
                    let controls = div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .when(cfg!(target_os = "linux"), |controls| {
                            controls
                                .child(
                                    Button::new(
                                        ("vpn-edit", index),
                                        if self.vpn_editor_loading.as_ref() == Some(&id) {
                                            "Opening…"
                                        } else {
                                            "Details…"
                                        },
                                    )
                                    .disabled(
                                        self.vpn_busy.is_some()
                                            || self.vpn_refreshing
                                            || self.vpn_import_busy
                                            || self.vpn_import_preview.is_some()
                                            || self.vpn_editor_loading.is_some()
                                            || self.vpn_editor_busy
                                            || self.vpn_editor.is_some()
                                            || self.vpn_delete_preparing.is_some()
                                            || self.vpn_delete_busy
                                            || self.vpn_delete_preview.is_some(),
                                    )
                                    .on_click(
                                        move |_, window, cx| {
                                            let id = edit_id.clone();
                                            edit_view.update(cx, |settings, cx| {
                                                settings.start_vpn_edit(id, window, cx)
                                            });
                                        },
                                    ),
                                )
                                .child(
                                    Button::new(
                                        ("vpn-delete", index),
                                        if preparing_delete {
                                            "Preparing…"
                                        } else {
                                            "Delete…"
                                        },
                                    )
                                    .disabled(
                                        self.vpn_busy.is_some()
                                            || self.vpn_refreshing
                                            || self.vpn_import_busy
                                            || self.vpn_import_preview.is_some()
                                            || self.vpn_editor_loading.is_some()
                                            || self.vpn_editor_busy
                                            || self.vpn_editor.is_some()
                                            || self.vpn_delete_preparing.is_some()
                                            || self.vpn_delete_busy
                                            || self.vpn_delete_preview.is_some(),
                                    )
                                    .on_click(
                                        move |_, _, cx| {
                                            let id = delete_id.clone();
                                            delete_view.update(cx, |settings, cx| {
                                                settings.request_vpn_delete(id, cx)
                                            });
                                        },
                                    ),
                                )
                        })
                        .child(control);
                    row_base()
                        .child(tile(
                            "icons/key.svg",
                            if profile.state == rmac_network::VpnState::Connected {
                                hsl(0x34c759)
                            } else {
                                accent()
                            },
                            22.0,
                        ))
                        .child(text_block(
                            profile.name.clone().into(),
                            Some(subtitle.into()),
                        ))
                        .child(controls)
                        .into_any_element()
                })
                .collect();
            cards.push(card(rows));
        }
        cards.push(section_header("Import Configuration"));
        if self.vpn_import_loading {
            cards.push(note_card("Checking installed VPN importers…"));
        } else if !self.vpn_import_capabilities.available {
            cards.push(note_card(
                self.vpn_import_capabilities
                    .limitation
                    .clone()
                    .unwrap_or_else(|| "VPN import is unavailable on this system".to_string()),
            ));
        } else if self.vpn_import_capabilities.plugins.is_empty() {
            cards.push(note_card(
                "No reviewed import-capable NetworkManager VPN plugins are installed.",
            ));
        } else {
            let import_rows = self
                .vpn_import_capabilities
                .plugins
                .iter()
                .enumerate()
                .map(|(index, capability)| {
                    let capability_id = capability.id.clone();
                    let import_view = view.clone();
                    row_base()
                        .child(tile("icons/key.svg", accent(), 20.0))
                        .child(text_block(
                            capability.name.clone().into(),
                            Some(capability.format_hint.clone().into()),
                        ))
                        .child(
                            Button::new(("vpn-import", index), "Import…")
                                .disabled(
                                    self.vpn_import_busy
                                        || self.vpn_busy.is_some()
                                        || self.vpn_import_preview.is_some()
                                        || self.vpn_editor_loading.is_some()
                                        || self.vpn_editor_busy
                                        || self.vpn_editor.is_some()
                                        || self.vpn_delete_preparing.is_some()
                                        || self.vpn_delete_busy
                                        || self.vpn_delete_preview.is_some(),
                                )
                                .on_click(move |_, _, cx| {
                                    let capability = capability_id.clone();
                                    import_view.update(cx, |settings, cx| {
                                        settings.choose_vpn_import(capability, cx)
                                    });
                                }),
                        )
                        .into_any_element()
                })
                .collect();
            cards.push(card(import_rows));
            if let Some(limitation) = &self.vpn_import_capabilities.limitation {
                cards.push(note_card(limitation.clone()));
            }
        }
        cards.push(note_card(
            "Connections are controlled by the system network service. Authentication prompts are handled by the installed VPN plugin.",
        ));
        self.pane(cards)
    }

    pub(super) fn render_vpn_editor(&self, cx: &Context<Self>) -> Vec<Div> {
        let Some(editor) = &self.vpn_editor else {
            return Vec::new();
        };
        let busy = self.vpn_editor_busy;
        let locked = busy
            || self.vpn_secret_preparing
            || self.vpn_secret_busy
            || self.vpn_secret_preview.is_some();
        let view = cx.entity();
        let persistent_view = view.clone();
        let authentication_view = view.clone();
        let authentication_configuration = editor.configuration.clone();
        let cancel_view = view.clone();
        let save_view = view.clone();
        let mut rows = vec![network_field_row(
            "Name",
            "Shown in VPN lists and connection menus",
            &editor.name,
            !locked,
        )];
        if editor.configuration.supports_vpn_options {
            rows.extend([
                network_field_row(
                    "Account Name",
                    "Optional non-secret username; passwords are not read here",
                    &editor.username,
                    !locked,
                ),
                row_base()
                    .child(text_block(
                        "Keep Connection".into(),
                        Some("Ask the VPN plugin to maintain the tunnel when supported".into()),
                    ))
                    .child(
                        Toggle::new("vpn-edit-persistent")
                            .checked(editor.persistent)
                            .disabled(locked)
                            .on_click(move |persistent, _, cx| {
                                persistent_view.update(cx, |settings, cx| {
                                    settings.set_vpn_editor_persistent(*persistent, cx)
                                });
                            }),
                    )
                    .into_any_element(),
                network_field_row(
                    "Connection Timeout",
                    "Seconds; 0 uses the VPN plugin default",
                    &editor.timeout,
                    !locked,
                ),
            ]);
        }
        if editor.configuration.supports_vpn_options {
            rows.push(
                row_base()
                    .child(tile("icons/key.svg", secondary(), 22.0))
                    .child(text_block(
                        "Saved Authentication".into(),
                        Some(
                            "Passwords, passphrases, and plugin tokens managed by NetworkManager"
                                .into(),
                        ),
                    ))
                    .child(
                        Button::new(
                            "vpn-forget-authentication",
                            if self.vpn_secret_preparing {
                                "Preparing…"
                            } else {
                                "Forget…"
                            },
                        )
                        .disabled(locked)
                        .on_click(move |_, _, cx| {
                            let configuration = authentication_configuration.clone();
                            authentication_view.update(cx, |settings, cx| {
                                settings.request_vpn_secret_clear(configuration, cx)
                            });
                        }),
                    )
                    .into_any_element(),
            );
        } else {
            rows.push(value_row(
                "icons/key.svg",
                secondary(),
                "Private Key".into(),
                "Never cleared here".into(),
            ));
        }
        let mut sections = vec![
            section_header(format!("{} · Details", editor.configuration.name)),
            note_card(
                "This editor changes only typed, non-secret fields. NetworkManager and the installed VPN plugin keep the complete plugin configuration, passwords, certificates, and private keys unchanged.",
            ),
            card(rows),
        ];
        if !editor.configuration.supports_vpn_options {
            sections.push(note_card(format!(
                "{} profiles can be renamed here. Their protocol-specific configuration remains under NetworkManager's authority.",
                editor.configuration.service
            )));
        }
        if let Some(error) = &editor.validation_error {
            sections.push(note_card(error.clone()));
        }
        if busy {
            sections.push(
                div()
                    .mb_2()
                    .child(Progress::indeterminate().label("Saving VPN details…")),
            );
        }
        sections.push(
            div()
                .flex()
                .justify_end()
                .items_center()
                .gap_2()
                .mb_3()
                .child(
                    Button::new("vpn-edit-cancel", "Cancel")
                        .disabled(locked)
                        .on_click(move |_, _, cx| {
                            cancel_view.update(cx, |settings, cx| settings.cancel_vpn_edit(cx));
                        }),
                )
                .child(
                    Button::new("vpn-edit-save", if busy { "Saving…" } else { "Save" })
                        .primary()
                        .disabled(locked)
                        .on_click(move |_, _, cx| {
                            save_view.update(cx, |settings, cx| settings.submit_vpn_edit(cx));
                        }),
                ),
        );
        sections
    }

    pub(super) fn render_vpn_secret_clear_dialog(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let preview = self.vpn_secret_preview.as_ref()?;
        let busy = self.vpn_secret_busy;
        let consequence = if preview.currently_connected {
            "The current connection will stay active. NetworkManager will remove saved passwords, certificate passphrases, proxy passwords, and plugin tokens, then the installed VPN plugin may ask for them after the next disconnect. This cannot be undone by rmac."
        } else {
            "NetworkManager will remove saved passwords, certificate passphrases, proxy passwords, and plugin tokens. The installed VPN plugin may ask for them on the next connection. This cannot be undone by rmac."
        };
        let content = div()
            .w(px(440.0))
            .v_flex()
            .gap_4()
            .p_5()
            .rounded(px(14.0))
            .border_1()
            .border_color(rmac_ui::mac::separator())
            .shadow_xl()
            .bg(rmac_ui::mac::raised())
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(17.0))
                            .font_weight(rmac_ui::mac::SEMIBOLD)
                            .text_color(label())
                            .child(format!("Forget authentication for “{}”?", preview.name)),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(secondary())
                            .child(preview.service.clone()),
                    ),
            )
            .child(note_card(consequence))
            .child(note_card(
                "The VPN profile, server configuration, certificates, and current tunnel are not deleted. Native WireGuard private keys are never handled by this action.",
            ))
            .when(busy, |dialog| {
                dialog.child(Progress::indeterminate().label("Forgetting saved authentication…"))
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        rmac_ui::dialog_button(
                            "vpn-secret-clear-cancel",
                            "Cancel",
                            rmac_ui::DialogButtonKind::Normal,
                        )
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.cancel_vpn_secret_clear(cx)
                        })),
                    )
                    .child(
                        rmac_ui::dialog_button(
                            "vpn-secret-clear-confirm",
                            "Forget",
                            rmac_ui::DialogButtonKind::Destructive,
                        )
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.confirm_vpn_secret_clear(cx)
                        })),
                    ),
            );
        Some(
            rmac_ui::dialog("vpn-secret-clear-dialog", content)
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" if !this.vpn_secret_busy => {
                            cx.stop_propagation();
                            this.cancel_vpn_secret_clear(cx);
                        }
                        "enter" if !this.vpn_secret_busy => {
                            cx.stop_propagation();
                            this.confirm_vpn_secret_clear(cx);
                        }
                        _ => {}
                    }
                }))
                .into_any_element(),
        )
    }

    pub(super) fn render_vpn_import_dialog(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let preview = self.vpn_import_preview.as_ref()?;
        let busy = self.vpn_import_busy;
        let content = div()
            .w(px(420.0))
            .v_flex()
            .gap_4()
            .p_5()
            .rounded(px(14.0))
            .border_1()
            .border_color(rmac_ui::mac::separator())
            .shadow_xl()
            .bg(rmac_ui::mac::raised())
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(17.0))
                            .font_weight(rmac_ui::mac::SEMIBOLD)
                            .text_color(label())
                            .child(format!("Import “{}”?", preview.name)),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(secondary())
                            .child(format!(
                                "{} recognized “{}”.",
                                preview.service, preview.source_name
                            )),
                    ),
            )
            .child(note_card(
                "The profile is temporary and cannot connect automatically. Import saves it to NetworkManager; passwords and keys remain under NetworkManager and the VPN plugin’s authority.",
            ))
            .when(busy, |dialog| {
                dialog.child(Progress::indeterminate().label("Finishing VPN import…"))
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        rmac_ui::dialog_button(
                            "vpn-import-cancel",
                            "Cancel",
                            rmac_ui::DialogButtonKind::Normal,
                        )
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.finish_vpn_import(false, cx)
                        })),
                    )
                    .child(
                        rmac_ui::dialog_button(
                            "vpn-import-confirm",
                            "Import",
                            rmac_ui::DialogButtonKind::Primary,
                        )
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.finish_vpn_import(true, cx)
                        })),
                    ),
            );
        Some(
            rmac_ui::dialog("vpn-import-dialog", content)
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" if !this.vpn_import_busy => {
                            cx.stop_propagation();
                            this.finish_vpn_import(false, cx);
                        }
                        "enter" if !this.vpn_import_busy => {
                            cx.stop_propagation();
                            this.finish_vpn_import(true, cx);
                        }
                        _ => {}
                    }
                }))
                .into_any_element(),
        )
    }

    pub(super) fn render_vpn_delete_dialog(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let preview = self.vpn_delete_preview.as_ref()?;
        let busy = self.vpn_delete_busy;
        let consequence = if preview.will_disconnect {
            "The active connection will disconnect first. The saved profile and its NetworkManager-managed secrets will then be removed. This cannot be undone."
        } else {
            "The saved profile and its NetworkManager-managed secrets will be removed. This cannot be undone."
        };
        let content = div()
            .w(px(420.0))
            .v_flex()
            .gap_4()
            .p_5()
            .rounded(px(14.0))
            .border_1()
            .border_color(rmac_ui::mac::separator())
            .shadow_xl()
            .bg(rmac_ui::mac::raised())
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(17.0))
                            .font_weight(rmac_ui::mac::SEMIBOLD)
                            .text_color(label())
                            .child(format!("Delete “{}”?", preview.name)),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(secondary())
                            .child(preview.service.clone()),
                    ),
            )
            .child(note_card(consequence))
            .when(busy, |dialog| {
                dialog.child(Progress::indeterminate().label("Deleting VPN profile…"))
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        rmac_ui::dialog_button(
                            "vpn-delete-cancel",
                            "Cancel",
                            rmac_ui::DialogButtonKind::Normal,
                        )
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_vpn_delete(cx))),
                    )
                    .child(
                        rmac_ui::dialog_button(
                            "vpn-delete-confirm",
                            "Delete",
                            rmac_ui::DialogButtonKind::Destructive,
                        )
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| this.confirm_vpn_delete(cx))),
                    ),
            );
        Some(
            rmac_ui::dialog("vpn-delete-dialog", content)
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" if !this.vpn_delete_busy => {
                            cx.stop_propagation();
                            this.cancel_vpn_delete(cx);
                        }
                        "enter" if !this.vpn_delete_busy => {
                            cx.stop_propagation();
                            this.confirm_vpn_delete(cx);
                        }
                        _ => {}
                    }
                }))
                .into_any_element(),
        )
    }

    pub(super) fn render_update_install_dialog(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let plan = self.updates_plan.as_ref()?;
        let requested = plan.requested.len();
        let changes = plan.changes.len();
        let installs = plan.change_count(rmac_updates::ChangeKind::Install);
        let removals = plan.change_count(rmac_updates::ChangeKind::Remove)
            + plan.change_count(rmac_updates::ChangeKind::Obsolete);
        let downgrades = plan.change_count(rmac_updates::ChangeKind::Downgrade);
        let destructive_preview = plan
            .changes
            .iter()
            .filter(|change| change.kind.is_destructive())
            .take(8)
            .map(|change| {
                format!(
                    "{} {} ({})",
                    change.name,
                    change.version,
                    change.kind.label()
                )
            })
            .collect::<Vec<_>>();
        let hidden_destructive = removals + downgrades - destructive_preview.len();
        let summary = format!(
            "PackageKit will apply {requested} requested updates through {changes} verified package changes. Dependencies are included in this preview."
        );
        let mut rows = vec![
            value_row(
                "icons/refresh-cw.svg",
                accent(),
                "Requested updates".into(),
                requested.to_string().into(),
            ),
            value_row(
                "icons/database.svg",
                secondary(),
                "Additional installs".into(),
                installs.to_string().into(),
            ),
        ];
        if removals > 0 {
            rows.push(value_row(
                "icons/shield.svg",
                hsl(0xff3b30),
                "Removals or replacements".into(),
                removals.to_string().into(),
            ));
        }
        if downgrades > 0 {
            rows.push(value_row(
                "icons/info.svg",
                hsl(0xff9500),
                "Downgrades".into(),
                downgrades.to_string().into(),
            ));
        }
        let view = cx.entity();
        let cancel_view = view.clone();
        let install_view = view.clone();
        let content = div()
            .w(px(460.0))
            .v_flex()
            .gap_4()
            .p_5()
            .rounded(px(14.0))
            .border_1()
            .border_color(rmac_ui::mac::separator())
            .shadow_xl()
            .bg(rmac_ui::mac::raised())
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(17.0))
                            .font_weight(rmac_ui::mac::SEMIBOLD)
                            .text_color(label())
                            .child("Install system updates?"),
                    )
                    .child(
                        div()
                            .text_size(rmac_ui::text_px(12.0))
                            .text_color(secondary())
                            .child(summary),
                    ),
            )
            .child(card(rows))
            .when(plan.has_destructive_changes(), |dialog| {
                let mut warning = format!(
                    "This verified plan changes packages destructively: {}.",
                    destructive_preview.join(", ")
                );
                if hidden_destructive > 0 {
                    warning.push_str(&format!(" Plus {hidden_destructive} more shown in the counts above."));
                }
                dialog.child(note_card(warning))
            })
            .when_some(plan.restart.label(), |dialog, restart| {
                dialog.child(note_card(format!("Expected after installation: {restart}.")))
            })
            .child(note_card(
                "rmac installs only the exact revalidated plan and keeps PackageKit's trusted-only flag enabled. Authorization may be requested.",
            ))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        rmac_ui::dialog_button(
                            "update-install-cancel",
                            "Cancel",
                            rmac_ui::DialogButtonKind::Normal,
                        )
                        .on_click(move |_, _, cx| {
                            cancel_view
                                .update(cx, |settings, cx| settings.cancel_update_plan(cx));
                        }),
                    )
                    .child(
                        rmac_ui::dialog_button(
                            "update-install-confirm",
                            "Install Updates",
                            rmac_ui::DialogButtonKind::Primary,
                        )
                        .on_click(move |_, _, cx| {
                            install_view
                                .update(cx, |settings, cx| settings.confirm_update_plan(cx));
                        }),
                    ),
            );
        Some(
            rmac_ui::dialog("update-install-dialog", content)
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" => {
                            cx.stop_propagation();
                            this.cancel_update_plan(cx);
                        }
                        "enter" => {
                            cx.stop_propagation();
                            this.confirm_update_plan(cx);
                        }
                        _ => {}
                    }
                }))
                .into_any_element(),
        )
    }

    /// Direct per-volume capacity state from the mount service.
    pub(super) fn storage_body(&self, cx: &Context<Self>) -> Div {
        let view = cx.entity();
        let refresh_view = view.clone();
        let mut body = div().v_flex().child(card(vec![row_base()
            .child(tile("icons/hard-drive.svg", accent(), 22.0))
            .child(text_block(
                "Mounted volumes".into(),
                Some("System, removable, and network volumes".into()),
            ))
            .child(
                Button::new("refresh-storage", "Refresh")
                    .busy(self.storage_busy || self.storage_stream_refreshing)
                    .disabled(self.system_data_loading || self.storage_busy)
                    .on_click(move |_, _, cx| {
                        refresh_view.update(cx, |settings, cx| settings.refresh_storage(cx));
                    }),
            )
            .into_any_element()]));

        if self.storage.is_empty() {
            return body.child(
                EmptyState::new("No storage volumes available")
                    .message("Refresh after the mount service becomes available")
                    .error(self.storage_error.is_some() || self.storage_stream_error.is_some()),
            );
        }

        for (index, volume) in self.storage.iter().enumerate() {
            let identity = volume.mount.identity.clone();
            let action_busy = self.storage_action_busy.as_deref() == Some(identity.as_str());
            let action_disabled = self.storage_action_busy.is_some() || self.storage_busy;
            let action_view = view.clone();
            body = body.child(section_header(volume.mount.name.clone()));
            let Some(usage) = volume.usage else {
                body = body.child(card(vec![row_base()
                    .child(tile("icons/hard-drive.svg", hsl(0xff9500), 22.0))
                    .child(text_block(
                        volume.mount.name.clone().into(),
                        volume.usage_error.clone().map(Into::into),
                    ))
                    .child(
                        Button::new(("open-storage-volume", index), "Review in Files")
                            .busy(action_busy)
                            .disabled(action_disabled)
                            .on_click(move |_, _, cx| {
                                action_view.update(cx, |settings, cx| {
                                    settings.open_storage_volume(identity.clone(), cx);
                                });
                            }),
                    )
                    .into_any_element()]));
                continue;
            };
            let identity = volume.mount.identity.clone();
            let action_view = view.clone();
            let available_color = if usage.is_low_space() {
                hsl(0xff3b30)
            } else {
                hsl(0x34c759)
            };
            body = body
                .child(
                    div()
                        .v_flex()
                        .gap_2()
                        .mb_3()
                        .p_4()
                        .rounded(px(10.0))
                        .bg(card_bg())
                        .border_1()
                        .border_color(if usage.is_low_space() {
                            rmac_ui::mac::warning_border()
                        } else {
                            sep()
                        })
                        .child(
                            div()
                                .h_flex()
                                .justify_between()
                                .items_center()
                                .child(
                                    div()
                                        .text_size(rmac_ui::text_px(15.0))
                                        .font_weight(rmac_ui::mac::SEMIBOLD)
                                        .text_color(label())
                                        .child(volume.mount.name.clone()),
                                )
                                .child(
                                    div()
                                        .h_flex()
                                        .items_center()
                                        .gap_3()
                                        .child(
                                            div()
                                                .text_size(rmac_ui::text_px(13.0))
                                                .text_color(secondary())
                                                .child(format!(
                                                    "{} available of {}",
                                                    fmt_gb(usage.available),
                                                    fmt_gb(usage.total)
                                                )),
                                        )
                                        .child(
                                            Button::new(
                                                ("open-storage-volume", index),
                                                "Review in Files",
                                            )
                                            .busy(action_busy)
                                            .disabled(action_disabled)
                                            .on_click(
                                                move |_, _, cx| {
                                                    action_view.update(cx, |settings, cx| {
                                                        settings.open_storage_volume(
                                                            identity.clone(),
                                                            cx,
                                                        );
                                                    });
                                                },
                                            ),
                                        ),
                                ),
                        )
                        .child(
                            div()
                                .w_full()
                                .h(px(10.0))
                                .rounded(px(5.0))
                                .bg(rmac_ui::mac::control_fill())
                                .child(
                                    div()
                                        .h_full()
                                        .w(gpui::relative(usage.used_fraction()))
                                        .rounded(px(5.0))
                                        .bg(if usage.is_low_space() {
                                            hsl(0xff3b30)
                                        } else {
                                            accent()
                                        }),
                                ),
                        ),
                )
                .child(card(vec![
                    value_row(
                        "icons/database.svg",
                        secondary(),
                        "Capacity".into(),
                        fmt_gb(usage.total).into(),
                    ),
                    value_row(
                        "icons/database.svg",
                        hsl(0xff9500),
                        "Used".into(),
                        fmt_gb(usage.used).into(),
                    ),
                    value_row(
                        "icons/database.svg",
                        available_color,
                        "Available".into(),
                        fmt_gb(usage.available).into(),
                    ),
                ]));
            if usage.is_low_space() {
                body = body.child(note_card(
                    "Space is low on this volume. Review large personal files and application caches before removing anything; rmac does not guess which files are safe to delete.",
                ));
            }
        }
        body.child(note_card(
            "Review in Files opens only a currently revalidated mounted volume. Storage categories and destructive cleanup actions stay hidden until they can be measured and reversed safely.",
        ))
    }
}
