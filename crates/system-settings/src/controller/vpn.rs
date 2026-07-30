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
}
