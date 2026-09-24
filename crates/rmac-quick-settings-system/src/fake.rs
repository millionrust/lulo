//! In-memory [`Backend`] double for quick-settings app and view-model tests.
//!
//! `FakeBackend` never touches Wi-Fi, Bluetooth, audio, power or Focus. It
//! records every call it receives (in order, as a short label) and can be
//! told to fail the next mutation or the next refresh, so callers can
//! assert the mutate-then-refresh sequencing in [`crate::execute`] without
//! a system bus. It is `!Sync` (interior `RefCell`s), which matches
//! `execute`'s single-threaded, blocking-executor contract.

use std::cell::RefCell;

use crate::Backend;

#[derive(Default)]
pub struct FakeBackend {
    calls: RefCell<Vec<String>>,
    fail_mutation: RefCell<Option<String>>,
    fail_refresh: RefCell<Option<String>>,
}

impl FakeBackend {
    pub fn new() -> Self {
        Self::default()
    }

    /// The next mutation call (`set_*` / `join_wifi`) returns this error
    /// instead of succeeding; the failure is consumed after one call.
    pub fn fail_next_mutation(&self, detail: impl Into<String>) {
        *self.fail_mutation.borrow_mut() = Some(detail.into());
    }

    /// The next refresh call (`wifi`, `bluetooth`, `audio`, `power`,
    /// `focus`) returns this error instead of succeeding; the failure is
    /// consumed after one call.
    pub fn fail_next_refresh(&self, detail: impl Into<String>) {
        *self.fail_refresh.borrow_mut() = Some(detail.into());
    }

    /// Every call this backend has received, in order, as a short label
    /// (for example `"set wifi false"`, `"read wifi"`).
    pub fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }

    fn mutation(&self, call: String) -> Result<(), String> {
        self.calls.borrow_mut().push(call);
        match self.fail_mutation.borrow_mut().take() {
            Some(detail) => Err(detail),
            None => Ok(()),
        }
    }

    fn refresh<T>(&self, call: &str, value: T) -> Result<T, String> {
        self.calls.borrow_mut().push(call.into());
        match self.fail_refresh.borrow_mut().take() {
            Some(detail) => Err(detail),
            None => Ok(value),
        }
    }
}

impl Backend for FakeBackend {
    fn set_wifi_enabled(&self, enabled: bool) -> Result<(), String> {
        self.mutation(format!("set wifi {enabled}"))
    }

    fn wifi(&self) -> Result<rmac_network::WifiSnapshot, String> {
        self.refresh(
            "read wifi",
            rmac_network::WifiSnapshot {
                available: true,
                enabled: false,
                ..Default::default()
            },
        )
    }

    fn join_wifi(&self, _network: &rmac_network::WifiNetworkId) -> Result<(), String> {
        self.mutation("join wifi".into())
    }

    fn set_bluetooth_powered(&self, powered: bool) -> Result<(), String> {
        self.mutation(format!("set bluetooth {powered}"))
    }

    fn set_bluetooth_device_connected(&self, device: &str, connected: bool) -> Result<(), String> {
        self.mutation(format!("connect {device} {connected}"))
    }

    fn set_default_output(&self, device: &str) -> Result<(), String> {
        self.mutation(format!("default output {device}"))
    }

    fn bluetooth(&self) -> Result<rmac_bluetooth::Snapshot, String> {
        self.refresh("read bluetooth", rmac_bluetooth::Snapshot::default())
    }

    fn set_output_volume(&self, volume: u8) -> Result<(), String> {
        self.mutation(format!("set volume {volume}"))
    }

    fn set_output_muted(&self, muted: bool) -> Result<(), String> {
        self.mutation(format!("set mute {muted}"))
    }

    fn audio(&self) -> Result<rmac_audio::Snapshot, String> {
        self.refresh(
            "read audio",
            rmac_audio::Snapshot {
                available: true,
                output: rmac_audio::Level {
                    volume: 64,
                    muted: false,
                },
                ..Default::default()
            },
        )
    }

    fn set_power_profile(&self, profile: rmac_power::PowerProfile) -> Result<(), String> {
        self.mutation(format!("set power {}", profile.id()))
    }

    fn power(&self) -> Result<rmac_power::Snapshot, String> {
        self.refresh("read power", rmac_power::Snapshot::default())
    }

    fn set_focus_enabled(&self, enabled: bool) -> Result<(), String> {
        self.mutation(format!("set focus {enabled}"))
    }

    fn focus(&self) -> Result<rmac_shell_settings::FocusSettings, String> {
        self.refresh(
            "read focus",
            rmac_shell_settings::FocusSettings {
                enabled: true,
                selected_mode: Some("Work".into()),
                ends_at_unix_ms: None,
            },
        )
    }
}
