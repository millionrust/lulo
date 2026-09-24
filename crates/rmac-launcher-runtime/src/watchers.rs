//! Installed-application and shell-settings watch authorities.

use super::*;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum CatalogHealth {
    #[default]
    Unmanaged,
    Starting,
    Healthy,
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogUpdate {
    pub health: CatalogHealth,
    pub revision: u64,
    pub changed: bool,
    /// Diagnostics only. Overlay snapshots deliberately omit this value.
    pub detail: Option<String>,
}

#[derive(Clone, Debug)]
pub struct CatalogEffect {
    pub visible: bool,
    pub request: Option<Request>,
}

/// Watch installed applications without a discovery/watch race. Failures keep
/// the provider's last-known-good catalog and retry with a bounded delay.
pub async fn watch_application_catalog(
    provider: rmac_launcher_providers::ApplicationProvider,
    sender: async_channel::Sender<CatalogUpdate>,
) {
    if sender
        .send(CatalogUpdate {
            health: CatalogHealth::Starting,
            revision: provider.revision(),
            changed: false,
            detail: None,
        })
        .await
        .is_err()
    {
        return;
    }
    loop {
        let (changed_tx, changed_rx) = async_channel::bounded(1);
        let setup = blocking::unblock(move || {
            let callback = changed_tx.clone();
            rmac_apps::watch_catalog(move || {
                let _ = callback.try_send(());
            })
            .map(|watcher| (watcher, changed_rx))
        })
        .await;
        let (watcher, changed) = match setup {
            Ok(setup) => setup,
            Err(error) => {
                if send_catalog_failure(&provider, &sender, error.to_string())
                    .await
                    .is_err()
                {
                    return;
                }
                wait_or_closed(&sender, Duration::from_secs(1)).await;
                if sender.is_closed() {
                    return;
                }
                continue;
            }
        };
        let _watcher = watcher;
        let mut refresh = true;
        loop {
            if refresh {
                let discovery = blocking::unblock(rmac_apps::discover_for_browsing).await;
                refresh = discovery.is_err();
                let update = match discovery {
                    Ok(catalog) => CatalogUpdate {
                        changed: provider.replace_catalog(catalog),
                        health: CatalogHealth::Healthy,
                        revision: provider.revision(),
                        detail: None,
                    },
                    Err(error) => CatalogUpdate {
                        health: CatalogHealth::Unavailable,
                        revision: provider.revision(),
                        changed: false,
                        detail: Some(error.to_string()),
                    },
                };
                if sender.send(update).await.is_err() {
                    return;
                }
            }

            let changed_event = futures_util::FutureExt::fuse(changed.recv());
            let should_retry = refresh;
            let retry = futures_util::FutureExt::fuse(async move {
                if should_retry {
                    async_io::Timer::after(Duration::from_secs(1)).await;
                } else {
                    std::future::pending::<()>().await;
                }
            });
            let closed = futures_util::FutureExt::fuse(sender.closed());
            futures_util::pin_mut!(changed_event, retry, closed);
            futures_util::select! {
                event = changed_event => {
                    if event.is_err() {
                        break;
                    }
                    refresh = true;
                },
                _ = retry => refresh = true,
                _ = closed => return,
            }
        }
    }
}

pub(super) async fn send_catalog_failure(
    provider: &rmac_launcher_providers::ApplicationProvider,
    sender: &async_channel::Sender<CatalogUpdate>,
    detail: String,
) -> Result<(), async_channel::SendError<CatalogUpdate>> {
    sender
        .send(CatalogUpdate {
            health: CatalogHealth::Unavailable,
            revision: provider.revision(),
            changed: false,
            detail: Some(detail),
        })
        .await
}

pub(super) async fn wait_or_closed<T>(sender: &async_channel::Sender<T>, duration: Duration) {
    let timer = futures_util::FutureExt::fuse(async_io::Timer::after(duration));
    let closed = futures_util::FutureExt::fuse(sender.closed());
    futures_util::pin_mut!(timer, closed);
    futures_util::select! {
        _ = timer => {},
        _ = closed => {},
    }
}

#[derive(Clone, Debug)]
pub enum SettingsUpdate {
    Snapshot(Box<rmac_shell_settings::ShellSettings>),
    Unavailable(String),
}

/// Watch the complete C4 authority without a load/watch race. Consumers retain
/// their last-good settings across failures and rebuild provider scope only
/// from complete snapshots.
pub async fn watch_shell_settings(sender: async_channel::Sender<SettingsUpdate>) {
    loop {
        let setup = blocking::unblock(|| {
            let store = rmac_shell_settings::ShellSettingsStore::from_environment()?;
            let watcher = store.watch()?;
            let snapshot = store.load()?;
            Ok::<_, rmac_shell_settings::Error>((store, watcher, snapshot.settings))
        })
        .await;
        let (mut store, watcher, initial) = match setup {
            Ok(setup) => setup,
            Err(error) => {
                if sender
                    .send(SettingsUpdate::Unavailable(error.to_string()))
                    .await
                    .is_err()
                {
                    return;
                }
                wait_or_closed(&sender, Duration::from_secs(1)).await;
                if sender.is_closed() {
                    return;
                }
                continue;
            }
        };
        if sender
            .send(SettingsUpdate::Snapshot(Box::new(initial)))
            .await
            .is_err()
        {
            return;
        }
        loop {
            let changed = futures_util::FutureExt::fuse(watcher.recv());
            let closed = futures_util::FutureExt::fuse(sender.closed());
            futures_util::pin_mut!(changed, closed);
            let event = futures_util::select! {
                event = changed => event,
                _ = closed => return,
            };
            match event {
                Ok(rmac_shell_settings::StoreEvent::Changed) => {
                    let (returned_store, loaded) = blocking::unblock(move || {
                        let loaded = store.load().map(|snapshot| snapshot.settings);
                        (store, loaded)
                    })
                    .await;
                    store = returned_store;
                    let update = loaded.map_or_else(
                        |error| SettingsUpdate::Unavailable(error.to_string()),
                        |settings| SettingsUpdate::Snapshot(Box::new(settings)),
                    );
                    if sender.send(update).await.is_err() {
                        return;
                    }
                }
                Ok(rmac_shell_settings::StoreEvent::WatchError(error)) => {
                    if sender
                        .send(SettingsUpdate::Unavailable(error.to_string()))
                        .await
                        .is_err()
                    {
                        return;
                    }
                    break;
                }
                Err(error) => {
                    if sender
                        .send(SettingsUpdate::Unavailable(error.to_string()))
                        .await
                        .is_err()
                    {
                        return;
                    }
                    break;
                }
            }
        }
    }
}
