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
    pub focus: SourceHealth,
    pub network: SourceHealth,
    pub bluetooth: SourceHealth,
    pub audio: SourceHealth,
    pub power: SourceHealth,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub status: rmac_shell_status::Snapshot,
    pub quick_settings: rmac_quick_settings::Inputs,
    pub health: HealthSnapshot,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Update {
    pub snapshot: Snapshot,
    /// Whether the compact top bar should request a frame.
    pub visible: bool,
    /// Whether an open Quick Settings surface should request a frame.
    pub quick_settings_visible: bool,
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
    quick_settings: rmac_quick_settings::Inputs,
    health: HealthSnapshot,
}

impl Coordinator {
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            status: self.status.snapshot(),
            quick_settings: self.quick_settings.clone(),
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
                    .apply(rmac_shell_status::Event::Settings(settings.clone()));
                self.health.settings = SourceHealth::Healthy;
            }
            Err(detail) => {
                self.health.settings = SourceHealth::Unavailable { detail };
            }
        }
        before != self.snapshot()
    }

    pub fn apply_focus(
        &mut self,
        focus: Result<rmac_focus_runtime::Projection, String>,
        writable: bool,
    ) -> bool {
        let before = self.snapshot();
        match focus {
            Ok(focus) => {
                self.status.apply(rmac_shell_status::Event::Focus(Some(
                    rmac_shell_status::FocusIndicator {
                        enabled: focus.enabled,
                        mode: focus.mode_name.clone(),
                        ends_at_unix_ms: focus.ends_at_unix_ms,
                    },
                )));
                self.quick_settings.focus = rmac_shell_settings::FocusSettings {
                    enabled: focus.enabled,
                    selected_mode: focus.mode_name,
                    ends_at_unix_ms: focus.ends_at_unix_ms,
                };
                self.quick_settings.focus_available = writable;
                self.health.focus = SourceHealth::Healthy;
            }
            Err(detail) => {
                // Preserve last-known-good status while disabling mutations.
                self.quick_settings.focus_available = false;
                self.health.focus = SourceHealth::Unavailable { detail };
            }
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
        set_quick_settings_unavailable(&mut self.quick_settings, sources);
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
                    self.quick_settings.wifi = wifi.clone();
                    self.status
                        .apply(rmac_shell_status::Event::Network(network));
                    self.status.apply(rmac_shell_status::Event::Wifi(wifi));
                    self.status.apply(rmac_shell_status::Event::Vpn(vpn));
                    self.health.network = SourceHealth::Healthy;
                }
                Err(detail) => {
                    self.quick_settings.wifi.available = false;
                    self.health.network = SourceHealth::Unavailable { detail };
                }
            }
        }
        if let Some(result) = batch.bluetooth {
            match result {
                Ok(snapshot) => {
                    self.quick_settings.bluetooth = snapshot.clone();
                    self.status
                        .apply(rmac_shell_status::Event::Bluetooth(snapshot));
                    self.health.bluetooth = SourceHealth::Healthy;
                }
                Err(detail) => {
                    self.quick_settings.bluetooth.available = false;
                    self.health.bluetooth = SourceHealth::Unavailable { detail };
                }
            }
        }
        if let Some(result) = batch.audio {
            match result {
                Ok(snapshot) => {
                    self.quick_settings.audio = snapshot.clone();
                    self.status.apply(rmac_shell_status::Event::Audio(snapshot));
                    self.health.audio = SourceHealth::Healthy;
                }
                Err(detail) => {
                    self.quick_settings.audio.available = false;
                    self.health.audio = SourceHealth::Unavailable { detail };
                }
            }
        }
        if let Some(result) = batch.power {
            match result {
                Ok(snapshot) => {
                    self.quick_settings.power = snapshot.clone();
                    self.status.apply(rmac_shell_status::Event::Power(snapshot));
                    self.health.power = SourceHealth::Healthy;
                }
                Err(detail) => {
                    self.quick_settings.power.profiles.available = false;
                    self.health.power = SourceHealth::Unavailable { detail };
                }
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

fn set_quick_settings_unavailable(inputs: &mut rmac_quick_settings::Inputs, sources: Sources) {
    if sources.network {
        inputs.wifi.available = false;
    }
    if sources.bluetooth {
        inputs.bluetooth.available = false;
    }
    if sources.audio {
        inputs.audio.available = false;
    }
    if sources.power {
        inputs.power.profiles.available = false;
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
    let (focus_tx, focus_rx) = async_channel::bounded(4);

    let compositor = watch_compositor(compositor_tx);
    let services = rmac_shell_status_linux::watch(service_tx);
    let settings = watch_settings(settings_tx);
    let focus = watch_focus(focus_tx);
    let consumer = consume(sender, compositor_rx, service_rx, settings_rx, focus_rx);
    let (_, _, _, _, _) = futures_util::try_join!(
        compositor,
        async {
            services
                .await
                .map_err(|error| Error::new("watch Linux services", error.to_string()))
        },
        settings,
        focus,
        consumer,
    )?;
    Ok(())
}

async fn watch_focus(
    sender: Sender<Result<rmac_focus_runtime::Projection, String>>,
) -> Result<(), Error> {
    loop {
        match watch_focus_once(&sender).await {
            Ok(()) if sender.is_closed() => return Ok(()),
            Ok(()) => {
                if sender
                    .send(Err("Focus runtime stopped; reconnecting".into()))
                    .await
                    .is_err()
                {
                    return Ok(());
                }
            }
            Err(error) => {
                if sender.send(Err(error.to_string())).await.is_err() {
                    return Ok(());
                }
            }
        }
        wait_or_closed(&sender, std::time::Duration::from_secs(1)).await;
        if sender.is_closed() {
            return Ok(());
        }
    }
}

async fn watch_focus_once(
    sender: &Sender<Result<rmac_focus_runtime::Projection, String>>,
) -> Result<(), Error> {
    let store = rmac_focus_store::Store::from_environment()
        .map_err(|error| Error::new("resolve Focus settings", error.to_string()))?;
    let clock = rmac_focus_runtime::ClockSampler::default();
    let initial_clock = clock.sample();
    let (mut runtime, mut update) = rmac_focus_runtime::Runtime::load(store, initial_clock)
        .map_err(|error| Error::new("load Focus policy", format!("{error:?}")))?;
    if sender.send(Ok(update.projection.clone())).await.is_err() {
        return Ok(());
    }

    let (hint_tx, hint_rx) = async_channel::bounded(8);
    let watcher = rmac_focus_linux::watch(hint_tx);
    let evaluator = async {
        loop {
            let now = clock.sample();
            let delay = rmac_focus_runtime::wake_delay(&update, now.unix_ms)
                .unwrap_or(std::time::Duration::from_secs(24 * 60 * 60));
            let timer = futures_util::FutureExt::fuse(async_io::Timer::after(delay));
            let hint = futures_util::FutureExt::fuse(hint_rx.recv());
            let closed = futures_util::FutureExt::fuse(sender.closed());
            futures_util::pin_mut!(timer, hint, closed);
            let available = futures_util::select! {
                _ = timer => true,
                event = hint => match event {
                    Ok(rmac_focus_linux::Event::Refresh(_)) => true,
                    Ok(rmac_focus_linux::Event::Unavailable) => false,
                    Err(_) => return Ok(()),
                },
                _ = closed => return Ok(()),
            };
            if !available {
                if sender
                    .send(Err("Focus time-change watcher is reconnecting".into()))
                    .await
                    .is_err()
                {
                    return Ok(());
                }
                continue;
            }
            update = runtime
                .apply_clock(clock.sample())
                .map_err(|error| Error::new("evaluate Focus policy", format!("{error:?}")))?;
            if sender.send(Ok(update.projection.clone())).await.is_err() {
                return Ok(());
            }
        }
    };
    let (_, _) = futures_util::try_join!(
        async {
            watcher
                .await
                .map_err(|error| Error::new("watch Focus time state", error.to_string()))
        },
        evaluator,
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
    focus: async_channel::Receiver<Result<rmac_focus_runtime::Projection, String>>,
) -> Result<(), Error> {
    let mut coordinator = Coordinator::default();
    let mut published = coordinator.snapshot();
    if sender
        .send(Update {
            snapshot: published.clone(),
            visible: true,
            quick_settings_visible: true,
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
        let focus_event = futures_util::FutureExt::fuse(focus.recv());
        let closed = futures_util::FutureExt::fuse(sender.closed());
        futures_util::pin_mut!(
            compositor_event,
            service_event,
            settings_event,
            focus_event,
            closed
        );
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
            event = focus_event => {
                let event = event.map_err(|_| Error::new("receive Focus state", "watcher stopped"))?;
                // Policy is live, but mutation remains disabled until the
                // cross-process Focus command channel is connected.
                coordinator.apply_focus(event, false);
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
        quick_settings_visible: next.quick_settings != previous.quick_settings,
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
            coordinator.snapshot().quick_settings.audio.output.volume,
            67
        );
        assert!(!coordinator.snapshot().quick_settings.audio.available);
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
        assert!(!update.quick_settings_visible);
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
        assert!(!update.quick_settings_visible);
    }

    #[test]
    fn live_focus_authority_replaces_preferences_and_survives_source_loss() {
        let mut coordinator = Coordinator::default();
        let mut settings = rmac_shell_settings::ShellSettings::default();
        settings.focus.enabled = true;
        settings.focus.selected_mode = Some("Stale preference".into());
        coordinator.apply_settings(Ok(settings));
        assert_eq!(coordinator.snapshot().status.focus, None);
        assert!(!coordinator.snapshot().quick_settings.focus_available);

        coordinator.apply_focus(
            Ok(rmac_focus_runtime::Projection {
                enabled: true,
                mode_name: Some("Work".into()),
                ends_at_unix_ms: Some(5_000),
            }),
            true,
        );
        let live = coordinator.snapshot();
        assert_eq!(
            live.status.focus.as_ref().unwrap().mode.as_deref(),
            Some("Work")
        );
        assert!(live.quick_settings.focus_available);
        assert_eq!(live.health.focus, SourceHealth::Healthy);

        coordinator.apply_focus(Err("Focus service restarted".into()), false);
        let unavailable = coordinator.snapshot();
        assert_eq!(
            unavailable.status.focus.as_ref().unwrap().mode.as_deref(),
            Some("Work")
        );
        assert!(!unavailable.quick_settings.focus_available);
        assert!(matches!(
            unavailable.health.focus,
            SourceHealth::Unavailable { .. }
        ));
    }

    #[test]
    fn live_focus_projection_does_not_imply_a_writable_command_channel() {
        let mut coordinator = Coordinator::default();
        coordinator.apply_focus(
            Ok(rmac_focus_runtime::Projection {
                enabled: true,
                mode_name: Some("Work".into()),
                ends_at_unix_ms: None,
            }),
            false,
        );
        let snapshot = coordinator.snapshot();
        assert_eq!(snapshot.status.focus.unwrap().mode.as_deref(), Some("Work"));
        assert!(!snapshot.quick_settings.focus_available);
        assert_eq!(snapshot.health.focus, SourceHealth::Healthy);
    }

    #[test]
    fn quick_settings_change_does_not_redraw_the_compact_bar() {
        let previous = Snapshot::default();
        let mut next = previous.clone();
        next.quick_settings.audio = rmac_audio::Snapshot {
            available: true,
            output: rmac_audio::Level {
                volume: 55,
                muted: false,
            },
            ..Default::default()
        };

        let update = publication(&previous, next).expect("Quick Settings changed");
        assert!(!update.visible);
        assert!(update.quick_settings_visible);
    }
}
