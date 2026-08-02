use std::cell::RefCell;

use rmac_quick_settings::{Command, Control, Inputs, Operation};

use super::*;

#[derive(Default)]
struct FakeBackend {
    calls: RefCell<Vec<String>>,
    fail_mutation: RefCell<Option<String>>,
    fail_refresh: RefCell<Option<String>>,
}

impl FakeBackend {
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

    fn set_bluetooth_powered(&self, powered: bool) -> Result<(), String> {
        self.mutation(format!("set bluetooth {powered}"))
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

fn operation(command: Command) -> Operation {
    let mut inputs = Inputs::default();
    inputs.wifi.available = true;
    inputs.bluetooth.available = true;
    inputs.audio.available = true;
    inputs.audio.has_output = true;
    inputs.power.profiles.available = true;
    inputs.power.profiles.supported = vec![rmac_power::PowerProfile::Balanced];
    inputs.focus_available = true;
    rmac_quick_settings::State::new(inputs)
        .begin(command)
        .expect("fixture command is supported")
}

#[test]
fn mutation_is_followed_by_an_authoritative_refresh() {
    let backend = FakeBackend::default();
    let operation = operation(Command::SetOutputVolume(70));
    let inputs = execute(&operation, &backend).expect("command succeeds");
    assert_eq!(inputs.audio.output.volume, 64);
    assert_eq!(backend.calls.into_inner(), ["set volume 70", "read audio"]);
}

#[test]
fn mutation_failure_does_not_attempt_a_refresh() {
    let backend = FakeBackend::default();
    *backend.fail_mutation.borrow_mut() = Some("permission denied".into());
    let error =
        execute(&operation(Command::SetWifiEnabled(false)), &backend).expect_err("mutation fails");
    assert_eq!(error.phase, Phase::Mutate);
    assert_eq!(error.control, Control::Wifi);
    assert_eq!(backend.calls.into_inner(), ["set wifi false"]);
}

#[test]
fn refresh_failure_is_distinct_from_a_rejected_mutation() {
    let backend = FakeBackend::default();
    *backend.fail_refresh.borrow_mut() = Some("service restarted".into());
    let error = execute(&operation(Command::SetBluetoothPowered(false)), &backend)
        .expect_err("refresh fails");
    assert_eq!(error.phase, Phase::Refresh);
    assert_eq!(error.control, Control::Bluetooth);
    assert_eq!(
        backend.calls.into_inner(),
        ["set bluetooth false", "read bluetooth"]
    );
}

#[test]
fn every_control_targets_only_its_owned_authority() {
    let cases = [
        (
            Command::SetWifiEnabled(false),
            ["set wifi false", "read wifi"],
        ),
        (
            Command::SetBluetoothPowered(false),
            ["set bluetooth false", "read bluetooth"],
        ),
        (
            Command::SetOutputMuted(true),
            ["set mute true", "read audio"],
        ),
        (
            Command::SetPowerProfile(rmac_power::PowerProfile::Balanced),
            ["set power balanced", "read power"],
        ),
    ];

    for (command, expected) in cases {
        let backend = FakeBackend::default();
        execute(&operation(command), &backend).expect("command succeeds");
        assert_eq!(backend.calls.into_inner(), expected);
    }
}

#[test]
fn focus_result_is_marked_available_only_after_authority_reread() {
    let backend = FakeBackend::default();
    let inputs =
        execute(&operation(Command::SetFocusEnabled(true)), &backend).expect("Focus succeeds");
    assert!(inputs.focus_available);
    assert!(inputs.focus.enabled);
    assert_eq!(backend.calls.into_inner(), ["set focus true", "read focus"]);
}

#[test]
fn focus_system_backend_routes_to_the_live_authority() {
    let source = include_str!("backend.rs");
    assert!(source.contains("rmac_focus_linux::client::set_enabled"));
    assert!(source.contains("rmac_focus_linux::client::state"));
}
