
use super::*;

fn available_inputs() -> Inputs {
    Inputs {
        wifi: rmac_network::WifiSnapshot {
            available: true,
            enabled: true,
            current_ssid: Some("Studio".into()),
            ..Default::default()
        },
        bluetooth: rmac_bluetooth::Snapshot {
            available: true,
            powered: true,
            ..Default::default()
        },
        audio: rmac_audio::Snapshot {
            available: true,
            has_output: true,
            output: rmac_audio::Level {
                volume: 42,
                muted: false,
            },
            ..Default::default()
        },
        power: rmac_power::Snapshot {
            profiles: rmac_power::Profiles {
                available: true,
                active: Some(rmac_power::PowerProfile::Balanced),
                supported: vec![
                    rmac_power::PowerProfile::PowerSaver,
                    rmac_power::PowerProfile::Balanced,
                ],
                performance_degraded: None,
            },
            ..Default::default()
        },
        focus: rmac_shell_settings::FocusSettings {
            enabled: false,
            selected_mode: Some("Work".into()),
            ends_at_unix_ms: None,
        },
        focus_available: true,
    }
}

#[test]
fn unavailable_control_rejects_a_mutation() {
    let mut state = State::default();
    assert_eq!(
        state.begin(Command::SetWifiEnabled(true)),
        Err(StartError::Unavailable(Control::Wifi))
    );
    assert_eq!(
        state.begin(Command::SetFocusEnabled(true)),
        Err(StartError::Unavailable(Control::Focus))
    );
}

#[test]
fn sound_requires_an_authoritative_default_output() {
    let mut inputs = available_inputs();
    inputs.audio.has_output = false;
    let mut state = State::new(inputs);
    let view = state.view();
    assert!(!view.sound.available);
    assert_eq!(view.sound.summary, "No Output Device");
    assert_eq!(
        state.begin(Command::SetOutputMuted(true)),
        Err(StartError::Unavailable(Control::Sound))
    );
}

#[test]
fn request_is_busy_without_optimistically_changing_authority() {
    let mut state = State::new(available_inputs());
    let operation = state
        .begin(Command::SetWifiEnabled(false))
        .expect("Wi-Fi is available");
    let view = state.view();
    assert!(view.wifi.busy);
    assert!(view.wifi.value);
    assert_eq!(
        state.begin(Command::SetWifiEnabled(false)),
        Err(StartError::Busy(Control::Wifi))
    );
    assert_eq!(operation.command, Command::SetWifiEnabled(false));
}

#[test]
fn failure_keeps_last_known_good_state_and_exposes_error() {
    let mut state = State::new(available_inputs());
    let operation = state
        .begin(Command::SetOutputMuted(true))
        .expect("sound is available");
    assert!(state.fail(&operation, "permission denied"));
    let view = state.view();
    assert!(!view.sound.busy);
    assert!(!view.sound.value.muted);
    assert_eq!(view.sound.error.as_deref(), Some("permission denied"));
}

#[test]
fn success_uses_the_refreshed_authoritative_snapshot() {
    let mut state = State::new(available_inputs());
    let operation = state
        .begin(Command::SetOutputVolume(70))
        .expect("sound is available");
    let mut authoritative = available_inputs();
    authoritative.audio.output.volume = 68;
    assert!(state.complete(&operation, authoritative));
    let view = state.view();
    assert!(!view.sound.busy);
    assert_eq!(view.sound.value.volume, 68);
}

#[test]
fn external_refresh_during_a_request_does_not_end_busy_state() {
    let mut state = State::new(available_inputs());
    let operation = state
        .begin(Command::SetBluetoothPowered(false))
        .expect("Bluetooth is available");
    let mut refreshed = available_inputs();
    refreshed.bluetooth.powered = false;
    state.refresh(refreshed);
    assert!(state.view().bluetooth.busy);
    assert!(state.complete(&operation, available_inputs()));
}

#[test]
fn one_completion_cannot_roll_back_an_unrelated_live_refresh() {
    let mut state = State::new(available_inputs());
    let operation = state
        .begin(Command::SetWifiEnabled(false))
        .expect("Wi-Fi is available");
    let mut refreshed = available_inputs();
    refreshed.audio.output.volume = 73;
    state.refresh(refreshed);

    let mut wifi_authority = available_inputs();
    wifi_authority.wifi.enabled = false;
    assert!(state.complete(&operation, wifi_authority));
    assert!(!state.view().wifi.value);
    assert_eq!(state.view().sound.value.volume, 73);
}

#[test]
fn invalid_volume_and_unsupported_profile_are_rejected() {
    let mut state = State::new(available_inputs());
    assert!(matches!(
        state.begin(Command::SetOutputVolume(101)),
        Err(StartError::Invalid {
            control: Control::Sound,
            ..
        })
    ));
    assert!(matches!(
        state.begin(Command::SetPowerProfile(
            rmac_power::PowerProfile::Performance
        )),
        Err(StartError::Unsupported {
            control: Control::Power,
            ..
        })
    ));
}

#[test]
fn stale_completion_cannot_overwrite_newer_state() {
    let mut state = State::new(available_inputs());
    let first = state
        .begin(Command::SetFocusEnabled(true))
        .expect("Focus is writable");
    assert!(state.fail(&first, "temporary failure"));
    let second = state
        .begin(Command::SetFocusEnabled(true))
        .expect("retry is allowed");
    let mut stale = available_inputs();
    stale.focus.enabled = true;
    assert!(!state.complete(&first, stale));
    assert!(state.view().focus.busy);
    assert!(state.fail(&second, "still unavailable"));
}

#[test]
fn one_popover_transfers_between_outputs_and_invoker_toggles_it_closed() {
    let view = State::new(available_inputs()).view();
    let mut popover = Popover::default();
    assert_eq!(
        popover.toggle("eDP-1".into(), &view),
        Outcome::Focused(Some(Control::Wifi))
    );
    assert_eq!(
        popover.output(),
        Some(&rmac_compositor::OutputId::from("eDP-1"))
    );
    assert_eq!(
        popover.toggle("HDMI-A-1".into(), &view),
        Outcome::Focused(Some(Control::Wifi))
    );
    assert_eq!(
        popover.toggle("HDMI-A-1".into(), &view),
        Outcome::Dismissed {
            output: "HDMI-A-1".into(),
            reason: DismissReason::Invoker,
        }
    );
    assert!(!popover.is_open());
}

#[test]
fn keyboard_focus_skips_unavailable_controls_and_wraps() {
    let mut inputs = available_inputs();
    inputs.wifi.available = false;
    inputs.audio.available = false;
    let view = State::new(inputs).view();
    let mut popover = Popover::default();
    popover.toggle("eDP-1".into(), &view);
    assert_eq!(popover.focused(), Some(Control::Bluetooth));
    assert_eq!(
        popover.interact(Interaction::FocusPrevious, &view),
        Outcome::Focused(Some(Control::Focus))
    );
    assert_eq!(
        popover.interact(Interaction::FocusNext, &view),
        Outcome::Focused(Some(Control::Bluetooth))
    );
}

#[test]
fn escape_and_outside_press_report_where_focus_must_return() {
    let view = State::new(available_inputs()).view();
    let mut popover = Popover::default();
    popover.toggle("eDP-1".into(), &view);
    assert_eq!(
        popover.interact(Interaction::Escape, &view),
        Outcome::Dismissed {
            output: "eDP-1".into(),
            reason: DismissReason::Escape,
        }
    );
    assert_eq!(popover.outside_press(), Outcome::Unchanged);

    popover.toggle("HDMI-A-1".into(), &view);
    assert_eq!(
        popover.outside_press(),
        Outcome::Dismissed {
            output: "HDMI-A-1".into(),
            reason: DismissReason::OutsidePress,
        }
    );

    popover.toggle("DP-2".into(), &view);
    assert_eq!(
        popover.owner_lost(),
        Outcome::Dismissed {
            output: "DP-2".into(),
            reason: DismissReason::OwnerLost,
        }
    );
}

#[test]
fn activation_uses_authoritative_values_and_busy_controls_ignore_repeats() {
    let mut state = State::new(available_inputs());
    let view = state.view();
    let mut popover = Popover::default();
    popover.toggle("eDP-1".into(), &view);
    assert_eq!(
        popover.interact(Interaction::Activate, &view),
        Outcome::Command(Command::SetWifiEnabled(false))
    );

    state
        .begin(Command::SetWifiEnabled(false))
        .expect("Wi-Fi request begins");
    assert_eq!(
        popover.interact(Interaction::Activate, &state.view()),
        Outcome::Unchanged
    );
}

#[test]
fn sound_and_power_support_keyboard_adjustment() {
    let mut inputs = available_inputs();
    inputs
        .power
        .profiles
        .supported
        .push(rmac_power::PowerProfile::Performance);
    let view = State::new(inputs).view();
    let mut popover = Popover::default();
    popover.toggle("eDP-1".into(), &view);
    popover.interact(Interaction::FocusNext, &view);
    popover.interact(Interaction::FocusNext, &view);
    assert_eq!(popover.focused(), Some(Control::Sound));
    assert_eq!(
        popover.interact(Interaction::Increment, &view),
        Outcome::Command(Command::SetOutputVolume(47))
    );
    assert_eq!(
        popover.interact(Interaction::Activate, &view),
        Outcome::Command(Command::SetOutputMuted(true))
    );

    popover.interact(Interaction::FocusNext, &view);
    assert_eq!(popover.focused(), Some(Control::Power));
    assert_eq!(
        popover.interact(Interaction::Increment, &view),
        Outcome::Command(Command::SetPowerProfile(
            rmac_power::PowerProfile::Performance
        ))
    );
    assert_eq!(
        popover.interact(Interaction::Decrement, &view),
        Outcome::Command(Command::SetPowerProfile(
            rmac_power::PowerProfile::PowerSaver
        ))
    );
}

#[test]
fn live_capability_loss_moves_focus_to_the_next_available_control() {
    let view = State::new(available_inputs()).view();
    let mut popover = Popover::default();
    popover.toggle("eDP-1".into(), &view);

    let mut inputs = available_inputs();
    inputs.wifi.available = false;
    let refreshed = State::new(inputs).view();
    assert_eq!(
        popover.refresh(&refreshed),
        Outcome::Focused(Some(Control::Bluetooth))
    );
}
