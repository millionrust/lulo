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
    /// Join a saved or open network from the Wi-Fi detail list.
    JoinWifi(rmac_network::WifiNetworkId),
    /// Connect or disconnect a paired device from the Bluetooth detail list.
    SetBluetoothDeviceConnected {
        device: String,
        connected: bool,
    },
    /// Make this output (a `rmac_audio::Device` id) the default.
    SetDefaultOutput(String),
}

impl Command {
    pub fn control(&self) -> Control {
        match self {
            Self::SetWifiEnabled(_) | Self::JoinWifi(_) => Control::Wifi,
            Self::SetBluetoothPowered(_) | Self::SetBluetoothDeviceConnected { .. } => {
                Control::Bluetooth
            }
            Self::SetOutputVolume(_) | Self::SetOutputMuted(_) | Self::SetDefaultOutput(_) => {
                Control::Sound
            }
            Self::SetPowerProfile(_) => Control::Power,
            Self::SetFocusEnabled(_) => Control::Focus,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Operation {
    pub(super) id: u64,
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
pub(super) struct Pending {
    pub(super) id: u64,
}

#[derive(Clone, Debug, Default)]
pub struct State {
    pub(super) inputs: Inputs,
    pub(super) pending: BTreeMap<Control, Pending>,
    pub(super) errors: BTreeMap<Control, String>,
    pub(super) next_id: u64,
}

pub(super) const FOCUS_ORDER: [Control; 5] = [
    Control::Wifi,
    Control::Bluetooth,
    Control::Sound,
    Control::Power,
    Control::Focus,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DismissReason {
    Escape,
    OutsidePress,
    Invoker,
    OwnerLost,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Interaction {
    Escape,
    FocusNext,
    FocusPrevious,
    Activate,
    Increment,
    Decrement,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Outcome {
    Unchanged,
    Focused(Option<Control>),
    Command(Command),
    Dismissed {
        output: rmac_compositor::OutputId,
        reason: DismissReason,
    },
}
