//! VPN saved-secret clearing and profile deletion lifecycle.

use super::*;

impl Settings {
    pub(in crate::controller) fn request_vpn_secret_clear(
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

    pub(in crate::controller) fn cancel_vpn_secret_clear(&mut self, cx: &mut Context<Self>) {
        if self.vpn_secret_busy || self.vpn_secret_preparing {
            return;
        }
        if self.vpn_secret_preview.take().is_some() {
            self.vpn_error = None;
            cx.notify();
        }
    }

    pub(in crate::controller) fn confirm_vpn_secret_clear(&mut self, cx: &mut Context<Self>) {
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

    pub(in crate::controller) fn request_vpn_delete(
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

    pub(in crate::controller) fn cancel_vpn_delete(&mut self, cx: &mut Context<Self>) {
        if self.vpn_delete_busy || self.vpn_delete_preparing.is_some() {
            return;
        }
        if self.vpn_delete_preview.take().is_some() {
            self.refresh_vpn(cx);
        }
    }

    pub(in crate::controller) fn confirm_vpn_delete(&mut self, cx: &mut Context<Self>) {
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
