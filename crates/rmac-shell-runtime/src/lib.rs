//! Live, last-known-good orchestration for shell status consumers.

use std::fmt;

use async_channel::Sender;
use rmac_shell_status_linux::Sources;

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
    pub network: SourceHealth,
    pub bluetooth: SourceHealth,
    pub audio: SourceHealth,
    pub power: SourceHealth,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Snapshot {
    pub status: rmac_shell_status::Snapshot,
    pub health: HealthSnapshot,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Update {
    pub snapshot: Snapshot,
    /// Whether a shell surface should request a frame for this publication.
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
    status: rmac_shell_status::State,
    health: HealthSnapshot,
}

impl Coordinator {
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            status: self.status.snapshot(),
            health: self.health.clone(),
        }
    }

    pub fn apply_compositor(&mut self, event: rmac_compositor::Event) -> bool {
        let before = self.snapshot();
        match event {
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
                self.status.apply(rmac_shell_status::Event::Compositor(
                    rmac_compositor::Event::ConnectionChanged { state },
                ));
            }
            event => {
                self.status
                    .apply(rmac_shell_status::Event::Compositor(event));
            }
        }
        before != self.snapshot()
    }

    pub fn apply_settings(
        &mut self,
        settings: Result<rmac_shell_settings::ShellSettings, String>,
    ) -> bool {
        let before = self.snapshot();
        match settings {
            Ok(settings) => {
                self.status
                    .apply(rmac_shell_status::Event::Settings(settings));
                self.health.settings = SourceHealth::Healthy;
            }
            Err(detail) => self.health.settings = SourceHealth::Unavailable { detail },
        }
        before != self.snapshot()
    }

    pub fn apply_service_unavailable(&mut self, sources: Sources, detail: String) -> bool {
        let before = self.snapshot();
        set_health(
            &mut self.health,
            sources,
            SourceHealth::Unavailable { detail },
        );
        before != self.snapshot()
    }

    pub fn refresh_services(&mut self, sources: Sources, reader: &impl ServiceReader) -> bool {
        self.apply_service_batch(read_service_batch(sources, reader))
    }

    fn apply_service_batch(&mut self, batch: ServiceBatch) -> bool {
        let before = self.snapshot();
        if let Some(result) = batch.network {
            match result {
                Ok((network, wifi, vpn)) => {
                    self.status
                        .apply(rmac_shell_status::Event::Network(network));
                    self.status.apply(rmac_shell_status::Event::Wifi(wifi));
                    self.status.apply(rmac_shell_status::Event::Vpn(vpn));
                    self.health.network = SourceHealth::Healthy;
                }
                Err(detail) => {
                    self.health.network = SourceHealth::Unavailable { detail };
                }
            }
        }
        if let Some(result) = batch.bluetooth {
            match result {
                Ok(snapshot) => {
                    self.status
                        .apply(rmac_shell_status::Event::Bluetooth(snapshot));
                    self.health.bluetooth = SourceHealth::Healthy;
                }
                Err(detail) => {
                    self.health.bluetooth = SourceHealth::Unavailable { detail };
                }
            }
        }
        if let Some(result) = batch.audio {
            match result {
                Ok(snapshot) => {
                    self.status.apply(rmac_shell_status::Event::Audio(snapshot));
                    self.health.audio = SourceHealth::Healthy;
                }
                Err(detail) => self.health.audio = SourceHealth::Unavailable { detail },
            }
        }
        if let Some(result) = batch.power {
            match result {
                Ok(snapshot) => {
                    self.status.apply(rmac_shell_status::Event::Power(snapshot));
                    self.health.power = SourceHealth::Healthy;
                }
                Err(detail) => self.health.power = SourceHealth::Unavailable { detail },
            }
        }
        before != self.snapshot()
    }
}

type NetworkBatch = (
    rmac_network::NetworkSnapshot,
    rmac_network::WifiSnapshot,
    rmac_network::VpnSnapshot,
);

struct ServiceBatch {
    network: Option<Result<NetworkBatch, String>>,
    bluetooth: Option<Result<rmac_bluetooth::Snapshot, String>>,
    audio: Option<Result<rmac_audio::Snapshot, String>>,
    power: Option<Result<rmac_power::Snapshot, String>>,
}

fn read_service_batch(sources: Sources, reader: &impl ServiceReader) -> ServiceBatch {
    ServiceBatch {
        network: sources.network.then(|| reader.network()),
        bluetooth: sources.bluetooth.then(|| reader.bluetooth()),
        audio: sources.audio.then(|| reader.audio()),
        power: sources.power.then(|| reader.power()),
    }
}

fn set_health(health: &mut HealthSnapshot, sources: Sources, state: SourceHealth) {
    if sources.network {
        health.network = state.clone();
    }
    if sources.bluetooth {
        health.bluetooth = state.clone();
    }
    if sources.audio {
        health.audio = state.clone();
    }
    if sources.power {
        health.power = state;
    }
}

pub trait ServiceReader: Send + Sync + 'static {
    fn network(
        &self,
    ) -> Result<
        (
            rmac_network::NetworkSnapshot,
            rmac_network::WifiSnapshot,
            rmac_network::VpnSnapshot,
        ),
        String,
    >;
    fn bluetooth(&self) -> Result<rmac_bluetooth::Snapshot, String>;
    fn audio(&self) -> Result<rmac_audio::Snapshot, String>;
    fn power(&self) -> Result<rmac_power::Snapshot, String>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemServiceReader;

impl ServiceReader for SystemServiceReader {
    fn network(
        &self,
    ) -> Result<
        (
            rmac_network::NetworkSnapshot,
            rmac_network::WifiSnapshot,
            rmac_network::VpnSnapshot,
        ),
        String,
    > {
        let network = rmac_network::network_snapshot().map_err(|error| error.to_string())?;
        let wifi = rmac_network::snapshot().map_err(|error| error.to_string())?;
        let vpn = rmac_network::vpn_snapshot().map_err(|error| error.to_string())?;
        Ok((network, wifi, vpn))
    }

    fn bluetooth(&self) -> Result<rmac_bluetooth::Snapshot, String> {
        rmac_bluetooth::snapshot().map_err(|error| error.to_string())
    }

    fn audio(&self) -> Result<rmac_audio::Snapshot, String> {
        rmac_audio::snapshot().map_err(|error| error.to_string())
    }

    fn power(&self) -> Result<rmac_power::Snapshot, String> {
        rmac_power::snapshot().map_err(|error| error.to_string())
    }
}

/// Publish coherent shell snapshots until the receiving side closes.
pub async fn watch(sender: Sender<Update>) -> Result<(), Error> {
    let (compositor_tx, compositor_rx) = async_channel::bounded(64);
    let (service_tx, service_rx) = async_channel::bounded(8);
    let (settings_tx, settings_rx) = async_channel::bounded(2);

    let compositor = watch_compositor(compositor_tx);
    let services = rmac_shell_status_linux::watch(service_tx);
    let settings = watch_settings(settings_tx);
    let consumer = consume(sender, compositor_rx, service_rx, settings_rx);
    let (_, _, _, _) = futures_util::try_join!(
        compositor,
        async {
            services
                .await
                .map_err(|error| Error::new("watch Linux services", error.to_string()))
        },
        settings,
        consumer,
    )?;
    Ok(())
}

async fn watch_compositor(sender: Sender<rmac_compositor::Event>) -> Result<(), Error> {
    loop {
        let watcher = futures_util::FutureExt::fuse(rmac_compositor_niri::watch(sender.clone()));
        let closed = futures_util::FutureExt::fuse(sender.closed());
        futures_util::pin_mut!(watcher, closed);
        futures_util::select! {
            result = watcher => match result {
                Ok(()) => return Ok(()),
                Err(error) => {
                    if sender.send(rmac_compositor::Event::ConnectionChanged {
                        state: rmac_compositor::ConnectionState::Disconnected,
                    }).await.is_err() {
                        return Ok(());
                    }
                    let _ = error;
                }
            },
            _ = closed => return Ok(()),
        }
        async_io::Timer::after(std::time::Duration::from_secs(1)).await;
    }
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
                wait_or_closed(&sender, std::time::Duration::from_secs(1)).await;
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
                }
                Err(_) => break,
            }
        }
    }
}

async fn wait_or_closed<T>(sender: &Sender<T>, duration: std::time::Duration) {
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
    services: async_channel::Receiver<rmac_shell_status_linux::Event>,
    settings: async_channel::Receiver<Result<rmac_shell_settings::ShellSettings, String>>,
) -> Result<(), Error> {
    let mut coordinator = Coordinator::default();
    let mut published = coordinator.snapshot();
    if sender
        .send(Update {
            snapshot: published.clone(),
            visible: true,
        })
        .await
        .is_err()
    {
        return Ok(());
    }
    loop {
        let compositor_event = futures_util::FutureExt::fuse(compositor.recv());
        let service_event = futures_util::FutureExt::fuse(services.recv());
        let settings_event = futures_util::FutureExt::fuse(settings.recv());
        let closed = futures_util::FutureExt::fuse(sender.closed());
        futures_util::pin_mut!(compositor_event, service_event, settings_event, closed);
        futures_util::select! {
            event = compositor_event => {
                let event = event.map_err(|_| Error::new("receive compositor state", "watcher stopped"))?;
                coordinator.apply_compositor(event);
            },
            event = service_event => {
                let event = event.map_err(|_| Error::new("receive Linux service state", "watcher stopped"))?;
                match event {
                    rmac_shell_status_linux::Event::Refresh(sources) => {
                        let batch = blocking::unblock(move || {
                            read_service_batch(sources, &SystemServiceReader)
                        }).await;
                        coordinator.apply_service_batch(batch);
                    }
                    rmac_shell_status_linux::Event::Unavailable { sources, detail } => {
                        coordinator.apply_service_unavailable(sources, detail);
                    }
                }
            },
            event = settings_event => {
                let event = event.map_err(|_| Error::new("receive shell settings", "watcher stopped"))?;
                coordinator.apply_settings(event);
            },
            _ = closed => return Ok(()),
        }
        let next = coordinator.snapshot();
        if let Some(update) = publication(&published, next) {
            let next = update.snapshot.clone();
            if sender.send(update).await.is_err() {
                return Ok(());
            }
            published = next;
        }
    }
}

fn publication(previous: &Snapshot, next: Snapshot) -> Option<Update> {
    (next != *previous).then(|| Update {
        visible: next.status != previous.status,
        snapshot: next,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeReader {
        audio: Result<rmac_audio::Snapshot, String>,
        power: Result<rmac_power::Snapshot, String>,
    }

    impl Default for FakeReader {
        fn default() -> Self {
            Self {
                audio: Ok(rmac_audio::Snapshot::default()),
                power: Ok(rmac_power::Snapshot::default()),
            }
        }
    }

    impl ServiceReader for FakeReader {
        fn network(
            &self,
        ) -> Result<
            (
                rmac_network::NetworkSnapshot,
                rmac_network::WifiSnapshot,
                rmac_network::VpnSnapshot,
            ),
            String,
        > {
            Err("network unavailable".into())
        }

        fn bluetooth(&self) -> Result<rmac_bluetooth::Snapshot, String> {
            Err("Bluetooth unavailable".into())
        }

        fn audio(&self) -> Result<rmac_audio::Snapshot, String> {
            self.audio.clone()
        }

        fn power(&self) -> Result<rmac_power::Snapshot, String> {
            self.power.clone()
        }
    }

    #[test]
    fn service_failure_retains_last_known_good_indicator() {
        let mut coordinator = Coordinator::default();
        let working = FakeReader {
            audio: Ok(rmac_audio::Snapshot {
                available: true,
                output: rmac_audio::Level {
                    volume: 67,
                    muted: false,
                },
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(coordinator.refresh_services(Sources::audio(), &working));
        let visible = coordinator.snapshot().status.sound;
        let failed = FakeReader {
            audio: Err("PipeWire restarted".into()),
            ..Default::default()
        };
        assert!(coordinator.refresh_services(Sources::audio(), &failed));
        assert_eq!(coordinator.snapshot().status.sound, visible);
        assert_eq!(
            coordinator.snapshot().health.audio,
            SourceHealth::Unavailable {
                detail: "PipeWire restarted".into()
            }
        );
    }

    #[test]
    fn duplicate_refresh_does_not_change_runtime_snapshot() {
        let mut coordinator = Coordinator::default();
        let reader = FakeReader {
            audio: Ok(rmac_audio::Snapshot::default()),
            ..Default::default()
        };
        assert!(coordinator.refresh_services(Sources::audio(), &reader));
        assert!(!coordinator.refresh_services(Sources::audio(), &reader));
    }

    #[test]
    fn hidden_service_failure_changes_health_without_erasing_state() {
        let mut coordinator = Coordinator::default();
        let mut settings = rmac_shell_settings::ShellSettings::default();
        settings.indicators.sound = false;
        coordinator.apply_settings(Ok(settings));
        assert!(
            coordinator.apply_service_unavailable(Sources::audio(), "monitor disconnected".into())
        );
        assert_eq!(coordinator.snapshot().status.sound, None);
    }

    #[test]
    fn health_only_publication_does_not_request_a_frame() {
        let previous = Snapshot::default();
        let mut next = previous.clone();
        next.health.audio = SourceHealth::Unavailable {
            detail: "PipeWire restarted".into(),
        };

        let update = publication(&previous, next).expect("health changed");
        assert!(!update.visible);
    }

    #[test]
    fn status_publication_requests_a_frame() {
        let previous = Snapshot::default();
        let mut next = previous.clone();
        next.status.focus = Some(rmac_shell_status::FocusIndicator {
            enabled: true,
            mode: Some("Work".into()),
            ends_at_unix_ms: None,
        });

        let update = publication(&previous, next).expect("status changed");
        assert!(update.visible);
    }
}
