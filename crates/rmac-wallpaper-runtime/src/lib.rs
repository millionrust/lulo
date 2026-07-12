//! Live, last-known-good orchestration for the wallpaper session process.

use std::fmt;
use std::time::Duration;

#[derive(Clone, Eq, PartialEq)]
pub enum SourceHealth {
    Starting,
    Healthy,
    Unavailable { detail: String },
}

impl fmt::Debug for SourceHealth {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Starting => formatter.write_str("Starting"),
            Self::Healthy => formatter.write_str("Healthy"),
            Self::Unavailable { .. } => formatter
                .debug_struct("Unavailable")
                .field("detail", &"<redacted>")
                .finish(),
        }
    }
}

impl SourceHealth {
    pub fn detail(&self) -> Option<&str> {
        match self {
            Self::Unavailable { detail } => Some(detail),
            Self::Starting | Self::Healthy => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HealthSnapshot {
    pub compositor: SourceHealth,
    pub settings: SourceHealth,
}

impl Default for HealthSnapshot {
    fn default() -> Self {
        Self {
            compositor: SourceHealth::Starting,
            settings: SourceHealth::Starting,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub plan: rmac_wallpaper::Plan,
    pub health: HealthSnapshot,
}

pub enum Update {
    Render {
        plan: rmac_wallpaper::Plan,
        resolved: rmac_wallpaper_system::Resolution,
        health: HealthSnapshot,
    },
    Health(HealthSnapshot),
}

impl fmt::Debug for Update {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Render {
                plan,
                resolved,
                health,
            } => formatter
                .debug_struct("Render")
                .field("outputs", &plan.surfaces.len())
                .field("plan_issues", &plan.issues)
                .field("resolution_issues", &resolved.issues)
                .field("health", health)
                .finish(),
            Self::Health(health) => formatter.debug_tuple("Health").field(health).finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    WatchCompositor,
    WatchSettings,
    Consume,
}

#[derive(Clone, Eq, PartialEq)]
pub struct Error {
    pub operation: Operation,
    detail: String,
}

impl Error {
    fn new(operation: Operation, detail: impl Into<String>) -> Self {
        Self {
            operation,
            detail: detail.into(),
        }
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Error")
            .field("operation", &self.operation)
            .field("detail", &"<redacted>")
            .finish()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Could not run the wallpaper service")
    }
}

impl std::error::Error for Error {}

#[derive(Default)]
pub struct Coordinator {
    compositor: rmac_compositor::State,
    settings: rmac_shell_settings::WallpaperSettings,
    health: HealthSnapshot,
}

impl Coordinator {
    pub fn ready(&self) -> bool {
        self.health.compositor != SourceHealth::Starting
            && self.health.settings != SourceHealth::Starting
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            plan: rmac_wallpaper::plan(&self.settings, &self.compositor.snapshot().outputs),
            health: self.health.clone(),
        }
    }

    pub fn apply_compositor(&mut self, event: rmac_compositor::Event) -> bool {
        let before = self.snapshot();
        match &event {
            rmac_compositor::Event::ConnectionChanged { state } => {
                self.health.compositor = match state {
                    rmac_compositor::ConnectionState::Connected => SourceHealth::Healthy,
                    rmac_compositor::ConnectionState::Connecting => SourceHealth::Starting,
                    rmac_compositor::ConnectionState::Disconnected => SourceHealth::Unavailable {
                        detail: "niri wallpaper output events are disconnected".into(),
                    },
                    rmac_compositor::ConnectionState::Reconnecting => SourceHealth::Unavailable {
                        detail: "niri wallpaper output events are reconnecting".into(),
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
                self.settings = settings.wallpaper;
                self.health.settings = SourceHealth::Healthy;
            }
            Err(detail) => self.health.settings = SourceHealth::Unavailable { detail },
        }
        before != self.snapshot()
    }
}

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
    loop {
        let compositor_event = futures_util::FutureExt::fuse(compositor.recv());
        let settings_event = futures_util::FutureExt::fuse(settings.recv());
        let closed = futures_util::FutureExt::fuse(sender.closed());
        futures_util::pin_mut!(compositor_event, settings_event, closed);
        futures_util::select! {
            event = compositor_event => {
                let event = event.map_err(|_| Error::new(Operation::Consume, "compositor watcher stopped"))?;
                coordinator.apply_compositor(event);
            },
            event = settings_event => {
                let event = event.map_err(|_| Error::new(Operation::Consume, "settings watcher stopped"))?;
                coordinator.apply_settings(event);
            },
            _ = closed => return Ok(()),
        }
        if !coordinator.ready() {
            continue;
        }
        let next = coordinator.snapshot();
        let plan_changed = published
            .as_ref()
            .is_none_or(|previous| previous.plan != next.plan);
        let health_changed = published
            .as_ref()
            .is_none_or(|previous| previous.health != next.health);
        if plan_changed {
            let plan = next.plan.clone();
            let resolution = blocking::unblock(move || {
                let resolved = rmac_wallpaper_system::resolve_plan(&plan);
                (plan, resolved)
            })
            .await;
            if sender
                .send(Update::Render {
                    plan: resolution.0,
                    resolved: resolution.1,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn output(id: &str) -> rmac_compositor::Output {
        rmac_compositor::Output {
            id: id.into(),
            make: "Test".into(),
            model: "Display".into(),
            serial: None,
            physical_size_mm: None,
            modes: vec![rmac_compositor::OutputMode {
                physical_size: rmac_compositor::PhysicalSize {
                    width: 1920,
                    height: 1080,
                },
                refresh_millihz: 60_000,
                preferred: true,
            }],
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

    fn settings(source: &str) -> rmac_shell_settings::ShellSettings {
        rmac_shell_settings::ShellSettings {
            wallpaper: rmac_shell_settings::WallpaperSettings {
                default: rmac_shell_settings::WallpaperSelection {
                    source: Some(source.into()),
                    fit: rmac_shell_settings::WallpaperFit::Fill,
                },
                per_output: Default::default(),
            },
            ..Default::default()
        }
    }

    #[test]
    fn waits_for_both_authorities_and_builds_hotplug_plans() {
        let mut coordinator = Coordinator::default();
        assert!(!coordinator.ready());
        coordinator.apply_settings(Ok(settings("builtin:rmac-aurora")));
        assert!(!coordinator.ready());
        coordinator.apply_compositor(rmac_compositor::Event::ConnectionChanged {
            state: rmac_compositor::ConnectionState::Connected,
        });
        assert!(coordinator.ready());
        coordinator.apply_compositor(rmac_compositor::Event::OutputsReplaced {
            outputs: vec![output("DP-2"), output("DP-1")],
        });
        assert_eq!(
            coordinator
                .snapshot()
                .plan
                .surfaces
                .iter()
                .map(|surface| surface.output.0.as_str())
                .collect::<Vec<_>>(),
            ["DP-1", "DP-2"]
        );
        coordinator.apply_compositor(rmac_compositor::Event::OutputsReplaced {
            outputs: vec![output("DP-1")],
        });
        assert_eq!(coordinator.snapshot().plan.surfaces.len(), 1);
    }

    #[test]
    fn source_failures_retain_last_known_good_visible_plan() {
        let mut coordinator = Coordinator::default();
        coordinator.apply_settings(Ok(settings("builtin:rmac-aurora")));
        coordinator.apply_compositor(rmac_compositor::Event::OutputsReplaced {
            outputs: vec![output("DP-1")],
        });
        let visible = coordinator.snapshot().plan;
        assert!(coordinator.apply_settings(Err("private settings path".into())));
        let failed = coordinator.snapshot();
        assert_eq!(failed.plan, visible);
        assert!(matches!(
            failed.health.settings,
            SourceHealth::Unavailable { .. }
        ));
        assert!(!format!("{:?}", failed.health).contains("private settings"));

        coordinator.apply_compositor(rmac_compositor::Event::ConnectionChanged {
            state: rmac_compositor::ConnectionState::Disconnected,
        });
        assert_eq!(coordinator.snapshot().plan, visible);
    }

    #[test]
    fn update_debug_redacts_health_and_file_sources() {
        let plan = rmac_wallpaper::Plan {
            surfaces: vec![rmac_wallpaper::Surface {
                output: "DP-1".into(),
                logical_size: rmac_compositor::LogicalSize {
                    width: 1.0,
                    height: 1.0,
                },
                scale: 1.0,
                fit: rmac_shell_settings::WallpaperFit::Fill,
                source: rmac_wallpaper::Source::File("/home/alex/private.png".into()),
            }],
            issues: Vec::new(),
        };
        let update = Update::Render {
            resolved: rmac_wallpaper_system::resolve_plan(&plan),
            plan,
            health: HealthSnapshot {
                compositor: SourceHealth::Unavailable {
                    detail: "secret socket path".into(),
                },
                settings: SourceHealth::Healthy,
            },
        };
        let debug = format!("{update:?}");
        assert!(!debug.contains("alex"));
        assert!(!debug.contains("secret socket"));
    }
}
