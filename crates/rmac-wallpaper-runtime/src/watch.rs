use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::{Coordinator, Error, Operation, Snapshot, SourceHealth, Update};

/// Publish render work only when the visible output plan changes. Health-only
/// changes remain available to diagnostics and never reopen or decode files.
pub async fn watch(sender: async_channel::Sender<Update>) -> Result<(), Error> {
    let (compositor_tx, compositor_rx) = async_channel::bounded(64);
    let (settings_tx, settings_rx) = async_channel::bounded(2);
    let compositor = async {
        rmac_compositor_niri::watch(compositor_tx)
            .await
            .map_err(|error| Error::new(Operation::WatchCompositor, error.to_string()))
    };
    let settings = watch_settings(settings_tx);
    let consumer = consume(sender, compositor_rx, settings_rx);
    let (_, _, _) = futures_util::try_join!(compositor, settings, consumer)?;
    Ok(())
}

async fn watch_settings(
    sender: async_channel::Sender<Result<rmac_shell_settings::ShellSettings, String>>,
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

async fn wait_or_closed<T>(sender: &async_channel::Sender<T>, duration: Duration) {
    let timer = futures_util::FutureExt::fuse(async_io::Timer::after(duration));
    let closed = futures_util::FutureExt::fuse(sender.closed());
    futures_util::pin_mut!(timer, closed);
    futures_util::select! {
        _ = timer => {},
        _ = closed => {},
    }
}

async fn consume(
    sender: async_channel::Sender<Update>,
    compositor: async_channel::Receiver<rmac_compositor::Event>,
    settings: async_channel::Receiver<Result<rmac_shell_settings::ShellSettings, String>>,
) -> Result<(), Error> {
    let mut coordinator = Coordinator::default();
    let mut published: Option<Snapshot> = None;
    let cache = std::sync::Arc::new(rmac_wallpaper_image::Cache::default());
    let (file_tx, file_rx) = async_channel::bounded(1);
    let mut _file_watcher = None;
    let mut file_paths: Vec<PathBuf> = Vec::new();
    loop {
        let compositor_event = futures_util::FutureExt::fuse(compositor.recv());
        let settings_event = futures_util::FutureExt::fuse(settings.recv());
        let file_event = futures_util::FutureExt::fuse(file_rx.recv());
        let closed = futures_util::FutureExt::fuse(sender.closed());
        futures_util::pin_mut!(compositor_event, settings_event, file_event, closed);
        let mut force_render = false;
        futures_util::select! {
            event = compositor_event => {
                let event = event.map_err(|_| Error::new(Operation::Consume, "compositor watcher stopped"))?;
                coordinator.apply_compositor(event);
            },
            event = settings_event => {
                let event = event.map_err(|_| Error::new(Operation::Consume, "settings watcher stopped"))?;
                coordinator.apply_settings(event);
            },
            event = file_event => match event {
                Ok(rmac_wallpaper_image::FileWatchEvent::Changed) => {
                    for path in &file_paths {
                        cache.invalidate_path(path);
                    }
                    coordinator.apply_file_health(SourceHealth::Healthy);
                    force_render = true;
                }
                Ok(rmac_wallpaper_image::FileWatchEvent::Failed { detail }) => {
                    coordinator.apply_file_health(SourceHealth::Unavailable { detail });
                }
                Err(_) => return Err(Error::new(Operation::Consume, "wallpaper file watcher stopped")),
            },
            _ = closed => return Ok(()),
        }
        if !coordinator.ready() {
            continue;
        }
        let mut next = coordinator.snapshot();
        let plan_changed = published
            .as_ref()
            .is_none_or(|previous| previous.plan != next.plan);
        let health_changed = published
            .as_ref()
            .is_none_or(|previous| previous.health != next.health);
        if plan_changed {
            file_paths = selected_file_paths(&next.plan);
            match rmac_wallpaper_image::watch_files(&file_paths, file_tx.clone()) {
                Ok(watcher) => {
                    _file_watcher = watcher;
                    coordinator.apply_file_health(SourceHealth::Healthy);
                }
                Err(error) => {
                    _file_watcher = None;
                    coordinator.apply_file_health(SourceHealth::Unavailable {
                        detail: error.detail().into(),
                    });
                }
            }
            next = coordinator.snapshot();
        }
        if plan_changed || force_render {
            let plan = next.plan.clone();
            let cache = cache.clone();
            let rasterization = blocking::unblock(move || {
                let rasterized = rmac_wallpaper_image::rasterize(&plan, &cache);
                (plan, rasterized)
            })
            .await;
            if sender
                .send(Update::Render {
                    plan: rasterization.0,
                    rasterized: rasterization.1,
                    health: next.health.clone(),
                })
                .await
                .is_err()
            {
                return Ok(());
            }
        } else if health_changed
            && sender
                .send(Update::Health(next.health.clone()))
                .await
                .is_err()
        {
            return Ok(());
        }
        published = Some(next);
    }
}

pub(crate) fn selected_file_paths(plan: &rmac_wallpaper::Plan) -> Vec<std::path::PathBuf> {
    plan.surfaces
        .iter()
        .filter_map(|surface| rmac_wallpaper::file_path(&surface.source).map(Path::to_path_buf))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
