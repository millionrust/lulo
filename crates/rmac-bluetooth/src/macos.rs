#[cfg(target_os = "macos")]
use std::process::Command;

use crate::Device;
#[cfg(target_os = "macos")]
use crate::{BluetoothService, Error, PairingSession, Snapshot, SystemBluetoothService};

#[cfg(target_os = "macos")]
impl BluetoothService for SystemBluetoothService {
    fn snapshot(&self) -> Result<Snapshot, Error> {
        let output = Command::new("system_profiler")
            .arg("SPBluetoothDataType")
            .output()
            .map_err(|error| Error::new("start Bluetooth profiler", error.to_string()))?;
        if !output.status.success() {
            return Err(Error::new(
                "read Bluetooth state",
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ));
        }
        Ok(parse_macos_snapshot(&String::from_utf8_lossy(
            &output.stdout,
        )))
    }

    fn set_powered(&self, _powered: bool) -> Result<(), Error> {
        Err(Error::new(
            "change Bluetooth power",
            "the macOS development adapter is read-only",
        ))
    }

    fn set_discoverable(&self, _discoverable: bool) -> Result<(), Error> {
        Err(Error::new(
            "change Bluetooth discoverability",
            "the macOS development adapter is read-only",
        ))
    }

    fn start_discovery(&self) -> Result<(), Error> {
        Ok(())
    }

    fn stop_discovery(&self) -> Result<(), Error> {
        Ok(())
    }

    fn set_connected(&self, _device_id: &str, _connected: bool) -> Result<(), Error> {
        Err(Error::new(
            "change Bluetooth connection",
            "the macOS development adapter is read-only",
        ))
    }

    fn pair(&self, _device_id: &str, session: &PairingSession) -> Result<Snapshot, Error> {
        session.finish();
        Err(Error::new(
            "pair Bluetooth device",
            "the macOS development adapter is read-only",
        ))
    }

    fn cancel_pairing(&self, _device_id: &str) -> Result<(), Error> {
        Err(Error::new(
            "cancel Bluetooth pairing",
            "the macOS development adapter is read-only",
        ))
    }

    fn remove_device(&self, _device_id: &str) -> Result<Snapshot, Error> {
        Err(Error::new(
            "forget Bluetooth device",
            "the macOS development adapter is read-only",
        ))
    }
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn parse_macos_snapshot(output: &str) -> Snapshot {
    let powered = output
        .lines()
        .any(|line| line.trim().eq_ignore_ascii_case("State: On"));
    let mut devices = Vec::new();
    let mut connected_group = false;
    let mut current: Option<Device> = None;
    let flush = |current: &mut Option<Device>, devices: &mut Vec<Device>| {
        if let Some(device) = current.take() {
            devices.push(device);
        }
    };
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed == "Connected:" {
            flush(&mut current, &mut devices);
            connected_group = true;
        } else if trimmed == "Not Connected:" {
            flush(&mut current, &mut devices);
            connected_group = false;
        } else if trimmed.ends_with(':') && !trimmed.contains(": ") {
            flush(&mut current, &mut devices);
            let name = trimmed.trim_end_matches(':').trim();
            if !matches!(
                name,
                "" | "Bluetooth" | "Bluetooth Controller" | "Controller"
            ) {
                current = Some(Device {
                    id: name.to_string(),
                    name: name.to_string(),
                    address: String::new(),
                    kind: String::new(),
                    paired: true,
                    trusted: true,
                    connected: connected_group,
                });
            }
        } else if let Some(device) = current.as_mut() {
            if let Some(value) = trimmed.strip_prefix("Address:") {
                device.address = value.trim().to_string();
            } else if let Some(value) = trimmed.strip_prefix("Minor Type:") {
                device.kind = value.trim().to_string();
            }
        }
    }
    flush(&mut current, &mut devices);
    sort_devices(&mut devices);
    Snapshot {
        available: output.contains("Bluetooth Controller:"),
        powered,
        adapter_name: Some("Bluetooth".into()),
        devices,
        ..Snapshot::default()
    }
}

pub(crate) fn sort_devices(devices: &mut [Device]) {
    devices.sort_by(|left, right| {
        right
            .connected
            .cmp(&left.connected)
            .then_with(|| right.paired.cmp(&left.paired))
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
}
