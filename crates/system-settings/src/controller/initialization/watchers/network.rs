//! NetworkManager Wi-Fi, Network, and VPN startup watcher.

use super::*;

impl Settings {
    pub(super) fn start_network_watchers(cx: &mut Context<Self>) {
        let (wifi_updates, wifi_update_rx) = async_channel::bounded(1);
        cx.background_executor()
            .spawn(async move {
                let _ = rmac_network::watch(wifi_updates).await;
            })
            .detach();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = wifi_update_rx.recv().await {
                match event {
                    rmac_network::WifiWatchEvent::Changed => {
                        let generations = match this.update(cx, |this: &mut Settings, cx| {
                            this.wifi_stream_error = None;
                            this.network_stream_error = None;
                            this.vpn_stream_error = None;
                            this.notify_if_current_pane(&["Wi-Fi", "Network", "VPN"], cx);
                            (
                                (!this.wifi_busy && !this.wifi_loading)
                                    .then_some(this.wifi_generation),
                                (!this.network_busy && !this.network_loading)
                                    .then_some(this.network_generation),
                                (this.vpn_busy.is_none()
                                    && !this.vpn_loading
                                    && !this.vpn_refreshing
                                    && !this.vpn_import_busy
                                    && this.vpn_import_preview.is_none()
                                    && this.vpn_delete_preparing.is_none()
                                    && !this.vpn_delete_busy
                                    && this.vpn_delete_preview.is_none())
                                    .then_some(this.vpn_generation),
                            )
                        }) {
                            Ok(generations) => generations,
                            Err(_) => break,
                        };
                        if generations.0.is_none()
                            && generations.1.is_none()
                            && generations.2.is_none()
                        {
                            continue;
                        }
                        // `rmac_network::{snapshot,network_snapshot,vpn_snapshot}()`
                        // use `zbus::blocking`; GPUI's background executor
                        // is not safe to block on synchronous D-Bus I/O
                        // from (LINUX-HW-07).
                        let results = blocking::unblock(move || {
                            (
                                generations.0.map(|_| rmac_network::snapshot()),
                                generations.1.map(|_| rmac_network::network_snapshot()),
                                generations.2.map(|_| rmac_network::vpn_snapshot()),
                            )
                        })
                        .await;
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                if let (Some(generation), Some(result)) =
                                    (generations.0, results.0)
                                {
                                    if wifi_stream_snapshot_is_current(
                                        generation,
                                        this.wifi_generation,
                                        this.wifi_busy,
                                        this.wifi_loading,
                                    ) {
                                        this.finish_wifi_stream_update(result);
                                    }
                                }
                                if let (Some(generation), Some(result)) =
                                    (generations.1, results.1)
                                {
                                    if network_stream_snapshot_is_current(
                                        generation,
                                        this.network_generation,
                                        this.network_busy,
                                        this.network_loading,
                                    ) {
                                        this.finish_network_stream_update(result);
                                    }
                                }
                                if let (Some(generation), Some(result)) =
                                    (generations.2, results.2)
                                {
                                    if vpn_stream_snapshot_is_current(
                                        generation,
                                        this.vpn_generation,
                                        this.vpn_busy.is_some()
                                            || this.vpn_refreshing
                                            || this.vpn_import_busy
                                            || this.vpn_import_preview.is_some()
                                            || this.vpn_editor_loading.is_some()
                                            || this.vpn_editor_busy
                                            || this.vpn_editor.is_some()
                                            || this.vpn_secret_preparing
                                            || this.vpn_secret_busy
                                            || this.vpn_secret_preview.is_some()
                                            || this.vpn_delete_preparing.is_some()
                                            || this.vpn_delete_busy
                                            || this.vpn_delete_preview.is_some(),
                                        this.vpn_loading,
                                    ) {
                                        this.finish_vpn_stream_update(result);
                                    }
                                }
                                this.notify_if_current_pane(&["Wi-Fi", "Network", "VPN"], cx);
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    rmac_network::WifiWatchEvent::Unavailable => {
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                eprintln!(
                                    "System Settings: network live-update watcher unavailable (NetworkManager reconnecting)"
                                );
                                this.wifi_stream_error =
                                    Some("Wi-Fi status isn\u{2019}t available right now".into());
                                this.network_stream_error =
                                    Some("Network status isn\u{2019}t available right now".into());
                                this.vpn_stream_error =
                                    Some("VPN status isn\u{2019}t available right now".into());
                                this.notify_if_current_pane(&["Wi-Fi", "Network", "VPN"], cx);
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
        })
        .detach();
    }
}
