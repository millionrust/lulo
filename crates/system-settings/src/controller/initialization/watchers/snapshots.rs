//! Initial connectivity, hardware, accessibility, and privacy snapshots.

use super::*;

impl Settings {
    pub(super) fn start_snapshot_loads(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async {
                    // Kick NetworkManager into an immediate scan so a
                    // freshly opened Wi-Fi pane does not read "No other
                    // networks found" while NM's own background scan
                    // interval catches up (SET-14). A scan failure (radio
                    // off, no adapter yet) is not fatal here: the snapshot
                    // read below is still the authoritative result and
                    // surfaces its own error if Wi-Fi is genuinely
                    // unavailable.
                    let _ = rmac_network::request_scan();
                    std::thread::sleep(Duration::from_millis(750));
                    rmac_network::snapshot()
                })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_wifi_update(result);
                cx.notify();
            });
        })
        .detach();

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

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let capabilities = cx
                .background_executor()
                .spawn(async { rmac_network::vpn_import_capabilities() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.vpn_import_capabilities = capabilities;
                this.vpn_import_loading = false;
                cx.notify();
            });
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_audio::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_audio_update(result, cx);
                cx.notify();
            });
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_power::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_power_update(result, cx);
                cx.notify();
            });
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_display::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_display_update(result);
                this.flush_display_stream_refresh(cx);
                cx.notify();
            });
        })
        .detach();

        // The backlight is read once at startup, then only ever changed by
        // this pane's slider or the hardware brightness keys (rmac-osd);
        // there is no logind signal to poll for, so it is not re-read.
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_osd::brightness() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if let Ok(value) = result {
                    this.brightness = Some(value);
                    this.brightness_slider = Self::brightness_slider(cx, f32::from(value));
                }
                cx.notify();
            });
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_input::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_input_update(result, cx);
                this.flush_input_stream_refresh(cx);
                cx.notify();
            });
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_keyboard::status() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_mac_keyboard_update(result);
                cx.notify();
            });
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_gtk_settings::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_gtk_text_update(result);
                this.run_pending_gtk_text_refresh(cx);
                cx.notify();
            });
        })
        .detach();

        let (gtk_text_updates, gtk_text_update_rx) = async_channel::bounded(2);
        std::thread::spawn(move || {
            let _ = rmac_gtk_settings::watch(gtk_text_updates);
        });
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = gtk_text_update_rx.recv().await {
                if this
                    .update(cx, |this: &mut Settings, cx| {
                        match event {
                            rmac_gtk_settings::WatchEvent::Available => {
                                this.gtk_text_stream_error = None;
                            }
                            rmac_gtk_settings::WatchEvent::Changed => {
                                this.gtk_text_stream_error = None;
                                this.queue_gtk_text_stream_refresh(cx);
                            }
                            rmac_gtk_settings::WatchEvent::Unavailable => {
                                this.gtk_text_stream_error = Some(
                                    "Live GTK text-scale updates are temporarily unavailable"
                                        .into(),
                                );
                            }
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_screen_reader::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_screen_reader_toggle_update(result);
                this.run_pending_screen_reader_toggle_refresh(cx);
                cx.notify();
            });
        })
        .detach();

        let (screen_reader_toggle_updates, screen_reader_toggle_update_rx) =
            async_channel::bounded(2);
        std::thread::spawn(move || {
            let _ = rmac_screen_reader::watch(screen_reader_toggle_updates);
        });
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = screen_reader_toggle_update_rx.recv().await {
                if this
                    .update(cx, |this: &mut Settings, cx| {
                        match event {
                            rmac_screen_reader::WatchEvent::Available => {
                                this.screen_reader_toggle_stream_error = None;
                            }
                            rmac_screen_reader::WatchEvent::Changed => {
                                this.screen_reader_toggle_stream_error = None;
                                this.queue_screen_reader_toggle_stream_refresh(cx);
                            }
                            rmac_screen_reader::WatchEvent::Unavailable => {
                                this.screen_reader_toggle_stream_error = Some(
                                    "Live screen reader updates are temporarily unavailable".into(),
                                );
                            }
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_privacy_linux::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_privacy_update(result);
                this.run_pending_privacy_refresh(cx);
                cx.notify();
            });
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let snapshot = cx
                .background_executor()
                .spawn(async { rmac_privacy_linux::security_coverage_snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.security_coverage = Some(snapshot);
                this.security_coverage_loading = false;
                cx.notify();
            });
        })
        .detach();
    }
}
