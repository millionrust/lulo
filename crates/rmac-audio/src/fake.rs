//! In-memory [`AudioService`] double for app and view-model tests.
//!
//! `FakeAudioService` holds its state in a `Mutex` and never touches
//! PipeWire, so app crates can exercise volume, mute and default-device
//! flows without a running audio graph. See
//! [`crate::contract::assert_audio_service_contract`] for the assertions
//! every implementation is expected to satisfy. This is a smaller seam
//! than `WifiService`/`BluetoothService`: it covers exactly the
//! snapshot/set_volume/set_muted/set_default_device surface that
//! `rmac-quick-settings-system::Backend` already consumes, not profile,
//! route, or balance control — see `docs/test-strategy.md`.

use std::sync::Mutex;

use super::*;

/// A snapshot-and-mutate audio service seam. `rmac-audio`'s existing
/// public `snapshot`/`set_volume`/`set_muted`/`set_default_device`
/// functions are untouched; `SystemAudioService` just delegates to them.
pub trait AudioService {
    fn snapshot(&self) -> Result<Snapshot, Error>;
    fn set_volume(&self, kind: DeviceKind, volume: u8) -> Result<(), Error>;
    fn set_muted(&self, kind: DeviceKind, muted: bool) -> Result<(), Error>;
    fn set_default_device(&self, kind: DeviceKind, device: &Device) -> Result<Snapshot, Error>;
}

pub struct SystemAudioService;

impl AudioService for SystemAudioService {
    fn snapshot(&self) -> Result<Snapshot, Error> {
        crate::snapshot()
    }

    fn set_volume(&self, kind: DeviceKind, volume: u8) -> Result<(), Error> {
        crate::set_volume(kind, volume)
    }

    fn set_muted(&self, kind: DeviceKind, muted: bool) -> Result<(), Error> {
        crate::set_muted(kind, muted)
    }

    fn set_default_device(&self, kind: DeviceKind, device: &Device) -> Result<Snapshot, Error> {
        crate::set_default_device(kind, device)
    }
}

/// An in-memory [`AudioService`] seeded with output/input devices and
/// levels. Construct with [`FakeAudioService::new`], seed it with
/// [`with_output`](Self::with_output) / [`with_input`](Self::with_input),
/// then pass `&fake` anywhere a `&impl AudioService` is expected.
pub struct FakeAudioService {
    state: Mutex<Snapshot>,
}

impl Default for FakeAudioService {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeAudioService {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(Snapshot {
                available: true,
                can_set_default: true,
                can_mute_input: true,
                ..Snapshot::default()
            }),
        }
    }

    pub fn with_output(self, device: Device, level: Level) -> Self {
        {
            let mut state = self.state.lock().unwrap();
            state.has_output = true;
            state.output = level;
            state.outputs.push(device);
        }
        self
    }

    pub fn with_input(self, device: Device, level: Level) -> Self {
        {
            let mut state = self.state.lock().unwrap();
            state.has_input = true;
            state.input = level;
            state.inputs.push(device);
        }
        self
    }
}

impl AudioService for FakeAudioService {
    fn snapshot(&self) -> Result<Snapshot, Error> {
        let state = self.state.lock().unwrap();
        if !state.available {
            return Err(Error::new("read audio state", "no audio service found"));
        }
        Ok(state.clone())
    }

    fn set_volume(&self, kind: DeviceKind, volume: u8) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();
        let level = match kind {
            DeviceKind::Output => &mut state.output,
            DeviceKind::Input => &mut state.input,
        };
        level.volume = volume.min(100);
        Ok(())
    }

    fn set_muted(&self, kind: DeviceKind, muted: bool) -> Result<(), Error> {
        let mut state = self.state.lock().unwrap();
        let level = match kind {
            DeviceKind::Output => &mut state.output,
            DeviceKind::Input => &mut state.input,
        };
        level.muted = muted;
        Ok(())
    }

    fn set_default_device(&self, kind: DeviceKind, device: &Device) -> Result<Snapshot, Error> {
        let mut state = self.state.lock().unwrap();
        let devices = match kind {
            DeviceKind::Output => &mut state.outputs,
            DeviceKind::Input => &mut state.inputs,
        };
        if !devices.iter().any(|candidate| candidate.id == device.id) {
            return Err(Error::new(
                "change the default audio device",
                "the device is no longer connected",
            ));
        }
        for candidate in devices.iter_mut() {
            candidate.is_default = candidate.id == device.id;
        }
        drop(state);
        self.snapshot()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract;

    fn device(id: &str, is_default: bool) -> Device {
        Device {
            id: id.to_string(),
            name: format!("Device {id}"),
            is_default,
            routes: Vec::new(),
            balance: None,
            authority_name: String::new(),
            authority_device_id: None,
            authority_route_device: None,
        }
    }

    #[test]
    fn fake_satisfies_the_shared_audio_service_contract() {
        let service = FakeAudioService::new()
            .with_output(
                device("speakers", true),
                Level {
                    volume: 50,
                    muted: false,
                },
            )
            .with_output(
                device("headphones", false),
                Level {
                    volume: 50,
                    muted: false,
                },
            );
        contract::assert_audio_service_contract(&service);
    }

    #[test]
    fn volume_is_clamped_to_one_hundred() {
        let service = FakeAudioService::new().with_output(
            device("speakers", true),
            Level {
                volume: 10,
                muted: false,
            },
        );
        service.set_volume(DeviceKind::Output, 250).unwrap();
        assert_eq!(service.snapshot().unwrap().output.volume, 100);
    }

    #[test]
    fn switching_default_to_an_unknown_device_is_rejected() {
        let service = FakeAudioService::new().with_output(
            device("speakers", true),
            Level {
                volume: 50,
                muted: false,
            },
        );
        assert!(service
            .set_default_device(DeviceKind::Output, &device("ghost", false))
            .is_err());
    }
}
