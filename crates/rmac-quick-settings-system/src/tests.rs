use rmac_quick_settings::{Command, Control, Inputs, Operation};

use super::*;
use crate::fake::FakeBackend;

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
    assert_eq!(backend.calls(), ["set volume 70", "read audio"]);
}

#[test]
fn mutation_failure_does_not_attempt_a_refresh() {
    let backend = FakeBackend::default();
    backend.fail_next_mutation("permission denied");
    let error =
        execute(&operation(Command::SetWifiEnabled(false)), &backend).expect_err("mutation fails");
    assert_eq!(error.phase, Phase::Mutate);
    assert_eq!(error.control, Control::Wifi);
    assert_eq!(backend.calls(), ["set wifi false"]);
}

#[test]
fn refresh_failure_is_distinct_from_a_rejected_mutation() {
    let backend = FakeBackend::default();
    backend.fail_next_refresh("service restarted");
    let error = execute(&operation(Command::SetBluetoothPowered(false)), &backend)
        .expect_err("refresh fails");
    assert_eq!(error.phase, Phase::Refresh);
    assert_eq!(error.control, Control::Bluetooth);
    assert_eq!(backend.calls(), ["set bluetooth false", "read bluetooth"]);
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
        assert_eq!(backend.calls(), expected);
    }
}

#[test]
fn focus_result_is_marked_available_only_after_authority_reread() {
    let backend = FakeBackend::default();
    let inputs =
        execute(&operation(Command::SetFocusEnabled(true)), &backend).expect("Focus succeeds");
    assert!(inputs.focus_available);
    assert!(inputs.focus.enabled);
    assert_eq!(backend.calls(), ["set focus true", "read focus"]);
}

#[test]
fn focus_system_backend_routes_to_the_live_authority() {
    let source = include_str!("backend.rs");
    assert!(source.contains("rmac_focus_linux::client::set_enabled"));
    assert!(source.contains("rmac_focus_linux::client::state"));
}
