//! Dock compositor, settings, catalog, places, and appearance watchers.

use super::*;

/// Publish coherent Dock snapshots until the receiving process closes.
pub async fn watch(sender: Sender<Update>) -> Result<(), Error> {
    let (compositor_tx, compositor_rx) = async_channel::bounded(64);
    let (settings_tx, settings_rx) = async_channel::bounded(2);
    let (catalog_tx, catalog_rx) = async_channel::bounded(2);
    let (places_tx, places_rx) = async_channel::bounded(2);
    let (appearance_tx, appearance_rx) = async_channel::bounded(2);

    let compositor = async {
        rmac_compositor_niri::watch(compositor_tx)
            .await
            .map_err(|error| Error::new("watch niri for the Dock", error.to_string()))
    };
    let settings = watch_settings(settings_tx);
    let catalog = watch_catalog(catalog_tx);
    let places = watch_places(places_tx);
    let appearance = watch_appearance(appearance_tx);
    let consumer = consume(
        sender,
        compositor_rx,
        settings_rx,
        catalog_rx,
        places_rx,
        appearance_rx,
    );
    let (_, _, _, _, _, _) =
        futures_util::try_join!(compositor, settings, catalog, places, appearance, consumer)?;
    Ok(())
}

async fn watch_appearance(sender: Sender<Result<bool, String>>) -> Result<(), Error> {
    loop {
        let setup = blocking::unblock(|| {
            let store = rmac_theme::ThemeStore::from_environment()?;
            let watcher = store.watch()?;
            Ok::<_, rmac_theme::Error>((store, watcher))
        })
        .await;
        let (store, watcher) = match setup {
            Ok(setup) => setup,
            Err(_) => {
                if sender
                    .send(Err("the Lulo OS appearance authority is unavailable".into()))
                    .await
                    .is_err()
                {
                    return Ok(());
                }
                wait_or_closed(&sender, Duration::from_secs(1)).await;
                continue;
            }
        };

        let (portal_tx, portal_rx) = async_channel::bounded(2);
        let portal = async {
            rmac_appearance_portal::watch(portal_tx.clone())
                .await
                .map_err(|_| Error::new("watch host appearance", "portal watcher stopped"))?;
            // The non-Linux adapter publishes one truthful unavailable
            // snapshot and returns. Keep its sender alive so the consumer can
            // still service theme-store changes and close deterministically.
            portal_tx.closed().await;
            Ok::<(), Error>(())
        };
        let consumer = consume_appearance(sender.clone(), store, watcher, portal_rx);
        let portal = futures_util::FutureExt::fuse(portal);
        let consumer = futures_util::FutureExt::fuse(consumer);
        let closed = futures_util::FutureExt::fuse(sender.closed());
        futures_util::pin_mut!(portal, consumer, closed);
        futures_util::select! {
            _ = portal => {},
            _ = consumer => {},
            _ = closed => return Ok(()),
        }
        if sender.is_closed() {
            return Ok(());
        }
        if sender
            .send(Err("the live appearance authority disconnected".into()))
            .await
            .is_err()
        {
            return Ok(());
        }
        wait_or_closed(&sender, Duration::from_secs(1)).await;
    }
}

async fn consume_appearance(
    sender: Sender<Result<bool, String>>,
    mut store: rmac_theme::ThemeStore,
    watcher: rmac_theme::ThemeWatcher,
    portal: async_channel::Receiver<rmac_appearance::Event>,
) -> Result<(), Error> {
    let mut host = None;
    loop {
        let portal_event = futures_util::FutureExt::fuse(portal.recv());
        let store_event = futures_util::FutureExt::fuse(watcher.recv());
        let closed = futures_util::FutureExt::fuse(sender.closed());
        futures_util::pin_mut!(portal_event, store_event, closed);
        match futures_util::select! {
            event = portal_event => AppearanceInput::Portal(event),
            event = store_event => AppearanceInput::Store(event),
            _ = closed => return Ok(()),
        } {
            AppearanceInput::Portal(Ok(rmac_appearance::Event::Snapshot(snapshot))) => {
                host = Some(snapshot);
            }
            AppearanceInput::Portal(Ok(rmac_appearance::Event::Unavailable(_))) => {
                if sender
                    .send(Err("the host appearance portal is unavailable".into()))
                    .await
                    .is_err()
                {
                    return Ok(());
                }
                continue;
            }
            AppearanceInput::Portal(Err(_)) => {
                return Err(Error::new("receive host appearance", "watcher stopped"));
            }
            AppearanceInput::Store(Ok(rmac_theme::StoreEvent::Changed)) => {}
            AppearanceInput::Store(Ok(rmac_theme::StoreEvent::WatchError(_)))
            | AppearanceInput::Store(Err(_)) => {
                return Err(Error::new("watch Lulo OS appearance", "watcher stopped"));
            }
        }

        let Some(current_host) = host.clone() else {
            continue;
        };
        let (returned_store, resolved) = blocking::unblock(move || {
            let resolved = store.load(&current_host);
            (store, resolved)
        })
        .await;
        store = returned_store;
        let result = resolved
            .map(|snapshot| snapshot.effective.motion == rmac_appearance::MotionPreference::Reduced)
            .map_err(|_| "the Lulo OS appearance preference could not be resolved".into());
        if sender.send(result).await.is_err() {
            return Ok(());
        }
    }
}

enum AppearanceInput {
    Portal(Result<rmac_appearance::Event, async_channel::RecvError>),
    Store(Result<rmac_theme::StoreEvent, async_channel::RecvError>),
}

async fn watch_places(
    sender: Sender<Result<rmac_places_system::Report, String>>,
) -> Result<(), Error> {
    loop {
        let loaded =
            blocking::unblock(|| rmac_places_system::snapshot(&rmac_places_system::SystemBackend))
                .await;
        let report = match loaded {
            Ok(report) => report,
            Err(error) => {
                if sender.send(Err(place_failure(&error))).await.is_err() {
                    return Ok(());
                }
                wait_or_closed(&sender, Duration::from_secs(1)).await;
                if sender.is_closed() {
                    return Ok(());
                }
                continue;
            }
        };

        let (changed_tx, changed_rx) = async_channel::bounded(1);
        let watch_report = report.clone();
        let watcher = blocking::unblock(move || {
            let callback_tx = changed_tx.clone();
            rmac_places_system::watch(&watch_report, move |event| {
                let _ = callback_tx.try_send(event);
            })
        })
        .await;
        let _watcher = match watcher {
            Ok(watcher) => watcher,
            Err(_) => {
                if sender.send(Ok(report)).await.is_err()
                    || sender
                        .send(Err("the user-place watcher is unavailable".into()))
                        .await
                        .is_err()
                {
                    return Ok(());
                }
                wait_or_closed(&sender, Duration::from_secs(1)).await;
                if sender.is_closed() {
                    return Ok(());
                }
                continue;
            }
        };

        // Close the snapshot-before-watch race. Once the watcher is installed,
        // verify the complete authority again; a mismatch restarts with a watch
        // set derived from the newer paths before anything is published.
        let verified =
            blocking::unblock(|| rmac_places_system::snapshot(&rmac_places_system::SystemBackend))
                .await;
        let verified = match verified {
            Ok(verified) if verified == report => verified,
            Ok(_) => continue,
            Err(error) => {
                if sender.send(Ok(report)).await.is_err()
                    || sender.send(Err(place_failure(&error))).await.is_err()
                {
                    return Ok(());
                }
                wait_or_closed(&sender, Duration::from_secs(1)).await;
                continue;
            }
        };
        if sender.send(Ok(verified)).await.is_err() {
            return Ok(());
        }

        // Filesystem notifications cover ordinary mutations. A slow bounded
        // reconciliation also catches mount-table backends that do not emit a
        // usable notification; unchanged snapshots never request a Dock frame.
        let changed = futures_util::FutureExt::fuse(changed_rx.recv());
        let reconcile =
            futures_util::FutureExt::fuse(async_io::Timer::after(Duration::from_secs(60)));
        let closed = futures_util::FutureExt::fuse(sender.closed());
        futures_util::pin_mut!(changed, reconcile, closed);
        futures_util::select! {
            event = changed => {
                if matches!(event, Ok(rmac_places_system::WatchEvent::Failed { .. }))
                    && sender.send(Err("the user-place watcher stopped unexpectedly".into())).await.is_err()
                {
                    return Ok(());
                }
            },
            _ = reconcile => {},
            _ = closed => return Ok(()),
        }
    }
}

fn place_failure(error: &rmac_places_system::Error) -> String {
    match error.operation {
        rmac_places_system::Operation::ResolveHome => "the home directory authority is unavailable",
        rmac_places_system::Operation::ReadUserDirs => {
            "the XDG user-directory authority is unavailable"
        }
        rmac_places_system::Operation::InspectPlace => "a configured user directory is unavailable",
        rmac_places_system::Operation::InspectTrash => "the desktop Trash authority is unavailable",
        rmac_places_system::Operation::WatchPlaces => "the user-place watcher is unavailable",
        rmac_places_system::Operation::OpenDownloads
        | rmac_places_system::Operation::EmptyTrash => "a Dock place operation failed",
    }
    .into()
}

async fn watch_settings(
    sender: Sender<Result<rmac_shell_settings::ShellSettings, String>>,
) -> Result<(), Error> {
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
                if sender.send(Err(error.to_string())).await.is_err() {
                    return Ok(());
                }
                wait_or_closed(&sender, Duration::from_secs(1)).await;
                if sender.is_closed() {
                    return Ok(());
                }
                continue;
            }
        };
        if sender.send(Ok(initial)).await.is_err() {
            return Ok(());
        }
        loop {
            let changed = futures_util::FutureExt::fuse(watcher.recv());
            let closed = futures_util::FutureExt::fuse(sender.closed());
            futures_util::pin_mut!(changed, closed);
            let event = futures_util::select! {
                event = changed => event,
                _ = closed => return Ok(()),
            };
            match event {
                Ok(rmac_shell_settings::StoreEvent::Changed) => {
                    let (returned_store, loaded) = blocking::unblock(move || {
                        let loaded = store.load().map(|snapshot| snapshot.settings);
                        (store, loaded)
                    })
                    .await;
                    store = returned_store;
                    if sender
                        .send(loaded.map_err(|error| error.to_string()))
                        .await
                        .is_err()
                    {
                        return Ok(());
                    }
                }
                Ok(rmac_shell_settings::StoreEvent::WatchError(error)) => {
                    if sender.send(Err(error.to_string())).await.is_err() {
                        return Ok(());
                    }
                    break;
                }
                Err(_) => break,
            }
        }
    }
}

async fn watch_catalog(
    sender: Sender<Result<Vec<rmac_apps::Application>, String>>,
) -> Result<(), Error> {
    loop {
        let (changed_tx, changed_rx) = async_channel::bounded(1);
        let setup = blocking::unblock(move || {
            let callback_tx = changed_tx.clone();
            rmac_apps::watch_catalog(move || {
                let _ = callback_tx.try_send(());
            })
            .map(|watcher| (watcher, changed_rx))
        })
        .await;
        let (watcher, changed) = match setup {
            Ok(setup) => setup,
            Err(error) => {
                if sender.send(Err(error.to_string())).await.is_err() {
                    return Ok(());
                }
                wait_or_closed(&sender, Duration::from_secs(1)).await;
                if sender.is_closed() {
                    return Ok(());
                }
                continue;
            }
        };
        let _watcher = watcher;
        let mut retry = true;
        loop {
            if retry {
                let result = blocking::unblock(rmac_apps::discover).await;
                retry = result.is_err();
                if sender
                    .send(result.map_err(|error| error.to_string()))
                    .await
                    .is_err()
                {
                    return Ok(());
                }
            }

            let changed_event = futures_util::FutureExt::fuse(changed.recv());
            let should_retry = retry;
            let retry_timer = futures_util::FutureExt::fuse(async move {
                if should_retry {
                    async_io::Timer::after(Duration::from_secs(1)).await;
                } else {
                    std::future::pending::<()>().await;
                }
            });
            let closed = futures_util::FutureExt::fuse(sender.closed());
            futures_util::pin_mut!(changed_event, retry_timer, closed);
            futures_util::select! {
                event = changed_event => {
                    if event.is_err() {
                        break;
                    }
                    retry = true;
                },
                _ = retry_timer => retry = true,
                _ = closed => return Ok(()),
            }
        }
    }
}

async fn wait_or_closed<T>(sender: &Sender<T>, duration: Duration) {
    let timer = futures_util::FutureExt::fuse(async_io::Timer::after(duration));
    let closed = futures_util::FutureExt::fuse(sender.closed());
    futures_util::pin_mut!(timer, closed);
    futures_util::select! {
        _ = timer => {},
        _ = closed => {},
    }
}
