//! Framework-neutral transaction model for the shell quick-settings surface.

use std::collections::BTreeMap;
use std::fmt;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Inputs {
    pub wifi: rmac_network::WifiSnapshot,
    pub bluetooth: rmac_bluetooth::Snapshot,
    pub audio: rmac_audio::Snapshot,
    pub power: rmac_power::Snapshot,
    pub focus: rmac_shell_settings::FocusSettings,
    /// True only while the writable shell-settings authority is reachable.
    pub focus_available: bool,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Control {
    Wifi,
    Bluetooth,
    Sound,
    Power,
    Focus,
}

impl Control {
    pub fn label(self) -> &'static str {
        match self {
            Self::Wifi => "Wi-Fi",
            Self::Bluetooth => "Bluetooth",
            Self::Sound => "Sound",
            Self::Power => "Power Mode",
            Self::Focus => "Focus",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command {
    SetWifiEnabled(bool),
    SetBluetoothPowered(bool),
    SetOutputVolume(u8),
    SetOutputMuted(bool),
    SetPowerProfile(rmac_power::PowerProfile),
    SetFocusEnabled(bool),
}

impl Command {
    pub fn control(&self) -> Control {
        match self {
            Self::SetWifiEnabled(_) => Control::Wifi,
            Self::SetBluetoothPowered(_) => Control::Bluetooth,
            Self::SetOutputVolume(_) | Self::SetOutputMuted(_) => Control::Sound,
            Self::SetPowerProfile(_) => Control::Power,
            Self::SetFocusEnabled(_) => Control::Focus,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Operation {
    id: u64,
    pub command: Command,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StartError {
    Busy(Control),
    Unavailable(Control),
    Invalid { control: Control, detail: String },
    Unsupported { control: Control, detail: String },
}

impl fmt::Display for StartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Busy(control) => write!(formatter, "{} is already changing", control.label()),
            Self::Unavailable(control) => write!(formatter, "{} is unavailable", control.label()),
            Self::Invalid { control, detail } => {
                write!(formatter, "invalid {} change: {detail}", control.label())
            }
            Self::Unsupported { control, detail } => {
                write!(
                    formatter,
                    "unsupported {} change: {detail}",
                    control.label()
                )
            }
        }
    }
}

impl std::error::Error for StartError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Tile<T> {
    pub available: bool,
    pub busy: bool,
    pub value: T,
    pub summary: String,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SoundValue {
    pub volume: u8,
    pub muted: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PowerValue {
    pub active: Option<rmac_power::PowerProfile>,
    pub supported: Vec<rmac_power::PowerProfile>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FocusValue {
    pub enabled: bool,
    pub mode: Option<String>,
    pub ends_at_unix_ms: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct View {
    pub wifi: Tile<bool>,
    pub bluetooth: Tile<bool>,
    pub sound: Tile<SoundValue>,
    pub power: Tile<PowerValue>,
    pub focus: Tile<FocusValue>,
}

#[derive(Clone, Debug)]
struct Pending {
    id: u64,
}

#[derive(Clone, Debug, Default)]
pub struct State {
    inputs: Inputs,
    pending: BTreeMap<Control, Pending>,
    errors: BTreeMap<Control, String>,
    next_id: u64,
}

impl State {
    pub fn new(inputs: Inputs) -> Self {
        Self {
            inputs,
            ..Self::default()
        }
    }

    pub fn inputs(&self) -> &Inputs {
        &self.inputs
    }

    /// Replace values read from platform authorities without disturbing an
    /// in-flight transaction. The UI never treats its requested value as fact.
    pub fn refresh(&mut self, inputs: Inputs) {
        self.inputs = inputs;
    }

    pub fn begin(&mut self, command: Command) -> Result<Operation, StartError> {
        let control = command.control();
        if self.pending.contains_key(&control) {
            return Err(StartError::Busy(control));
        }
        self.validate(&command)?;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let operation = Operation {
            id: self.next_id,
            command,
        };
        self.pending.insert(control, Pending { id: operation.id });
        self.errors.remove(&control);
        Ok(operation)
    }

    /// Complete only the currently matching operation and adopt a fresh read
    /// for that control. Unrelated live inputs cannot be rolled back by a
    /// concurrently completing task. Late superseded results are ignored.
    pub fn complete(&mut self, operation: &Operation, authoritative: Inputs) -> bool {
        let control = operation.command.control();
        if !self.matches(operation) {
            return false;
        }
        match control {
            Control::Wifi => self.inputs.wifi = authoritative.wifi,
            Control::Bluetooth => self.inputs.bluetooth = authoritative.bluetooth,
            Control::Sound => self.inputs.audio = authoritative.audio,
            Control::Power => self.inputs.power = authoritative.power,
            Control::Focus => {
                self.inputs.focus = authoritative.focus;
                self.inputs.focus_available = authoritative.focus_available;
            }
        }
        self.pending.remove(&control);
        self.errors.remove(&control);
        true
    }

    /// Retain the last authoritative value and expose an actionable failure.
    pub fn fail(&mut self, operation: &Operation, detail: impl Into<String>) -> bool {
        let control = operation.command.control();
        if !self.matches(operation) {
            return false;
        }
        self.pending.remove(&control);
        self.errors.insert(control, detail.into());
        true
    }

    pub fn dismiss_error(&mut self, control: Control) -> bool {
        self.errors.remove(&control).is_some()
    }

    pub fn view(&self) -> View {
        View {
            wifi: Tile {
                available: self.inputs.wifi.available,
                busy: self.is_busy(Control::Wifi),
                value: self.inputs.wifi.enabled,
                summary: wifi_summary(&self.inputs.wifi),
                error: self.error(Control::Wifi),
            },
            bluetooth: Tile {
                available: self.inputs.bluetooth.available,
                busy: self.is_busy(Control::Bluetooth),
                value: self.inputs.bluetooth.powered,
                summary: bluetooth_summary(&self.inputs.bluetooth),
                error: self.error(Control::Bluetooth),
            },
            sound: Tile {
                available: self.inputs.audio.available,
                busy: self.is_busy(Control::Sound),
                value: SoundValue {
                    volume: self.inputs.audio.output.volume,
                    muted: self.inputs.audio.output.muted,
                },
                summary: sound_summary(&self.inputs.audio),
                error: self.error(Control::Sound),
            },
            power: Tile {
                available: self.inputs.power.profiles.available,
                busy: self.is_busy(Control::Power),
                value: PowerValue {
                    active: self.inputs.power.profiles.active,
                    supported: self.inputs.power.profiles.supported.clone(),
                },
                summary: power_summary(&self.inputs.power),
                error: self.error(Control::Power),
            },
            focus: Tile {
                available: self.inputs.focus_available,
                busy: self.is_busy(Control::Focus),
                value: FocusValue {
                    enabled: self.inputs.focus.enabled,
                    mode: self.inputs.focus.selected_mode.clone(),
                    ends_at_unix_ms: self.inputs.focus.ends_at_unix_ms,
                },
                summary: focus_summary(&self.inputs.focus),
                error: self.error(Control::Focus),
            },
        }
    }

    fn validate(&self, command: &Command) -> Result<(), StartError> {
        let available = match command.control() {
            Control::Wifi => self.inputs.wifi.available,
            Control::Bluetooth => self.inputs.bluetooth.available,
            Control::Sound => self.inputs.audio.available,
            Control::Power => self.inputs.power.profiles.available,
            Control::Focus => self.inputs.focus_available,
        };
        if !available {
            return Err(StartError::Unavailable(command.control()));
        }
        match command {
            Command::SetOutputVolume(volume) if *volume > 100 => Err(StartError::Invalid {
                control: Control::Sound,
                detail: "volume must be between 0 and 100".into(),
            }),
            Command::SetPowerProfile(profile)
                if !self.inputs.power.profiles.supported.contains(profile) =>
            {
                Err(StartError::Unsupported {
                    control: Control::Power,
                    detail: format!("{} is not supported by this computer", profile.label()),
                })
            }
            _ => Ok(()),
        }
    }

    fn matches(&self, operation: &Operation) -> bool {
        self.pending
            .get(&operation.command.control())
            .is_some_and(|pending| pending.id == operation.id)
    }

    fn is_busy(&self, control: Control) -> bool {
        self.pending.contains_key(&control)
    }

    fn error(&self, control: Control) -> Option<String> {
        self.errors.get(&control).cloned()
    }
}

fn wifi_summary(snapshot: &rmac_network::WifiSnapshot) -> String {
    if !snapshot.available {
        "Unavailable".into()
    } else if !snapshot.enabled {
        "Off".into()
    } else {
        snapshot
            .current_ssid
            .clone()
            .unwrap_or_else(|| "Not Connected".into())
    }
}

fn bluetooth_summary(snapshot: &rmac_bluetooth::Snapshot) -> String {
    if !snapshot.available {
        return "Unavailable".into();
    }
    if !snapshot.powered {
        return "Off".into();
    }
    let connected = snapshot
        .devices
        .iter()
        .filter(|device| device.connected)
        .count();
    match connected {
        0 => "On".into(),
        1 => "1 Device Connected".into(),
        count => format!("{count} Devices Connected"),
    }
}

fn sound_summary(snapshot: &rmac_audio::Snapshot) -> String {
    if !snapshot.available {
        "Unavailable".into()
    } else if snapshot.output.muted {
        "Muted".into()
    } else {
        format!("{}%", snapshot.output.volume)
    }
}

fn power_summary(snapshot: &rmac_power::Snapshot) -> String {
    if !snapshot.profiles.available {
        "Unavailable".into()
    } else {
        snapshot
            .profiles
            .active
            .map(rmac_power::PowerProfile::label)
            .unwrap_or("Unknown")
            .into()
    }
}

fn focus_summary(focus: &rmac_shell_settings::FocusSettings) -> String {
    if !focus.enabled {
        "Off".into()
    } else {
        focus.selected_mode.clone().unwrap_or_else(|| "On".into())
    }
}

#[cfg(test)]
mod tests {
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
}
