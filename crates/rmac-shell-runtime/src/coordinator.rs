use rmac_shell_status_linux::Sources;

use crate::{HealthSnapshot, ServiceReader, Snapshot, SourceHealth};

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

    pub fn apply_notifications(
        &mut self,
        notifications: Result<rmac_notifications::Indicator, String>,
    ) -> bool {
        let before = self.snapshot();
        match notifications {
            Ok(notifications) => {
                self.status.apply(rmac_shell_status::Event::Notifications(
                    rmac_shell_status::NotificationIndicator {
                        unread_count: notifications.unread_count,
                        has_urgent: notifications.has_urgent,
                    },
                ));
                self.health.notifications = SourceHealth::Healthy;
            }
            Err(detail) => {
                self.health.notifications = SourceHealth::Unavailable { detail };
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

    pub(crate) fn apply_service_batch(&mut self, batch: ServiceBatch) -> bool {
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

pub(crate) struct ServiceBatch {
    network: Option<Result<NetworkBatch, String>>,
    bluetooth: Option<Result<rmac_bluetooth::Snapshot, String>>,
    audio: Option<Result<rmac_audio::Snapshot, String>>,
    power: Option<Result<rmac_power::Snapshot, String>>,
}

pub(crate) fn read_service_batch(sources: Sources, reader: &impl ServiceReader) -> ServiceBatch {
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
