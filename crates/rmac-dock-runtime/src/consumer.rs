//! Dock source-event consumption and coherent publication.

use super::*;

pub(super) async fn consume(
    sender: Sender<Update>,
    compositor: async_channel::Receiver<rmac_compositor::Event>,
    settings: async_channel::Receiver<Result<rmac_shell_settings::ShellSettings, String>>,
    catalog: async_channel::Receiver<Result<Vec<rmac_apps::Application>, String>>,
    places: async_channel::Receiver<Result<rmac_places_system::Report, String>>,
    appearance: async_channel::Receiver<Result<bool, String>>,
) -> Result<(), Error> {
    let mut coordinator = Coordinator::default();
    coordinator.restore_recents(blocking::unblock(load_recents).await);
    let mut published = None;
    loop {
        let compositor_event = futures_util::FutureExt::fuse(compositor.recv());
        let settings_event = futures_util::FutureExt::fuse(settings.recv());
        let catalog_event = futures_util::FutureExt::fuse(catalog.recv());
        let places_event = futures_util::FutureExt::fuse(places.recv());
        let appearance_event = futures_util::FutureExt::fuse(appearance.recv());
        let closed = futures_util::FutureExt::fuse(sender.closed());
        futures_util::pin_mut!(
            compositor_event,
            settings_event,
            catalog_event,
            places_event,
            appearance_event,
            closed
        );
        futures_util::select! {
            event = compositor_event => {
                let event = event.map_err(|_| Error::new("receive Dock compositor state", "watcher stopped"))?;
                let refresh_displays = compositor_event_affects_displays(&event);
                coordinator.apply_compositor(event);
                if refresh_displays {
                    let primary = blocking::unblock(primary_output).await;
                    coordinator.apply_primary_output(primary);
                }
            },
            event = settings_event => {
                let event = event.map_err(|_| Error::new("receive Dock settings", "watcher stopped"))?;
                coordinator.apply_settings(event);
            },
            event = catalog_event => {
                let event = event.map_err(|_| Error::new("receive application catalog", "watcher stopped"))?;
                coordinator.apply_catalog(event);
            },
            event = places_event => {
                let event = event.map_err(|_| Error::new("receive Dock user places", "watcher stopped"))?;
                coordinator.apply_places(event);
            },
            event = appearance_event => {
                let event = event.map_err(|_| Error::new("receive Dock appearance", "watcher stopped"))?;
                coordinator.apply_appearance(event);
            },
            _ = closed => return Ok(()),
        }

        if let Some(recents) = coordinator.take_dirty_recents() {
            if let Err(error) = blocking::unblock(move || save_recents(&recents)).await {
                eprintln!("could not save the Dock's recent apps: {error}");
            }
        }
        if !coordinator.ready() {
            continue;
        }
        let next = coordinator.snapshot();
        if published.as_ref() == Some(&next) {
            continue;
        }
        let update = publication(published.as_ref(), next);
        let next = update.snapshot.clone();
        if sender.send(update).await.is_err() {
            return Ok(());
        }
        published = Some(next);
    }
}

/// `$XDG_STATE_HOME/rmac/dock-recents`, one application ID per line.
fn recents_path() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|home| home.join(".local/state"))
        })
        .map(|state| state.join("rmac/dock-recents"))
}

pub(super) fn load_recents() -> Vec<String> {
    recents_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|contents| rmac_dock::recents::decode(&contents))
        .unwrap_or_default()
}

pub(super) fn save_recents(recents: &[String]) -> std::io::Result<()> {
    let Some(path) = recents_path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Write beside and rename so a crash never leaves a torn file.
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, rmac_dock::recents::encode(recents))?;
    std::fs::rename(&temporary, &path)
}

pub(super) fn compositor_event_affects_displays(event: &rmac_compositor::Event) -> bool {
    matches!(
        event,
        rmac_compositor::Event::Snapshot { .. }
            | rmac_compositor::Event::OutputsReplaced { .. }
            | rmac_compositor::Event::WorkspacesReplaced { .. }
    ) || matches!(
        event,
        rmac_compositor::Event::Unknown { source_kind, .. } if source_kind == "ConfigLoaded"
    )
}

pub(super) fn primary_output() -> Result<Option<rmac_compositor::OutputId>, String> {
    rmac_display::snapshot()
        .map(|snapshot| {
            snapshot
                .outputs
                .into_iter()
                .find(|output| output.primary && output.logical.is_some())
                .map(|output| rmac_compositor::OutputId(output.id))
        })
        .map_err(|error| error.to_string())
}

pub(super) fn publication(previous: Option<&Snapshot>, next: Snapshot) -> Update {
    Update {
        visible: previous.is_none_or(|previous| {
            previous.settings != next.settings
                || previous.model != next.model
                || previous.content != next.content
                || previous.outputs != next.outputs
                || previous.surface_plan != next.surface_plan
                || previous.overview_visible != next.overview_visible
                || previous.reduced_motion != next.reduced_motion
        }),
        snapshot: next,
    }
}
