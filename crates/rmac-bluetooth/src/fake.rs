//! In-memory [`BluetoothService`] double for app and view-model tests.
//!
//! `FakeBluetoothService` holds its state in a `Mutex` and never touches
//! BlueZ, so app crates can exercise adapter power, discovery, pairing and
//! device-connect flows without a D-Bus session or hardware. See
//! [`crate::contract::assert_bluetooth_service_contract`] for the
//! state-machine assertions every implementation is expected to satisfy.

use std::sync::Mutex;

use super::*;

#[derive(Clone, Debug, Default)]
struct FakeState {
    available: bool,
    powered: bool,
    discoverable: bool,
    discovering: bool,
    adapter_name: Option<String>,
    devices: Vec<Device>,
}

/// An in-memory [`BluetoothService`] seeded with known devices.
///
/// Construct with [`FakeBluetoothService::new`], seed it with
/// [`with_device`](Self::with_device), then pass `&fake` anywhere a
/// `&impl BluetoothService` (or `&dyn BluetoothService`) is expected.
pub struct FakeBluetoothService {
    state: Mutex<FakeState>,
}

impl Default for FakeBluetoothService {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeBluetoothService {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(FakeState {
                available: true,
                powered: true,
                discoverable: false,
                discovering: false,
                adapter_name: Some("Fake Adapter".to_string()),
                devices: Vec::new(),
            }),
        }
    }

    /// No Bluetooth adapter is present: `snapshot` reports
    /// `available: false` and every mutation fails.
    pub fn without_adapter() -> Self {
        Self {
            state: Mutex::new(FakeState {
                available: false,
                powered: false,
                discoverable: false,
                discovering: false,
                adapter_name: None,
                devices: Vec::new(),
            }),
        }
    }

    pub fn with_powered(self, powered: bool) -> Self {
        self.state.lock().unwrap().powered = powered;
        self
    }

    pub fn with_device(self, device: Device) -> Self {
        self.state.lock().unwrap().devices.push(device);
        self
    }
}

impl BluetoothService for FakeBluetoothService {
    fn snapshot(&self) -> Result<Snapshot, Error> {
        let state = self.state.lock().unwrap();
        if !state.available {
            return Err(Error::new("read Bluetooth state", "no adapter found"));
        }
        Ok(Snapshot {
            available: true,
            powered: state.powered,
            discoverable: state.discoverable,
            discovering: state.discovering,
            adapter_name: state.adapter_name.clone(),
            devices: state.devices.clone(),
        })
    }

    fn set_powered(&self, powered: bool) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();
        if !state.available {
            return Err(Error::new("change Bluetooth power", "no adapter found"));
        }
        state.powered = powered;
        if !powered {
            state.discoverable = false;
            state.discovering = false;
            for device in &mut state.devices {
                device.connected = false;
            }
        }
        Ok(())
    }

    fn set_discoverable(&self, discoverable: bool) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();
        if !state.powered {
            return Err(Error::new(
                "change Bluetooth discoverability",
                "Bluetooth is off",
            ));
        }
        state.discoverable = discoverable;
        Ok(())
    }

    fn start_discovery(&self) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();
        if !state.powered {
            return Err(Error::new("start Bluetooth discovery", "Bluetooth is off"));
        }
        state.discovering = true;
        Ok(())
    }

    fn stop_discovery(&self) -> Result<(), Error> {
        self.state.lock().unwrap().discovering = false;
        Ok(())
    }

    fn set_connected(&self, device_id: &str, connected: bool) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();
        if !state.powered {
            return Err(Error::new(
                "change Bluetooth connection",
                "Bluetooth is off",
            ));
        }
        let device = state
            .devices
            .iter_mut()
            .find(|device| device.id == device_id)
            .ok_or_else(|| Error::new("change Bluetooth connection", "the device is unknown"))?;
        if connected && !device.paired {
            return Err(Error::new(
                "change Bluetooth connection",
                "the device is not paired",
            ));
        }
        device.connected = connected;
        Ok(())
    }

    fn pair(&self, device_id: &str, session: &PairingSession) -> Result<Snapshot, Error> {
        {
            let state = self.state.lock().unwrap();
            if !state.powered {
                return Err(Error::new("pair Bluetooth device", "Bluetooth is off"));
            }
            if !state.devices.iter().any(|device| device.id == device_id) {
                return Err(Error::new("pair Bluetooth device", "the device is unknown"));
            }
        }
        let outcome = session.outcome();
        if outcome.canceled || outcome.rejected || outcome.timed_out {
            return Err(Error::pairing(
                "pair Bluetooth device",
                "pairing did not complete",
                session,
            ));
        }
        {
            let mut state = self.state.lock().unwrap();
            let device = state
                .devices
                .iter_mut()
                .find(|device| device.id == device_id)
                .expect("checked above");
            device.paired = true;
            device.trusted = true;
            device.connected = true;
        }
        self.snapshot()
    }

    fn cancel_pairing(&self, device_id: &str) -> Result<(), Error> {
        let state = self.state.lock().unwrap();
        if !state.devices.iter().any(|device| device.id == device_id) {
            return Err(Error::new(
                "cancel Bluetooth pairing",
                "the device is unknown",
            ));
        }
        Ok(())
    }

    fn remove_device(&self, device_id: &str) -> Result<Snapshot, Error> {
        {
            let mut state = self.state.lock().unwrap();
            let before = state.devices.len();
            state.devices.retain(|device| device.id != device_id);
            if state.devices.len() == before {
                return Err(Error::new(
                    "remove Bluetooth device",
                    "the device is unknown",
                ));
            }
        }
        self.snapshot()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract;

    fn device(id: &str, paired: bool, connected: bool) -> Device {
        Device {
            id: id.to_string(),
            name: format!("Device {id}"),
            address: "AA:BB:CC:DD:EE:FF".to_string(),
            kind: "headphones".to_string(),
            paired,
            trusted: paired,
            connected,
        }
    }

    #[test]
    fn fake_satisfies_the_shared_bluetooth_service_contract() {
        let service = FakeBluetoothService::new().with_device(device("headset", false, false));
        contract::assert_bluetooth_service_contract(&service);
    }

    #[test]
    fn fake_reports_errors_without_an_adapter() {
        let service = FakeBluetoothService::without_adapter();
        assert!(service.snapshot().is_err());
        assert!(service.set_powered(true).is_err());
        assert!(service.start_discovery().is_err());
    }

    #[test]
    fn connecting_an_unpaired_device_is_rejected() {
        let service = FakeBluetoothService::new().with_device(device("mouse", false, false));
        assert!(service.set_connected("mouse", true).is_err());
    }

    #[test]
    fn powering_off_disconnects_every_device() {
        let service = FakeBluetoothService::new().with_device(device("headset", true, true));
        service.set_powered(false).unwrap();
        let snapshot = service.set_powered(true).and_then(|_| service.snapshot());
        // Powering back on does not restore the previous connection.
        assert!(!snapshot.unwrap().devices[0].connected);
    }

    #[test]
    fn pairing_a_canceled_session_fails_without_changing_state() {
        let service = FakeBluetoothService::new().with_device(device("phone", false, false));
        let (session, _events) = PairingSession::new();
        session.cancel();
        assert!(service.pair("phone", &session).is_err());
        let snapshot = service.snapshot().unwrap();
        assert!(!snapshot.devices[0].paired);
    }
}
