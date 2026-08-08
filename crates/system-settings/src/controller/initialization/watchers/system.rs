//! System, storage, privacy, login-item, time, and sharing startup watchers.

use super::*;

impl Settings {
    pub(super) fn start_system_watchers(cx: &mut Context<Self>) {
        // Hardware discovery launches multiple platform commands, including
        // system_profiler. Keep it off the first-frame path and redraw once the
        // complete read-only snapshot is available.
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let snapshot = cx
                .background_executor()
                .spawn(async { gather_system_snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.apply_system_snapshot(snapshot);
                this.run_pending_system_info_refresh(cx);
                this.run_pending_storage_refresh(cx);
                cx.notify();
            });
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let capability = cx
                .background_executor()
                .spawn(async { gather_screen_reader_capability() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.screen_reader = capability;
                this.screen_reader_loading = false;
                cx.notify();
            });
        })
        .detach();

        #[cfg(target_os = "linux")]
        {
            let (system_info_updates, system_info_update_rx) = async_channel::bounded(1);
            cx.background_executor()
                .spawn(async move {
                    let _ = rmac_system_info::watch(system_info_updates).await;
                })
                .detach();
            cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
                while let Ok(event) = system_info_update_rx.recv().await {
                    if this
                        .update(cx, |this: &mut Settings, cx| {
                            match event {
                                rmac_system_info::WatchEvent::Changed => {
                                    this.queue_system_info_stream_refresh(cx);
                                }
                                rmac_system_info::WatchEvent::Unavailable => {
                                    this.system_data_stream_error = Some(
                                        "Live hostname updates are temporarily unavailable".into(),
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

        #[cfg(target_os = "linux")]
        {
            let (storage_updates, storage_update_rx) = async_channel::bounded(1);
            cx.background_executor()
                .spawn(async move {
                    let _ = rmac_mounts::watch(storage_updates).await;
                })
                .detach();
            cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
                while let Ok(event) = storage_update_rx.recv().await {
                    if this
                        .update(cx, |this: &mut Settings, cx| {
                            match event {
                                rmac_mounts::WatchEvent::Changed => {
                                    this.queue_storage_stream_refresh(cx);
                                }
                                rmac_mounts::WatchEvent::Unavailable => {
                                    this.storage_stream_error = Some(
                                        "Live mounted-volume updates are temporarily unavailable"
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
            let result = cx
                .background_executor()
                .spawn(async { rmac_sharing_linux::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_sharing_update(result);
                cx.notify();
            });
        })
        .detach();

        let (privacy_updates, privacy_update_rx) = async_channel::bounded(1);
        cx.background_executor()
            .spawn(async move {
                let _ = rmac_privacy_linux::watch(privacy_updates).await;
            })
            .detach();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = privacy_update_rx.recv().await {
                if this
                    .update(cx, |this: &mut Settings, cx| {
                        match event {
                            rmac_privacy::WatchEvent::Changed => {
                                this.queue_privacy_stream_refresh(cx);
                            }
                            rmac_privacy::WatchEvent::Unavailable => {
                                this.privacy_stream_error = Some(
                                    "Live portal permission updates are temporarily unavailable"
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

        let (login_item_updates, login_item_update_rx) = async_channel::bounded(1);
        cx.background_executor()
            .spawn(async move {
                let _ = rmac_login_items_linux::watch(login_item_updates).await;
            })
            .detach();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = login_item_update_rx.recv().await {
                match event {
                    rmac_login_items::WatchEvent::Changed => {
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                this.queue_login_items_stream_refresh(cx);
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    rmac_login_items::WatchEvent::Unavailable => {
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                this.login_items_stream_error = Some(
                                    "Live Login Items updates are temporarily unavailable".into(),
                                );
                                cx.notify();
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

        let (time_updates, time_update_rx) = async_channel::bounded(1);
        cx.background_executor()
            .spawn(async move {
                let _ = rmac_time_linux::watch(time_updates).await;
            })
            .detach();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = time_update_rx.recv().await {
                match event {
                    rmac_time::WatchEvent::Changed => {
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                this.queue_time_stream_refresh(cx);
                                cx.notify();
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    rmac_time::WatchEvent::Unavailable => {
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                this.time_stream_error = Some(
                                    "Live date and time updates are temporarily unavailable".into(),
                                );
                                cx.notify();
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

        let (sharing_updates, sharing_update_rx) = async_channel::bounded(1);
        cx.background_executor()
            .spawn(async move {
                let _ = rmac_sharing_linux::watch(sharing_updates).await;
            })
            .detach();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = sharing_update_rx.recv().await {
                match event {
                    rmac_sharing::WatchEvent::Changed => {
                        let result = cx
                            .background_executor()
                            .spawn(async { rmac_sharing_linux::snapshot() })
                            .await;
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                if !this.sharing_busy {
                                    this.finish_sharing_update(result);
                                    this.sharing_stream_error = None;
                                    cx.notify();
                                }
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    rmac_sharing::WatchEvent::Unavailable => {
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                this.sharing_stream_error =
                                    Some("Live Sharing updates are temporarily unavailable".into());
                                cx.notify();
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

        // Keep the visible clock current without putting System Settings on a
        // frame-rate loop. Two low-frequency wakeups per minute are enough for
        // the minute-resolution label and stop redrawing when another pane is
        // selected.
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| loop {
            async_io::Timer::after(std::time::Duration::from_secs(30)).await;
            if this
                .update(cx, |this: &mut Settings, cx| {
                    if this.nav.is_empty() && this.current().name.as_ref() == "Date & Time" {
                        cx.notify();
                    }
                })
                .is_err()
            {
                break;
            }
        })
        .detach();
    }
}
