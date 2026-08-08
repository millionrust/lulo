//! Network stream, external-change, and refresh lifecycle.

use super::*;

impl Settings {
    pub(in crate::controller) fn finish_network_stream_update(
        &mut self,
        result: std::result::Result<rmac_network::NetworkSnapshot, rmac_network::Error>,
    ) {
        match result {
            Ok(snapshot) => {
                let stale_editor = self.apply_external_network_snapshot(snapshot);
                self.network_stream_error = None;
                if stale_editor {
                    self.network_error = Some(
                        "The connection profile changed outside System Settings. Reopen Details to edit the current values."
                            .into(),
                    );
                }
            }
            Err(_) => {
                self.network_stream_error =
                    Some("Live Network state could not be refreshed from NetworkManager".into());
            }
        }
    }

    pub(in crate::controller) fn apply_external_network_snapshot(
        &mut self,
        snapshot: rmac_network::NetworkSnapshot,
    ) -> bool {
        let stale_editor = self.network_editor.as_ref().is_some_and(|editor| {
            snapshot
                .devices
                .iter()
                .filter_map(|device| device.configuration.as_ref())
                .find(|configuration| configuration.id == editor.configuration.id)
                != Some(&editor.configuration)
        });
        self.network = snapshot;
        if stale_editor {
            self.network_editor = None;
        }
        stale_editor
    }

    pub(in crate::controller) fn finish_network_update(
        &mut self,
        result: std::result::Result<rmac_network::NetworkSnapshot, rmac_network::Error>,
    ) {
        self.network_loading = false;
        self.network_busy = false;
        match result {
            Ok(snapshot) => {
                if self.apply_external_network_snapshot(snapshot) {
                    self.network_error = Some(
                        "The connection profile changed outside System Settings. Reopen Details to edit the current values."
                            .into(),
                    );
                } else {
                    self.network_error = None;
                }
                self.network_stream_error = None;
            }
            Err(error) => {
                self.network_error = Some(format!("Could not update Network: {error}").into());
            }
        }
    }

    pub(in crate::controller) fn refresh_network(&mut self, cx: &mut Context<Self>) {
        if self.network_busy || self.network_loading {
            return;
        }
        self.network_generation = self.network_generation.wrapping_add(1);
        self.network_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_network::network_snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_network_update(result);
                cx.notify();
            });
        })
        .detach();
    }
}
