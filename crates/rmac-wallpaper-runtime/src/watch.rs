use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::{Coordinator, Error, Operation, Snapshot, SourceHealth, Update};

/// Publish render work only when the visible output plan changes. Health-only
/// changes remain available to diagnostics and never reopen or decode files.
pub async fn watch(sender: async_channel::Sender<Update>) -> Result<(), Error> {
    let (compositor_tx, compositor_rx) = async_channel::bounded(64);
    let (settings_tx, settings_rx) = async_channel::bounded(2);
    let (appearance_tx, appearance_rx) = async_channel::bounded(2);
    let compositor = watch_compositor(compositor_tx);
    let settings = watch_settings(settings_tx);
    let appearance = watch_appearance(appearance_tx);
    let consumer = consume(sender, compositor_rx, settings_rx, appearance_rx);
    let (_, _, _, _) = futures_util::try_join!(compositor, settings, appearance, consumer)?;
    Ok(())
}

/// Not every session runs inside niri (a nested-Wayland test harness, for
/// one). A missing socket is reported as a disconnected compositor, the
/// same signal a connection that drops after it was once open already
/// sends, instead of aborting the whole wallpaper runtime: the settings
/// and appearance sources keep publishing, and `consume` keeps rendering
/// built-ins with an empty output plan (see `Coordinator::ready`, which
/// only waits on `Starting`, not `Unavailable`).
async fn watch_compositor(
    sender: async_channel::Sender<rmac_compositor::Event>,
) -> Result<(), Error> {
    loop {
        match rmac_compositor_niri::watch(sender.clone()).await {
            Ok(()) => return Ok(()),
            Err(rmac_compositor_niri::Error::MissingSocketPath) => {
                if sender
                    .send(rmac_compositor::Event::ConnectionChanged {
                        state: rmac_compositor::ConnectionState::Disconnected,
                    })
                    .await
                    .is_err()
                {
                    return Ok(());
                }
                wait_or_closed(&sender, Duration::from_secs(5)).await;
                if sender.is_closed() {
                    return Ok(());
                }
            }
            Err(error) => {
                return Err(Error::new(Operation::WatchCompositor, error.to_string()));
            }
        }
    }
}

/// Publish whether built-ins should be drawn dark, resolved the same way the
/// shell's design tokens are: the rmac theme preference over the host
/// Settings portal. Only a change is published; there is no polling.
async fn watch_appearance(sender: async_channel::Sender<bool>) -> Result<(), Error> {
    let (host_tx, host_rx) = async_channel::bounded(8);
    let host_source = async move {
        if let Err(error) = rmac_appearance_portal::watch(host_tx).await {
            eprintln!("wallpaper appearance portal stopped: {error}");
        }
        Ok::<(), Error>(())
    };
    let resolver = async move {
        let watcher = blocking::unblock(|| {
            rmac_theme::ThemeStore::from_environment().and_then(|store| store.watch())
        })
        .await;
        let mut theme = match watcher {
            Ok(watcher) => Some(watcher),
            Err(error) => {
                eprintln!("wallpaper cannot follow the theme preference: {error}");
                None
            }
        };
        let mut host_events = Some(host_rx);
        // Publish from the theme preference at once so the first frame never
        // waits on the portal; the portal snapshot refines it when it arrives.
        let mut host = rmac_appearance::Snapshot::unavailable("waiting for the Settings portal");
        let mut published = None;
        loop {
            let dark = resolve_dark(host.clone()).await;
            if published != Some(dark) {
                published = Some(dark);
                if sender.send(dark).await.is_err() {
                    return Ok::<(), Error>(());
                }
            }
            // The wait futures borrow the sources, so they are dropped before
            // a source that ended is cleared below.
            let wake = {
                let host_event = futures_util::FutureExt::fuse(async {
                    match &host_events {
                        Some(events) => events.recv().await.ok(),
                        None => futures_util::future::pending().await,
                    }
                });
                let theme_event = futures_util::FutureExt::fuse(async {
                    match &theme {
                        Some(watcher) => watcher.recv().await.ok(),
                        None => futures_util::future::pending().await,
                    }
                });
                let closed = futures_util::FutureExt::fuse(sender.closed());
                futures_util::pin_mut!(host_event, theme_event, closed);
                futures_util::select! {
                    event = host_event => AppearanceWake::Host(event),
                    event = theme_event => AppearanceWake::Theme(event),
                    _ = closed => AppearanceWake::Closed,
                }
            };
            match wake {
                AppearanceWake::Host(Some(rmac_appearance::Event::Snapshot(snapshot))) => {
                    host = snapshot;
                }
                AppearanceWake::Host(Some(rmac_appearance::Event::Unavailable(error))) => {
                    host = rmac_appearance::Snapshot::unavailable(error.to_string());
                }
                AppearanceWake::Host(None) => host_events = None,
                AppearanceWake::Theme(Some(rmac_theme::StoreEvent::Changed)) => {}
                AppearanceWake::Theme(Some(rmac_theme::StoreEvent::WatchError(error))) => {
                    eprintln!("wallpaper theme preference watch failed: {error}");
                    theme = None;
                }
                AppearanceWake::Theme(None) => theme = None,
                AppearanceWake::Closed => return Ok::<(), Error>(()),
            }
        }
    };
    let (_, _) = futures_util::try_join!(host_source, resolver)?;
    Ok(())
}

enum AppearanceWake {
    Host(Option<rmac_appearance::Event>),
    Theme(Option<rmac_theme::StoreEvent>),
    Closed,
}

async fn resolve_dark(host: rmac_appearance::Snapshot) -> bool {
    blocking::unblock(move || {
        match rmac_theme::ThemeStore::from_environment().and_then(|store| store.load(&host)) {
            Ok(snapshot) => {
                snapshot.effective.color_scheme == rmac_appearance::ResolvedColorScheme::Dark
            }
            Err(error) => {
                // A fresh rmac session is dark, so an unreadable preference
                // keeps the dark artwork unless the host asks for light.
                eprintln!("wallpaper cannot read the theme preference: {error}");
                host.color_scheme != rmac_appearance::ColorScheme::PreferLight
            }
        }
    })
    .await
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
    appearance: async_channel::Receiver<bool>,
) -> Result<(), Error> {
    let mut coordinator = Coordinator::default();
    let mut dark: Option<bool> = None;
    let mut published: Option<Snapshot> = None;
    let cache = std::sync::Arc::new(rmac_wallpaper_image::Cache::default());
    let (file_tx, file_rx) = async_channel::bounded(1);
    let mut _file_watcher = None;
    let mut file_paths: Vec<PathBuf> = Vec::new();
    loop {
        let compositor_event = futures_util::FutureExt::fuse(compositor.recv());
        let settings_event = futures_util::FutureExt::fuse(settings.recv());
        let file_event = futures_util::FutureExt::fuse(file_rx.recv());
        let appearance_event = futures_util::FutureExt::fuse(appearance.recv());
        let closed = futures_util::FutureExt::fuse(sender.closed());
        futures_util::pin_mut!(
            compositor_event,
            settings_event,
            file_event,
            appearance_event,
            closed
        );
        let mut force_render = false;
        futures_util::select! {
            event = appearance_event => {
                let event = event.map_err(|_| Error::new(Operation::Consume, "appearance watcher stopped"))?;
                // Only built-ins have light and dark forms; a user file is
                // not redrawn when the appearance changes.
                if dark.is_some_and(|previous| previous != event)
                    && published.as_ref().is_some_and(|previous| has_built_in(&previous.plan))
                {
                    force_render = true;
                }
                dark = Some(event);
            },
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
        let Some(dark) = dark.filter(|_| coordinator.ready()) else {
            continue;
        };
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
                let rasterized = rmac_wallpaper_image::rasterize_for(&plan, &cache, dark);
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

fn has_built_in(plan: &rmac_wallpaper::Plan) -> bool {
    plan.surfaces
        .iter()
        .any(|surface| matches!(surface.source, rmac_wallpaper::Source::BuiltIn(_)))
}

pub(crate) fn selected_file_paths(plan: &rmac_wallpaper::Plan) -> Vec<std::path::PathBuf> {
    plan.surfaces
        .iter()
        .filter_map(|surface| rmac_wallpaper::file_path(&surface.source).map(Path::to_path_buf))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
