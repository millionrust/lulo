//! Shell settings, compositor, input, and shortcut startup watchers.

use super::*;

impl Settings {
    pub(super) fn start_shell_input_watchers(cx: &mut Context<Self>) {
        let (shell_settings_updates, shell_settings_update_rx) = async_channel::bounded(2);
        cx.background_executor()
            .spawn(watch_shell_settings(shell_settings_updates))
            .detach();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(update) = shell_settings_update_rx.recv().await {
                if this
                    .update(cx, |this: &mut Settings, cx| {
                        if this.apply_shell_settings_stream_update(update, cx) {
                            this.refresh_wallpaper_preview(cx);
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

        let (dock_compositor_events, dock_compositor_event_rx) = async_channel::bounded(64);
        cx.background_executor()
            .spawn(async move {
                loop {
                    let result = rmac_compositor_niri::watch(dock_compositor_events.clone()).await;
                    if dock_compositor_events.is_closed() {
                        return;
                    }
                    if result.is_err()
                        && dock_compositor_events
                            .send(rmac_compositor::Event::ConnectionChanged {
                                state: rmac_compositor::ConnectionState::Disconnected,
                            })
                            .await
                            .is_err()
                    {
                        return;
                    }
                    async_io::Timer::after(Duration::from_secs(1)).await;
                }
            })
            .detach();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = dock_compositor_event_rx.recv().await {
                if this
                    .update(cx, |this: &mut Settings, cx| {
                        let refresh_displays = compositor_event_affects_displays(&event);
                        let refresh_input = compositor_event_affects_input(&event);
                        let input_config_failed = compositor_input_config_failed(&event);
                        this.dock_compositor.apply(event);
                        if refresh_displays {
                            this.request_display_stream_refresh(cx);
                        }
                        if refresh_input {
                            this.request_input_stream_refresh(cx);
                        } else if input_config_failed == Some(true) {
                            this.input_error = Some(
                                "Could not update Input settings: niri rejected its latest configuration reload; the last known-good values remain visible."
                                    .into(),
                            );
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

        #[cfg(target_os = "linux")]
        {
            let (input_events, input_event_rx) = async_channel::bounded(2);
            cx.background_executor()
                .spawn(async move {
                    loop {
                        let result = rmac_input::watch(input_events.clone()).await;
                        if input_events.is_closed() {
                            return;
                        }
                        if let Err(error) = result {
                            if input_events
                                .send(rmac_input::WatchEvent::WatchError(error.to_string()))
                                .await
                                .is_err()
                            {
                                return;
                            }
                        }
                        async_io::Timer::after(Duration::from_secs(1)).await;
                    }
                })
                .detach();
            cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
                while let Ok(event) = input_event_rx.recv().await {
                    if this
                        .update(cx, |this: &mut Settings, cx| {
                            match event {
                                rmac_input::WatchEvent::Changed => {
                                    this.input_stream_error = None;
                                    this.request_input_stream_refresh(cx);
                                }
                                rmac_input::WatchEvent::WatchError(error) => {
                                    this.input_stream_error = Some(
                                        format!(
                                            "Live input-device updates are unavailable: {error}"
                                        )
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
        }

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = blocking::unblock(rmac_shortcuts::backend_status).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_shortcut_status_update(result);
                cx.notify();
            });
        })
        .detach();
    }
}
