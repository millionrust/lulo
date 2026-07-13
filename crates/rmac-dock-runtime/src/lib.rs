//! Event-driven, last-known-good orchestration for the Dock process.

use std::fmt;
use std::time::Duration;

use async_channel::Sender;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum SourceHealth {
    #[default]
    Starting,
    Healthy,
    Unavailable {
        detail: String,
    },
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HealthSnapshot {
    pub compositor: SourceHealth,
    pub settings: SourceHealth,
    pub catalog: SourceHealth,
    pub displays: SourceHealth,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Snapshot {
    pub model: rmac_dock::Model,
    pub outputs: Vec<rmac_compositor::OutputId>,
    pub health: HealthSnapshot,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Update {
    pub snapshot: Snapshot,
    pub visible: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error {
    operation: &'static str,
    detail: String,
}

impl Error {
    fn new(operation: &'static str, detail: impl Into<String>) -> Self {
        Self {
            operation,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "could not {}: {}", self.operation, self.detail)
    }
}

impl std::error::Error for Error {}

#[derive(Default)]
pub struct Coordinator {
    compositor: rmac_compositor::State,
    settings: rmac_shell_settings::ShellSettings,
    catalog: Vec<rmac_apps::Application>,
    primary_output: Option<rmac_compositor::OutputId>,
    health: HealthSnapshot,
}

impl Coordinator {
    pub fn snapshot(&self) -> Snapshot {
        let compositor = self.compositor.snapshot();
        Snapshot {
            model: rmac_dock::Model::build(
                &self.settings.pinned_apps,
                &self.settings.dock,
                &self.catalog,
                &compositor,
            ),
            outputs: rmac_dock::surface_outputs(
                &compositor,
                &self.settings.dock.outputs,
                self.primary_output.as_ref(),
            ),
            health: self.health.clone(),
        }
    }

    pub fn ready(&self) -> bool {
        !matches!(self.health.compositor, SourceHealth::Starting)
            && !matches!(self.health.settings, SourceHealth::Starting)
            && !matches!(self.health.catalog, SourceHealth::Starting)
            && (!matches!(
                self.settings.dock.outputs,
                rmac_shell_settings::OutputScope::Primary
            ) || !matches!(self.health.displays, SourceHealth::Starting))
    }

    pub fn apply_compositor(&mut self, event: rmac_compositor::Event) -> bool {
        let before = self.snapshot();
        match &event {
            rmac_compositor::Event::ConnectionChanged { state } => {
                self.health.compositor = match state {
                    rmac_compositor::ConnectionState::Connected => SourceHealth::Healthy,
                    rmac_compositor::ConnectionState::Connecting => SourceHealth::Starting,
                    rmac_compositor::ConnectionState::Disconnected => SourceHealth::Unavailable {
                        detail: "niri compositor events are disconnected".into(),
                    },
                    rmac_compositor::ConnectionState::Reconnecting => SourceHealth::Unavailable {
                        detail: "niri compositor events are reconnecting".into(),
                    },
                };
            }
            _ => self.health.compositor = SourceHealth::Healthy,
        }
        self.compositor.apply(event);
        before != self.snapshot()
    }

    pub fn apply_settings(
        &mut self,
        result: Result<rmac_shell_settings::ShellSettings, String>,
    ) -> bool {
        let before = self.snapshot();
        match result {
            Ok(settings) => {
                self.settings = settings;
                self.health.settings = SourceHealth::Healthy;
            }
            Err(detail) => self.health.settings = SourceHealth::Unavailable { detail },
        }
        before != self.snapshot()
    }

    pub fn apply_catalog(&mut self, result: Result<Vec<rmac_apps::Application>, String>) -> bool {
        let before = self.snapshot();
        match result {
            Ok(catalog) => {
                self.catalog = catalog;
                self.health.catalog = SourceHealth::Healthy;
            }
            Err(detail) => self.health.catalog = SourceHealth::Unavailable { detail },
        }
        before != self.snapshot()
    }

    pub fn set_primary_output(&mut self, output: Option<rmac_compositor::OutputId>) -> bool {
        let before = self.snapshot();
        self.primary_output = output;
        before != self.snapshot()
    }

    pub fn apply_primary_output(
        &mut self,
        result: Result<Option<rmac_compositor::OutputId>, String>,
    ) -> bool {
        let before = self.snapshot();
        match result {
            Ok(output) => {
                self.primary_output = output;
                self.health.displays = SourceHealth::Healthy;
            }
            Err(detail) => self.health.displays = SourceHealth::Unavailable { detail },
        }
        before != self.snapshot()
    }
}

/// Publish coherent Dock snapshots until the receiving process closes.
pub async fn watch(sender: Sender<Update>) -> Result<(), Error> {
    let (compositor_tx, compositor_rx) = async_channel::bounded(64);
    let (settings_tx, settings_rx) = async_channel::bounded(2);
    let (catalog_tx, catalog_rx) = async_channel::bounded(2);

    let compositor = async {
        rmac_compositor_niri::watch(compositor_tx)
            .await
            .map_err(|error| Error::new("watch niri for the Dock", error.to_string()))
    };
    let settings = watch_settings(settings_tx);
    let catalog = watch_catalog(catalog_tx);
    let consumer = consume(sender, compositor_rx, settings_rx, catalog_rx);
    let (_, _, _, _) = futures_util::try_join!(compositor, settings, catalog, consumer)?;
    Ok(())
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

async fn consume(
    sender: Sender<Update>,
    compositor: async_channel::Receiver<rmac_compositor::Event>,
    settings: async_channel::Receiver<Result<rmac_shell_settings::ShellSettings, String>>,
    catalog: async_channel::Receiver<Result<Vec<rmac_apps::Application>, String>>,
) -> Result<(), Error> {
    let mut coordinator = Coordinator::default();
    let mut published = None;
    loop {
        let compositor_event = futures_util::FutureExt::fuse(compositor.recv());
        let settings_event = futures_util::FutureExt::fuse(settings.recv());
        let catalog_event = futures_util::FutureExt::fuse(catalog.recv());
        let closed = futures_util::FutureExt::fuse(sender.closed());
        futures_util::pin_mut!(compositor_event, settings_event, catalog_event, closed);
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
            _ = closed => return Ok(()),
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

fn compositor_event_affects_displays(event: &rmac_compositor::Event) -> bool {
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

fn primary_output() -> Result<Option<rmac_compositor::OutputId>, String> {
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

fn publication(previous: Option<&Snapshot>, next: Snapshot) -> Update {
    Update {
        visible: previous.is_none_or(|previous| {
            previous.model != next.model || previous.outputs != next.outputs
        }),
        snapshot: next,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn app(id: &str) -> rmac_apps::Application {
        rmac_apps::Application {
            id: id.into(),
            name: id.trim_end_matches(".desktop").into(),
            generic_name: None,
            keywords: Vec::new(),
            source: PathBuf::from(format!("/apps/{id}")),
            icon: None,
            categories: Vec::new(),
            launch: rmac_apps::LaunchSpec::Command {
                program: id.into(),
                args: Vec::new(),
                working_dir: None,
                terminal: false,
            },
            actions: Vec::new(),
        }
    }

    #[test]
    fn coordinator_waits_for_every_source_to_resolve() {
        let mut coordinator = Coordinator::default();
        assert!(!coordinator.ready());
        coordinator.apply_settings(Ok(Default::default()));
        coordinator.apply_catalog(Err("catalog unavailable".into()));
        assert!(!coordinator.ready());
        coordinator.apply_compositor(rmac_compositor::Event::ConnectionChanged {
            state: rmac_compositor::ConnectionState::Disconnected,
        });
        assert!(coordinator.ready());
    }

    #[test]
    fn primary_scope_waits_for_display_authority() {
        let mut coordinator = Coordinator::default();
        let mut settings = rmac_shell_settings::ShellSettings::default();
        settings.dock.outputs = rmac_shell_settings::OutputScope::Primary;
        coordinator.apply_settings(Ok(settings));
        coordinator.apply_catalog(Ok(Vec::new()));
        coordinator.apply_compositor(rmac_compositor::Event::OutputsReplaced {
            outputs: vec![output("eDP-1"), output("DP-1")],
        });

        assert!(!coordinator.ready());
        coordinator.apply_primary_output(Ok(Some(rmac_compositor::OutputId::from("DP-1"))));

        assert!(coordinator.ready());
        assert_eq!(
            coordinator.snapshot().outputs,
            [rmac_compositor::OutputId::from("DP-1")]
        );
    }

    #[test]
    fn display_failure_retains_last_known_primary_output() {
        let mut coordinator = Coordinator::default();
        let mut settings = rmac_shell_settings::ShellSettings::default();
        settings.dock.outputs = rmac_shell_settings::OutputScope::Primary;
        coordinator.apply_settings(Ok(settings));
        coordinator.apply_compositor(rmac_compositor::Event::OutputsReplaced {
            outputs: vec![output("eDP-1"), output("DP-1")],
        });
        coordinator.apply_primary_output(Ok(Some(rmac_compositor::OutputId::from("eDP-1"))));

        coordinator.apply_primary_output(Err("display service unavailable".into()));

        assert_eq!(
            coordinator.snapshot().outputs,
            [rmac_compositor::OutputId::from("eDP-1")]
        );
        assert!(matches!(
            coordinator.snapshot().health.displays,
            SourceHealth::Unavailable { .. }
        ));
    }

    #[test]
    fn catalog_failure_retains_last_known_good_items() {
        let mut coordinator = Coordinator::default();
        let settings = rmac_shell_settings::ShellSettings {
            pinned_apps: vec![rmac_shell_settings::AppId("finder.desktop".into())],
            ..Default::default()
        };
        coordinator.apply_settings(Ok(settings));
        coordinator.apply_catalog(Ok(vec![app("finder.desktop")]));
        let item = coordinator.snapshot().model.items[0].clone();
        coordinator.apply_catalog(Err("filesystem watch failed".into()));
        assert_eq!(coordinator.snapshot().model.items[0], item);
        assert!(matches!(
            coordinator.snapshot().health.catalog,
            SourceHealth::Unavailable { .. }
        ));
    }

    #[test]
    fn settings_and_compositor_changes_rebuild_the_authoritative_model() {
        let mut coordinator = Coordinator::default();
        coordinator.apply_catalog(Ok(vec![app("terminal.desktop")]));
        let settings = rmac_shell_settings::ShellSettings {
            pinned_apps: vec![rmac_shell_settings::AppId("terminal.desktop".into())],
            ..Default::default()
        };
        coordinator.apply_settings(Ok(settings));
        assert!(!coordinator.snapshot().model.items[0].running);

        coordinator.apply_compositor(rmac_compositor::Event::WindowsReplaced {
            windows: vec![rmac_compositor::Window {
                id: rmac_compositor::WindowId(5),
                title: Some("Terminal".into()),
                app_id: Some("terminal".into()),
                pid: None,
                workspace: None,
                focused: true,
                floating: false,
                urgent: false,
                focus_timestamp: None,
                layout: Default::default(),
            }],
        });
        coordinator.apply_compositor(rmac_compositor::Event::FocusChanged {
            focus: rmac_compositor::FocusState {
                target: Some(rmac_compositor::FocusTarget::Window(
                    rmac_compositor::WindowId(5),
                )),
                window: Some(rmac_compositor::WindowId(5)),
                ..Default::default()
            },
        });
        assert!(coordinator.snapshot().model.items[0].running);
        assert!(coordinator.snapshot().model.items[0].active);
    }

    fn output(id: &str) -> rmac_compositor::Output {
        rmac_compositor::Output {
            id: id.into(),
            make: String::new(),
            model: String::new(),
            serial: None,
            physical_size_mm: None,
            modes: Vec::new(),
            current_mode: Some(0),
            custom_mode: false,
            vrr_supported: false,
            vrr_enabled: false,
            logical: Some(rmac_compositor::LogicalOutput {
                position: Default::default(),
                size: rmac_compositor::LogicalSize {
                    width: 1920.0,
                    height: 1080.0,
                },
                scale: 1.0,
                transform: "normal".into(),
            }),
        }
    }

    #[test]
    fn output_hotplug_changes_only_authoritative_surface_candidates() {
        let mut coordinator = Coordinator::default();
        coordinator.apply_compositor(rmac_compositor::Event::OutputsReplaced {
            outputs: vec![output("eDP-1")],
        });
        assert_eq!(
            coordinator.snapshot().outputs,
            [rmac_compositor::OutputId::from("eDP-1")]
        );
        coordinator.apply_compositor(rmac_compositor::Event::OutputsReplaced {
            outputs: Vec::new(),
        });
        assert!(coordinator.snapshot().outputs.is_empty());
    }

    #[test]
    fn health_only_publication_does_not_request_a_dock_frame() {
        let previous = Snapshot::default();
        let mut next = previous.clone();
        next.health.catalog = SourceHealth::Unavailable {
            detail: "catalog watcher restarted".into(),
        };
        let update = publication(Some(&previous), next);
        assert!(!update.visible);
    }

    #[test]
    fn display_refresh_hints_are_narrow_and_forward_compatible() {
        assert!(compositor_event_affects_displays(
            &rmac_compositor::Event::OutputsReplaced {
                outputs: Vec::new(),
            }
        ));
        assert!(compositor_event_affects_displays(
            &rmac_compositor::Event::Unknown {
                source_kind: "ConfigLoaded".into(),
                payload: Default::default(),
            }
        ));
        assert!(!compositor_event_affects_displays(
            &rmac_compositor::Event::WindowsReplaced {
                windows: Vec::new(),
            }
        ));
    }
}
