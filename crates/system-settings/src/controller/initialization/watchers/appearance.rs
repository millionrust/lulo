//! Theme-store and appearance-portal startup watchers.

use super::*;

impl Settings {
    pub(super) fn start_appearance_watchers(cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { load_theme_state().await })
                .await;
            let _ = this.update(cx, |this: &mut Settings, cx| {
                this.finish_theme_update(result);
                this.run_pending_theme_refresh(cx);
                cx.notify();
            });
        })
        .detach();

        let (theme_store_updates, theme_store_update_rx) = async_channel::bounded(2);
        cx.background_executor()
            .spawn(async move {
                loop {
                    let watcher = match rmac_theme::ThemeStore::from_environment()
                        .and_then(|store| store.watch())
                    {
                        Ok(watcher) => watcher,
                        Err(_) => {
                            if theme_store_updates
                                .send(ThemeStoreWatchEvent::Unavailable)
                                .await
                                .is_err()
                            {
                                return;
                            }
                            async_io::Timer::after(Duration::from_secs(1)).await;
                            continue;
                        }
                    };
                    if theme_store_updates
                        .send(ThemeStoreWatchEvent::Available)
                        .await
                        .is_err()
                    {
                        return;
                    }
                    loop {
                        match watcher.recv().await {
                            Ok(rmac_theme::StoreEvent::Changed) => {
                                if theme_store_updates
                                    .send(ThemeStoreWatchEvent::Changed)
                                    .await
                                    .is_err()
                                {
                                    return;
                                }
                            }
                            Ok(rmac_theme::StoreEvent::WatchError(_)) | Err(_) => {
                                if theme_store_updates
                                    .send(ThemeStoreWatchEvent::Unavailable)
                                    .await
                                    .is_err()
                                {
                                    return;
                                }
                                break;
                            }
                        }
                    }
                    async_io::Timer::after(Duration::from_secs(1)).await;
                }
            })
            .detach();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = theme_store_update_rx.recv().await {
                if this
                    .update(cx, |this: &mut Settings, cx| {
                        match event {
                            ThemeStoreWatchEvent::Available => {
                                this.theme_store_stream_error = None;
                            }
                            ThemeStoreWatchEvent::Changed => {
                                this.theme_store_stream_error = None;
                                this.queue_theme_stream_refresh(cx);
                            }
                            ThemeStoreWatchEvent::Unavailable => {
                                this.theme_store_stream_error = Some(
                                    "Live Lulo OS appearance updates are temporarily unavailable"
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

        let (portal_appearance_updates, portal_appearance_update_rx) = async_channel::bounded(2);
        cx.background_executor()
            .spawn(async move {
                let _ = rmac_appearance_portal::watch(portal_appearance_updates).await;
            })
            .detach();
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            while let Ok(event) = portal_appearance_update_rx.recv().await {
                if this
                    .update(cx, |this: &mut Settings, cx| {
                        match event {
                            rmac_appearance::Event::Snapshot(_) => {
                                this.theme_portal_stream_error = None;
                                this.queue_theme_stream_refresh(cx);
                            }
                            rmac_appearance::Event::Unavailable(_) => {
                                this.theme_portal_stream_error = Some(
                                    "Live desktop appearance updates are temporarily unavailable"
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
}
