use crate::{
    Command, Control, FocusValue, Inputs, Operation, Pending, PowerValue, SoundValue, StartError,
    State, Tile, View,
};

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
                available: self.inputs.audio.available && self.inputs.audio.has_output,
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
            Control::Sound => self.inputs.audio.available && self.inputs.audio.has_output,
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
            Command::JoinWifi(id) => {
                let wifi = &self.inputs.wifi;
                let network = wifi.networks.iter().find(|network| network.id == *id);
                match network {
                    _ if !wifi.enabled => Err(StartError::Unavailable(Control::Wifi)),
                    Some(network) if crate::detail::joins_directly(network) => Ok(()),
                    Some(_) => Err(StartError::Unsupported {
                        control: Control::Wifi,
                        detail: "this network needs a password; join it in Wi-Fi Settings".into(),
                    }),
                    None => Err(StartError::Invalid {
                        control: Control::Wifi,
                        detail: "the network is no longer in range".into(),
                    }),
                }
            }
            Command::SetBluetoothDeviceConnected { device, .. } => {
                let bluetooth = &self.inputs.bluetooth;
                let paired = bluetooth
                    .devices
                    .iter()
                    .any(|candidate| candidate.id == *device && candidate.paired);
                if !bluetooth.powered {
                    Err(StartError::Unavailable(Control::Bluetooth))
                } else if paired {
                    Ok(())
                } else {
                    Err(StartError::Invalid {
                        control: Control::Bluetooth,
                        detail: "the device is not paired with this computer".into(),
                    })
                }
            }
            Command::SetDefaultOutput(device) => {
                let audio = &self.inputs.audio;
                if !audio.can_set_default {
                    Err(StartError::Unsupported {
                        control: Control::Sound,
                        detail: "the sound server cannot change the output".into(),
                    })
                } else if audio.outputs.iter().any(|output| output.id == *device) {
                    Ok(())
                } else {
                    Err(StartError::Invalid {
                        control: Control::Sound,
                        detail: "the output is no longer connected".into(),
                    })
                }
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
    } else if !snapshot.has_output {
        "No Output Device".into()
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
