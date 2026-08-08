//! Software Update, time, locale, and Login Items startup watchers.

use super::*;

impl Settings {
    pub(super) fn start_updates_locale_watchers(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = rmac_updates_linux::snapshot(rmac_updates::Request::cached()).await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_update_status(result);
                this.run_pending_update_refresh(cx);
                cx.notify();
            });
        })
        .detach();

        #[cfg(target_os = "linux")]
        {
            let (update_events, update_event_rx) = async_channel::bounded(1);
            cx.background_executor()
                .spawn(async move {
                    let _ = rmac_updates_linux::watch(update_events).await;
                })
                .detach();
            cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
                while let Ok(event) = update_event_rx.recv().await {
                    if this
                        .update(cx, |this: &mut Settings, cx| {
                            match event {
                                rmac_updates::WatchEvent::Changed => {
                                    this.queue_update_stream_refresh(cx);
                                }
                                rmac_updates::WatchEvent::Unavailable => {
                                    this.updates_stream_error = Some(
                                        "Live PackageKit updates are temporarily unavailable"
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
                .spawn(async { rmac_time_linux::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_time_update(result);
                this.run_pending_time_refresh(cx);
                cx.notify();
            });
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_locale_linux::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_locale_update(result);
                this.run_pending_locale_refresh(cx);
                cx.notify();
            });
        })
        .detach();

        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { rmac_login_items_linux::snapshot() })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_login_items_update(result);
                this.run_pending_login_items_refresh(cx);
                cx.notify();
            });
        })
        .detach();

        let (locale_updates, locale_update_rx) = async_channel::bounded(1);
        cx.background_executor()
            .spawn(async move {
                let _ = rmac_locale_linux::watch(locale_updates).await;
            })
            .detach();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = locale_update_rx.recv().await {
                match event {
                    rmac_locale::WatchEvent::Changed => {
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                this.queue_locale_stream_refresh(cx);
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    rmac_locale::WatchEvent::Unavailable => {
                        if this
                            .update(cx, |this: &mut Settings, cx| {
                                this.locale_stream_error = Some(
                                    "Live language and region updates are temporarily unavailable"
                                        .into(),
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
    }
}
