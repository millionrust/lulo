//! Portal-backed VPN import lifecycle.

use super::*;

impl Settings {
    pub(in crate::controller) fn choose_vpn_import(
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

    pub(in crate::controller) fn finish_vpn_import(&mut self, keep: bool, cx: &mut Context<Self>) {
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
