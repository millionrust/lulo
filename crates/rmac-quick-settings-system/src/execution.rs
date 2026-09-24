use rmac_quick_settings::{Command, Control, Inputs, Operation};

use crate::{Backend, Error, Phase};

/// Execute a validated operation and return a fresh value for its authority.
/// Other fields remain default because `rmac-quick-settings` merges only the
/// operation's owned field when completing it.
pub fn execute(operation: &Operation, backend: &impl Backend) -> Result<Inputs, Error> {
    let control = operation.command.control();
    mutate(&operation.command, backend)
        .map_err(|detail| Error::new(control, Phase::Mutate, detail))?;
    if matches!(operation.command, Command::SetOutputVolume(_)) {
        let _ = rmac_sound::play(rmac_sound::Cue::VolumeTick);
    }
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
        Command::JoinWifi(network) => backend.join_wifi(network),
        Command::SetBluetoothDeviceConnected { device, connected } => {
            backend.set_bluetooth_device_connected(device, *connected)
        }
        Command::SetDefaultOutput(device) => backend.set_default_output(device),
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
