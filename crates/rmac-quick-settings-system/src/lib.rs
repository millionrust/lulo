//! Blocking platform executor for typed quick-settings operations.
//!
//! Call [`execute`] from a blocking/background executor, never a render path.

use std::fmt;

use rmac_quick_settings::{Command, Control, Inputs, Operation};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    Mutate,
    Refresh,
}

impl fmt::Display for Phase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Mutate => "change",
            Self::Refresh => "refresh",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Error {
    pub control: Control,
    pub phase: Phase,
    detail: String,
}

impl Error {
    fn new(control: Control, phase: Phase, detail: impl Into<String>) -> Self {
        Self {
            control,
            phase,
            detail: detail.into(),
        }
    }

    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "could not {} {}: {}",
            self.phase,
            self.control.label(),
            self.detail
        )
    }
}

impl std::error::Error for Error {}

pub trait Backend {
    fn set_wifi_enabled(&self, enabled: bool) -> Result<(), String>;
    fn wifi(&self) -> Result<rmac_network::WifiSnapshot, String>;

    fn set_bluetooth_powered(&self, powered: bool) -> Result<(), String>;
    fn bluetooth(&self) -> Result<rmac_bluetooth::Snapshot, String>;

    fn set_output_volume(&self, volume: u8) -> Result<(), String>;
    fn set_output_muted(&self, muted: bool) -> Result<(), String>;
    fn audio(&self) -> Result<rmac_audio::Snapshot, String>;

    fn set_power_profile(&self, profile: rmac_power::PowerProfile) -> Result<(), String>;
    fn power(&self) -> Result<rmac_power::Snapshot, String>;

    fn set_focus_enabled(&self, enabled: bool) -> Result<(), String>;
    fn focus(&self) -> Result<rmac_shell_settings::FocusSettings, String>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemBackend;

impl Backend for SystemBackend {
    fn set_wifi_enabled(&self, enabled: bool) -> Result<(), String> {
        rmac_network::set_enabled(enabled).map_err(|error| error.to_string())
    }

    fn wifi(&self) -> Result<rmac_network::WifiSnapshot, String> {
        rmac_network::snapshot().map_err(|error| error.to_string())
    }

    fn set_bluetooth_powered(&self, powered: bool) -> Result<(), String> {
        rmac_bluetooth::set_powered(powered).map_err(|error| error.to_string())
    }

    fn bluetooth(&self) -> Result<rmac_bluetooth::Snapshot, String> {
        rmac_bluetooth::snapshot().map_err(|error| error.to_string())
    }

    fn set_output_volume(&self, volume: u8) -> Result<(), String> {
        rmac_audio::set_volume(rmac_audio::DeviceKind::Output, volume)
            .map_err(|error| error.to_string())
    }

    fn set_output_muted(&self, muted: bool) -> Result<(), String> {
        rmac_audio::set_muted(rmac_audio::DeviceKind::Output, muted)
            .map_err(|error| error.to_string())
    }

    fn audio(&self) -> Result<rmac_audio::Snapshot, String> {
        rmac_audio::snapshot().map_err(|error| error.to_string())
    }

    fn set_power_profile(&self, profile: rmac_power::PowerProfile) -> Result<(), String> {
        rmac_power::set_profile(profile).map_err(|error| error.to_string())
    }

    fn power(&self) -> Result<rmac_power::Snapshot, String> {
        rmac_power::snapshot().map_err(|error| error.to_string())
    }

    fn set_focus_enabled(&self, enabled: bool) -> Result<(), String> {
        let _ = enabled;
        Err("the live Focus command authority is not connected".into())
    }

    fn focus(&self) -> Result<rmac_shell_settings::FocusSettings, String> {
        Err("the live Focus command authority is not connected".into())
    }
}

/// Execute a validated operation and return a fresh value for its authority.
/// Other fields remain default because `rmac-quick-settings` merges only the
/// operation's owned field when completing it.
pub fn execute(operation: &Operation, backend: &impl Backend) -> Result<Inputs, Error> {
    let control = operation.command.control();
    mutate(&operation.command, backend)
        .map_err(|detail| Error::new(control, Phase::Mutate, detail))?;
    refresh(control, backend).map_err(|detail| Error::new(control, Phase::Refresh, detail))
}

fn mutate(command: &Command, backend: &impl Backend) -> Result<(), String> {
    match command {
        Command::SetWifiEnabled(enabled) => backend.set_wifi_enabled(*enabled),
        Command::SetBluetoothPowered(powered) => backend.set_bluetooth_powered(*powered),
        Command::SetOutputVolume(volume) => backend.set_output_volume(*volume),
        Command::SetOutputMuted(muted) => backend.set_output_muted(*muted),
        Command::SetPowerProfile(profile) => backend.set_power_profile(*profile),
        Command::SetFocusEnabled(enabled) => backend.set_focus_enabled(*enabled),
    }
}

fn refresh(control: Control, backend: &impl Backend) -> Result<Inputs, String> {
    let mut inputs = Inputs::default();
    match control {
        Control::Wifi => inputs.wifi = backend.wifi()?,
        Control::Bluetooth => inputs.bluetooth = backend.bluetooth()?,
        Control::Sound => inputs.audio = backend.audio()?,
        Control::Power => inputs.power = backend.power()?,
        Control::Focus => {
            inputs.focus = backend.focus()?;
            inputs.focus_available = true;
        }
    }
    Ok(inputs)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

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
        let error = execute(&operation(Command::SetWifiEnabled(false)), &backend)
            .expect_err("mutation fails");
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
    fn system_backend_never_mutates_legacy_focus_preferences() {
        let backend = SystemBackend;
        assert!(backend.set_focus_enabled(true).is_err());
        assert!(backend.focus().is_err());
    }
}
