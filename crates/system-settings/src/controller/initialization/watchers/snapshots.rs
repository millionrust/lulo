//! Initial connectivity, hardware, accessibility, and privacy snapshots.
//!
//! Every `finish_*_update` called from here is shared with a later,
//! explicit user action (Refresh, applying a change) that reuses the same
//! function and the same `*_error` field so that action's own failure stays
//! visible. A failure on this first, passive read is a different thing --
//! "this subsystem has no backing service on this machine", not "something
//! you just did failed" -- and every pane already says so quietly on its
//! own (its `Some(snapshot)`-or-placeholder branch), so the `*_error` this
//! path would otherwise leave behind is cleared right after the call to
//! keep it out of the window-wide Settings banner (`global_settings_error`).

use super::*;

impl Settings {
    pub(super) fn start_snapshot_loads(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            // `rmac_network::snapshot()`/`request_scan()` use
            // `zbus::blocking` (a synchronous D-Bus call that also opens
            // its own connection); GPUI's background executor is not safe
            // to block on that (LINUX-HW-07 — the same class of bug fixed
            // for input/keyboard/gtk/screen-reader/privacy below).
            // `blocking::unblock` runs it on the dedicated blocking-task
            // pool instead of stalling the small, fixed-size executor.
            //
            // Read whatever NetworkManager already knows first, with no
            // scan trigger and no settle delay, so the pane shows the
            // current Wi-Fi state (radio on/off, connected network, the
            // access points NM already has cached) on the very first
            // frame instead of sitting on "Loading" — a fresh scan below
            // only refines the *nearby-network* list in the background,
            // same as the Mac (SET-14).
            let immediate = blocking::unblock(rmac_network::snapshot).await;
            let showed_immediate = immediate.is_ok();
            let _ = this.update(cx, |this: &mut Settings, cx| {
                if showed_immediate {
                    this.finish_wifi_update(immediate);
                    this.wifi_error = None;
                }
                this.notify_if_showing(&["Wi-Fi"], cx);
            });

            let result = blocking::unblock(|| {
                // Kick NetworkManager into a scan so the network list
                // picks up anything not already cached (SET-14). A scan
                // failure (radio off, no adapter yet) is not fatal here:
                // the snapshot read below is still the authoritative
                // result and surfaces its own error if Wi-Fi is genuinely
                // unavailable.
                let _ = rmac_network::request_scan();
                std::thread::sleep(Duration::from_millis(750));
                rmac_network::snapshot()
            })
            .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                // The scan lands ~0.8 s after launch. Repaint the whole
                // window only when it changed something (most often it
                // returns what the immediate read above already showed).
                let before = this.wifi_view_state();
                this.finish_wifi_update(result);
                this.wifi_error = None;
                if this.wifi_view_state() != before {
                    this.notify_if_showing(&["Wi-Fi"], cx);
                }
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
                this.network_error = None;
                this.notify_if_showing(&["Network"], cx);
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
                this.vpn_error = None;
                this.notify_if_showing(&["VPN", "Network"], cx);
            });
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            // `vpn_import_capabilities()` shells out to `nmcli` to probe
            // available importers; GPUI's background executor is not safe
            // to spawn child processes from (LINUX-HW-07).
            let capabilities = blocking::unblock(rmac_network::vpn_import_capabilities).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.vpn_import_capabilities = capabilities;
                this.vpn_import_loading = false;
                this.notify_if_showing(&["VPN", "Network"], cx);
            });
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            // `rmac_audio::snapshot()` shells out to `pw-dump`; GPUI's
            // background executor is not safe to spawn child processes
            // from (LINUX-HW-07).
            let result = blocking::unblock(rmac_audio::snapshot).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_audio_update(result, cx);
                this.audio_error = None;
                this.notify_if_showing(&["Sound"], cx);
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
                this.power_error = None;
                this.notify_if_showing(&["Battery"], cx);
            });
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            // `rmac_display::snapshot()` shells out to `niri msg`; GPUI's
            // background executor is not safe to spawn child processes
            // from (LINUX-HW-07).
            let result = blocking::unblock(rmac_display::snapshot).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_display_update(result);
                this.display_error = None;
                this.flush_display_stream_refresh(cx);
                this.notify_if_showing(&["Displays"], cx);
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
                this.notify_if_showing(&["Displays"], cx);
            });
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            // `rmac_input::snapshot()` shells out to niri (`niri msg`,
            // `niri validate`). GPUI's `background_executor()` is not safe
            // to spawn child processes from -- doing so here left the
            // Keyboard/Mouse/Trackpad panes stuck on "Loading input
            // settings…" forever (LINUX-HW-03). `blocking::unblock` runs it
            // on the dedicated blocking-task pool the rest of Settings
            // already uses for `Command`-spawning work.
            let result = blocking::unblock(rmac_input::snapshot).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_input_update(result, cx);
                this.input_error = None;
                this.flush_input_stream_refresh(cx);
                this.notify_if_showing(
                    &[
                        "Keyboard",
                        "Mouse",
                        "Trackpad",
                        "Touchscreen",
                        "Language & Region",
                        "Accessibility",
                    ],
                    cx,
                );
            });
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            // See the input snapshot load above: `rmac_keyboard::status()`
            // also shells out, so it needs the same dedicated pool.
            let result = blocking::unblock(rmac_keyboard::status).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_mac_keyboard_update(result);
                this.notify_if_showing(&["Keyboard"], cx);
            });
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            // `rmac_gtk_settings::snapshot()` shells out to `gsettings`;
            // GPUI's background executor is not safe to spawn child
            // processes from (LINUX-HW-07).
            let result = blocking::unblock(rmac_gtk_settings::snapshot).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_gtk_text_update(result);
                this.gtk_text_error = None;
                this.run_pending_gtk_text_refresh(cx);
                this.notify_if_showing(&["Accessibility", "Appearance"], cx);
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
                                eprintln!(
                                    "System Settings: GTK text-scale live-update watcher unavailable"
                                );
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
            // `rmac_screen_reader::snapshot()` shells out to `gsettings`;
            // GPUI's background executor is not safe to spawn child
            // processes from (LINUX-HW-07).
            let result = blocking::unblock(rmac_screen_reader::snapshot).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_screen_reader_toggle_update(result);
                this.screen_reader_toggle_error = None;
                this.run_pending_screen_reader_toggle_refresh(cx);
                this.notify_if_showing(&["Accessibility"], cx);
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
                                eprintln!(
                                    "System Settings: screen reader live-update watcher unavailable"
                                );
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
                this.privacy_error = None;
                this.run_pending_privacy_refresh(cx);
                this.notify_if_showing(&["Privacy & Security"], cx);
            });
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            // `security_coverage_snapshot()` shells out (e.g. to probe the
            // firewall); GPUI's background executor is not safe to spawn
            // child processes from (LINUX-HW-07).
            let snapshot = blocking::unblock(rmac_privacy_linux::security_coverage_snapshot).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.security_coverage = Some(snapshot);
                this.security_coverage_loading = false;
                this.notify_if_showing(&["Privacy & Security"], cx);
            });
        })
        .detach();
    }
}
